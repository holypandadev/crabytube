use std::{
    io,
    num::{NonZeroU16, NonZeroU32},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use bytes::Bytes;
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use futures_util::{
    Stream, StreamExt,
    stream::{self, BoxStream},
};
use rodio::{Decoder, Player, Source};
use stream_download::{
    Settings, StreamDownload,
    source::{DecodeError, SourceStream},
    storage::temp::TempStorageProvider,
};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::live;
use crate::{
    music::Track,
    playback::resolver::{
        AudioQuality, AudioStream, StreamError, StreamResolver, TrackId,
        ValidatedPlaybackResolution,
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackState {
    #[default]
    Idle,
    Resolving,
    Buffering,
    Seeking,
    Playing,
    Paused,
    Stopped,
    Finished,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackStatus {
    pub state: PlaybackState,
    pub track: Option<Track>,
    pub duration: Option<Duration>,
    /// A live broadcast: it has no duration and cannot be sought.
    pub live: bool,
    pub position: Duration,
    pub volume: f32,
    pub underruns: u64,
    pub output_device: Option<String>,
    pub resolved_format: Option<ResolvedAudioFormat>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedAudioFormat {
    pub mime_type: String,
    pub bitrate: Option<u32>,
    pub itag: Option<u32>,
}

impl ResolvedAudioFormat {
    pub fn label(&self) -> String {
        let mut parts = vec![String::from("AAC")];
        if let Some(bitrate) = self.bitrate {
            parts.push(format!("{} kbps", bitrate / 1_000));
        }
        parts.push(self.mime_type.clone());
        if let Some(itag) = self.itag {
            parts.push(format!("itag {itag}"));
        }
        parts.join(" · ")
    }
}

impl Default for PlaybackStatus {
    fn default() -> Self {
        Self {
            state: PlaybackState::Idle,
            track: None,
            duration: None,
            live: false,
            position: Duration::ZERO,
            volume: 0.8,
            underruns: 0,
            output_device: None,
            resolved_format: None,
            error: None,
        }
    }
}

#[derive(Clone)]
pub struct PlaybackStatusReceiver {
    id: u64,
    receiver: watch::Receiver<PlaybackStatus>,
}

impl PartialEq for PlaybackStatusReceiver {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for PlaybackStatusReceiver {}

impl std::hash::Hash for PlaybackStatusReceiver {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

pub fn playback_status_stream(
    receiver: &PlaybackStatusReceiver,
) -> BoxStream<'static, PlaybackStatus> {
    stream::unfold(
        (receiver.receiver.clone(), None::<PlaybackStatus>),
        |(mut receiver, mut previous)| async move {
            loop {
                let status = if previous.is_none() {
                    receiver.borrow_and_update().clone()
                } else {
                    receiver.changed().await.ok()?;
                    receiver.borrow_and_update().clone()
                };
                let changed = previous
                    .as_ref()
                    .is_none_or(|old| playback_status_event_changed(old, &status));
                previous = Some(status.clone());
                if changed {
                    return Some((status, (receiver, previous)));
                }
            }
        },
    )
    .boxed()
}

fn playback_status_event_changed(old: &PlaybackStatus, new: &PlaybackStatus) -> bool {
    old.state != new.state
        || old.track != new.track
        || old.duration != new.duration
        || old.live != new.live
        || old.underruns != new.underruns
        || old.output_device != new.output_device
        || old.resolved_format != new.resolved_format
        || old.error != new.error
        || (!matches!(
            new.state,
            PlaybackState::Buffering | PlaybackState::Seeking | PlaybackState::Playing
        ) && old.position != new.position)
}

static NEXT_PLAYBACK_STATUS_STREAM_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct PlaybackHandle {
    commands: mpsc::UnboundedSender<Command>,
    status: watch::Receiver<PlaybackStatus>,
    status_stream_id: u64,
}

impl PlaybackHandle {
    pub fn new(resolver: Arc<dyn StreamResolver>) -> Self {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (status_tx, status) = watch::channel(PlaybackStatus::default());
        let status_stream_id = NEXT_PLAYBACK_STATUS_STREAM_ID.fetch_add(1, Ordering::Relaxed);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let spawned = thread::Builder::new()
            .name("crabytube-playback".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => runtime
                        .block_on(actor(command_rx, events_rx, events_tx, status_tx, resolver)),
                    Err(error) => error!(?error, "could not start playback runtime"),
                }
            });
        if let Err(error) = spawned {
            error!(?error, "could not start playback actor thread");
        }
        Self {
            commands,
            status,
            status_stream_id,
        }
    }

    pub fn status(&self) -> PlaybackStatus {
        self.status.borrow().clone()
    }

    pub fn status_receiver(&self) -> PlaybackStatusReceiver {
        PlaybackStatusReceiver {
            id: self.status_stream_id,
            receiver: self.status.clone(),
        }
    }

    pub fn play(&self, track: Track) {
        let _ = self.commands.send(Command::Play(track));
    }

    pub fn prefetch(&self, track: Track) {
        let _ = self.commands.send(Command::Prefetch(track));
    }

    pub fn prewarm(&self) {
        let _ = self.commands.send(Command::Prewarm);
    }

    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    pub fn resume(&self) {
        let _ = self.commands.send(Command::Resume);
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }

    pub fn seek(&self, position: Duration) {
        let _ = self.commands.send(Command::Seek(position));
    }

    pub fn set_volume(&self, volume: f32) {
        let _ = self
            .commands
            .send(Command::SetVolume(volume.clamp(0.0, 1.0)));
    }

    pub fn set_output_device(&self, device_id: Option<String>) {
        let _ = self.commands.send(Command::SetOutputDevice(device_id));
    }
}

#[derive(Debug, Error)]
enum EngineError {
    #[error("HTTP stream: {0}")]
    Network(String),
    #[error("decoder: {0}")]
    Decoder(String),
    #[error("audio worker: {0}")]
    Worker(String),
}

fn engine_error_message(error: &EngineError) -> &'static str {
    match error {
        EngineError::Network(_) => {
            "The audio stream could not be reached or was cut off by YouTube. Try again later."
        }
        EngineError::Decoder(_) => "This audio format could not be decoded.",
        EngineError::Worker(_) => "Playback could not continue.",
    }
}

enum Command {
    Play(Track),
    /// Resolves a track in the background so a later `Play` can skip the work.
    Prefetch(Track),
    /// Loads the player runtime before the first track is chosen.
    Prewarm,
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    SetOutputDevice(Option<String>),
}

enum Event {
    PreparedAhead {
        track: Track,
        generation: u64,
        duration: Option<Duration>,
        cancellation: CancellationToken,
        source: Result<PreparedAudio, EngineError>,
    },
    Resolved {
        generation: u64,
        selected_at: Instant,
        track: Track,
        cancellation: CancellationToken,
        result: Result<ValidatedPlaybackResolution, StreamError>,
    },
    Prepared {
        generation: u64,
        selected_at: Instant,
        track: Track,
        duration: Option<Duration>,
        source: Result<PreparedAudio, EngineError>,
    },
    Seeked {
        generation: u64,
        seek_id: u64,
        requested: Duration,
        result: Result<(), String>,
    },
    TrackEnded {
        generation: u64,
    },
    Failed {
        generation: u64,
        error: EngineError,
    },
    /// A prefetch could not resolve its track.
    PrefetchFailed {
        generation: u64,
    },
    /// A pause has lasted long enough to release the output device, if it is still that pause.
    OutputIdle {
        serial: u64,
    },
}

/// A pause this long releases the output device. An open device keeps calling back with silence,
/// and opening it again on resume takes a few tens of milliseconds.
const OUTPUT_IDLE_AFTER_PAUSE: Duration = Duration::from_secs(5);

/// Starts the grace period of a pause. The output is released when it ends, if the same pause still holds.
fn schedule_output_release(
    pause_serial: &mut u64,
    events_tx: &mpsc::UnboundedSender<Event>,
    after: Duration,
) {
    *pause_serial = pause_serial.wrapping_add(1);
    let serial = *pause_serial;
    let events_tx = events_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(after).await;
        let _ = events_tx.send(Event::OutputIdle { serial });
    });
}

/// Decoded samples reach the output in chunks of this many interleaved samples: one channel
/// operation per chunk instead of one per sample keeps the per-sample cost low while playing.
const AUDIO_CHUNK_SAMPLES: usize = 2048;

/// Hands one prepared chunk to the output channel, counting it as produced first.
/// Returns false when the output has gone.
fn hand_over(samples_tx: &Sender<Vec<f32>>, produced: &AtomicU64, chunk: Vec<f32>) -> bool {
    produced.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    samples_tx.send(chunk).is_ok()
}

enum WorkerCommand {
    Seek(Duration, tokio::sync::oneshot::Sender<Result<(), String>>),
    Stop,
}

struct ActiveAudio {
    samples: Receiver<Vec<f32>>,
    /// Samples the decoder has handed over so far; the cursor is behind it by what is buffered.
    produced: Arc<AtomicU64>,
    cursor: Arc<AtomicU64>,
    rate: u32,
    channels: u16,
    duration: Option<Duration>,
    worker: Sender<WorkerCommand>,
    first_sample_output: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
    drained: Sender<()>,
    selected_at: Instant,
}

impl ActiveAudio {
    /// A rodio source that reads this track's chunks from where the decoder is now.
    fn source(&self) -> AudioSource {
        AudioSource {
            samples: self.samples.clone(),
            drained: self.drained.clone(),
            current: Vec::new(),
            index: 0,
            cursor: Arc::clone(&self.cursor),
            rate: NonZeroU32::new(self.rate).unwrap_or(NonZeroU32::MIN),
            channels: NonZeroU16::new(self.channels).unwrap_or(NonZeroU16::MIN),
            duration: self.duration,
            first_sample_output: Arc::clone(&self.first_sample_output),
            underruns: Arc::clone(&self.underruns),
            in_underrun: false,
        }
    }
}

struct AudioSource {
    samples: Receiver<Vec<f32>>,
    /// Signalled each time a chunk is taken, so a decoder waiting for room can refill.
    drained: Sender<()>,
    /// The chunk being played, and the index of its next sample.
    current: Vec<f32>,
    index: usize,
    cursor: Arc<AtomicU64>,
    rate: NonZeroU32,
    channels: NonZeroU16,
    duration: Option<Duration>,
    first_sample_output: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
    in_underrun: bool,
}

impl Iterator for AudioSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index == self.current.len() {
            match self.samples.try_recv() {
                Ok(chunk) => {
                    // A signal already waiting is enough: the decoder re-checks the buffer when it wakes.
                    let _ = self.drained.try_send(());
                    self.current = chunk;
                    self.index = 0;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    if !self.in_underrun {
                        self.underruns.fetch_add(1, Ordering::Relaxed);
                        self.in_underrun = true;
                    }
                    return Some(0.0);
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => return None,
            }
        }
        let sample = self.current[self.index];
        self.index += 1;
        self.cursor.fetch_add(1, Ordering::Relaxed);
        self.first_sample_output.store(true, Ordering::Relaxed);
        self.in_underrun = false;
        Some(sample)
    }
}

