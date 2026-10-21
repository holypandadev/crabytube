//! Messages, and the enums they carry: every event the interface reacts to.

use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Home,
    Search,
    /// The user's lists: Favorite songs and the playlists.
    Library,
    /// One of those lists, with its songs.
    List(ListId),
    Settings,
    NowPlaying,
    CollectionDetail(BrowseTarget),
}

/// One of the user's lists: Favorite songs, which is always there, or a playlist they made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum ListId {
    #[default]
    Favorites,
    /// A playlist, by id.
    Playlist(u64),
}

/// A list whose rows can be dragged to another place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DragList {
    /// The songs of a list.
    Songs(ListId),
    /// The playlists on the Library page. Favorite songs stays first.
    Lists,
    /// Up next, the whole queue.
    Queue,
}

/// A row being dragged: the place it was taken from and the place it would land.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RowDrag {
    pub(super) list: DragList,
    pub(super) from: usize,
    pub(super) to: usize,
    /// The pointer moved far enough to start the drag. From then on, letting the row go never plays
    /// it, even back where it started.
    pub(super) moved: bool,
    /// Where the pointer is over the list's area, and where it was when it first moved there.
    pub(super) pointer: Option<iced::Point>,
    pub(super) start: Option<iced::Point>,
}

/// The field for a new playlist's name, or for a playlist's name and cover.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlaylistEditor {
    pub(super) target: PlaylistEditTarget,
    pub(super) name: String,
    pub(super) cover: CoverChoice,
    /// The Open dialog for a cover from the computer is open.
    pub(super) picking: bool,
    /// The last picture chosen could not be made a cover.
    pub(super) cover_failed: bool,
}

/// The cover a playlist being edited takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum CoverChoice {
    /// The covers of its first songs.
    #[default]
    Automatic,
    /// One of its songs' covers, by its address.
    Song(String),
    /// A picture from the user's computer, by its name in `playlist_covers`.
    File(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PlaylistEditTarget {
    /// A new playlist, which starts with the song of the Save panel that opened the field, if any.
    New(Option<Box<Track>>),
    /// A new name and cover for this playlist.
    Edit(u64),
}

/// Why the last change to a list did nothing, shown where it was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PlaylistNotice {
    /// No more playlists can be made.
    TooManyPlaylists,
    /// The playlist has no room for another song.
    PlaylistFull,
    /// The playlists together have no room for another song.
    AllFull,
    /// Favorite songs has no room for another song.
    FavoritesFull,
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
    LibrarySort(crate::preferences::LibrarySort),
    ShowLyricsDefault(bool),
    SearchSuggestions(bool),
    TrackNotifications(bool),
    ScaleWithWindow(bool),
    CheckUpdates(bool),
}

