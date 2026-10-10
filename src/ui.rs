mod artwork_glue;
mod collection;
mod diagnostics;
mod favorites;
mod home;
mod home_panels;
mod layout;
mod messages;
mod now_playing;
mod playback_glue;
mod settings;
mod shell;
mod subscriptions;
mod theme;
#[cfg(feature = "diagnostics")]
mod timing;
use collection::CollectionState;

use std::time::Duration;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use crate::preferences::{Persistence, Preferences, RepeatMode, Snapshot};
use iced::widget::image;
use iced::{Element, Subscription, Task};
use rand::{Rng, seq::SliceRandom};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::music::{
    BrowseMore, BrowsePage, BrowseTarget, HomeItem, HomeItemKind, HomeShelf, MusicService, Track,
    YouTubeMusicService, YouTubeSession,
};
use crate::playback::{PlaybackHandle, PlaybackState, PlaybackStatus, YouTubeStreamResolver};
use diagnostics::*;
#[cfg(feature = "diagnostics")]
pub use diagnostics::{
    DIAGNOSTIC_NOT_PRESENTED, DiagnosticAutoplay, diagnostic_failure_code,
    enable_diagnostic_screenshots, enable_diagnostic_scroll, enable_diagnostic_scroll_page,
    enable_diagnostic_soak, set_diagnostic_autoplay,
};
use iced::widget::operation::{self, AbsoluteOffset};
use messages::{Message, NowPlayingTab, Page, Preference};

#[allow(dead_code)]
struct App {
    quality_setting: crate::playback_policy::QualitySetting,
    lyrics: Option<crate::lyrics::Lyrics>,
    lyrics_error: Option<String>,
    lyrics_loading: bool,
    lyrics_waiting_for_duration: bool,
    lyrics_generation: u64,
    lyrics_cancel: Option<CancellationToken>,
    lyric_active: Option<usize>,
    lyrics_follow_paused: bool,
    /// Set by a seek: the next lyric update scrolls to the active line even if it did not change.
    lyrics_resync: bool,
    /// When the playback status last arrived; the karaoke wait counts from it.
    playback_status_at: std::time::Instant,
    /// A karaoke wake-up is already on its way, so another one is not scheduled.
    karaoke_tick_pending: bool,
    lyrics_auto_scroll_until: Option<std::time::Instant>,
    lyrics_scroll: AbsoluteOffset,
    lyrics_heights: Vec<f32>,
    window_width: f32,
    window_height: f32,
    window_scale_factor: f32,
    /// Whether the window has focus; `None` until the first focus event.
    window_focused: Option<bool>,
    /// The track the last notification announced, and the notification it replaces.
    announced_track: Option<String>,
    track_notice_id: u32,
    window_geometry_generation: u64,
    storage_notice: Option<String>,
    storage_cache_usage: crate::storage::CacheUsage,
    // Disk I/O runs in a worker Task; these guard stale completions and
    // show a busy state instead of blocking `update`.
    storage_loading: bool,
    storage_generation: u64,
    output_devices: Vec<crate::playback::output::OutputDeviceChoice>,
    output_devices_loading: bool,
    output_devices_error: Option<String>,
    preferences: Preferences,
    persistence: Option<Persistence>,
    /// Bumped on each remote volume change; only the newest debounce timer saves.
    volume_save_generation: u64,
    /// Path the unreadable saved state was moved to at startup, shown in Storage.
    state_recovery: Option<String>,
    youtube_session: YouTubeSession,
    favorites: Vec<Track>,
    favorites_context: Arc<[Track]>,
    library_notice: Option<String>,
    page: Page,
    history: Vec<Page>,
    forward: Vec<Page>,
    collection: CollectionState,
    collection_cache: VecDeque<(BrowseTarget, Arc<crate::music::Collection>)>,
    /// Video ID of the last track whose successor was prefetched.
    prefetch_requested: Option<String>,
    collection_scroll: f32,
    home_scroll: AbsoluteOffset,
    search_scroll: AbsoluteOffset,
    library_scroll: AbsoluteOffset,
    queue_scroll: AbsoluteOffset,
    queue: Vec<Track>,
    queue_index: Option<usize>,
    queue_source: Option<QueueSource>,
    radio_track_ids: HashSet<String>,
    queue_notice: Option<String>,
    queue_generation: u64,
    liked: HashSet<String>,
    shuffle: bool,
    repeat: RepeatMode,
    autoplay: bool,
    now_playing_tab: NowPlayingTab,
    now_playing_return: Option<Page>,
    search_query: String,
    search_committed: bool,
    catalog_results: Vec<HomeItem>,
    catalog_continuation: Option<crate::music::Continuation>,
    catalog_more_loading: bool,
    catalog_more_error: Option<String>,
    radio_loading: bool,
    radio_attempted_for: Option<String>,
    radio_error: Option<String>,
    radio_generation: u64,
    radio_cancel: Option<CancellationToken>,
    /// Immutable search snapshot shared by the visible rows' play messages.
    search_results: Arc<[Track]>,
    search_error: Option<String>,
    search_loading: bool,
    search_request_id: u64,
    search_cancel: Option<CancellationToken>,
    search_history: Vec<String>,
    suggestions: Vec<String>,
    suggest_loading: bool,
    suggest_request_id: u64,
    suggest_cancel: Option<CancellationToken>,
    show_suggestions: bool,
    suggestion_index: Option<usize>,
    /// What the user typed before arrowing through suggestions; Escape restores it.
    typed_query: Option<String>,
    music_service: Arc<dyn MusicService>,
    playback: PlaybackHandle,
    discord_presence: crate::discord::DiscordPresence,
    mpris: crate::mpris::MprisRemote,
    tray: crate::tray::TrayRemote,
    tray_available: bool,
    tray_status: String,
    window_hidden_to_tray: bool,
    /// Set by our own minimize button, and by focus changes checked with the window.
    window_minimized: bool,
    /// The local time the Home greeting shows.
    clock: crate::local_clock::LocalClock,
    process_reading: Option<crate::process_stats::ProcessReading>,
    /// "Jump back in": tracks and collections the user started, newest first.
    recent: Vec<crate::preferences::RecentPlay>,
    /// The track whose start was last recorded in `recent`.
    recent_track_id: Option<String>,
    discord_presence_key: Option<(String, bool)>,
    discord_presence_position: Duration,
    discord_presence_updated_at: Option<std::time::Instant>,
    discord_presence_enabled_last: bool,
    playback_status: PlaybackStatus,
    seek_preview: Option<f32>,
    artwork: Option<image::Handle>,
    artwork_cache: crate::artwork::ArtworkCache,
    artwork_low_memory: bool,
    scroll_preload_pending: bool,
    resize_work_generation: u64,
    scroll_preload_timer: bool,
    #[cfg(feature = "diagnostics")]
    diagnostic_scroll_step: u32,
    artwork_requested: HashSet<(String, u32)>,
    /// Covers whose download failed on this visit to a page; see `artwork_glue`.
    artwork_failed: HashMap<(String, u32), artwork_glue::FailedCover>,
    /// Whether a timer is already set to reconcile failed covers.
    artwork_retry_pending: bool,
    /// Whether the player's cover has had its one retry.
    artwork_selected_retried: bool,
    /// Set while the "Copied" notice shows; the value is the number of the copy that set it.
    copy_notice: Option<u64>,
    copy_notice_count: u64,
    artwork_cancel: Option<CancellationToken>,
    // Per-URL in-flight tokens so scroll reconciliation can keep useful
    // work and cancel only URLs that left the window of interest.
    artwork_batch_tokens: HashMap<(String, u32), (u64, CancellationToken)>,
    artwork_request_id: u64,
    artwork_selected_request_id: u64,
    artwork_generation: u64,
    home_shelves: Vec<HomeShelf>,
    home_loading: bool,
    home_error: Option<String>,
    home_generation: u64,
    home_cancel: Option<CancellationToken>,
    home_more_cancel: Option<CancellationToken>,
    /// Quick Picks and collections derived from the Home feed.
    home_picks: home::HomePicks,
    /// Smooth scrolling of the Home carousel.
    carousel: home::Carousel,
    /// Size of the page area below the tabs, as laid out (see `shell_view`).
    area: iced::Size,
    /// Interface zoom that keeps proportions when the window grows (see `ui_zoom_for`).
    ui_zoom: f32,
    /// Window size in unzoomed logical pixels.
    os_window: (f32, f32),
    /// Until when newly arrived covers are still fading in.
    fade_until: Option<std::time::Instant>,
    /// Video ID whose ••• actions are open, if any.
    row_menu: Option<String>,
    /// Durations learned from playback, collections and search subtitles.
    known_durations: HashMap<String, Duration>,
}

fn cache_cleared_notice(kind: crate::storage::CacheKind, cleared: &Result<u64, String>) -> String {
    match cleared {
        Ok(0) => format!("{} is already empty.", kind.label()),
        Ok(bytes) => format!(
            "Cleared {} ({:.2} MiB removed).",
            kind.label(),
            *bytes as f64 / 1_048_576.0
        ),
        Err(error) => {
            warn!(cache = kind.label(), %error, "Could not clear cache");
            format!("Could not clear {}: {error}", kind.label())
        }
    }
}

/// Progress and the lyric highlight refresh this often while playing. Word-timed lyrics use the
/// same tick: at 100 ms the Now Playing view alone costs about 9 % of a core.
const PLAYBACK_TICK: Duration = Duration::from_millis(250);

fn should_close_to_tray(preference_enabled: bool, tray_available: bool) -> bool {
    preference_enabled && tray_available
}

impl Default for App {
    fn default() -> Self {
        let session = YouTubeSession::default();
        Self::new(
            Arc::new(YouTubeMusicService::with_session(session.clone())),
            session,
        )
    }
}