impl Source for AudioSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZeroU16 {
        self.channels
    }
    fn sample_rate(&self) -> NonZeroU32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }
}

fn position(cursor: &AtomicU64, rate: u32, channels: u16) -> Duration {
    let frames = cursor.load(Ordering::Relaxed) / u64::from(channels.max(1));
    Duration::from_secs_f64(frames as f64 / f64::from(rate.max(1)))
}

fn generation_matches(active: u64, event: u64) -> bool {
    active == event
}

/// Whether a seek lands at or past the end of a track whose length is known.
fn seek_ends_track(target: Duration, duration: Option<Duration>) -> bool {
    duration.is_some_and(|duration| target >= duration)
}

/// Error text for the logs. Media URLs are signed, so every http(s) URL in the text becomes `<url>`.
fn redact_urls(text: &str) -> String {
    let mut redacted = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("http") {
        let candidate = &rest[start..];
        if !(candidate.starts_with("http://") || candidate.starts_with("https://")) {
            redacted.push_str(&rest[..start + "http".len()]);
            rest = &rest[start + "http".len()..];
            continue;
        }
        redacted.push_str(&rest[..start]);
        redacted.push_str("<url>");
        let end = candidate
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ',' | '}'))
            .unwrap_or(candidate.len());
        rest = &candidate[end..];
    }
    redacted.push_str(rest);
    redacted
}

fn transition(status: &mut PlaybackStatus, next: PlaybackState) {
    if is_valid_transition(status.state, next) {
        status.state = next;
    } else {
        warn!(from = ?status.state, to = ?next, "invalid playback state transition");
        status.state = PlaybackState::Error;
        status.error = Some("Playback entered an invalid state.".into());
    }
}

fn is_valid_transition(from: PlaybackState, to: PlaybackState) -> bool {
    use PlaybackState::{
        Buffering, Error, Finished, Idle, Paused, Playing, Resolving, Seeking, Stopped,
    };
    from == to
        || matches!(
            (from, to),
            (Idle, Resolving | Stopped | Error)
                | (Resolving, Buffering | Paused | Stopped | Error)
                | (
                    Buffering,
                    Playing | Paused | Seeking | Stopped | Finished | Error | Resolving
                )
                | (
                    Playing,
                    Paused | Buffering | Seeking | Stopped | Finished | Error | Resolving
                )
                | (
                    Seeking,
                    Buffering | Paused | Playing | Stopped | Error | Resolving
                )
                | (
                    Paused,
                    Playing | Buffering | Seeking | Stopped | Finished | Error | Resolving
                )
                | (Stopped | Finished, Resolving | Stopped | Idle | Error)
                | (Error, Resolving | Stopped | Idle)
        )
}

