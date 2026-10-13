//! The macOS system media controls: the media keys, headphone buttons and Control Center take
//! their commands from `MPRemoteCommandCenter`, and Now Playing shows what
//! `MPNowPlayingInfoCenter` is given. The rules that decide what to publish and when are in
//! `media_session`; this file only talks to the MediaPlayer framework.
//!
//! The system calls the command handlers on the main thread and they only queue a command. The
//! cover is read from the file the app already saved, never from a URL the system would download,
//! and it is kept as compressed bytes: an image is decoded each time the system asks for one and
//! released with it.

use std::{
    ptr::NonNull,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

use block2::RcBlock;
use objc2::{AnyThread, rc::Retained, runtime::AnyObject};
use objc2_app_kit::NSImage;
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle,
    MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
    MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
    MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState, MPRemoteCommand,
    MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};
use tokio::sync::mpsc::UnboundedSender;

pub use crate::media_session::{CommandReceiver, PlayerState, RemoteCommand, command_stream};
use crate::media_session::{
    MediaEvent, NowPlaying, Playback, PlaybackKind, Previous, SEEK_TOLERANCE, changes,
    command_channel, command_for, drain, now_playing, playback,
};

/// A cover file larger than this is ignored.
const MAX_COVER_BYTES: u64 = 8 * 1024 * 1024;
/// How often the cover file is looked for while the app is still downloading it.
const COVER_POLL: Duration = Duration::from_secs(1);
/// How long to wait for the cover of a track before giving up on it.
const COVER_PATIENCE: Duration = Duration::from_secs(60);

/// A command the app listens to, and the handle that removes the listener again.
struct Registration {
    command: Retained<MPRemoteCommand>,
    target: Retained<AnyObject>,
}

/// The commands whose availability follows the queue.
struct Controls {
    next: Retained<MPRemoteCommand>,
    previous: Retained<MPRemoteCommand>,
    seek: Retained<MPRemoteCommand>,
}

enum Cover {
    /// The track has none, or the file never appeared.
    Missing,
    /// The URL is known and its file is not on disk yet.
    Waiting {
        since: Instant,
        checked: Instant,
    },
    Ready(Retained<MPMediaItemArtwork>),
}

struct Session {
    /// The last report the system was given, and when: by the monotonic clock, which stops while
    /// the Mac sleeps, and by the wall clock, which does not.
    published: Option<(PlayerState, Instant, SystemTime)>,
    /// The cover URL of the track that is playing.
    cover_url: Option<String>,
    cover: Cover,
    registrations: Vec<Registration>,
    controls: Option<Controls>,
}

pub struct MprisRemote {
    commands: CommandReceiver,
    session: Mutex<Session>,
}

// SAFETY: the Objective-C objects in the session belong to the MediaPlayer framework, whose command
// center and info center may be used from any thread, and the mutex serializes every use.
unsafe impl Send for MprisRemote {}
unsafe impl Sync for MprisRemote {}

impl MprisRemote {
    pub fn start() -> Self {
        let (sender, commands) = command_channel();
        let (registrations, controls) = register_commands(&sender);
        Self {
            commands,
            session: Mutex::new(Session {
                published: None,
                cover_url: None,
                cover: Cover::Missing,
                registrations,
                controls: Some(controls),
            }),
        }
    }

    /// Tells the system what changed since the last report. Most reports change nothing the
    /// system shows, and then this does no more than compare.
    pub fn update(&self, state: PlayerState) {
        let Ok(mut session) = self.session.lock() else {
            return;
        };
        let now = Instant::now();
        let wall = SystemTime::now();
        let cover_changed = session.follow_cover(&state, now);
        let previous = session
            .published
            .as_ref()
            .map(|(state, at, _)| Previous { state, at: *at });
        let mut changes = changes(previous, &state, now);
        // The system extrapolates the position by the wall clock. A sleep that the monotonic
        // clock did not see leaves it ahead of the playback, so the report is sent again.
        if let Some((published, at, published_wall)) = &session.published
            && playback(published).kind == PlaybackKind::Playing
            && slept_since(*at, *published_wall, now, wall)
        {
            tracing::info!("the Mac slept since the last report; reporting the position again");
            changes.playback = true;
        }
        if !changes.any() && !cover_changed {
            return;
        }
        if changes.controls {
            session.set_controls(&state);
        }
        session.publish(state, now, wall);
    }