impl App {
    // All disk walks/deletes run in a worker Task. The sync
    // `disk_cache_usage` / `clear_cache` helpers stay in storage.rs for tests,
    // but `update` never calls them directly.
    fn refresh_storage_cache_usage_task(&mut self) -> Task<Message> {
        self.storage_generation = self.storage_generation.wrapping_add(1);
        let generation = self.storage_generation;
        self.storage_loading = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(crate::storage::disk_cache_usage)
                    .await
                    .unwrap_or_default()
            },
            move |usage| Message::StorageUsageLoaded { generation, usage },
        )
    }

    fn clear_cache_task(&mut self, kind: crate::storage::CacheKind) -> Task<Message> {
        self.storage_generation = self.storage_generation.wrapping_add(1);
        let generation = self.storage_generation;
        self.storage_loading = true;
        self.storage_notice = Some(format!("Working on {}…", kind.label()));
        Task::perform(
            async move {
                let cleared =
                    tokio::task::spawn_blocking(move || crate::storage::clear_cache(kind))
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| e.to_string()));
                let usage = tokio::task::spawn_blocking(crate::storage::disk_cache_usage)
                    .await
                    .unwrap_or_default();
                (cleared, usage)
            },
            move |(cleared_bytes, usage)| Message::StorageCleared {
                generation,
                kind,
                cleared_bytes,
                usage,
            },
        )
    }

    fn export_diagnostics_task(&mut self) -> Task<Message> {
        self.storage_generation = self.storage_generation.wrapping_add(1);
        let generation = self.storage_generation;
        self.storage_loading = true;
        self.storage_notice = Some("Preparing cache diagnostics…".into());
        let mut diagnostics = serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "artwork_cache_entries": self.artwork_cache.len(),
            "artwork_cache_bytes": self.artwork_cache.bytes(),
            "queue_len": self.queue.len(),
            "favorites": self.favorites.len(),
            "search_history_len": self.search_history.len(),
            "playback_state": format!("{:?}", self.playback_status.state),
            "audio_underrun_episodes": self.playback_status.underruns,
            "resolved_audio_format": self
                .playback_status
                .resolved_format
                .as_ref()
                .map(crate::playback::ResolvedAudioFormat::label),
            "quality": format!("{:?}", self.preferences.quality),
            "window": [self.window_width, self.window_height],
        });
        let path = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("crabytube/diagnostics.json");
        Task::perform(
            async move {
                let usage = tokio::task::spawn_blocking(crate::storage::disk_cache_usage)
                    .await
                    .unwrap_or_default();
                diagnostics["disk_cache_bytes"] = serde_json::json!({
                    "artwork": usage.artwork_bytes,
                    "lyrics": usage.lyrics_bytes,
                    "home_metadata": usage.home_bytes,
                    "player_js": usage.player_js_bytes,
                    "botguard_snapshots": usage.botguard_bytes,
                    "total": usage.total_bytes(),
                });
                diagnostics["disk_cache_limits_bytes"] = serde_json::json!({
                    "artwork": crate::storage::MAX_ARTWORK_DISK_BYTES,
                    "lyrics": crate::storage::MAX_LYRICS_DISK_BYTES,
                    "home_metadata": crate::storage::MAX_HOME_DISK_BYTES,
                    "player_js": crate::storage::MAX_PLAYER_JS_DISK_BYTES,
                    "botguard_snapshots": crate::storage::MAX_BOTGUARD_DISK_BYTES,
                });
                let write_path = path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let bytes = serde_json::to_vec_pretty(&diagnostics)
                        .map_err(|error| error.to_string())?;
                    if let Some(parent) = write_path.parent() {
                        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                    }
                    std::fs::write(&write_path, bytes).map_err(|error| error.to_string())?;
                    Ok::<_, String>(write_path.display().to_string())
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
                (result, usage)
            },
            move |(result, usage)| Message::DiagnosticsExported {
                generation,
                result,
                usage,
            },
        )
    }

    fn export_local_data_task(&mut self) -> Task<Message> {
        // Serialize + file write run in the worker; `update` stays responsive.
        self.storage_loading = true;
        self.storage_notice = Some("Exporting local data…".into());
        let snapshot = self.snapshot();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let mut snapshot = snapshot;
                    snapshot.normalize();
                    let bytes =
                        serde_json::to_vec_pretty(&snapshot).map_err(|error| error.to_string())?;
                    if bytes.len() > crate::preferences::MAX_STATE_BYTES {
                        return Err(format!(
                            "Local-data backup exceeds the {} MiB size limit.",
                            crate::preferences::MAX_STATE_BYTES / 1024 / 1024
                        ));
                    }
                    let dir = dirs::document_dir().ok_or("Documents folder is unavailable")?;
                    let path = dir.join("crabytube-backup.json");
                    // Atomic write via temp file in the same directory.
                    let tmp = dir.join(format!(
                        ".crabytube-backup-{}.tmp",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos())
                            .unwrap_or(0)
                    ));
                    std::fs::write(&tmp, &bytes).map_err(|error| error.to_string())?;
                    std::fs::rename(&tmp, &path).map_err(|error| error.to_string())?;
                    Ok::<_, String>(path.display().to_string())
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|r| r)
            },
            |result| Message::LocalDataExported { result },
        )
    }

    fn import_local_data_task(&mut self) -> Task<Message> {
        // Metadata + read + parse run in the worker; snapshot apply stays
        // on the UI thread via `LocalDataImported`.
        self.storage_loading = true;
        self.storage_notice = Some("Importing local data…".into());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(|| {
                    let dir = dirs::document_dir().ok_or("Documents folder is unavailable")?;
                    let path = dir.join("crabytube-backup.json");
                    let size = std::fs::metadata(&path)
                        .map_err(|error| error.to_string())?
                        .len();
                    if size > crate::preferences::MAX_STATE_BYTES as u64 {
                        return Err(format!(
                            "Local-data backup exceeds the {} MiB size limit.",
                            crate::preferences::MAX_STATE_BYTES / 1024 / 1024
                        ));
                    }
                    let data = std::fs::read(&path).map_err(|error| error.to_string())?;
                    let mut snapshot: crate::preferences::Snapshot =
                        serde_json::from_slice(&data).map_err(|error| error.to_string())?;
                    snapshot.normalize();
                    Ok::<_, String>(snapshot)
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|r| r)
            },
            |result| Message::LocalDataImported { result },
        )
    }

    fn refresh_output_devices_task(&mut self) -> Task<Message> {
        if self.output_devices_loading {
            return Task::none();
        }
        self.output_devices_loading = true;
        self.output_devices_error = None;
        Task::perform(
            async {
                tokio::task::spawn_blocking(crate::playback::output::enumerate_output_devices)
                    .await
                    .unwrap_or_else(|error| Err(error.to_string()))
            },
            Message::OutputDevicesLoaded,
        )
    }

    fn hide_window_to_tray_task(&mut self) -> Task<Message> {
        if !self.tray_available {
            return Task::none();
        }
        self.window_hidden_to_tray = true;
        iced::window::latest().and_then(|id| iced::window::set_mode(id, iced::window::Mode::Hidden))
    }

    fn show_window_from_tray_task(&mut self) -> Task<Message> {
        self.window_hidden_to_tray = false;
        iced::window::latest().and_then(move |id| {
            Task::batch([
                iced::window::set_mode(id, iced::window::Mode::Windowed),
                iced::window::gain_focus(id),
            ])
        })
    }

    fn close_window_task(&mut self) -> Task<Message> {
        if should_close_to_tray(self.preferences.close_to_tray, self.tray_available) {
            self.hide_window_to_tray_task()
        } else {
            iced::window::latest().and_then(iced::window::close)
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        #[cfg(feature = "diagnostics")]
        let (name, started) = (timing::message_name(&message), std::time::Instant::now());
        let task = self.update_inner(message);
        self.mpris.update(self.mpris_player_state());
        #[cfg(feature = "diagnostics")]
        timing::record_update(name, started.elapsed());
        task
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::StartDrag => {
                return iced::window::latest().and_then(iced::window::drag);
            }
            Message::StartResize(direction) => {
                return iced::window::latest()
                    .and_then(move |id| iced::window::drag_resize(id, direction));
            }
            Message::MinimizeWindow => {
                self.window_minimized = true;
                return iced::window::latest().and_then(|id| iced::window::minimize(id, true));
            }
            Message::ToggleMaximize => {
                return iced::window::latest().and_then(iced::window::toggle_maximize);
            }
            Message::CloseWindow => {
                return self.close_window_task();
            }
            Message::CloseRequested => {
                return self.close_window_task();
            }
            Message::QuitApp => {
                return iced::window::latest().and_then(iced::window::close);
            }

            Message::ShuffleContext(..)
            | Message::RetryRadio
            | Message::RadioLoaded { .. }
            | Message::MoveQueue(..)
            | Message::RemoveFromQueue(..)
            | Message::ClearUpcomingQueue
            | Message::SeekTo(..)
            | Message::ToggleShuffle
            | Message::CycleRepeat
            | Message::ToggleAutoplay
            | Message::TogglePlayback
            | Message::NextTrack
            | Message::PrevTrack
            | Message::PlayCollection(..)
            | Message::PlayContext(..)
            | Message::SetVolume(..)
            | Message::SeekChanged(..)
            | Message::SeekReleased
            | Message::PrewarmPlayback
            | Message::PlaybackTick
            | Message::PlaybackStatusChanged(..)
            | Message::MprisCommand(..)
            | Message::SelectTrack(..)
            | Message::PlayNext(..)
            | Message::AddToQueue(..) => return self.update_playback(message),
            Message::CatalogCompleted { request_id, result } => {
                if request_id != self.search_request_id {
                    return Task::none();
                }
                self.search_cancel = None;
                self.search_loading = false;
                match result {
                    Some(Ok(page)) => {
                        self.search_results = page
                            .items
                            .iter()
                            .filter_map(|item| item.track.clone())
                            .collect::<Vec<_>>()
                            .into();
                        self.catalog_results = page.items;
                        self.row_menu = None;
                        let learned: Vec<(String, Duration)> = self
                            .catalog_results
                            .iter()
                            .filter_map(|item| {
                                let track = item.track.as_ref()?;
                                let duration = item.duration.or_else(|| {
                                    parse_clock(home::subtitle_duration(&item.subtitle)?)
                                })?;
                                Some((track.video_id.clone(), duration))
                            })
                            .collect();
                        for (video_id, duration) in learned {
                            self.remember_duration(video_id, duration);
                        }
                        self.catalog_continuation = None;
                        self.catalog_more_loading = false;
                        self.catalog_more_error = None;
                        self.search_error = None;
                        self.search_scroll = AbsoluteOffset::default();
                        return self.preload_current_page_artwork();
                    }
                    Some(Err(e)) => self.search_error = Some(e),
                    None => {}
                }
            }

            Message::CatalogMoreCompleted { request_id, result } => {
                if request_id != self.search_request_id {
                    return Task::none();
                }
                self.search_cancel = None;
                self.search_loading = false;
                self.catalog_more_loading = false;
                match result {
                    Some(Ok(page)) => {
                        let previous_count = self.catalog_results.len();
                        let previous_token = self
                            .catalog_continuation
                            .as_ref()
                            .map(|continuation| continuation.token.clone());
                        let mut seen: HashSet<_> =
                            self.catalog_results.iter().map(catalog_item_key).collect();
                        self.catalog_results.extend(
                            page.items
                                .into_iter()
                                .filter(|item| seen.insert(catalog_item_key(item)))
                                .take(
                                    crate::music::MAX_CATALOG_ITEMS
                                        .saturating_sub(self.catalog_results.len()),
                                ),
                        );
                        self.catalog_continuation = page.continuation.filter(|continuation| {
                            self.catalog_results.len() < crate::music::MAX_CATALOG_ITEMS
                                && (self.catalog_results.len() > previous_count
                                    || Some(continuation.token.as_str())
                                        != previous_token.as_deref())
                        });
                        self.search_results = self
                            .catalog_results
                            .iter()
                            .filter_map(|item| item.track.clone())
                            .collect::<Vec<_>>()
                            .into();
                        self.catalog_more_error = None;
                        return self.preload_current_page_artwork();
                    }
                    Some(Err(error)) => self.catalog_more_error = Some(error),
                    None => {}
                }
            }

            Message::Resized { width, height } => {
                // Sizes arrive divided by the zoom in effect; undo it first.
                self.os_window = (width * self.ui_zoom, height * self.ui_zoom);
                self.apply_ui_zoom();
                let (width, height) = self.os_window;
                if width >= layout::MIN_WIDTH && height >= layout::MIN_HEIGHT {
                    self.preferences.window_width = width;
                    self.preferences.window_height = height;
                }
                return self.schedule_resize_work();
            }
            Message::ResizeWorkDue(generation) => {
                if generation == self.resize_work_generation {
                    // The layout settled: refit lyrics, fetch covers for the new
                    // sizes and save the geometry once.
                    self.rebuild_lyrics_heights();
                    self.persist();
                    return self.preload_current_page_artwork();
                }
            }
            Message::DisplayScaleChanged(scale_factor) => {
                let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
                    scale_factor.clamp(0.75, 4.0)
                } else {
                    1.0
                };
                if (self.window_scale_factor - scale_factor).abs() > f32::EPSILON {
                    self.window_scale_factor = scale_factor;
                    return self.preload_current_page_artwork();
                }
            }
            Message::PersistWindowGeometry(generation) => {
                if generation == self.window_geometry_generation {
                    self.persist();
                    return self.preload_current_page_artwork();
                }
            }
            Message::RestoreWindowSize => {
                let (width, height) = layout::default_window_size();
                self.preferences.window_width = width;
                self.preferences.window_height = height;
                self.window_width = width;
                self.window_height = height;
                self.window_geometry_generation = self.window_geometry_generation.wrapping_add(1);
                self.persist();
                return iced::window::latest()
                    .and_then(move |id| iced::window::resize(id, iced::Size::new(width, height)));
            }
            Message::RetryLyrics => {
                if let Some(t) = self.playback_status.track.clone() {
                    return self.load_lyrics(t);
                }
            }
            Message::LyricsLoaded { generation, result } => {
                if generation != self.lyrics_generation {
                    return Task::none();
                }
                self.lyrics_loading = false;
                self.lyrics_cancel = None;
                match result {
                    Ok(lyrics) => {
                        self.lyrics_error = None;
                        if !lyrics.lines.is_empty() {
                            self.lyrics_waiting_for_duration = false;
                        }
                        self.lyrics = Some(lyrics);
                        self.rebuild_lyrics_heights();
                    }
                    Err(error) => self.lyrics_error = Some(error),
                }
            }
            Message::LyricsProgress { generation, lyrics } => {
                if generation != self.lyrics_generation || !self.lyrics_loading {
                    return Task::none();
                }
                // Keep better timed results if a provider has already won.
                if self
                    .lyrics
                    .as_ref()
                    .is_some_and(|current| !current.lines.is_empty())
                {
                    return Task::none();
                }
                self.lyrics = Some(lyrics);
                self.rebuild_lyrics_heights();
            }

            Message::Preference(change) => {
                // Slider drags send many values; they save once on release instead.
                let continuous = matches!(
                    change,
                    Preference::LyricsOffset(_) | Preference::LyricsFontSize(_)
                );
                match change {
                    Preference::RestoreQueue(v) => self.preferences.remember_queue = v,
                    Preference::History(v) => {
                        self.preferences.remember_history = v;
                        if !v {
                            self.search_history.clear();
                            self.recent.clear();
                        }
                    }
                    Preference::ShuffleAll(v) => self.preferences.shuffle_all = v,
                    Preference::ReducedMotion(v) => self.preferences.reduced_motion = v,
                    Preference::Startup(v) => {
                        self.preferences.startup = v;
                        if let Err(error) = crate::preferences::set_startup(v) {
                            self.preferences.startup = !v;
                            warn!(%error,"Could not update startup preference");
                        }
                    }
                    Preference::CloseToTray(v) => self.preferences.close_to_tray = v,
                    Preference::Autoplay(v) => {
                        self.autoplay = v;
                        self.preferences.autoplay = v;
                        if !v {
                            if let Some(token) = self.radio_cancel.take() {
                                token.cancel();
                            }
                            self.radio_generation = self.radio_generation.wrapping_add(1);
                            self.radio_loading = false;
                            self.radio_attempted_for = None;
                        }
                    }
                    Preference::DiscordPresence(v) => {
                        self.preferences.discord_presence = v;
                        self.sync_discord_presence(true);
                    }
                    Preference::Quality(v) => {
                        self.preferences.quality = v;
                        self.quality_setting.set(v);
                    }
                    Preference::OutputDevice(device_id) => {
                        self.preferences.audio_output_device_id = device_id.clone();
                        self.playback.set_output_device(device_id);
                    }
                    Preference::GpuRendering(v) => {
                        self.preferences.gpu_rendering = v;
                    }
                    Preference::Muted(v) => {
                        self.preferences.muted = v;
                        self.playback
                            .set_volume(self.preferences.effective_volume());
                    }
                    Preference::LyricsOffset(v) => {
                        self.preferences.lyrics_offset_ms = v.clamp(-5000, 5000);
                    }
                    Preference::SongLyricsOffset(v) => {
                        if let Some(track) = self.playback_status.track.clone() {
                            self.preferences
                                .remember_lyrics_offset(track.video_id, v.clamp(-5000, 5000));
                        }
                    }
                    Preference::LyricsFontSize(v) => {
                        self.preferences.lyrics_font_size = v.clamp(12.0, 26.0);
                        self.rebuild_lyrics_heights();
                    }
                    Preference::LyricsTranslation(v) => {
                        self.preferences.lyrics_show_translation = v;
                        self.rebuild_lyrics_heights();
                    }
                    Preference::LyricsFollow(v) => {
                        self.preferences.lyrics_follow = v;
                        if v {
                            self.lyrics_follow_paused = false;
                        }
                    }
                    Preference::ShowLyricsDefault(v) => self.preferences.show_lyrics_default = v,
                    Preference::TrackNotifications(v) => self.preferences.track_notifications = v,
                    Preference::ScaleWithWindow(v) => {
                        self.preferences.scale_with_window = v;
                        self.apply_ui_zoom();
                    }
                    Preference::SearchSuggestions(v) => {
                        self.preferences.search_suggestions = v;
                        if !v {
                            self.invalidate_suggestions();
                        }
                    }
                    Preference::UiLanguage(v) => {
                        let changed = self.preferences.ui_language != v;
                        self.preferences.ui_language = v;
                        crate::i18n::set_current_language(v);
                        if changed {
                            // Catalog text follows the app language: show that
                            // language's saved Home at once, then refresh it.
                            self.persist();
                            self.home_shelves = crate::home_cache::load().unwrap_or_default();
                            self.home_picks.rebuild(&self.home_shelves);
                            self.search_results = Arc::from([]);
                            self.catalog_results.clear();
                            self.search_committed = false;
                            return Task::batch([
                                self.load_home_task(),
                                self.preload_current_page_artwork(),
                            ]);
                        }
                    }
                    Preference::UiDensity(v) => {
                        let previous = self.preferences.ui_density;
                        if previous != v {
                            self.search_scroll.y *=
                                v.result_row_height() / previous.result_row_height();
                            self.library_scroll.y *=
                                v.library_row_pitch() / previous.library_row_pitch();
                            self.queue_scroll.y *= v.queue_row_pitch() / previous.queue_row_pitch();
                        }
                        self.preferences.ui_density = v;
                    }
                }
                if !continuous {
                    self.persist();
                }
            }
            Message::ClearHistory => {
                self.search_history.clear();
                self.recent.clear();
                self.persist();
            }

            Message::Navigate(page) => {
                if matches!(&page, Page::Settings) {
                    let storage_task = self.refresh_storage_cache_usage_task();
                    let devices_task = if self.output_devices_loading {
                        Task::none()
                    } else {
                        self.refresh_output_devices_task()
                    };
                    let navigation = self.navigate(page);
                    return Task::batch([navigation, storage_task, devices_task]);
                }
                return self.navigate(page);
            }
            Message::OpenNowPlaying => {
                if self.playback_status.track.is_some() {
                    if self.preferences.show_lyrics_default
                        && !matches!(self.page, Page::NowPlaying)
                    {
                        self.now_playing_tab = NowPlayingTab::Lyrics;
                        if self.lyrics.is_none()
                            && !self.lyrics_loading
                            && let Some(track) = self.playback_status.track.clone()
                        {
                            let lyrics = self.load_lyrics(track);
                            if !matches!(self.page, Page::NowPlaying) {
                                self.now_playing_return = Some(self.page.clone());
                            }
                            let navigation = self.navigate_internal(Page::NowPlaying, false);
                            return Task::batch([navigation, lyrics]);
                        }
                    }
                    if !matches!(self.page, Page::NowPlaying) {
                        self.now_playing_return = Some(self.page.clone());
                    }
                    return self.navigate_internal(Page::NowPlaying, false);
                }
            }
            Message::CloseNowPlaying => {
                if let Some(previous) = self.now_playing_return.take() {
                    if self.history.last() == Some(&previous) {
                        self.history.pop();
                    }
                    let task = self.navigate_internal(previous, false);
                    return task;
                } else {
                    let task = self.navigate_internal(Page::Home, false);
                    return task;
                }
            }
            Message::ToggleLike(video_id) => {
                if !self.liked.contains(&video_id)
                    && self.favorites.len() >= crate::preferences::MAX_LOCAL_FAVORITES
                {
                    self.library_notice = Some(format!(
                        "Local favorites are limited to {} tracks.",
                        crate::preferences::MAX_LOCAL_FAVORITES
                    ));
                    return Task::none();
                }
                self.library_notice = None;
                if self.liked.contains(&video_id) {
                    self.liked.remove(&video_id);
                    self.favorites.retain(|t| t.video_id != video_id);
                    self.favorites_context = Arc::from(self.favorites.clone());
                } else {
                    let track = self
                        .queue
                        .iter()
                        .chain(self.search_results.iter())
                        .find(|t| t.video_id == video_id)
                        .cloned();
                    if let Some(track) = track {
                        self.favorites.push(track);
                        self.favorites_context = Arc::from(self.favorites.clone());
                        self.liked.insert(video_id);
                    }
                }
                self.persist();
            }
            Message::SetNowPlayingTab(tab) => {
                self.now_playing_tab = tab;
                if tab == NowPlayingTab::Queue {
                    return self.preload_current_page_artwork();
                }
                if tab == NowPlayingTab::Lyrics
                    && self.lyrics.is_none()
                    && !self.lyrics_loading
                    && let Some(track) = self.playback_status.track.clone()
                {
                    return self.load_lyrics(track);
                }
            }
            Message::OpenCollection(target) => {
                if !target.is_public_collection() {
                    return Task::none();
                }
                return self.navigate(Page::CollectionDetail(target));
            }
            Message::Back => {
                if let Some(page) = self.history.pop() {
                    self.forward.push(self.page.clone());
                    return self.navigate_internal(page, false);
                }
            }
            Message::Forward => {
                if let Some(page) = self.forward.pop() {
                    self.history.push(self.page.clone());
                    if self.history.len() > 40 {
                        let excess = self.history.len() - 40;
                        self.history.drain(0..excess);
                    }
                    return self.navigate_internal(page, false);
                }
            }
            Message::RemoveSearchHistoryItem(item) => {
                self.search_history.retain(|past| past != &item);
                self.persist();
            }
            Message::ClearArtworkCache => {
                self.artwork_cache.clear();
                self.artwork_failed.clear();
                self.cancel_artwork_batch();
                self.artwork_generation = self.artwork_generation.wrapping_add(1);
                self.artwork = None;
                return self.clear_cache_task(crate::storage::CacheKind::Artwork);
            }
            Message::ClearLyricsCache => {
                return self.clear_cache_task(crate::storage::CacheKind::Lyrics);
            }
            Message::ClearHomeCache => {
                return self.clear_cache_task(crate::storage::CacheKind::Home);
            }
            Message::ClearPlayerJsCache => {
                return self.clear_cache_task(crate::storage::CacheKind::PlayerJs);
            }
            Message::ClearBotGuardCache => {
                return self.clear_cache_task(crate::storage::CacheKind::BotGuard);
            }
            Message::RefreshCacheUsage => {
                self.storage_notice = Some("Refreshing cache sizes…".into());
                return self.refresh_storage_cache_usage_task();
            }
            Message::StorageUsageLoaded { generation, usage } => {
                if generation != self.storage_generation {
                    return Task::none();
                }
                self.storage_loading = false;
                self.storage_cache_usage = usage;
                if self.storage_notice.as_deref() == Some("Refreshing cache sizes…")
                    || self.storage_notice.as_deref() == Some("Cache sizes refreshed.")
                {
                    self.storage_notice = Some("Cache sizes refreshed.".into());
                }
            }
            Message::StorageCleared {
                generation,
                kind,
                cleared_bytes,
                usage,
            } => {
                if generation != self.storage_generation {
                    return Task::none();
                }
                self.storage_loading = false;
                self.storage_cache_usage = usage;
                self.storage_notice = Some(cache_cleared_notice(kind, &cleared_bytes));
            }
            Message::DiagnosticsExported {
                generation,
                result,
                usage,
            } => {
                if generation != self.storage_generation {
                    return Task::none();
                }
                self.storage_loading = false;
                self.storage_cache_usage = usage;
                self.storage_notice = Some(match result {
                    Ok(path) => format!("Diagnostics exported to {path}"),
                    Err(error) => format!("Diagnostics export failed: {error}"),
                });
            }
            Message::LocalDataExported { result } => {
                self.storage_loading = false;
                self.storage_notice = Some(match result {
                    Ok(path) => format!("Local data exported to {path}."),
                    Err(error) => format!("Local-data export failed: {error}"),
                });
            }
            Message::LocalDataImported { result } => match result {
                Ok(snapshot) => {
                    self.storage_loading = false;
                    self.storage_notice = Some("Local-data backup imported.".into());
                    return self.apply_snapshot(snapshot);
                }
                Err(error) => {
                    self.storage_loading = false;
                    self.storage_notice = Some(format!("Local-data import failed: {error}"));
                }
            },
            Message::RefreshOutputDevices => return self.refresh_output_devices_task(),
            Message::OutputDevicesLoaded(result) => {
                self.output_devices_loading = false;
                match result {
                    Ok(devices) => {
                        self.output_devices = devices;
                        self.output_devices_error = None;
                    }
                    Err(error) => self.output_devices_error = Some(error),
                }
            }
            Message::ExportDiagnostics => {
                return self.export_diagnostics_task();
            }
            Message::ExportLocalData => {
                return self.export_local_data_task();
            }
            Message::ImportLocalData => {
                return self.import_local_data_task();
            }
            Message::LyricsScrolled(offset) => {
                self.lyrics_scroll = offset;
                // Manual exploration suspends autoscroll; offer return.
                let programmatic = self
                    .lyrics_auto_scroll_until
                    .is_some_and(|until| std::time::Instant::now() < until);
                if !programmatic && self.preferences.lyrics_follow && self.lyric_active.is_some() {
                    self.lyrics_follow_paused = true;
                }
            }
            Message::ResumeLyricsFollow => {
                self.lyrics_follow_paused = false;
                if let Some(index) = self.lyric_active {
                    self.lyrics_auto_scroll_until =
                        Some(std::time::Instant::now() + Duration::from_millis(250));
                    return operation::scroll_to(
                        "lyrics",
                        AbsoluteOffset {
                            x: 0.0,
                            y: self.lyrics_scroll_offset(index),
                        },
                    );
                }
            }
            Message::AdjustLyricsOffset(delta) => {
                let next = (self.current_lyrics_offset() + delta).clamp(-5000, 5000);
                return self.update(Message::Preference(Preference::SongLyricsOffset(next)));
            }
            Message::RetryCollection => {
                if let Page::CollectionDetail(target) = &self.page {
                    return self.load_collection(target.clone());
                }
            }
            Message::RetryCollectionMore => {
                self.collection.more_error = None;
                return self.load_collection_more();
            }
            Message::CollectionLoaded { generation, result } => {
                if self.collection.complete(generation, result) {
                    if let (Some(target), Some(data)) =
                        (self.collection.target.clone(), self.collection.data.clone())
                    {
                        self.collection_cache
                            .retain(|(cached, _)| cached != &target);
                        self.collection_cache.push_front((target, data));
                        self.collection_cache.truncate(MAX_CACHED_COLLECTIONS);
                    }
                    if let Some(data) = &self.collection.data {
                        tracing::debug!(
                            tracks = data.tracks.len(),
                            has_more = data.continuation.is_some(),
                            "collection loaded"
                        );
                    }
                    return self.preload_current_page_artwork();
                }
            }
            Message::CollectionMoreLoaded { generation, result } => {
                if generation != self.collection.generation {
                    return Task::none();
                }
                self.collection.more_loading = false;
                if let Err(error) = &result {
                    self.collection.more_error = Some(error.clone());
                }
                if let (Some(data), Ok(more)) =
                    (self.collection.data.as_mut().map(Arc::make_mut), result)
                {
                    let before = data.tracks.len();
                    let previous_continuation = data.continuation.clone();
                    let mut seen: HashSet<_> = data
                        .tracks
                        .iter()
                        .map(|entry| entry.track.video_id.clone())
                        .collect();
                    let remaining_tracks =
                        crate::music::MAX_COLLECTION_TRACKS.saturating_sub(data.tracks.len());
                    data.tracks.extend(
                        more.tracks
                            .into_iter()
                            .filter(|entry| seen.insert(entry.track.video_id.clone()))
                            .take(remaining_tracks),
                    );
                    data.continuation = if data.tracks.len() >= crate::music::MAX_COLLECTION_TRACKS
                        || (data.tracks.len() == before
                            && more.continuation == previous_continuation)
                    {
                        None
                    } else {
                        more.continuation
                    };
                    self.collection.tracks_context = data
                        .tracks
                        .iter()
                        .map(|entry| entry.track.clone())
                        .collect::<Vec<_>>()
                        .into();
                    if let (Some(target), Some(shared)) =
                        (self.collection.target.clone(), self.collection.data.clone())
                    {
                        self.collection_cache
                            .retain(|(cached, _)| cached != &target);
                        self.collection_cache.push_front((target, shared));
                        self.collection_cache.truncate(MAX_CACHED_COLLECTIONS);
                    }
                    return self.preload_current_page_artwork();
                }
            }
            Message::CollectionScrolled(y) => {
                self.collection_scroll = y;
                let rows_top = self.collection_rows_offset();
                let pitch = layout::COLLECTION_ROW_PITCH;
                let artwork = self.scroll_preload_task();
                let more = if self.collection.data.as_ref().is_some_and(|data| {
                    data.continuation.is_some()
                        && y > (rows_top + data.tracks.len() as f32 * pitch
                            - (self.window_height - 300.0))
                            .max(0.0)
                }) {
                    self.load_collection_more()
                } else {
                    Task::none()
                };
                return Task::batch([artwork, more]);
            }
            Message::ScrollPreloadDue
            | Message::ArtworkRetryDue
            | Message::ArtworkLoaded { .. }
            | Message::FadeFrame(..) => {
                return self.update_artwork(message);
            }
            Message::HomeScrolled(offset) => {
                self.home_scroll = offset;
                return self.scroll_preload_task();
            }
            Message::SearchScrolled(offset) => {
                self.search_scroll = offset;
                return self.scroll_preload_task();
            }
            Message::LibraryScrolled(offset) => {
                self.library_scroll = offset;
                return self.scroll_preload_task();
            }
            Message::QueueScrolled(offset) => {
                self.queue_scroll = offset;
                return self.scroll_preload_task();
            }
            Message::PersistState => self.persist(),
            Message::VolumeSaveDue(generation) => {
                if generation == self.volume_save_generation {
                    self.persist();
                }
                return Task::none();
            }
            #[cfg(feature = "diagnostics")]
            Message::DiagnosticScrollStep
            | Message::DiagnosticShot(..)
            | Message::DiagnosticChurn(..)
            | Message::DiagnosticSoak(..)
            | Message::DiagnosticCapture(..)
            | Message::DiagnosticCaptured(..)
            | Message::DiagnosticFrame(..) => return self.update_diagnostics(message),
            Message::TrayServiceStarted(Ok(available)) => {
                self.tray_available = available;
                self.tray_status = if available {
                    "System tray is active.".into()
                } else {
                    "Waiting for a compatible system tray host.".into()
                };
            }
            Message::TrayServiceStarted(Err(error)) => {
                self.tray_available = false;
                self.tray_status = format!("System tray unavailable: {error}");
            }
            Message::TrayCommand(command) => match command {
                crate::tray::TrayCommand::ShowWindow => {
                    return self.show_window_from_tray_task();
                }
                crate::tray::TrayCommand::HideWindow => {
                    return self.hide_window_to_tray_task();
                }
                crate::tray::TrayCommand::TogglePlayback => {
                    return self.update(Message::TogglePlayback);
                }
                crate::tray::TrayCommand::NextTrack => {
                    return self.update(Message::NextTrack);
                }
                crate::tray::TrayCommand::PreviousTrack => {
                    return self.update(Message::PrevTrack);
                }
                crate::tray::TrayCommand::Quit => {
                    return self.update(Message::QuitApp);
                }
                crate::tray::TrayCommand::AvailabilityChanged(available) => {
                    self.tray_available = available;
                    self.tray_status = if available {
                        "System tray is active.".into()
                    } else {
                        "System tray host is offline; closing the window will quit CrabyTube."
                            .into()
                    };
                    if !available && self.window_hidden_to_tray {
                        return self.show_window_from_tray_task();
                    }
                }
            },
            Message::SearchChanged(query) => {
                // Typing again leaves suggestion browsing.
                self.typed_query = None;
                self.suggestion_index = None;
                if let Some(token) = self.search_cancel.take() {
                    token.cancel();
                }
                self.search_request_id = self.search_request_id.wrapping_add(1);
                self.search_loading = false;
                self.search_committed = false;
                self.search_error = None;
                // Preserve previous results while typing; the new query only
                // starts suggestions. Results are replaced on submit.
                // Category reset would discard the user's filter context;
                // keep it until a new search commits.
                if !self.preferences.search_suggestions {
                    self.search_query = query;
                    return Task::none();
                }
                return self.start_suggestions(query);
            }
            Message::SearchSubmitted => {
                self.invalidate_suggestions();
                self.search_committed = !self.search_query.trim().is_empty();
                self.push_history(self.search_query.clone());
                let navigation = self.navigate(Page::Search);
                let search = self.start_search(self.search_query.clone(), false);
                return Task::batch([navigation, search]);
            }
            Message::CopyToClipboard(text) => {
                self.copy_notice_count = self.copy_notice_count.wrapping_add(1);
                let count = self.copy_notice_count;
                self.copy_notice = Some(count);
                return Task::batch([
                    iced::clipboard::write(text),
                    Task::perform(
                        async { tokio::time::sleep(Duration::from_millis(1500)).await },
                        move |()| Message::CopyNoticeExpired(count),
                    ),
                ]);
            }
            Message::CopyNoticeExpired(count) => {
                if self.copy_notice == Some(count) {
                    self.copy_notice = None;
                }
            }
            Message::SearchFor(query) => {
                self.invalidate_suggestions();
                self.search_committed = !query.trim().is_empty();
                self.push_history(query.clone());
                let navigation = self.navigate(Page::Search);
                let search = self.start_search(query, false);
                return Task::batch([navigation, search]);
            }
            Message::LoadMoreCatalog => return self.load_more_catalog_task(),
            Message::SuggestionChosen(suggestion) => {
                self.invalidate_suggestions();
                self.search_query = suggestion.clone();
                self.search_committed = true;
                self.push_history(suggestion.clone());
                let navigation = self.navigate(Page::Search);
                let search = self.start_search(suggestion, false);
                return Task::batch([navigation, search]);
            }
            Message::DismissSuggestions => {
                self.invalidate_suggestions();
            }
            Message::KeyPressed(event) => {
                use iced::keyboard::{Event as KeyEvent, Key, key::Named};
                if let KeyEvent::KeyPressed {
                    key,
                    modifiers,
                    physical_key,
                    repeat,
                    ..
                } = event
                {
                    // A held key repeats. Only the arrows that move through suggestions follow the repeats;
                    // every other shortcut acts once per press.
                    let suggestion_arrow =
                        matches!(key, Key::Named(Named::ArrowDown | Named::ArrowUp));
                    if repeat && !(suggestion_arrow && self.show_suggestions) {
                        return Task::none();
                    }
                    // Escape priority: suggestions > row menu > Now Playing > Settings.
                    if matches!(key, Key::Named(Named::Escape)) {
                        if self.show_suggestions {
                            if let Some(typed) = self.typed_query.take() {
                                self.search_query = typed;
                            }
                            self.invalidate_suggestions();
                        } else if self.row_menu.is_some() {
                            self.row_menu = None;
                        } else if matches!(self.page, Page::NowPlaying) {
                            return self.update(Message::CloseNowPlaying);
                        } else if matches!(self.page, Page::Settings | Page::CollectionDetail(_)) {
                            return self.update(Message::Back);
                        }
                        return Task::none();
                    }
                    if self.show_suggestions && !self.suggestions.is_empty() {
                        if matches!(key, Key::Named(Named::ArrowDown)) {
                            let next = match self.suggestion_index {
                                None => 0,
                                Some(i) => (i + 1) % self.suggestions.len(),
                            };
                            return self.highlight_suggestion(next);
                        }
                        if matches!(key, Key::Named(Named::ArrowUp)) {
                            let prev = match self.suggestion_index {
                                None | Some(0) => self.suggestions.len().saturating_sub(1),
                                Some(i) => i - 1,
                            };
                            return self.highlight_suggestion(prev);
                        }
                    }
                    // Matched by the physical key, so the shortcut works on any keyboard layout.
                    if key.to_latin(physical_key) == Some('k')
                        && (modifiers.control() || modifiers.command())
                    {
                        let focus = operation::focus("search-field");
                        if matches!(self.page, Page::Home | Page::Search) {
                            return focus;
                        }
                        return Task::batch([self.navigate(Page::Home), focus]);
                    }
                    // Global playback keys; text fields capture their own keys first.
                    if modifiers.is_empty() || modifiers.shift() {
                        match key {
                            Key::Named(Named::Space) => {
                                return self.update(Message::TogglePlayback);
                            }
                            Key::Named(Named::ArrowRight) if modifiers.shift() => {
                                return self.update(Message::NextTrack);
                            }
                            Key::Named(Named::ArrowLeft) if modifiers.shift() => {
                                return self.update(Message::PrevTrack);
                            }
                            Key::Named(Named::ArrowRight | Named::ArrowLeft) => {
                                let step = if matches!(key, Key::Named(Named::ArrowRight)) {
                                    5.0
                                } else {
                                    -5.0
                                };
                                let position = self.playback_status.position.as_secs_f64();
                                let duration = self
                                    .playback_status
                                    .duration
                                    .map_or(position, |duration| duration.as_secs_f64());
                                if self.playback_status.track.is_some() {
                                    self.seek_playback(Duration::from_secs_f64(
                                        (position + step).clamp(0.0, duration.max(0.0)),
                                    ));
                                }
                                return Task::none();
                            }
                            _ => {}
                        }
                    }
                }
            }
            Message::SuggestionsCompleted { request_id, result } => {
                if request_id != self.suggest_request_id {
                    return Task::none();
                }
                self.suggest_cancel = None;
                self.suggest_loading = false;
                self.suggestion_index = None;
                if let Some(suggestions) = result {
                    // Merge live suggestions with history matches. When the
                    // service returns nothing, still offer history matches and
                    // the raw query so the dropdown always appears while typing.
                    let query = self.search_query.trim().to_owned();
                    let query_lower = query.to_lowercase();
                    let mut merged = suggestions;
                    for past in &self.search_history.clone() {
                        if !query_lower.is_empty()
                            && past.to_lowercase().contains(&query_lower)
                            && !merged.iter().any(|s| s == past)
                        {
                            merged.push(past.clone());
                            if merged.len() >= 8 {
                                break;
                            }
                        }
                    }
                    if merged.is_empty() && !query.is_empty() {
                        merged.push(query);
                    }
                    merged.truncate(8);
                    self.suggestions = merged;
                    self.show_suggestions =
                        !self.search_query.trim().is_empty() && !self.suggestions.is_empty();
                }
            }
            Message::HomeLoaded { generation, result } => {
                if generation != self.home_generation {
                    return Task::none();
                }
                self.home_cancel = None;
                let Some(result) = result else {
                    return Task::none();
                };
                self.home_loading = false;
                match result {
                    Ok(shelves) => {
                        tracing::info!(
                            shelves = shelves.len(),
                            items = shelves.iter().map(|shelf| shelf.items.len()).sum::<usize>(),
                            "YouTube Music Home feed loaded"
                        );
                        self.home_shelves = shelves;
                        self.home_picks.rebuild(&self.home_shelves);
                        self.learn_shelf_durations();
                        self.home_error = None;
                        let shelves = self.home_shelves.clone();
                        drop(tokio::task::spawn_blocking(move || {
                            crate::home_cache::save(&shelves)
                        }));
                        let artwork_task = self.preload_current_page_artwork();
                        let more_task = self.load_home_more_task(generation);
                        #[cfg_attr(not(feature = "diagnostics"), allow(unused_mut))]
                        let mut tasks = vec![artwork_task, more_task];
                        tasks.push(Task::perform(
                            async { tokio::time::sleep(Duration::from_secs(2)).await },
                            |()| Message::PrewarmPlayback,
                        ));
                        #[cfg(feature = "diagnostics")]
                        tasks.push(diagnostic_autoplay_task());
                        #[cfg(feature = "diagnostics")]
                        tasks.push(diagnostic_page_task());
                        return Task::batch(tasks);
                    }
                    Err(error) => {
                        warn!(%error, "YouTube Music Home feed failed");
                        self.home_error = Some(error);
                    }
                }
            }
            Message::HomeMoreLoaded { generation, result } => {
                if generation != self.home_generation {
                    return Task::none();
                }
                self.home_more_cancel = None;
                let Some(result) = result else {
                    return Task::none();
                };
                match result {
                    Ok(shelves) => {
                        if shelves.is_empty() {
                            return Task::none();
                        }
                        crate::music::append_unique_shelves(&mut self.home_shelves, shelves);
                        self.home_picks.rebuild(&self.home_shelves);
                        self.learn_shelf_durations();
                        tracing::info!(
                            shelves = self.home_shelves.len(),
                            items = self
                                .home_shelves
                                .iter()
                                .map(|shelf| shelf.items.len())
                                .sum::<usize>(),
                            "supplemental YouTube Music Home sections loaded"
                        );
                        let shelves = self.home_shelves.clone();
                        drop(tokio::task::spawn_blocking(move || {
                            crate::home_cache::save(&shelves)
                        }));
                        return self.preload_current_page_artwork();
                    }
                    Err(error) => {
                        tracing::debug!(%error, "supplemental Home sections unavailable");
                    }
                }
            }
            Message::WindowFocusChanged(focused) => {
                self.window_focused = Some(focused);
                if focused {
                    self.window_minimized = false;
                } else {
                    // Minimizing takes the focus away too, so ask the window what happened.
                    return iced::window::latest()
                        .and_then(iced::window::is_minimized)
                        .map(|minimized| {
                            Message::WindowMinimizedChecked(minimized.unwrap_or(false))
                        });
                }
            }
            Message::WindowMinimizedChecked(minimized) => {
                self.window_minimized = minimized;
            }
            Message::ClockTick => {
                self.clock = crate::local_clock::LocalClock::now();
            }
            Message::StatusReading(reading) => {
                self.process_reading = Some(reading);
            }
            Message::TrackNotificationSent(id) => {
                if id != 0 {
                    self.track_notice_id = id;
                }
            }
            Message::AreaMeasured(area) => {
                let changed = (self.area.width - area.width).abs() > 0.5
                    || (self.area.height - area.height).abs() > 0.5;
                self.area = area;
                if changed {
                    return self.schedule_resize_work();
                }
            }
            Message::CarouselStep { forward } => {
                let layout = self.home_layout(self.area);
                let max = layout.max_offset(self.home_picks.items.len());
                let page = layout.visible.max(1) as f32 * layout.stride();
                let target = if forward {
                    (self.carousel.offset + page).min(max)
                } else {
                    (self.carousel.offset - page).max(0.0)
                };
                // Snap to a card edge so a step never stops mid-card.
                let target = if target >= max - 0.5 {
                    max
                } else {
                    (target / layout.stride()).round() * layout.stride()
                };
                self.carousel.start(target, self.preferences.reduced_motion);
                if !self.carousel.animating() {
                    return operation::scroll_to(
                        home::CAROUSEL_ID,
                        AbsoluteOffset { x: target, y: 0.0 },
                    );
                }
            }
            Message::CarouselFrame(now) => {
                let x = self.carousel.step(now);
                return operation::scroll_to(home::CAROUSEL_ID, AbsoluteOffset { x, y: 0.0 });
            }
            Message::CarouselScrolled(x) => {
                // Our own scroll_to also reports here; only manual scrolling moves the offset.
                if !self.carousel.animating() {
                    self.carousel.offset = x;
                }
            }
            Message::ToggleRowMenu(video_id) => {
                self.row_menu = if self.row_menu.as_deref() == Some(video_id.as_str()) {
                    None
                } else {
                    Some(video_id)
                };
            }
            Message::LikeTrack(mut track) => {
                self.row_menu = None;
                let known = self
                    .queue
                    .iter()
                    .chain(self.search_results.iter())
                    .any(|queued| queued.video_id == track.video_id);
                if self.liked.contains(&track.video_id) || known {
                    return self.update(Message::ToggleLike(track.video_id));
                }
                if self.favorites.len() >= crate::preferences::MAX_LOCAL_FAVORITES {
                    return Task::none();
                }
                track.duration_secs = track
                    .duration_secs
                    .or_else(|| self.stored_duration_secs(&track.video_id));
                self.liked.insert(track.video_id.clone());
                self.favorites.push(track);
                self.favorites_context = Arc::from(self.favorites.clone());
                self.persist();
            }
            Message::ResetSettings => {
                let current = std::mem::take(&mut self.preferences);
                let mut defaults = Preferences {
                    window_width: current.window_width,
                    window_height: current.window_height,
                    lyrics_offsets: current.lyrics_offsets,
                    lyrics_offset_order: current.lyrics_offset_order,
                    ..Preferences::default()
                };
                if current.startup && crate::preferences::set_startup(false).is_err() {
                    defaults.startup = true;
                }
                self.preferences = defaults;
                self.quality_setting.set(self.preferences.quality);
                self.autoplay = self.preferences.autoplay;
                self.shuffle = self.preferences.shuffle;
                self.repeat = self.preferences.repeat;
                crate::i18n::set_current_language(self.preferences.ui_language);
                self.playback
                    .set_volume(self.preferences.effective_volume());
                self.playback
                    .set_output_device(self.preferences.audio_output_device_id.clone());
                self.sync_discord_presence(true);
                self.rebuild_lyrics_heights();
                self.persist();
            }
            Message::ReloadHome => {
                self.home_error = None;
                return self.load_home_task();
            }
        }
        Task::none()
    }

    fn new(music_service: Arc<dyn MusicService>, session: YouTubeSession) -> Self {
        let quality_setting = crate::playback_policy::QualitySetting::default();
        let playback = PlaybackHandle::new(Arc::new(crate::playback_policy::ConfiguredResolver {
            inner: Arc::new(YouTubeStreamResolver::new(session.clone())),
            quality: quality_setting.clone(),
        }));
        let discord_presence = crate::discord::DiscordPresence::start();
        let mpris = crate::mpris::MprisRemote::start();
        let tray = crate::tray::TrayRemote::start();
        Self {
            quality_setting,
            lyrics: None,
            lyrics_error: None,
            lyrics_loading: false,
            lyrics_waiting_for_duration: false,
            lyrics_generation: 0,
            lyrics_cancel: None,
            lyric_active: None,
            lyrics_follow_paused: false,
            lyrics_resync: false,
            playback_status_at: std::time::Instant::now(),
            karaoke_tick_pending: false,
            lyrics_auto_scroll_until: None,
            lyrics_scroll: AbsoluteOffset::default(),
            lyrics_heights: Vec::new(),
            window_width: 1600.0,
            window_height: 900.0,
            window_scale_factor: 1.0,
            window_focused: None,
            announced_track: None,
            track_notice_id: 0,
            window_geometry_generation: 0,
            storage_notice: None,
            storage_cache_usage: crate::storage::CacheUsage::default(),
            storage_loading: false,
            storage_generation: 0,
            output_devices: vec![crate::playback::output::OutputDeviceChoice {
                id: None,
                label: String::from("System default"),
            }],
            output_devices_loading: false,
            output_devices_error: None,
            preferences: Preferences::default(),
            persistence: None,
            volume_save_generation: 0,
            state_recovery: None,
            youtube_session: session,
            favorites: Vec::new(),
            favorites_context: Arc::from([]),
            library_notice: None,
            page: Page::Home,
            history: Vec::new(),
            forward: Vec::new(),
            collection: CollectionState::default(),
            collection_cache: VecDeque::new(),
            prefetch_requested: None,
            collection_scroll: 0.0,
            home_scroll: AbsoluteOffset::default(),
            search_scroll: AbsoluteOffset::default(),
            library_scroll: AbsoluteOffset::default(),
            queue: Vec::new(),
            queue_index: None,
            queue_source: None,
            radio_track_ids: HashSet::new(),
            queue_notice: None,
            queue_scroll: AbsoluteOffset::default(),
            queue_generation: 0,
            liked: HashSet::new(),
            shuffle: false,
            repeat: RepeatMode::Off,
            autoplay: true,
            now_playing_tab: NowPlayingTab::Queue,
            now_playing_return: None,
            search_query: String::new(),
            search_committed: false,
            catalog_results: Vec::new(),
            catalog_continuation: None,
            catalog_more_loading: false,
            catalog_more_error: None,
            radio_loading: false,
            radio_attempted_for: None,
            radio_error: None,
            radio_generation: 0,
            radio_cancel: None,
            search_results: Arc::from([]),
            search_error: None,
            search_loading: false,
            search_request_id: 0,
            search_cancel: None,
            search_history: Vec::new(),
            suggestions: Vec::new(),
            suggest_loading: false,
            suggest_request_id: 0,
            suggest_cancel: None,
            show_suggestions: false,
            suggestion_index: None,
            typed_query: None,
            music_service,
            playback,
            discord_presence,
            mpris,
            tray,
            tray_available: false,
            tray_status: "System tray is starting.".into(),
            window_hidden_to_tray: false,
            window_minimized: false,
            clock: crate::local_clock::LocalClock::now(),
            process_reading: None,
            recent: Vec::new(),
            recent_track_id: None,
            discord_presence_key: None,
            discord_presence_position: Duration::ZERO,
            discord_presence_updated_at: None,
            discord_presence_enabled_last: false,
            playback_status: PlaybackStatus::default(),
            seek_preview: None,
            artwork: None,
            artwork_cache: crate::artwork::ArtworkCache::default(),
            artwork_low_memory: false,
            scroll_preload_pending: false,
            resize_work_generation: 0,
            scroll_preload_timer: false,
            #[cfg(feature = "diagnostics")]
            diagnostic_scroll_step: 0,
            artwork_requested: HashSet::new(),
            artwork_failed: HashMap::new(),
            artwork_retry_pending: false,
            artwork_selected_retried: false,
            copy_notice: None,
            copy_notice_count: 0,
            artwork_cancel: None,
            artwork_batch_tokens: HashMap::new(),
            artwork_request_id: 0,
            artwork_selected_request_id: 0,
            artwork_generation: 0,
            home_shelves: Vec::new(),
            home_loading: true,
            home_error: None,
            home_generation: 0,
            home_cancel: None,
            home_more_cancel: None,
            home_picks: home::HomePicks::default(),
            carousel: home::Carousel::default(),
            area: estimated_area(1024.0, 768.0),
            ui_zoom: 1.0,
            os_window: (1024.0, 768.0),
            fade_until: None,
            row_menu: None,
            known_durations: HashMap::new(),
        }
    }

    fn boot() -> (Self, Task<Message>) {
        let mut app = Self::default();
        let path = crate::preferences::state_path();
        match crate::preferences::load(&path) {
            Ok(saved) => {
                app.preferences = saved.preferences;
                app.quality_setting.set(app.preferences.quality);
                app.queue = saved.queue;
                app.queue_index = saved.queue_index;
                app.search_history = saved.history;
                app.recent = saved.recent;
                app.favorites = saved.favorites;
                app.favorites_context = Arc::from(app.favorites.clone());
                app.liked = app.favorites.iter().map(|t| t.video_id.clone()).collect();
                app.autoplay = app.preferences.autoplay;
                app.shuffle = app.preferences.shuffle;
                app.repeat = app.preferences.repeat;
                app.playback.set_volume(app.preferences.effective_volume());
                app.playback
                    .set_output_device(app.preferences.audio_output_device_id.clone());
                app.persistence = Some(Persistence::start(path));
            }
            Err(error) => {
                // Keep the unreadable file as evidence, then start from defaults with saving on.
                warn!(%error, "saved state could not be read; starting with defaults");
                match crate::preferences::quarantine(&path) {
                    Ok(moved) => {
                        app.state_recovery = Some(moved.display().to_string());
                        app.persistence = Some(Persistence::start(path));
                    }
                    Err(move_error) => {
                        warn!(%move_error, "could not move unreadable state aside; saving stays off");
                    }
                }
            }
        }
        crate::i18n::set_current_language(app.preferences.ui_language);
        if let Some(shelves) = crate::home_cache::load() {
            app.home_shelves = shelves;
            app.home_picks.rebuild(&app.home_shelves);
            app.learn_shelf_durations();
            tracing::debug!("loaded anonymous Home snapshot from disk");
        }
        let home_task = app.load_home_task();
        let tray_task = Task::perform(app.tray.start_service(), Message::TrayServiceStarted);
        let artwork_task = if app.home_shelves.is_empty() {
            Task::none()
        } else {
            app.preload_current_page_artwork()
        };
        let (window_width, window_height) =
            layout::clamped_window(app.preferences.window_width, app.preferences.window_height);
        app.os_window = (window_width, window_height);
        app.apply_ui_zoom();
        app.area = estimated_area(app.window_width, app.window_height);
        // The window opens at the saved size (see `run`); read back what the
        // compositor actually gave it.
        let restore_window = iced::window::latest()
            .and_then(iced::window::size)
            .map(|size| Message::Resized {
                width: size.width,
                height: size.height,
            });
        let display_scale = iced::window::latest()
            .and_then(iced::window::scale_factor)
            .map(Message::DisplayScaleChanged);
        app.mpris.update(app.mpris_player_state());
        #[cfg(feature = "diagnostics")]
        let diagnostic_exit = Task::batch([
            diagnostic_exit_task(),
            diagnostic_tour_task(),
            diagnostic_soak_task(),
        ]);
        #[cfg(not(feature = "diagnostics"))]
        let diagnostic_exit = Task::none();
        (
            app,
            Task::batch([
                home_task,
                artwork_task,
                tray_task,
                restore_window,
                display_scale,
                diagnostic_exit,
            ]),
        )
    }

    fn persist(&self) {
        if let Some(writer) = &self.persistence {
            let mut preferences = self.preferences.clone();
            preferences.autoplay = self.autoplay;
            writer.save(Snapshot {
                version: 1,
                preferences,
                queue: self.queue.clone(),
                queue_index: self.queue_index,
                history: self.search_history.clone(),
                favorites: self.favorites.clone(),
                recent: self.recent.clone(),
            });
        }
    }

    fn tr<'a>(&self, source: &'a str) -> std::borrow::Cow<'a, str> {
        crate::i18n::translate(self.preferences.ui_language, source)
    }

    fn tr_static(&self, source: &'static str) -> &'static str {
        crate::i18n::translate_static(self.preferences.ui_language, source)
    }

    fn tr_string(&self, source: String) -> String {
        if self.preferences.ui_language == crate::preferences::UiLanguage::English {
            source
        } else {
            crate::i18n::translate(self.preferences.ui_language, &source).into_owned()
        }
    }

    fn snapshot(&self) -> Snapshot {
        let mut preferences = self.preferences.clone();
        preferences.autoplay = self.autoplay;
        Snapshot {
            version: 1,
            preferences,
            queue: self.queue.clone(),
            queue_index: self.queue_index,
            history: self.search_history.clone(),
            favorites: self.favorites.clone(),
            recent: self.recent.clone(),
        }
    }

    fn apply_snapshot(&mut self, mut snapshot: Snapshot) -> Task<Message> {
        snapshot.normalize();
        self.preferences = snapshot.preferences.clone();
        crate::i18n::set_current_language(self.preferences.ui_language);
        let (window_width, window_height) = layout::clamped_window(
            self.preferences.window_width,
            self.preferences.window_height,
        );
        self.preferences.window_width = window_width;
        self.preferences.window_height = window_height;
        self.window_width = window_width;
        self.window_height = window_height;
        self.window_geometry_generation = self.window_geometry_generation.wrapping_add(1);
        self.autoplay = self.preferences.autoplay;
        self.queue = snapshot.queue;
        self.queue_index = snapshot.queue_index;
        self.radio_track_ids.clear();
        self.queue_notice = None;
        self.queue_scroll = AbsoluteOffset::default();
        self.search_history = snapshot.history;
        self.recent = snapshot.recent;
        self.favorites = snapshot.favorites;
        self.favorites_context = Arc::from(self.favorites.clone());
        self.library_notice = None;
        self.liked = self.favorites.iter().map(|t| t.video_id.clone()).collect();
        self.persist();
        iced::window::latest().and_then(move |id| {
            iced::window::resize(id, iced::Size::new(window_width, window_height))
        })
    }

    fn current_lyrics_offset(&self) -> i64 {
        if let Some(track) = self.playback_status.track.as_ref()
            && let Some(per_song) = self.preferences.lyrics_offsets.get(&track.video_id)
        {
            return *per_song;
        }
        self.preferences.lyrics_offset_ms
    }

    /// Shrinks the artwork cache while a BotGuard helper runs, and restores it afterwards.
    /// While a playback helper runs, drop the covers that are not on screen to
    /// lower the peak. Visible covers stay, so nothing blanks out.
    fn apply_helper_memory_budget(&mut self) {
        let helper = crate::memory::helper_running();
        if helper != self.artwork_low_memory {
            self.artwork_low_memory = helper;
            if helper {
                let mut keep: HashSet<String> = self
                    .current_page_artwork()
                    .into_iter()
                    .map(|(url, _)| url)
                    .collect();
                keep.extend(self.playing_cover_url().map(str::to_owned));
                self.artwork_cache.retain_urls(|url| keep.contains(url));
            }
        }
    }

    /// Physical pixels per layout pixel: display scale times interface zoom.
    pub(super) fn pixel_scale(&self) -> f32 {
        self.window_scale_factor * self.ui_zoom
    }

    /// Recomputes the interface zoom from the unzoomed window size.
    fn apply_ui_zoom(&mut self) {
        #[cfg(feature = "diagnostics")]
        let (width, height) = diagnostic_viewport().unwrap_or(self.os_window);
        #[cfg(not(feature = "diagnostics"))]
        let (width, height) = self.os_window;
        self.ui_zoom = ui_zoom_for(width, height, self.preferences.scale_with_window);
        theme::set_hairline_for_zoom(self.ui_zoom);
        self.window_width = width / self.ui_zoom;
        self.window_height = height / self.ui_zoom;
    }

    /// Debounces work that depends on the window or page size.
    fn schedule_resize_work(&mut self) -> Task<Message> {
        self.resize_work_generation = self.resize_work_generation.wrapping_add(1);
        let generation = self.resize_work_generation;
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_millis(150)).await;
                generation
            },
            Message::ResizeWorkDue,
        )
    }

    /// Resolves the next queued track once the current one has 30 seconds left, so the change is quick.
    fn request_prefetch_near_end(&mut self) {
        let status = &self.playback_status;
        if status.state != PlaybackState::Playing {
            return;
        }
        let Some(duration) = status.duration else {
            return;
        };
        if duration.saturating_sub(status.position) > Duration::from_secs(30) {
            return;
        }
        let Some(next) = self
            .next_queue_index()
            .and_then(|index| self.queue.get(index))
            .cloned()
        else {
            return;
        };
        if self.prefetch_requested.as_deref() == Some(next.video_id.as_str()) {
            return;
        }
        self.prefetch_requested = Some(next.video_id.clone());
        // The lyrics cache is keyed by duration, so only a known duration makes the prefetch reusable.
        let duration = self.track_duration(&next.video_id).or_else(|| {
            next.duration_secs
                .map(|secs| Duration::from_secs(u64::from(secs)))
        });
        if let Some(duration) = duration {
            let lyrics_track = next.clone();
            drop(tokio::spawn(async move {
                let _ = crate::lyrics::load(lyrics_track, Some(duration)).await;
            }));
        }
        self.playback.prefetch(next);
    }

    /// The large cover is only shown on Now Playing. When the track changes, the cover of the track
    /// that stopped is dropped: otherwise each track leaves a 640 px image (about 1.6 MiB) in the
    /// cache until the byte budget evicts it, and the process keeps growing through a session.
    /// Drops the large cover of the track that played before a change, unless the new track
    /// shows the same cover. The caller passes both tracks, because the playback status may
    /// already hold the new one when this runs.
    fn release_previous_cover(&mut self, previous: Option<&Track>, current: Option<&Track>) {
        let current = current.and_then(|track| track.thumbnail_url.as_deref());
        let Some(url) = previous.and_then(|track| track.thumbnail_url.as_deref()) else {
            return;
        };
        if Some(url) != current {
            self.artwork_cache
                .remove_variant(url, crate::artwork::LARGE_MAX);
        }
    }

    /// The notification for a track that started while the window is in the background.
    fn track_notice_task(&mut self) -> Option<Task<Message>> {
        let track = self.playback_status.track.clone();
        if !wants_track_notice(
            self.preferences.track_notifications,
            self.window_focused,
            self.playback_status.state,
            track.as_ref(),
            self.announced_track.as_deref(),
        ) {
            return None;
        }
        let track = track?;
        self.announced_track = Some(track.video_id.clone());
        let notice = crate::notifications::TrackNotice {
            replaces_id: self.track_notice_id,
            cover: track
                .thumbnail_url
                .as_deref()
                .and_then(crate::artwork::saved_cover_path),
            title: track.title,
            artist: track.artist,
        };
        Some(Task::perform(
            crate::notifications::show(notice),
            Message::TrackNotificationSent,
        ))
    }

    /// Scrolls the lyrics view to the active line when the line changed.
    fn follow_lyrics_task(&mut self, old_active: Option<usize>, resync: bool) -> Task<Message> {
        if (resync || old_active != self.lyric_active)
            && self.now_playing_tab == NowPlayingTab::Lyrics
            && self.preferences.lyrics_follow
            && !self.lyrics_follow_paused
            && let Some(index) = self.lyric_active
        {
            self.lyrics_auto_scroll_until =
                Some(std::time::Instant::now() + Duration::from_millis(250));
            return operation::scroll_to(
                "lyrics",
                AbsoluteOffset {
                    x: 0.0,
                    y: self.lyrics_scroll_offset(index),
                },
            );
        }
        Task::none()
    }

    fn effective_lyrics_position(&self) -> f64 {
        self.playback_status.position.as_secs_f64() + self.current_lyrics_offset() as f64 / 1000.0
    }

    fn load_home_task(&mut self) -> Task<Message> {
        if let Some(previous) = self.home_cancel.take() {
            previous.cancel();
        }
        if let Some(previous) = self.home_more_cancel.take() {
            previous.cancel();
        }
        self.home_generation = self.home_generation.wrapping_add(1);
        let generation = self.home_generation;
        let cancellation = CancellationToken::new();
        self.home_cancel = Some(cancellation.clone());
        self.home_loading = true;
        let service = Arc::clone(&self.music_service);
        Task::perform(
            async move {
                tokio::select! {
                    _ = cancellation.cancelled() => None,
                    result = service.home() => Some(result.map_err(|error| error.user_message())),
                }
            },
            move |result| Message::HomeLoaded { generation, result },
        )
    }

    fn load_home_more_task(&mut self, generation: u64) -> Task<Message> {
        if let Some(previous) = self.home_more_cancel.take() {
            previous.cancel();
        }
        let cancellation = CancellationToken::new();
        self.home_more_cancel = Some(cancellation.clone());
        let service = Arc::clone(&self.music_service);
        Task::perform(
            async move {
                tokio::select! {
                    _ = cancellation.cancelled() => None,
                    result = service.home_more() => Some(result.map_err(|error| error.user_message())),
                }
            },
            move |result| Message::HomeMoreLoaded { generation, result },
        )
    }

    /// Highlights a suggestion and shows it in the search field, so Enter
    /// (handled by the field's submit) searches exactly what is shown.
    fn highlight_suggestion(&mut self, index: usize) -> Task<Message> {
        let Some(choice) = self.suggestions.get(index).cloned() else {
            return Task::none();
        };
        if self.typed_query.is_none() {
            self.typed_query = Some(self.search_query.clone());
        }
        self.suggestion_index = Some(index);
        self.search_query = choice;
        operation::move_cursor_to_end("search-field")
    }

    fn invalidate_suggestions(&mut self) {
        self.typed_query = None;
        if let Some(token) = self.suggest_cancel.take() {
            token.cancel();
        }
        self.suggest_request_id = self.suggest_request_id.wrapping_add(1);
        self.suggest_loading = false;
        self.show_suggestions = false;
        self.suggestion_index = None;
    }

    fn push_history(&mut self, query: String) {
        let query = query.trim().to_owned();
        if query.is_empty() || !self.preferences.remember_history {
            return;
        }
        self.search_history.retain(|past| past != &query);
        self.search_history.insert(0, query);
        self.search_history.truncate(30);
        self.persist();
    }

    fn start_suggestions(&mut self, query: String) -> Task<Message> {
        if let Some(previous) = self.suggest_cancel.take() {
            previous.cancel();
        }
        self.suggest_request_id = self.suggest_request_id.wrapping_add(1);
        let trimmed = query.trim().to_owned();
        self.search_query = query;
        if trimmed.is_empty() || trimmed.len() < 2 {
            self.suggestions.clear();
            self.show_suggestions = false;
            self.suggest_loading = false;
            return Task::none();
        }
        self.suggest_loading = true;
        self.show_suggestions = true;
        let request_id = self.suggest_request_id;
        let cancellation = CancellationToken::new();
        self.suggest_cancel = Some(cancellation.clone());
        let service = Arc::clone(&self.music_service);
        Task::perform(
            async move {
                tokio::select! {
                    _ = cancellation.cancelled() => return None,
                    _ = tokio::time::sleep(Duration::from_millis(220)) => {}
                }
                tokio::select! {
                    _ = cancellation.cancelled() => None,
                    result = service.suggestions(&trimmed) => {
                        match result {
                            Ok(suggestions) => Some(suggestions),
                            Err(err) => {
                                warn!(error = ?err, "suggestions failed");
                                Some(Vec::new())
                            }
                        }
                    },
                }
            },
            move |result| Message::SuggestionsCompleted { request_id, result },
        )
    }

    fn start_search(&mut self, query: String, debounce: bool) -> Task<Message> {
        if let Some(previous) = self.search_cancel.take() {
            previous.cancel();
        }
        if self.search_query != query {
            self.search_scroll = AbsoluteOffset::default();
        }
        self.search_query = query.clone();
        self.search_request_id = self.search_request_id.wrapping_add(1);
        self.search_error = None;
        self.catalog_continuation = None;
        self.catalog_more_loading = false;
        self.catalog_more_error = None;
        // Keep stale results visible while the new query resolves so the
        // page never flashes an ugly empty "Searching..." state.
        if query.trim().is_empty() {
            self.search_results = Arc::from([]);
            self.catalog_results.clear();
            self.catalog_continuation = None;
            self.search_loading = false;
            return Task::none();
        }

        self.search_loading = true;
        let request_id = self.search_request_id;
        let cancellation = CancellationToken::new();
        self.search_cancel = Some(cancellation.clone());
        let service = Arc::clone(&self.music_service);

        Task::perform(
            async move {
                if debounce {
                    tokio::select! {
                        _ = cancellation.cancelled() => return None,
                        _ = tokio::time::sleep(Duration::from_millis(300)) => {}
                    }
                }
                tokio::select! {
                    _ = cancellation.cancelled() => None,
                    result = service.search_catalog_filtered_page(&query, crate::music::CatalogFilter::All, None) => Some(result.map_err(|err| {
                        warn!(error = ?err, "YouTube Music search failed");
                        err.user_message()
                    })),
                }
            },
            move |result| Message::CatalogCompleted { request_id, result },
        )
    }

    fn load_more_catalog_task(&mut self) -> Task<Message> {
        if self.search_loading || self.catalog_more_loading {
            return Task::none();
        }
        let Some(continuation) = self.catalog_continuation.clone() else {
            return Task::none();
        };
        let query = self.search_query.clone();
        let filter = crate::music::CatalogFilter::All;
        let request_id = self.search_request_id;
        let cancellation = CancellationToken::new();
        self.search_cancel = Some(cancellation.clone());
        self.catalog_more_loading = true;
        self.catalog_more_error = None;
        self.search_loading = true;
        let service = Arc::clone(&self.music_service);
        Task::perform(
            async move {
                tokio::select! {
                    _ = cancellation.cancelled() => None,
                    result = service.search_catalog_filtered_page(&query, filter, Some(continuation)) => Some(result.map_err(|error| {
                        warn!(error = ?error, "YouTube Music search continuation failed");
                        error.user_message()
                    })),
                }
            },
            move |result| Message::CatalogMoreCompleted { request_id, result },
        )
    }

    fn navigate(&mut self, page: Page) -> Task<Message> {
        self.navigate_internal(page, true)
    }

    fn navigate_internal(&mut self, page: Page, remember_current: bool) -> Task<Message> {
        if self.page == page {
            return Task::none();
        }
        if matches!(page, Page::NowPlaying) && self.playback_status.track.is_none() {
            return Task::none();
        }
        // Remember where Now Playing was opened from, so closing it returns there.
        if matches!(page, Page::NowPlaying) && !matches!(self.page, Page::NowPlaying) {
            self.now_playing_return = Some(self.page.clone());
        }
        if remember_current {
            self.history.push(self.page.clone());
            // Bounded stack: discard oldest, keep the 40 most recent.
            if self.history.len() > 40 {
                let excess = self.history.len() - 40;
                self.history.drain(0..excess);
            }
            // New navigation invalidates Forward.
            self.forward.clear();
        }
        let leaving_collection = matches!(self.page, Page::CollectionDetail(_));
        self.collection.cancel();
        if leaving_collection && !matches!(page, Page::CollectionDetail(_)) {
            // Rows and their covers are dropped with the page; hand the freed
            // heap back so repeated visits do not ratchet the footprint up.
            self.release_offscreen_artwork_for(&page);
            crate::memory::trim_heap();
        }
        // Leaving the page cancels all in-flight artwork and invalidates
        // the reconciled window. Scroll events do not bump the generation.
        self.cancel_artwork_batch();
        self.artwork_generation = self.artwork_generation.wrapping_add(1);
        self.invalidate_suggestions();
        self.page = page.clone();
        if self.page == Page::Home {
            self.enter_home();
            tracing::info!(scroll_y = self.home_scroll.y, "returned to cached Home");
            Task::batch([
                operation::scroll_to("home-content", self.home_scroll),
                self.preload_current_page_artwork(),
            ])
        } else if self.page == Page::Search {
            Task::batch([
                operation::scroll_to("search-content", self.search_scroll),
                self.preload_current_page_artwork(),
            ])
        } else if self.page == Page::Library {
            Task::batch([
                operation::scroll_to("library-content", self.library_scroll),
                self.preload_current_page_artwork(),
            ])
        } else if let Page::CollectionDetail(target) = page {
            self.load_collection(target)
        } else if self.page == Page::NowPlaying {
            Task::batch([
                operation::scroll_to("queue-content", self.queue_scroll),
                self.preload_current_page_artwork(),
            ])
        } else {
            Task::none()
        }
    }

    fn learn_shelf_durations(&mut self) {
        let learned: Vec<(String, Duration)> = self
            .home_shelves
            .iter()
            .flat_map(|shelf| &shelf.items)
            .filter_map(|item| Some((item.track.as_ref()?.video_id.clone(), item.duration?)))
            .collect();
        for (video_id, duration) in learned {
            self.remember_duration(video_id, duration);
        }
    }

    fn load_lyrics(&mut self, track: Track) -> Task<Message> {
        // A track still named by its video ID has nothing to search for; its metadata reloads the lyrics.
        if metadata_pending(&track) {
            return Task::none();
        }
        if let Some(token) = self.lyrics_cancel.take() {
            token.cancel();
        }
        self.lyrics_generation = self.lyrics_generation.wrapping_add(1);
        self.lyrics = None;
        self.lyrics_error = None;
        self.lyric_active = None;
        self.lyrics_scroll = AbsoluteOffset::default();
        self.lyrics_heights.clear();
        self.lyrics_loading = true;
        let duration = self.track_duration(&track.video_id).or_else(|| {
            self.playback_status
                .track
                .as_ref()
                .filter(|current| current.video_id == track.video_id)
                .and(self.playback_status.duration)
        });
        self.lyrics_waiting_for_duration = duration.is_none();
        let token = CancellationToken::new();
        self.lyrics_cancel = Some(token.clone());
        let generation = self.lyrics_generation;
        let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
        let progress_stream = futures_util::stream::unfold(progress_rx, |mut receiver| async {
            receiver.recv().await.map(|lyrics| (lyrics, receiver))
        });
        let progress = Task::run(progress_stream, move |lyrics| Message::LyricsProgress {
            generation,
            lyrics,
        });
        let load = Task::perform(
            async move {
                tokio::select! {
                    _ = token.cancelled() => Err("Cancelled".into()),
                    result = crate::lyrics::load_with_progress(track, duration, move |lyrics| {
                        let _ = progress_tx.send(lyrics);
                    }) => result,
                }
            },
            move |result| Message::LyricsLoaded { generation, result },
        );
        Task::batch([
            operation::scroll_to("lyrics", AbsoluteOffset::<f32>::default()),
            load,
            progress,
        ])
    }

    fn lyrics_layout_width(&self) -> f32 {
        self.lyrics_text_width()
    }

    fn rebuild_lyrics_heights(&mut self) {
        let width = self.lyrics_layout_width();
        let font_size = self.preferences.lyrics_font_size;
        let show_translation = self.preferences.lyrics_show_translation;
        self.lyrics_heights = self.lyrics.as_ref().map_or_else(Vec::new, |lyrics| {
            lyrics
                .lines
                .iter()
                .map(|line| {
                    crate::lyrics::estimated_line_height(line, width, font_size, show_translation)
                })
                .collect()
        });
    }

    fn lyrics_scroll_offset(&self, index: usize) -> f32 {
        crate::lyrics::estimated_scroll_offset(&self.lyrics_heights, index)
    }

    fn sync_discord_presence(&mut self, force: bool) {
        if !self.preferences.discord_presence {
            if self.discord_presence_enabled_last || self.discord_presence_key.is_some() {
                self.discord_presence.clear();
            }
            self.discord_presence_enabled_last = false;
            self.discord_presence_key = None;
            self.discord_presence_updated_at = None;
            return;
        }
        self.discord_presence_enabled_last = true;

        let Some(track) = self.playback_status.track.clone() else {
            if self.discord_presence_key.take().is_some() {
                self.discord_presence.clear();
            }
            self.discord_presence_updated_at = None;
            return;
        };
        let state = self.playback_status.state;
        let paused = match state {
            PlaybackState::Playing => false,
            PlaybackState::Paused => true,
            _ => {
                let track_changed = self
                    .discord_presence_key
                    .as_ref()
                    .is_some_and(|(active_id, _)| active_id != &track.video_id);
                let should_clear = matches!(
                    state,
                    PlaybackState::Idle
                        | PlaybackState::Resolving
                        | PlaybackState::Stopped
                        | PlaybackState::Finished
                        | PlaybackState::Error
                ) || track_changed;
                if should_clear {
                    if self.discord_presence_key.take().is_some() {
                        self.discord_presence.clear();
                    }
                    self.discord_presence_updated_at = None;
                }
                return;
            }
        };
        let key = (track.video_id.clone(), paused);
        let position = self.playback_status.position;
        let expected_position = self
            .discord_presence_updated_at
            .map(|updated| {
                self.discord_presence_position.saturating_add(if paused {
                    Duration::ZERO
                } else {
                    updated.elapsed()
                })
            })
            .unwrap_or(self.discord_presence_position);
        let was_seeked = position.abs_diff(expected_position) > Duration::from_secs(3);
        if !force && self.discord_presence_key.as_ref() == Some(&key) && !was_seeked {
            return;
        }

        let paused_label = self.tr("Paused").into_owned();
        let listen_label = self.tr("Listen on YouTube Music").into_owned();
        self.discord_presence.update(crate::discord::Presence {
            video_id: track.video_id.clone(),
            title: track.title,
            artist: track.artist,
            album: track.album,
            artwork_url: track.thumbnail_url,
            position,
            duration: self
                .playback_status
                .duration
                .or_else(|| self.track_duration(&track.video_id)),
            paused,
            paused_label,
            listen_label,
        });
        self.discord_presence_key = Some(key);
        self.discord_presence_position = position;
        self.discord_presence_updated_at = Some(std::time::Instant::now());
    }

    fn view(&self) -> Element<'_, Message> {
        #[cfg(feature = "diagnostics")]
        let started = std::time::Instant::now();
        let element = self.view_content();
        #[cfg(feature = "diagnostics")]
        timing::record_view(started, started.elapsed());
        element
    }

    fn view_content(&self) -> Element<'_, Message> {
        #[cfg(feature = "diagnostics")]
        if DIAGNOSTIC_SHOTS.get().is_some() {
            // `CRABYTUBE_VIEWPORT=WxH` lays the app out as if the window had that
            // size, for compositors that force their own window size.
            let shell: Element<'_, Message> = match diagnostic_viewport() {
                Some((width, height)) => iced::widget::container(self.shell_view())
                    .width(width / self.ui_zoom)
                    .height(height / self.ui_zoom)
                    .into(),
                None => self.shell_view(),
            };
            // The screenshot tour must not react to a pointer that happens to be over the window.
            return iced::widget::stack![
                shell,
                iced::widget::opaque(
                    iced::widget::Space::new()
                        .width(iced::Length::Fill)
                        .height(iced::Length::Fill)
                ),
            ]
            .into();
        }
        self.shell_view()
    }
}