async fn actor(
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut events: mpsc::UnboundedReceiver<Event>,
    events_tx: mpsc::UnboundedSender<Event>,
    status_tx: watch::Sender<PlaybackStatus>,
    resolver: Arc<dyn StreamResolver>,
) {
    let mut status = PlaybackStatus::default();
    let _ = status_tx.send(status.clone());
    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    let mut generation = 0_u64;
    let mut seek_generation = 0_u64;
    let mut cancellation: Option<CancellationToken> = None;
    let mut active: Option<ActiveAudio> = None;
    let mut output: Option<PlaybackOutput> = None;
    let mut output_device_id: Option<String> = None;
    let mut paused_by_user = false;
    // Counts pauses, so a release timer from an earlier pause is ignored.
    let mut pause_serial = 0_u64;
    let mut first_audio_logged = false;
    let mut decoder_finished = false;
    let mut observed_underruns = 0_u64;
    // The next track, opened and decoding ahead of its `Play`.
    let mut prepared_ahead: Option<PreparedAhead> = None;
    // Set when an expired URL stopped a track: where to resume once the new stream is ready.
    let mut resume_at: Option<Duration> = None;
    // One re-resolution per track; a second refusal is reported as a failure.
    let mut resumed_this_track = false;
    // The prefetch in flight, tracked so that Play and Stop can cancel it or wait for it.
    let mut prefetching: Option<Prefetching> = None;
    // A Play that waits for the prefetch of its track.
    let mut awaiting: Option<Awaiting> = None;

    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Prewarm) => {
                    let resolver = Arc::clone(&resolver);
                    tokio::spawn(async move { resolver.prewarm().await });
                }
                Some(Command::Prefetch(track)) => {
                    // Nothing changes while a Play waits for a prefetch, or while this track is already
                    // prepared or in flight. Any other prefetch replaces the one before it.
                    let wanted = awaiting.is_some()
                        || prefetching.as_ref().is_some_and(|prefetch| prefetch.video_id == track.video_id)
                        || prepared_ahead.as_ref().is_some_and(|ahead| ahead.track.video_id == track.video_id);
                    if !wanted {
                        if let Some(stale) = prefetching.take() {
                            stale.cancellation.cancel();
                        }
                        if let Some(stale) = prepared_ahead.take() {
                            stale.discard();
                        }
                        let resolver = Arc::clone(&resolver);
                        let events_tx = events_tx.clone();
                        let prefetch_generation = next_generation();
                        let token = CancellationToken::new();
                        prefetching = Some(Prefetching {
                            video_id: track.video_id.clone(),
                            generation: prefetch_generation,
                            cancellation: token.clone(),
                            decoder_events: Vec::new(),
                        });
                        tokio::spawn(async move {
                            let mut track = track;
                            let Ok(stream) = resolver.resolve(&TrackId(track.video_id.clone()), AudioQuality::Auto, token.clone()).await else {
                                let _ = events_tx.send(Event::PrefetchFailed { generation: prefetch_generation });
                                return;
                            };
                            stream.fill_track(&mut track);
                            if let Some((duration, source)) = prepare_stream(stream, token.clone(), prefetch_generation, events_tx.clone(), false).await {
                                let _ = events_tx.send(Event::PreparedAhead { track, generation: prefetch_generation, duration, cancellation: token, source });
                            }
                        });
                    }
                }
                Some(Command::Play(track)) => {
                    if let Some(token) = cancellation.take() { token.cancel(); }
                    if let Some(audio) = active.take() { let _ = audio.worker.send(WorkerCommand::Stop); }
                    if let Some(output) = output.as_ref() {
                        output.player.clear();
                    }
                    awaiting = None;
                    // The output stays open between tracks: reopening the device costs time at every change.
                    let prepared = prepared_ahead.take();
                    let plan = play_plan(
                        prepared.as_ref().map(|ahead| ahead.track.video_id.as_str()),
                        prefetching.as_ref().map(|prefetch| prefetch.video_id.as_str()),
                        &track.video_id,
                    );
                    let ahead = match prepared {
                        Some(ahead) if plan == PlayPlan::Prepared => Some(ahead),
                        Some(stale) => {
                            stale.discard();
                            None
                        }
                        None => None,
                    };
                    // The prefetch of this track is waited for; a prefetch of any other track is cancelled.
                    let in_flight = match prefetching.take() {
                        Some(prefetch) if plan == PlayPlan::Awaited => Some(prefetch),
                        Some(stale) => {
                            stale.cancellation.cancel();
                            None
                        }
                        None => None,
                    };
                    generation = match (&ahead, &in_flight) {
                        (Some(ahead), _) => ahead.generation,
                        (None, Some(prefetch)) => prefetch.generation,
                        (None, None) => next_generation(),
                    };
                    seek_generation = seek_generation.wrapping_add(1);
                    paused_by_user = false;
                    resume_at = None;
                    resumed_this_track = false;
                    first_audio_logged = false;
                    decoder_finished = false;
                    let selected_at = Instant::now();
                    debug!(video_id = %track.video_id, prepared = ahead.is_some(), "search result selected");
                    transition(&mut status, PlaybackState::Resolving);
                    status.track = Some(track.clone());
                    status.duration = None;
                    status.position = Duration::ZERO;
                    status.underruns = 0;
                    status.resolved_format = None;
                    status.error = None;
                    let _ = status_tx.send(status.clone());
                    let events_tx = events_tx.clone();
                    let current_generation = generation;
                    match (ahead, in_flight) {
                        (Some(ahead), _) => {
                            // The stream was opened while the previous track played: hand it to the output now.
                            // Its track carries the metadata the resolver filled in, so the UI can show it.
                            cancellation = Some(ahead.cancellation.clone());
                            let _ = events_tx.send(Event::Prepared { generation: ahead.generation, selected_at, track: ahead.track, duration: ahead.duration, source: Ok(ahead.source) });
                            // Decoder events from while it was prepared apply now that it plays.
                            for event in ahead.decoder_events {
                                let _ = events_tx.send(event);
                            }
                        }
                        (None, Some(prefetch)) => {
                            // The same track is still being prefetched: its stream starts playback when it lands.
                            cancellation = Some(prefetch.cancellation.clone());
                            prefetching = Some(prefetch);
                            awaiting = Some(Awaiting { track, selected_at });
                        }
                        (None, None) => {
                            let token = CancellationToken::new();
                            cancellation = Some(token.clone());
                            let resolver = Arc::clone(&resolver);
                            tokio::spawn(async move {
                                let result = resolver.resolve(&TrackId(track.video_id.clone()), AudioQuality::Auto, token.clone()).await;
                                let _ = events_tx.send(Event::Resolved { generation: current_generation, selected_at, track, cancellation: token, result });
                            });
                        }
                    }
                }
                Some(Command::Pause) => {
                    if status.state == PlaybackState::Resolving {
                        paused_by_user = true;
                        transition(&mut status, PlaybackState::Paused);
                        schedule_output_release(&mut pause_serial, &events_tx, OUTPUT_IDLE_AFTER_PAUSE);
                        let _ = status_tx.send(status.clone());
                    } else if status.state == PlaybackState::Seeking {
                        // The seek lands paused: its Seeked event leaves the output closed or silent.
                        paused_by_user = true;
                    } else if matches!(status.state, PlaybackState::Playing | PlaybackState::Buffering) {
                        if let Some(output) = output.as_ref() {
                            output.player.pause();
                        }
                        paused_by_user = true;
                        transition(&mut status, PlaybackState::Paused);
                        info!(position_ms = status.position.as_millis(), "pause");
                        schedule_output_release(&mut pause_serial, &events_tx, OUTPUT_IDLE_AFTER_PAUSE);
                        let _ = status_tx.send(status.clone());
                    }
                }
                Some(Command::Resume) => {
                    if status.state == PlaybackState::Paused {
                        paused_by_user = false;
                        if let Some(audio) = active.as_ref() {
                            // Released during a long pause: open the output again. The new source reads
                            // the chunks the decoder kept, so playback continues where it stopped.
                            let ready = if output.is_some() {
                                true
                            } else {
                                let started = std::time::Instant::now();
                                match reopen_output(audio, status.volume, output_device_id.as_deref()) {
                                    Ok(opened) => {
                                        status.output_device = Some(opened.device_name.clone());
                                        output = Some(opened);
                                        info!(elapsed_ms = started.elapsed().as_millis(), "audio output reopened after a pause");
                                        true
                                    }
                                    Err(error) => {
                                        transition(&mut status, PlaybackState::Error);
                                        status.error = Some(error.user_message());
                                        status.output_device = None;
                                        false
                                    }
                                }
                            };
                            if ready {
                                if let Some(output) = output.as_ref() {
                                    output.player.play();
                                }
                                transition(&mut status, PlaybackState::Buffering);
                            }
                        } else if cancellation.as_ref().is_some_and(|token| !token.is_cancelled()) {
                            transition(&mut status, PlaybackState::Resolving);
                        }
                        info!(position_ms = status.position.as_millis(), "resume");
                        let _ = status_tx.send(status.clone());
                    } else if status.state == PlaybackState::Seeking {
                        // Play during a seek: the Seeked event starts the output.
                        paused_by_user = false;
                    }
                }
                Some(Command::Stop) => {
                    if let Some(token) = cancellation.take() { token.cancel(); }
                    if let Some(audio) = active.take() { let _ = audio.worker.send(WorkerCommand::Stop); }
                    // Events of the stopped track, including one prepared ahead, must not apply to what follows.
                    if let Some(stale) = prepared_ahead.take() { stale.discard(); }
                    if let Some(prefetch) = prefetching.take() { prefetch.cancellation.cancel(); }
                    awaiting = None;
                    generation = next_generation();
                    if let Some(output) = output.as_ref() {
                        output.player.clear();
                    }
                    output = None;
                    status.output_device = None;
                    transition(&mut status, PlaybackState::Stopped);
                    status.position = Duration::ZERO;
                    info!("stop");
                    let _ = status_tx.send(status.clone());
                }
                Some(Command::SetVolume(volume)) => {
                    if let Some(output) = output.as_ref() {
                        output.player.set_volume(volume);
                    }
                    status.volume = volume;
                    let _ = status_tx.send(status.clone());
                }
                Some(Command::SetOutputDevice(device_id)) => {
                    output_device_id = device_id;
                }
                Some(Command::Seek(position)) => {
                    if !status.live && active.is_some() && seek_ends_track(position, status.duration) {
                        // Past the end there is nothing left to play: the track finishes as it does at its last sample.
                        let _ = events_tx.send(Event::TrackEnded { generation });
                    // A live broadcast has no position to go back to.
                    } else if let Some(audio) = active.as_ref().filter(|_| !status.live) {
                        seek_generation = seek_generation.wrapping_add(1);
                        let current_seek_generation = seek_generation;
                        let (reply, response) = tokio::sync::oneshot::channel();
                        if audio.worker.send(WorkerCommand::Seek(position, reply)).is_ok() {
                            if let Some(output) = output.as_ref() {
                                output.player.pause();
                                output.player.clear();
                            }
                            let events_tx = events_tx.clone();
                            let current_generation = generation;
                            tokio::spawn(async move {
                                let result = response.await.unwrap_or_else(|_| Err("audio decoder stopped during seek".into()));
                                let _ = events_tx.send(Event::Seeked { generation: current_generation, seek_id: current_seek_generation, requested: position, result });
                            });
                            transition(&mut status, PlaybackState::Seeking);
                            let _ = status_tx.send(status.clone());
                        }
                    }
                }
                None => break,
            },
            event = events.recv() => match event {
                Some(event) if decoder_event_held(&prefetching, &prepared_ahead, &event) => {
                    hold_decoder_event(&mut prefetching, &mut prepared_ahead, event);
                }
                Some(Event::PreparedAhead { track, generation: ahead_generation, duration, cancellation: token, source }) => {
                    let tracked = prefetching.as_ref().is_some_and(|prefetch| prefetch.generation == ahead_generation);
                    if let Some(waiting) = awaiting.take_if(|_| tracked && generation_matches(generation, ahead_generation)) {
                        // A Play waits for this stream: it starts exactly as a stream prepared ahead would.
                        let held = prefetching.take().map(|prefetch| prefetch.decoder_events).unwrap_or_default();
                        let _ = events_tx.send(Event::Prepared { generation: ahead_generation, selected_at: waiting.selected_at, track, duration, source });
                        for event in held {
                            let _ = events_tx.send(event);
                        }
                    } else if tracked {
                        let held = prefetching.take().map(|prefetch| prefetch.decoder_events).unwrap_or_default();
                        match source {
                            Ok(source) => {
                                let ahead = PreparedAhead { track, generation: ahead_generation, duration, cancellation: token, source, decoder_events: held };
                                if let Some(stale) = prepared_ahead.replace(ahead) {
                                    stale.discard();
                                }
                            }
                            Err(error) => debug!(error = %redact_urls(&format!("{error:?}")), "next track could not be prepared ahead"),
                        }
                    } else {
                        // Its prefetch was superseded or stopped, so no Play wants this stream.
                        token.cancel();
                        if let Ok(source) = source {
                            let _ = source.worker.send(WorkerCommand::Stop);
                        }
                    }
                }
                Some(Event::PrefetchFailed { generation: failed_generation }) => {
                    let _ = prefetching.take_if(|prefetch| prefetch.generation == failed_generation);
                    if let Some(waiting) = awaiting.take_if(|_| generation_matches(generation, failed_generation)) {
                        // The prefetch could not resolve the track, so the waiting Play resolves it.
                        let token = CancellationToken::new();
                        cancellation = Some(token.clone());
                        let resolver = Arc::clone(&resolver);
                        let events_tx = events_tx.clone();
                        let current_generation = generation;
                        tokio::spawn(async move {
                            let result = resolver.resolve(&TrackId(waiting.track.video_id.clone()), AudioQuality::Auto, token.clone()).await;
                            let _ = events_tx.send(Event::Resolved { generation: current_generation, selected_at: waiting.selected_at, track: waiting.track, cancellation: token, result });
                        });
                    }
                }
                Some(Event::Resolved { generation: event_generation, selected_at, track, cancellation: token, result }) if generation_matches(generation, event_generation) => match result {
                    Ok(stream) => {
                        info!(elapsed_ms = selected_at.elapsed().as_millis(), "click to stream resolved");
                        if !paused_by_user {
                            transition(&mut status, PlaybackState::Buffering);
                        }
                        status.duration = stream.duration;
                        status.resolved_format = Some(ResolvedAudioFormat {
                            mime_type: stream.mime_type.clone(),
                            bitrate: stream.bitrate,
                            itag: stream.itag,
                        });
                        let _ = status_tx.send(status.clone());
                        let mut track = track;
                        stream.fill_track(&mut track);
                        let events_tx = events_tx.clone();
                        tokio::spawn(async move {
                            if let Some((duration, source)) = prepare_stream(stream, token, event_generation, events_tx.clone(), true).await {
                                let _ = events_tx.send(Event::Prepared { generation: event_generation, selected_at, track, duration, source });
                            }
                        });
                    }
                    Err(StreamError::Cancelled) => {}
                    Err(error) => {
                        output = None;
                        resume_at = None;
                        error!(error = %redact_urls(&format!("{error:?}")), "stream resolution failed");
                        transition(&mut status, PlaybackState::Error);
                        status.error = Some(error.user_message());
                        status.output_device = None;
                        let _ = status_tx.send(status.clone());
                    }
                },
                Some(Event::Prepared { generation: event_generation, selected_at, track, duration, source }) if generation_matches(generation, event_generation) => match source {
                    Ok(active_audio) => {
                        // A paused track does not open the device: resume opens it and queues the source.
                        if output.is_none() && !paused_by_user {
                            let opened = open_selected_output(status.volume, output_device_id.as_deref());
                            match opened {
                                Ok(opened) => {
                                    status.output_device = Some(opened.device_name.clone());
                                    output = Some(opened);
                                }
                                Err(error) => {
                                    let _ = active_audio.worker.send(WorkerCommand::Stop);
                                    transition(&mut status, PlaybackState::Error);
                                    status.error = Some(error.user_message());
                                    status.output_device = None;
                                    let _ = status_tx.send(status.clone());
                                    continue;
                                }
                            }
                        }
                        status.track = Some(track);
                        status.duration = duration;
                        status.live = active_audio.live;
                        status.position = Duration::ZERO;
                        status.underruns = 0;
                        observed_underruns = 0;
                        transition(&mut status, if paused_by_user { PlaybackState::Paused } else { PlaybackState::Buffering });
                        if let Some(output) = output.as_ref() {
                            output.player.set_volume(status.volume);
                        }
                        let rodio_source = AudioSource {
                            samples: active_audio.samples.clone(),
                            drained: active_audio.drained.clone(),
                            current: Vec::new(),
                            index: 0,
                            cursor: Arc::clone(&active_audio.cursor),
                            rate: active_audio.rate,
                            channels: active_audio.channels,
                            duration,
                            first_sample_output: Arc::clone(&active_audio.first_sample_output),
                            underruns: Arc::clone(&active_audio.underruns),
                            in_underrun: false,
                        };
                        active = Some(ActiveAudio { samples: active_audio.samples, produced: active_audio.produced, cursor: active_audio.cursor, rate: active_audio.rate.get(), channels: active_audio.channels.get(), duration, worker: active_audio.worker, first_sample_output: active_audio.first_sample_output, underruns: active_audio.underruns, drained: active_audio.drained, selected_at });
                        if let Some(position) = resume_at.take() {
                            // The new decoder seeks to where playback stopped first; the Seeked event then starts the output.
                            seek_generation = seek_generation.wrapping_add(1);
                            let current_seek_generation = seek_generation;
                            let (reply, response) = tokio::sync::oneshot::channel();
                            if let Some(audio) = active.as_ref() {
                                let _ = audio.worker.send(WorkerCommand::Seek(position, reply));
                            }
                            let events_tx = events_tx.clone();
                            let current_generation = generation;
                            tokio::spawn(async move {
                                let result = response.await.unwrap_or_else(|_| Err("audio decoder stopped during seek".into()));
                                let _ = events_tx.send(Event::Seeked { generation: current_generation, seek_id: current_seek_generation, requested: position, result });
                            });
                            transition(&mut status, PlaybackState::Seeking);
                        } else if let Some(output) = output.as_ref() {
                            output.player.append(rodio_source);
                            if paused_by_user {
                                schedule_output_release(&mut pause_serial, &events_tx, OUTPUT_IDLE_AFTER_PAUSE);
                            } else {
                                output.player.play();
                            }
                        }
                        info!("audio source prepared");
                        let _ = status_tx.send(status.clone());
                    }
                    Err(error) => {
                        output = None;
                        error!(error = %redact_urls(&format!("{error:?}")), "stream open or decoder setup failed");
                        transition(&mut status, PlaybackState::Error);
                        status.error = Some(engine_error_message(&error).into());
                        let _ = status_tx.send(status.clone());
                    }
                },
                Some(Event::Seeked { generation: event_generation, seek_id, requested, result }) if generation_matches(generation, event_generation) && generation_matches(seek_generation, seek_id) => {
                    match (result, active.as_ref(), output.as_ref()) {
                        (Ok(()), Some(audio), Some(output)) => {
                            audio.first_sample_output.store(false, Ordering::Relaxed);
                            observed_underruns = audio.underruns.load(Ordering::Relaxed);
                            status.position = position(&audio.cursor, audio.rate, audio.channels);
                            transition(&mut status, if paused_by_user { PlaybackState::Paused } else { PlaybackState::Buffering });
                            let rodio_source = audio.source();
                            output.player.append(rodio_source);
                            if paused_by_user {
                                schedule_output_release(&mut pause_serial, &events_tx, OUTPUT_IDLE_AFTER_PAUSE);
                            } else {
                                output.player.play();
                            }
                            info!(position_ms = requested.as_millis(), "seek");
                        }
                        (Ok(()), Some(audio), None) => {
                            // Only a paused track loses its output; the new position waits for the resume.
                            audio.first_sample_output.store(false, Ordering::Relaxed);
                            observed_underruns = audio.underruns.load(Ordering::Relaxed);
                            status.position = position(&audio.cursor, audio.rate, audio.channels);
                            transition(&mut status, if paused_by_user { PlaybackState::Paused } else { PlaybackState::Buffering });
                            info!(position_ms = requested.as_millis(), "seek");
                        }
                        (Err(error), Some(audio), output) => {
                            // The output was cleared for the seek, so the unchanged source is queued again.
                            warn!(%error, "seek failed; continuing from the current position");
                            audio.first_sample_output.store(false, Ordering::Relaxed);
                            observed_underruns = audio.underruns.load(Ordering::Relaxed);
                            status.position = position(&audio.cursor, audio.rate, audio.channels);
                            if let Some(output) = output {
                                output.player.append(audio.source());
                                if paused_by_user {
                                    schedule_output_release(&mut pause_serial, &events_tx, OUTPUT_IDLE_AFTER_PAUSE);
                                } else {
                                    output.player.play();
                                }
                            }
                            transition(&mut status, if paused_by_user { PlaybackState::Paused } else { PlaybackState::Buffering });
                        }
                        (Err(error), None, _) => {
                            error!(%error, "seek failed and the audio stream is gone");
                            transition(&mut status, PlaybackState::Error);
                            status.error = Some(engine_error_message(&EngineError::Worker(error)).into());
                        }
                        _ => {}
                    }
                    let _ = status_tx.send(status.clone());
                }
                Some(Event::TrackEnded { generation: event_generation }) if generation_matches(generation, event_generation) => {
                    decoder_finished = true;
                    info!(generation, "TrackEnded received from decoder EOF");
                }
                Some(Event::Failed { generation: event_generation, error: failure }) if generation_matches(generation, event_generation) && !resumed_this_track && refused_by_googlevideo(&failure) && status.track.is_some() => {
                    // Googlevideo refused the URL mid-track, usually because it expired. Resolve once more and
                    // resume from the position playback reached; the output stays open, so the pause is short.
                    resumed_this_track = true;
                    resume_at = Some(status.position);
                    warn!(failure = %redact_urls(&format!("{failure:?}")), position_ms = status.position.as_millis(), "stream URL refused mid-track; resolving again from this position");
                    if let Some(audio) = active.take() { let _ = audio.worker.send(WorkerCommand::Stop); }
                    if let Some(output) = output.as_ref() {
                        output.player.clear();
                    }
                    let Some(track) = status.track.clone() else { continue };
                    let token = CancellationToken::new();
                    cancellation = Some(token.clone());
                    generation = next_generation();
                    decoder_finished = false;
                    first_audio_logged = false;
                    transition(&mut status, PlaybackState::Resolving);
                    status.error = None;
                    let _ = status_tx.send(status.clone());
                    let resolver = Arc::clone(&resolver);
                    let events_tx = events_tx.clone();
                    let current_generation = generation;
                    let selected_at = Instant::now();
                    tokio::spawn(async move {
                        let result = resolver.resolve(&TrackId(track.video_id.clone()), AudioQuality::Auto, token.clone()).await;
                        let _ = events_tx.send(Event::Resolved { generation: current_generation, selected_at, track, cancellation: token, result });
                    });
                }
                Some(Event::Failed { generation: event_generation, error: failure }) if generation_matches(generation, event_generation) => {
                    error!(failure = %redact_urls(&format!("{failure:?}")), "playback stream failed after starting");
                    transition(&mut status, PlaybackState::Error);
                    status.error = Some(engine_error_message(&failure).into());
                    if let Some(audio) = active.take() { let _ = audio.worker.send(WorkerCommand::Stop); }
                    if let Some(output) = output.as_ref() {
                        output.player.clear();
                    }
                    output = None;
                    status.output_device = None;
                    let _ = status_tx.send(status.clone());
                }
                Some(Event::OutputIdle { serial }) if serial == pause_serial && paused_by_user => {
                    // Still paused after the grace period: close the device, which otherwise keeps calling back.
                    if output.is_some() {
                        output = None;
                        info!("audio output released after a long pause");
                    }
                }
                Some(_) => {}
                None => break,
            },
            _ = ticker.tick(), if active.is_some() && !paused_by_user => {
                if let Some(audio) = active.as_ref() {
                    status.position = position(&audio.cursor, audio.rate, audio.channels);
                    let underruns = audio.underruns.load(Ordering::Relaxed);
                    status.underruns = underruns;
                    if underruns > observed_underruns {
                        observed_underruns = underruns;
                        if status.state == PlaybackState::Playing {
                            if let Some(output) = output.as_ref() {
                                output.player.pause();
                            }
                            transition(&mut status, PlaybackState::Buffering);
                            info!(underruns, "audio output underrun; buffering for recovery");
                        }
                    }
                    let audio_started = audio.first_sample_output.load(Ordering::Relaxed);
                    if !first_audio_logged && audio_started {
                        first_audio_logged = true;
                        info!(elapsed_ms = audio.selected_at.elapsed().as_millis(), "playback started after device consumed first audio sample");
                    }
                    // Samples the decoder has handed over and the output has not played yet.
                    let buffered = audio.produced.load(Ordering::Relaxed).saturating_sub(audio.cursor.load(Ordering::Relaxed));
                    if !paused_by_user && !decoder_finished {
                        if buffered < u64::from(audio.channels) * u64::from(audio.rate / 8) && status.state == PlaybackState::Playing {
                            if let Some(output) = output.as_ref() {
                                output.player.pause();
                            }
                            transition(&mut status, PlaybackState::Buffering);
                            info!("buffering started");
                        } else if buffered > u64::from(audio.channels) * u64::from(audio.rate / 2) && status.state == PlaybackState::Buffering {
                            if let Some(output) = output.as_ref() {
                                output.player.play();
                            }
                            if audio_started { transition(&mut status, PlaybackState::Playing); }
                        }
                    }
                    let finished = decoder_finished && buffered == 0;
                    if finished {
                        transition(&mut status, PlaybackState::Finished);
                        status.position = status.duration.unwrap_or(status.position);
                        if let Some(token) = cancellation.take() {
                            token.cancel();
                        }
                        if let Some(audio) = active.as_ref() {
                            let _ = audio.worker.send(WorkerCommand::Stop);
                        }
                        if let Some(output) = output.as_ref() {
                            output.player.clear();
                        }
                        active = None;
                        info!("track finished");
                    }
                    let _ = status_tx.send(status.clone());
                }
            }
        }
    }
}