    pub fn command_receiver(&self) -> CommandReceiver {
        self.commands.clone()
    }

    pub fn status(&self) -> String {
        "Media keys and Now Playing are active".into()
    }

    pub fn drain(&self) -> Vec<RemoteCommand> {
        drain(&self.commands)
    }
}

impl Drop for MprisRemote {
    fn drop(&mut self) {
        let Ok(mut session) = self.session.lock() else {
            return;
        };
        for registration in session.registrations.drain(..) {
            // SAFETY: the target is the object `addTargetWithHandler` returned for this command.
            unsafe {
                registration
                    .command
                    .removeTarget(Some(&registration.target))
            };
        }
        // SAFETY: plain property writes on the shared info center.
        unsafe {
            let center = MPNowPlayingInfoCenter::defaultCenter();
            center.setNowPlayingInfo(None);
            center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
        }
    }
}

/// Listens to the commands the app can carry out. Each handler only queues a command.
fn register_commands(sender: &UnboundedSender<RemoteCommand>) -> (Vec<Registration>, Controls) {
    // SAFETY: the shared command center is always available, and every handler returns at once.
    unsafe {
        let center = MPRemoteCommandCenter::sharedCommandCenter();
        let mut registrations = Vec::new();
        let mut listen = |command: &Retained<MPRemoteCommand>,
                          event: fn(&MPRemoteCommandEvent) -> MediaEvent| {
            let sender = sender.clone();
            let handler = RcBlock::new(move |event_ptr: NonNull<MPRemoteCommandEvent>| {
                let _ = sender.send(command_for(event(event_ptr.as_ref())));
                MPRemoteCommandHandlerStatus::Success
            });
            command.setEnabled(true);
            let target = command.addTargetWithHandler(&handler);
            registrations.push(Registration {
                command: command.clone(),
                target,
            });
        };
        listen(&center.playCommand(), |_| MediaEvent::Play);
        listen(&center.pauseCommand(), |_| MediaEvent::Pause);
        listen(&center.togglePlayPauseCommand(), |_| MediaEvent::Toggle);
        let next = center.nextTrackCommand();
        listen(&next, |_| MediaEvent::Next);
        let previous = center.previousTrackCommand();
        listen(&previous, |_| MediaEvent::Previous);
        let seek: Retained<MPRemoteCommand> = center.changePlaybackPositionCommand().into_super();
        listen(&seek, |event| {
            // SAFETY: this command only sends this kind of event.
            let event = &*(event as *const MPRemoteCommandEvent)
                .cast::<MPChangePlaybackPositionCommandEvent>();
            MediaEvent::SetPosition(event.positionTime())
        });
        (
            registrations,
            Controls {
                next,
                previous,
                seek,
            },
        )
    }
}

impl Session {
    /// Follows the cover of the track that plays. Returns whether what the system should show
    /// changed: the cover of the previous track was dropped, or the new one was found.
    fn follow_cover(&mut self, state: &PlayerState, now: Instant) -> bool {
        if state.art_url != self.cover_url {
            self.cover_url.clone_from(&state.art_url);
            let had_cover = matches!(self.cover, Cover::Ready(_));
            self.cover = if state.art_url.is_some() {
                Cover::Waiting {
                    since: now,
                    checked: now - COVER_POLL,
                }
            } else {
                Cover::Missing
            };
            if !matches!(self.cover, Cover::Waiting { .. }) {
                return had_cover;
            }
        }
        let (Cover::Waiting { since, checked }, Some(url)) = (&self.cover, &self.cover_url) else {
            return false;
        };
        if now.saturating_duration_since(*checked) < COVER_POLL {
            return false;
        }
        let since = *since;
        match crate::artwork::saved_cover_path(url).and_then(|path| read_cover(&path)) {
            Some(artwork) => {
                self.cover = Cover::Ready(artwork);
                true
            }
            None if now.saturating_duration_since(since) > COVER_PATIENCE => {
                self.cover = Cover::Missing;
                false
            }
            None => {
                self.cover = Cover::Waiting {
                    since,
                    checked: now,
                };
                false
            }
        }
    }

