//! Versioned local state. No credentials, signed streams or raw service responses.
use crate::music::{BrowseTarget, Track};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const MAX_QUEUE_TRACKS: usize = 2_000;
pub const MAX_LOCAL_FAVORITES: usize = 10_000;
pub const MAX_RECENT_PLAYS: usize = 12;
pub const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;
const MAX_LYRICS_OFFSETS: usize = 500;
const MAX_LYRICS_OFFSET_VIDEO_ID_BYTES: usize = 256;
const MIN_WINDOW_WIDTH: f32 = 480.0;
const MIN_WINDOW_HEIGHT: f32 = 640.0;
const MAX_WINDOW_WIDTH: f32 = 7_680.0;
const MAX_WINDOW_HEIGHT: f32 = 4_320.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiFontFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiLanguage {
    #[default]
    English,
    Spanish,
}

impl UiLanguage {
    pub const ALL: [Self; 2] = [Self::English, Self::Spanish];
}

impl std::fmt::Display for UiLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::English => "English",
            Self::Spanish => "Español",
        })
    }
}

impl UiFontFamily {
    pub const ALL: [Self; 3] = [Self::SansSerif, Self::Serif, Self::Monospace];
}

impl std::fmt::Display for UiFontFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::SansSerif => "Sans serif",
            Self::Serif => "Serif",
            Self::Monospace => "Monospace",
        };
        f.write_str(crate::i18n::translate(crate::i18n::current_language(), label).as_ref())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiDensity {
    Compact,
    #[default]
    Comfortable,
}

impl UiDensity {
    pub const ALL: [Self; 2] = [Self::Comfortable, Self::Compact];

    pub const fn result_row_height(self) -> f32 {
        match self {
            Self::Comfortable => 64.0,
            Self::Compact => 48.0,
        }
    }

    pub const fn queue_row_pitch(self) -> f32 {
        match self {
            Self::Comfortable => 54.0,
            Self::Compact => 42.0,
        }
    }

    pub const fn library_row_pitch(self) -> f32 {
        match self {
            Self::Comfortable => 76.0,
            Self::Compact => 56.0,
        }
    }
}

impl std::fmt::Display for UiDensity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
        };
        f.write_str(crate::i18n::translate(crate::i18n::current_language(), label).as_ref())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub autoplay: bool,
    pub shuffle_all: bool,
    pub remember_queue: bool,
    pub remember_history: bool,
    pub reduced_motion: bool,
    /// Obsolete: no longer read or written; kept so older state files still load.
    #[serde(skip_serializing)]
    pub artwork_background: bool,
    /// Obsolete: no longer read or written; kept so older state files still load.
    #[serde(skip_serializing)]
    pub background_opacity: f32,
    /// Obsolete: no longer read or written; kept so older state files still load.
    #[serde(skip_serializing)]
    pub sidebar_collapsed: bool,
    pub window_width: f32,
    pub window_height: f32,
    /// Obsolete: no longer read or written; kept so older state files still load.
    #[serde(skip_serializing)]
    pub ui_font_family: UiFontFamily,
    pub ui_language: UiLanguage,
    pub ui_density: UiDensity,
    pub startup: bool,
    pub close_to_tray: bool,
    pub discord_presence: bool,
    pub audio_output_device_id: Option<String>,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// Renders with the GPU (wgpu) instead of the CPU renderer. Applied at startup.
    pub gpu_rendering: bool,
    /// Opens Now Playing on the Lyrics tab instead of Up Next.
    pub show_lyrics_default: bool,
    /// Requests live search suggestions while typing.
    pub search_suggestions: bool,
    /// Shows a desktop notification when a track starts while the window is in the background.
    pub track_notifications: bool,
    /// Scales the whole interface with the window so proportions stay the same.
    pub scale_with_window: bool,
    pub lyrics_offset_ms: i64,
    pub lyrics_font_size: f32,
    pub lyrics_show_translation: bool,
    pub lyrics_follow: bool,
    #[serde(default)]
    pub lyrics_offsets: std::collections::HashMap<String, i64>,
    /// Oldest adjusted video ID first; kept in sync with the bounded offsets map.
    #[serde(default)]
    pub lyrics_offset_order: Vec<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            autoplay: true,
            shuffle_all: false,
            remember_queue: true,
            remember_history: true,
            reduced_motion: false,
            artwork_background: true,
            background_opacity: 0.48,
            sidebar_collapsed: false,
            window_width: if cfg!(target_os = "linux") {
                1024.0
            } else {
                1440.0
            },
            window_height: if cfg!(target_os = "linux") {
                768.0
            } else {
                900.0
            },
            ui_font_family: UiFontFamily::default(),
            ui_language: UiLanguage::default(),
            ui_density: UiDensity::default(),
            startup: false,
            close_to_tray: false,
            discord_presence: false,
            audio_output_device_id: None,
            volume: 0.8,
            muted: false,
            shuffle: false,
            repeat: RepeatMode::Off,
            gpu_rendering: false,
            show_lyrics_default: false,
            search_suggestions: true,
            track_notifications: false,
            scale_with_window: true,
            lyrics_offset_ms: 0,
            lyrics_font_size: 16.0,
            lyrics_show_translation: true,
            lyrics_follow: true,
            lyrics_offsets: std::collections::HashMap::new(),
            lyrics_offset_order: Vec::new(),
        }
    }
}