/// Process-wide generation source: each track gets a new value, so events from an older
/// track (including one prepared ahead) can never pass for the current one.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// A stream opened and decoding while the previous track plays; used by a matching `Play`.
struct PreparedAhead {
    track: Track,
    generation: u64,
    duration: Option<Duration>,
    cancellation: CancellationToken,
    source: PreparedAudio,
    /// Decoder events of this stream, kept so they apply if it plays.
    decoder_events: Vec<Event>,
}

impl PreparedAhead {
    fn discard(self) {
        self.cancellation.cancel();
        let _ = self.source.worker.send(WorkerCommand::Stop);
    }
}

/// A prefetch whose stream is being resolved or opened. Play and Stop can cancel or wait for it.
struct Prefetching {
    video_id: String,
    generation: u64,
    cancellation: CancellationToken,
    /// Decoder events of this stream. They arrive before the stream is prepared.
    decoder_events: Vec<Event>,
}

/// A Play that waits for the prefetch of its track.
struct Awaiting {
    track: Track,
    selected_at: Instant,
}

/// How a Play starts its track.
#[derive(Debug, PartialEq, Eq)]
enum PlayPlan {
    /// The stream prepared ahead for this track plays now.
    Prepared,
    /// The prefetch of this track is still in flight, and it starts playback when it lands.
    Awaited,
    /// Nothing is ready for this track, so it is resolved.
    Resolve,
}

