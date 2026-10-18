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

use serde_json::{Value, json};

/// Discord Application IDs are public identifiers and are safe to ship in a desktop binary.
const APPLICATION_ID: &str = "1557135013537906789";

/// Discord takes five activity updates in twenty seconds; a gap of four keeps within that.
const MIN_ACTIVITY_INTERVAL: Duration = Duration::from_secs(4);

/// The longest text Discord takes in the details, the state and the cover's hover text, counted
/// in UTF-16 units as Discord counts them, and the shortest.
const MAX_TEXT_UNITS: usize = 128;
const MIN_TEXT_UNITS: usize = 2;

/// The longest cover URL Discord takes; a longer one makes it refuse the whole activity.
const MAX_IMAGE_URL: usize = 300;

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
    /// Discord answered, but refused the activity.
    Rejected = 5,
}

impl ConnectionStatus {
    fn from_raw(value: u8) -> Self {
        match value {
            1 => Self::Connecting,
            2 => Self::Connected,
            3 => Self::Unavailable,
            4 => Self::Error,
            5 => Self::Rejected,
            _ => Self::Idle,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Enabled · waiting for playback",
            Self::Connecting => "Connecting to Discord…",
            Self::Connected => "Connected to Discord",
            // In the Flatpak a Discord opened after the app stays out of reach until the app restarts.
            Self::Unavailable if crate::flatpak::in_flatpak() => {
                "Discord not found · if it was opened after CrabyTube, restart CrabyTube"
            }
            Self::Unavailable => "Discord desktop client unavailable · retrying…",
            Self::Error => "Discord connection error · retrying…",
            Self::Rejected => "Discord did not accept the activity",
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
                let mut client: Option<ipc::Connection> = None;
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
                            if let Some(connection) = client.as_mut() {
                                last_sent = Some(Instant::now());
                                if connection.set_activity(None).is_err() {
                                    client = None;
                                }
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
                    let activity = build_activity(presence, session.start_ms);
                    last_sent = Some(Instant::now());
                    // A connection that broke, as when Discord restarted, is opened again at once.
                    for _ in 0..2 {
                        if client.is_none() {
                            set_status(&worker_status, ConnectionStatus::Connecting);
                            match ipc::Connection::open(APPLICATION_ID) {
                                Ok(connection) => client = Some(connection),
                                Err(error) => {
                                    report(&worker_status, ConnectionStatus::Unavailable, &error);
                                    break;
                                }
                            }
                        }
                        let Some(connection) = client.as_mut() else {
                            break;
                        };
                        match connection.set_activity(Some(activity.clone())) {
                            Ok(()) => {
                                set_status(&worker_status, ConnectionStatus::Connected);
                                break;
                            }
                            Err(ipc::Failure::Refused(reason)) => {
                                report(&worker_status, ConnectionStatus::Rejected, &reason);
                                break;
                            }
                            Err(ipc::Failure::Broken(reason)) => {
                                report(&worker_status, ConnectionStatus::Error, &reason);
                                client = None;
                            }
                        }
                    }
                }
                if let Some(mut connection) = client {
                    let _ = connection.set_activity(None);
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

/// Sets a failed status, and writes why to the log when the status changes, so a retry every few
/// seconds does not repeat it.
fn report(status: &AtomicU8, value: ConnectionStatus, reason: &str) {
    let previous = ConnectionStatus::from_raw(status.swap(value as u8, Ordering::AcqRel));
    if previous != value {
        match value {
            ConnectionStatus::Unavailable => tracing::info!(reason, "Discord is not reachable"),
            _ => tracing::warn!(reason, "Discord Rich Presence failed"),
        }
    }
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

/// The activity Discord shows for `presence`, in the form its `SET_ACTIVITY` command takes. Every
/// text is fitted to Discord's limits, which refuse the whole activity when one text is off.
fn build_activity(presence: &Presence, start_ms: i64) -> Value {
    let title = fit(&presence.title);
    let state = if presence.paused {
        fit(&format!("{} · {}", presence.paused_label, presence.artist))
    } else {
        fit(&presence.artist)
    };
    // Listening, so Discord says "Listening to CrabyTube".
    let mut activity = json!({ "type": 2, "details": title, "state": state });

    if !presence.paused {
        let mut timestamps = json!({ "start": start_ms });
        if let Some(duration) = presence.duration {
            timestamps["end"] = json!(start_ms.saturating_add(duration.as_millis() as i64));
        }
        activity["timestamps"] = timestamps;
    }

    // Asset keys must exist in the Discord application; unknown keys are simply not shown.
    let mut assets = json!({
        "large_text": presence.album.as_deref().map_or_else(|| title.clone(), fit),
        "small_image": if presence.paused { "paused" } else { "playing" },
    });
    if let Some(url) = presence.artwork_url.as_deref()
        && url.starts_with("https://")
        && url.len() <= MAX_IMAGE_URL
    {
        assets["large_image"] = json!(url);
    }
    activity["assets"] = assets;

    if let Some(url) = watch_url(&presence.video_id) {
        let label = bounded(&presence.listen_label, 32);
        activity["buttons"] = json!([{ "label": label, "url": url }]);
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

/// Fits a text to Discord's limits: at most 128 UTF-16 units, cut with an ellipsis, and at least two,
/// filled with blank braille cells. Those render as nothing and are not whitespace, so they survive
/// trimming on Discord's side.
fn fit(value: &str) -> String {
    let units = |text: &str| text.encode_utf16().count();
    let mut text = value.trim().to_owned();
    if units(&text) > MAX_TEXT_UNITS {
        while units(&text) > MAX_TEXT_UNITS - 1 {
            text.pop();
        }
        text.push('…');
    }
    while units(&text) < MIN_TEXT_UNITS {
        text.push('\u{2800}');
    }
    text
}

/// Discord's local IPC: a socket on Unix and a named pipe on Windows, carrying frames of an opcode
/// and a length, both 32-bit little-endian, followed by that much JSON.
mod ipc {
    use std::io::{self, Read, Write};
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};

    /// How long an answer from Discord is waited for.
    const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
    /// Discord's frames are a few kilobytes; anything far larger is not Discord talking.
    const MAX_FRAME_BYTES: usize = 64 * 1024;

    const HANDSHAKE: u32 = 0;
    const FRAME: u32 = 1;
    const CLOSE: u32 = 2;
    const PING: u32 = 3;
    const PONG: u32 = 4;

    /// Why an activity did not reach Discord's status.
    #[derive(Debug)]
    pub enum Failure {
        /// The connection broke; another one may work.
        Broken(String),
        /// Discord answered and refused the activity.
        Refused(String),
    }

    pub struct Connection {
        pipe: Pipe,
        nonce: u64,
    }

    impl Connection {
        /// Connects to the first Discord client that answers its handshake. Every place a Discord
        /// client may listen is tried in turn: a stale socket, or the empty file a Flatpak sandbox
        /// leaves where a socket was, does not hide the others.
        pub fn open(client_id: &str) -> Result<Self, String> {
            let mut last = String::from("no Discord client is listening");
            for path in candidates() {
                let Ok(pipe) = Pipe::open(&path) else {
                    continue;
                };
                let mut connection = Connection { pipe, nonce: 0 };
                match connection.handshake(client_id) {
                    Ok(()) => return Ok(connection),
                    Err(error) => last = error,
                }
            }
            Err(last)
        }

        fn handshake(&mut self, client_id: &str) -> Result<(), String> {
            self.send(HANDSHAKE, &json!({ "v": 1, "client_id": client_id }))
                .map_err(|error| error.to_string())?;
            let ready = self.receive().map_err(failure_text)?;
            if ready["evt"] == "READY" {
                Ok(())
            } else {
                Err(format!("Discord did not get ready: {ready}"))
            }
        }

        /// Shows `activity`, or clears it for `None`, and waits for Discord's answer.
        pub fn set_activity(&mut self, activity: Option<Value>) -> Result<(), Failure> {
            self.nonce += 1;
            let nonce = format!("crabytube-{}-{}", std::process::id(), self.nonce);
            let command = json!({
                "cmd": "SET_ACTIVITY",
                "args": { "pid": std::process::id(), "activity": activity },
                "nonce": nonce,
            });
            self.send(FRAME, &command)
                .map_err(|error| Failure::Broken(error.to_string()))?;
            // Discord may send other frames first; the answer carries the command's nonce.
            for _ in 0..8 {
                let answer = self.receive()?;
                if answer["nonce"] != nonce.as_str() {
                    continue;
                }
                if answer["evt"] == "ERROR" {
                    return Err(Failure::Refused(format!(
                        "{} ({})",
                        answer["data"]["message"]
                            .as_str()
                            .unwrap_or("no reason given"),
                        answer["data"]["code"]
                    )));
                }
                return Ok(());
            }
            Err(Failure::Broken(
                "Discord did not answer the activity".into(),
            ))
        }

        fn send(&mut self, opcode: u32, payload: &Value) -> io::Result<()> {
            let body = serde_json::to_vec(payload)?;
            let mut frame = Vec::with_capacity(8 + body.len());
            frame.extend_from_slice(&opcode.to_le_bytes());
            frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
            frame.extend_from_slice(&body);
            self.pipe.write_all(&frame)?;
            self.pipe.flush()
        }

        /// The next frame of data. Pings are answered on the way; a close is a broken connection.
        fn receive(&mut self) -> Result<Value, Failure> {
            let broken = |error: io::Error| Failure::Broken(error.to_string());
            let deadline = Instant::now() + REPLY_TIMEOUT;
            loop {
                let mut header = [0_u8; 8];
                self.pipe
                    .read_within(&mut header, deadline)
                    .map_err(broken)?;
                let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
                let length =
                    u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
                if length > MAX_FRAME_BYTES {
                    return Err(Failure::Broken(format!("a frame of {length} bytes")));
                }
                let mut body = vec![0_u8; length];
                self.pipe.read_within(&mut body, deadline).map_err(broken)?;
                let value: Value = serde_json::from_slice(&body)
                    .map_err(|error| Failure::Broken(error.to_string()))?;
                match opcode {
                    FRAME => return Ok(value),
                    CLOSE => {
                        return Err(Failure::Broken(format!(
                            "Discord closed the connection: {}",
                            value["message"].as_str().unwrap_or("no reason given")
                        )));
                    }
                    PING => self.send(PONG, &value).map_err(broken)?,
                    _ => {}
                }
            }
        }
    }

    fn failure_text(failure: Failure) -> String {
        match failure {
            Failure::Broken(text) | Failure::Refused(text) => text,
        }
    }

    /// The places a Discord client listens: the runtime and temporary folders, and in them the
    /// folders of the Flatpak, Vesktop and Snap packages of Discord, each with ten numbered sockets.
    /// Only sockets are kept.
    #[cfg(unix)]
    fn candidates() -> Vec<std::path::PathBuf> {
        use std::os::unix::fs::FileTypeExt;
        use std::path::PathBuf;
        const PLACES: [&str; 7] = [
            "",
            "app/com.discordapp.Discord",
            ".flatpak/com.discordapp.Discord/xdg-run",
            "app/dev.vencord.Vesktop",
            ".flatpak/dev.vencord.Vesktop/xdg-run",
            "snap.discord",
            "snap.discord-canary",
        ];
        let mut roots: Vec<PathBuf> = Vec::new();
        for root in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
            .into_iter()
            .filter_map(std::env::var_os)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .chain(std::iter::once(PathBuf::from("/tmp")))
        {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        let mut paths = Vec::new();
        for root in &roots {
            for place in PLACES {
                for index in 0..10 {
                    let path = root.join(place).join(format!("discord-ipc-{index}"));
                    if std::fs::metadata(&path).is_ok_and(|meta| meta.file_type().is_socket()) {
                        paths.push(path);
                    }
                }
            }
        }
        paths
    }

    #[cfg(windows)]
    fn candidates() -> Vec<std::path::PathBuf> {
        (0..10)
            .map(|index| std::path::PathBuf::from(format!(r"\\.\pipe\discord-ipc-{index}")))
            .collect()
    }

    #[cfg(not(any(unix, windows)))]
    fn candidates() -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    /// The connection itself. Reads wait for their bytes at most until a deadline.
    #[cfg(unix)]
    struct Pipe(std::os::unix::net::UnixStream);

    #[cfg(unix)]
    impl Pipe {
        fn open(path: &std::path::Path) -> io::Result<Self> {
            let stream = std::os::unix::net::UnixStream::connect(path)?;
            stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
            Ok(Pipe(stream))
        }

        fn read_within(&mut self, buffer: &mut [u8], deadline: Instant) -> io::Result<()> {
            let left = deadline.saturating_duration_since(Instant::now());
            match self
                .0
                .set_read_timeout(Some(left.max(Duration::from_millis(1))))
            {
                Ok(()) => {}
                // macOS refuses a new timeout (EINVAL) once Discord has closed its end, as it may
                // after a refusal, though its reply still waits to be read. Reading then cannot
                // block: it returns that reply or the end of the stream.
                Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
                Err(error) => return Err(error),
            }
            self.0.read_exact(buffer)
        }
    }

    /// A named pipe, read once it holds the bytes asked for: a synchronous read of a pipe has no
    /// timeout of its own.
    #[cfg(windows)]
    struct Pipe(std::fs::File);

    #[cfg(windows)]
    impl Pipe {
        fn open(path: &std::path::Path) -> io::Result<Self> {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map(Pipe)
        }

        fn read_within(&mut self, buffer: &mut [u8], deadline: Instant) -> io::Result<()> {
            use std::os::windows::io::AsRawHandle;
            loop {
                let mut available: u32 = 0;
                // SAFETY: the handle is the open pipe's, and only the count of bytes waiting is
                // asked for, into a local.
                let peeked = unsafe {
                    windows_sys::Win32::System::Pipes::PeekNamedPipe(
                        self.0.as_raw_handle(),
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut available,
                        std::ptr::null_mut(),
                    )
                };
                if peeked == 0 {
                    return Err(io::Error::last_os_error());
                }
                if available as usize >= buffer.len() {
                    return self.0.read_exact(buffer);
                }
                if Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    struct Pipe;

    #[cfg(not(any(unix, windows)))]
    impl Pipe {
        fn open(_path: &std::path::Path) -> io::Result<Self> {
            Err(io::ErrorKind::Unsupported.into())
        }

        fn read_within(&mut self, _buffer: &mut [u8], _deadline: Instant) -> io::Result<()> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    #[cfg(any(unix, windows))]
    impl Write for Pipe {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.write(data)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    #[cfg(not(any(unix, windows)))]
    impl Write for Pipe {
        fn write(&mut self, _data: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::Unsupported.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// The test's fake Discord listens on a Unix socket.
    #[cfg(all(test, unix))]
    pub(super) fn connect_to(
        path: &std::path::Path,
        client_id: &str,
    ) -> Result<Connection, String> {
        let pipe = Pipe::open(path).map_err(|error| error.to_string())?;
        let mut connection = Connection { pipe, nonce: 0 };
        connection.handshake(client_id)?;
        Ok(connection)
    }
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
    fn texts_are_fitted_to_discords_limits_in_utf16_units() {
        let units = |text: &str| text.encode_utf16().count();
        assert_eq!(units(&fit("A")), 2);
        assert_eq!(units(&fit("")), 2);
        assert_eq!(fit("Abc"), "Abc");
        assert_eq!(units(&fit(&"a".repeat(200))), MAX_TEXT_UNITS);
        assert!(fit(&"a".repeat(200)).ends_with('…'));
        // Letters outside the basic plane count twice, as in Discord.
        let wide = "𝓢".repeat(100);
        assert!(units(&fit(&wide)) <= MAX_TEXT_UNITS);
    }

    #[test]
    fn an_album_of_one_character_and_a_long_paused_state_stay_within_limits() {
        let presence = Presence {
            album: Some("÷".into()),
            artist: "x".repeat(140),
            paused: true,
            paused_label: "Pausiert".into(),
            artwork_url: Some(format!("https://example.invalid/{}", "c".repeat(400))),
            ..presence_fixture(Duration::from_secs(1))
        };
        let value = build_activity(&presence, 0);
        let units = |value: &Value| value.as_str().unwrap().encode_utf16().count();
        assert!(units(&value["assets"]["large_text"]) >= MIN_TEXT_UNITS);
        assert!(units(&value["state"]) <= MAX_TEXT_UNITS);
        assert!(value["assets"].get("large_image").is_none());
        assert_eq!(value["type"], 2);
    }

    #[cfg(unix)]
    #[test]
    fn the_client_skips_what_is_not_discord_and_hears_a_refusal() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;
        let folder = std::env::temp_dir().join(format!("crabytube-discord-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("discord-ipc-0");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut frame = || {
                let mut header = [0_u8; 8];
                stream.read_exact(&mut header).unwrap();
                let length = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
                let mut body = vec![0_u8; length];
                stream.read_exact(&mut body).unwrap();
                (
                    stream.try_clone().unwrap(),
                    serde_json::from_slice::<Value>(&body).unwrap(),
                )
            };
            let reply = |stream: &mut std::os::unix::net::UnixStream, value: Value| {
                let body = serde_json::to_vec(&value).unwrap();
                stream.write_all(&1_u32.to_le_bytes()).unwrap();
                stream
                    .write_all(&(body.len() as u32).to_le_bytes())
                    .unwrap();
                stream.write_all(&body).unwrap();
            };
            let (mut out, handshake) = frame();
            assert_eq!(handshake["client_id"], "test");
            reply(&mut out, json!({ "cmd": "DISPATCH", "evt": "READY" }));
            let (mut out, accepted) = frame();
            reply(
                &mut out,
                json!({ "cmd": "SET_ACTIVITY", "nonce": accepted["nonce"] }),
            );
            let (mut out, refused) = frame();
            reply(
                &mut out,
                json!({ "cmd": "SET_ACTIVITY", "evt": "ERROR", "nonce": refused["nonce"],
                        "data": { "code": 4000, "message": "child \"state\" fails" } }),
            );
        });
        let mut connection = ipc::connect_to(&path, "test").unwrap();
        assert!(
            connection
                .set_activity(Some(json!({ "details": "ok" })))
                .is_ok()
        );
        match connection.set_activity(Some(json!({ "details": "x" }))) {
            Err(ipc::Failure::Refused(reason)) => assert!(reason.contains("4000")),
            other => panic!("expected a refusal, got {other:?}"),
        }
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&folder);
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
