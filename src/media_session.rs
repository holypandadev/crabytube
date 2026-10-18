//! The part of the system media controls that does not depend on the operating system: the state
//! the player reports, the commands the system sends back, and the rules that turn one into the
//! other. A platform backend (Now Playing on macOS) only talks to its own API and uses these.

use std::{
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use futures_util::{StreamExt, stream, stream::BoxStream};
use tokio::sync::{Mutex as AsyncMutex, mpsc::UnboundedReceiver, mpsc::UnboundedSender};

/// How far the reported position may stray from where playback should be before the system is told
/// again. Smaller differences are the wake-up timing of the progress tick.
pub const SEEK_TOLERANCE: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerState {
    /// The video id of the track, which names it in `mpris:trackid`.
    pub track_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
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
    /// The track is loading, or the next one is about to: playback was asked for, and the position
    /// does not move yet.
    pub loading: bool,
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

/// What the system can ask the player to do, as the platform reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    /// Jump to this many seconds into the track.
    SetPosition(f64),
}

pub fn command_for(event: MediaEvent) -> RemoteCommand {
    match event {
        MediaEvent::Play => RemoteCommand::Play,
        MediaEvent::Pause => RemoteCommand::Pause,
        MediaEvent::Toggle => RemoteCommand::PlayPause,
        MediaEvent::Next => RemoteCommand::Next,
        MediaEvent::Previous => RemoteCommand::Previous,
        // A position is never negative, and a value that is not a number is no position at all.
        MediaEvent::SetPosition(seconds) if seconds.is_finite() => {
            RemoteCommand::SeekTo((seconds.max(0.0) * 1_000_000.0).round() as i64)
        }
        MediaEvent::SetPosition(_) => RemoteCommand::SeekTo(0),
    }
}

/// The track as the system shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: Option<f64>,
}

/// `None` when nothing is loaded, so the system shows no track.
pub fn now_playing(state: &PlayerState) -> Option<NowPlaying> {
    let title = state.title.clone().filter(|title| !title.is_empty())?;
    Some(NowPlaying {
        title,
        artist: state.artist.clone().filter(|artist| !artist.is_empty()),
        album: state.album.clone().filter(|album| !album.is_empty()),
        duration_secs: state
            .duration
            .filter(|duration| !duration.is_zero())
            .map(|duration| duration.as_secs_f64()),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackKind {
    Playing,
    Paused,
    Stopped,
}

/// Where playback is and how fast it moves. The system advances its progress bar by itself from
/// these two, so the rate is 1 while playing and 0 otherwise, a track that loads included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub kind: PlaybackKind,
    pub position_secs: f64,
    pub rate: f64,
}

pub fn playback(state: &PlayerState) -> Playback {
    let kind = match state.playback_status.as_str() {
        "Playing" => PlaybackKind::Playing,
        "Paused" => PlaybackKind::Paused,
        _ => PlaybackKind::Stopped,
    };
    Playback {
        kind,
        position_secs: state.position.as_secs_f64(),
        rate: if kind == PlaybackKind::Playing && !state.loading {
            1.0
        } else {
            0.0
        },
    }
}

/// What the previous report said, and when it was made.
#[derive(Clone, Copy, Debug)]
pub struct Previous<'a> {
    pub state: &'a PlayerState,
    pub at: Instant,
}

/// What a new report changes for the system.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// The track, or the part of it that the system shows.
    pub track: bool,
    /// The state or the position: paused, resumed, stopped, a seek or a stall.
    pub playback: bool,
    /// Which controls are available.
    pub controls: bool,
}

impl Changes {
    pub fn any(self) -> bool {
        self.track || self.playback || self.controls
    }
}

/// Compares a report with the previous one. Playback that goes on as expected changes nothing:
/// the system extrapolates the position from the rate, so it is told again only when the position
/// leaves that line.
pub fn changes(previous: Option<Previous<'_>>, state: &PlayerState, now: Instant) -> Changes {
    let Some(previous) = previous else {
        return Changes {
            track: true,
            playback: true,
            controls: true,
        };
    };
    let before = previous.state;
    Changes {
        track: now_playing(before) != now_playing(state),
        playback: playback_changed(before, previous.at, state, now),
        controls: (
            before.can_play,
            before.can_pause,
            before.can_go_next,
            before.can_go_previous,
        ) != (
            state.can_play,
            state.can_pause,
            state.can_go_next,
            state.can_go_previous,
        ),
    }
}