fn play_plan(prepared: Option<&str>, prefetching: Option<&str>, requested: &str) -> PlayPlan {
    if prepared == Some(requested) {
        PlayPlan::Prepared
    } else if prefetching == Some(requested) {
        PlayPlan::Awaited
    } else {
        PlayPlan::Resolve
    }
}

/// The stream a decoder event belongs to.
fn decoder_event_generation(event: &Event) -> Option<u64> {
    match event {
        Event::TrackEnded { generation } | Event::Failed { generation, .. } => Some(*generation),
        _ => None,
    }
}

/// Whether a decoder event belongs to a stream that is not playing yet. Such events are kept with it.
fn decoder_event_held(
    prefetching: &Option<Prefetching>,
    prepared_ahead: &Option<PreparedAhead>,
    event: &Event,
) -> bool {
    decoder_event_generation(event).is_some_and(|generation| {
        prefetching
            .as_ref()
            .is_some_and(|prefetch| prefetch.generation == generation)
            || prepared_ahead
                .as_ref()
                .is_some_and(|ahead| ahead.generation == generation)
    })
}

fn hold_decoder_event(
    prefetching: &mut Option<Prefetching>,
    prepared_ahead: &mut Option<PreparedAhead>,
    event: Event,
) {
    let generation = decoder_event_generation(&event);
    if let Some(prefetch) = prefetching
        .as_mut()
        .filter(|prefetch| Some(prefetch.generation) == generation)
    {
        prefetch.decoder_events.push(event);
    } else if let Some(ahead) = prepared_ahead
        .as_mut()
        .filter(|ahead| Some(ahead.generation) == generation)
    {
        ahead.decoder_events.push(event);
    }
}

/// Opens the stream and starts its decoder. An expired URL fails before any output work.
/// Returns `None` when cancelled first.
async fn prepare_stream(
    stream: ValidatedPlaybackResolution,
    token: CancellationToken,
    generation: u64,
    events: mpsc::UnboundedSender<Event>,
    // Only a stream that plays now takes the diagnostic refusal; a prefetch does not.
    #[cfg_attr(not(feature = "diagnostics"), allow(unused_variables))] for_playback: bool,
) -> Option<(Option<Duration>, Result<PreparedAudio, EngineError>)> {
    if stream.is_expired(SystemTime::now()) {
        let error =
            EngineError::Network("validated media URL expired before decoder startup".into());
        return Some((None, Err(error)));
    }
    let initial_duration = stream.duration;
    if stream.live {
        // A broadcast has no length: its fragments are fetched as they come, from the live edge.
        let stream = stream.into_stream();
        let client = match media_client(&stream) {
            Ok(client) => client,
            Err(error) => return Some((None, Err(error))),
        };
        let url = match reqwest::Url::parse(&stream.url) {
            Ok(url) => url,
            Err(error) => {
                return Some((None, Err(EngineError::Network(error.to_string()))));
            }
        };
        let fetch_error = Arc::new(Mutex::new(None));
        let reader = live::open_broadcast(client, url, token.clone(), Arc::clone(&fetch_error));
        let source = create_audio_source(
            reader,
            MediaKind::Live,
            token,
            generation,
            events,
            fetch_error,
            for_playback,
        )
        .await;
        return Some((None, source));
    }
    let opened = tokio::select! {
        _ = token.cancelled() => return None,
        opened = open_stream(stream.into_stream()) => opened,
    };
    let (reader, fetch_error) = match opened {
        Ok(opened) => opened,
        Err(error) => return Some((None, Err(error))),
    };
    let byte_len = reader.content_length();
    let source = create_audio_source(
        reader,
        MediaKind::Progressive { byte_len },
        token,
        generation,
        events,
        fetch_error,
        for_playback,
    )
    .await;
    Some((initial_duration, source))
}

struct PreparedAudio {
    samples: Receiver<Vec<f32>>,
    produced: Arc<AtomicU64>,
    cursor: Arc<AtomicU64>,
    rate: NonZeroU32,
    channels: NonZeroU16,
    worker: Sender<WorkerCommand>,
    first_sample_output: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
    drained: Sender<()>,
    live: bool,
}

struct PlaybackOutput {
    _device: rodio::MixerDeviceSink,
    player: Player,
    device_name: String,
}

impl PlaybackOutput {
    fn open(volume: f32, selected_device_id: Option<&str>) -> Result<Self, StreamError> {
        let (device, device_name) = crate::playback::output::open_output_device(selected_device_id)
            .map_err(StreamError::AudioDevice)?;
        let player = Player::connect_new(device.mixer());
        player.set_volume(volume);
        Ok(Self {
            _device: device,
            player,
            device_name,
        })
    }
}

/// Opens the selected device, or the system default when the selected one has disappeared
/// (unplugged headphones, for example).
fn open_selected_output(
    volume: f32,
    selected_device_id: Option<&str>,
) -> Result<PlaybackOutput, StreamError> {
    PlaybackOutput::open(volume, selected_device_id).or_else(|error| {
        if selected_device_id.is_some() {
            warn!(
                ?error,
                "selected output device unavailable; using the system default"
            );
            PlaybackOutput::open(volume, None)
        } else {
            Err(error)
        }
    })
}

/// Opens the output again after a release, with a source that reads the track's chunks.
fn reopen_output(
    audio: &ActiveAudio,
    volume: f32,
    selected_device_id: Option<&str>,
) -> Result<PlaybackOutput, StreamError> {
    let opened = open_selected_output(volume, selected_device_id)?;
    opened.player.append(audio.source());
    Ok(opened)
}

