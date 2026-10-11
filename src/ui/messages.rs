//! Messages, and the enums they carry: every event the interface reacts to.

use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Home,
    Search,
    Library,
    Settings,
    NowPlaying,
    CollectionDetail(BrowseTarget),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum NowPlayingTab {
    #[default]
    Queue,
    Lyrics,
}

#[derive(Clone, Debug)]
pub(super) enum Preference {
    RestoreQueue(bool),
    History(bool),
    ShuffleAll(bool),
    ReducedMotion(bool),
    Startup(bool),
    CloseToTray(bool),
    Autoplay(bool),
    DiscordPresence(bool),
    OutputDevice(Option<String>),
    Muted(bool),
    GpuRendering(bool),
    LyricsOffset(i64),
    SongLyricsOffset(i64),
    LyricsFontSize(f32),
    LyricsTranslation(bool),
    LyricsFollow(bool),
    UiLanguage(crate::preferences::UiLanguage),
    UiDensity(crate::preferences::UiDensity),
    ShowLyricsDefault(bool),
    SearchSuggestions(bool),
    TrackNotifications(bool),
    ScaleWithWindow(bool),
}

#[derive(Clone, Debug)]
pub(super) enum Message {
    StartDrag,
    /// Resize from a window edge (the window has no system decorations).
    StartResize(iced::window::Direction),
    MinimizeWindow,
    ToggleMaximize,
    CloseWindow,
    CloseRequested,
    QuitApp,
    /// Moves a queue entry. No control in the interface sends it at the moment.
    #[allow(dead_code)]
    MoveQueue(usize, usize),
    RemoveFromQueue(usize),
    ClearUpcomingQueue,
    Resized {
        width: f32,
        height: f32,
    },
    DisplayScaleChanged(f32),
    LyricsLoaded {
        generation: u64,
        result: Result<crate::lyrics::Lyrics, String>,
    },
    LyricsProgress {
        generation: u64,
        lyrics: crate::lyrics::Lyrics,
    },
    RetryLyrics,
    SeekTo(f64),
    Preference(Preference),
    ClearHistory,
    /// The first press on the history Clear button. It asks first when "Jump back in" goes too.
    AskClearHistory,
    Navigate(Page),
    OpenCollection(BrowseTarget),
    OpenNowPlaying,
    CloseNowPlaying,
    ToggleLike(String),
    ToggleShuffle,
    CycleRepeat,
    SetNowPlayingTab(NowPlayingTab),
    Back,
    Forward,
    RemoveSearchHistoryItem(String),
    /// The pointer entered a recent-search chip, or left it (`None`).
    ChipHovered(Option<String>),
    ClearArtworkCache,
    ClearLyricsCache,
    ClearHomeCache,
    ClearPlayerJsCache,
    ClearBotGuardCache,
    // Async storage worker results. Disk walks/deletes never run in
    // `update`; they complete here with generation-guarded results.
    StorageUsageLoaded {
        generation: u64,
        usage: crate::storage::CacheUsage,
    },
    StorageCleared {
        generation: u64,
        kind: crate::storage::CacheKind,
        cleared_bytes: Result<u64, String>,
        usage: crate::storage::CacheUsage,
    },
    DiagnosticsExported {
        generation: u64,
        result: Result<String, String>,
        usage: crate::storage::CacheUsage,
    },
    LocalDataExported {
        result: Result<String, String>,
    },
    LocalDataImported {
        result: Result<crate::preferences::Snapshot, String>,
    },
    RefreshOutputDevices,
    OutputDevicesLoaded(Result<Vec<crate::playback::output::OutputDeviceChoice>, String>),
    /// The Background portal's answer to a start-at-login request inside a Flatpak.
    StartupRequested {
        enabled: bool,
        result: Result<bool, String>,
    },
    ExportDiagnostics,
    ExportLocalData,
    ImportLocalData,
    LyricsScrolled(AbsoluteOffset),
    ResumeLyricsFollow,
    /// Steps the current song's lyrics offset. No control in the interface sends it at the moment.
    #[allow(dead_code)]
    AdjustLyricsOffset(i64),
    RetryCollection,
    RetryCollectionMore,
    CollectionLoaded {
        generation: u64,
        result: Result<BrowsePage, String>,
    },
    CollectionMoreLoaded {
        generation: u64,
        result: Result<BrowseMore, String>,
    },
    CollectionScrolled(f32),
    SearchScrolled {
        offset: AbsoluteOffset,
        /// The list is close enough to its end to ask for the next page of results.
        near_end: bool,
    },
    LibraryScrolled(AbsoluteOffset),
    QueueScrolled(AbsoluteOffset),
    TogglePlayback,
    NextTrack,
    PrevTrack,
    SetVolume(f32),
    /// Saves state after a slider drag ends; continuous slider moves do not save.
    PersistState,
    VolumeSaveDue(u64),
    SeekChanged(f32),
    SeekReleased,
    PlaybackTick,
    /// The cover of the restored track, read from disk at startup. `None` when no file was saved.
    RestoredCoverLoaded {
        url: String,
        handle: Option<image::Handle>,
    },
    /// Two seconds after Home appears, so the first track skips the player page download.
    PrewarmPlayback,
    /// Fires once the scroll throttle window ends; see `scroll_preload_task`.
    ScrollPreloadDue,
    /// Fires when a failed cover may be requested again; see `schedule_cover_retry`.
    ArtworkRetryDue,
    /// Puts text on the clipboard and shows a short "Copied" notice.
    CopyToClipboard(String),
    /// Hides the "Copied" notice, unless a newer copy has replaced it (the value is that copy's number).
    CopyNoticeExpired(u64),
    /// Fires once a resize settles; see the `Resized` handler.
    ResizeWorkDue(u64),
    /// Diagnostics-only: one step of the `--scroll-test` sweep.
    #[cfg(feature = "diagnostics")]
    DiagnosticScrollStep,
    /// Diagnostics-only: runs one step of the `--screenshots` tour.
    #[cfg(feature = "diagnostics")]
    DiagnosticShot(usize),
    #[cfg(feature = "diagnostics")]
    DiagnosticCapture(usize),
    /// Diagnostics-only: one step of the `CRABYTUBE_CHURN=1` playlist open/close loop.
    #[cfg(feature = "diagnostics")]
    DiagnosticChurn(usize),
    /// Diagnostics-only: one step of the `--soak` schedule (see `diagnostic_soak_task`).
    #[cfg(feature = "diagnostics")]
    DiagnosticSoak(usize),
    #[cfg(feature = "diagnostics")]
    DiagnosticCaptured(usize, iced::window::Screenshot),
    /// A frame the window presented; the tour captures only after one that follows a state change.
    #[cfg(feature = "diagnostics")]
    DiagnosticFrame(std::time::Instant),
    PlaybackStatusChanged(PlaybackStatus),
    MprisCommand(crate::mpris::RemoteCommand),
    TrayServiceStarted(Result<bool, String>),
    TrayCommand(crate::tray::TrayCommand),
    SelectTrack(Track),
    /// Plays the queue entry at this position. A queue row names its place, so a duplicate plays the row clicked.
    PlayQueueIndex(usize),
    PlayContext(std::sync::Arc<[Track]>, usize),
    PlayCollection(std::sync::Arc<[Track]>, usize, QueueSource),
    ShuffleContext(std::sync::Arc<[Track]>),
    ArtworkLoaded {
        url: String,
        max_dim: u32,
        generation: u64,
        request_id: u64,
        handle: Option<image::Handle>,
        selected: bool,
    },
    SearchChanged(String),
    /// Escape was pressed on Home or Search; the flag says whether the search field had focus.
    SearchEscape(bool),
    SearchSubmitted,
    SearchFor(String),
    SuggestionChosen(String),
    DismissSuggestions,
    KeyPressed(iced::keyboard::Event),
    CatalogCompleted {
        request_id: u64,
        result: Option<Result<crate::music::CatalogPage, String>>,
    },
    CatalogMoreCompleted {
        request_id: u64,
        result: Option<Result<crate::music::CatalogPage, String>>,
    },
    RadioLoaded {
        generation: u64,
        result: Result<Vec<Track>, String>,
    },
    RetryRadio,
    SuggestionsCompleted {
        request_id: u64,
        result: Option<Vec<String>>,
    },
    HomeLoaded {
        generation: u64,
        result: Option<Result<Vec<HomeShelf>, String>>,
    },
    HomeMoreLoaded {
        generation: u64,
        result: Option<Result<Vec<HomeShelf>, String>>,
    },
    ReloadHome,
    /// Measured size of the page area (see `shell_view`).
    AreaMeasured(iced::Size),
    /// The window gained or lost focus.
    WindowFocusChanged(bool),
    /// Whether the window is minimized, asked of the window after it lost focus.
    WindowMinimizedChecked(bool),
    /// A minute passed while Home is visible, so the greeting may have changed.
    ClockTick,
    /// Memory and CPU for the Home status line, sent only when the figures shown change.
    StatusReading(crate::process_stats::ProcessReading),
    /// A track notification was sent; zero when it was not.
    TrackNotificationSent(u32),
    /// Scrolls the Home carousel by one screen.
    CarouselStep {
        forward: bool,
    },
    CarouselScrolled(f32),
    CarouselFrame(std::time::Instant),
    /// The volume button of Now Playing: opens the rail, or mutes while it is open.
    ToggleVolumeRail,
    /// Redraws while the volume rail opens or closes.
    VolumeRailFrame(std::time::Instant),
    /// Closes the volume rail once it has not been touched for a while.
    VolumeRailIdle,
    /// Redraws while covers fade in.
    FadeFrame(std::time::Instant),
    ToggleRowMenu(RowMenu),
    PlayNext(Track),
    AddToQueue(Track),
    /// Likes or unlikes a track that may not be in the queue or results.
    LikeTrack(Track),
    /// The first press on Reset. The reset itself waits for the confirmation.
    AskResetSettings,
    /// Resets the preferences. This is also the second press on the Reset confirmation.
    ResetSettings,
    /// The second press on an import confirmation: applies the backup that was read.
    ConfirmImport,
    /// The second press on the confirmation for turning the history off.
    ConfirmTurnOffHistory,
    CancelConfirmation,
    /// A confirmation's timer ended. The value numbers the confirmation that started the timer.
    ConfirmationExpired(u64),
}

/// A row whose ••• menu is open: its list and its position in that list. A queue can hold the same
/// track twice, so the track's id alone does not tell the rows apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RowMenu {
    pub(super) list: &'static str,
    pub(super) index: usize,
}

/// A destructive change that waits for a second press. It clears itself after a few seconds, when
/// the user presses something else, or when Settings is left.
#[derive(Clone, Debug)]
pub(super) struct PendingConfirmation {
    pub(super) kind: Confirmation,
    /// Numbers the confirmation, so the timer of an older one cannot clear a newer one.
    pub(super) count: u64,
}

#[derive(Clone, Debug)]
pub(super) enum Confirmation {
    Reset,
    /// The backup, already read and parsed. Nothing is applied until the second press.
    Import(Box<crate::preferences::Snapshot>),
    /// Clears the search history and "Jump back in".
    ClearHistory,
    /// Turns the history off, which also clears "Jump back in".
    TurnOffHistory,
}
