use std::{
    fmt,
    num::NonZero,
    str::FromStr,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, FromSample, OutputCallbackInfo, OutputStreamTimestamp, SampleFormat, SizedSample,
    StreamConfig,
};
use rodio::mixer::Mixer;
use tracing::{debug, warn};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputDeviceChoice {
    pub id: Option<String>,
    pub label: String,
}

impl fmt::Display for OutputDeviceChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.id.is_none() {
            f.write_str(
                crate::i18n::translate(crate::i18n::current_language(), &self.label).as_ref(),
            )
        } else {
            f.write_str(&self.label)
        }
    }
}

pub fn enumerate_output_devices() -> Result<Vec<OutputDeviceChoice>, String> {
    without_panics(list_output_devices)
}

/// Runs a device query of cpal, which on Windows panics when the audio service cannot create its
/// device enumerator, as when the remote desktop session that carried the audio has just ended.
/// That is reported as an error like any other missing device.
fn without_panics<T>(query: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(query))
        .unwrap_or_else(|_| Err(String::from("The audio system is not available")))
}

fn list_output_devices() -> Result<Vec<OutputDeviceChoice>, String> {
    let host = cpal::default_host();
    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let mut choices = vec![OutputDeviceChoice {
        id: None,
        label: String::from("System default"),
    }];
    let devices = host.output_devices().map_err(|error| error.to_string())?;
    for device in devices {
        let Ok(id) = device.id() else {
            continue;
        };
        let id = id.to_string();
        if default_id.as_deref() == Some(&id) {
            continue;
        }
        let label = device
            .description()
            .map(|description| description.name().to_owned())
            .unwrap_or_else(|_| String::from("Audio output"));
        choices.push(OutputDeviceChoice {
            id: Some(id),
            label,
        });
    }
    choices[1..].sort_by(|left, right| {
        left.label
            .to_lowercase()
            .cmp(&right.label.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(choices)
}

pub fn open_output_device(selected_id: Option<&str>) -> Result<(OutputStream, String), String> {
    without_panics(|| open_device(selected_id))
}

fn open_device(selected_id: Option<&str>) -> Result<(OutputStream, String), String> {
    let host = cpal::default_host();
    let device = match selected_id {
        Some(id) => {
            let id = cpal::DeviceId::from_str(id).map_err(|error| error.to_string())?;
            host.device_by_id(&id)
                .ok_or_else(|| "The selected output device is no longer available".to_owned())?
        }
        None => host
            .default_output_device()
            .ok_or_else(|| "No system default output device is available".to_owned())?,
    };
    let label = device
        .description()
        .map(|description| description.name().to_owned())
        .unwrap_or_else(|_| String::from("Audio output"));
    let stream = open_stream(&device)?;
    Ok((stream, label))
}

/// The largest device latency taken into account. A longer reading is treated as unreliable.
const MAX_DEVICE_LATENCY: Duration = Duration::from_secs(1);

/// An open output: a cpal stream that plays the mixer the player feeds, and reports when each of its
/// buffers is heard. rodio's own device sink keeps that timing to itself.
pub struct OutputStream {
    _stream: cpal::Stream,
    mixer: Mixer,
    clock: Arc<PlaybackClock>,
    /// Set by the stream when its device went away; the stream plays nothing after that.
    lost: Arc<AtomicBool>,
}

impl OutputStream {
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    pub fn clock(&self) -> &PlaybackClock {
        &self.clock
    }

    /// Whether the device went away (unplugged, or the remote session that carried it ended), so
    /// the output has to be opened again.
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }

    /// Diagnostics only: reports the device gone, as its stream does when it is unplugged.
    #[cfg(feature = "diagnostics")]
    pub fn mark_lost(&self) {
        self.lost.store(true, Ordering::Relaxed);
    }
}

/// Backend errors in a row, with no request for audio between them, that mean the device is gone.
const FAILURES_FOR_A_LOST_DEVICE: u32 = 50;

/// Whether a stream error leaves the stream dead: the device is gone, or the stream has to be
/// built again. Underruns and backend hiccups do not stop it.
fn ends_the_stream(error: &cpal::StreamError) -> bool {
    matches!(
        error,
        cpal::StreamError::DeviceNotAvailable | cpal::StreamError::StreamInvalidated
    )
}

/// When the device last finished playing a buffer, from cpal's timestamps.
#[derive(Default)]
pub struct PlaybackClock {
    /// Nanoseconds after `clock_base()` at which the last buffer finishes playing. Zero before any buffer.
    buffer_ends: AtomicU64,
}

/// The instant that clock readings are measured from, so the audio callback can store one in an atomic.
fn clock_base() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

impl PlaybackClock {
    fn record_buffer_end(&self, end: Instant) {
        let nanos = u64::try_from(end.saturating_duration_since(clock_base()).as_nanos())
            .unwrap_or(u64::MAX);
        // Zero means no buffer yet, so an end exactly at the base is stored one nanosecond later.
        self.buffer_ends.store(nanos.max(1), Ordering::Relaxed);
    }

    /// The position the listener hears at `now`. The pulled position runs ahead of what is audible by
    /// the samples the device still has to play: those before the end of its last buffer.
    pub fn heard_position(
        &self,
        pulled_samples: u64,
        now: Instant,
        rate: u32,
        channels: u16,
    ) -> Duration {
        let samples_per_second = f64::from(rate.max(1)) * f64::from(channels.max(1));
        let in_flight = match self.buffer_ends.load(Ordering::Relaxed) {
            0 => 0.0,
            nanos => clock_base()
                .checked_add(Duration::from_nanos(nanos))
                .map_or(0.0, |end| {
                    end.saturating_duration_since(now).as_secs_f64() * samples_per_second
                }),
        };
        Duration::from_secs_f64((pulled_samples as f64 - in_flight).max(0.0) / samples_per_second)
    }
}

/// The latency from the audio callback to the moment the device plays its first sample of that buffer.
fn device_latency(timing: &OutputStreamTimestamp) -> Duration {
    timing
        .playback
        .duration_since(&timing.callback)
        .unwrap_or(Duration::ZERO)
        .min(MAX_DEVICE_LATENCY)
}

/// Fills one device buffer with the mixer's samples, and silence where the mixer has none.
fn fill<T: SizedSample + FromSample<f32>>(mixer: &mut impl Iterator<Item = f32>, out: &mut [T]) {
    for sample in out {
        *sample = mixer.next().map(T::from_sample).unwrap_or(T::EQUILIBRIUM);
    }
}

/// Opens the device's default format. It asks for a 4096-frame buffer first and falls back to the
/// device's own buffer when that size is refused, as rodio's device sink did.
fn open_stream(device: &cpal::Device) -> Result<OutputStream, String> {
    let supported = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    let mut last_error = String::new();
    for buffer_size in [BufferSize::Fixed(4096), BufferSize::Default] {
        let config = StreamConfig {
            channels: supported.channels(),
            sample_rate: supported.sample_rate(),
            buffer_size,
        };
        match build_stream(device, supported.sample_format(), &config) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn build_stream(
    device: &cpal::Device,
    format: SampleFormat,
    config: &StreamConfig,
) -> Result<OutputStream, String> {
    match format {
        SampleFormat::F32 => build_typed::<f32>(device, config),
        SampleFormat::I16 => build_typed::<i16>(device, config),
        SampleFormat::U16 => build_typed::<u16>(device, config),
        SampleFormat::I32 => build_typed::<i32>(device, config),
        SampleFormat::U32 => build_typed::<u32>(device, config),
        other => Err(format!("unsupported sample format {other:?}")),
    }
}

fn build_typed<T>(device: &cpal::Device, config: &StreamConfig) -> Result<OutputStream, String>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let _ = clock_base();
    let channels = config.channels;
    let rate = config.sample_rate;
    let (mixer, mut source) = rodio::mixer::mixer(
        NonZero::new(channels).ok_or("the device reports no channels")?,
        NonZero::new(rate).ok_or("the device reports no sample rate")?,
    );
    let clock = Arc::new(PlaybackClock::default());
    let callback_clock = Arc::clone(&clock);
    let lost = Arc::new(AtomicBool::new(false));
    let error_lost = Arc::clone(&lost);
    // Backend errors since the device last asked for audio. ALSA reports an unplugged device only
    // as such errors, again and again, never as a device that went away.
    let failures = Arc::new(AtomicU32::new(0));
    let data_failures = Arc::clone(&failures);
    let stream = device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], info: &OutputCallbackInfo| {
                let now = Instant::now();
                data_failures.store(0, Ordering::Relaxed);
                let latency = device_latency(&info.timestamp());
                fill(&mut source, data);
                let frames = data.len() / usize::from(channels.max(1));
                let buffer = Duration::from_secs_f64(frames as f64 / f64::from(rate));
                callback_clock.record_buffer_end(now + latency + buffer);
            },
            move |error| {
                let in_a_row = if matches!(error, cpal::StreamError::BackendSpecific { .. }) {
                    failures.fetch_add(1, Ordering::Relaxed) + 1
                } else {
                    0
                };
                if ends_the_stream(&error) || in_a_row == FAILURES_FOR_A_LOST_DEVICE {
                    warn!(%error, "audio output failed");
                    error_lost.store(true, Ordering::Relaxed);
                } else if in_a_row < FAILURES_FOR_A_LOST_DEVICE {
                    debug!(%error, "audio output hiccup");
                }
            },
            None,
        )
        .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    Ok(OutputStream {
        _stream: stream,
        mixer,
        clock,
        lost,
    })
}

#[cfg(test)]
mod tests {
    use super::{OutputDeviceChoice, enumerate_output_devices, open_output_device};
    use std::time::Duration;

    #[test]
    fn a_panic_in_a_device_query_is_reported_as_an_error() {
        let answer: Result<(), String> = super::without_panics(|| panic!("Class not registered"));
        assert_eq!(
            answer,
            Err(String::from("The audio system is not available"))
        );
        assert_eq!(super::without_panics(|| Ok(7)), Ok(7));
    }

    #[test]
    fn only_a_device_that_went_away_ends_the_stream() {
        use super::ends_the_stream;
        use cpal::{BackendSpecificError, StreamError};
        assert!(ends_the_stream(&StreamError::DeviceNotAvailable));
        assert!(ends_the_stream(&StreamError::StreamInvalidated));
        assert!(!ends_the_stream(&StreamError::BufferUnderrun));
        assert!(!ends_the_stream(&StreamError::BackendSpecific {
            err: BackendSpecificError {
                description: String::from("poll failed"),
            },
        }));
    }

    #[test]
    fn default_output_choice_has_a_stable_setting_label() {
        let default = OutputDeviceChoice {
            id: None,
            label: String::from("System default"),
        };
        assert_eq!(default.to_string(), "System default");
        assert_eq!(default.id, None);
    }

    #[test]
    fn device_enumeration_returns_a_default_choice_or_a_clear_backend_error() {
        match enumerate_output_devices() {
            Ok(choices) => {
                assert_eq!(
                    choices.first().and_then(|choice| choice.id.as_deref()),
                    None
                );
                assert_eq!(
                    choices.first().map(|choice| choice.label.as_str()),
                    Some("System default")
                );
                assert!(choices.iter().skip(1).all(|choice| choice.id.is_some()));
            }
            Err(error) => assert!(!error.is_empty()),
        }
    }

    #[test]
    fn malformed_output_device_ids_are_rejected_before_opening_a_device() {
        assert!(open_output_device(Some("not-a-valid-device-id")).is_err());
    }

    fn about(seconds: f64, expected: Duration) -> bool {
        (seconds - expected.as_secs_f64()).abs() < 1e-6
    }

    #[test]
    fn the_heard_position_leaves_out_what_the_device_still_has_to_play() {
        let clock = super::PlaybackClock::default();
        // Before any buffer, the pulled position is all there is: 1000 samples at 1 kHz stereo.
        let before_any = super::clock_base();
        assert!(about(
            clock
                .heard_position(1_000, before_any, 1_000, 2)
                .as_secs_f64(),
            Duration::from_millis(500)
        ));
        // The last buffer ends 50 ms from now, and 100 of the 1000 samples are still to play.
        let now = super::clock_base() + Duration::from_secs(1);
        clock.record_buffer_end(now + Duration::from_millis(50));
        assert!(about(
            clock.heard_position(1_000, now, 1_000, 2).as_secs_f64(),
            Duration::from_millis(450)
        ));
        // Once that buffer has played out, the pulled position is what is heard.
        assert!(about(
            clock
                .heard_position(1_000, now + Duration::from_millis(100), 1_000, 2)
                .as_secs_f64(),
            Duration::from_millis(500)
        ));
        // A device that reports a huge lag never takes the position below zero.
        clock.record_buffer_end(now + Duration::from_secs(10));
        assert_eq!(clock.heard_position(1_000, now, 1_000, 2), Duration::ZERO);
    }

    #[test]
    fn device_latency_is_the_gap_from_callback_to_playback_within_one_second() {
        use cpal::{OutputStreamTimestamp, StreamInstant};
        let callback = StreamInstant::new(10, 0);
        let timing = |playback: StreamInstant| OutputStreamTimestamp { callback, playback };
        assert_eq!(
            super::device_latency(&timing(StreamInstant::new(10, 30_000_000))),
            Duration::from_millis(30)
        );
        // A playback time before the callback adds no latency.
        assert_eq!(
            super::device_latency(&timing(StreamInstant::new(9, 0))),
            Duration::ZERO
        );
        assert_eq!(
            super::device_latency(&timing(StreamInstant::new(60, 0))),
            super::MAX_DEVICE_LATENCY
        );
    }

    #[test]
    fn a_device_buffer_takes_the_mixer_then_silence_in_its_own_format() {
        let mut mixer = [0.5_f32, -0.5].into_iter();
        let mut out = [0.0_f32; 4];
        super::fill(&mut mixer, &mut out);
        assert_eq!(out, [0.5, -0.5, 0.0, 0.0]);

        let mut one_sample = [1.0_f32].into_iter();
        let mut converted = [0_i16; 2];
        super::fill(&mut one_sample, &mut converted);
        assert!(converted[0] > 32_000);
        assert_eq!(converted[1], 0);
    }
}
