//! The Windows system media controls: the media keys, the headphone buttons and the media panel of
//! the system flyout take their commands from `SystemMediaTransportControls`, and show what its
//! display updater is given. The rules that decide what to publish and when are in `media_session`;
//! this file only talks to the Windows media APIs.
//!
//! The controls come from a `MediaPlayer` that never plays anything: it only owns the transport
//! controls, so no window handle is needed. Its command manager is switched off so the player does
//! not publish a state of its own. All of it lives on a thread of its own, with its own Windows
//! Runtime apartment, so the window system keeps the one it sets up on the main thread. The system
//! calls the handlers on its own threads and they only queue a command. The cover is read from the
//! file the app already saved, never from a URL the system would download, and handed over as bytes
//! held in memory.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, Playback::MediaPlayer,
        PlaybackPositionChangeRequestedEventArgs, SystemMediaTransportControls,
        SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    },
    Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference},
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize},
    core::HSTRING,
};

use crate::media_session::{
    Changes, MediaEvent, NowPlaying, PlaybackKind, Previous, changes, command_channel, command_for,
    drain, now_playing, playback,
};
pub use crate::media_session::{CommandReceiver, PlayerState, RemoteCommand, command_stream};

/// A cover file larger than this is ignored.
const MAX_COVER_BYTES: u64 = 8 * 1024 * 1024;
/// How often the cover file is looked for while the app is still downloading it.
const COVER_POLL: Duration = Duration::from_secs(1);
/// How long to wait for the cover of a track before giving up on it.
const COVER_PATIENCE: Duration = Duration::from_secs(60);
/// How long closing the app waits for the thread of the controls to finish.
const STOP_PATIENCE: Duration = Duration::from_secs(2);

enum Cover {
    /// The track has none, or the file never appeared.
    Missing,
    /// The cover of the track, kept as compressed bytes so it can be given to the system again
    /// whenever the track information is rewritten.
    Ready(RandomAccessStreamReference),
    /// The URL is known and its file is not on disk yet.
    Waiting { since: Instant, checked: Instant },
}

/// The transport controls and the handlers that listen to them.
struct Controls {
    player: MediaPlayer,
    transport: SystemMediaTransportControls,
    button_token: i64,
    position_token: i64,
}

/// What the thread of the controls works on.
struct Session {
    controls: Controls,
    /// The last report the system was given, and when.
    published: Option<(PlayerState, Instant)>,
    /// The cover URL of the track that is playing.
    cover_url: Option<String>,
    cover: Cover,
}

/// A report for the thread of the controls, with what it changes.
enum Job {
    Update {
        /// Boxed, so a stop does not carry the room of a whole state.
        state: Box<PlayerState>,
        at: Instant,
        changes: Changes,
    },
    /// Take the app out of the system, and say so when it is done.
    Stop(mpsc::SyncSender<()>),
}

/// The thread of the controls.
struct Worker {
    jobs: Sender<Job>,
    thread: JoinHandle<()>,
    /// Set while the thread waits for the cover file of the track, which then has to be looked
    /// for even when nothing else changes.
    cover_pending: Arc<AtomicBool>,
}

pub struct MprisRemote {
    commands: CommandReceiver,
    /// `None` when the system media controls could not be started, for example on an edition of
    /// Windows without the media features.
    worker: Option<Worker>,
    /// The last report handed to the thread, and when.
    reported: Mutex<Option<(PlayerState, Instant)>>,
    failure: Option<String>,
}

impl MprisRemote {
    pub fn start() -> Self {
        let (sender, commands) = command_channel();
        // The tests build an app many times, and creating and dropping the media player of
        // Windows for each one now and then crashed the test process on a machine without
        // audio. Only the test of the media session starts it.
        #[cfg(test)]
        if !tests::MEDIA_SESSION_UNDER_TEST.load(std::sync::atomic::Ordering::Relaxed) {
            return Self {
                commands,
                worker: None,
                reported: Mutex::new(None),
                failure: Some("The system media controls are not started in tests".into()),
            };
        }
        let (jobs, inbox) = mpsc::channel();
        let (started, outcome) = mpsc::sync_channel(1);
        let cover_pending = Arc::new(AtomicBool::new(false));
        let pending = cover_pending.clone();
        let spawned = std::thread::Builder::new()
            .name("crabytube-media".into())
            .spawn(move || run(&sender, &inbox, &started, &pending));
        let outcome = match spawned {
            Ok(thread) => outcome
                .recv()
                .map_err(|_| "the media thread ended".to_string())
                .and_then(|result| result)
                .map(|()| thread),
            Err(error) => Err(error.to_string()),
        };
        match outcome {
            Ok(thread) => Self {
                commands,
                worker: Some(Worker {
                    jobs,
                    thread,
                    cover_pending,
                }),
                reported: Mutex::new(None),
                failure: None,
            },
            Err(error) => {
                tracing::warn!(%error, "The system media controls could not be started");
                Self {
                    commands,
                    worker: None,
                    reported: Mutex::new(None),
                    failure: Some(error),
                }
            }
        }
    }