async fn fetch_artwork(
    url: String,
    cancellation: CancellationToken,
    max_dim: u32,
) -> Option<image::Handle> {
    crate::artwork::fetch_sized(url, cancellation, max_dim).await
}

/// The cover of the track that plays, which has its own download lane.
async fn fetch_player_artwork(
    url: String,
    cancellation: CancellationToken,
    max_dim: u32,
) -> Option<image::Handle> {
    crate::artwork::fetch_player(url, cancellation, max_dim).await
}

/// The saved window size, so the window opens at it instead of being resized
/// after mapping (Wayland compositors may ignore a later resize request).
fn initial_window_size() -> iced::Size {
    // Diagnostics: `CRABYTUBE_WINDOW=1280x720` opens at that size.
    #[cfg(feature = "diagnostics")]
    if let Some((width, height)) = std::env::var("CRABYTUBE_WINDOW").ok().and_then(|value| {
        let (width, height) = value.split_once('x')?;
        Some((width.parse::<f32>().ok()?, height.parse::<f32>().ok()?))
    }) {
        return iced::Size::new(width, height);
    }
    let saved = crate::preferences::load(&crate::preferences::state_path())
        .ok()
        .map(|snapshot| {
            (
                snapshot.preferences.window_width,
                snapshot.preferences.window_height,
            )
        });
    let (width, height) = match saved {
        Some((width, height)) => layout::clamped_window(width, height),
        None => layout::default_window_size(),
    };
    iced::Size::new(width, height)
}