impl Preferences {
    pub fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    pub fn remember_lyrics_offset(&mut self, video_id: String, offset_ms: i64) {
        if video_id.is_empty() || video_id.len() > MAX_LYRICS_OFFSET_VIDEO_ID_BYTES {
            return;
        }

        self.lyrics_offsets
            .insert(video_id.clone(), offset_ms.clamp(-5000, 5000));
        self.lyrics_offset_order
            .retain(|existing| existing != &video_id);
        self.lyrics_offset_order.push(video_id);

        while self.lyrics_offset_order.len() > MAX_LYRICS_OFFSETS {
            let oldest = self.lyrics_offset_order.remove(0);
            self.lyrics_offsets.remove(&oldest);
        }
    }

    fn normalize_lyrics_offsets(&mut self) {
        self.lyrics_offsets.retain(|video_id, _| {
            !video_id.is_empty() && video_id.len() <= MAX_LYRICS_OFFSET_VIDEO_ID_BYTES
        });
        for offset in self.lyrics_offsets.values_mut() {
            *offset = (*offset).clamp(-5000, 5000);
        }

        let mut seen = HashSet::with_capacity(self.lyrics_offsets.len());
        self.lyrics_offset_order.retain(|video_id| {
            self.lyrics_offsets.contains_key(video_id) && seen.insert(video_id.clone())
        });
        let mut untracked = self
            .lyrics_offsets
            .keys()
            .filter(|video_id| !seen.contains(*video_id))
            .cloned()
            .collect::<Vec<_>>();
        // Older snapshots have no order field; choose a deterministic baseline
        // and treat these entries as older than offsets adjusted in this version.
        untracked.sort_unstable();
        self.lyrics_offset_order.extend(untracked);

        let excess = self
            .lyrics_offset_order
            .len()
            .saturating_sub(MAX_LYRICS_OFFSETS);
        for oldest in self.lyrics_offset_order.drain(..excess) {
            self.lyrics_offsets.remove(&oldest);
        }
    }
}
/// Something the user started playing, listed under "Jump back in" on Home.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RecentPlay {
    Track(Track),
    Collection(RecentCollection),
}

/// An album or playlist the user started, kept with what is needed to open it again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecentCollection {
    pub title: String,
    pub browse: BrowseTarget,
    /// Tracks in the collection when it started playing.
    pub track_count: usize,
    pub thumbnail_url: Option<String>,
    /// Albums are labelled as such; other collections are labelled as playlists.
    #[serde(default)]
    pub album: bool,
}

impl RecentPlay {
    /// Identifies the entry for de-duplication: the video ID of a track, or the browse ID of a
    /// collection.
    pub fn key(&self) -> String {
        match self {
            Self::Track(track) => format!("track:{}", track.video_id),
            Self::Collection(collection) => format!("collection:{}", collection.browse.browse_id),
        }
    }