    /// Tells the system what changed since the last report. Most reports change nothing the
    /// system shows, and then this does no more than compare.
    pub fn update(&self, state: PlayerState) {
        let Some(worker) = &self.worker else {
            return;
        };
        let Ok(mut reported) = self.reported.lock() else {
            return;
        };
        let now = Instant::now();
        let previous = reported
            .as_ref()
            .map(|(state, at)| Previous { state, at: *at });
        let changes = changes(previous, &state, now);
        let new_cover = reported
            .as_ref()
            .is_none_or(|(before, _)| before.art_url != state.art_url);
        if !changes.any() && !new_cover && !worker.cover_pending.load(Ordering::Relaxed) {
            return;
        }
        let job = Job::Update {
            state: Box::new(state.clone()),
            at: now,
            changes,
        };
        if worker.jobs.send(job).is_ok() {
            *reported = Some((state, now));
        }
    }

    pub fn command_receiver(&self) -> CommandReceiver {
        self.commands.clone()
    }

    pub fn status(&self) -> String {
        match &self.failure {
            None => "Media keys and the media panel are active".into(),
            Some(error) => format!("System media controls are unavailable: {error}"),
        }
    }

    pub fn drain(&self) -> Vec<RemoteCommand> {
        drain(&self.commands)
    }
}

impl Drop for MprisRemote {
    fn drop(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        // The thread is given a moment to take the app out of the system; a call into Windows that
        // hangs must not keep the app from closing.
        let (done, finished) = mpsc::sync_channel(1);
        if worker.jobs.send(Job::Stop(done)).is_ok() && finished.recv_timeout(STOP_PATIENCE).is_ok()
        {
            let _ = worker.thread.join();
        }
    }
}
/// The thread of the controls: starts them, reports whether that worked and then follows the
/// reports until it is told to stop or the app drops its end.
fn run(
    commands: &tokio::sync::mpsc::UnboundedSender<RemoteCommand>,
    inbox: &Receiver<Job>,
    started: &mpsc::SyncSender<Result<(), String>>,
    cover_pending: &AtomicBool,
) {
    // SAFETY: plain initialization of the Windows Runtime on this thread, which has no apartment
    // yet.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let controls = match Controls::start(commands) {
        Ok(controls) => controls,
        Err(error) => {
            let _ = started.send(Err(error.to_string()));
            return;
        }
    };
    let _ = started.send(Ok(()));
    let mut session = Session {
        controls,
        published: None,
        cover_url: None,
        cover: Cover::Missing,
    };
    while let Ok(job) = inbox.recv() {
        let Job::Update { state, at, changes } = job else {
            session.controls.stop();
            if let Job::Stop(done) = job {
                let _ = done.send(());
            }
            return;
        };
        let cover_changed = session.follow_cover(&state, at);
        cover_pending.store(
            matches!(session.cover, Cover::Waiting { .. }),
            Ordering::Relaxed,
        );
        if !changes.any() && !cover_changed {
            continue;
        }
        if let Err(error) = session.publish(*state, at, changes, cover_changed) {
            tracing::warn!(%error, "The system media controls were not updated");
        }
    }
    // The app dropped its end without asking.
    session.controls.stop();
}

impl Controls {
    /// Creates the controls and listens to the buttons the app can carry out. Each handler only
    /// queues a command.
    fn start(
        sender: &tokio::sync::mpsc::UnboundedSender<RemoteCommand>,
    ) -> windows::core::Result<Self> {
        let player = MediaPlayer::new()?;
        player.CommandManager()?.SetIsEnabled(false)?;
        let transport = player.SystemMediaTransportControls()?;
        transport.SetIsEnabled(false)?;
        let buttons = sender.clone();
        let button_token = transport.ButtonPressed(&TypedEventHandler::<
            SystemMediaTransportControls,
            SystemMediaTransportControlsButtonPressedEventArgs,
        >::new(move |_, arguments| {
            if let Some(arguments) = arguments.as_ref()
                && let Some(command) = command_for_button(arguments.Button()?)
            {
                let _ = buttons.send(command);
            }
            Ok(())
        }))?;
        let seeks = sender.clone();
        let position_token = transport.PlaybackPositionChangeRequested(&TypedEventHandler::<
            SystemMediaTransportControls,
            PlaybackPositionChangeRequestedEventArgs,
        >::new(
            move |_, arguments| {
                if let Some(arguments) = arguments.as_ref() {
                    let position = seconds(arguments.RequestedPlaybackPosition()?);
                    let _ = seeks.send(command_for(MediaEvent::SetPosition(position)));
                }
                Ok(())
            },
        ))?;
        Ok(Self {
            player,
            transport,
            button_token,
            position_token,
        })
    }

