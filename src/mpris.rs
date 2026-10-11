//! Linux MPRIS controls. The D-Bus worker never waits on the UI or audio thread.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

use futures_util::{StreamExt, stream, stream::BoxStream};
use tokio::sync::{Mutex as AsyncMutex, mpsc::UnboundedReceiver, mpsc::UnboundedSender};
use zbus::{
    blocking::connection::Builder,
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value},
};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.crabytube";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
/// Slack for the wake-up timing when deciding that a position jump was a seek.
const SEEK_TOLERANCE_SECS: f64 = 1.5;
const TRACK_ID: &str = "/org/mpris/MediaPlayer2/Track/current";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerState {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Artwork location for shells that show a cover (`mpris:artUrl`).
    pub art_url: Option<String>,
    pub playback_status: String,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f64,
    pub shuffle: bool,
    pub loop_status: String,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RemoteCommand {
    Next,
    Previous,
    Pause,
    Play,
    PlayPause,
    Stop,
    SeekBy(i64),
    SeekTo(i64),
    SetVolume(f64),
    SetShuffle(bool),
    SetLoopStatus(String),
    Raise,
    Quit,
}

#[derive(Clone)]
pub struct CommandReceiver {
    id: u64,
    receiver: Arc<AsyncMutex<UnboundedReceiver<RemoteCommand>>>,
}

impl PartialEq for CommandReceiver {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for CommandReceiver {}

impl Hash for CommandReceiver {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

pub fn command_stream(receiver: &CommandReceiver) -> BoxStream<'static, RemoteCommand> {
    stream::unfold(receiver.clone(), |receiver| async move {
        let command = receiver.receiver.lock().await.recv().await?;
        Some((command, receiver))
    })
    .boxed()
}

pub struct MprisRemote {
    state: Arc<Mutex<PlayerState>>,
    commands: CommandReceiver,
    wake: SyncSender<()>,
    status: Arc<Mutex<String>>,
}

impl MprisRemote {
    pub fn start() -> Self {
        let state = Arc::new(Mutex::new(PlayerState {
            playback_status: "Stopped".into(),
            volume: 1.0,
            loop_status: "None".into(),
            ..PlayerState::default()
        }));
        let status = Arc::new(Mutex::new("Starting MPRIS session-bus service…".into()));
        static NEXT_COMMAND_STREAM_ID: AtomicU64 = AtomicU64::new(1);
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let commands = CommandReceiver {
            id: NEXT_COMMAND_STREAM_ID.fetch_add(1, Ordering::Relaxed),
            receiver: Arc::new(AsyncMutex::new(command_rx)),
        };
        let (wake, wake_rx) = mpsc::sync_channel(1);
        #[cfg(test)]
        if std::env::var_os("CRABYTUBE_TEST_MPRIS_BUS").as_deref()
            != Some(std::ffi::OsStr::new("1"))
        {
            if let Ok(mut current) = status.lock() {
                *current = "MPRIS disabled outside an isolated D-Bus test".into();
            }
            drop(wake_rx);
            return Self {
                state,
                commands,
                wake,
                status,
            };
        }
        let worker_state = Arc::clone(&state);
        let worker_status = Arc::clone(&status);
        let spawn = thread::Builder::new()
            .name("crabytube-mpris".into())
            .spawn(move || {
                let connection = Builder::session()
                    .and_then(|builder| builder.name(BUS_NAME))
                    .and_then(|builder| {
                        builder.serve_at(
                            OBJECT_PATH,
                            RootInterface {
                                commands: command_tx.clone(),
                            },
                        )
                    })
                    .and_then(|builder| {
                        builder.serve_at(
                            OBJECT_PATH,
                            PlayerInterface {
                                state: Arc::clone(&worker_state),
                                commands: command_tx,
                            },
                        )
                    })
                    .and_then(Builder::build);
                let connection = match connection {
                    Ok(connection) => connection,
                    Err(error) => {
                        if let Ok(mut status) = worker_status.lock() {
                            *status = format!("MPRIS unavailable: {error}");
                        }
                        return;
                    }
                };
                if let Ok(mut status) = worker_status.lock() {
                    *status = "MPRIS media controls active on the Linux session bus".into();
                }

                let mut last_state = PlayerState::default();
                let mut last_position_change = Instant::now();
                loop {
                    let poll_interval = worker_state
                        .lock()
                        .map(|state| {
                            if state.playback_status == "Playing" {
                                Duration::from_millis(250)
                            } else {
                                Duration::from_secs(1)
                            }
                        })
                        .unwrap_or(Duration::from_secs(1));
                    match wake_rx.recv_timeout(poll_interval) {
                        Ok(_) => {}
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    let current = worker_state
                        .lock()
                        .map(|state| state.clone())
                        .unwrap_or_default();
                    if state_properties_changed(&last_state, &current) {
                        emit_changed(&connection, &last_state, &current);
                    }
                    if is_seek(&last_state, &current, last_position_change.elapsed()) {
                        emit_seeked(&connection, &current);
                    }
                    if current.position != last_state.position {
                        last_position_change = Instant::now();
                    }
                    last_state = current;
                }
                let _ = connection.close();
            });
        if let Err(error) = spawn
            && let Ok(mut status) = status.lock()
        {
            *status = format!("MPRIS worker could not start: {error}");
        }
        Self {
            state,
            commands,
            wake,
            status,
        }
    }

    pub fn update(&self, state: PlayerState) {
        if let Ok(mut current) = self.state.lock() {
            if *current == state {
                return;
            }
            *current = state;
        }
        let _ = self.wake.try_send(());
    }

    pub fn command_receiver(&self) -> CommandReceiver {
        self.commands.clone()
    }

    pub fn status(&self) -> String {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| "MPRIS status unavailable".into())
    }

    pub fn drain(&self) -> Vec<RemoteCommand> {
        let Ok(mut receiver) = self.commands.receiver.try_lock() else {
            return Vec::new();
        };
        let mut commands = Vec::new();
        while let Ok(command) = receiver.try_recv() {
            commands.push(command);
        }
        commands
    }
}

struct RootInterface {
    commands: UnboundedSender<RemoteCommand>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl RootInterface {
    fn raise(&self) -> zbus::fdo::Result<()> {
        self.commands
            .send(RemoteCommand::Raise)
            .map_err(|_| zbus::fdo::Error::Failed("CrabyTube is shutting down".into()))
    }

    fn quit(&self) -> zbus::fdo::Result<()> {
        self.commands
            .send(RemoteCommand::Quit)
            .map_err(|_| zbus::fdo::Error::Failed("CrabyTube is shutting down".into()))
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        "CrabyTube".into()
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        crate::APP_ID.into()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct PlayerInterface {
    state: Arc<Mutex<PlayerState>>,
    commands: UnboundedSender<RemoteCommand>,
}

impl PlayerInterface {
    fn snapshot(&self) -> PlayerState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    fn send(&self, command: RemoteCommand) {
        let _ = self.commands.send(command);
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    fn next(&self) {
        self.send(RemoteCommand::Next);
    }
    fn previous(&self) {
        self.send(RemoteCommand::Previous);
    }
    fn pause(&self) {
        self.send(RemoteCommand::Pause);
    }
    fn play(&self) {
        self.send(RemoteCommand::Play);
    }
    fn play_pause(&self) {
        self.send(RemoteCommand::PlayPause);
    }
    fn stop(&self) {
        self.send(RemoteCommand::Stop);
    }

    fn seek(&self, offset: i64) {
        self.send(RemoteCommand::SeekBy(offset));
    }

    fn set_position(&self, track_id: OwnedObjectPath, position: i64) {
        if track_id.as_str() == TRACK_ID {
            self.send(RemoteCommand::SeekTo(position));
        }
    }

    fn open_uri(&self, _uri: String) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "Opening external URIs is not implemented".into(),
        ))
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.snapshot().playback_status
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        self.snapshot().loop_status
    }

    #[zbus(property)]
    fn set_loop_status(&self, value: &str) -> zbus::fdo::Result<()> {
        if ["None", "Track", "Playlist"].contains(&value) {
            self.send(RemoteCommand::SetLoopStatus(value.into()));
            Ok(())
        } else {
            Err(zbus::fdo::Error::InvalidArgs(
                "LoopStatus must be None, Track, or Playlist".into(),
            ))
        }
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn set_rate(&self, value: f64) -> zbus::fdo::Result<()> {
        if value == 1.0 {
            Ok(())
        } else {
            Err(zbus::fdo::Error::NotSupported(
                "Playback speed control is not implemented".into(),
            ))
        }
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.snapshot().shuffle
    }

    #[zbus(property)]
    fn set_shuffle(&self, value: bool) {
        self.send(RemoteCommand::SetShuffle(value));
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        metadata(&self.snapshot())
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.snapshot().volume
    }

    #[zbus(property)]
    fn set_volume(&self, value: f64) {
        if value.is_finite() {
            self.send(RemoteCommand::SetVolume(value.clamp(0.0, 1.0)));
        }
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        position_micros(&self.snapshot())
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.snapshot().can_go_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.snapshot().can_go_previous
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.snapshot().can_play
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.snapshot().can_pause
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.snapshot().duration.is_some()
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

fn metadata(state: &PlayerState) -> HashMap<String, OwnedValue> {
    let mut values = HashMap::new();
    let track = ObjectPath::try_from(TRACK_ID).expect("static MPRIS object path is valid");
    values.insert("mpris:trackid".into(), variant(track));
    if let Some(title) = &state.title {
        values.insert("xesam:title".into(), variant(title.clone()));
    }
    if let Some(artist) = &state.artist {
        values.insert("xesam:artist".into(), variant(vec![artist.clone()]));
    }
    if let Some(album) = &state.album {
        values.insert("xesam:album".into(), variant(album.clone()));
    }
    if let Some(art_url) = &state.art_url {
        values.insert("mpris:artUrl".into(), variant(art_url.clone()));
    }
    if let Some(duration) = state.duration {
        values.insert(
            "mpris:length".into(),
            variant(duration.as_micros().min(i64::MAX as u128) as i64),
        );
    }
    values
}

fn variant<T>(value: T) -> OwnedValue
where
    Value<'static>: From<T>,
{
    OwnedValue::try_from(Value::from(value)).expect("MPRIS metadata type is representable")
}

fn position_micros(state: &PlayerState) -> i64 {
    let value = state.position.as_micros();
    value.min(i64::MAX as u128) as i64
}

fn state_properties_changed(old: &PlayerState, new: &PlayerState) -> bool {
    old.title != new.title
        || old.artist != new.artist
        || old.album != new.album
        || old.playback_status != new.playback_status
        || old.duration != new.duration
        || old.volume != new.volume
        || old.shuffle != new.shuffle
        || old.loop_status != new.loop_status
        || old.can_play != new.can_play
        || old.can_pause != new.can_pause
        || old.can_go_next != new.can_go_next
        || old.can_go_previous != new.can_go_previous
}

fn emit_changed(connection: &zbus::blocking::Connection, old: &PlayerState, new: &PlayerState) {
    let mut changed = HashMap::<String, OwnedValue>::new();
    if old.playback_status != new.playback_status {
        changed.insert(
            "PlaybackStatus".into(),
            variant(new.playback_status.clone()),
        );
    }
    if old.loop_status != new.loop_status {
        changed.insert("LoopStatus".into(), variant(new.loop_status.clone()));
    }
    if old.shuffle != new.shuffle {
        changed.insert("Shuffle".into(), variant(new.shuffle));
    }
    if old.volume != new.volume {
        changed.insert("Volume".into(), variant(new.volume));
    }
    if old.title != new.title
        || old.artist != new.artist
        || old.album != new.album
        || old.duration != new.duration
    {
        changed.insert("Metadata".into(), variant(metadata(new)));
    }
    if old.can_play != new.can_play {
        changed.insert("CanPlay".into(), variant(new.can_play));
    }
    if old.can_pause != new.can_pause {
        changed.insert("CanPause".into(), variant(new.can_pause));
    }
    if old.can_go_next != new.can_go_next {
        changed.insert("CanGoNext".into(), variant(new.can_go_next));
    }
    if old.can_go_previous != new.can_go_previous {
        changed.insert("CanGoPrevious".into(), variant(new.can_go_previous));
    }
    if !changed.is_empty() {
        let body = (PLAYER_IFACE, changed, Vec::<String>::new());
        let _ = connection.emit_signal(
            None::<&str>,
            OBJECT_PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &body,
        );
    }
}

/// Whether the position moved other than by playback: a user seek, or a jump by the shell.
/// A track change is not a seek, and neither is a pause or a resume.
fn is_seek(old: &PlayerState, new: &PlayerState, since_last_change: Duration) -> bool {
    if old.title != new.title || old.playback_status != new.playback_status {
        return false;
    }
    if old.position == new.position {
        return false;
    }
    let played = if new.playback_status == "Playing" {
        since_last_change.as_secs_f64()
    } else {
        0.0
    };
    let moved = new.position.as_secs_f64() - old.position.as_secs_f64();
    (moved - played).abs() > SEEK_TOLERANCE_SECS
}

/// `Seeked` tells clients the position jumped, so they do not interpolate across it.
fn emit_seeked(connection: &zbus::blocking::Connection, state: &PlayerState) {
    let _ = connection.emit_signal(
        None::<&str>,
        OBJECT_PATH,
        PLAYER_IFACE,
        "Seeked",
        &position_micros(state),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_jump_is_a_seek_only_when_playback_does_not_explain_it() {
        let at = |title: &str, status: &str, seconds: u64| PlayerState {
            title: Some(title.into()),
            playback_status: status.into(),
            position: Duration::from_secs(seconds),
            ..PlayerState::default()
        };
        let half_second = Duration::from_millis(500);
        // Normal playback: the position moved as far as the time since the last change.
        assert!(!is_seek(
            &at("a", "Playing", 10),
            &at("a", "Playing", 11),
            Duration::from_secs(1)
        ));
        // A jump forward or backward while playing.
        assert!(is_seek(
            &at("a", "Playing", 10),
            &at("a", "Playing", 40),
            half_second
        ));
        assert!(is_seek(
            &at("a", "Playing", 30),
            &at("a", "Playing", 5),
            half_second
        ));
        // Paused: any move is a seek, but a track change never is.
        assert!(is_seek(
            &at("a", "Paused", 10),
            &at("a", "Paused", 20),
            half_second
        ));
        assert!(!is_seek(
            &at("a", "Paused", 10),
            &at("a", "Paused", 10),
            half_second
        ));
        assert!(!is_seek(
            &at("a", "Playing", 10),
            &at("b", "Playing", 40),
            half_second
        ));
        // A pause or a resume is a playback change, not a seek.
        assert!(!is_seek(
            &at("a", "Playing", 10),
            &at("a", "Paused", 30),
            half_second
        ));
    }

    #[tokio::test]
    async fn command_stream_delivers_dbus_events_without_polling() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let receiver = CommandReceiver {
            id: 0,
            receiver: Arc::new(AsyncMutex::new(receiver)),
        };
        let mut events = Box::pin(command_stream(&receiver));
        sender.send(RemoteCommand::PlayPause).unwrap();
        assert_eq!(events.next().await, Some(RemoteCommand::PlayPause));
    }

    #[test]
    fn metadata_uses_mpris_types_and_bounds_duration() {
        let state = PlayerState {
            title: Some("Track".into()),
            artist: Some("Artist".into()),
            duration: Some(Duration::from_secs(180)),
            ..PlayerState::default()
        };
        let values = metadata(&state);
        assert_eq!(
            String::try_from(values["xesam:title"].try_clone().unwrap()).unwrap(),
            "Track"
        );
        assert_eq!(
            i64::try_from(values["mpris:length"].try_clone().unwrap()).unwrap(),
            180_000_000
        );
        let track_id =
            OwnedObjectPath::try_from(values["mpris:trackid"].try_clone().unwrap()).unwrap();
        assert_eq!(track_id.as_str(), TRACK_ID);
    }

    #[test]
    fn position_does_not_exceed_mpris_integer_range() {
        let state = PlayerState {
            position: Duration::MAX,
            ..PlayerState::default()
        };
        assert_eq!(position_micros(&state), i64::MAX);
    }

    #[test]
    fn isolated_session_bus_publishes_metadata_and_dispatches_commands() {
        if std::env::var_os("CRABYTUBE_TEST_MPRIS_BUS").as_deref()
            != Some(std::ffi::OsStr::new("1"))
        {
            return;
        }
        let service = MprisRemote::start();
        service.update(PlayerState {
            title: Some("Bus test".into()),
            artist: Some("CrabyTube".into()),
            playback_status: "Paused".into(),
            can_play: true,
            can_pause: true,
            ..PlayerState::default()
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while service.status().starts_with("Starting") && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            service.status().contains("controls active"),
            "{}",
            service.status()
        );

        let client = zbus::blocking::Connection::session().unwrap();
        let root =
            zbus::blocking::Proxy::new(&client, BUS_NAME, OBJECT_PATH, "org.mpris.MediaPlayer2")
                .unwrap();
        let can_raise: bool = root.get_property("CanRaise").unwrap();
        let can_quit: bool = root.get_property("CanQuit").unwrap();
        assert!(can_raise);
        assert!(can_quit);
        let proxy =
            zbus::blocking::Proxy::new(&client, BUS_NAME, OBJECT_PATH, PLAYER_IFACE).unwrap();
        let status: String = proxy.get_property("PlaybackStatus").unwrap();
        let metadata: HashMap<String, OwnedValue> = proxy.get_property("Metadata").unwrap();
        assert_eq!(status, "Paused");
        assert_eq!(
            String::try_from(metadata["xesam:title"].try_clone().unwrap()).unwrap(),
            "Bus test"
        );
        let _: () = root.call("Raise", &()).unwrap();
        let _: () = root.call("Quit", &()).unwrap();
        let _: () = proxy.call("PlayPause", &()).unwrap();
        let command_deadline = Instant::now() + Duration::from_secs(1);
        let mut received = Vec::new();
        while received.is_empty() && Instant::now() < command_deadline {
            received = service.drain();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            received,
            vec![
                RemoteCommand::Raise,
                RemoteCommand::Quit,
                RemoteCommand::PlayPause
            ]
        );
    }
}
