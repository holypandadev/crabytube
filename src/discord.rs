//! Optional Discord Rich Presence over the user's local Discord IPC socket.
//! No Discord token or network credential is used by this integration.

use std::{
    sync::mpsc::{self, Receiver, Sender},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};

/// Discord Application IDs are public identifiers and are safe to ship in a desktop binary.
const APPLICATION_ID: &str = "1557135013537906789";

/// Discord rate-limits activity updates; keep a gap between sends.
const MIN_ACTIVITY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub struct Presence {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub artwork_url: Option<String>,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub paused: bool,
    /// Localized word shown before the artist while paused.
    pub paused_label: String,
    /// Localized label for the button that opens the track.
    pub listen_label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ConnectionStatus {
    Idle = 0,
    Connecting = 1,
    Connected = 2,
    Unavailable = 3,
    Error = 4,
}

impl ConnectionStatus {
    fn from_raw(value: u8) -> Self {
        match value {
            1 => Self::Connecting,
            2 => Self::Connected,
            3 => Self::Unavailable,
            4 => Self::Error,
            _ => Self::Idle,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Enabled · waiting for playback",
            Self::Connecting => "Connecting to Discord…",
            Self::Connected => "Connected to Discord",
            Self::Unavailable => "Discord desktop client unavailable · retrying…",
            Self::Error => "Discord connection error · retrying…",
        }
    }
}

pub struct DiscordPresence {
    sender: Sender<Option<Presence>>,
    status: Arc<AtomicU8>,
}

impl DiscordPresence {
    pub fn start() -> Self {
        let (sender, receiver) = mpsc::channel::<Option<Presence>>();
        let status = Arc::new(AtomicU8::new(ConnectionStatus::Idle as u8));
        let worker_status = Arc::clone(&status);
        let spawned = thread::Builder::new()
            .name("crabytube-discord-rpc".into())
            .spawn(move || {
                let mut last: Option<Presence> = None;
                let mut session: Option<Session> = None;
                let mut client: Option<DiscordIpcClient> = None;
                let mut last_sent: Option<Instant> = None;
                loop {
                    let received = match receiver.recv_timeout(Duration::from_secs(15)) {
                        Ok(update) => Some(update),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                    };
                    let mut update = latest_update(&receiver, received);
                    // Hold the next send until the interval passes, then keep only the newest update.
                    if let Some(wait) = last_sent
                        .and_then(|sent: Instant| MIN_ACTIVITY_INTERVAL.checked_sub(sent.elapsed()))
                    {
                        thread::sleep(wait);
                        update = latest_update(&receiver, update);
                    }
                    match update {
                        Some(Some(presence)) => {
                            session = Some(session_after(session.take(), &presence, now_millis()));
                            last = Some(presence);
                        }
                        Some(None) => {
                            last = None;
                            session = None;
                            if let Some(client) = client.as_mut() {
                                last_sent = Some(Instant::now());
                                let _ = client.clear_activity();
                            }
                            set_status(&worker_status, ConnectionStatus::Idle);
                            continue;
                        }
                        None if last.is_none() => continue,
                        None => {}
                    }

                    let (Some(presence), Some(session)) = (last.as_ref(), session.as_ref()) else {
                        continue;
                    };
                    if client.is_none() {
                        set_status(&worker_status, ConnectionStatus::Connecting);
                        let mut candidate = DiscordIpcClient::new(APPLICATION_ID);
                        match candidate.connect() {
                            Ok(()) => client = Some(candidate),
                            Err(error) => {
                                set_status(&worker_status, ConnectionStatus::Unavailable);
                                tracing::debug!(%error, "Discord IPC is not available");
                                continue;
                            }
                        }
                    }
                    let activity = build_activity(presence, session.start_ms);
                    last_sent = Some(Instant::now());
                    let update_error = client
                        .as_mut()
                        .and_then(|connected| connected.set_activity(activity).err());
                    if let Some(error) = update_error {
                        set_status(&worker_status, ConnectionStatus::Error);
                        tracing::debug!(%error, "Discord Rich Presence update failed");
                        client = None;
                    } else {
                        set_status(&worker_status, ConnectionStatus::Connected);
                    }
                }
                if let Some(mut client) = client {
                    let _ = client.clear_activity();
                    let _ = client.close();
                }
            });
        if let Err(error) = spawned {
            // The receiver drops with the failed closure, so later sends report Error.
            tracing::error!(%error, "could not start the Discord presence thread");
            set_status(&status, ConnectionStatus::Error);
        }
        Self { sender, status }
    }

    pub fn update(&self, presence: Presence) {
        if self.status() != ConnectionStatus::Connected {
            set_status(&self.status, ConnectionStatus::Connecting);
        }
        if self.sender.send(Some(presence)).is_err() {
            set_status(&self.status, ConnectionStatus::Error);
        }
    }

    pub fn clear(&self) {
        set_status(&self.status, ConnectionStatus::Idle);
        if self.sender.send(None).is_err() {
            set_status(&self.status, ConnectionStatus::Error);
        }
    }

    pub fn status(&self) -> ConnectionStatus {
        ConnectionStatus::from_raw(self.status.load(Ordering::Acquire))
    }
}

fn set_status(status: &AtomicU8, value: ConnectionStatus) {
    status.store(value as u8, Ordering::Release);
}

/// Drains everything queued behind `update` so only the newest state is applied.
fn latest_update(
    receiver: &Receiver<Option<Presence>>,
    mut update: Option<Option<Presence>>,
) -> Option<Option<Presence>> {
    while let Ok(newer) = receiver.try_recv() {
        update = Some(newer);
    }
    update
}

/// Playback anchor shared by every refresh of the same track and play state.
#[derive(Clone, Debug, PartialEq)]
struct Session {
    key: (String, String),
    paused: bool,
    /// Wall-clock millis at which the track's position was zero.
    start_ms: i64,
}

/// Progress drift beyond this is treated as a seek and re-anchors the timeline.
const SEEK_TOLERANCE_MS: i64 = 3_000;

/// Keeps the previous anchor on refreshes; re-anchors on track change,
/// play/pause change, or a seek.
fn session_after(previous: Option<Session>, presence: &Presence, now_ms: i64) -> Session {
    let position_ms = presence.position.as_millis() as i64;
    let key = (presence.title.clone(), presence.artist.clone());
    if let Some(previous) = previous
        && previous.key == key
        && previous.paused == presence.paused
        && (presence.paused
            || (now_ms - previous.start_ms - position_ms).abs() <= SEEK_TOLERANCE_MS)
    {
        return previous;
    }
    Session {
        key,
        paused: presence.paused,
        start_ms: now_ms - position_ms,
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn build_activity(presence: &Presence, start_ms: i64) -> activity::Activity<'static> {
    let title = activity_text(bounded(&presence.title, 120));
    let artist = bounded(&presence.artist, 120);
    let state = if presence.paused {
        format!("{} · {artist}", presence.paused_label)
    } else {
        artist
    };
    let mut activity = activity::Activity::new()
        .name("CrabyTube")
        .details(title.clone())
        .state(activity_text(state))
        .activity_type(activity::ActivityType::Listening);

    if !presence.paused {
        let timestamps = activity::Timestamps::new().start(start_ms);
        let timestamps = presence.duration.map_or(timestamps.clone(), |duration| {
            timestamps.end(start_ms.saturating_add(duration.as_millis() as i64))
        });
        activity = activity.timestamps(timestamps);
    }

    // Asset keys must exist in the Discord application; unknown keys are simply not shown.
    let mut assets = activity::Assets::new()
        .large_text(
            presence
                .album
                .as_deref()
                .map_or_else(|| title.clone(), |album| bounded(album, 120)),
        )
        .small_image(if presence.paused { "paused" } else { "playing" });
    if let Some(url) = presence.artwork_url.as_deref()
        && url.starts_with("https://")
    {
        assets = assets.large_image(url.to_owned());
    }
    activity = activity.assets(assets);

    if let Some(url) = watch_url(&presence.video_id) {
        let label = bounded(&presence.listen_label, 32);
        activity = activity.buttons(vec![activity::Button::new(label, url)]);
    }
    activity
}

/// Builds the public watch link, accepting only well-formed YouTube video ids.
fn watch_url(video_id: &str) -> Option<String> {
    let valid = !video_id.is_empty()
        && video_id.len() <= 32
        && video_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| format!("https://music.youtube.com/watch?v={video_id}"))
}

fn bounded(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// Discord drops activity text shorter than two characters, so short text gets blank braille cells.
/// They render as nothing and are not whitespace, so they survive trimming on Discord's side.
fn activity_text(value: String) -> String {
    let mut text = value;
    while text.chars().count() < 2 {
        text.push('\u{2800}');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_status_labels_cover_the_ipc_lifecycle() {
        assert_eq!(
            ConnectionStatus::Idle.label(),
            "Enabled · waiting for playback"
        );
        assert_eq!(
            ConnectionStatus::Connecting.label(),
            "Connecting to Discord…"
        );
        assert_eq!(ConnectionStatus::Connected.label(), "Connected to Discord");
        assert!(
            ConnectionStatus::Unavailable
                .label()
                .contains("unavailable")
        );
        assert!(ConnectionStatus::Error.label().contains("error"));
    }

    #[test]
    fn unknown_connection_status_values_fall_back_to_idle() {
        assert_eq!(ConnectionStatus::from_raw(u8::MAX), ConnectionStatus::Idle);
    }

    #[test]
    fn presence_text_is_bounded_for_discord_limits() {
        assert_eq!(bounded(&"a".repeat(200), 120).len(), 120);
    }

    #[test]
    fn short_activity_text_is_padded_to_the_two_character_minimum() {
        assert_eq!(activity_text("A".into()).chars().count(), 2);
        assert_eq!(activity_text(String::new()).chars().count(), 2);
        assert_eq!(activity_text("Abc".into()), "Abc");
    }

    #[test]
    fn paused_activity_omits_running_progress_timestamps() {
        let presence = Presence {
            paused: true,
            ..presence_fixture(Duration::from_secs(30))
        };
        let value = serde_json::to_value(build_activity(&presence, 0)).unwrap();
        assert_eq!(value["state"], "Paused · Artist");
        assert!(value.get("timestamps").is_none());
    }

    #[test]
    fn activity_links_to_the_track_and_shows_play_state() {
        let value =
            serde_json::to_value(build_activity(&presence_fixture(Duration::from_secs(1)), 0))
                .unwrap();
        assert_eq!(
            value["buttons"][0]["url"],
            "https://music.youtube.com/watch?v=abc123DEF_-"
        );
        assert_eq!(value["buttons"][0]["label"], "Listen on YouTube Music");
        assert_eq!(value["assets"]["small_image"], "playing");
        assert_eq!(value["assets"]["large_text"], "Album");
    }

    #[test]
    fn watch_button_is_omitted_for_malformed_video_ids() {
        assert_eq!(watch_url("bad id/../x"), None);
        assert_eq!(watch_url(""), None);
        assert!(watch_url("abc123DEF_-").is_some());
    }

    fn presence_fixture(position: Duration) -> Presence {
        Presence {
            video_id: "abc123DEF_-".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: Some("Album".into()),
            artwork_url: None,
            position,
            duration: Some(Duration::from_secs(180)),
            paused: false,
            paused_label: "Paused".into(),
            listen_label: "Listen on YouTube Music".into(),
        }
    }

    #[test]
    fn queued_updates_coalesce_to_the_newest_state() {
        let (sender, receiver) = mpsc::channel::<Option<Presence>>();
        sender.send(Some(playing_at(1))).unwrap();
        sender.send(Some(playing_at(2))).unwrap();
        sender.send(None).unwrap();
        sender.send(Some(playing_at(3))).unwrap();

        let latest = latest_update(&receiver, Some(Some(playing_at(0))));
        assert_eq!(latest.unwrap().unwrap().position, Duration::from_secs(3));
        assert!(receiver.try_recv().is_err());
    }

    fn playing_at(secs: u64) -> Presence {
        Presence {
            duration: None,
            ..presence_fixture(Duration::from_secs(secs))
        }
    }

    #[test]
    fn progress_anchor_survives_refreshes_and_resets_on_seek() {
        let first = session_after(None, &playing_at(30), 1_000_000);
        assert_eq!(first.start_ms, 970_000);

        // A periodic refresh at the same progress keeps the original anchor.
        let refreshed = session_after(Some(first.clone()), &playing_at(45), 1_015_000);
        assert_eq!(refreshed.start_ms, 970_000);

        // A seek far from the expected progress re-anchors the timeline.
        let seeked = session_after(Some(first), &playing_at(120), 1_015_000);
        assert_eq!(seeked.start_ms, 895_000);
    }
}
