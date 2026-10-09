//! Development tools behind the `diagnostics` feature: the screenshot tour, the churn loop,
//! the scroll test, the `--soak` schedule and the viewport checks.

use super::*;

impl App {
    #[cfg(feature = "diagnostics")]
    /// The diagnostics messages of `update_inner`, which routes them here.
    pub(super) fn update_diagnostics(&mut self, message: Message) -> Task<Message> {
        match message {
            #[cfg(feature = "diagnostics")]
            Message::DiagnosticScrollStep => {
                // A triangle-free sawtooth over ~3000 px keeps both draw and scroll busy.
                self.diagnostic_scroll_step = self.diagnostic_scroll_step.wrapping_add(1);
                let y = (self.diagnostic_scroll_step % 125) as f32 * 24.0;
                let content = match self.page {
                    Page::Library => "library-content",
                    Page::Search => "search-content",
                    _ => "home-content",
                };
                return operation::scroll_to(content, AbsoluteOffset { x: 0.0, y });
            }
            #[cfg(feature = "diagnostics")]
            Message::DiagnosticShot(step) => {
                let Some((_, delay_ms)) = DIAGNOSTIC_TOUR.get(step) else {
                    return iced::exit();
                };
                diagnostic_state_changed();
                let action = match step {
                    1 => self.update(Message::Navigate(Page::Settings)),
                    2 => {
                        // Typing on Home: suggestions must float without moving the field.
                        let navigation = self.update(Message::Navigate(Page::Home));
                        self.search_query = "daft pu".into();
                        let typing = self.update(Message::SearchChanged("daft pu".into()));
                        Task::batch([navigation, typing])
                    }
                    3 => self.update(Message::SearchFor("daft punk".into())),
                    4 => {
                        let navigation = self.update(Message::Navigate(Page::Home));
                        let step = self.update(Message::CarouselStep { forward: true });
                        Task::batch([navigation, step])
                    }
                    5 => {
                        let tracks: Vec<Track> =
                            self.search_results.iter().take(4).cloned().collect();
                        for track in tracks {
                            let _ = self.update(Message::LikeTrack(track));
                        }
                        self.update(Message::Navigate(Page::Library))
                    }
                    6 => self.update(Message::OpenNowPlaying),
                    7 => {
                        let open = self.update(Message::SetNowPlayingTab(NowPlayingTab::Lyrics));
                        // The capture shows a part of the song with an active line and word timing.
                        // CRABYTUBE_TOUR_LYRICS_AT=<seconds> picks another part, for example a line with background vocals.
                        let at = std::env::var("CRABYTUBE_TOUR_LYRICS_AT")
                            .ok()
                            .and_then(|value| value.parse::<f64>().ok())
                            .unwrap_or(45.0);
                        let seek = self.update(Message::SeekTo(at));
                        Task::batch([open, seek])
                    }
                    8 => Task::batch([
                        self.update(Message::Navigate(Page::Settings)),
                        operation::scroll_to(
                            "settings-content",
                            AbsoluteOffset { x: 0.0, y: 700.0 },
                        ),
                    ]),
                    _ => Task::none(),
                };
                let delay = Duration::from_millis(*delay_ms);
                return Task::batch([
                    action,
                    Task::perform(async move { tokio::time::sleep(delay).await }, move |()| {
                        Message::DiagnosticCapture(step)
                    }),
                ]);
            }
            #[cfg(feature = "diagnostics")]
            Message::DiagnosticChurn(step) => {
                const STEPS: usize = 32;
                let rss = std::fs::read_to_string("/proc/self/status")
                    .ok()
                    .and_then(|status| {
                        status
                            .lines()
                            .find(|line| line.starts_with("VmRSS:"))
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                tracing::info!(step, %rss, page = ?self.page, "diagnostic churn");
                if step >= STEPS {
                    return iced::exit();
                }
                let items = &self.home_picks.items;
                let action = if step % 2 == 0 && !items.is_empty() {
                    match items[(step / 2) % items.len()].browse_target.clone() {
                        Some(target) => self.update(Message::OpenCollection(target)),
                        None => Task::none(),
                    }
                } else {
                    self.update(Message::Navigate(Page::Home))
                };
                return Task::batch([
                    action,
                    Task::perform(
                        async { tokio::time::sleep(Duration::from_millis(2_500)).await },
                        move |()| Message::DiagnosticChurn(step + 1),
                    ),
                ]);
            }
            #[cfg(feature = "diagnostics")]
            Message::DiagnosticSoak(step) => {
                // Home (from boot), Now Playing with lyrics, window minimized, then a pause of half a phase.
                let phase = diagnostic_soak_phase();
                let (name, action, delay) = match step {
                    0 => ("home", Task::none(), phase),
                    1 => (
                        "now-playing-lyrics",
                        Task::batch([
                            self.update(Message::OpenNowPlaying),
                            self.update(Message::SetNowPlayingTab(NowPlayingTab::Lyrics)),
                        ]),
                        phase,
                    ),
                    2 => (
                        "minimized",
                        iced::window::latest().and_then(|id| iced::window::minimize(id, true)),
                        phase,
                    ),
                    3 => ("paused", self.update(Message::TogglePlayback), phase / 2),
                    _ => {
                        tracing::info!("soak phase: end");
                        return iced::exit();
                    }
                };
                tracing::info!("soak phase: {name}");
                return Task::batch([
                    action,
                    Task::perform(async move { tokio::time::sleep(delay).await }, move |()| {
                        Message::DiagnosticSoak(step + 1)
                    }),
                ]);
            }
            #[cfg(feature = "diagnostics")]
            Message::DiagnosticCapture(step) => {
                // A hidden or unpresented window keeps the last frame, so the capture would be stale.
                if !diagnostic_presented_since_change() {
                    if diagnostic_since_change() < PRESENTATION_TIMEOUT {
                        return Task::perform(
                            async { tokio::time::sleep(Duration::from_millis(250)).await },
                            move |()| Message::DiagnosticCapture(step),
                        );
                    }
                    tracing::error!(
                        step,
                        "window is not being presented (hidden or on an inactive output); captures would be stale"
                    );
                    diagnostic_fail(DIAGNOSTIC_NOT_PRESENTED);
                    return iced::exit();
                }
                // Fixed delays would race the network: wait for what the step shows, with a limit.
                if let Some(missing) = diagnostic_missing(self, step) {
                    if diagnostic_waited(step) < diagnostic_wait_limit(step) {
                        return Task::perform(
                            async { tokio::time::sleep(Duration::from_millis(250)).await },
                            move |()| Message::DiagnosticCapture(step),
                        );
                    }
                    tracing::warn!(
                        step,
                        missing,
                        "diagnostic capture taken without its condition"
                    );
                }
                diagnostic_wait_done();
                return iced::window::latest()
                    .and_then(iced::window::screenshot)
                    .map(move |shot| Message::DiagnosticCaptured(step, shot));
            }
            #[cfg(feature = "diagnostics")]
            Message::DiagnosticCaptured(step, shot) => {
                diagnostic_check_fit(
                    self,
                    iced::Size::new(shot.size.width as f32, shot.size.height as f32),
                );
                if let (Some(dir), Some((name, _))) =
                    (DIAGNOSTIC_SHOTS.get(), DIAGNOSTIC_TOUR.get(step))
                {
                    let path = dir.join(format!("{step}-{name}.png"));
                    let saved = ::image::save_buffer(
                        &path,
                        &shot.rgba,
                        shot.size.width,
                        shot.size.height,
                        ::image::ExtendedColorType::Rgba8,
                    );
                    tracing::info!(path = %path.display(), ok = saved.is_ok(), "diagnostic screenshot");
                }
                return self.update(Message::DiagnosticShot(step + 1));
            }
            #[cfg(feature = "diagnostics")]
            Message::DiagnosticFrame(at) => diagnostic_frame_presented(at),
            _ => unreachable!("update_inner routes only this group here"),
        }
        Task::none()
    }
}

/// Playback requested from the command line for measurements. Only diagnostics builds accept it.
#[cfg(feature = "diagnostics")]
#[derive(Clone, Debug)]
pub struct DiagnosticAutoplay {
    pub video_id: String,
    pub exit_after: Option<Duration>,
}

#[cfg(feature = "diagnostics")]
pub(super) static DIAGNOSTIC_AUTOPLAY: std::sync::OnceLock<DiagnosticAutoplay> =
    std::sync::OnceLock::new();

/// `--scroll-test`: sweeps the Home list at a steady rate so frame intervals can be measured.
#[cfg(feature = "diagnostics")]
pub(super) static DIAGNOSTIC_SCROLL: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "diagnostics")]
pub fn enable_diagnostic_scroll() {
    DIAGNOSTIC_SCROLL.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_subscription() -> Subscription<Message> {
    let scroll = if DIAGNOSTIC_SCROLL.load(std::sync::atomic::Ordering::SeqCst) {
        iced::time::every(Duration::from_millis(16)).map(|_| Message::DiagnosticScrollStep)
    } else {
        Subscription::none()
    };
    // Listening only while a frame is awaited keeps an idle app from redrawing in a loop.
    let frames = if diagnostic_frames_awaited() {
        iced::window::frames().map(Message::DiagnosticFrame)
    } else {
        Subscription::none()
    };
    Subscription::batch([scroll, frames])
}

#[cfg(not(feature = "diagnostics"))]
pub(super) fn diagnostic_subscription() -> Subscription<Message> {
    Subscription::none()
}

#[cfg(feature = "diagnostics")]
pub(super) static DIAGNOSTIC_AUTOPLAY_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "diagnostics")]
pub fn set_diagnostic_autoplay(request: DiagnosticAutoplay) {
    let _ = DIAGNOSTIC_AUTOPLAY.set(request);
}