    /// Takes the app out of the system: no more handlers, no track and no session.
    fn stop(self) {
        let _ = self.transport.RemoveButtonPressed(self.button_token);
        let _ = self
            .transport
            .RemovePlaybackPositionChangeRequested(self.position_token);
        if let Ok(updater) = self.transport.DisplayUpdater() {
            let _ = updater.ClearAll();
            let _ = updater.Update();
        }
        let _ = self
            .transport
            .SetPlaybackStatus(MediaPlaybackStatus::Closed);
        let _ = self.transport.SetIsEnabled(false);
        let _ = self.player.Close();
    }
}

impl Session {
    /// Follows the cover of the track that plays. Returns whether what the system shows has to be
    /// written again: the cover of the previous track went away, or the new one was found.
    fn follow_cover(&mut self, state: &PlayerState, now: Instant) -> bool {
        let mut changed = false;
        if state.art_url != self.cover_url {
            self.cover_url.clone_from(&state.art_url);
            changed = matches!(self.cover, Cover::Ready(_));
            self.cover = if state.art_url.is_some() {
                Cover::Waiting {
                    since: now,
                    checked: now - COVER_POLL,
                }
            } else {
                Cover::Missing
            };
        }
        let (Cover::Waiting { since, checked }, Some(url)) = (&self.cover, &self.cover_url) else {
            return changed;
        };
        if now.saturating_duration_since(*checked) < COVER_POLL {
            return changed;
        }
        let since = *since;
        match crate::artwork::saved_cover_path(url).and_then(|path| read_cover(&path)) {
            Some(thumbnail) => {
                self.cover = Cover::Ready(thumbnail);
                true
            }
            None if now.saturating_duration_since(since) > COVER_PATIENCE => {
                self.cover = Cover::Missing;
                changed
            }
            None => {
                self.cover = Cover::Waiting {
                    since,
                    checked: now,
                };
                changed
            }
        }
    }

    fn publish(
        &mut self,
        state: PlayerState,
        now: Instant,
        changes: Changes,
        cover_changed: bool,
    ) -> windows::core::Result<()> {
        let track = now_playing(&state);
        let transport = &self.controls.transport;
        let updater = transport.DisplayUpdater()?;
        match &track {
            Some(track) => {
                if changes.track || cover_changed {
                    // The whole track is written again, cover included.
                    updater.ClearAll()?;
                    updater.SetType(MediaPlaybackType::Music)?;
                    let music = updater.MusicProperties()?;
                    music.SetTitle(&HSTRING::from(track.title.as_str()))?;
                    music.SetArtist(&HSTRING::from(track.artist.as_deref().unwrap_or_default()))?;
                    music.SetAlbumTitle(&HSTRING::from(
                        track.album.as_deref().unwrap_or_default(),
                    ))?;
                    if let Cover::Ready(thumbnail) = &self.cover {
                        updater.SetThumbnail(thumbnail)?;
                    }
                    updater.Update()?;
                }
                if changes.controls || self.published.is_none() {
                    set_controls(transport, &state)?;
                }
                if changes.playback || changes.track {
                    transport.SetPlaybackStatus(status_for(&state))?;
                    update_timeline(transport, track, &state)?;
                }
                transport.SetIsEnabled(true)?;
            }
            None => {
                updater.ClearAll()?;
                updater.Update()?;
                transport.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;
                transport.SetIsEnabled(false)?;
            }
        }
        self.published = Some((state, now));
        Ok(())
    }
}
/// Windows moves the progress bar of a playing session by itself, from the moment its state last
/// changed. A track that loads is Changing, so the bar starts when the music does.
fn status_for(state: &PlayerState) -> MediaPlaybackStatus {
    match playback(state).kind {
        PlaybackKind::Playing if state.loading => MediaPlaybackStatus::Changing,
        PlaybackKind::Playing => MediaPlaybackStatus::Playing,
        PlaybackKind::Paused => MediaPlaybackStatus::Paused,
        PlaybackKind::Stopped => MediaPlaybackStatus::Stopped,
    }
}