fn playback_changed(
    before: &PlayerState,
    before_at: Instant,
    state: &PlayerState,
    now: Instant,
) -> bool {
    // The end of loading is when the position starts to move, which the system must hear from.
    if playback(before).kind != playback(state).kind || before.loading != state.loading {
        return true;
    }
    let expected = if playback(before).rate > 0.0 {
        before.position + now.saturating_duration_since(before_at)
    } else {
        before.position
    };
    state.position.abs_diff(expected) > SEEK_TOLERANCE
}

/// The commands the system sends, delivered to the app as a stream.
#[derive(Clone, Debug)]
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

/// A sender for the platform backend and the receiver the app listens on.
pub fn command_channel() -> (UnboundedSender<RemoteCommand>, CommandReceiver) {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    (
        sender,
        CommandReceiver {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            receiver: Arc::new(AsyncMutex::new(receiver)),
        },
    )
}

pub fn command_stream(receiver: &CommandReceiver) -> BoxStream<'static, RemoteCommand> {
    stream::unfold(receiver.clone(), |receiver| async move {
        let command = receiver.receiver.lock().await.recv().await?;
        Some((command, receiver))
    })
    .boxed()
}

/// Takes every command that has arrived and not been read yet.
pub fn drain(receiver: &CommandReceiver) -> Vec<RemoteCommand> {
    let Ok(mut queue) = receiver.receiver.try_lock() else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    while let Ok(command) = queue.try_recv() {
        commands.push(command);
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(position_secs: u64) -> PlayerState {
        PlayerState {
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            art_url: Some("https://example.invalid/cover".into()),
            playback_status: "Playing".into(),
            position: Duration::from_secs(position_secs),
            duration: Some(Duration::from_secs(200)),
            can_play: true,
            can_pause: true,
            can_go_next: true,
            can_go_previous: false,
            ..PlayerState::default()
        }
    }

    #[test]
    fn events_become_the_commands_the_app_already_handles() {
        assert_eq!(command_for(MediaEvent::Play), RemoteCommand::Play);
        assert_eq!(command_for(MediaEvent::Pause), RemoteCommand::Pause);
        assert_eq!(command_for(MediaEvent::Toggle), RemoteCommand::PlayPause);
        assert_eq!(command_for(MediaEvent::Next), RemoteCommand::Next);
        assert_eq!(command_for(MediaEvent::Previous), RemoteCommand::Previous);
    }

    #[test]
    fn a_position_is_sent_in_microseconds_and_never_negative() {
        assert_eq!(
            command_for(MediaEvent::SetPosition(12.5)),
            RemoteCommand::SeekTo(12_500_000)
        );
        assert_eq!(
            command_for(MediaEvent::SetPosition(-3.0)),
            RemoteCommand::SeekTo(0)
        );
        assert_eq!(
            command_for(MediaEvent::SetPosition(f64::NAN)),
            RemoteCommand::SeekTo(0)
        );
        assert_eq!(
            command_for(MediaEvent::SetPosition(f64::INFINITY)),
            RemoteCommand::SeekTo(0)
        );
    }

    #[test]
    fn the_track_is_what_the_state_names() {
        let track = now_playing(&playing(5)).expect("a track");
        assert_eq!(track.title, "Song");
        assert_eq!(track.artist.as_deref(), Some("Artist"));
        assert_eq!(track.album.as_deref(), Some("Album"));
        assert_eq!(track.duration_secs, Some(200.0));
        // Empty fields are left out, and with no title there is no track.
        let bare = PlayerState {
            title: Some("Song".into()),
            artist: Some(String::new()),
            duration: Some(Duration::ZERO),
            ..PlayerState::default()
        };
        let track = now_playing(&bare).expect("a track");
        assert_eq!(
            (track.artist, track.album, track.duration_secs),
            (None, None, None)
        );
        assert_eq!(now_playing(&PlayerState::default()), None);
    }

    #[test]
    fn the_rate_is_one_while_playing_and_zero_otherwise() {
        let mut state = playing(42);
        let now = playback(&state);
        assert_eq!(
            (now.kind, now.position_secs, now.rate),
            (PlaybackKind::Playing, 42.0, 1.0)
        );
        state.playback_status = "Paused".into();
        let paused = playback(&state);
        assert_eq!((paused.kind, paused.rate), (PlaybackKind::Paused, 0.0));
        state.playback_status = "Stopped".into();
        assert_eq!(playback(&state).kind, PlaybackKind::Stopped);
        state.playback_status = "anything else".into();
        assert_eq!(playback(&state).rate, 0.0);
    }

    #[test]
    fn the_first_report_changes_everything() {
        let all = changes(None, &playing(0), Instant::now());
        assert!(all.track && all.playback && all.controls);
    }

    #[test]
    fn steady_playback_tells_the_system_nothing_new() {
        let start = Instant::now();
        let before = playing(10);
        let previous = Previous {
            state: &before,
            at: start,
        };
        // A quarter of a second later the position is a quarter of a second further.
        let mut next = playing(10);
        next.position += Duration::from_millis(250);
        let now = start + Duration::from_millis(250);
        assert!(!changes(Some(previous), &next, now).any());
        // And so is the same report a minute on, as long as the position kept pace.
        let mut later = playing(70);
        later.position += Duration::from_millis(100);
        let now = start + Duration::from_secs(60);
        assert!(!changes(Some(previous), &later, now).any());
    }

    #[test]
    fn a_seek_a_stall_and_a_pause_are_reported() {
        let start = Instant::now();
        let before = playing(10);
        let previous = Previous {
            state: &before,
            at: start,
        };
        let now = start + Duration::from_secs(1);
        // A jump forward and one back.
        assert!(changes(Some(previous), &playing(120), now).playback);
        assert!(changes(Some(previous), &playing(0), start + Duration::from_secs(20)).playback);
        // Playing for ten seconds without the position moving is a stall.
        let stalled = changes(
            Some(previous),
            &playing(10),
            start + Duration::from_secs(10),
        );
        assert!(stalled.playback && !stalled.track);
        // A pause and a resume.
        let mut paused = playing(11);
        paused.playback_status = "Paused".into();
        assert!(changes(Some(previous), &paused, now).playback);
        let paused_before = paused.clone();
        let paused_previous = Previous {
            state: &paused_before,
            at: start,
        };
        // While paused the position is expected to stay where it is, however long it takes.
        assert!(
            !changes(
                Some(paused_previous),
                &paused,
                start + Duration::from_secs(90)
            )
            .any()
        );
        assert!(changes(Some(paused_previous), &playing(11), now).playback);
    }

    #[test]
    fn a_new_track_and_a_late_duration_are_reported_but_a_new_cover_alone_is_not() {
        let start = Instant::now();
        let before = playing(10);
        let previous = Previous {
            state: &before,
            at: start,
        };
        let now = start + Duration::from_millis(250);
        let mut other = playing(10);
        other.position += Duration::from_millis(250);
        other.title = Some("Another".into());
        assert!(changes(Some(previous), &other, now).track);
        let mut longer = playing(10);
        longer.position += Duration::from_millis(250);
        longer.duration = Some(Duration::from_secs(201));
        assert!(changes(Some(previous), &longer, now).track);
        // The cover is fetched by the backend, which watches the URL itself.
        let mut cover = playing(10);
        cover.position += Duration::from_millis(250);
        cover.art_url = Some("https://example.invalid/other".into());
        assert!(!changes(Some(previous), &cover, now).any());
    }

    #[test]
    fn a_track_that_loads_stands_still_until_it_plays() {
        let start = Instant::now();
        let loading = PlayerState {
            loading: true,
            ..playing(0)
        };
        assert_eq!(playback(&loading).kind, PlaybackKind::Playing);
        assert_eq!(playback(&loading).rate, 0.0);
        let previous = Previous {
            state: &loading,
            at: start,
        };
        // Three seconds of loading leave the position where it was, which is no news.
        let now = start + Duration::from_secs(3);
        assert!(!changes(Some(previous), &loading, now).any());
        // The moment it plays is: the system takes its progress from then.
        assert!(changes(Some(previous), &playing(0), now).playback);
    }

    #[test]
    fn the_available_controls_follow_the_queue() {
        let start = Instant::now();
        let before = playing(10);
        let previous = Previous {
            state: &before,
            at: start,
        };
        let mut next = playing(10);
        next.position += Duration::from_millis(250);
        next.can_go_previous = true;
        let result = changes(Some(previous), &next, start + Duration::from_millis(250));
        assert!(result.controls && !result.track && !result.playback);
    }

    #[test]
    fn commands_sent_by_the_system_are_drained_in_order() {
        let (sender, receiver) = command_channel();
        assert!(drain(&receiver).is_empty());
        sender.send(command_for(MediaEvent::Next)).unwrap();
        sender.send(command_for(MediaEvent::Pause)).unwrap();
        assert_eq!(
            drain(&receiver),
            vec![RemoteCommand::Next, RemoteCommand::Pause]
        );
        assert!(drain(&receiver).is_empty());
        // Two channels never compare equal, so the app subscribes to each separately.
        assert_ne!(receiver, command_channel().1);
        assert_eq!(receiver, receiver.clone());
    }
}