struct YoutubeHttpSource {
    client: reqwest::Client,
    url: reqwest::Url,
    stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send + Sync>>,
    content_length: Option<u64>,
    last_error: Arc<Mutex<Option<String>>>,
}

/// Diagnostics only: how far the output has played, from the cursor the audio source advances.
#[cfg(feature = "diagnostics")]
fn played_seconds(cursor: &AtomicU64, rate: u32, channels: u16) -> f64 {
    cursor.load(Ordering::Relaxed) as f64 / (f64::from(rate) * f64::from(channels))
}

/// Googlevideo refusing an audio range with 403: the URL expired or was revoked, and a fresh
/// resolution fixes it. Other failures are reported as they were.
fn refused_by_googlevideo(error: &EngineError) -> bool {
    matches!(error, EngineError::Network(message) if *message == YoutubeHttpError::Status(403).to_string())
}

const MAX_HTTP_RANGE_BYTES: u64 = 1_048_576;

#[derive(Debug, Error)]
enum YoutubeHttpError {
    #[error("HTTP stream request failed: {0}")]
    Request(String),
    #[error("HTTP stream returned status {0}")]
    Status(u16),
    #[error("HTTP stream server did not honor the requested byte range")]
    RangeUnsupported,
}

impl DecodeError for YoutubeHttpError {}

impl YoutubeHttpSource {
    async fn request_range(&self, range: String) -> Result<reqwest::Response, YoutubeHttpError> {
        let query_keys: Vec<String> = self
            .url
            .query_pairs()
            .map(|(key, _)| key.into_owned())
            .collect();
        use sha2::{Digest, Sha256};
        let url_fingerprint = Sha256::digest(self.url.as_str().as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        tracing::debug!(range, host = self.url.host_str(), ?query_keys, %url_fingerprint, "requesting audio byte range");
        let response = self
            .client
            .get(self.url.clone())
            .header(reqwest::header::RANGE, range.clone())
            .send()
            .await
            .map_err(|error| {
                let message = error.without_url().to_string();
                record_fetch_error(&self.last_error, message.clone());
                YoutubeHttpError::Request(message)
            })?;
        tracing::debug!(status = %response.status(), range, "audio byte range response");
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let error_code = xml_error_code(&body);
            tracing::warn!(%status, range, ?error_code, "Googlevideo rejected the audio range");
            let error = if status.is_success() {
                YoutubeHttpError::RangeUnsupported
            } else {
                YoutubeHttpError::Status(status.as_u16())
            };
            record_fetch_error(&self.last_error, error.to_string());
            return Err(error);
        }
        clear_fetch_error(&self.last_error);
        Ok(response)
    }

    fn set_response(&mut self, response: reqwest::Response) {
        if self.content_length.is_none() {
            self.content_length = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.rsplit('/').next())
                .and_then(|value| value.parse().ok());
        }
        self.stream = Box::pin(response.bytes_stream());
    }
}

fn xml_error_code(body: &str) -> Option<String> {
    let value = body.split_once("<Code>")?.1.split_once("</Code>")?.0;
    (!value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'))
    .then(|| value.to_owned())
}

impl Stream for YoutubeHttpSource {
    type Item = Result<Bytes, reqwest::Error>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.get_mut().stream.as_mut().poll_next(cx)
    }
}

impl SourceStream for YoutubeHttpSource {
    type Params = (
        reqwest::Client,
        reqwest::Url,
        Option<u64>,
        Arc<Mutex<Option<String>>>,
    );
    type StreamCreationError = YoutubeHttpError;

    async fn create(
        (client, url, content_length, last_error): Self::Params,
    ) -> Result<Self, Self::StreamCreationError> {
        let mut source = Self {
            client,
            url,
            stream: Box::pin(futures_util::stream::empty()),
            content_length,
            last_error,
        };
        let end = source
            .content_length
            .map(|length| length.saturating_sub(1).min(MAX_HTTP_RANGE_BYTES - 1))
            .unwrap_or(MAX_HTTP_RANGE_BYTES - 1);
        let response = source.request_range(format!("bytes=0-{end}")).await?;
        source.set_response(response);
        Ok(source)
    }

    fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    async fn seek_range(&mut self, start: u64, end: Option<u64>) -> io::Result<()> {
        let stream_end = end
            .or(self.content_length)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unknown stream size"))?;
        let chunk_end = stream_end.min(start.saturating_add(MAX_HTTP_RANGE_BYTES));
        if chunk_end <= start {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid byte range",
            ));
        }
        let range = format!("bytes={start}-{}", chunk_end - 1);
        let response = self
            .request_range(range)
            .await
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.set_response(response);
        Ok(())
    }

    async fn reconnect(&mut self, current_position: u64) -> io::Result<()> {
        self.seek_range(current_position, None).await
    }

    fn supports_seek(&self) -> bool {
        self.content_length.is_some()
    }
}

/// The HTTP client for a media URL, with the headers the player response asked for.
fn media_client(stream: &AudioStream) -> Result<reqwest::Client, EngineError> {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in &stream.request_headers {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| EngineError::Network(error.to_string()))?;
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|error| EngineError::Network(error.to_string()))?;
        headers.insert(name, value);
    }
    crate::network::configure_http(reqwest::Client::builder())
        .connect_timeout(Duration::from_secs(6))
        // A per-read timeout: a total timeout cuts off slow links that are still delivering data.
        .read_timeout(Duration::from_secs(15))
        .default_headers(headers)
        .build()
        .map_err(|error| EngineError::Network(error.to_string()))
}

async fn open_stream(
    stream: AudioStream,
) -> Result<
    (
        StreamDownload<TempStorageProvider>,
        Arc<Mutex<Option<String>>>,
    ),
    EngineError,
> {
    let client = media_client(&stream)?;
    let url = reqwest::Url::parse(&stream.url)
        .map_err(|error| EngineError::Network(error.to_string()))?;
    let fetch_error = Arc::new(Mutex::new(None));
    let source =
        YoutubeHttpSource::create((client, url, stream.content_length, Arc::clone(&fetch_error)))
            .await
            .map_err(|error| EngineError::Network(error.to_string()))?;
    let reader = StreamDownload::from_stream(
        source,
        TempStorageProvider::new_in(stream_temp_dir()),
        Settings::default().prefetch_bytes(384 * 1024),
    )
    .await
    .map_err(|error| {
        EngineError::Network(fetch_error_message(&fetch_error).unwrap_or_else(|| error.to_string()))
    })?;
    Ok((reader, fetch_error))
}