    pub fn thumbnail_url(&self) -> Option<&str> {
        match self {
            Self::Track(track) => track.thumbnail_url.as_deref(),
            Self::Collection(collection) => collection.thumbnail_url.as_deref(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub version: u32,
    pub preferences: Preferences,
    pub queue: Arc<Vec<Track>>,
    pub queue_index: Option<usize>,
    pub history: Arc<Vec<String>>,
    pub favorites: Arc<Vec<Track>>,
    /// Newest first, without repeats.
    pub recent: Arc<Vec<RecentPlay>>,
}
impl Snapshot {
    pub fn normalize(&mut self) {
        self.version = 1;
        self.preferences.volume = self.preferences.volume.clamp(0.0, 1.0);
        self.preferences.background_opacity = self.preferences.background_opacity.clamp(0.0, 0.75);
        self.preferences.window_width = self
            .preferences
            .window_width
            .clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH);
        self.preferences.window_height = self
            .preferences
            .window_height
            .clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT);
        self.preferences.lyrics_offset_ms = self.preferences.lyrics_offset_ms.clamp(-5000, 5000);
        self.preferences.lyrics_font_size = self.preferences.lyrics_font_size.clamp(12.0, 22.0);
        if self
            .preferences
            .audio_output_device_id
            .as_ref()
            .is_some_and(|id| id.len() > 1_024)
        {
            self.preferences.audio_output_device_id = None;
        }
        self.preferences.normalize_lyrics_offsets();
        Arc::make_mut(&mut self.queue).truncate(MAX_QUEUE_TRACKS);
        for track in Arc::make_mut(&mut self.queue)
            .iter_mut()
            .chain(Arc::make_mut(&mut self.favorites).iter_mut())
        {
            cap_track(track);
        }
        self.queue_index = self.queue_index.filter(|index| *index < self.queue.len());
        if !self.preferences.remember_queue {
            Arc::make_mut(&mut self.queue).clear();
            self.queue_index = None;
        }
        if !self.preferences.remember_history {
            Arc::make_mut(&mut self.history).clear();
            Arc::make_mut(&mut self.recent).clear();
        }
        Arc::make_mut(&mut self.history).truncate(30);
        let mut keys = HashSet::new();
        Arc::make_mut(&mut self.recent).retain(|play| keys.insert(play.key()));
        Arc::make_mut(&mut self.recent).truncate(MAX_RECENT_PLAYS);
        for play in Arc::make_mut(&mut self.recent).iter_mut() {
            match play {
                RecentPlay::Track(track) => cap_track(track),
                RecentPlay::Collection(collection) => {
                    cap_string(&mut collection.title, 512);
                    cap_string(&mut collection.browse.browse_id, 256);
                }
            }
        }
        for item in Arc::make_mut(&mut self.history).iter_mut() {
            cap_string(item, 256);
        }
        let mut seen = HashSet::new();
        Arc::make_mut(&mut self.favorites).retain(|track| seen.insert(track.video_id.clone()));
        Arc::make_mut(&mut self.favorites).truncate(MAX_LOCAL_FAVORITES);
    }
}
pub fn state_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("crabytube/state.json")
}
/// The state file as it was before the last write. It is read only when the state file is unreadable.
pub fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

pub fn load(path: &Path) -> Result<Snapshot, String> {
    if !path.exists() {
        return Ok(Snapshot::default());
    }
    match read_snapshot(path) {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => {
            let backup = backup_path(path);
            match backup.exists().then(|| read_snapshot(&backup)) {
                Some(Ok(snapshot)) => Ok(snapshot),
                _ => Err(error),
            }
        }
    }
}