/// `--screenshots <dir>`: page name and the wait before each capture.
#[cfg(feature = "diagnostics")]
pub(super) const DIAGNOSTIC_TOUR: [(&str, u64); 9] = [
    ("home", 0),
    ("settings", 1_000),
    ("typing", 3_000),
    ("search", 6_000),
    ("home-carousel-step", 1_500),
    ("favorites", 2_500),
    ("now-playing", 2_500),
    ("lyrics", 1_000),
    ("settings-scrolled", 1_200),
];

/// `CRABYTUBE_VIEWPORT=WxH`: lay the app out as if the window had that size.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_viewport() -> Option<(f32, f32)> {
    let value = std::env::var("CRABYTUBE_VIEWPORT").ok()?;
    let (width, height) = value.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

#[cfg(feature = "diagnostics")]
pub(super) static DIAGNOSTIC_SHOTS: std::sync::OnceLock<std::path::PathBuf> =
    std::sync::OnceLock::new();

#[cfg(feature = "diagnostics")]
pub fn enable_diagnostic_screenshots(dir: std::path::PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = DIAGNOSTIC_SHOTS.set(dir);
}

#[cfg(feature = "diagnostics")]
pub(super) static DIAGNOSTIC_SCROLL_PAGE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// `--scroll-page library|search` opens that page once Home loads, so the scroll test
/// measures a long list instead of Home.
#[cfg(feature = "diagnostics")]
pub fn enable_diagnostic_scroll_page(page: String) {
    let _ = DIAGNOSTIC_SCROLL_PAGE.set(page);
}