/// Collections kept in memory for instant back navigation.
const MAX_CACHED_COLLECTIONS: usize = 4;

/// Size the layout is designed at. Larger windows zoom the whole interface so
/// it keeps these proportions instead of spreading out.
const DESIGN_SIZE: (f32, f32) = (1280.0, 800.0);

/// Interface zoom for an unzoomed window size: 1.0 up to the design size, then
/// grows with the smaller of the two ratios.
fn ui_zoom_for(width: f32, height: f32, enabled: bool) -> f32 {
    if !enabled || !width.is_finite() || !height.is_finite() {
        return 1.0;
    }
    let zoom = (width / DESIGN_SIZE.0)
        .min(height / DESIGN_SIZE.1)
        .clamp(1.0, 3.0);
    // Whole hundredths keep text from re-rasterizing on every pixel of a drag.
    (zoom * 100.0).floor() / 100.0
}

/// Page area for a window size, before the first layout reports the real one.
fn estimated_area(window_width: f32, window_height: f32) -> iced::Size {
    use theme::{GUTTER, NAV_H, PLAYER_H, TITLE_BAR_H};
    iced::Size::new(
        (window_width - 2.0 * GUTTER - 2.0).max(320.0),
        (window_height - TITLE_BAR_H - NAV_H - PLAYER_H - 4.0).max(240.0),
    )
}