/// Which buttons the system offers follows the queue.
fn set_controls(
    transport: &SystemMediaTransportControls,
    state: &PlayerState,
) -> windows::core::Result<()> {
    transport.SetIsPlayEnabled(state.can_play)?;
    transport.SetIsPauseEnabled(state.can_pause)?;
    transport.SetIsNextEnabled(state.can_go_next)?;
    transport.SetIsPreviousEnabled(state.can_go_previous)?;
    transport.SetIsStopEnabled(false)
}

/// Where playback is and how long the track is, so that the progress bar goes on by itself while
/// the playback status is Playing.
fn update_timeline(
    transport: &SystemMediaTransportControls,
    track: &NowPlaying,
    state: &PlayerState,
) -> windows::core::Result<()> {
    let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
    let end = track.duration_secs.unwrap_or(0.0);
    timeline.SetStartTime(time_span(0.0))?;
    timeline.SetMinSeekTime(time_span(0.0))?;
    timeline.SetEndTime(time_span(end))?;
    timeline.SetMaxSeekTime(time_span(end))?;
    timeline.SetPosition(time_span(playback(state).position_secs.min(end.max(0.0))))?;
    transport.UpdateTimelineProperties(&timeline)
}

/// The command for a button of the system controls. The buttons the app has no use for give none.
fn command_for_button(button: SystemMediaTransportControlsButton) -> Option<RemoteCommand> {
    let event = match button {
        SystemMediaTransportControlsButton::Play => MediaEvent::Play,
        SystemMediaTransportControlsButton::Pause => MediaEvent::Pause,
        SystemMediaTransportControlsButton::Next => MediaEvent::Next,
        SystemMediaTransportControlsButton::Previous => MediaEvent::Previous,
        SystemMediaTransportControlsButton::Stop => return Some(RemoteCommand::Stop),
        _ => return None,
    };
    Some(command_for(event))
}

/// A time span is counted in intervals of 100 nanoseconds.
const TICKS_PER_SECOND: f64 = 10_000_000.0;

fn time_span(seconds: f64) -> TimeSpan {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    TimeSpan {
        Duration: (seconds * TICKS_PER_SECOND).round() as i64,
    }
}

fn seconds(span: TimeSpan) -> f64 {
    span.Duration as f64 / TICKS_PER_SECOND
}

/// The cover file the app saved, as bytes in memory the system can read. Nothing is decoded here:
/// the system decodes the image when it shows it.
fn read_cover(path: &std::path::Path) -> Option<RandomAccessStreamReference> {
    if std::fs::metadata(path).ok()?.len() > MAX_COVER_BYTES {
        return None;
    }
    thumbnail(&std::fs::read(path).ok()?).ok()
}