    fn set_controls(&self, state: &PlayerState) {
        let Some(controls) = &self.controls else {
            return;
        };
        // SAFETY: plain property writes on commands that live as long as the session.
        unsafe {
            controls.next.setEnabled(state.can_go_next);
            controls.previous.setEnabled(state.can_go_previous);
            controls.seek.setEnabled(state.duration.is_some());
        }
    }

    fn publish(&mut self, state: PlayerState, now: Instant, wall: SystemTime) {
        let track = now_playing(&state);
        let playback = playback(&state);
        let cover = match &self.cover {
            Cover::Ready(artwork) => Some(artwork),
            _ => None,
        };
        // SAFETY: plain property writes on the shared info center, with a dictionary built here.
        unsafe {
            let center = MPNowPlayingInfoCenter::defaultCenter();
            match track {
                Some(track) => {
                    center.setNowPlayingInfo(Some(&info_dictionary(&track, &playback, cover)));
                    center.setPlaybackState(playback_state(playback.kind));
                }
                None => {
                    center.setNowPlayingInfo(None);
                    center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
                }
            }
        }
        self.published = Some((state, now, wall));
    }
}

/// Whether the Mac slept, or the process stood still, between the last report and now: the wall
/// clock moved further than the monotonic one by more than the tolerance for the progress tick.
/// A wall clock that went back counts as no sleep.
fn slept_since(at: Instant, wall_at: SystemTime, now: Instant, wall_now: SystemTime) -> bool {
    let monotonic = now.saturating_duration_since(at);
    wall_now
        .duration_since(wall_at)
        .is_ok_and(|wall| wall.saturating_sub(monotonic) > SEEK_TOLERANCE)
}

fn playback_state(kind: PlaybackKind) -> MPNowPlayingPlaybackState {
    match kind {
        PlaybackKind::Playing => MPNowPlayingPlaybackState::Playing,
        PlaybackKind::Paused => MPNowPlayingPlaybackState::Paused,
        PlaybackKind::Stopped => MPNowPlayingPlaybackState::Stopped,
    }
}

/// What Now Playing shows: the track, where playback is and how fast it moves, so that the
/// progress bar goes on by itself, and the cover.
fn info_dictionary(
    track: &NowPlaying,
    playback: &Playback,
    cover: Option<&Retained<MPMediaItemArtwork>>,
) -> Retained<NSDictionary<NSString, AnyObject>> {
    let mut keys: Vec<&NSString> = Vec::new();
    let mut values: Vec<Retained<AnyObject>> = Vec::new();
    let mut add = |key: &'static NSString, value: Retained<AnyObject>| {
        keys.push(key);
        values.push(value);
    };
    // SAFETY: the keys are constants of the MediaPlayer framework.
    unsafe {
        add(MPMediaItemPropertyTitle, string(&track.title));
        if let Some(artist) = &track.artist {
            add(MPMediaItemPropertyArtist, string(artist));
        }
        if let Some(album) = &track.album {
            add(MPMediaItemPropertyAlbumTitle, string(album));
        }
        if let Some(duration) = track.duration_secs {
            add(MPMediaItemPropertyPlaybackDuration, number(duration));
        }
        add(
            MPNowPlayingInfoPropertyElapsedPlaybackTime,
            number(playback.position_secs),
        );
        add(MPNowPlayingInfoPropertyPlaybackRate, number(playback.rate));
        if let Some(cover) = cover {
            add(
                MPMediaItemPropertyArtwork,
                Retained::into_super(Retained::into_super(cover.clone())),
            );
        }
    }
    let objects: Vec<&AnyObject> = values.iter().map(|value| &**value).collect();
    NSDictionary::from_slices(&keys, &objects)
}