/// Whether a playing track deserves a notification: the setting is on, the window is known to be
/// in the background, the title is known, and the track has not been announced yet.
fn wants_track_notice(
    enabled: bool,
    window_focused: Option<bool>,
    state: PlaybackState,
    track: Option<&Track>,
    announced: Option<&str>,
) -> bool {
    enabled
        && window_focused == Some(false)
        && matches!(state, PlaybackState::Playing | PlaybackState::Buffering)
        && track.is_some_and(|track| {
            !metadata_pending(track) && announced != Some(track.video_id.as_str())
        })
}

/// A track whose title is still empty or its video ID, as tracks started by ID alone are.
fn metadata_pending(track: &Track) -> bool {
    track.title.trim().is_empty() || track.title == track.video_id
}

/// Parses `m:ss` or `h:mm:ss`.
fn parse_clock(clock: &str) -> Option<Duration> {
    let mut seconds = 0_u64;
    for part in clock.split(':') {
        seconds = seconds
            .checked_mul(60)?
            .checked_add(part.parse::<u64>().ok()?)?;
    }
    Some(Duration::from_secs(seconds))
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn catalog_item_key(item: &HomeItem) -> (HomeItemKind, String, Option<String>, Option<String>) {
    (
        item.kind,
        item.title.clone(),
        item.browse_id.clone(),
        item.track.as_ref().map(|track| track.video_id.clone()),
    )
}

/// Where the current queue came from, recorded when playback starts.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct QueueSource {
    pub title: String,
    /// Full size of the source list, which can exceed the queue's capped length.
    pub track_count: usize,
    /// Where the list opens from, when it is an album or playlist that can be opened again.
    pub browse: Option<crate::music::BrowseTarget>,
    pub thumbnail_url: Option<String>,
    pub album: bool,
}

fn bounded_queue_context(tracks: &[Track], index: usize, limit: usize) -> (Vec<Track>, usize) {
    if tracks.is_empty() || index >= tracks.len() || limit == 0 {
        return (Vec::new(), 0);
    }
    let start = index.saturating_sub(25);
    let end = start.saturating_add(limit).min(tracks.len());
    (tracks[start..end].to_vec(), index - start)
}

const DEFAULT_TOKIO_WORKERS: usize = 2;

fn worker_thread_count(setting: Option<&str>) -> usize {
    setting
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(DEFAULT_TOKIO_WORKERS)
}

struct AppExecutor(tokio::runtime::Runtime);

impl iced::Executor for AppExecutor {
    fn new() -> Result<Self, iced::futures::io::Error> {
        let worker_threads =
            worker_thread_count(std::env::var("TOKIO_WORKER_THREADS").ok().as_deref());
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .enable_all()
            .build()
            .map(Self)
            .map_err(iced::futures::io::Error::other)
    }

    fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        drop(self.0.spawn(future));
    }

    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.0.block_on(future)
    }

    fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self.0.enter();
        f()
    }
}

/// Selects the renderer before any thread starts. Iced reads `ICED_BACKEND` at startup:
/// the CPU renderer is forced unless the saved GPU preference is on, in which case wgpu is
/// tried first and tiny-skia remains the fallback.
pub fn configure_renderer_from_preferences() {
    let gpu = crate::preferences::load(&crate::preferences::state_path())
        .map(|snapshot| snapshot.preferences.gpu_rendering)
        .unwrap_or(false);
    let backend = if gpu { "wgpu,tiny-skia" } else { "tiny-skia" };
    if std::env::var_os("ICED_BACKEND").is_none() {
        // SAFETY: called from main before any other thread is started, so no other thread
        // can read the environment concurrently.
        unsafe { std::env::set_var("ICED_BACKEND", backend) };
    }
}

/// The window and taskbar icon (`assets/app-icon-128.png`).
fn window_icon() -> Option<iced::window::Icon> {
    let decoded = ::image::load_from_memory(include_bytes!("../assets/app-icon-128.png")).ok()?;
    let rgba = decoded.into_rgba8();
    let (width, height) = rgba.dimensions();
    iced::window::icon::from_rgba(rgba.into_raw(), width, height).ok()
}