#[cfg(feature = "diagnostics")]
pub(super) static DIAGNOSTIC_SOAK: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `--soak`: one long playback session in phases (see `Message::DiagnosticSoak`), for memory and CPU
/// measurements with the window visible, with lyrics, minimized and paused.
#[cfg(feature = "diagnostics")]
pub fn enable_diagnostic_soak() {
    DIAGNOSTIC_SOAK.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Seconds per `--soak` phase. `CRABYTUBE_SOAK_PHASE_SECS` shortens it for a dry run; default 600.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_soak_phase() -> Duration {
    let seconds = std::env::var("CRABYTUBE_SOAK_PHASE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600);
    Duration::from_secs(seconds)
}

/// Starts the `--soak` schedule at boot. Nothing runs unless the flag was given.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_soak_task() -> Task<Message> {
    if DIAGNOSTIC_SOAK.load(std::sync::atomic::Ordering::SeqCst) {
        Task::done(Message::DiagnosticSoak(0))
    } else {
        Task::none()
    }
}

#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_page_task() -> Task<Message> {
    match DIAGNOSTIC_SCROLL_PAGE.get().map(String::as_str) {
        Some("library") => Task::done(Message::Navigate(Page::Library)),
        Some("search") => Task::done(Message::SearchFor("daft punk".into())),
        _ => Task::none(),
    }
}

