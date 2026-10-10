mod engine;
mod live;
pub mod output;
pub(crate) mod pot;
mod resolver;

pub use engine::{
    PlaybackHandle, PlaybackState, PlaybackStatus, PlaybackStatusReceiver, ResolvedAudioFormat,
    playback_status_stream,
};
pub use pot::{
    BotguardHelper, PoToken, PoTokenCache, PoTokenError, PoTokenProvider, PoTokenRequest, PoTokens,
    set_upcoming_video_ids,
};
pub use resolver::{
    AudioCodec, AudioQuality, AudioStream, AudioStreamDiagnostics, ChallengeCapture, ClientProfile,
    GvsPoBinding, MAX_PERSISTED_PLAYER_JS_ENTRIES, MAX_PERSISTED_PREPROCESSED_BYTES, StreamError,
    StreamResolver, TrackId, ValidatedPlaybackResolution, YouTubeStreamResolver,
};

pub(crate) use resolver::parse_ytcfg;

#[cfg(feature = "diagnostics")]
static RANGE_FAILURE_INJECTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// `CRABYTUBE_FAIL_RANGE_AT=<seconds>` (diagnostics only): once the output has played that many
/// seconds, the playing stream fails once with HTTP 403, to exercise the recovery from an expired
/// URL. At `0` the first GVS preflight answers 403 instead, which exercises the retry after refused
/// saved tokens. The downloader fetches the whole stream eagerly and reconnects on its own, so the
/// refusal is injected where playback sees it rather than in the HTTP range request.
#[cfg(feature = "diagnostics")]
pub(crate) fn range_failure_at_seconds() -> Option<f64> {
    std::env::var("CRABYTUBE_FAIL_RANGE_AT").ok()?.parse().ok()
}

/// Consumes the one injected refusal. It fires once per process, so the recovery can succeed.
#[cfg(feature = "diagnostics")]
pub(crate) fn take_injected_range_failure() -> bool {
    !RANGE_FAILURE_INJECTED.swap(true, std::sync::atomic::Ordering::SeqCst)
}

/// Whether the mid-track refusal is due: the output has played `CRABYTUBE_FAIL_RANGE_AT` seconds.
#[cfg(feature = "diagnostics")]
pub(crate) fn take_injected_playback_failure(played_seconds: f64) -> bool {
    range_failure_at_seconds().is_some_and(|at| played_seconds >= at)
        && take_injected_range_failure()
}