pub fn run() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .executor::<AppExecutor>()
        .font(theme::FONT_REGULAR_BYTES)
        .font(theme::FONT_BOLD_BYTES)
        .default_font(theme::MONO)
        .title("CrabyTube")
        .subscription(App::subscription)
        .theme(|_: &App| theme::app_theme())
        .scale_factor(|app: &App| app.ui_zoom)
        .window(iced::window::Settings {
            size: initial_window_size(),
            min_size: Some(iced::Size::new(layout::MIN_WIDTH, layout::MIN_HEIGHT)),
            icon: window_icon(),
            #[cfg(target_os = "linux")]
            platform_specific: iced::window::settings::PlatformSpecific {
                // Matches `crabytube.desktop`, so docks and task switchers
                // show the app icon on Wayland.
                application_id: "crabytube".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .exit_on_close_request(false)
        .decorations(false)
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str) -> Track {
        Track {
            video_id: "abc123".into(),
            title: title.into(),
            artist: "Artist".into(),
            album: None,
            thumbnail_url: None,
            duration_secs: None,
        }
    }

    #[test]
    fn a_track_is_announced_only_when_it_plays_in_the_background() {
        let track = song("Song");
        let play = |enabled, focus, state, announced| {
            wants_track_notice(enabled, focus, state, Some(&track), announced)
        };
        assert!(play(true, Some(false), PlaybackState::Playing, None));
        assert!(play(true, Some(false), PlaybackState::Buffering, None));
        // Off by default, focused, unknown focus, not playing, or already announced: no notice.
        assert!(!play(false, Some(false), PlaybackState::Playing, None));
        assert!(!play(true, Some(true), PlaybackState::Playing, None));
        assert!(!play(true, None, PlaybackState::Playing, None));
        assert!(!play(true, Some(false), PlaybackState::Paused, None));
        assert!(!play(
            true,
            Some(false),
            PlaybackState::Playing,
            Some("abc123")
        ));
        // A track whose title is still its ID waits for the metadata.
        let unnamed = Track {
            title: "abc123".into(),
            ..song("")
        };
        assert!(!wants_track_notice(
            true,
            Some(false),
            PlaybackState::Playing,
            Some(&unnamed),
            None
        ));
    }

    use super::{
        App, Message, Page, PlaybackState, Preference, bounded_queue_context, should_close_to_tray,
        worker_thread_count,
    };
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn runtime_worker_count_has_small_default_and_honors_valid_override() {
        assert_eq!(worker_thread_count(None), 2);
        assert_eq!(worker_thread_count(Some("4")), 4);
        assert_eq!(worker_thread_count(Some("0")), 2);
        assert_eq!(worker_thread_count(Some("invalid")), 2);
    }

    #[test]
    fn close_to_tray_never_hides_window_without_a_live_tray_host() {
        assert!(should_close_to_tray(true, true));
        assert!(!should_close_to_tray(false, true));
        assert!(!should_close_to_tray(true, false));
    }

    #[test]
    fn bounded_queue_context_preserves_selected_track_and_cursor_at_scale() {
        for count in [100, 1_000, 10_000] {
            let tracks: Vec<_> = (0..count)
                .map(|index| crate::music::Track {
                    duration_secs: None,
                    video_id: format!("id-{index}"),
                    title: format!("Track {index}"),
                    artist: "Artist".into(),
                    album: None,
                    thumbnail_url: None,
                })
                .collect();
            let index = if count == 10_000 { 9_000 } else { count - 1 };
            let (queue, cursor) =
                bounded_queue_context(&tracks, index, crate::preferences::MAX_QUEUE_TRACKS);
            assert!(queue.len() <= crate::preferences::MAX_QUEUE_TRACKS);
            assert_eq!(queue[cursor].video_id, tracks[index].video_id);
            assert!(cursor <= 25);
        }
    }

    fn shelf_with_artwork(index: usize) -> crate::music::HomeShelf {
        use crate::music::{HomeItem, HomeItemKind, HomeShelf};
        HomeShelf {
            title: if index == 0 {
                "All-time essentials".into()
            } else {
                format!("Shelf {index}")
            },
            items: (0..20)
                .map(|item| HomeItem {
                    title: format!("Item {item}"),
                    subtitle: String::new(),
                    thumbnail_url: Some(format!("cover-{index}-{item}")),
                    kind: HomeItemKind::Song,
                    browse_id: None,
                    browse_target: None,
                    track: Some(crate::music::Track {
                        duration_secs: None,
                        video_id: format!("video-{index}-{item}"),
                        title: format!("Item {item}"),
                        artist: String::new(),
                        album: None,
                        thumbnail_url: Some(format!("cover-{index}-{item}")),
                    }),
                    duration: None,
                })
                .collect(),
        }
    }

    fn with_picks(mut app: App) -> App {
        let shelves = app.home_shelves.clone();
        app.home_picks.rebuild(&shelves);
        app
    }

    #[test]
    fn other_queues_clear_the_recorded_queue_source() {
        let track = crate::music::Track {
            duration_secs: None,
            video_id: "abc123DEF_-".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: None,
            thumbnail_url: None,
        };
        let mut app = App {
            queue_source: Some(super::QueueSource {
                title: "Long Mix".into(),
                track_count: 5000,
                browse: None,
                thumbnail_url: None,
                album: false,
            }),
            ..App::default()
        };
        assert_eq!(
            app.queue_source.as_ref().map(|source| source.track_count),
            Some(5000)
        );

        let _ = app.update(Message::PlayContext(Arc::from(vec![track]), 0));
        assert!(app.queue_source.is_none());
    }

    #[test]
    fn scroll_preload_holds_one_timer_for_a_burst_of_scroll_events() {
        let mut app = App::default();
        let _ = app.scroll_preload_task();
        let _ = app.scroll_preload_task();
        let _ = app.scroll_preload_task();
        assert!(app.scroll_preload_timer);
        assert!(app.scroll_preload_pending);

        let _ = app.update(Message::ScrollPreloadDue);
        assert!(!app.scroll_preload_timer);
        assert!(!app.scroll_preload_pending);
    }

    #[test]
    fn shuffle_and_repeat_changes_are_recorded_in_preferences() {
        let mut app = App::default();

        let _ = app.update(Message::CycleRepeat);
        let _ = app.update(Message::ToggleShuffle);

        assert_eq!(app.preferences.repeat, crate::preferences::RepeatMode::All);
        assert!(app.preferences.shuffle);
    }

    #[test]
    fn now_playing_returns_to_the_page_it_was_opened_from() {
        let mut app = App {
            page: Page::Library,
            playback_status: crate::playback::PlaybackStatus {
                track: Some(crate::music::Track {
                    duration_secs: None,
                    video_id: "abc123DEF_-".into(),
                    title: "Song".into(),
                    artist: "Artist".into(),
                    album: None,
                    thumbnail_url: None,
                }),
                ..crate::playback::PlaybackStatus::default()
            },
            ..App::default()
        };

        let _ = app.navigate_internal(Page::NowPlaying, true);
        assert_eq!(app.now_playing_return, Some(Page::Library));

        // Leaving Now Playing for another page must not overwrite the origin.
        let _ = app.navigate_internal(Page::Home, true);
        assert_eq!(app.now_playing_return, Some(Page::Library));
    }

    #[test]
    fn lyric_follow_scrolls_only_when_the_active_line_changes() {
        let mut app = App {
            now_playing_tab: super::NowPlayingTab::Lyrics,
            lyric_active: Some(3),
            ..App::default()
        };
        app.preferences.lyrics_follow = true;

        // Same line as before: nothing to scroll.
        let _ = app.follow_lyrics_task(Some(3), false);
        assert!(app.lyrics_auto_scroll_until.is_none());

        // Line changed: follow sets the auto-scroll guard.
        let _ = app.follow_lyrics_task(Some(2), false);
        assert!(app.lyrics_auto_scroll_until.is_some());
    }

    #[test]
    fn a_favorite_saved_without_its_artist_gets_it_when_the_track_resolves() {
        let mut app = App::default();
        let mut saved = song("Saved");
        saved.video_id = "abc".into();
        saved.artist = String::new();
        app.favorites = vec![saved.clone()];
        app.playback_status.track = Some(saved.clone());
        let mut resolved = saved.clone();
        resolved.artist = "Resolved Artist".into();
        drop(app.update_playback_status(PlaybackStatus {
            track: Some(resolved),
            state: PlaybackState::Playing,
            ..PlaybackStatus::default()
        }));
        assert_eq!(app.favorites[0].artist, "Resolved Artist");
    }

    #[test]
    fn synced_lyrics_wake_at_the_next_word_instead_of_a_fixed_tick() {
        let word = |start: f64| crate::lyrics::Word {
            start,
            end: start + 0.4,
            text: "word".into(),
        };
        let synced = crate::lyrics::Lyrics {
            source: "fixture".into(),
            lines: vec![crate::lyrics::Line {
                start: 10.0,
                end: None,
                text: "word word".into(),
                words: vec![word(10.0), word(10.5)],
                translation: None,
                romanization: None,
                background: None,
            }],
            plain: None,
        };
        let mut app = App {
            page: Page::NowPlaying,
            now_playing_tab: super::NowPlayingTab::Lyrics,
            lyrics: Some(synced),
            ..App::default()
        };
        let status = PlaybackStatus {
            track: Some(song("Track")),
            position: Duration::from_secs_f64(10.2),
            state: PlaybackState::Playing,
            ..PlaybackStatus::default()
        };
        drop(app.update_playback_status(status));
        assert!(app.karaoke_active());
        assert!(app.karaoke_tick_pending, "one wake-up is on its way");
        // The next word starts 0.3 s after the position, plus the small margin.
        let wait = app.karaoke_wait().unwrap();
        assert!(
            (Duration::from_millis(300)..=Duration::from_millis(330)).contains(&wait),
            "wait was {wait:?}"
        );
        // The wake-up itself clears the pending flag.
        drop(app.update(Message::PlaybackTick));
        assert!(!app.karaoke_tick_pending);

        // Lyrics without any timing keep the progress tick.
        app.lyrics = Some(crate::lyrics::Lyrics {
            source: "plain".into(),
            lines: Vec::new(),
            plain: Some("no timing".into()),
        });
        assert!(!app.karaoke_active());
    }

    #[test]
    fn line_timed_lyrics_wake_at_the_next_line_start() {
        let line = |start: f64, text: &str| crate::lyrics::Line {
            start,
            end: None,
            text: text.into(),
            words: Vec::new(),
            translation: None,
            romanization: None,
            background: None,
        };
        let mut app = App {
            page: Page::NowPlaying,
            now_playing_tab: super::NowPlayingTab::Lyrics,
            lyrics: Some(crate::lyrics::Lyrics {
                source: "line-timed".into(),
                lines: vec![line(10.0, "first"), line(13.0, "second")],
                plain: None,
            }),
            ..App::default()
        };
        drop(app.update_playback_status(PlaybackStatus {
            track: Some(song("Track")),
            position: Duration::from_secs_f64(12.5),
            state: PlaybackState::Playing,
            ..PlaybackStatus::default()
        }));
        assert!(
            app.karaoke_active(),
            "line timing alone must drive the lyrics"
        );
        // The next line starts half a second away, plus the small margin.
        let wait = app.karaoke_wait().unwrap();
        assert!(
            (Duration::from_millis(500)..=Duration::from_millis(530)).contains(&wait),
            "wait was {wait:?}"
        );
    }

    #[test]
    fn now_playing_ticks_once_a_second_and_other_pages_four_times() {
        assert_eq!(
            super::subscriptions::progress_tick(&Page::NowPlaying),
            Duration::from_secs(1)
        );
        assert_eq!(
            super::subscriptions::progress_tick(&Page::Home),
            super::PLAYBACK_TICK
        );
    }

    #[test]
    fn every_seek_resumes_lyrics_following() {
        let mut app = App {
            now_playing_tab: super::NowPlayingTab::Lyrics,
            ..App::default()
        };
        app.lyrics_follow_paused = true;
        app.seek_playback(Duration::from_secs(12));
        assert!(!app.lyrics_follow_paused);
        assert!(app.lyrics_resync);
    }

    #[test]
    fn a_backward_jump_brings_the_lyrics_back_and_resumes_following() {
        let mut app = App {
            now_playing_tab: super::NowPlayingTab::Lyrics,
            ..App::default()
        };
        app.preferences.lyrics_follow = true;
        let line = |start: f64| crate::lyrics::Line {
            start,
            end: None,
            text: format!("line at {start}"),
            words: Vec::new(),
            translation: None,
            romanization: None,
            background: None,
        };
        app.lyrics = Some(crate::lyrics::Lyrics {
            source: "fixture".into(),
            lines: [0.0, 10.0, 20.0].into_iter().map(line).collect(),
            plain: None,
        });
        let track = song("Track");
        let at = |seconds: f64| PlaybackStatus {
            track: Some(track.clone()),
            position: Duration::from_secs_f64(seconds),
            state: PlaybackState::Playing,
            ..PlaybackStatus::default()
        };

        drop(app.update_playback_status(at(25.0)));
        assert_eq!(app.lyric_active, Some(2));
        // The user scrolled the lyrics by hand, which paused following.
        app.lyrics_follow_paused = true;
        app.lyrics_auto_scroll_until = None;

        drop(app.update_playback_status(at(12.0)));
        assert_eq!(app.lyric_active, Some(1));
        assert!(!app.lyrics_follow_paused, "a jump back resumes following");
        assert!(
            app.lyrics_auto_scroll_until.is_some(),
            "the view scrolls to the line that is now sung"
        );
    }

    #[test]
    fn settled_window_resize_preloads_artwork_for_the_wider_viewport() {
        let mut app = with_picks(App {
            page: Page::Home,
            window_width: 480.0,
            window_height: 640.0,
            home_shelves: vec![shelf_with_artwork(0)],
            ..App::default()
        });

        let area = iced::Size::new(1214.0, 520.0);
        let resize_task = app.update(Message::AreaMeasured(area));
        assert!(
            app.artwork_requested.is_empty(),
            "work waits for the resize to settle"
        );

        let artwork_task = app.update(Message::ResizeWorkDue(app.resize_work_generation));

        let layout = app.home_layout(area);
        assert!(app.artwork_requested.contains(&(
            "cover-0-11".into(),
            crate::artwork::variant_for_pixels(layout.card)
        )));
        drop((resize_task, artwork_task));
    }

    #[test]
    fn display_scale_change_reloads_home_artwork_at_the_needed_size() {
        let mut app = with_picks(App {
            page: Page::Home,
            home_shelves: vec![shelf_with_artwork(0)],
            ..App::default()
        });

        let task = app.update(Message::DisplayScaleChanged(2.0));

        assert_eq!(app.window_scale_factor, 2.0);
        let layout = app.home_layout(app.area);
        // The doubled size, downgraded only as far as the decode budget needs.
        let variant = App::effective_variant_for_count(
            crate::artwork::variant_for_pixels(layout.card * 2.0),
            app.home_picks.items.len(),
        );
        assert!(variant > crate::artwork::variant_for_pixels(layout.card));
        assert!(
            app.artwork_requested
                .contains(&("cover-0-0".into(), variant))
        );
        drop(task);
    }

    #[test]
    fn home_preload_does_not_queue_more_artwork_than_the_cache_can_keep() {
        let mut app = with_picks(App {
            page: Page::Home,
            home_shelves: (0..10).map(shelf_with_artwork).collect(),
            window_width: 2560.0,
            window_height: 1440.0,
            ..App::default()
        });

        let task = app.preload_current_page_artwork();

        // Every card of the one Home shelf, at a size the cache can hold.
        assert_eq!(app.artwork_requested.len(), 20);
        assert!(app.artwork_requested.len() <= crate::artwork::MAX_ARTWORK_ENTRIES);
        let requested_bytes: usize = app
            .artwork_requested
            .iter()
            .map(|(_, dim)| {
                (*dim as usize)
                    .saturating_mul(*dim as usize)
                    .saturating_mul(4)
            })
            .sum();
        assert!(requested_bytes <= crate::artwork::MAX_ARTWORK_BYTES);
        drop(task);
    }

    #[test]
    fn artwork_scroll_reconciliation_keeps_overlap_and_rejects_cancelled_results() {
        use iced::widget::image;
        let mut app = App::default();
        let keep_key = ("keep".to_owned(), crate::artwork::THUMB_MAX);
        let stale_key = ("stale".to_owned(), crate::artwork::THUMB_MAX);
        let keep_token = tokio_util::sync::CancellationToken::new();
        let stale_token = tokio_util::sync::CancellationToken::new();
        let keep_id = app.next_artwork_request_id();
        let stale_id = app.next_artwork_request_id();
        app.artwork_requested
            .extend([keep_key.clone(), stale_key.clone()]);
        app.artwork_batch_tokens
            .insert(keep_key.clone(), (keep_id, keep_token.clone()));
        app.artwork_batch_tokens
            .insert(stale_key.clone(), (stale_id, stale_token.clone()));
        let task = app.reconcile_artwork(vec![
            ("keep".into(), crate::artwork::THUMB_MAX),
            ("new".into(), crate::artwork::THUMB_MAX),
        ]);

        assert!(!keep_token.is_cancelled());
        assert!(stale_token.is_cancelled());
        assert_eq!(
            app.artwork_batch_tokens.get(&keep_key).map(|(id, _)| *id),
            Some(keep_id)
        );
        let new_id = app
            .artwork_batch_tokens
            .get(&("new".into(), crate::artwork::THUMB_MAX))
            .map(|(id, _)| *id)
            .unwrap();

        let stale_result = app.update(Message::ArtworkLoaded {
            url: "stale".into(),
            max_dim: crate::artwork::THUMB_MAX,
            generation: app.artwork_generation,
            request_id: stale_id,
            handle: Some(image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255])),
            selected: false,
        });
        assert!(!app.artwork_cache.contains_key("stale"));
        drop(stale_result);

        let new_result = app.update(Message::ArtworkLoaded {
            url: "new".into(),
            max_dim: crate::artwork::THUMB_MAX,
            generation: app.artwork_generation,
            request_id: new_id,
            handle: Some(image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255])),
            selected: false,
        });
        assert!(app.artwork_cache.contains_key("new"));
        drop((task, new_result));
        app.cancel_artwork_batch();
    }

    #[test]
    fn collection_covers_share_the_bounded_cache_and_are_released_on_leave() {
        use crate::music::BrowseTarget;
        use iced::widget::image;
        let cover = || image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255]);
        let mut app = App::default();
        for index in 0..crate::artwork::MAX_ARTWORK_ENTRIES {
            app.cache_artwork_handle(format!("art-{index}"), crate::artwork::CARD_MAX, cover());
        }
        drop(app.reconcile_artwork(vec![("art-0".into(), crate::artwork::CARD_MAX)]));
        app.cache_artwork_handle(
            format!("art-{}", crate::artwork::MAX_ARTWORK_ENTRIES),
            crate::artwork::CARD_MAX,
            cover(),
        );
        assert!(app.artwork_cache.contains_key("art-0"));
        assert!(!app.artwork_cache.contains_key("art-1"));

        let target = BrowseTarget {
            browse_id: "VLfixture".into(),
            params: None,
            page_type: Some("MUSIC_PAGE_TYPE_PLAYLIST".into()),
        };
        app.page = Page::CollectionDetail(target);
        let request_id = app.next_artwork_request_id();
        app.artwork_batch_tokens.insert(
            ("collection-art".into(), crate::artwork::THUMB_MAX),
            (request_id, tokio_util::sync::CancellationToken::new()),
        );
        app.artwork_requested
            .insert(("collection-art".into(), crate::artwork::THUMB_MAX));
        let task = app.update(Message::ArtworkLoaded {
            url: "collection-art".into(),
            max_dim: crate::artwork::THUMB_MAX,
            generation: app.artwork_generation,
            request_id,
            handle: Some(cover()),
            selected: false,
        });
        assert!(app.artwork_cache.contains_key("collection-art"));
        drop(task);
        // Leaving the collection releases covers the destination does not show.
        drop(app.update(Message::Navigate(Page::Home)));
        assert!(!app.artwork_cache.contains_key("collection-art"));
    }

    #[test]
    fn artwork_cache_evicts_by_bytes_not_only_entry_count() {
        use crate::artwork::MAX_ARTWORK_BYTES;
        use iced::widget::image;
        let mut app = App::default();
        // 640x640 RGBA = 1_638_400 bytes; eleven exceed the 16 MiB budget.
        for index in 0..11 {
            app.cache_artwork_handle(
                format!("large-{index}"),
                crate::artwork::LARGE_MAX,
                image::Handle::from_rgba(640, 640, vec![0u8; 640 * 640 * 4]),
            );
        }
        assert!(app.artwork_cache.bytes() <= MAX_ARTWORK_BYTES);
        assert!(!app.artwork_cache.contains_key("large-0"));
        assert!(app.artwork_cache.contains_key("large-10"));
        // Re-touch keeps the visible cover.
        drop(app.reconcile_artwork(vec![("large-10".into(), crate::artwork::LARGE_MAX)]));
        app.cache_artwork_handle(
            "large-11".into(),
            crate::artwork::LARGE_MAX,
            image::Handle::from_rgba(640, 640, vec![0u8; 640 * 640 * 4]),
        );
        assert!(app.artwork_cache.contains_key("large-10"));
        assert!(!app.artwork_cache.contains_key("large-1"));
    }

    #[test]
    fn catalog_continuation_merges_unique_results_and_rejects_stale_pages() {
        use crate::music::{CatalogPage, Continuation, HomeItem, HomeItemKind};
        let item = |id: &str| HomeItem {
            title: format!("Title {id}"),
            subtitle: "Artist".into(),
            thumbnail_url: None,
            kind: HomeItemKind::Song,
            browse_id: None,
            browse_target: None,
            track: Some(fixture_track(id)),
            duration: None,
        };
        let mut app = App {
            page: Page::Search,
            search_committed: true,
            search_query: "artist".into(),
            search_request_id: 7,
            search_loading: true,
            catalog_more_loading: true,
            catalog_results: vec![item("first")],
            catalog_continuation: Some(Continuation {
                token: "next".into(),
            }),
            ..App::default()
        };
        drop(app.update(Message::CatalogMoreCompleted {
            request_id: 6,
            result: Some(Ok(CatalogPage {
                items: vec![item("stale")],
                continuation: None,
            })),
        }));
        assert_eq!(app.catalog_results.len(), 1);

        drop(app.update(Message::CatalogMoreCompleted {
            request_id: 7,
            result: Some(Ok(CatalogPage {
                items: vec![item("first"), item("second")],
                continuation: Some(Continuation {
                    token: "next-2".into(),
                }),
            })),
        }));
        assert_eq!(app.catalog_results.len(), 2);
        assert_eq!(app.search_results.len(), 2);
        assert_eq!(
            app.catalog_continuation
                .as_ref()
                .map(|value| value.token.as_str()),
            Some("next-2")
        );
        assert!(!app.catalog_more_loading);
        assert!(!app.search_loading);
    }

    #[test]
    fn navigation_updates_the_active_page() {
        let mut app = App::default();

        let _task = app.update(Message::Navigate(Page::Library));

        assert_eq!(app.page, Page::Library);
    }

    #[test]
    fn back_restores_previous_routes_without_discarding_older_history() {
        let mut app = App::default();
        let _ = app.update(Message::Navigate(Page::Search));
        let _ = app.update(Message::Navigate(Page::Library));

        assert_eq!(app.history, vec![Page::Home, Page::Search]);
        let _ = app.update(Message::Back);
        assert_eq!(app.page, Page::Search);
        assert_eq!(app.history, vec![Page::Home]);
        let _ = app.update(Message::Back);
        assert_eq!(app.page, Page::Home);
        assert!(app.history.is_empty());
    }

    #[test]
    fn collection_back_bar_and_escape_return_to_the_results_with_their_scroll() {
        use iced::keyboard::{Event as KeyEvent, Key, Modifiers, key::Named};
        let target = crate::music::BrowseTarget {
            browse_id: "MPREb_fixture".into(),
            params: None,
            page_type: None,
        };
        let mut app = App {
            page: Page::Search,
            search_scroll: AbsoluteOffset { x: 0.0, y: 480.0 },
            ..App::default()
        };
        let _ = app.update(Message::Navigate(Page::CollectionDetail(target.clone())));
        assert!(matches!(app.back_target(), Some(Message::Back)));
        let _ = app.update(Message::Back);
        assert_eq!(app.page, Page::Search);
        assert_eq!(app.search_scroll.y, 480.0, "the results keep their scroll");

        let _ = app.update(Message::Navigate(Page::CollectionDetail(target)));
        let escape = KeyEvent::KeyPressed {
            key: Key::Named(Named::Escape),
            modified_key: Key::Named(Named::Escape),
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::Escape),
            location: iced::keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        };
        let _ = app.update(Message::KeyPressed(escape));
        assert_eq!(app.page, Page::Search);
        assert_eq!(app.search_scroll.y, 480.0);
    }

    #[test]
    fn a_held_key_acts_once_and_ctrl_k_follows_the_physical_key() {
        use iced::keyboard::{Event as KeyEvent, Key, Modifiers, key::Named};
        let mut app = App::default();
        let _ = app.update(Message::Navigate(Page::Settings));
        let press =
            |key: Key, physical: iced::keyboard::key::Code, modifiers: Modifiers, repeat: bool| {
                KeyEvent::KeyPressed {
                    modified_key: key.clone(),
                    key,
                    physical_key: iced::keyboard::key::Physical::Code(physical),
                    location: iced::keyboard::Location::Standard,
                    modifiers,
                    text: None,
                    repeat,
                }
            };
        let _ = app.update(Message::KeyPressed(press(
            Key::Named(Named::Escape),
            iced::keyboard::key::Code::Escape,
            Modifiers::empty(),
            true,
        )));
        assert_eq!(
            app.page,
            Page::Settings,
            "a held Escape must not leave the page"
        );
        let _ = app.update(Message::KeyPressed(press(
            Key::Named(Named::Escape),
            iced::keyboard::key::Code::Escape,
            Modifiers::empty(),
            false,
        )));
        assert_ne!(app.page, Page::Settings, "a single Escape leaves the page");

        let _ = app.update(Message::Navigate(Page::Settings));
        let _ = app.update(Message::KeyPressed(press(
            Key::Character("к".into()),
            iced::keyboard::key::Code::KeyK,
            Modifiers::CTRL,
            false,
        )));
        assert_eq!(app.page, Page::Home, "Ctrl+K works with a non-Latin layout");
    }

    #[test]
    fn copy_notice_hides_only_when_the_latest_copy_expires() {
        let mut app = App::default();
        drop(app.update(Message::CopyToClipboard("Title".into())));
        let first = app.copy_notice_count;
        drop(app.update(Message::CopyToClipboard("Artist".into())));
        let latest = app.copy_notice_count;
        assert!(app.copy_notice.is_some());
        // The timer of the first copy must not hide the notice of the second.
        drop(app.update(Message::CopyNoticeExpired(first)));
        assert!(app.copy_notice.is_some());
        drop(app.update(Message::CopyNoticeExpired(latest)));
        assert!(app.copy_notice.is_none());
    }

    #[test]
    fn now_playing_back_bar_returns_to_the_page_it_was_opened_from() {
        let mut app = App {
            page: Page::Search,
            ..App::default()
        };
        app.playback_status.track = Some(song("Track"));
        let _ = app.update(Message::OpenNowPlaying);
        assert_eq!(app.page, Page::NowPlaying);
        assert!(matches!(app.back_target(), Some(Message::CloseNowPlaying)));
        let _ = app.update(Message::CloseNowPlaying);
        assert_eq!(app.page, Page::Search);
        assert!(
            App::default().back_target().is_none(),
            "Home has nowhere to go back to"
        );
    }

    #[test]
    fn history_discards_oldest_beyond_forty_and_keeps_newest() {
        let mut app = App::default();
        // 45 distinct navigations: history must keep the 40 most recent.
        for i in 0..45 {
            let page = if i % 2 == 0 {
                Page::Search
            } else {
                Page::Library
            };
            // Force distinct pages so navigate_internal records each step.
            if app.page == page {
                let _ = app.update(Message::Navigate(Page::Home));
            }
            let _ = app.update(Message::Navigate(page));
        }
        assert_eq!(app.history.len(), 40);
        // The first five entries are discarded, leaving the alternating
        // Search/Library routes from the newest forty navigations.
        assert_eq!(app.page, Page::Search);
        assert_eq!(app.history.first(), Some(&Page::Search));
        assert_eq!(app.history.last(), Some(&Page::Library));
        assert!(app.history.iter().enumerate().all(|(index, page)| {
            if index % 2 == 0 {
                page == &Page::Search
            } else {
                page == &Page::Library
            }
        }));
        // Back still works after overflow.
        let before = app.page.clone();
        let _ = app.update(Message::Back);
        assert_eq!(app.page, Page::Library);
        assert_ne!(app.page, before);
    }

    #[test]
    fn obsolete_home_refresh_cannot_replace_the_current_response() {
        let mut app = App::default();
        let task = app.update(Message::ReloadHome);
        let first = app.home_cancel.clone().expect("refresh token is set");
        let newer_task = app.update(Message::ReloadHome);
        assert!(first.is_cancelled());
        let current_generation = app.home_generation;

        let _ = app.update(Message::HomeLoaded {
            generation: current_generation.wrapping_sub(1),
            result: Some(Err("obsolete response".into())),
        });
        assert!(app.home_loading);
        assert!(app.home_error.is_none());
        drop((task, newer_task));
    }

    #[tokio::test]
    async fn successful_home_load_replaces_loading_placeholder_state() {
        use crate::music::HomeShelf;

        let mut app = App {
            home_generation: 7,
            home_loading: true,
            ..App::default()
        };
        let task = app.update(Message::HomeLoaded {
            generation: 7,
            result: Some(Ok(vec![HomeShelf {
                title: "Fresh shelf".into(),
                items: vec![],
            }])),
        });

        assert!(!app.home_loading);
        assert_eq!(app.home_shelves.len(), 1);
        assert_eq!(app.home_shelves[0].title, "Fresh shelf");
        assert!(app.home_error.is_none());
        drop(task);
    }

    #[tokio::test]
    async fn supplemental_home_sections_append_after_primary_without_duplicates() {
        use crate::music::HomeShelf;

        let mut app = App {
            home_generation: 7,
            home_shelves: vec![HomeShelf {
                title: "Primary mix shelf".into(),
                items: vec![],
            }],
            ..App::default()
        };
        let task = app.update(Message::HomeMoreLoaded {
            generation: 7,
            result: Some(Ok(vec![
                HomeShelf {
                    title: "Primary mix shelf".into(),
                    items: vec![],
                },
                HomeShelf {
                    title: "New releases".into(),
                    items: vec![],
                },
            ])),
        });

        assert_eq!(app.home_shelves.len(), 2);
        assert_eq!(app.home_shelves[0].title, "Primary mix shelf");
        assert_eq!(app.home_shelves[1].title, "New releases");
        drop(task);
    }

    #[test]
    fn stale_supplemental_home_response_is_discarded() {
        use crate::music::HomeShelf;

        let mut app = App {
            home_generation: 2,
            home_shelves: vec![HomeShelf {
                title: "Current shelf".into(),
                items: vec![],
            }],
            ..App::default()
        };
        let _ = app.update(Message::HomeMoreLoaded {
            generation: 1,
            result: Some(Ok(vec![HomeShelf {
                title: "Obsolete shelf".into(),
                items: vec![],
            }])),
        });

        assert_eq!(app.home_shelves.len(), 1);
        assert_eq!(app.home_shelves[0].title, "Current shelf");
    }

    #[test]
    fn early_plain_lyrics_remain_visible_while_synced_lookup_finishes() {
        use crate::lyrics::Lyrics;
        let early = Lyrics {
            source: "LRCLIB".into(),
            lines: Vec::new(),
            plain: Some("Available text".into()),
        };
        let mut app = App {
            lyrics_generation: 4,
            lyrics_loading: true,
            ..App::default()
        };
        drop(app.update(Message::LyricsProgress {
            generation: 4,
            lyrics: early.clone(),
        }));
        assert!(app.lyrics_loading);
        assert_eq!(
            app.lyrics.as_ref().unwrap().plain.as_deref(),
            Some("Available text")
        );

        drop(app.update(Message::LyricsProgress {
            generation: 3,
            lyrics: Lyrics {
                source: "stale".into(),
                lines: Vec::new(),
                plain: Some("old song".into()),
            },
        }));
        assert_eq!(app.lyrics.as_ref().unwrap().source, "LRCLIB");

        drop(app.update(Message::LyricsLoaded {
            generation: 4,
            result: Err("Synced lyrics could not load".into()),
        }));
        assert!(!app.lyrics_loading);
        assert_eq!(
            app.lyrics.as_ref().unwrap().plain.as_deref(),
            Some("Available text")
        );
        assert!(app.lyrics_error.is_some());
    }

    #[test]
    fn newer_query_cancels_previous_suggestions_without_starting_results() {
        let mut app = App::default();
        let first_task = app.update(Message::SearchChanged("first query".into()));
        let first_request = app.suggest_cancel.clone().expect("suggestion token is set");
        let second_task = app.update(Message::SearchChanged("second query".into()));

        assert!(first_request.is_cancelled());
        assert!(!app.search_loading);
        assert!(!app.search_committed);
        assert_eq!(app.search_query, "second query");

        drop((first_task, second_task));
    }

    #[test]
    fn search_opens_results_and_starts_without_debounce() {
        let mut app = App::default();

        let task = app.update(Message::SearchFor("chill music".into()));

        assert_eq!(app.page, Page::Search);
        assert_eq!(app.search_query, "chill music");
        assert!(app.search_loading);
        drop(task);
    }

    #[test]
    fn enter_submits_the_current_query_immediately() {
        let mut app = App {
            search_query: "Daft Punk".into(),
            ..App::default()
        };

        let task = app.update(Message::SearchSubmitted);

        assert_eq!(app.page, Page::Search);
        assert_eq!(app.search_query, "Daft Punk");
        assert!(app.search_loading);
        drop(task);
    }
    #[test]
    fn home_detail_back_keeps_shelves_scroll_and_artwork_and_cancels_request() {
        use crate::music::{BrowseTarget, HomeShelf};
        use iced::widget::{image, operation::AbsoluteOffset};
        let mut app = App {
            home_shelves: vec![HomeShelf {
                title: "Existing Home".into(),
                items: vec![crate::music::HomeItem {
                    title: "Mix".into(),
                    subtitle: String::new(),
                    thumbnail_url: Some("fixture-art".into()),
                    kind: crate::music::HomeItemKind::Playlist,
                    browse_id: Some("VLmix".into()),
                    browse_target: Some(BrowseTarget {
                        browse_id: "VLmix".into(),
                        params: None,
                        page_type: Some("MUSIC_PAGE_TYPE_PLAYLIST".into()),
                    }),
                    track: None,
                    duration: None,
                }],
            }],
            home_loading: false,
            home_scroll: AbsoluteOffset { x: 0.0, y: 480.0 },
            ..App::default()
        };
        let shelves = app.home_shelves.clone();
        app.home_picks.rebuild(&shelves);
        app.artwork_cache.insert(
            "fixture-art".into(),
            crate::artwork::LARGE_MAX,
            image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255]),
        );
        let target = BrowseTarget {
            browse_id: "VLfixture".into(),
            params: Some("fixture-params".into()),
            page_type: Some("MUSIC_PAGE_TYPE_PLAYLIST".into()),
        };
        let _ = app.update(Message::OpenCollection(target.clone()));
        assert_eq!(app.page, Page::CollectionDetail(target));
        assert!(app.collection.loading);
        let generation = app.collection.generation;
        let cancellation = app.collection.cancellation.clone().unwrap();
        let _ = app.update(Message::Back);
        assert_eq!(app.page, Page::Home);
        assert!(cancellation.is_cancelled());
        assert_eq!(app.home_shelves[0].title, "Existing Home");
        assert_eq!(app.home_scroll.y, 480.0);
        assert!(app.artwork_cache.contains_key("fixture-art"));
        assert!(!app.home_loading);
        let _ = app.update(Message::CollectionLoaded {
            generation,
            result: Err("obsolete".into()),
        });
        assert!(app.collection.error.is_none());
        assert_eq!(app.page, Page::Home);
    }

    fn fixture_track(id: &str) -> crate::music::Track {
        crate::music::Track {
            duration_secs: None,
            video_id: id.into(),
            title: format!("Title {id}"),
            artist: "Artist".into(),
            album: None,
            thumbnail_url: None,
        }
    }

    #[test]
    fn each_track_change_releases_only_the_previous_large_cover() {
        let mut app = App::default();
        let tracks: Vec<crate::music::Track> = (0..4)
            .map(|index| {
                let mut track = fixture_track(&format!("cover-{index}"));
                track.thumbnail_url = Some(format!(
                    "https://i.ytimg.com/vi/cover-{index}/hqdefault.jpg"
                ));
                track
            })
            .collect();
        let queue: Arc<[crate::music::Track]> = Arc::from(tracks.clone());
        let side = crate::artwork::LARGE_MAX;
        for (index, track) in tracks.iter().enumerate() {
            let _ = app.update(Message::PlayContext(Arc::clone(&queue), index));
            for earlier in &tracks[..index] {
                let url = earlier.thumbnail_url.as_deref().unwrap_or_default();
                assert!(
                    app.artwork_cache.get_exact(url, side).is_none(),
                    "after track {index} the cover of {url} is still cached"
                );
            }
            // The cover of the track that just started is cached when it loads.
            let url = track.thumbnail_url.clone().unwrap_or_default();
            let pixels = vec![0; (side * side * 4) as usize];
            app.artwork_cache
                .insert(url, side, image::Handle::from_rgba(side, side, pixels));
        }
        let last = tracks
            .last()
            .and_then(|track| track.thumbnail_url.as_deref());
        assert!(
            app.artwork_cache
                .get_exact(last.unwrap_or_default(), side)
                .is_some()
        );
    }

    #[test]
    fn discord_presence_clears_stale_tracks_without_clearing_same_track_buffering() {
        let mut app = App::default();
        app.preferences.discord_presence = true;
        app.playback_status.track = Some(fixture_track("new"));
        app.playback_status.state = PlaybackState::Buffering;
        app.discord_presence_enabled_last = true;
        app.discord_presence_key = Some(("old".into(), false));
        app.discord_presence_updated_at = Some(std::time::Instant::now());

        app.sync_discord_presence(false);
        assert!(app.discord_presence_key.is_none());
        assert!(app.discord_presence_updated_at.is_none());

        app.discord_presence_key = Some(("new".into(), false));
        app.discord_presence_updated_at = Some(std::time::Instant::now());
        app.sync_discord_presence(false);
        assert_eq!(
            app.discord_presence_key.as_ref().map(|key| key.0.as_str()),
            Some("new")
        );

        app.playback_status.state = PlaybackState::Error;
        app.sync_discord_presence(false);
        assert!(app.discord_presence_key.is_none());
        assert!(app.discord_presence_updated_at.is_none());
    }

    #[test]
    fn output_device_selection_is_stored_in_preferences() {
        let mut app = App::default();
        let _ = app.update(Message::Preference(Preference::OutputDevice(Some(
            "alsa:device-123".into(),
        ))));
        assert_eq!(
            app.preferences.audio_output_device_id.as_deref(),
            Some("alsa:device-123")
        );
    }

    #[test]
    fn play_context_sets_queue_and_next_prev_move_within_it() {
        let mut app = App::default();
        let tracks: Arc<[crate::music::Track]> =
            vec![fixture_track("a"), fixture_track("b"), fixture_track("c")].into();
        let _ = app.update(Message::PlayContext(Arc::clone(&tracks), 0));
        assert_eq!(app.queue.len(), 3);
        assert_eq!(app.queue_index, Some(0));
        assert_eq!(app.playback_status.track.as_ref().unwrap().video_id, "a");

        let _ = app.update(Message::NextTrack);
        assert_eq!(app.queue_index, Some(1));
        assert_eq!(app.playback_status.track.as_ref().unwrap().video_id, "b");

        let _ = app.update(Message::PrevTrack);
        // Within 0s of start, Prev goes back to index 0.
        assert_eq!(app.queue_index, Some(0));

        // Next past the end stays at the last item.
        let _ = app.update(Message::PlayContext(tracks, 2));
        let _ = app.update(Message::NextTrack);
        assert_eq!(app.queue_index, Some(2));
    }

    #[test]
    fn reorder_queue_preserves_current_track_and_repeat_one_does_not_block_manual_next() {
        let mut app = App {
            queue: vec![fixture_track("a"), fixture_track("b"), fixture_track("c")],
            queue_index: Some(1),
            repeat: super::RepeatMode::One,
            autoplay: false,
            ..App::default()
        };
        let _ = app.update(Message::MoveQueue(0, 2));
        assert_eq!(
            app.queue
                .iter()
                .map(|t| t.video_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );
        assert_eq!(app.queue_index, Some(0));
        let _ = app.update(Message::NextTrack);
        assert_eq!(app.queue_index, Some(1));
        assert_eq!(app.playback_status.track.as_ref().unwrap().video_id, "c");
    }
    #[test]
    fn clearing_search_query_invalidates_outstanding_suggestions() {
        let mut app = App {
            search_query: "old query".into(),
            suggest_request_id: 1,
            ..App::default()
        };
        let task = app.update(Message::SearchChanged(String::new()));
        assert_eq!(app.suggest_request_id, 2);
        assert!(!app.show_suggestions);
        let _ = app.update(Message::SuggestionsCompleted {
            request_id: 1,
            result: Some(vec!["stale".into()]),
        });
        assert!(app.suggestions.is_empty());
        drop(task);
    }

    #[test]
    fn select_track_without_context_creates_single_item_queue() {
        let mut app = App::default();
        let track = fixture_track("solo");
        let _ = app.update(Message::SelectTrack(track.clone()));
        assert_eq!(app.queue.len(), 1);
        assert_eq!(app.queue_index, Some(0));
        assert_eq!(app.playback_status.track.as_ref().unwrap().video_id, "solo");
    }

    #[test]
    fn double_tap_on_resolving_track_does_not_restart_resolution() {
        let mut app = App::default();
        let tracks: Arc<[crate::music::Track]> =
            vec![fixture_track("a"), fixture_track("b")].into();
        let _ = app.update(Message::PlayContext(Arc::clone(&tracks), 0));
        // Still resolving the same track: a second tap is ignored instead
        // of cancelling the in-flight resolution.
        let _ = app.update(Message::PlayContext(tracks, 0));
        assert_eq!(app.queue_index, Some(0));
        assert_eq!(app.playback_status.track.as_ref().unwrap().video_id, "a");
    }

    #[test]
    fn liking_the_playing_track_keeps_its_length_for_favorites() {
        let mut app = App::default();
        app.playback_status.track = Some(fixture_track("playing"));
        app.playback_status.duration = Some(Duration::from_secs(215));
        drop(app.update(Message::LikeTrack(fixture_track("playing"))));
        assert_eq!(app.favorites[0].duration_secs, Some(215));
    }

    #[test]
    fn removing_and_clearing_the_queue_keep_the_playing_track() {
        let mut app = App {
            queue: vec![
                fixture_track("a"),
                fixture_track("b"),
                fixture_track("c"),
                fixture_track("d"),
            ],
            queue_index: Some(1),
            ..App::default()
        };
        // Index 1 is the playing track, so it stays.
        drop(app.update(Message::RemoveFromQueue(1)));
        assert_eq!(app.queue[1].video_id, "b");
        drop(app.update(Message::RemoveFromQueue(2)));
        assert_eq!(app.queue_index, Some(1));
        assert_eq!(app.queue.len(), 3);
        drop(app.update(Message::RemoveFromQueue(0)));
        assert_eq!(app.queue_index, Some(0));
        assert_eq!(app.queue[0].video_id, "b");
        assert_eq!(app.queue.len(), 2);
        drop(app.update(Message::ClearUpcomingQueue));
        assert_eq!(app.queue.len(), 1);
        assert_eq!(app.queue_index, Some(0));
    }

    #[test]
    fn ctrl_k_opens_home_with_the_search_field() {
        use iced::keyboard::{Event as KeyEvent, Key, Modifiers};
        let mut app = App {
            page: Page::Library,
            ..App::default()
        };
        let _ = app.update(Message::KeyPressed(KeyEvent::KeyPressed {
            key: Key::Character("k".into()),
            modified_key: Key::Character("k".into()),
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyK),
            location: iced::keyboard::Location::Standard,
            modifiers: Modifiers::CTRL,
            text: None,
            repeat: false,
        }));
        assert_eq!(app.page, Page::Home);
    }

    #[test]
    fn suggestions_fallback_offers_raw_query_when_service_is_empty() {
        let mut app = App {
            search_query: "daft".into(),
            suggest_request_id: 1,
            ..App::default()
        };
        let _ = app.update(Message::SuggestionsCompleted {
            request_id: 1,
            result: Some(Vec::new()),
        });
        assert!(app.show_suggestions);
        assert_eq!(app.suggestions, vec!["daft"]);
    }

    #[test]
    fn window_chrome_messages_leave_state_untouched() {
        let mut app = App::default();
        let before = (app.page.clone(), app.search_query.clone(), app.queue.len());
        for msg in [
            Message::StartDrag,
            Message::MinimizeWindow,
            Message::ToggleMaximize,
            Message::CloseWindow,
        ] {
            drop(app.update(msg));
        }
        assert_eq!(
            before,
            (app.page.clone(), app.search_query.clone(), app.queue.len(),)
        );
    }

    #[test]
    fn single_character_input_never_starts_results_or_suggestions() {
        let mut app = App {
            page: Page::Search,
            ..App::default()
        };
        let task = app.update(Message::SearchChanged("a".into()));
        assert_eq!(app.search_query, "a");
        assert!(!app.search_loading);
        assert!(!app.search_committed);
        assert!(!app.show_suggestions);
        assert!(app.suggestions.is_empty());
        assert!(app.search_results.is_empty());
        drop(task);
    }

    #[test]
    fn shuffle_context_enables_shuffle_and_starts_from_first() {
        let mut app = App::default();
        let tracks: Arc<[crate::music::Track]> = vec![
            fixture_track("a"),
            fixture_track("b"),
            fixture_track("c"),
            fixture_track("d"),
        ]
        .into();
        let _ = app.update(Message::ShuffleContext(tracks));
        assert!(app.shuffle);
        assert_eq!(app.queue.len(), 4);
        assert_eq!(app.queue_index, Some(0));
        let mut ids: Vec<_> = app.queue.iter().map(|t| t.video_id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn play_context_shares_one_allocation_per_list() {
        // Per-row messages share one list allocation; playback keeps only its
        // bounded queue window when the source list is larger than the queue.
        for size in [100, 1000, 10_000] {
            let tracks: Arc<[crate::music::Track]> = (0..size)
                .map(|i| fixture_track(&format!("t{i}")))
                .collect::<Vec<_>>()
                .into();
            let base_ptr = Arc::as_ptr(&tracks);
            // Cloning the Arc per row must not copy the Track vector.
            let mut strong_after_clones = 0;
            for index in [0, size / 2, size - 1] {
                let shared = Arc::clone(&tracks);
                assert!(std::ptr::addr_eq(Arc::as_ptr(&shared), base_ptr));
                strong_after_clones = Arc::strong_count(&shared);
                let mut app = App::default();
                let _ = app.update(Message::PlayContext(shared, index));
                let (expected_queue, expected_cursor) =
                    bounded_queue_context(&tracks, index, crate::preferences::MAX_QUEUE_TRACKS);
                assert_eq!(app.queue_index, Some(expected_cursor));
                assert_eq!(
                    app.playback_status.track.as_ref().unwrap().video_id,
                    format!("t{index}")
                );
                assert_eq!(app.queue.len(), expected_queue.len());
                assert_eq!(app.queue[expected_cursor].video_id, format!("t{index}"));
            }
            // Original + last clone alive here: count grows linearly, not quadratically.
            assert!(strong_after_clones >= 2);
            drop(tracks);
        }
    }

    #[test]
    fn play_context_selects_correct_track_after_reorder_and_pagination() {
        let mut app = App::default();
        let tracks: Arc<[crate::music::Track]> = vec![
            fixture_track("a"),
            fixture_track("b"),
            fixture_track("c"),
            fixture_track("d"),
        ]
        .into();
        // Simulate paginated second batch appended after first play.
        let _ = app.update(Message::PlayContext(Arc::clone(&tracks), 1));
        assert_eq!(app.playback_status.track.as_ref().unwrap().video_id, "b");
        // Reorder queue (move 0 -> 2) must keep current track identity.
        let _ = app.update(Message::MoveQueue(0, 2));
        assert!(app.queue.iter().any(|t| t.video_id == "b"));
        // A click on the reordered Arc snapshot still resolves by index.
        let reordered: Arc<[crate::music::Track]> = app.queue.clone().into();
        let pos = reordered.iter().position(|t| t.video_id == "c").unwrap();
        let _ = app.update(Message::PlayContext(reordered, pos));
        assert_eq!(app.playback_status.track.as_ref().unwrap().video_id, "c");
        assert_eq!(app.queue_index, Some(pos));
    }

    #[test]
    fn typing_shows_suggestions_until_the_query_is_submitted() {
        let mut app = App {
            page: Page::Search,
            search_results: vec![fixture_track("old")].into(),
            ..App::default()
        };
        let _ = app.update(Message::SearchChanged("new query".into()));
        assert_eq!(app.page, Page::Search);
        assert!(!app.search_loading);
        assert!(app.show_suggestions);
        // Previous results stay visible while typing; submit replaces them.
        assert_eq!(app.search_results.len(), 1);
        assert_eq!(app.search_results[0].video_id, "old");
        let _ = app.update(Message::SearchSubmitted);
        assert!(app.search_loading);
        assert!(app.search_committed);
        assert!(!app.show_suggestions);
    }

    #[test]
    fn submitting_esc_and_navigating_invalidate_pending_suggestions() {
        let mut app = App {
            page: Page::Search,
            ..App::default()
        };
        let _ = app.update(Message::SearchChanged("daft punk".into()));
        let pending = app.suggest_request_id;
        assert!(app.show_suggestions);
        let _ = app.update(Message::SearchSubmitted);
        assert!(!app.show_suggestions);
        // Late response for the invalidated request must not reopen.
        let _ = app.update(Message::SuggestionsCompleted {
            request_id: pending,
            result: Some(vec!["stale".into()]),
        });
        assert!(!app.show_suggestions);

        let _ = app.update(Message::SearchChanged("daft".into()));
        assert!(app.show_suggestions);
        let _ = app.update(Message::DismissSuggestions);
        assert!(!app.show_suggestions);

        let _ = app.update(Message::SearchChanged("radio".into()));
        let stale = app.suggest_request_id;
        let _ = app.update(Message::Navigate(Page::Library));
        assert!(!app.show_suggestions);
        let _ = app.update(Message::SuggestionsCompleted {
            request_id: stale,
            result: Some(vec!["late".into()]),
        });
        assert!(!app.show_suggestions);
    }

    #[test]
    fn suggestion_arrows_and_enter_select_without_click() {
        use iced::keyboard::{Event as KeyEvent, Key, Modifiers, key::Named};
        let mut app = App {
            page: Page::Search,
            suggestions: vec!["one".into(), "two".into()],
            show_suggestions: true,
            ..App::default()
        };
        let key = |named: Named| KeyEvent::KeyPressed {
            key: Key::Named(named),
            modified_key: Key::Named(named),
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::ArrowDown),
            location: iced::keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        };
        let _ = app.update(Message::KeyPressed(key(Named::ArrowDown)));
        assert_eq!(app.suggestion_index, Some(0));
        let _ = app.update(Message::KeyPressed(key(Named::ArrowDown)));
        assert_eq!(app.suggestion_index, Some(1));
        let _ = app.update(Message::KeyPressed(key(Named::ArrowUp)));
        assert_eq!(app.suggestion_index, Some(0));
        // The highlighted suggestion is what the field shows and what Enter searches.
        assert_eq!(app.search_query, app.suggestions[0]);
        let _ = app.update(Message::SearchSubmitted);
        assert_eq!(app.search_query, app.suggestions[0]);
        assert!(app.search_committed);
    }

    #[test]
    fn escape_restores_the_typed_query_after_arrowing() {
        use iced::keyboard::{Event as KeyEvent, Key, Modifiers, key::Named};
        let mut app = App {
            search_query: "daft".into(),
            suggestions: vec!["daft punk".into(), "daft punk get lucky".into()],
            show_suggestions: true,
            ..App::default()
        };
        let key = |named: Named| KeyEvent::KeyPressed {
            key: Key::Named(named),
            modified_key: Key::Named(named),
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::ArrowDown),
            location: iced::keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        };
        let _ = app.update(Message::KeyPressed(key(Named::ArrowDown)));
        assert_eq!(app.search_query, "daft punk");
        let _ = app.update(Message::KeyPressed(key(Named::Escape)));
        assert_eq!(app.search_query, "daft");
        assert!(!app.show_suggestions);
    }

    #[test]
    fn history_chips_support_independent_removal() {
        let mut app = App {
            search_history: vec!["a".into(), "b".into()],
            ..App::default()
        };
        let _ = app.update(Message::RemoveSearchHistoryItem("a".into()));
        assert_eq!(app.search_history, vec!["b".to_string()]);
    }

    #[test]
    fn forward_restores_after_back_and_clears_on_new_navigation() {
        let mut app = App::default();
        let _ = app.update(Message::Navigate(Page::Search));
        let _ = app.update(Message::Navigate(Page::Library));
        let _ = app.update(Message::Back);
        assert_eq!(app.page, Page::Search);
        let _ = app.update(Message::Forward);
        assert_eq!(app.page, Page::Library);
        let _ = app.update(Message::Back);
        let _ = app.update(Message::Navigate(Page::Settings));
        assert!(app.forward.is_empty());
    }

    #[test]
    fn escape_priority_is_suggestions_then_settings() {
        use iced::keyboard::{Event as KeyEvent, Key, Modifiers, key::Named};
        let esc = KeyEvent::KeyPressed {
            key: Key::Named(Named::Escape),
            modified_key: Key::Named(Named::Escape),
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::Escape),
            location: iced::keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        };
        let mut app = App {
            page: Page::Search,
            show_suggestions: true,
            suggestions: vec!["x".into()],
            ..App::default()
        };
        let _ = app.update(Message::KeyPressed(esc.clone()));
        assert!(!app.show_suggestions);
        let mut app = App {
            page: Page::Settings,
            history: vec![Page::Home],
            ..App::default()
        };
        let _ = app.update(Message::KeyPressed(esc.clone()));
        assert_eq!(app.page, Page::Home);
    }

    // ---- Artwork variants: viewport-first, no permanent drop, large window ----
    #[test]
    fn effective_variant_downgrades_to_cover_large_windows_with_reserve() {
        use crate::artwork::{CARD_MAX, CARD_MID_MAX, LARGE_MAX, THUMB_MAX};
        // Few covers: keep requested quality.
        assert_eq!(App::effective_variant_for_count(CARD_MAX, 10), CARD_MAX);
        assert_eq!(App::effective_variant_for_count(LARGE_MAX, 5), LARGE_MAX);
        // Many covers at 320px: downgrade so the window fits + 2 MiB reserve.
        let many = App::effective_variant_for_count(CARD_MAX, 150);
        assert!(many <= CARD_MID_MAX);
        // Even at 128px the reserve is respected (150 thumbs fit in 16 MiB).
        assert_eq!(App::effective_variant_for_count(THUMB_MAX, 150), THUMB_MAX);
    }

    // ---- Artwork: 60 small scrolls, one fetch each, then real cancel on leave ----
    #[test]
    fn sixty_small_scrolls_with_same_urls_fetch_once_and_complete() {
        let mut app = with_picks(App {
            page: Page::Home,
            home_shelves: vec![shelf_with_artwork(0)],
            window_width: 1024.0,
            window_height: 768.0,
            ..App::default()
        });
        // First scroll establishes the window.
        let first = app.preload_current_page_artwork();
        let first_requested = app.artwork_requested.clone();
        assert!(!first_requested.is_empty());
        let first_gen = app.artwork_generation;
        drop(first);
        // 60×1px scrolls with identical visible URLs: coalesced, no cancel,
        // no generation bump, no duplicate fetch.
        for _ in 0..60 {
            app.home_scroll.y += 1.0;
            // Nudge back so the visible shelf window is unchanged (same
            // shelves/columns); only the pixel offset moved.
            app.home_scroll.y -= 1.0;
            let task = app.preload_current_page_artwork();
            assert_eq!(app.artwork_generation, first_gen);
            assert_eq!(app.artwork_requested, first_requested);
            drop(task);
        }
        // In-flight can still complete (generation unchanged).
        let (url, dim) = first_requested.iter().next().unwrap().clone();
        let request_id = app
            .artwork_batch_tokens
            .get(&(url.clone(), dim))
            .map(|(id, _)| *id)
            .unwrap();
        let done = app.update(Message::ArtworkLoaded {
            url: url.clone(),
            max_dim: dim,
            generation: first_gen,
            request_id,
            handle: Some(iced::widget::image::Handle::from_rgba(8, 8, vec![0; 256])),
            selected: false,
        });
        assert!(app.artwork_cache.contains_variant(&url, dim));
        drop(done);
        // Leaving the page cancels everything for real.
        drop(app.update(Message::Navigate(Page::Library)));
        assert!(app.artwork_requested.is_empty());
        assert!(app.artwork_batch_tokens.is_empty());
    }

    // ---- Storage worker: results are generation-guarded ----
    #[test]
    fn stale_storage_results_are_ignored_and_busy_clears() {
        let mut app = App::default();
        let t1 = app.refresh_storage_cache_usage_task();
        let gen1 = app.storage_generation;
        assert!(app.storage_loading);
        // A newer request supersedes the first.
        let t2 = app.refresh_storage_cache_usage_task();
        let gen2 = app.storage_generation;
        assert_ne!(gen1, gen2);
        // Stale completion is ignored, busy stays until the latest lands.
        let _ = app.update(Message::StorageUsageLoaded {
            generation: gen1,
            usage: crate::storage::CacheUsage::default(),
        });
        assert!(app.storage_loading);
        let _ = app.update(Message::StorageUsageLoaded {
            generation: gen2,
            usage: crate::storage::CacheUsage {
                artwork_bytes: 7,
                ..Default::default()
            },
        });
        assert!(!app.storage_loading);
        assert_eq!(app.storage_cache_usage.artwork_bytes, 7);
        drop((t1, t2));
    }
}