#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_tour_task() -> Task<Message> {
    if std::env::var_os("CRABYTUBE_CHURN").is_some() {
        return Task::perform(
            async { tokio::time::sleep(Duration::from_secs(8)).await },
            |()| Message::DiagnosticChurn(0),
        );
    }
    if DIAGNOSTIC_SHOTS.get().is_none() {
        return Task::none();
    }
    diagnostic_state_changed();
    // The compositor may ignore the size requested at creation; ask again once mapped.
    let resize = match std::env::var("CRABYTUBE_WINDOW").ok().and_then(|value| {
        let (width, height) = value.split_once('x')?;
        Some(iced::Size::new(width.parse().ok()?, height.parse().ok()?))
    }) {
        Some(size) => iced::window::latest().and_then(move |id| iced::window::resize(id, size)),
        None => Task::none(),
    };
    Task::batch([
        resize,
        // Each step waits for its own content (see `diagnostic_missing`), so this is only a settle.
        Task::perform(
            async { tokio::time::sleep(Duration::from_secs(1)).await },
            |()| Message::DiagnosticShot(0),
        ),
    ])
}

/// The step and when it started waiting, so each step gets its own time limit.
#[cfg(feature = "diagnostics")]
pub(super) static DIAGNOSTIC_WAIT_SINCE: std::sync::Mutex<Option<(usize, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// How long the tour has waited for `step`. The first call for a step starts the clock.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_waited(step: usize) -> Duration {
    let mut since = DIAGNOSTIC_WAIT_SINCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match *since {
        Some((waiting, started)) if waiting == step => started.elapsed(),
        _ => {
            *since = Some((step, std::time::Instant::now()));
            Duration::ZERO
        }
    }
}

#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_wait_done() {
    *DIAGNOSTIC_WAIT_SINCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// What a tour step still needs on screen before it is captured, if anything.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_missing(app: &App, step: usize) -> Option<&'static str> {
    let autoplay = DIAGNOSTIC_AUTOPLAY
        .get()
        .is_some_and(|request| !request.video_id.is_empty());
    if matches!(step, 0 | 2 | 4) && app.home_picks.items.is_empty() {
        return Some("Home items");
    }
    if step == 0 && autoplay && app.playback_status.state != PlaybackState::Playing {
        return Some("playback started");
    }
    let artwork_pending = app.artwork.is_none()
        && app
            .playback_status
            .track
            .as_ref()
            .is_some_and(|track| track.thumbnail_url.is_some());
    if matches!(step, 0 | 6) && artwork_pending {
        return Some("current artwork");
    }
    if step == 7 {
        if autoplay
            && app
                .playback_status
                .track
                .as_ref()
                .is_none_or(metadata_pending)
        {
            return Some("track metadata");
        }
        if app.lyrics_loading || (app.lyrics.is_none() && app.lyrics_error.is_none()) {
            return Some("LyricsLoaded");
        }
    }
    None
}

/// How long the tour waits for `step` before it captures anyway and logs what was missing.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_wait_limit(step: usize) -> Duration {
    if step == 7 {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(30)
    }
}