/// Streamed audio is buffered in the cache directory rather than `/tmp`, which is RAM on many systems.
fn stream_temp_dir() -> std::path::PathBuf {
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("crabytube/stream");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub(super) fn record_fetch_error(target: &Mutex<Option<String>>, error: String) {
    if let Ok(mut value) = target.lock() {
        *value = Some(error);
    }
}

fn clear_fetch_error(target: &Mutex<Option<String>>) {
    if let Ok(mut value) = target.lock() {
        *value = None;
    }
}

fn fetch_error_message(source: &Mutex<Option<String>>) -> Option<String> {
    source.lock().ok().and_then(|value| value.clone())
}

/// What the decoder reads: a file of known or unknown length, or a live broadcast.
#[derive(Clone, Copy)]
enum MediaKind {
    Progressive { byte_len: Option<u64> },
    Live,
}

async fn create_audio_source<R>(
    reader: R,
    media: MediaKind,
    cancellation: CancellationToken,
    generation: u64,
    events: mpsc::UnboundedSender<Event>,
    fetch_error: Arc<Mutex<Option<String>>>,
    #[cfg_attr(not(feature = "diagnostics"), allow(unused_variables))] for_playback: bool,
) -> Result<PreparedAudio, EngineError>
where
    R: std::io::Read + std::io::Seek + Send + Sync + 'static,
{
    let (prepared_tx, prepared_rx) = tokio::sync::oneshot::channel();
    let live = matches!(media, MediaKind::Live);
    let byte_len = match media {
        MediaKind::Progressive { byte_len } => byte_len,
        MediaKind::Live => None,
    };
    tokio::task::spawn_blocking(move || {
        let builder = Decoder::builder()
            .with_data(reader)
            .with_mime_type("audio/mp4")
            .with_seekable(byte_len.is_some());
        let builder = match byte_len {
            Some(length) => builder.with_byte_len(length),
            None => builder,
        };
        let decoder = match builder.build() {
            Ok(decoder) => decoder,
            Err(error) => {
                if let Some(message) = fetch_error_message(&fetch_error) {
                    let _ = prepared_tx.send(Err(EngineError::Network(message)));
                } else {
                    let _ = prepared_tx.send(Err(EngineError::Decoder(error.to_string())));
                }
                return;
            }
        };
        let rate = decoder.sample_rate();
        let channels = decoder.channels();
        let rate_u32 = rate.get();
        let channels_u16 = channels.get();
        // About one second of audio in chunks, plus room for the preparation, which sends up to 12.
        let (samples_tx, samples_rx) = bounded::<Vec<f32>>(
            (rate_u32 as usize * channels_u16 as usize).clamp(24_000, 96_000) / AUDIO_CHUNK_SAMPLES
                + 2,
        );
        let pump_samples = samples_rx.clone();
        // The output signals each chunk it takes, so a decoder waiting for room wakes without polling.
        let (drained_tx, drained_rx) = bounded::<()>(1);
        let drained_keepalive = drained_tx.clone();
        // The decoder waits once about half a second of chunks is waiting for the output.
        let pump_threshold =
            (rate_u32 as usize * channels_u16 as usize / 2).min(48_000) / AUDIO_CHUNK_SAMPLES;
        let produced = Arc::new(AtomicU64::new(0));
        let (worker_tx, worker_rx) = unbounded::<WorkerCommand>();
        let cursor = Arc::new(AtomicU64::new(0));
        let first_sample_output = Arc::new(AtomicBool::new(false));
        let underruns = Arc::new(AtomicU64::new(0));
        let output = PreparedAudio {
            samples: samples_rx,
            produced: Arc::clone(&produced),
            cursor: Arc::clone(&cursor),
            rate,
            channels,
            worker: worker_tx,
            first_sample_output: Arc::clone(&first_sample_output),
            underruns: Arc::clone(&underruns),
            drained: drained_tx,
            live,
        };
        let mut decoder = decoder;
        let mut samples_sent = 0_usize;
        let mut chunk = Vec::with_capacity(AUDIO_CHUNK_SAMPLES);
        while samples_sent < (rate_u32 as usize * channels_u16 as usize / 3).min(24_000) {
            if cancellation.is_cancelled() {
                let _ = prepared_tx.send(Err(EngineError::Worker("playback was cancelled".into())));
                return;
            }
            match decoder.next() {
                Some(sample) => {
                    chunk.push(sample);
                    samples_sent += 1;
                    if chunk.len() == AUDIO_CHUNK_SAMPLES
                        && !hand_over(&samples_tx, &produced, std::mem::take(&mut chunk))
                    {
                        let _ = prepared_tx.send(Err(EngineError::Worker(
                            "playback source was dropped".into(),
                        )));
                        return;
                    }
                }
                None => break,
            }
        }
        if !chunk.is_empty() && !hand_over(&samples_tx, &produced, chunk) {
            let _ = prepared_tx.send(Err(EngineError::Worker(
                "playback source was dropped".into(),
            )));
            return;
        }
        let worker_cancellation = cancellation.clone();
        let worker_result = thread::Builder::new()
            .name("crabytube-decoder".into())
            .spawn(move || {
                info!("first audio frame decoded");
                // Holding a sender keeps the signal channel open for as long as this thread runs.
                let _drained_open = drained_keepalive;
                // A chunk that is decoded but not accepted by the channel yet, and whether the
                // decoder has reached the end of the stream.
                let mut pending: Option<Vec<f32>> = None;
                let mut ended = false;
                loop {
                    if worker_cancellation.is_cancelled() {
                        break;
                    }
                    // A full output parks the decoder until a command arrives or the output takes a chunk.
                    let waiting_for_output =
                        pending.is_none() && !ended && pump_samples.len() > pump_threshold;
                    let command = if waiting_for_output {
                        crossbeam_channel::select! {
                            recv(worker_rx) -> message => match message {
                                Ok(command) => Some(command),
                                Err(_) => break,
                            },
                            recv(drained_rx) -> _ => None,
                        }
                    } else {
                        match worker_rx.try_recv() {
                            Ok(command) => Some(command),
                            Err(crossbeam_channel::TryRecvError::Empty) => None,
                            Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                        }
                    };
                    match command {
                        Some(WorkerCommand::Stop) => break,
                        Some(WorkerCommand::Seek(target, reply)) => {
                            let result =
                                decoder.try_seek(target).map_err(|error| error.to_string());
                            if result.is_ok() {
                                // Chunks decoded before the seek belong to the old position.
                                pending = None;
                                ended = false;
                                while pump_samples.try_recv().is_ok() {}
                                let target_samples = (target.as_secs_f64()
                                    * f64::from(rate_u32)
                                    * f64::from(channels_u16))
                                    as u64;
                                cursor.store(target_samples, Ordering::Relaxed);
                                produced.store(target_samples, Ordering::Relaxed);
                            }
                            let _ = reply.send(result);
                        }
                        None => {}
                    }
                    if waiting_for_output {
                        continue;
                    }
                    if let Some(chunk) = pending.take() {
                        match samples_tx.send_timeout(chunk, Duration::from_millis(25)) {
                            Ok(()) => {}
                            Err(crossbeam_channel::SendTimeoutError::Timeout(chunk)) => {
                                pending = Some(chunk);
                            }
                            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => break,
                        }
                        continue;
                    }
                    if ended {
                        if let Some(message) = fetch_error_message(&fetch_error) {
                            let _ = events.send(Event::Failed {
                                generation,
                                error: EngineError::Network(message),
                            });
                        } else {
                            let _ = events.send(Event::TrackEnded { generation });
                        }
                        break;
                    }
                    #[cfg(feature = "diagnostics")]
                    if for_playback
                        && crate::playback::take_injected_playback_failure(played_seconds(
                            &cursor,
                            rate_u32,
                            channels_u16,
                        ))
                    {
                        tracing::warn!("diagnostic: refused the audio stream mid-track with 403");
                        let _ = events.send(Event::Failed {
                            generation,
                            error: EngineError::Network(YoutubeHttpError::Status(403).to_string()),
                        });
                        break;
                    }
                    let mut chunk = Vec::with_capacity(AUDIO_CHUNK_SAMPLES);
                    while chunk.len() < AUDIO_CHUNK_SAMPLES {
                        match decoder.next() {
                            Some(sample) => chunk.push(sample),
                            None => {
                                ended = true;
                                break;
                            }
                        }
                    }
                    if !chunk.is_empty() {
                        produced.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                        pending = Some(chunk);
                    }
                }
            });
        if let Err(error) = worker_result {
            let _ = prepared_tx.send(Err(EngineError::Worker(error.to_string())));
            return;
        }
        let _ = prepared_tx.send(Ok(output));
    })
    .await
    .map_err(|error| EngineError::Worker(error.to_string()))?;
    prepared_rx
        .await
        .map_err(|error| EngineError::Worker(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use std::{
        num::{NonZeroU16, NonZeroU32},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        time::Duration,
    };

    use crossbeam_channel::bounded;
    use futures_util::StreamExt;
    use tokio::sync::watch;

    use super::{
        AudioSource, EngineError, PlaybackState, PlaybackStatus, PlaybackStatusReceiver,
        ResolvedAudioFormat, engine_error_message, generation_matches, is_valid_transition,
        playback_status_stream, redact_urls, seek_ends_track,
    };

    #[tokio::test]
    async fn playback_status_stream_emits_state_changes_but_filters_position_ticks() {
        let initial = PlaybackStatus {
            state: PlaybackState::Playing,
            ..PlaybackStatus::default()
        };
        let (sender, receiver) = watch::channel(initial.clone());
        let receiver = PlaybackStatusReceiver { id: 1, receiver };
        let mut statuses = playback_status_stream(&receiver);

        assert_eq!(statuses.next().await.unwrap(), initial);

        let mut position_only = initial.clone();
        position_only.position = Duration::from_secs(1);
        sender.send(position_only.clone()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), statuses.next())
                .await
                .is_err()
        );

        position_only.state = PlaybackState::Paused;
        sender.send(position_only).unwrap();
        let observed = tokio::time::timeout(Duration::from_secs(1), statuses.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observed.state, PlaybackState::Paused);
        assert_eq!(observed.position, Duration::from_secs(1));

        let mut paused_seek = observed.clone();
        paused_seek.position = Duration::from_secs(2);
        sender.send(paused_seek).unwrap();
        let observed_seek = tokio::time::timeout(Duration::from_secs(1), statuses.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observed_seek.state, PlaybackState::Paused);
        assert_eq!(observed_seek.position, Duration::from_secs(2));
    }

    #[test]
    fn audio_source_counts_contiguous_silence_as_one_underrun_episode() {
        let (sample_tx, samples) = bounded(4);
        let cursor = Arc::new(AtomicU64::new(0));
        let first_sample_output = Arc::new(AtomicBool::new(false));
        let underruns = Arc::new(AtomicU64::new(0));
        sample_tx.send(vec![0.25]).unwrap();
        let mut source = AudioSource {
            samples,
            current: Vec::new(),
            index: 0,
            cursor: Arc::clone(&cursor),
            rate: NonZeroU32::new(48_000).unwrap(),
            channels: NonZeroU16::new(2).unwrap(),
            duration: None,
            first_sample_output: Arc::clone(&first_sample_output),
            underruns: Arc::clone(&underruns),
            drained: bounded::<()>(1).0,
            in_underrun: false,
        };

        assert_eq!(source.next(), Some(0.25));
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(underruns.load(Ordering::Relaxed), 1);

        sample_tx.send(vec![0.75]).unwrap();
        assert_eq!(source.next(), Some(0.75));
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(underruns.load(Ordering::Relaxed), 2);
        assert_eq!(cursor.load(Ordering::Relaxed), 2);
        assert!(first_sample_output.load(Ordering::Relaxed));
    }

    #[test]
    fn audio_source_plays_each_chunk_in_order_and_skips_empty_ones() {
        let (chunk_tx, samples) = bounded(4);
        let cursor = Arc::new(AtomicU64::new(0));
        let underruns = Arc::new(AtomicU64::new(0));
        chunk_tx.send(Vec::new()).unwrap();
        chunk_tx.send(vec![0.1, -0.2, 0.3]).unwrap();
        chunk_tx.send(vec![0.4]).unwrap();
        let mut source = AudioSource {
            samples,
            current: Vec::new(),
            index: 0,
            cursor: Arc::clone(&cursor),
            rate: NonZeroU32::new(48_000).unwrap(),
            channels: NonZeroU16::new(2).unwrap(),
            duration: None,
            first_sample_output: Arc::new(AtomicBool::new(false)),
            underruns: Arc::clone(&underruns),
            drained: bounded::<()>(1).0,
            in_underrun: false,
        };

        let played: Vec<f32> = (0..4).filter_map(|_| source.next()).collect();
        assert_eq!(played, vec![0.1, -0.2, 0.3, 0.4]);
        assert_eq!(cursor.load(Ordering::Relaxed), 4);
        assert_eq!(underruns.load(Ordering::Relaxed), 0);
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(underruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn resolved_audio_format_reports_actual_mime_bitrate_and_itag() {
        let format = ResolvedAudioFormat {
            mime_type: String::from("audio/mp4; codecs=\"mp4a.40.2\""),
            bitrate: Some(128_000),
            itag: Some(140),
        };
        assert_eq!(
            format.label(),
            "AAC · 128 kbps · audio/mp4; codecs=\"mp4a.40.2\" · itag 140"
        );
    }

    #[test]
    fn playback_state_machine_accepts_the_real_start_pause_resume_stop_path() {
        let states = [
            PlaybackState::Idle,
            PlaybackState::Resolving,
            PlaybackState::Buffering,
            PlaybackState::Playing,
            PlaybackState::Seeking,
            PlaybackState::Paused,
            PlaybackState::Playing,
            PlaybackState::Finished,
        ];
        assert!(
            states
                .windows(2)
                .all(|pair| is_valid_transition(pair[0], pair[1]))
        );
        assert_eq!(PlaybackStatus::default().state, PlaybackState::Idle);
    }

    #[test]
    fn idle_cannot_jump_directly_to_playing() {
        assert!(!is_valid_transition(
            PlaybackState::Idle,
            PlaybackState::Playing
        ));
    }

    #[test]
    fn resolving_pause_and_decoder_eof_have_explicit_transitions() {
        assert!(is_valid_transition(
            PlaybackState::Resolving,
            PlaybackState::Paused
        ));
        assert!(is_valid_transition(
            PlaybackState::Paused,
            PlaybackState::Buffering
        ));
        assert!(is_valid_transition(
            PlaybackState::Playing,
            PlaybackState::Finished
        ));
        assert!(is_valid_transition(
            PlaybackState::Finished,
            PlaybackState::Stopped
        ));
        assert!(!is_valid_transition(
            PlaybackState::Idle,
            PlaybackState::Finished
        ));
    }

    #[test]
    fn stale_resolution_generations_are_rejected() {
        assert!(generation_matches(42, 42));
        assert!(!generation_matches(43, 42));
    }

    #[test]
    fn a_seek_to_or_past_the_end_finishes_the_track() {
        let length = Duration::from_secs(200);
        assert!(seek_ends_track(length, Some(length)));
        assert!(seek_ends_track(Duration::from_secs(260), Some(length)));
        assert!(!seek_ends_track(Duration::from_secs(199), Some(length)));
        // Without a known length the seek is sent to the decoder as before.
        assert!(!seek_ends_track(Duration::from_secs(260), None));
    }

    #[test]
    fn media_urls_are_redacted_from_error_text() {
        assert_eq!(redact_urls("plain failure"), "plain failure");
        assert_eq!(
            redact_urls(
                r#"Network("https://rr1.example.test/videoplayback?sig=abc") after 3 tries"#
            ),
            r#"Network("<url>") after 3 tries"#
        );
        assert_eq!(
            redact_urls("error for url (http://example.test/x), retry"),
            "error for url (<url>), retry"
        );
        assert_eq!(redact_urls("the http header"), "the http header");
    }

    #[test]
    fn range_network_failure_is_distinct_from_decoder_failure() {
        assert!(engine_error_message(&EngineError::Network("HTTP 403".into())).contains("cut off"));
        assert!(
            engine_error_message(&EngineError::Decoder("invalid AAC".into())).contains("decoded")
        );
        assert_ne!(
            engine_error_message(&EngineError::Network(String::new())),
            engine_error_message(&EngineError::Decoder(String::new()))
        );
        assert!(is_valid_transition(
            PlaybackState::Playing,
            PlaybackState::Error
        ));
    }

    /// Serves one canned HTTP response per incoming connection, in order.
    fn serve_responses(responses: Vec<&'static str>) -> u16 {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let port = listener.local_addr().expect("local addr").port();
        std::thread::spawn(move || {
            for response in responses {
                let (mut socket, _) = listener.accept().expect("accept");
                // Read the whole request so the close does not reset the client.
                let mut request = Vec::new();
                let mut chunk = [0_u8; 512];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match socket.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&chunk[..read]),
                    }
                }
                socket
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });
        port
    }

    #[tokio::test]
    async fn fetch_error_clears_after_a_successful_range_request() {
        use super::{YoutubeHttpSource, fetch_error_message};
        use std::sync::{Arc, Mutex};

        let port = serve_responses(vec![
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 0-3/4\r\nConnection: close\r\n\r\nABCD",
        ]);
        let last_error = Arc::new(Mutex::new(None));
        let source = YoutubeHttpSource {
            client: reqwest::Client::new(),
            url: reqwest::Url::parse(&format!("http://127.0.0.1:{port}/audio")).unwrap(),
            stream: Box::pin(futures_util::stream::empty()),
            content_length: None,
            last_error: Arc::clone(&last_error),
        };

        assert!(source.request_range("bytes=0-3".into()).await.is_err());
        assert!(fetch_error_message(&last_error).is_some());

        assert!(source.request_range("bytes=0-3".into()).await.is_ok());
        assert_eq!(fetch_error_message(&last_error), None);
    }

    #[test]
    fn only_a_googlevideo_403_is_recovered_by_resolving_again() {
        let refused = EngineError::Network(super::YoutubeHttpError::Status(403).to_string());
        assert!(super::refused_by_googlevideo(&refused));
        let other_status = EngineError::Network(super::YoutubeHttpError::Status(404).to_string());
        assert!(!super::refused_by_googlevideo(&other_status));
        assert!(!super::refused_by_googlevideo(&EngineError::Decoder(
            "HTTP stream returned status 403".into()
        )));
    }

    #[tokio::test]
    async fn each_pause_schedules_a_release_tagged_with_its_own_serial() {
        let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let mut pause_serial = 0_u64;
        super::schedule_output_release(&mut pause_serial, &events_tx, Duration::from_millis(5));
        super::schedule_output_release(&mut pause_serial, &events_tx, Duration::from_millis(5));
        assert_eq!(pause_serial, 2);
        let mut released = Vec::new();
        for _ in 0..2 {
            match events.recv().await {
                Some(super::Event::OutputIdle { serial }) => released.push(serial),
                _ => panic!("expected a scheduled output release"),
            }
        }
        released.sort_unstable();
        assert_eq!(released, vec![1, 2]);
    }

    #[test]
    fn a_seek_that_was_paused_lands_paused_or_buffering() {
        use super::PlaybackState::{Buffering, Paused, Seeking};
        assert!(super::is_valid_transition(Seeking, Paused));
        assert!(super::is_valid_transition(Seeking, Buffering));
    }

    #[test]
    fn a_play_uses_the_prepared_stream_or_waits_for_the_prefetch_of_its_track() {
        use super::PlayPlan::{Awaited, Prepared, Resolve};
        assert_eq!(super::play_plan(Some("a"), None, "a"), Prepared);
        assert_eq!(super::play_plan(None, Some("a"), "a"), Awaited);
        assert_eq!(super::play_plan(Some("b"), Some("a"), "a"), Awaited);
        assert_eq!(super::play_plan(Some("a"), None, "b"), Resolve);
        assert_eq!(super::play_plan(None, None, "a"), Resolve);
    }

    #[test]
    fn taking_a_chunk_signals_the_decoder_that_waits_for_room() {
        let (sample_tx, samples) = bounded(4);
        let (drained, signalled) = bounded::<()>(1);
        sample_tx.send(vec![0.5, 0.5]).unwrap();
        let mut source = AudioSource {
            samples,
            drained,
            current: Vec::new(),
            index: 0,
            cursor: Arc::new(AtomicU64::new(0)),
            rate: NonZeroU32::new(48_000).unwrap(),
            channels: NonZeroU16::new(2).unwrap(),
            duration: None,
            first_sample_output: Arc::new(AtomicBool::new(false)),
            underruns: Arc::new(AtomicU64::new(0)),
            in_underrun: false,
        };
        // Nothing is taken yet, so nothing wakes the decoder.
        assert!(signalled.try_recv().is_err());
        assert_eq!(source.next(), Some(0.5));
        assert!(signalled.try_recv().is_ok());
    }

    #[test]
    fn decoder_events_of_a_stream_that_is_not_playing_yet_are_held_for_it() {
        let prefetching = Some(super::Prefetching {
            video_id: "a".into(),
            generation: 7,
            cancellation: tokio_util::sync::CancellationToken::new(),
            decoder_events: Vec::new(),
        });
        let prepared: Option<super::PreparedAhead> = None;
        assert!(super::decoder_event_held(
            &prefetching,
            &prepared,
            &super::Event::TrackEnded { generation: 7 }
        ));
        assert!(super::decoder_event_held(
            &prefetching,
            &prepared,
            &super::Event::Failed {
                generation: 7,
                error: EngineError::Decoder("stream ended early".into()),
            }
        ));
        assert!(!super::decoder_event_held(
            &prefetching,
            &prepared,
            &super::Event::TrackEnded { generation: 8 }
        ));
        assert!(!super::decoder_event_held(
            &None,
            &prepared,
            &super::Event::TrackEnded { generation: 7 }
        ));
    }
}