fn thumbnail(bytes: &[u8]) -> windows::core::Result<RandomAccessStreamReference> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.join()?;
    writer.DetachStream()?;
    stream.Seek(0)?;
    RandomAccessStreamReference::CreateFromStream(&stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set by the test that needs the real media session; see `MprisRemote::start`.
    pub(super) static MEDIA_SESSION_UNDER_TEST: AtomicBool = AtomicBool::new(false);

    #[test]
    fn the_buttons_of_the_system_become_the_commands_the_app_handles() {
        let command = command_for_button;
        assert_eq!(
            command(SystemMediaTransportControlsButton::Play),
            Some(RemoteCommand::Play)
        );
        assert_eq!(
            command(SystemMediaTransportControlsButton::Pause),
            Some(RemoteCommand::Pause)
        );
        assert_eq!(
            command(SystemMediaTransportControlsButton::Next),
            Some(RemoteCommand::Next)
        );
        assert_eq!(
            command(SystemMediaTransportControlsButton::Previous),
            Some(RemoteCommand::Previous)
        );
        assert_eq!(
            command(SystemMediaTransportControlsButton::Stop),
            Some(RemoteCommand::Stop)
        );
        assert_eq!(
            command(SystemMediaTransportControlsButton::FastForward),
            None
        );
        assert_eq!(command(SystemMediaTransportControlsButton::ChannelUp), None);
    }

    #[test]
    fn time_spans_count_hundreds_of_nanoseconds_and_are_never_negative() {
        assert_eq!(time_span(1.0).Duration, 10_000_000);
        assert_eq!(time_span(12.5).Duration, 125_000_000);
        assert_eq!(time_span(-3.0).Duration, 0);
        assert_eq!(time_span(f64::NAN).Duration, 0);
        assert_eq!(time_span(f64::INFINITY).Duration, 0);
        assert_eq!(
            seconds(TimeSpan {
                Duration: 125_000_000
            }),
            12.5
        );
    }

    #[test]
    fn a_seek_of_the_system_arrives_in_microseconds() {
        let seek = command_for(MediaEvent::SetPosition(seconds(time_span(12.5))));
        assert_eq!(seek, RemoteCommand::SeekTo(12_500_000));
    }

    #[test]
    fn saved_cover_bytes_become_a_stream_the_system_can_read() {
        // SAFETY: plain initialization of the Windows Runtime on the test thread.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let bytes = vec![7u8; 5_000];
        let reference = thumbnail(&bytes).expect("a stream reference");
        let stream = reference
            .OpenReadAsync()
            .expect("a read operation")
            .join()
            .expect("a stream");
        assert_eq!(stream.Size().unwrap(), bytes.len() as u64);
        // A file that is missing gives no cover.
        assert!(read_cover(std::path::Path::new(r"C:\nonexistent\cover.img")).is_none());
    }

    fn reported(status: &str, position_secs: u64) -> PlayerState {
        PlayerState {
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            playback_status: status.into(),
            position: Duration::from_secs(position_secs),
            duration: Some(Duration::from_secs(200)),
            can_play: true,
            can_pause: true,
            can_go_next: true,
            can_go_previous: false,
            ..PlayerState::default()
        }
    }

    /// The media session of this test process, as the system lists it, once one shows up.
    fn our_session(
        manager: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager,
        wanted: bool,
    ) -> Option<windows::Media::Control::GlobalSystemMediaTransportControlsSession> {
        let exe = std::env::current_exe().unwrap();
        let exe = exe.file_name().unwrap().to_string_lossy().to_lowercase();
        for _ in 0..50 {
            let found = manager.GetSessions().unwrap().into_iter().find(|session| {
                session
                    .SourceAppUserModelId()
                    .is_ok_and(|id| id.to_string().to_lowercase().contains(&exe))
            });
            if found.is_some() == wanted {
                return found;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    #[test]
    #[ignore = "needs a desktop session with the media features of Windows"]
    fn what_the_player_reports_reaches_the_system_and_leaves_with_the_app() {
        use windows::Media::Control::{
            GlobalSystemMediaTransportControlsSessionManager as Manager,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
        };
        // SAFETY: plain initialization of the Windows Runtime on the test thread.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        MEDIA_SESSION_UNDER_TEST.store(true, std::sync::atomic::Ordering::Relaxed);
        let remote = MprisRemote::start();
        assert!(remote.failure.is_none(), "{}", remote.status());
        let manager = Manager::RequestAsync().unwrap().join().unwrap();
        remote.update(reported("Playing", 10));
        let session = our_session(&manager, true).expect("a media session");
        let properties = session
            .TryGetMediaPropertiesAsync()
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(properties.Title().unwrap(), "Song");
        assert_eq!(properties.Artist().unwrap(), "Artist");
        assert_eq!(properties.AlbumTitle().unwrap(), "Album");
        let info = session.GetPlaybackInfo().unwrap();
        assert_eq!(info.PlaybackStatus().unwrap(), Status::Playing);
        let controls = info.Controls().unwrap();
        assert!(controls.IsPauseEnabled().unwrap() && controls.IsNextEnabled().unwrap());
        assert!(!controls.IsPreviousEnabled().unwrap());
        let timeline = session.GetTimelineProperties().unwrap();
        assert_eq!(seconds(timeline.EndTime().unwrap()), 200.0);
        // Paused, with the controls of the queue changed.
        let mut paused = reported("Paused", 10);
        paused.can_go_previous = true;
        remote.update(paused);
        std::thread::sleep(Duration::from_millis(500));
        let info = session.GetPlaybackInfo().unwrap();
        assert_eq!(info.PlaybackStatus().unwrap(), Status::Paused);
        assert!(info.Controls().unwrap().IsPreviousEnabled().unwrap());
        // Nothing loaded: the system shows no session.
        remote.update(PlayerState::default());
        assert!(our_session(&manager, false).is_none());
        remote.update(reported("Playing", 0));
        assert!(our_session(&manager, true).is_some());
        // And it leaves with the app.
        drop(remote);
        assert!(our_session(&manager, false).is_none());
    }
}