/// How long the window may go without presenting a frame after a state change, and how long a
/// page may disagree with the window, before the tour reports it.
#[cfg(feature = "diagnostics")]
pub(super) const PRESENTATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Exit status when the window stops presenting frames during the tour.
#[cfg(feature = "diagnostics")]
pub const DIAGNOSTIC_NOT_PRESENTED: i32 = 3;

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy)]
pub(super) struct DiagnosticPresentation {
    /// When the tour last changed the state: a navigation, a seek, or its start.
    changed_at: Option<std::time::Instant>,
    /// When the window last presented a frame.
    last_frame: Option<std::time::Instant>,
    /// Whether the frame subscription listens for the frame after the latest change.
    awaiting_frame: bool,
    /// When the tour started; a page that disagrees with the window is reported after the timeout.
    started: Option<std::time::Instant>,
    /// Exit status when the tour failed.
    failure: Option<i32>,
}

#[cfg(feature = "diagnostics")]
pub(super) static DIAGNOSTIC_PRESENTATION: std::sync::Mutex<DiagnosticPresentation> =
    std::sync::Mutex::new(DiagnosticPresentation {
        changed_at: None,
        last_frame: None,
        awaiting_frame: false,
        started: None,
        failure: None,
    });

#[cfg(feature = "diagnostics")]
pub(super) fn with_presentation<T>(action: impl FnOnce(&mut DiagnosticPresentation) -> T) -> T {
    let mut state = DIAGNOSTIC_PRESENTATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    action(&mut state)
}

/// A tour step changed the state: the next frame the window presents must come after this.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_state_changed() {
    with_presentation(|state| {
        let now = std::time::Instant::now();
        state.changed_at = Some(now);
        state.started.get_or_insert(now);
        state.awaiting_frame = true;
    });
}

#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_frame_presented(at: std::time::Instant) {
    // CRABYTUBE_TOUR_IGNORE_FRAMES=1 drops the frames, to check that the tour reports a window
    // that does not present.
    if std::env::var_os("CRABYTUBE_TOUR_IGNORE_FRAMES").is_some() {
        return;
    }
    with_presentation(|state| {
        state.last_frame = Some(at);
        if state.changed_at.is_some_and(|changed| at > changed) {
            state.awaiting_frame = false;
        }
    });
}

#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_frames_awaited() -> bool {
    with_presentation(|state| state.awaiting_frame)
}

/// Whether the window presented a frame after the latest state change.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_presented_since_change() -> bool {
    with_presentation(|state| presented_after(state.changed_at, state.last_frame))
}

/// A frame counts only when the window presented it after the change it should show.
#[cfg(feature = "diagnostics")]
pub(super) fn presented_after(
    changed: Option<std::time::Instant>,
    frame: Option<std::time::Instant>,
) -> bool {
    matches!((changed, frame), (Some(changed), Some(frame)) if frame > changed)
}

#[cfg(all(test, feature = "diagnostics"))]
mod presentation_tests {
    use super::{presented_after, window_and_page_agree};
    use std::time::{Duration, Instant};

    #[test]
    fn the_buffer_and_the_page_must_agree_with_the_window() {
        // The page fills the window once the zoom is applied.
        assert!(window_and_page_agree(
            (800.0, 560.0),
            1.25,
            1.0,
            (1000.0, 700.0),
            (1000.0, 700.0)
        ));
        // A 1024 px wide layout inside a 1234 px buffer is a stale frame.
        assert!(!window_and_page_agree(
            (1000.0, 600.0),
            1.0,
            1.0,
            (1024.0, 768.0),
            (1234.0, 768.0)
        ));
        // A page larger than the window cannot be seen whole.
        assert!(!window_and_page_agree(
            (1100.0, 600.0),
            1.0,
            1.0,
            (1000.0, 700.0),
            (1000.0, 700.0)
        ));
    }

    #[test]
    fn only_a_frame_after_the_change_counts_as_presented() {
        let change = Instant::now();
        let later = change + Duration::from_millis(1);
        assert!(presented_after(Some(change), Some(later)));
        assert!(!presented_after(Some(change), Some(change)));
        assert!(!presented_after(Some(later), Some(change)));
        assert!(!presented_after(Some(change), None));
        assert!(!presented_after(None, Some(later)));
    }
}