fn read_snapshot(path: &Path) -> Result<Snapshot, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if data.len() > MAX_STATE_BYTES {
        return Err(format!(
            "Saved state exceeds the {} MiB limit",
            MAX_STATE_BYTES / 1024 / 1024
        ));
    }
    let mut snapshot: Snapshot = serde_json::from_slice(&data)
        .map_err(|e| format!("Saved settings could not be read: {e}"))?;
    snapshot.normalize();
    Ok(snapshot)
}
/// Moves an unreadable state file aside as `state.json.bad-<unix seconds>`, so the
/// app can start with defaults and keep saving without losing the original.
pub fn quarantine(path: &Path) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let target = path.with_extension(format!("json.bad-{stamp}"));
    std::fs::rename(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

pub fn write(path: &Path, value: &Snapshot) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing config directory")?;
    crate::storage::create_private_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    let mut value = value.clone();
    value.normalize();
    let data = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    if data.len() > MAX_STATE_BYTES {
        return Err(format!(
            "Local state exceeds the {} MiB limit",
            MAX_STATE_BYTES / 1024 / 1024
        ));
    }
    write_private_synced(&temp, &data)?;
    // The previous state becomes the backup that `load` reads when the state file is unreadable.
    if path.exists() {
        let _ = std::fs::copy(path, backup_path(path));
    }
    std::fs::rename(&temp, path).map_err(|e| e.to_string())?;
    // Makes the rename itself durable. Best effort: not every platform can sync a folder.
    if let Ok(folder) = std::fs::File::open(parent) {
        let _ = folder.sync_all();
    }
    Ok(())
}