#[derive(Clone, Debug)]
pub(super) enum Message {
    StartDrag,
    /// Resize from a window edge (the window has no system decorations). On macOS the system
    /// frame does it.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    StartResize(iced::window::Direction),
    MinimizeWindow,
    ToggleMaximize,
    CloseWindow,
    CloseRequested,
    /// Asks the latest release whether it is newer: by hand from Settings, or by the daily check.
    CheckForUpdates {
        manual: bool,
    },
    UpdateChecked {
        manual: bool,
        result: Result<Option<crate::updater::Available>, crate::updater::UpdateError>,
    },
    /// Downloads and installs the update found, or opens its page where the app cannot install it.
    InstallUpdate,
    UpdateProgress {
        received: u64,
        total: u64,
    },
    UpdateDownloaded(Result<std::path::PathBuf, crate::updater::UpdateError>),
    /// The window moved: its new top left corner, in the interface's units.
    WindowMoved(iced::Point),
    /// The window has not moved again since the move of this count.
    WindowMoveSettled(u64),
    /// Opens Settings at its Updates section, from the link on Home.
    OpenUpdates,
    /// How far the Flatpak portal has got with the update.
    FlatpakUpdateProgress(crate::updater::FlatpakProgress),
    /// Starts the version the Flatpak portal installed, and quits.
    RestartForUpdate,
    RestartStarted(Result<(), crate::updater::UpdateError>),
    OpenUpdatePage,
    /// The main window exists: at start, or opened again from the tray.
    WindowOpened(iced::window::Id),
    QuitApp,
    RemoveFromQueue(usize),
    /// Moves a queue entry one place up or down.
    MoveInQueue {
        index: usize,
        up: bool,
    },
    ClearUpcomingQueue,
    Resized {
        width: f32,
        height: f32,
    },
    /// The window size after a resize, with whether the window is maximized, to keep for next time.
    WindowGeometry {
        size: (f32, f32),
        maximized: bool,
    },
    /// The size of the monitor the window opened on, asked for after a first launch.
    MonitorMeasured(Option<iced::Size>),
    /// The window and the monitor it is on, measured once after the window opened at a saved size,
    /// in case that size is larger than the monitor.
    SavedWindowMeasured {
        size: iced::Size,
        monitor: Option<iced::Size>,
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
    /// The catalog's English names of the track whose lyrics were not found under its own.
    LyricsEnglishNames {
        generation: u64,
        names: Option<(String, String)>,
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
    /// Steps the current song's lyrics offset: the Earlier and Later buttons over the lyrics.
    AdjustLyricsOffset(i64),
    RetryCollection,
    /// "Save as playlist" on a playlist or album of YouTube Music.
    SaveCollection,
    /// "Save as playlist" over Up next: the whole queue.
    SaveQueue,
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
    LibraryListsScrolled(AbsoluteOffset),
    /// The text that filters the playlists on the Library page.
    LibraryFilterChanged(String),
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
    /// A while after a seek bar change made without the mouse button held, as with the arrow keys
    /// or the wheel, which send no release: that change is applied unless a newer one came.
    SeekSettled(u64),
    /// The main mouse button went down or up, anywhere in the window.
    PointerHeld(bool),
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
    SeekNoticeExpired(u64),
    /// Fires once a resize settles; see the `Resized` handler.
    ResizeWorkDue(u64),
    /// Sent after the sizes of a burst of resize events were read; the zoom they ended with is the
    /// one Iced reads the next ones with.
    ZoomSettled,
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
    /// Another launch asked this instance to show its window.
    ShowWindowRequested,
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
    /// Escape pressed in the search field, which takes the key to give up its focus. `repeat` is
    /// a key held down.
    SearchFieldEscape {
        repeat: bool,
    },
    SearchSubmitted,
    SearchFor(String),
    /// Opens the page of the artist of this name, looked up in the catalog; a name it does not
    /// find is searched for.
    FindArtist(String),
    ArtistFound {
        request: u64,
        name: String,
        page: Option<BrowseTarget>,
    },
    SuggestionChosen(String),
    DismissSuggestions,
    KeyPressed(iced::keyboard::Event),
    /// An arrow or Enter for the row cursor, with whether the search field had the focus.
    RowKey {
        key: super::keyboard::RowKey,
        typing: bool,
    },
    /// The mouse wheel turned over a volume control.
    VolumeScrolled(iced::mouse::ScrollDelta),
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
    WindowMinimizedChecked(Option<bool>),
    /// A minute passed while Home is visible, so the greeting may have changed.
    ClockTick,
    /// A second passed while Settings is visible, so its note follows whether saving fails.
    SaveStatusCheck,
    /// Memory and CPU for the Home status line, sent only when the figures shown change.
    StatusReading(crate::process_stats::ProcessReading),
    /// A track notification was sent; zero when it was not.
    TrackNotificationSent(u32),
    /// Scrolls the Home carousel by one screen.
    CarouselStep {
        forward: bool,
    },
    CarouselScrolled(f32),
    /// Home scrolled down to the rows under its first screen, or back.
    HomeScrolled(f32),
    /// The arrows of a row of cards under Home's first screen.
    StripStep {
        strip: super::home::Strip,
        forward: bool,
    },
    StripScrolled(super::home::Strip, f32),
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
    /// "Start radio": loads the catalog radio of the track, then plays it with the track first.
    StartRadio(Track),
    /// The radio asked for by "Start radio", or the reason it could not be loaded.
    StartRadioLoaded {
        track: Track,
        /// The queue generation when the radio was asked for; a queue changed since then wins.
        generation: u64,
        /// The radio generation when it was asked for: a track picked or a radio asked for since
        /// then wins too.
        request: u64,
        result: Result<Vec<Track>, String>,
    },
    /// Opens the Save panel for a song: the lists it is in, and the others it can go to.
    OpenSaveDialog(Track),
    CloseSaveDialog,
    /// Puts the song in the list, or takes it out when it is there.
    ToggleSaved {
        list: ListId,
        track: Track,
    },
    /// Opens the field for a new playlist's name. With a song, the playlist starts with it.
    NewPlaylist(Option<Track>),
    /// Opens the fields for a playlist's name and cover.
    EditPlaylist(u64),
    PlaylistNameChanged(String),
    /// A cover for the playlist being edited.
    PlaylistCoverChosen(CoverChoice),
    /// Opens the system's Open dialog for a cover from the computer.
    ChooseCoverFile,
    /// What the Open dialog gave: a picture, nothing when cancelled, or why it failed.
    CoverFilePicked(Result<Option<std::path::PathBuf>, String>),
    /// The picture made into a cover, by its name, or why it could not be.
    CoverFileImported(Result<String, String>),
    /// Makes the playlist, or saves its name and cover.
    SubmitPlaylistEditor,
    CancelPlaylistEditor,
    RemoveFromList {
        list: ListId,
        index: usize,
    },
    /// Moves a list's song one place up or down.
    MoveInList {
        list: ListId,
        index: usize,
        up: bool,
    },
    /// Moves a playlist one place up or down on the Library page.
    MovePlaylist {
        index: usize,
        up: bool,
    },
    PlayPlaylist {
        id: u64,
        index: usize,
    },
    ShufflePlaylist(u64),
    /// Delete on a playlist: a panel asks first. The playlist goes on its Delete (`DeletePlaylist`).
    AskDeletePlaylist(u64),
    DeletePlaylist(u64),
    CancelDeletePlaylist,
    /// Closes the panel over the window: Save to, a ••• menu or the question to delete.
    CloseOverlay,
    /// The main button went down on a row that can be dragged.
    DragPressed {
        list: DragList,
        index: usize,
    },
    /// A finger was held still on a row that can be dragged: the row is lifted at once.
    DragHeld {
        list: DragList,
        index: usize,
    },
    /// During a press on a row, the pointer moved to this place over the list's area, which is this
    /// tall.
    DragMoved(iced::Point, f32),
    /// The list scrolls on while a dragged row is held at its edge.
    DragTick(u64),
    /// Redraws the covers that are still loading, which pulse.
    LoadingPulse,
    /// The press of a drag ended over a row.
    DragReleased,
    /// The note of this count has been shown long enough.
    ToastExpired(u64),
    /// The button of the note of this count: Undo, or Open.
    ToastPressed(u64),
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