fn string(value: &str) -> Retained<AnyObject> {
    Retained::into_super(Retained::into_super(NSString::from_str(value)))
}

fn number(value: f64) -> Retained<AnyObject> {
    Retained::into_super(Retained::into_super(Retained::into_super(
        NSNumber::new_f64(value),
    )))
}

/// The artwork for a cover file the app saved. The compressed bytes are all that is kept: the
/// image is decoded whenever the system asks for it and handed over to be released with its own
/// pool, so nothing decoded outlives the request.
fn read_cover(path: &std::path::Path) -> Option<Retained<MPMediaItemArtwork>> {
    if std::fs::metadata(path).ok()?.len() > MAX_COVER_BYTES {
        return None;
    }
    let data = NSData::from_vec(std::fs::read(path).ok()?);
    // The size is read from one decoded copy, which is released at once.
    let size = NSImage::initWithData(NSImage::alloc(), &data)?.size();
    if size.width <= 0.0 || size.height <= 0.0 {
        return None;
    }
    let handler = RcBlock::new(move |_requested: CGSize| -> NonNull<NSImage> {
        let image = NSImage::initWithData(NSImage::alloc(), &data)
            .unwrap_or_else(|| NSImage::init(NSImage::alloc()));
        NonNull::new(Retained::autorelease_ptr(image)).expect("an image is never null")
    });
    // SAFETY: the handler always returns a valid, autoreleased image.
    Some(unsafe {
        MPMediaItemArtwork::initWithBoundsSize_requestHandler(
            MPMediaItemArtwork::alloc(),
            CGSize::new(size.width, size.height),
            &handler,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(status: &str, position_secs: u64) -> PlayerState {
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
            ..PlayerState::default()
        }
    }

    #[test]
    fn a_sleep_the_monotonic_clock_did_not_see_is_noticed() {
        let at = Instant::now();
        let wall_at = SystemTime::now();
        let second = Duration::from_secs(1);
        // Playback that went on: both clocks moved the same.
        assert!(!slept_since(
            at,
            wall_at,
            at + 30 * second,
            wall_at + 30 * second
        ));
        // The progress tick is a little late on one clock or the other.
        assert!(!slept_since(
            at,
            wall_at,
            at + 5 * second,
            wall_at + 5 * second + Duration::from_millis(900)
        ));
        // A minute of sleep: the wall clock moved, the monotonic one hardly did.
        assert!(slept_since(at, wall_at, at + second, wall_at + 61 * second));
        // A wall clock set back is not a sleep.
        assert!(!slept_since(
            at,
            wall_at + 100 * second,
            at + second,
            wall_at
        ));
    }

    fn png_file(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("crabytube-{name}-{}.img", std::process::id()));
        let image = image::RgbaImage::from_pixel(8, 4, image::Rgba([200, 30, 30, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn text(value: Retained<AnyObject>) -> String {
        value.downcast::<NSString>().expect("a string").to_string()
    }

    fn real(value: Retained<AnyObject>) -> f64 {
        value.downcast::<NSNumber>().expect("a number").as_f64()
    }

    #[test]
    fn the_dictionary_carries_the_track_the_position_and_the_rate() {
        let track = now_playing(&state("Playing", 42)).unwrap();
        let dictionary = info_dictionary(&track, &playback(&state("Playing", 42)), None);
        // SAFETY: the keys are constants of the MediaPlayer framework.
        unsafe {
            assert_eq!(
                text(dictionary.objectForKey(MPMediaItemPropertyTitle).unwrap()),
                "Song"
            );
            assert_eq!(
                text(dictionary.objectForKey(MPMediaItemPropertyArtist).unwrap()),
                "Artist"
            );
            assert_eq!(
                text(
                    dictionary
                        .objectForKey(MPMediaItemPropertyAlbumTitle)
                        .unwrap()
                ),
                "Album"
            );
            assert_eq!(
                real(
                    dictionary
                        .objectForKey(MPMediaItemPropertyPlaybackDuration)
                        .unwrap()
                ),
                200.0
            );
            assert_eq!(
                real(
                    dictionary
                        .objectForKey(MPNowPlayingInfoPropertyElapsedPlaybackTime)
                        .unwrap()
                ),
                42.0
            );
            assert_eq!(
                real(
                    dictionary
                        .objectForKey(MPNowPlayingInfoPropertyPlaybackRate)
                        .unwrap()
                ),
                1.0
            );
            assert!(
                dictionary
                    .objectForKey(MPMediaItemPropertyArtwork)
                    .is_none()
            );
        }
        // Paused, the rate is zero.
        let paused = playback(&state("Paused", 42));
        let dictionary = info_dictionary(&track, &paused, None);
        // SAFETY: as above.
        let rate = unsafe { dictionary.objectForKey(MPNowPlayingInfoPropertyPlaybackRate) };
        assert_eq!(real(rate.unwrap()), 0.0);
    }

    #[test]
    fn a_saved_cover_becomes_artwork_that_decodes_on_request() {
        let path = png_file("cover");
        let artwork = read_cover(&path).expect("artwork");
        // SAFETY: plain reads of the artwork.
        let (bounds, image) = unsafe {
            (
                artwork.bounds(),
                artwork.imageWithSize(CGSize::new(8.0, 4.0)),
            )
        };
        assert_eq!((bounds.size.width, bounds.size.height), (8.0, 4.0));
        assert!(image.is_some());
        let dictionary = info_dictionary(
            &now_playing(&state("Playing", 0)).unwrap(),
            &playback(&state("Playing", 0)),
            Some(&artwork),
        );
        // SAFETY: as above.
        assert!(unsafe { dictionary.objectForKey(MPMediaItemPropertyArtwork) }.is_some());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_file_that_is_not_an_image_or_is_missing_gives_no_artwork() {
        let path =
            std::env::temp_dir().join(format!("crabytube-notimage-{}.img", std::process::id()));
        std::fs::write(&path, b"not an image at all").unwrap();
        assert!(read_cover(&path).is_none());
        let _ = std::fs::remove_file(path);
        assert!(read_cover(std::path::Path::new("/nonexistent/cover.img")).is_none());
    }

    #[test]
    fn what_the_player_reports_reaches_the_system_and_leaves_with_the_app() {
        let remote = MprisRemote::start();
        // SAFETY: plain reads of the shared info center.
        let center = unsafe { MPNowPlayingInfoCenter::defaultCenter() };
        remote.update(state("Playing", 10));
        // SAFETY: as above.
        unsafe {
            assert_eq!(center.playbackState(), MPNowPlayingPlaybackState::Playing);
            let info = center.nowPlayingInfo().expect("now playing info");
            assert_eq!(
                text(info.objectForKey(MPMediaItemPropertyTitle).unwrap()),
                "Song"
            );
        }
        remote.update(state("Paused", 10));
        // SAFETY: as above.
        unsafe {
            assert_eq!(center.playbackState(), MPNowPlayingPlaybackState::Paused);
            let info = center.nowPlayingInfo().unwrap();
            assert_eq!(
                real(
                    info.objectForKey(MPNowPlayingInfoPropertyPlaybackRate)
                        .unwrap()
                ),
                0.0
            );
        }
        // Nothing loaded: the system shows no track.
        remote.update(PlayerState::default());
        // SAFETY: as above.
        unsafe {
            assert!(center.nowPlayingInfo().is_none());
            assert_eq!(center.playbackState(), MPNowPlayingPlaybackState::Stopped);
        }
        remote.update(state("Playing", 0));
        drop(remote);
        // SAFETY: as above.
        unsafe {
            assert!(center.nowPlayingInfo().is_none());
            assert_eq!(center.playbackState(), MPNowPlayingPlaybackState::Stopped);
        }
    }
}