/// Writes a file that only the owner can read (on Unix) and flushes it before it is renamed into place.
fn write_private_synced(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(data).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

fn cap_track(track: &mut Track) {
    cap_string(&mut track.video_id, 256);
    cap_string(&mut track.title, 512);
    cap_string(&mut track.artist, 1_024);
    if let Some(album) = &mut track.album {
        cap_string(album, 512);
    }
    if track
        .thumbnail_url
        .as_ref()
        .is_some_and(|url| url.len() > 2_048)
    {
        track.thumbnail_url = None;
    }
}

fn cap_string(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}
/// A single writer coalesces rapid UI edits and preserves ordering.
pub struct Persistence {
    pending: Arc<(Mutex<PendingSnapshot>, Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
}
#[derive(Default)]
struct PendingSnapshot {
    latest: Option<Snapshot>,
    closed: bool,
}
impl Persistence {
    pub fn start(path: PathBuf) -> Self {
        let pending = Arc::new((Mutex::new(PendingSnapshot::default()), Condvar::new()));
        let worker_pending = Arc::clone(&pending);
        let worker = std::thread::spawn(move || {
            loop {
                let (lock, wake) = &*worker_pending;
                let Ok(mut state) = lock.lock() else {
                    return;
                };
                while state.latest.is_none() && !state.closed {
                    let Ok(next) = wake.wait(state) else {
                        return;
                    };
                    state = next;
                }
                if state.latest.is_none() && state.closed {
                    break;
                }
                let Some(mut snapshot) = state.latest.take() else {
                    continue;
                };
                loop {
                    let Ok((next, timeout)) = wake.wait_timeout(state, Duration::from_millis(120))
                    else {
                        return;
                    };
                    state = next;
                    if let Some(latest) = state.latest.take() {
                        snapshot = latest;
                        continue;
                    }
                    if timeout.timed_out() || state.closed {
                        break;
                    }
                }
                snapshot.normalize();
                if let Err(error) = write(&path, &snapshot) {
                    tracing::warn!(%error,"Could not save local settings");
                }
            }
        });
        Self {
            pending,
            worker: Some(worker),
        }
    }
    pub fn save(&self, snapshot: Snapshot) {
        let (lock, wake) = &*self.pending;
        if let Ok(mut state) = lock.lock()
            && !state.closed
        {
            state.latest = Some(snapshot);
            wake.notify_one();
        }
    }
}
impl Drop for Persistence {
    fn drop(&mut self) {
        let (lock, wake) = &*self.pending;
        if let Ok(mut state) = lock.lock() {
            state.closed = true;
            wake.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
/// Register this installed desktop binary with the current user's session.
pub fn set_startup(enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let path = dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("autostart/crabytube.desktop");
        if !enabled {
            return match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            };
        }
        let program = autostart_program(
            std::env::var_os("APPIMAGE"),
            std::env::current_exe().map_err(|e| e.to_string())?,
        );
        let content = format!(
            "[Desktop Entry]\nType=Application\nName=CrabyTube\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            desktop_exec(&program.to_string_lossy())
        );
        let parent = path.parent().ok_or("Missing autostart directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temp = path.with_extension("desktop.tmp");
        std::fs::write(&temp, content).map_err(|e| e.to_string())?;
        std::fs::rename(temp, path).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = enabled;
        Err("Launch on startup is currently available on Linux only.".into())
    }
}

/// The program that starts at login. From an AppImage that is the image file: the executable inside its
/// mount disappears when the app exits.
#[cfg(target_os = "linux")]
fn autostart_program(
    appimage: Option<std::ffi::OsString>,
    exe: std::path::PathBuf,
) -> std::path::PathBuf {
    appimage
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or(exe)
}

/// The value of `Exec=` for `program`. The argument is quoted, with the characters the spec reserves
/// inside quotes escaped and `%` doubled so no field code is read from the path. The desktop file
/// format then escapes each backslash of the value.
#[cfg(target_os = "linux")]
fn desktop_exec(program: &str) -> String {
    let mut exec = String::from("\"");
    for c in program.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                exec.push('\\');
                exec.push(c);
            }
            '%' => exec.push_str("%%"),
            other => exec.push(other),
        }
    }
    exec.push('"');
    exec.replace('\\', "\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn an_exec_value_quotes_the_path_and_escapes_what_the_spec_reserves() {
        assert_eq!(
            desktop_exec("/opt/app/CrabyTube"),
            r#""/opt/app/CrabyTube""#
        );
        // A percent sign is a field code unless doubled.
        assert_eq!(desktop_exec("/x/100%/bin"), r#""/x/100%%/bin""#);
        // Inside quotes a dollar sign, a backtick, a quote and a backslash are escaped; the file
        // format then doubles the backslash that escape adds.
        assert_eq!(desktop_exec("/a$b"), r#""/a\\$b""#);
        assert_eq!(desktop_exec("/a\"b"), r#""/a\\"b""#);
        assert_eq!(desktop_exec("/a`b"), r#""/a\\`b""#);
        assert_eq!(desktop_exec(r"/a\b"), r#""/a\\\\b""#);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_appimage_registers_the_image_rather_than_its_mount() {
        let exe = std::path::PathBuf::from("/tmp/.mount_CrabyX/usr/bin/crabytube");
        let image = std::ffi::OsString::from("/home/user/CrabyTube.AppImage");
        assert_eq!(
            autostart_program(Some(image), exe.clone()),
            std::path::PathBuf::from("/home/user/CrabyTube.AppImage")
        );
        assert_eq!(autostart_program(None, exe.clone()), exe);
        assert_eq!(
            autostart_program(Some(std::ffi::OsString::new()), exe.clone()),
            exe
        );
    }

    #[test]
    fn saved_state_is_readable_only_by_the_owner_on_unix() {
        let dir = std::env::temp_dir().join(format!("crabytube-state-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");
        write(&path, &Snapshot::default()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_write_keeps_the_previous_state_as_backup_and_load_falls_back_to_it() {
        let dir =
            std::env::temp_dir().join(format!("crabytube-state-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");
        let first = Snapshot {
            history: vec!["first".into()].into(),
            ..Snapshot::default()
        };
        let second = Snapshot {
            history: vec!["second".into()].into(),
            ..Snapshot::default()
        };
        write(&path, &first).unwrap();
        write(&path, &second).unwrap();
        assert_eq!(*load(&path).unwrap().history, vec!["second".to_string()]);
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(*load(&path).unwrap().history, vec!["first".to_string()]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recent_plays_keep_the_newest_unique_entries_and_respect_history_privacy() {
        let track = |id: &str| Track {
            video_id: id.into(),
            title: "Title".into(),
            artist: "Artist".into(),
            album: None,
            thumbnail_url: None,
            duration_secs: None,
        };
        let collection = RecentCollection {
            title: "Mix".into(),
            browse: BrowseTarget {
                browse_id: "VLPL1".into(),
                params: None,
                page_type: None,
            },
            track_count: 3,
            thumbnail_url: None,
            album: false,
        };
        let mut s = Snapshot {
            recent: vec![
                RecentPlay::Track(track("a")),
                RecentPlay::Collection(collection),
                RecentPlay::Track(track("a")),
            ]
            .into(),
            ..Snapshot::default()
        };
        for index in 0..20 {
            Arc::make_mut(&mut s.recent).push(RecentPlay::Track(track(&format!("v{index}"))));
        }
        s.normalize();
        assert_eq!(s.recent.len(), MAX_RECENT_PLAYS);
        assert_eq!(s.recent[0].key(), "track:a");
        assert_eq!(s.recent[1].key(), "collection:VLPL1");
        assert_eq!(
            s.recent
                .iter()
                .filter(|play| play.key() == "track:a")
                .count(),
            1
        );
        s.preferences.remember_history = false;
        s.normalize();
        assert!(s.recent.is_empty());
    }

    #[test]
    fn restore_rejects_invalid_cursor_and_respects_privacy_toggles() {
        let mut s = Snapshot {
            queue_index: Some(10),
            history: vec!["query".into()].into(),
            ..Snapshot::default()
        };
        s.preferences.remember_history = false;
        s.normalize();
        assert!(s.queue_index.is_none());
        assert!(s.history.is_empty());
    }
    #[test]
    fn old_settings_receive_safe_defaults() {
        let p: Preferences = serde_json::from_str("{}").unwrap();
        assert!(p.remember_queue);
        assert_eq!(p.window_width, Preferences::default().window_width);
        assert_eq!(p.window_height, Preferences::default().window_height);
        assert_eq!(p.ui_language, UiLanguage::English);
        assert_eq!(p.ui_density, UiDensity::Comfortable);
    }

    #[test]
    fn obsolete_preference_keys_load_but_are_not_written_back() {
        let legacy: Preferences = serde_json::from_str(
            r#"{"quality":"High","sidebar_collapsed":true,"artwork_background":false,"background_opacity":0.3,"ui_font_family":"Monospace"}"#,
        )
        .unwrap();
        assert_eq!(legacy.background_opacity, 0.3);
        let written = String::from_utf8(serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(!written.contains("quality"));
        assert!(!written.contains("sidebar_collapsed"));
        assert!(!written.contains("artwork_background"));
        assert!(!written.contains("ui_font_family"));
    }

    #[test]
    fn ui_language_round_trips_and_legacy_preferences_default_to_english() {
        let preferences = Preferences {
            ui_language: UiLanguage::Spanish,
            ..Preferences::default()
        };
        let restored: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&preferences).unwrap()).unwrap();
        assert_eq!(restored.ui_language, UiLanguage::Spanish);

        let legacy: Preferences = serde_json::from_str(r#"{"ui_density":"Compact"}"#).unwrap();
        assert_eq!(legacy.ui_language, UiLanguage::English);
    }

    #[test]
    fn ui_density_round_trips_and_has_consistent_virtual_row_metrics() {
        let preferences = Preferences {
            ui_density: UiDensity::Compact,
            ..Preferences::default()
        };
        let restored: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&preferences).unwrap()).unwrap();
        assert_eq!(restored.ui_density, UiDensity::Compact);
        assert!(
            UiDensity::Compact.result_row_height() < UiDensity::Comfortable.result_row_height()
        );
        assert!(UiDensity::Compact.queue_row_pitch() < UiDensity::Comfortable.queue_row_pitch());
        assert!(
            UiDensity::Compact.library_row_pitch() < UiDensity::Comfortable.library_row_pitch()
        );
    }

    #[test]
    fn selected_output_device_round_trips_and_older_preferences_default_to_system() {
        let preferences = Preferences {
            audio_output_device_id: Some("alsa:device-123".into()),
            ..Preferences::default()
        };
        let restored: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&preferences).unwrap()).unwrap();
        assert_eq!(
            restored.audio_output_device_id.as_deref(),
            Some("alsa:device-123")
        );
        let older: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(older.audio_output_device_id, None);
    }

    #[test]
    fn effective_volume_preserves_volume_while_muted_and_restores_it_when_unmuted() {
        let preferences = Preferences {
            volume: 0.37,
            muted: true,
            ..Preferences::default()
        };
        assert_eq!(preferences.effective_volume(), 0.0);
        let unmuted = Preferences {
            muted: false,
            ..preferences
        };
        assert_eq!(unmuted.effective_volume(), 0.37);
    }

    #[test]
    fn per_song_lyrics_offsets_evict_only_the_least_recent_entry() {
        let mut preferences = Preferences::default();
        for index in 0..MAX_LYRICS_OFFSETS {
            preferences.remember_lyrics_offset(format!("track-{index:03}"), index as i64);
        }
        // Re-adjusting an existing song makes it recent without duplicating it.
        preferences.remember_lyrics_offset("track-000".into(), 7000);
        preferences.remember_lyrics_offset("track-new".into(), -7000);

        assert_eq!(preferences.lyrics_offsets.len(), MAX_LYRICS_OFFSETS);
        assert_eq!(preferences.lyrics_offset_order.len(), MAX_LYRICS_OFFSETS);
        assert!(!preferences.lyrics_offsets.contains_key("track-001"));
        assert_eq!(preferences.lyrics_offsets.get("track-000"), Some(&5000));
        assert_eq!(preferences.lyrics_offsets.get("track-new"), Some(&-5000));
        assert_eq!(preferences.lyrics_offset_order.last().unwrap(), "track-new");
        assert_eq!(
            preferences
                .lyrics_offset_order
                .iter()
                .filter(|video_id| *video_id == "track-000")
                .count(),
            1
        );

        preferences.remember_lyrics_offset("x".repeat(MAX_LYRICS_OFFSET_VIDEO_ID_BYTES + 1), 0);
        assert_eq!(preferences.lyrics_offsets.len(), MAX_LYRICS_OFFSETS);
    }

    #[test]
    fn legacy_and_oversized_lyrics_offset_maps_normalize_without_clearing_everything() {
        let mut snapshot: Snapshot = serde_json::from_value(serde_json::json!({
            "preferences": {
                "lyrics_offsets": {
                    "legacy-000": 9000,
                    "legacy-001": -9000,
                    "legacy-002": 250
                }
            }
        }))
        .unwrap();
        snapshot.normalize();
        assert_eq!(snapshot.preferences.lyrics_offsets.len(), 3);
        assert_eq!(snapshot.preferences.lyrics_offsets["legacy-000"], 5000);
        assert_eq!(snapshot.preferences.lyrics_offsets["legacy-001"], -5000);
        assert_eq!(snapshot.preferences.lyrics_offsets["legacy-002"], 250);
        assert_eq!(snapshot.preferences.lyrics_offset_order.len(), 3);

        snapshot.preferences.lyrics_offsets.clear();
        snapshot.preferences.lyrics_offset_order.clear();
        for index in 0..=MAX_LYRICS_OFFSETS {
            snapshot
                .preferences
                .lyrics_offsets
                .insert(format!("legacy-{index:03}"), index as i64);
        }
        snapshot
            .preferences
            .lyrics_offsets
            .insert("x".repeat(257), 42);
        snapshot.normalize();
        assert_eq!(
            snapshot.preferences.lyrics_offsets.len(),
            MAX_LYRICS_OFFSETS
        );
        assert_eq!(
            snapshot.preferences.lyrics_offset_order.len(),
            MAX_LYRICS_OFFSETS
        );
        assert!(
            !snapshot
                .preferences
                .lyrics_offsets
                .contains_key("legacy-000")
        );
        assert!(
            snapshot
                .preferences
                .lyrics_offsets
                .contains_key("legacy-500")
        );
        assert!(
            snapshot
                .preferences
                .lyrics_offsets
                .keys()
                .all(|video_id| video_id.len() <= MAX_LYRICS_OFFSET_VIDEO_ID_BYTES)
        );
    }

    #[test]
    fn snapshot_clamps_restored_window_geometry_to_supported_bounds() {
        let mut snapshot = Snapshot::default();
        snapshot.preferences.window_width = -1.0;
        snapshot.preferences.window_height = 99_999.0;
        snapshot.normalize();
        assert_eq!(snapshot.preferences.window_width, MIN_WINDOW_WIDTH);
        assert_eq!(snapshot.preferences.window_height, MAX_WINDOW_HEIGHT);
    }

    #[test]
    fn snapshot_enforces_explicit_track_count_and_metadata_limits() {
        let mut snapshot = Snapshot {
            queue: (0..MAX_QUEUE_TRACKS + 10)
                .map(|index| Track {
                    duration_secs: None,
                    video_id: format!("id-{index}"),
                    title: "é".repeat(300),
                    artist: "artist".into(),
                    album: None,
                    thumbnail_url: Some("x".repeat(3_000)),
                })
                .collect::<Vec<_>>()
                .into(),
            favorites: (0..MAX_LOCAL_FAVORITES + 10)
                .map(|index| Track {
                    duration_secs: None,
                    video_id: format!("fav-{index}"),
                    title: "favorite".into(),
                    artist: "artist".into(),
                    album: None,
                    thumbnail_url: None,
                })
                .collect::<Vec<_>>()
                .into(),
            ..Snapshot::default()
        };
        snapshot.normalize();
        assert_eq!(snapshot.queue.len(), MAX_QUEUE_TRACKS);
        assert_eq!(snapshot.favorites.len(), MAX_LOCAL_FAVORITES);
        assert!(snapshot.queue[0].title.len() <= 512);
        assert!(
            snapshot.queue[0]
                .title
                .is_char_boundary(snapshot.queue[0].title.len())
        );
        assert_eq!(snapshot.queue[0].thumbnail_url, None);
    }

    #[test]
    fn oversized_state_file_is_rejected_before_deserialization() {
        let path = std::env::temp_dir().join(format!(
            "crabytube-oversized-state-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, vec![b' '; MAX_STATE_BYTES + 1]).unwrap();
        let error = load(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(error.contains("16 MiB limit"));
    }
    #[test]
    fn state_file_from_a_build_with_google_sign_in_still_loads() {
        let path = std::env::temp_dir().join(format!(
            "crabytube-legacy-google-state-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{"version":1,"preferences":{"experimental_google":true,"volume":0.5},"history":["query"]}"#,
        )
        .unwrap();
        let snapshot = load(&path);
        let _ = std::fs::remove_file(&path);
        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot.preferences.volume, 0.5);
        assert_eq!(*snapshot.history, ["query"]);
        let written = String::from_utf8(serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert!(!written.contains("experimental_google"));
    }
    #[test]
    fn legacy_saved_tracks_without_album_metadata_still_load() {
        let track: Track = serde_json::from_str(
            r#"{"video_id":"old","title":"Song","artist":"Artist","thumbnail_url":null}"#,
        )
        .unwrap();
        assert_eq!(track.video_id, "old");
        assert_eq!(track.album, None);
    }
    #[test]
    fn round_trip_preserves_queue_without_autostart() {
        let mut s = Snapshot::default();
        Arc::make_mut(&mut s.queue).push(Track {
            duration_secs: None,
            video_id: "fixture".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: None,
            thumbnail_url: None,
        });
        s.queue_index = Some(0);
        let mut decoded: Snapshot =
            serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        decoded.normalize();
        assert_eq!(decoded.queue, s.queue);
        assert_eq!(decoded.queue_index, Some(0));
    }

    #[test]
    fn older_preferences_keep_normal_close_behavior_by_default() {
        let preferences: Preferences = serde_json::from_str("{}").unwrap();
        assert!(!preferences.close_to_tray);
    }

    #[test]
    fn unreadable_state_is_moved_aside_and_the_original_is_kept() {
        let dir =
            std::env::temp_dir().join(format!("crabytube-quarantine-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::fs::write(&path, b"{ not json").unwrap();

        assert!(load(&path).is_err());
        let moved = quarantine(&path).unwrap();

        assert!(!path.exists());
        assert!(moved.is_file());
        assert!(moved.to_string_lossy().contains("state.json.bad-"));
        assert_eq!(std::fs::read(&moved).unwrap(), b"{ not json");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persistence_coalesces_to_latest_snapshot_and_flushes_on_drop() {
        let path = std::env::temp_dir().join(format!(
            "crabytube-persistence-test-{}.json",
            std::process::id()
        ));
        let persistence = Persistence::start(path.clone());
        persistence.save(Snapshot {
            history: vec!["old".into()].into(),
            ..Snapshot::default()
        });
        persistence.save(Snapshot {
            history: vec!["latest".into()].into(),
            ..Snapshot::default()
        });
        drop(persistence);
        assert_eq!(*load(&path).unwrap().history, vec!["latest"]);
        let _ = std::fs::remove_file(path);
    }
}