#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_since_change() -> Duration {
    with_presentation(|state| {
        state
            .changed_at
            .map_or(Duration::ZERO, |changed| changed.elapsed())
    })
}

#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_fail(code: i32) {
    with_presentation(|state| state.failure = Some(code));
}

/// The exit status a failed tour asks for; `None` when the tour did not fail.
#[cfg(feature = "diagnostics")]
pub fn diagnostic_failure_code() -> Option<i32> {
    with_presentation(|state| state.failure)
}

/// Whether the captured buffer and the page agree with the window. The buffer must be the
/// window's size, and the page, measured in layout units, must fit inside it once zoom and the
/// window's scale are applied. All sizes are in logical units except `buffer`, which is physical.
#[cfg(feature = "diagnostics")]
pub(super) fn window_and_page_agree(
    page_layout: (f32, f32),
    zoom: f32,
    scale: f32,
    window_logical: (f32, f32),
    buffer: (f32, f32),
) -> bool {
    const TOLERANCE: f32 = 2.0;
    let window = (window_logical.0 * scale, window_logical.1 * scale);
    let buffer_matches =
        (buffer.0 - window.0).abs() <= TOLERANCE && (buffer.1 - window.1).abs() <= TOLERANCE;
    let page = (page_layout.0 * zoom * scale, page_layout.1 * zoom * scale);
    let page_fits = page.0 <= window.0 + TOLERANCE && page.1 <= window.1 + TOLERANCE;
    buffer_matches && page_fits
}

/// Logs when the captured buffer and the page disagree with the window after the timeout.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_check_fit(app: &App, buffer: iced::Size) {
    let agree = window_and_page_agree(
        (app.area.width, app.area.height),
        app.ui_zoom,
        app.window_scale_factor,
        app.os_window,
        (buffer.width, buffer.height),
    );
    if agree {
        return;
    }
    let waited = with_presentation(|state| state.started.map_or(Duration::ZERO, |s| s.elapsed()));
    if waited >= PRESENTATION_TIMEOUT {
        tracing::error!(
            page = ?app.area,
            window = ?app.os_window,
            buffer = ?buffer,
            zoom = app.ui_zoom,
            "page size does not match the window"
        );
    }
}

/// Closes the window after `--exit-after`, measured from startup.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_exit_task() -> Task<Message> {
    match DIAGNOSTIC_AUTOPLAY
        .get()
        .and_then(|request| request.exit_after)
    {
        Some(after) => Task::perform(async move { tokio::time::sleep(after).await }, |()| {
            Message::QuitApp
        }),
        None => Task::none(),
    }
}

/// Starts the requested track once, after Home has loaded.
#[cfg(feature = "diagnostics")]
pub(super) fn diagnostic_autoplay_task() -> Task<Message> {
    let Some(request) = DIAGNOSTIC_AUTOPLAY.get() else {
        return Task::none();
    };
    if request.video_id.is_empty()
        || DIAGNOSTIC_AUTOPLAY_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return Task::none();
    }
    // A comma-separated list plays as a queue, so token batching can be measured.
    let tracks: Vec<crate::music::Track> = request
        .video_id
        .split(',')
        .filter(|id| !id.is_empty())
        .map(|id| crate::music::Track {
            video_id: id.to_owned(),
            title: id.to_owned(),
            artist: String::new(),
            album: None,
            thumbnail_url: None,
            duration_secs: None,
        })
        .collect();
    let play = Message::PlayContext(Arc::from(tracks), 0);
    // CRABYTUBE_AUTOPLAY_DELAY_MS waits after Home so a measurement can click late.
    match std::env::var("CRABYTUBE_AUTOPLAY_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        Some(delay) if delay > 0 => Task::perform(
            async move { tokio::time::sleep(Duration::from_millis(delay)).await },
            move |()| play,
        ),
        _ => Task::done(play),
    }
}
