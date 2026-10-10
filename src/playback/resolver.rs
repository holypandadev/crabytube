use std::{
    collections::HashMap,
    ops::Deref,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{BotguardHelper, PoTokenError, PoTokenProvider, PoTokenRequest};

const MWEB_UA: &str = "Mozilla/5.0 (iPad; CPU OS 16_7_10 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1,gzip(gfe)";
const BOTGUARD_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36(KHTML, like Gecko)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GvsPoBinding {
    VisitorData,
    VideoId,
}

impl GvsPoBinding {
    pub fn label(self) -> &'static str {
        match self {
            Self::VisitorData => "visitor_data",
            Self::VideoId => "video_id",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientProfile {
    Mweb,
    WebMusic,
}

#[derive(Clone, Copy)]
struct PlayerClient {
    name: &'static str,
    version: &'static str,
    user_agent: &'static str,
    config_url: &'static str,
    api_host: &'static str,
    requires_gvs_po_token: bool,
}

/// A player page older than this is fetched again, so a stale challenge or cipher cannot stay for days.
const PLAYER_RUNTIME_TTL: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// Whether a cached player runtime, fetched at `fetched_at`, may still be used at `now`.
fn runtime_is_fresh(fetched_at: std::time::Instant, now: std::time::Instant) -> bool {
    now.saturating_duration_since(fetched_at) < PLAYER_RUNTIME_TTL
}

#[derive(Clone)]
struct PlayerRuntime {
    fetched_at: std::time::Instant,
    profile: PlayerClient,
    endpoint: String,
    origin: String,
    header_id: String,
    version: String,
    user_agent: Option<String>,
    signature_timestamp: Option<u64>,
    player_js_url: Option<String>,
    context: Value,
    botguard_ytcfg_json: String,
    botguard_user_agent: String,
    homepage_challenge_json: Option<String>,
    event_id: Option<String>,
    gvs_binding: GvsPoBinding,
}

impl ClientProfile {
    fn config(self) -> PlayerClient {
        match self {
            Self::Mweb => PlayerClient {
                name: "MWEB",
                version: "2.20260708.05.00",
                user_agent: MWEB_UA,
                config_url: "https://m.youtube.com",
                api_host: "www.youtube.com",
                requires_gvs_po_token: true,
            },
            Self::WebMusic => PlayerClient {
                name: "WEB_REMIX",
                version: "1.20260707.12.00",
                user_agent: "",
                config_url: "https://music.youtube.com",
                api_host: "music.youtube.com",
                requires_gvs_po_token: true,
            },
        }
    }
}

impl PlayerClient {
    fn fallback_client_id(self) -> &'static str {
        match self.name {
            "MWEB" => "2",
            "WEB_REMIX" => "67",
            _ => "0",
        }
    }
}
use crate::music::{Track, YouTubeSession};
use async_trait::async_trait;
use reqwest::{Client, Url};
use serde_json::{Value, json};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{info, instrument, warn};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackId(pub String);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioQuality {
    #[default]
    Auto,
    Low,
    Normal,
    High,
    Best,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCodec {
    Aac,
}

/// The widest thumbnail the player response lists for the video.
fn largest_thumbnail(payload: &Value) -> Option<String> {
    payload
        .pointer("/videoDetails/thumbnail/thumbnails")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|thumbnail| {
            let url = thumbnail.get("url").and_then(Value::as_str)?;
            let width = thumbnail.get("width").and_then(Value::as_u64).unwrap_or(0);
            Some((width, url.to_owned()))
        })
        .max_by_key(|(width, _)| *width)
        .map(|(_, url)| url)
}

#[derive(Clone, PartialEq, Eq)]
pub struct AudioStream {
    pub diagnostics: Box<AudioStreamDiagnostics>,
    pub url: String,
    pub mime_type: String,
    pub codec: AudioCodec,
    pub bitrate: Option<u32>,
    pub content_length: Option<u64>,
    pub duration: Option<Duration>,
    /// True for a live broadcast: the media is a fragment stream that never ends.
    pub live: bool,
    pub title: String,
    /// Channel name from `videoDetails`, used when the caller only knows the video ID.
    pub author: Option<String>,
    /// Largest thumbnail from `videoDetails`.
    pub thumbnail_url: Option<String>,
    pub itag: Option<u32>,
    pub adaptive_format_count: usize,
    pub audio_format_count: usize,
    pub signature_cipher: bool,
    pub signature_deciphered: bool,
    pub n_present: bool,
    pub n_transformed: bool,
    pub request_headers: Vec<(String, String)>,
    /// Ephemeral raw challenge inputs for the local challenge-probe only.
    /// Never serialize or log this object; its Debug output is redacted.
    pub challenge_capture: Option<Box<ChallengeCapture>>,
}

/// A media URL may cross into the playback engine only after GVS accepted its
/// minimal byte-range preflight. The wrapper also records its short-lived URL
/// lifetime so callers can reject stale resolutions before opening them.
#[derive(Clone, PartialEq, Eq)]
pub struct ValidatedPlaybackResolution {
    stream: AudioStream,
    pub created_at: SystemTime,
    pub expires_at: Option<SystemTime>,
}

impl std::fmt::Debug for ValidatedPlaybackResolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedPlaybackResolution")
            .field("resolution_id", &self.stream.diagnostics.resolution_id)
            .field("url_fp", &self.stream.diagnostics.url_fp)
            .field(
                "preflight_status",
                &self.stream.diagnostics.preflight_status,
            )
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("title", &self.stream.title)
            .finish_non_exhaustive()
    }
}

impl AudioStream {
    /// Fills what the caller left out (usually only the video ID is known) from the
    /// player response. Metadata the caller already has is kept.
    pub fn fill_track(&self, track: &mut Track) {
        let title_missing = track.title.trim().is_empty() || track.title == track.video_id;
        if title_missing && self.title != "Unknown track" {
            track.title = self.title.clone();
        }
        if track.artist.trim().is_empty()
            && let Some(author) = &self.author
        {
            track.artist = author.clone();
        }
        if track.thumbnail_url.is_none() {
            track.thumbnail_url = self.thumbnail_url.clone();
        }
        if track.duration_secs.is_none() {
            track.duration_secs = self
                .duration
                .and_then(|duration| u32::try_from(duration.as_secs()).ok());
        }
    }
}

impl ValidatedPlaybackResolution {
    fn after_preflight(stream: AudioStream) -> Self {
        let created_at = SystemTime::now();
        let expires_at = stream
            .diagnostics
            .expires_at_unix
            .and_then(|seconds| u64::try_from(seconds).ok())
            .map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds));
        Self {
            stream,
            created_at,
            expires_at,
        }
    }

    /// Diagnostic-only resolvers may construct a validated value from a
    /// stream whose preflight was already performed by the probe.
    #[doc(hidden)]
    pub fn from_preflight(stream: AudioStream) -> Result<Self, StreamError> {
        if !matches!(stream.diagnostics.preflight_status, 200 | 206) {
            return Err(StreamError::MediaRejected(format!(
                "preflight status {} is not accepted",
                stream.diagnostics.preflight_status
            )));
        }
        Ok(Self::after_preflight(stream))
    }

    pub fn is_expired(&self, now: SystemTime) -> bool {
        self.expires_at
            .is_some_and(|expiry| expiry <= now + Duration::from_secs(30))
    }

    pub fn into_stream(self) -> AudioStream {
        self.stream
    }
}

impl Deref for ValidatedPlaybackResolution {
    type Target = AudioStream;
    fn deref(&self) -> &Self::Target {
        &self.stream
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ChallengeCapture {
    pub player_js_url: String,
    pub player_js: String,
    pub preprocessed_player: String,
    pub raw_signature_cipher: Option<String>,
    pub raw_signature: Option<String>,
    pub signature_parameter: String,
    pub raw_url: String,
    pub raw_n: Option<String>,
    pub native_signature: Option<String>,
    pub native_n: Option<String>,
}

impl std::fmt::Debug for ChallengeCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChallengeCapture")
            .field(
                "player_js_url_fp",
                &fingerprint8(self.player_js_url.as_bytes()),
            )
            .field("player_js_fp", &fingerprint8(self.player_js.as_bytes()))
            .field(
                "raw_signature_cipher_fp",
                &self
                    .raw_signature_cipher
                    .as_deref()
                    .map(|v| fingerprint8(v.as_bytes())),
            )
            .field(
                "raw_signature_fp",
                &self
                    .raw_signature
                    .as_deref()
                    .map(|v| fingerprint8(v.as_bytes())),
            )
            .field("raw_url_fp", &fingerprint8(self.raw_url.as_bytes()))
            .field(
                "raw_n_fp",
                &self.raw_n.as_deref().map(|v| fingerprint8(v.as_bytes())),
            )
            .field(
                "native_signature_fp",
                &self
                    .native_signature
                    .as_deref()
                    .map(|v| fingerprint8(v.as_bytes())),
            )
            .field(
                "native_n_fp",
                &self.native_n.as_deref().map(|v| fingerprint8(v.as_bytes())),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioStreamDiagnostics {
    pub resolution_id: u64,
    pub validation_attempt: u8,
    pub preflight_status: u16,
    pub player_response_fp: String,
    pub player_js_fp: String,
    pub pot_fp: Option<String>,
    pub gvs_binding: GvsPoBinding,
    pub url_fp: String,
    pub expires_at_unix: Option<i64>,
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("temporary network failure: {0}")]
    Network(String),
    #[error("YouTube client configuration failed: {0}")]
    Config(String),
    #[error("YouTube cannot play this track: {0}")]
    PlayerUnavailable(String),
    #[error("this track requires an authenticated session: {0}")]
    AuthenticationRequired(String),
    #[error("YouTube integrity token generation failed: {0}")]
    PoToken(String),
    #[error("YouTube player challenge could not be solved: {0}")]
    Cipher(String),
    #[error("YouTube rejected the media URL: {0}")]
    MediaRejected(String),
    #[error("YouTube returned no supported audio format")]
    UnsupportedFormat,
    #[error("stream resolution failed: {0}")]
    Resolution(String),
    #[error("audio decoding failed: {0}")]
    Decoder(String),
    #[error("no usable audio output device: {0}")]
    AudioDevice(String),
    #[error("playback failed: {0}")]
    Playback(String),
    #[error("live streams are not supported yet")]
    LiveUnsupported,
    #[error("operation cancelled")]
    Cancelled,
}

impl StreamError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Network(_) => "A temporary network problem interrupted playback. Try again.",
            Self::Config(_) => "YouTube configuration could not be loaded. Try again shortly.",
            Self::PlayerUnavailable(_) => "This track is unavailable for playback.",
            Self::LiveUnsupported => "Live streams are not supported yet",
            Self::AuthenticationRequired(_) => {
                "YouTube requires sign-in for this track; it can't be played without an account."
            }
            Self::PoToken(_) => "YouTube could not validate this playback session.",
            Self::Cipher(_) => "YouTube's player response could not be processed.",
            Self::MediaRejected(_) => "YouTube rejected the audio stream. Try again later.",
            Self::Resolution(_) => "YouTube could not provide this track right now.",
            Self::UnsupportedFormat => "This track has no supported audio format.",
            Self::Decoder(_) => "This audio format could not be decoded.",
            Self::AudioDevice(_) => "No audio output device is available.",
            Self::Playback(_) => "Playback could not continue.",
            Self::Cancelled => "Playback was cancelled.",
        }
        .into()
    }
}

#[async_trait]
pub trait StreamResolver: Send + Sync {
    async fn resolve(
        &self,
        track: &TrackId,
        quality: AudioQuality,
        cancellation: CancellationToken,
    ) -> Result<ValidatedPlaybackResolution, StreamError>;

    /// Loads what the first resolution needs, ahead of time. Failures are ignored here.
    async fn prewarm(&self) {}
}

/// YouTube-specific HTTP and Innertube details stay outside the playback engine.
pub struct YouTubeStreamResolver {
    token_provider: std::sync::Arc<dyn PoTokenProvider>,
    profile: ClientProfile,
    runtime: tokio::sync::Mutex<Option<PlayerRuntime>>,
    player_cache: tokio::sync::Mutex<HashMap<String, PlayerJsCacheEntry>>,
    /// Plays live broadcasts that the primary client refuses: the mobile web client serves their audio.
    live_fallback: Option<Box<YouTubeStreamResolver>>,
}

#[derive(Clone)]
struct PlayerJsCacheEntry {
    /// Fingerprint of the raw player JavaScript this entry describes.
    source_fp: String,
    /// The raw JavaScript on disk, handed to the helper when there is no preprocessed file.
    source_path: Option<std::path::PathBuf>,
    /// The preprocessed JavaScript on disk, written by the helper.
    preprocessed_path: Option<std::path::PathBuf>,
}

pub const MAX_PERSISTED_PREPROCESSED_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_PERSISTED_PLAYER_JS_ENTRIES: usize = 8;
/// Player versions kept on disk. Each version is one URL; the newest two are enough for a restart.
const MAX_PERSISTED_PLAYER_VERSIONS: usize = 2;

fn persisted_player_js_path(url_key: &str, source_fp: &str) -> Option<std::path::PathBuf> {
    Some(
        dirs::cache_dir()?
            .join("crabytube/player-js")
            .join(format!("{url_key}-{source_fp}.json")),
    )
}

fn raw_player_js_path(url_key: &str, source_fp: &str) -> Option<std::path::PathBuf> {
    Some(
        dirs::cache_dir()?
            .join("crabytube/player-js")
            .join(format!("{url_key}-{source_fp}.js")),
    )
}

/// Keeps the raw source on disk for the helper, then trims the directory.
fn store_raw_player_js(path: &std::path::Path, source: &str) {
    let Some(parent) = path.parent() else {
        return;
    };
    let _ = std::fs::create_dir_all(parent);
    if std::fs::write(path, source).is_ok() {
        trim_persisted_player_js_cache(parent);
    }
}

/// The raw player JavaScript saved for a URL, if its content still matches the fingerprint in the
/// file name. The URL names the player version, so the saved copy is the file the network would
/// return. A copy that fails the check is removed so the network copy replaces it.
fn saved_raw_player_js(url_key: &str) -> Option<(String, String, Option<std::path::PathBuf>)> {
    let directory = dirs::cache_dir()?.join("crabytube/player-js");
    let prefix = format!("{url_key}-");
    let (path, fingerprint) = std::fs::read_dir(&directory)
        .ok()?
        .flatten()
        .find_map(|entry| {
            let path = entry.path();
            let fingerprint = path
                .file_name()?
                .to_str()?
                .strip_prefix(&prefix)?
                .strip_suffix(".js")?
                .to_owned();
            Some((path, fingerprint))
        })?;
    let source = std::fs::read_to_string(&path).ok()?;
    if sha256_hex(source.as_bytes()) != fingerprint {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    // Using a version counts as recent, so the trim keeps the versions that are still in use.
    if let Ok(file) = std::fs::File::options().write(true).open(&path) {
        let _ = file.set_modified(SystemTime::now());
    }
    Some((source, fingerprint, Some(path)))
}

/// Whether a saved preprocessed file is present and plausibly sized. The helper checks its contents.
fn persisted_player_js_ready(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| {
        metadata.len() > 0 && metadata.len() <= MAX_PERSISTED_PREPROCESSED_BYTES * 2
    })
}

fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Diagnostics only: the preprocessed code stored in a helper-written cache file.
#[cfg(feature = "diagnostics")]
fn read_preprocessed_for_capture(path: &std::path::Path) -> Option<String> {
    let value: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    value.get("preprocessed")?.as_str().map(str::to_owned)
}

/// Paths handed to the helper for one preprocessing run.
struct JsHelperInput {
    player_js_path: Option<String>,
    preprocessed_cache_path: Option<String>,
    source_fp: Option<String>,
    output_cache_path: Option<String>,
}

/// Keeps the most recently used player versions. Files are named `<URL key>-<fingerprint>.<ext>`,
/// so the files that share a URL key are one version and are kept or removed together.
fn trim_persisted_player_js_cache(directory: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut versions: HashMap<String, (SystemTime, Vec<std::path::PathBuf>)> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|ext| ext == "json" || ext == "js")
        {
            continue;
        }
        let Some(url_key) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.get(..64))
        else {
            continue;
        };
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        let version = versions
            .entry(url_key.to_owned())
            .or_insert((UNIX_EPOCH, Vec::new()));
        version.0 = version.0.max(modified);
        version.1.push(path);
    }
    if versions.len() <= MAX_PERSISTED_PLAYER_VERSIONS {
        return;
    }
    let mut ordered: Vec<_> = versions.into_values().collect();
    ordered.sort_by_key(|version| std::cmp::Reverse(version.0));
    for (_, paths) in ordered.into_iter().skip(MAX_PERSISTED_PLAYER_VERSIONS) {
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl YouTubeStreamResolver {
    pub fn new(session: YouTubeSession) -> Self {
        let token_provider: std::sync::Arc<dyn PoTokenProvider> =
            std::sync::Arc::new(BotguardHelper::new());
        Self {
            token_provider: std::sync::Arc::clone(&token_provider),
            profile: ClientProfile::WebMusic,
            runtime: tokio::sync::Mutex::new(None),
            player_cache: tokio::sync::Mutex::new(HashMap::new()),
            live_fallback: Some(Box::new(Self::with_provider(
                session,
                token_provider,
                ClientProfile::Mweb,
            ))),
        }
    }

    pub fn with_provider(
        _session: YouTubeSession,
        token_provider: std::sync::Arc<dyn PoTokenProvider>,
        profile: ClientProfile,
    ) -> Self {
        Self {
            token_provider,
            profile,
            runtime: tokio::sync::Mutex::new(None),
            player_cache: tokio::sync::Mutex::new(HashMap::new()),
            live_fallback: None,
        }
    }

    fn client() -> Result<&'static Client, StreamError> {
        crate::network::shared().map_err(StreamError::Network)
    }

    async fn fetch_player_runtime(
        profile: PlayerClient,
        cancellation: &CancellationToken,
    ) -> Result<PlayerRuntime, StreamError> {
        let page = Self::fetch_page(
            profile.config_url,
            (!profile.user_agent.is_empty()).then_some(profile.user_agent),
            cancellation,
        )
        .await?;
        let config = parse_ytcfg(&page).ok_or_else(|| {
            StreamError::Config("client config did not contain an Innertube configuration".into())
        })?;
        let mut context = config
            .get("INNERTUBE_CONTEXT")
            .and_then(Value::as_object)
            .cloned()
            .map(Value::Object)
            .ok_or_else(|| {
                StreamError::Config("client config is missing Innertube context".into())
            })?;
        let client_context = context
            .get_mut("client")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                StreamError::Config("client config is missing its client context".into())
            })?;
        client_context.insert("hl".into(), Value::String("en".into()));
        client_context.insert("timeZone".into(), Value::String("UTC".into()));
        client_context.insert("utcOffsetMinutes".into(), Value::Number(0.into()));
        let version = client_context
            .get("clientVersion")
            .and_then(Value::as_str)
            .unwrap_or(profile.version)
            .to_owned();
        let user_agent = client_context
            .get("userAgent")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| (!profile.user_agent.is_empty()).then(|| profile.user_agent.to_owned()));
        let header_id = config
            .get("INNERTUBE_CONTEXT_CLIENT_NAME")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
            .unwrap_or_else(|| profile.fallback_client_id().to_owned());
        let api_host = config
            .get("INNERTUBE_HOST")
            .and_then(Value::as_str)
            .unwrap_or(profile.api_host);
        let api_key = config
            .get("INNERTUBE_API_KEY")
            .and_then(Value::as_str)
            .unwrap_or("AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX30");
        let signature_timestamp = config.get("STS").and_then(Value::as_u64);
        // By default BotGuard runs with a desktop browser identity, separate from MWEB.
        // The opt-in unified profile exists only for diagnosing GVS stability.
        let botguard_user_agent = if profile.name == "MWEB"
            && std::env::var("CRABYTUBE_UA_PROFILE").as_deref() == Ok("unified-mweb")
        {
            profile.user_agent
        } else {
            BOTGUARD_UA
        };
        let botguard_page = if profile.requires_gvs_po_token {
            Some(
                Self::fetch_page(
                    "https://www.youtube.com/",
                    Some(botguard_user_agent),
                    cancellation,
                )
                .await?,
            )
        } else {
            None
        };
        let botguard_config = botguard_page.as_deref().and_then(parse_ytcfg);
        let homepage_challenge_json = botguard_page
            .as_deref()
            .and_then(parse_yt_at_n)
            .and_then(|attestation| attestation.get("bgChallenge").cloned())
            .and_then(|challenge| serde_json::to_string(&challenge).ok());
        let event_id = botguard_config
            .as_ref()
            .unwrap_or(&config)
            .get("EVENT_ID")
            .and_then(Value::as_str)
            .filter(|event| !event.is_empty())
            .map(str::to_owned);
        if profile.requires_gvs_po_token && event_id.is_some() && homepage_challenge_json.is_none()
        {
            warn!(
                "BotGuard homepage challenge unavailable while EVENT_ID is present; legacy challenge fallback is not equivalent for this session"
            );
        }
        let gvs_binding = if has_video_id_gvs_binding(&config) {
            GvsPoBinding::VideoId
        } else {
            GvsPoBinding::VisitorData
        };
        let botguard_ytcfg_json = serde_json::to_string(
            botguard_config.as_ref().unwrap_or(&config),
        )
        .map_err(|error| StreamError::Config(format!("ytcfg serialization failed: {error}")))?;
        let endpoint =
            format!("https://{api_host}/youtubei/v1/player?key={api_key}&prettyPrint=false");
        let origin = format!("https://{api_host}");
        let api_key_present = config
            .get("INNERTUBE_API_KEY")
            .and_then(|value| value.as_str())
            .is_some();
        let visitor_data_present = context
            .pointer("/client/visitorData")
            .and_then(|value| value.as_str())
            .is_some();
        let user_agent_present = user_agent.is_some();
        let user_agent_fp = user_agent
            .as_deref()
            .map(|value| fingerprint8(value.as_bytes()));
        let signature_timestamp_present = signature_timestamp.is_some();
        let context_client_keys = context
            .pointer("/client")
            .and_then(|value| value.as_object())
            .map(|client| client.keys().cloned().collect::<Vec<_>>());
        info!(
            client = profile.name,
            client_version = %version,
            client_id = %header_id,
            api_key_present,
            visitor_data_present,
            user_agent_present,
            user_agent_fp = ?user_agent_fp,
            signature_timestamp_present,
            context_client_keys = ?context_client_keys,
            event_id_present = event_id.is_some(),
            homepage_challenge_present = homepage_challenge_json.is_some(),
            bg_visitor_matches_player_visitor = botguard_config.as_ref().and_then(|cfg| cfg.pointer("/INNERTUBE_CONTEXT/client/visitorData")).and_then(|value| value.as_str()) == context.pointer("/client/visitorData").and_then(|value| value.as_str()),
            gvs_binding = gvs_binding.label(),
            api_host,
            "Innertube client config loaded"
        );
        Ok(PlayerRuntime {
            fetched_at: std::time::Instant::now(),
            profile,
            endpoint,
            origin,
            header_id,
            version,
            user_agent,
            botguard_user_agent: botguard_user_agent.to_owned(),
            signature_timestamp,
            player_js_url: player_js_url(&config),
            context,
            botguard_ytcfg_json,
            homepage_challenge_json,
            event_id,
            gvs_binding,
        })
    }

    async fn fetch_page(
        url: &str,
        user_agent: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<String, StreamError> {
        const MAX_ATTEMPTS: u8 = 3;
        for attempt in 1..=MAX_ATTEMPTS {
            if cancellation.is_cancelled() {
                return Err(StreamError::Cancelled);
            }
            let mut request = Self::client()?.get(url);
            if let Some(user_agent) = user_agent {
                request = request.header(reqwest::header::USER_AGENT, user_agent);
            }
            let response = tokio::select! {
                _ = cancellation.cancelled() => return Err(StreamError::Cancelled),
                response = request.send() => match response {
                    Ok(response) => response,
                    Err(error) if attempt < MAX_ATTEMPTS && transient_reqwest_error(&error) => {
                        retry_config_get(attempt, url, None, cancellation).await?;
                        continue;
                    }
                    Err(error) => return Err(StreamError::Network(error.to_string())),
                },
            };
            let status = response.status();
            if retryable_config_status(status) {
                if attempt < MAX_ATTEMPTS {
                    retry_config_get(
                        attempt,
                        url,
                        response
                            .headers()
                            .get(reqwest::header::RETRY_AFTER)
                            .and_then(|value| value.to_str().ok())
                            .and_then(|value| value.parse::<u64>().ok()),
                        cancellation,
                    )
                    .await?;
                    continue;
                }
                return Err(StreamError::Network(format!(
                    "configuration GET returned HTTP {status} after {attempt} attempts"
                )));
            }
            if !status.is_success() {
                return Err(StreamError::Config(format!(
                    "configuration GET returned HTTP {status}"
                )));
            }
            let page = tokio::select! {
                _ = cancellation.cancelled() => return Err(StreamError::Cancelled),
                page = response.text() => match page {
                    Ok(page) => page,
                    Err(error) if attempt < MAX_ATTEMPTS && transient_reqwest_error(&error) => {
                        retry_config_get(attempt, url, None, cancellation).await?;
                        continue;
                    }
                    Err(error) => return Err(StreamError::Network(format!("configuration response read failed: {error}"))),
                },
            };
            return Ok(page);
        }
        Err(StreamError::Network(
            "configuration GET retry limit exhausted".into(),
        ))
    }

    /// Posts the player request for one track and returns the parsed response.
    async fn post_player(
        &self,
        runtime: &PlayerRuntime,
        visitor_data: &str,
        track: &TrackId,
    ) -> Result<Value, StreamError> {
        let request = player_request(
            &track.0,
            runtime.context.clone(),
            runtime.signature_timestamp,
        );
        let mut player_request = Self::client()?
            .post(&runtime.endpoint)
            .header("X-YouTube-Client-Name", &runtime.header_id)
            .header("X-YouTube-Client-Version", &runtime.version)
            .header("Origin", &runtime.origin)
            .header("X-Goog-Visitor-Id", visitor_data);
        if let Some(user_agent) = runtime.user_agent.as_deref() {
            player_request = player_request.header(reqwest::header::USER_AGENT, user_agent);
        }
        let response = player_request
            .json(&request)
            .send()
            .await
            .map_err(|error| StreamError::Network(error.to_string()))?;
        response
            .error_for_status()
            .map_err(|error| StreamError::Network(error.to_string()))?
            .json::<Value>()
            .await
            .map_err(|error| StreamError::Resolution(error.to_string()))
    }

    async fn player_runtime(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<PlayerRuntime, StreamError> {
        // The lock covers only the lookup and the store, never the page fetch, so callers do not queue behind it.
        let cached = self.runtime.lock().await.clone();
        if let Some(runtime) =
            cached.filter(|runtime| runtime_is_fresh(runtime.fetched_at, std::time::Instant::now()))
        {
            return Ok(runtime);
        }
        let mut loaded = Self::fetch_player_runtime(self.profile.config(), cancellation).await?;
        adopt_visitor(&mut loaded);
        *self.runtime.lock().await = Some(loaded.clone());
        Ok(loaded)
    }

    async fn ensure_signature_timestamp(
        &self,
        runtime: &mut PlayerRuntime,
        video_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<(), StreamError> {
        if runtime.signature_timestamp.is_some() {
            return Ok(());
        }
        if runtime.player_js_url.is_none() {
            // WEB_REMIX config comes from music.youtube.com; when it lacks PLAYER_JS_URL,
            // the standard watch page provides it.
            let webpage_url = format!(
                "https://www.youtube.com/watch?v={video_id}&bpctr=9999999999&has_verified=1"
            );
            let webpage = Self::fetch_page(&webpage_url, None, cancellation).await?;
            runtime.player_js_url = parse_ytcfg(&webpage).and_then(|config| player_js_url(&config));
            let mut cached_runtime = self.runtime.lock().await;
            if let Some(cached) = cached_runtime.as_mut() {
                cached.player_js_url = runtime.player_js_url.clone();
            }
        }
        let Some(js_url) = runtime.player_js_url.clone() else {
            warn!(
                client = runtime.profile.name,
                "player JS URL unavailable in music and watch-page config"
            );
            return Ok(());
        };
        let player_url = Url::parse("https://www.youtube.com/")
            .and_then(|base| base.join(&js_url))
            .map_err(|error| StreamError::Resolution(format!("invalid player JS URL: {error}")))?;
        let player_cache_key = sha256_hex(player_url.as_str().as_bytes());
        // A saved copy for this URL is reused, so a restart does not download the player again.
        let (player_js, source_fp, source_path) = match saved_raw_player_js(&player_cache_key) {
            Some(saved) => {
                info!(client = runtime.profile.name, "player JS reused from disk");
                saved
            }
            None => {
                let player_js = Self::fetch_page(
                    player_url.as_str(),
                    runtime.user_agent.as_deref(),
                    cancellation,
                )
                .await?;
                let source_fp = sha256_hex(player_js.as_bytes());
                let source_path = raw_player_js_path(&player_cache_key, &source_fp);
                if let Some(path) = &source_path {
                    store_raw_player_js(path, &player_js);
                }
                (player_js, source_fp, source_path)
            }
        };
        runtime.signature_timestamp = parse_signature_timestamp(&player_js);
        drop(player_js);
        {
            let mut cache = self.player_cache.lock().await;
            let prepared = cache
                .get(&player_cache_key)
                .filter(|entry| entry.source_fp == source_fp)
                .and_then(|entry| entry.preprocessed_path.clone());
            if !cache.contains_key(&player_cache_key) && cache.len() >= 4 {
                cache.clear();
            }
            cache.insert(
                player_cache_key,
                PlayerJsCacheEntry {
                    source_fp,
                    source_path,
                    preprocessed_path: prepared,
                },
            );
        }
        if let Some(signature_timestamp) = runtime.signature_timestamp {
            let mut cached_runtime = self.runtime.lock().await;
            if let Some(cached) = cached_runtime.as_mut()
                && cached.player_js_url.as_deref() == Some(js_url.as_str())
            {
                cached.signature_timestamp = Some(signature_timestamp);
            }
            info!(
                client = runtime.profile.name,
                signature_timestamp, "signature timestamp extracted from player JS"
            );
        } else {
            warn!(
                client = runtime.profile.name,
                "player JS contained no signature timestamp"
            );
        }
        Ok(())
    }
}

/// A body cut off mid-transfer reports as a decode error, so it counts as transient too.
fn transient_reqwest_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_body() || error.is_decode()
}

fn sanitized_reqwest_error(error: &reqwest::Error) -> String {
    format!(
        "timeout={} connect={} body={} status={}",
        error.is_timeout(),
        error.is_connect(),
        error.is_body(),
        error
            .status()
            .map_or_else(|| "none".to_owned(), |status| status.to_string()),
    )
}

fn retryable_config_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

async fn retry_config_get(
    attempt: u8,
    url: &str,
    retry_after_secs: Option<u64>,
    cancellation: &CancellationToken,
) -> Result<(), StreamError> {
    // Stable per URL/attempt jitter, avoiding another RNG dependency while
    // spreading retries slightly across concurrent desktop clients.
    let fingerprint = sha256_hex(format!("{url}:{attempt}").as_bytes());
    let jitter = u64::from_str_radix(&fingerprint[..2], 16).unwrap_or(0) % 61;
    let exponential_ms = u64::from(attempt) * 80;
    let retry_after_ms = retry_after_secs.unwrap_or(0).saturating_mul(1000).min(1000);
    let delay = Duration::from_millis(retry_after_ms.max(exponential_ms + jitter));
    warn!(
        attempt,
        retry_after_ms,
        delay_ms = delay.as_millis(),
        "retrying transient idempotent configuration GET"
    );
    tokio::select! {
        _ = cancellation.cancelled() => Err(StreamError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

#[async_trait]
impl StreamResolver for YouTubeStreamResolver {
    async fn prewarm(&self) {
        match self.player_runtime(&CancellationToken::new()).await {
            Ok(_) => info!("player runtime prewarmed"),
            Err(error) => warn!(?error, "player runtime prewarm failed"),
        }
    }

    #[instrument(skip(self, cancellation), fields(track_id_fp = %fingerprint8(track.0.as_bytes())))]
    async fn resolve(
        &self,
        track: &TrackId,
        quality: AudioQuality,
        cancellation: CancellationToken,
    ) -> Result<ValidatedPlaybackResolution, StreamError> {
        let mut max_validation_attempts = if self.profile == ClientProfile::WebMusic {
            1
        } else {
            3
        };
        let mut validation_attempt = 0;
        while validation_attempt < max_validation_attempts {
            validation_attempt += 1;
            static NEXT_RESOLUTION_ID: AtomicU64 = AtomicU64::new(1);
            let resolution_id = NEXT_RESOLUTION_ID.fetch_add(1, Ordering::Relaxed);
            let resolution_started = std::time::Instant::now();
            info!(
                resolution_id,
                validation_attempt,
                ?quality,
                "stream resolution started"
            );
            if cancellation.is_cancelled() {
                return Err(StreamError::Cancelled);
            }
            let mut runtime = self.player_runtime(&cancellation).await?;
            if self.profile == ClientProfile::WebMusic && runtime.signature_timestamp.is_none() {
                self.ensure_signature_timestamp(&mut runtime, &track.0, &cancellation)
                    .await?;
            }
            let visitor_data = runtime_visitor_data(&runtime)?;
            let tokens_future = async {
                if !runtime.profile.requires_gvs_po_token {
                    return Ok(None);
                }
                self.token_provider
                    .tokens(
                        po_token_request(&runtime, &visitor_data, track),
                        cancellation.child_token(),
                    )
                    .await
                    .map(Some)
                    .map_err(|error| match error {
                        PoTokenError::Cancelled => StreamError::Cancelled,
                        _ => StreamError::PoToken(error.to_string()),
                    })
            };
            // The player request carries no PO token, so it runs while the tokens are minted.
            let (tokens, payload) = tokio::select! {
                _ = cancellation.cancelled() => return Err(StreamError::Cancelled),
                joined = async { tokio::join!(tokens_future, self.post_player(&runtime, &visitor_data, track)) } => {
                    let (tokens, payload) = joined;
                    (tokens?, payload?)
                }
            };
            // Tokens restored from the saved cache are the only ones a 403 can blame on stale state.
            let tokens_reused = tokens.as_ref().is_some_and(|tokens| tokens.reused);
            let player_response_fp =
                fingerprint8(&serde_json::to_vec(&payload).unwrap_or_default());
            let playability = payload
                .pointer("/playabilityStatus/status")
                .and_then(Value::as_str);
            if playability.is_some_and(|status| status != "OK") {
                // The primary client cannot serve a live broadcast. Its player response says
                // so (`isLiveContent`), so the live fallback is tried only for those videos.
                if self.profile == ClientProfile::WebMusic && is_live_content(&payload) {
                    warn!(
                        resolution_id,
                        client = runtime.profile.name,
                        "live broadcast refused by the primary client; trying the live fallback"
                    );
                    let Some(fallback) = self.live_fallback.as_deref() else {
                        return Err(StreamError::LiveUnsupported);
                    };
                    return match fallback.resolve(track, quality, cancellation.clone()).await {
                        Ok(resolution) => Ok(resolution),
                        Err(StreamError::Cancelled) => Err(StreamError::Cancelled),
                        Err(error) => {
                            warn!(?error, "live fallback could not resolve the broadcast");
                            Err(StreamError::LiveUnsupported)
                        }
                    };
                }
                let reason = payload
                    .pointer("/playabilityStatus/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unspecified playability status");
                warn!(client = runtime.profile.name, status = ?playability, reason, "player client rejected track");
                let message = format!("{}: {playability:?} ({reason})", runtime.profile.name);
                return Err(match playability.unwrap_or_default() {
                    "LOGIN_REQUIRED" | "AGE_CHECK_REQUIRED" => {
                        StreamError::AuthenticationRequired(message)
                    }
                    _ => StreamError::PlayerUnavailable(message),
                });
            }

            let formats = payload
                .pointer("/streamingData/adaptiveFormats")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let audio_format_count = formats
                .iter()
                .filter(|item| {
                    item.get("mimeType")
                        .and_then(|value| value.as_str())
                        .is_some_and(|mime| mime.starts_with("audio/"))
                })
                .count();
            info!(resolution_id, player_response_fp = %player_response_fp, player_status = ?playability, adaptive_format_count = formats.len(), audio_format_count, "player response received");
            let Some(format) = choose_aac_format(formats, quality) else {
                if formats.is_empty() {
                    warn!(resolution_id, player_response_fp = %player_response_fp, "YouTube player response had no adaptive audio formats; direct formats unavailable");
                    return Err(StreamError::UnsupportedFormat);
                }
                let summaries: Vec<_> = formats
                    .iter()
                    .map(|item| {
                        (
                            item.get("itag").and_then(Value::as_u64),
                            item.get("mimeType")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown"),
                            item.get("url")
                                .and_then(Value::as_str)
                                .is_some_and(|url| !url.is_empty()),
                            item.get("signatureCipher")
                                .or_else(|| item.get("cipher"))
                                .is_some(),
                        )
                    })
                    .collect();
                let ciphered_audio_formats = formats
                    .iter()
                    .filter(|item| {
                        item.get("mimeType")
                            .and_then(Value::as_str)
                            .is_some_and(|mime| mime.starts_with("audio/"))
                            && item
                                .get("signatureCipher")
                                .or_else(|| item.get("cipher"))
                                .is_some()
                    })
                    .count();
                warn!(
                    resolution_id,
                    player_response_fp = %player_response_fp,
                    ?summaries,
                    ciphered_audio_formats,
                    "player returned formats but none matched audio-only AAC with a direct URL"
                );
                if ciphered_audio_formats > 0 {
                    return Err(StreamError::Cipher(format!(
                        "YouTube returned {ciphered_audio_formats} audio format(s) with signatureCipher but no direct URL; a compatible player signature decipher is required"
                    )));
                }
                return Err(StreamError::UnsupportedFormat);
            };

            let duration = payload
                .pointer("/videoDetails/lengthSeconds")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs);
            let (mut stream_url, player_js_fp, challenge_capture) = self
                .resolve_format_url(format, &payload, &runtime, &cancellation)
                .await?;
            if let Some(tokens) = &tokens {
                let gvs_token = match runtime.gvs_binding {
                    GvsPoBinding::VisitorData => &tokens.session.value,
                    GvsPoBinding::VideoId => &tokens.content.value,
                };
                set_po_token(&mut stream_url, gvs_token);
            }
            let media_url = stream_url.to_string();
            let live = is_live_media_url(&media_url);
            let player_js_fp = player_js_fp.unwrap_or_else(|| "not-required".into());
            let selected_pot = tokens.as_ref().map(|tokens| match runtime.gvs_binding {
                GvsPoBinding::VisitorData => &tokens.session.value,
                GvsPoBinding::VideoId => &tokens.content.value,
            });
            let url = Url::parse(&media_url).ok();
            let expires_at_unix = url.as_ref().and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "expire")
                    .and_then(|(_, value)| value.parse::<i64>().ok())
            });
            let url_fp = fingerprint8(media_url.as_bytes());
            let mut request_headers = vec![
                ("X-YouTube-Client-Name".into(), runtime.header_id.clone()),
                ("X-YouTube-Client-Version".into(), runtime.version.clone()),
                ("Origin".into(), runtime.origin.clone()),
                ("X-Goog-Visitor-Id".into(), visitor_data.clone()),
            ];
            if let Some(user_agent) = runtime.user_agent.clone() {
                request_headers.push(("User-Agent".into(), user_agent));
            }
            let mut audio_stream = AudioStream {
                diagnostics: Box::new(AudioStreamDiagnostics {
                    resolution_id,
                    validation_attempt,
                    preflight_status: 0,
                    player_response_fp,
                    player_js_fp,
                    pot_fp: selected_pot.map(|value| fingerprint8(value.as_bytes())),
                    gvs_binding: runtime.gvs_binding,
                    url_fp,
                    expires_at_unix,
                }),
                url: media_url,
                mime_type: format["mimeType"]
                    .as_str()
                    .unwrap_or("audio/mp4")
                    .to_owned(),
                codec: AudioCodec::Aac,
                bitrate: format["bitrate"]
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok()),
                content_length: format["contentLength"]
                    .as_str()
                    .and_then(|value| value.parse().ok()),
                duration,
                live,
                title: payload
                    .pointer("/videoDetails/title")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown track")
                    .to_owned(),
                author: payload
                    .pointer("/videoDetails/author")
                    .and_then(Value::as_str)
                    .filter(|author| !author.is_empty())
                    .map(str::to_owned),
                thumbnail_url: largest_thumbnail(&payload),
                itag: format
                    .get("itag")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok()),
                adaptive_format_count: formats.len(),
                audio_format_count: formats
                    .iter()
                    .filter(|item| {
                        item.get("mimeType")
                            .and_then(Value::as_str)
                            .is_some_and(|mime| mime.starts_with("audio/"))
                    })
                    .count(),
                signature_cipher: format
                    .get("signatureCipher")
                    .or_else(|| format.get("cipher"))
                    .and_then(Value::as_str)
                    .is_some(),
                signature_deciphered: format
                    .get("signatureCipher")
                    .or_else(|| format.get("cipher"))
                    .and_then(Value::as_str)
                    .and_then(|cipher| {
                        Url::parse(&format!("https://youtube.invalid/?{cipher}")).ok()
                    })
                    .is_some_and(|cipher| cipher.query_pairs().any(|(key, _)| key == "s")),
                n_present: format_has_n_param(format),
                n_transformed: format_has_n_param(format),
                request_headers,
                challenge_capture: challenge_capture.map(Box::new),
            };
            info!(
                itag = ?format.get("itag"),
                mime = %audio_stream.mime_type,
                bitrate = ?audio_stream.bitrate,
                content_length = ?audio_stream.content_length,
                resolution_ms = resolution_started.elapsed().as_millis(),
                resolution_id = audio_stream.diagnostics.resolution_id,
                player_response_fp = %audio_stream.diagnostics.player_response_fp,
                player_js_fp = %audio_stream.diagnostics.player_js_fp,
                pot_fp = ?audio_stream.diagnostics.pot_fp,
                url_fp = %audio_stream.diagnostics.url_fp,
                "stream resolved"
            );
            let mut headers = reqwest::header::HeaderMap::new();
            for (name, value) in &audio_stream.request_headers {
                headers.insert(
                    reqwest::header::HeaderName::from_bytes(name.as_bytes())
                        .map_err(|error| StreamError::Network(error.to_string()))?,
                    reqwest::header::HeaderValue::from_str(value)
                        .map_err(|error| StreamError::Network(error.to_string()))?,
                );
            }
            let preflight_client = crate::network::shared().map_err(StreamError::Network)?;
            info!(resolution_id, validation_attempt, "GVS preflight started");
            let response = tokio::select! {
                _ = cancellation.cancelled() => return Err(StreamError::Cancelled),
                response = preflight_client.get(&audio_stream.url)
                    .headers(headers)
                    .header(reqwest::header::RANGE, "bytes=0-1")
                    .send() => response.map_err(|error| StreamError::Network(format!("GVS preflight request failed ({})", sanitized_reqwest_error(&error))))?,
            };
            let status = response.status();
            drop(response);
            // Diagnostics only: CRABYTUBE_FAIL_RANGE_AT=0 refuses the first preflight with 403.
            #[cfg(feature = "diagnostics")]
            let status = if super::range_failure_at_seconds() == Some(0.0)
                && super::take_injected_range_failure()
            {
                warn!(
                    resolution_id,
                    validation_attempt, "diagnostic: refused the GVS preflight with 403"
                );
                reqwest::StatusCode::FORBIDDEN
            } else {
                status
            };
            if status == reqwest::StatusCode::OK || status == reqwest::StatusCode::PARTIAL_CONTENT {
                audio_stream.diagnostics.preflight_status = status.as_u16();
                info!(resolution_id, validation_attempt, %status, "GVS preflight accepted resolution");
                let validated = ValidatedPlaybackResolution::after_preflight(audio_stream);
                if validated.is_expired(SystemTime::now()) {
                    return Err(StreamError::MediaRejected(
                        "GVS URL expires within the safety window".into(),
                    ));
                }
                return Ok(validated);
            }
            if status == reqwest::StatusCode::FORBIDDEN
                && (validation_attempt < max_validation_attempts || tokens_reused)
            {
                if tokens_reused {
                    // Saved tokens were refused: drop them and the saved runtime, then retry once cold.
                    warn!(
                        resolution_id,
                        "GVS refused saved BotGuard tokens; retrying with a fresh visitor"
                    );
                    self.token_provider.forget_visitor(&visitor_data).await;
                    crate::runtime_cache::forget_visitor();
                    crate::runtime_cache::forget_web();
                    *self.runtime.lock().await = None;
                    max_validation_attempts = max_validation_attempts.max(validation_attempt + 1);
                }
                let delay = if validation_attempt == 1 { 175 } else { 375 };
                warn!(resolution_id, validation_attempt, %status, backoff_ms = delay, "GVS rejected resolution; discarding complete context before one fresh resolution");
                tokio::select! {
                    _ = cancellation.cancelled() => return Err(StreamError::Cancelled),
                    _ = tokio::time::sleep(Duration::from_millis(delay)) => {},
                }
                continue;
            }
            warn!(resolution_id, validation_attempt, %status, "GVS preflight exhausted full-resolution retry limit");
            return Err(StreamError::MediaRejected(format!(
                "GVS preflight returned HTTP {status} after {validation_attempt} full resolution attempt(s)"
            )));
        }
        Err(StreamError::Network(
            "GVS preflight retry limit exhausted".into(),
        ))
    }
}

fn parse_signature_timestamp(player_js: &str) -> Option<u64> {
    ["signatureTimestamp", "sts"]
        .into_iter()
        .find_map(|marker| {
            let mut offset = 0;
            while let Some(found) = player_js[offset..].find(marker) {
                let start = offset + found + marker.len();
                let tail = player_js[start..].trim_start();
                if let Some(value) = tail.strip_prefix(':') {
                    let digits = value
                        .trim_start()
                        .bytes()
                        .take_while(|byte| byte.is_ascii_digit())
                        .count();
                    if digits == 5 {
                        return value.trim_start()[..digits].parse().ok();
                    }
                }
                offset = start;
                if offset >= player_js.len() {
                    break;
                }
            }
            None
        })
}

fn format_has_n_param(format: &Value) -> bool {
    let cipher = format
        .get("signatureCipher")
        .or_else(|| format.get("cipher"))
        .and_then(Value::as_str)
        .and_then(|cipher| Url::parse(&format!("https://youtube.invalid/?{cipher}")).ok());
    let url = format
        .get("url")
        .and_then(Value::as_str)
        .and_then(|url| Url::parse(url).ok())
        .or_else(|| {
            cipher.as_ref().and_then(|cipher| {
                cipher
                    .query_pairs()
                    .find(|(key, _)| key == "url")
                    .and_then(|(_, url)| Url::parse(&url).ok())
            })
        });
    url.is_some_and(|url| url.query_pairs().any(|(key, _)| key == "n"))
}

impl YouTubeStreamResolver {
    async fn resolve_format_url(
        &self,
        format: &Value,
        payload: &Value,
        runtime: &PlayerRuntime,
        cancellation: &CancellationToken,
    ) -> Result<(Url, Option<String>, Option<ChallengeCapture>), StreamError> {
        let cipher = format
            .get("signatureCipher")
            .or_else(|| format.get("cipher"))
            .and_then(Value::as_str);
        let cipher_url = cipher
            .and_then(|cipher| Url::parse(&format!("https://youtube.invalid/?{cipher}")).ok());
        let raw_url = format
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                cipher_url.as_ref().and_then(|url| {
                    url.query_pairs()
                        .find(|(key, _)| key == "url")
                        .map(|(_, value)| value.into_owned())
                })
            })
            .ok_or(StreamError::UnsupportedFormat)?;
        #[cfg(feature = "diagnostics")]
        let raw_url_for_capture = raw_url.clone();
        let mut url = Url::parse(&raw_url)
            .map_err(|error| StreamError::Resolution(format!("invalid stream URL: {error}")))?;
        let signature = cipher_url.as_ref().and_then(|cipher| {
            cipher
                .query_pairs()
                .find(|(key, _)| key == "s")
                .map(|(_, value)| value.into_owned())
        });
        let signature_parameter = cipher_url
            .as_ref()
            .and_then(|cipher| {
                cipher
                    .query_pairs()
                    .find(|(key, _)| key == "sp")
                    .map(|(_, value)| value.into_owned())
            })
            .unwrap_or_else(|| "signature".to_owned());
        let direct_signature = cipher_url.as_ref().and_then(|cipher| {
            cipher
                .query_pairs()
                .find(|(key, _)| key == "sig" || key == "signature")
                .map(|(_, value)| value.into_owned())
        });
        let n_value = url
            .query_pairs()
            .find(|(key, _)| key == "n")
            .map(|(_, value)| value.into_owned());
        if signature.is_none() && direct_signature.is_none() && n_value.is_none() {
            return Ok((url, None, None));
        }
        let mut player_js_fp = None;
        #[cfg(feature = "diagnostics")]
        let mut challenge_capture = None;
        #[cfg(not(feature = "diagnostics"))]
        let challenge_capture = None;
        let transformed = if signature.is_some() || n_value.is_some() {
            let js_url = payload
                .pointer("/assets/js")
                .and_then(Value::as_str)
                .or(runtime.player_js_url.as_deref())
                .ok_or_else(|| {
                    StreamError::Resolution("YouTube player JS URL is missing".into())
                })?;
            let player_url = Url::parse("https://www.youtube.com/")
                .and_then(|base| base.join(js_url))
                .map_err(|error| {
                    StreamError::Resolution(format!("invalid player JS URL: {error}"))
                })?;
            let cache_key = sha256_hex(player_url.as_str().as_bytes());
            let cached = self.player_cache.lock().await.get(&cache_key).cloned();
            let entry = match cached {
                Some(entry) => entry,
                None => {
                    let source = Self::fetch_page(
                        player_url.as_str(),
                        runtime.user_agent.as_deref(),
                        cancellation,
                    )
                    .await?;
                    let source_fp = sha256_hex(source.as_bytes());
                    let source_path = raw_player_js_path(&cache_key, &source_fp);
                    if let Some(path) = &source_path {
                        store_raw_player_js(path, &source);
                    }
                    let entry = PlayerJsCacheEntry {
                        source_fp,
                        source_path,
                        preprocessed_path: None,
                    };
                    let mut cache = self.player_cache.lock().await;
                    if !cache.contains_key(&cache_key) && cache.len() >= 4 {
                        cache.clear();
                    }
                    cache.insert(cache_key.clone(), entry.clone());
                    entry
                }
            };
            player_js_fp = Some(fingerprint8(entry.source_fp.as_bytes()));
            let preprocessed_path = entry
                .preprocessed_path
                .clone()
                .filter(|path| path.is_file())
                .or_else(|| {
                    persisted_player_js_path(&cache_key, &entry.source_fp)
                        .filter(|path| persisted_player_js_ready(path))
                });
            let transform_cache = if entry.preprocessed_path.is_some() {
                "memory"
            } else if preprocessed_path.is_some() {
                "disk"
            } else {
                "cold"
            };
            info!(player_js_fp = %player_js_fp.as_deref().unwrap_or("unknown"), transform_cache, "player JS transform cache selected");
            let raw_path = entry.source_path.clone().filter(|path| path.is_file());
            if raw_path.is_none() && preprocessed_path.is_none() {
                self.player_cache.lock().await.remove(&cache_key);
                return Err(StreamError::Resolution(
                    "player JS was evicted from the cache; retry the track".into(),
                ));
            }
            // A preprocessed file from an earlier launch is used when the helper accepts it;
            // otherwise the helper preprocesses the raw source and writes the file for next time.
            let output_path = persisted_player_js_path(&cache_key, &entry.source_fp);
            if let Some(parent) = output_path.as_ref().and_then(|path| path.parent()) {
                let _ = std::fs::create_dir_all(parent);
            }
            let input = JsHelperInput {
                player_js_path: raw_path.as_deref().map(path_string),
                preprocessed_cache_path: preprocessed_path.as_deref().map(path_string),
                source_fp: Some(entry.source_fp.clone()),
                output_cache_path: output_path.as_deref().map(path_string),
            };
            let result = tokio::select! {
                _ = cancellation.cancelled() => return Err(StreamError::Cancelled),
                result = run_js_challenge_helper(input, signature.clone(), n_value.clone(), cancellation.child_token()) => result.map_err(|error| match error {
                    StreamError::Cancelled => StreamError::Cancelled,
                    other => StreamError::Cipher(other.to_string()),
                })?,
            };
            info!(js_helper_peak_rss_kib = ?result.peak_rss_kib, "JS challenge helper finished");
            if let Some(memory_profile) = &result.memory_profile {
                let stages: Vec<_> = memory_profile
                    .iter()
                    .map(|stage| (stage.stage.as_str(), stage.rss_kib))
                    .collect();
                info!(js_helper_memory_stages = ?stages, "JS challenge helper staged memory profile");
            }
            let stored_preprocessed = if result.preprocessed_written {
                output_path.clone()
            } else {
                preprocessed_path.clone()
            }
            .filter(|path| path.is_file());
            #[cfg(feature = "diagnostics")]
            {
                challenge_capture = Some(ChallengeCapture {
                    player_js_url: player_url.to_string(),
                    player_js: raw_path
                        .as_deref()
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .unwrap_or_default(),
                    preprocessed_player: stored_preprocessed
                        .as_deref()
                        .and_then(read_preprocessed_for_capture)
                        .unwrap_or_default(),
                    raw_signature_cipher: cipher.map(str::to_owned),
                    raw_signature: signature.clone(),
                    signature_parameter: signature_parameter.clone(),
                    raw_url: raw_url_for_capture.clone(),
                    raw_n: n_value.clone(),
                    native_signature: result.signature.clone(),
                    native_n: result.n.clone(),
                });
            }
            if let Some(path) = stored_preprocessed
                && let Some(entry) = self.player_cache.lock().await.get_mut(&cache_key)
            {
                entry.preprocessed_path = Some(path);
            }
            result
        } else {
            JsTransformResult {
                signature: None,
                n: None,
                preprocessed_written: false,
                peak_rss_kib: None,
                memory_profile: None,
            }
        };
        let final_signature = transformed.signature.or(direct_signature);
        let mut query: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(key, _)| {
                key != "n"
                    && (final_signature.is_none()
                        || (key != &signature_parameter && key != "signature" && key != "sig"))
            })
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        if let Some(signature) = final_signature {
            query.push((signature_parameter, signature));
        }
        if let Some(n) = transformed.n.or(n_value) {
            query.push(("n".into(), n));
        }
        url.query_pairs_mut().clear().extend_pairs(query);
        Ok((url, player_js_fp, challenge_capture))
    }
}

#[derive(serde::Deserialize)]
struct JsTransformResult {
    signature: Option<String>,
    n: Option<String>,
    /// True when the helper wrote a new preprocessed file to the path it was given.
    #[serde(default)]
    preprocessed_written: bool,
    peak_rss_kib: Option<u64>,
    #[serde(default)]
    memory_profile: Option<Vec<JsHelperMemoryStage>>,
}

#[derive(serde::Deserialize, Debug)]
struct JsHelperMemoryStage {
    stage: String,
    rss_kib: Option<u64>,
}

/// Upper bound for one JS challenge helper run before it is killed.
const JS_HELPER_TIMEOUT: Duration = Duration::from_secs(15);

async fn run_js_challenge_helper(
    input: JsHelperInput,
    signature: Option<String>,
    n: Option<String>,
    cancellation: CancellationToken,
) -> Result<JsTransformResult, StreamError> {
    match run_js_helper_once(
        &input,
        signature.clone(),
        n.clone(),
        cancellation.child_token(),
    )
    .await
    {
        Err(error)
            if !matches!(error, StreamError::Cancelled)
                && input.preprocessed_cache_path.is_some()
                && input.player_js_path.is_some() =>
        {
            // The saved file was unusable: drop it and preprocess the raw source once.
            warn!(%error, "saved player JS preprocessing rejected; preprocessing the raw source");
            if let Some(path) = &input.preprocessed_cache_path {
                let _ = std::fs::remove_file(path);
            }
            let raw_only = JsHelperInput {
                preprocessed_cache_path: None,
                ..input
            };
            run_js_helper_once(&raw_only, signature, n, cancellation).await
        }
        other => other,
    }
}

async fn run_js_helper_once(
    input: &JsHelperInput,
    signature: Option<String>,
    n: Option<String>,
    cancellation: CancellationToken,
) -> Result<JsTransformResult, StreamError> {
    use std::process::Stdio;
    use tokio::{io::AsyncWriteExt, process::Command};
    #[derive(serde::Serialize)]
    struct Request {
        protocol_version: u16,
        player_js_path: Option<String>,
        preprocessed_cache_path: Option<String>,
        source_fp: Option<String>,
        output_cache_path: Option<String>,
        signature: Option<String>,
        n: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Failure {
        error: String,
    }
    let name = if cfg!(windows) {
        "js-challenge-helper.exe"
    } else {
        "js-challenge-helper"
    };
    // Overrides are for diagnostics builds only; release builds run the helper beside the app.
    #[cfg(feature = "diagnostics")]
    let override_path = std::env::var_os("CRABYTUBE_JS_CHALLENGE_HELPER");
    #[cfg(not(feature = "diagnostics"))]
    let override_path: Option<std::ffi::OsString> = None;
    let mut command = if let Some(path) = override_path {
        Command::new(path)
    } else if let Ok(current) = std::env::current_exe()
        && let Some(path) = current
            .parent()
            .map(|parent| parent.join(name))
            .filter(|path| path.is_file())
    {
        Command::new(path)
    } else {
        // Never fall back to $PATH: a helper found there would run with our input.
        return Err(StreamError::Decoder(format!(
            "{name} is not installed next to the application executable"
        )));
    };
    // The player parser briefly frees large allocations. Bound glibc's
    // arenas and return freed pages to the OS in this short-lived child.
    // Respect allocator settings supplied by the caller.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    for (key, value) in [
        ("MALLOC_ARENA_MAX", "1"),
        ("MALLOC_TRIM_THRESHOLD_", "65536"),
        ("MALLOC_MMAP_THRESHOLD_", "131072"),
        ("MALLOC_TOP_PAD_", "0"),
    ] {
        if std::env::var_os(key).is_none() {
            command.env(key, value);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| StreamError::Decoder(format!("JS helper could not start: {error}")))?;
    let request = serde_json::to_vec(&Request {
        protocol_version: 2,
        player_js_path: input.player_js_path.clone(),
        preprocessed_cache_path: input.preprocessed_cache_path.clone(),
        source_fp: input.source_fp.clone(),
        output_cache_path: input.output_cache_path.clone(),
        signature,
        n,
    })
    .map_err(|error| StreamError::Decoder(error.to_string()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| StreamError::Decoder("JS helper stdin unavailable".into()))?;
    stdin
        .write_all(&request)
        .await
        .map_err(|error| StreamError::Decoder(error.to_string()))?;
    drop(stdin);
    // A stuck helper is killed when the timed-out future drops the child (kill_on_drop).
    let output = tokio::select! {
        _ = cancellation.cancelled() => return Err(StreamError::Cancelled),
        result = tokio::time::timeout(JS_HELPER_TIMEOUT, child.wait_with_output()) => result
            .map_err(|_| StreamError::Decoder("JS helper timed out".into()))?
            .map_err(|error| StreamError::Decoder(error.to_string()))?,
    };
    if !output.status.success() {
        let message = serde_json::from_slice::<Failure>(&output.stdout)
            .map(|failure| failure.error)
            .unwrap_or_else(|_| format!("helper exited with {}", output.status));
        return Err(StreamError::Decoder(message));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| StreamError::Decoder(format!("invalid helper response: {error}")))
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn fingerprint8(data: &[u8]) -> String {
    sha256_hex(data).chars().take(8).collect()
}

fn set_po_token(url: &mut Url, token: &str) {
    let query = url
        .query_pairs()
        .filter(|(key, _)| key != "pot")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(query)
        .append_pair("pot", token);
}

/// Uses the saved visitor when there is one, so saved BotGuard tokens match the player
/// request. Without one, this launch's visitor is saved for the next launch.
fn adopt_visitor(runtime: &mut PlayerRuntime) {
    match crate::runtime_cache::visitor() {
        Some(saved) => set_visitor(runtime, &saved),
        None => {
            if let Ok(visitor) = runtime_visitor_data(runtime) {
                crate::runtime_cache::store_visitor(&visitor);
            }
        }
    }
}

/// Sets the visitor in the player context and in the BotGuard config, which the helper
/// mints tokens from.
fn set_visitor(runtime: &mut PlayerRuntime, visitor: &str) {
    if let Some(client) = runtime
        .context
        .get_mut("client")
        .and_then(Value::as_object_mut)
    {
        client.insert("visitorData".into(), Value::String(visitor.to_owned()));
    }
    if let Ok(mut config) = serde_json::from_str::<Value>(&runtime.botguard_ytcfg_json)
        && let Some(client) = config
            .pointer_mut("/INNERTUBE_CONTEXT/client")
            .and_then(Value::as_object_mut)
    {
        client.insert("visitorData".into(), Value::String(visitor.to_owned()));
        runtime.botguard_ytcfg_json = config.to_string();
    }
}

fn runtime_visitor_data(runtime: &PlayerRuntime) -> Result<String, StreamError> {
    runtime
        .context
        .pointer("/client/visitorData")
        .and_then(Value::as_str)
        .filter(|visitor| !visitor.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| StreamError::Resolution("client config is missing Visitor Data".into()))
}

fn po_token_request(
    runtime: &PlayerRuntime,
    visitor_data: &str,
    track: &TrackId,
) -> PoTokenRequest {
    PoTokenRequest {
        visitor_data: visitor_data.to_owned(),
        video_id: track.0.clone(),
        user_agent: runtime.botguard_user_agent.clone(),
        ytcfg_json: runtime.botguard_ytcfg_json.clone(),
        homepage_challenge_json: runtime.homepage_challenge_json.clone(),
        event_id: runtime.event_id.clone(),
    }
}

fn player_request(video_id: &str, context: Value, signature_timestamp: Option<u64>) -> Value {
    let mut request = json!({
        "context": context,
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "playbackContext": { "contentPlaybackContext": { "html5Preference": "HTML5_PREF_WANTS" } }
    });
    if let Some(signature_timestamp) = signature_timestamp {
        request["playbackContext"]["contentPlaybackContext"]["signatureTimestamp"] =
            json!(signature_timestamp);
    }
    request
}

fn player_js_url(config: &Value) -> Option<String> {
    config
        .get("PLAYER_JS_URL")
        .and_then(Value::as_str)
        .or_else(|| {
            config
                .get("WEB_PLAYER_CONTEXT_CONFIGS")
                .and_then(Value::as_object)
                .and_then(|configs| {
                    configs
                        .values()
                        .find_map(|context| context.get("jsUrl").and_then(Value::as_str))
                })
        })
        .map(str::to_owned)
}

pub(crate) fn parse_ytcfg(page: &str) -> Option<Value> {
    let marker = "ytcfg.set(";
    let mut search_from = 0;
    while let Some(relative) = page.get(search_from..)?.find(marker) {
        let start = search_from + relative + marker.len();
        let bytes = page.as_bytes();
        let mut cursor = start;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'{') {
            search_from = cursor.saturating_add(1);
            continue;
        }
        let json_start = cursor;
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        while let Some(&byte) = bytes.get(cursor) {
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else if byte == b'"' {
                in_string = true;
            } else if byte == b'{' {
                depth += 1;
            } else if byte == b'}' {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Ok(value) = serde_json::from_slice::<Value>(&bytes[json_start..=cursor])
                        && value
                            .pointer("/INNERTUBE_CONTEXT/client/clientName")
                            .is_some()
                    {
                        return Some(value);
                    }
                    break;
                }
            }
            cursor += 1;
        }
        search_from = cursor.saturating_add(1);
    }
    None
}

fn parse_yt_at_n(page: &str) -> Option<Value> {
    let marker = "ytAtN(";
    let mut search_from = 0;
    while let Some(relative) = page.get(search_from..)?.find(marker) {
        let call_at = search_from + relative + marker.len();
        let mut open_brace = call_at;
        while page
            .as_bytes()
            .get(open_brace)
            .is_some_and(u8::is_ascii_whitespace)
        {
            open_brace += 1;
        }
        if page.as_bytes().get(open_brace) == Some(&b'{')
            && let Some(object_end) = matching_js_object_end(page, open_brace)
            && let Some(call) = page
                .get(open_brace..=object_end)
                .and_then(|source| json5::from_str::<Value>(source).ok())
            && let Some(response) = call.get("R")
        {
            let attestation = match response {
                Value::String(raw) => json5::from_str::<Value>(raw).ok(),
                value => Some(value.clone()),
            };
            if let Some(attestation) = attestation
                && attestation.get("bgChallenge").is_some_and(Value::is_object)
            {
                return Some(attestation);
            }
        }
        search_from = call_at.saturating_add(1);
    }
    None
}

fn matching_js_object_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut cursor = start;
    while let Some(&byte) = bytes.get(cursor) {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
        } else if matches!(byte, b'{' | b'[' | b'(') {
            stack.push(byte);
        } else if matches!(byte, b'}' | b']' | b')') {
            let expected = match byte {
                b'}' => b'{',
                b']' => b'[',
                _ => b'(',
            };
            if stack.pop() != Some(expected) {
                return None;
            }
            if stack.is_empty() {
                return Some(cursor);
            }
        }
        cursor += 1;
    }
    None
}

fn has_video_id_gvs_binding(config: &Value) -> bool {
    config
        .get("WEB_PLAYER_CONTEXT_CONFIGS")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(serde_json::Map::values)
        .filter_map(|entry| {
            entry
                .get("serializedExperimentFlags")
                .and_then(Value::as_str)
        })
        .any(|flags| {
            Url::parse(&format!("https://youtube.invalid/?{flags}"))
                .ok()
                .is_some_and(|url| {
                    url.query_pairs().any(|(key, value)| {
                        key == "html5_generate_content_po_token" && value == "true"
                    })
                })
        })
}

/// Whether the player response describes a live broadcast.
fn is_live_content(payload: &Value) -> bool {
    payload
        .pointer("/videoDetails/isLiveContent")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Live audio URLs carry `live=1`. Their media is a fragment stream that ends with the broadcast.
fn is_live_media_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.query_pairs()
            .any(|(key, value)| key == "live" && value == "1")
    })
}

fn choose_aac_format(formats: &[Value], quality: AudioQuality) -> Option<&Value> {
    formats
        .iter()
        .filter(|format| {
            format
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|url| !url.is_empty())
                || format
                    .get("signatureCipher")
                    .or_else(|| format.get("cipher"))
                    .and_then(Value::as_str)
                    .is_some_and(|cipher| !cipher.is_empty())
        })
        .filter(|format| {
            let mime = format
                .get("mimeType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            mime.starts_with("audio/") && !mime.starts_with("video/")
        })
        .filter(|format| {
            let mime = format
                .get("mimeType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            mime.starts_with("audio/mp4") && mime.contains("mp4a.40.2")
        })
        .filter(|format| {
            quality_allows(
                quality,
                format
                    .get("bitrate")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
            )
        })
        .max_by_key(|format| {
            format
                .get("bitrate")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        })
}

fn quality_allows(quality: AudioQuality, bitrate: u64) -> bool {
    match quality {
        AudioQuality::Auto | AudioQuality::Best => true,
        AudioQuality::Low => bitrate <= 96_000,
        AudioQuality::Normal => (96_000..=160_000).contains(&bitrate),
        AudioQuality::High => bitrate >= 128_000,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_player_runtime_is_reused_for_six_hours_only() {
        use super::{PLAYER_RUNTIME_TTL, runtime_is_fresh};
        let fetched = std::time::Instant::now();
        assert!(runtime_is_fresh(fetched, fetched));
        assert!(runtime_is_fresh(
            fetched,
            fetched + PLAYER_RUNTIME_TTL - std::time::Duration::from_secs(1)
        ));
        assert!(!runtime_is_fresh(fetched, fetched + PLAYER_RUNTIME_TTL));
    }

    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    use super::{
        AudioCodec, AudioQuality, AudioStream, AudioStreamDiagnostics, ClientProfile, GvsPoBinding,
        MWEB_UA, StreamError, StreamResolver, TrackId, ValidatedPlaybackResolution,
        YouTubeStreamResolver, choose_aac_format, has_video_id_gvs_binding, is_live_content,
        is_live_media_url, parse_signature_timestamp, parse_yt_at_n, parse_ytcfg,
        persisted_player_js_ready, player_js_url, player_request, retryable_config_status,
        set_po_token,
    };

    #[test]
    fn saved_player_preprocessing_is_used_only_when_present_and_nonempty() {
        let path = std::env::temp_dir().join(format!(
            "crabytube-player-js-ready-{}.json",
            std::process::id()
        ));
        assert!(!persisted_player_js_ready(&path));
        std::fs::write(&path, b"").unwrap();
        assert!(!persisted_player_js_ready(&path));
        std::fs::write(
            &path,
            br#"{"version":3,"source_fp":"f","preprocessed":"code"}"#,
        )
        .unwrap();
        assert!(persisted_player_js_ready(&path));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn po_token_is_encoded_once_and_survives_url_clone_and_query_edit() {
        let token = "opaque+/=token%value";
        let mut url =
            reqwest::Url::parse("https://rr.googlevideo.com/videoplayback?itag=140&pot=stale")
                .expect("fixture URL");
        set_po_token(&mut url, token);
        assert_eq!(url.query_pairs().filter(|(key, _)| key == "pot").count(), 1);
        assert_eq!(
            url.query_pairs()
                .find(|(key, _)| key == "pot")
                .map(|(_, value)| value.into_owned())
                .as_deref(),
            Some(token)
        );
        let encoded = url.as_str().to_owned();
        assert!(encoded.contains("pot=opaque%2B%2F%3Dtoken%25value"));
        assert!(!encoded.contains("%252B"));
        let mut cloned = url.clone();
        cloned.query_pairs_mut().append_pair("range_probe", "1");
        assert_eq!(
            cloned
                .query_pairs()
                .find(|(key, _)| key == "pot")
                .map(|(_, value)| value.into_owned())
                .as_deref(),
            Some(token)
        );
    }

    fn format(itag: u64, mime: &str, bitrate: u64, url: bool) -> serde_json::Value {
        json!({"itag":itag,"mimeType":mime,"bitrate":bitrate,"url":url.then_some("https://example.invalid/audio")})
    }

    #[test]
    fn auto_prefers_highest_supported_audio_only_aac() {
        let formats = [
            format(251, "audio/webm; codecs=\"opus\"", 160_000, true),
            format(140, "audio/mp4; codecs=\"mp4a.40.2\"", 128_000, true),
            format(18, "video/mp4; codecs=\"avc1, mp4a.40.2\"", 96_000, true),
        ];

        assert_eq!(
            choose_aac_format(&formats, AudioQuality::Auto)
                .and_then(|format| format["itag"].as_u64()),
            Some(140)
        );
    }

    #[test]
    fn quality_caps_and_minimums_are_applied() {
        let formats = [format(
            140,
            "audio/mp4; codecs=\"mp4a.40.2\"",
            128_000,
            true,
        )];
        assert!(choose_aac_format(&formats, AudioQuality::Low).is_none());
        assert_eq!(
            choose_aac_format(&formats, AudioQuality::Normal)
                .and_then(|format| format["itag"].as_u64()),
            Some(140)
        );
    }

    #[test]
    fn format_selection_skips_entries_without_progressive_urls() {
        let formats = [format(
            140,
            "audio/mp4; codecs=\"mp4a.40.2\"",
            128_000,
            false,
        )];
        assert!(choose_aac_format(&formats, AudioQuality::Auto).is_none());
    }

    #[test]
    fn mweb_player_request_uses_the_current_client_context_without_player_token() {
        let profile = ClientProfile::Mweb.config();
        let request = player_request(
            "video-id",
            json!({"client":{"clientName":"MWEB","clientVersion":"2.20990101.00.00","visitorData":"visitor-id","userAgent":MWEB_UA,"hl":"en","gl":"PR","timeZone":"UTC","utcOffsetMinutes":0},"request":{},"user":{},"clickTracking":{}}),
            Some(20347),
        );
        assert_eq!(request["videoId"], "video-id");
        assert_eq!(
            request.pointer("/context/client/clientName"),
            Some(&json!("MWEB"))
        );
        assert_eq!(
            request.pointer("/context/client/clientVersion"),
            Some(&json!("2.20990101.00.00"))
        );
        assert_eq!(
            request.pointer("/context/client/userAgent"),
            Some(&json!(MWEB_UA))
        );
        assert_eq!(profile.fallback_client_id(), "2");
        assert_eq!(profile.config_url, "https://m.youtube.com");
        assert_eq!(profile.api_host, "www.youtube.com");
        assert_eq!(
            request.pointer("/context/client/visitorData"),
            Some(&json!("visitor-id"))
        );
        assert_eq!(
            request.pointer("/playbackContext/contentPlaybackContext/html5Preference"),
            Some(&json!("HTML5_PREF_WANTS"))
        );
        assert_eq!(
            request.pointer("/playbackContext/contentPlaybackContext/signatureTimestamp"),
            Some(&json!(20347))
        );
        assert!(request.get("serviceIntegrityDimensions").is_none());
        assert!(request.get("cookies").is_none());
    }

    #[test]
    fn web_music_profile_is_distinct_and_uses_music_host_defaults() {
        let profile = ClientProfile::WebMusic.config();
        assert_eq!(profile.name, "WEB_REMIX");
        assert_eq!(profile.fallback_client_id(), "67");
        assert_eq!(profile.version, "1.20260707.12.00");
        assert_eq!(profile.config_url, "https://music.youtube.com");
        assert_eq!(profile.api_host, "music.youtube.com");
        assert!(profile.requires_gvs_po_token);
        assert!(profile.user_agent.is_empty());
    }

    #[test]
    fn default_stream_resolver_uses_web_music_as_primary_client() {
        let resolver = YouTubeStreamResolver::new(crate::music::YouTubeSession::default());
        assert_eq!(resolver.profile, ClientProfile::WebMusic);
    }

    #[test]
    fn validated_resolution_requires_an_accepted_preflight_and_tracks_expiry() {
        let make_stream = |status, expires_at_unix| AudioStream {
            diagnostics: Box::new(AudioStreamDiagnostics {
                resolution_id: 7,
                validation_attempt: 1,
                preflight_status: status,
                player_response_fp: "player".into(),
                player_js_fp: "js".into(),
                pot_fp: None,
                gvs_binding: GvsPoBinding::VideoId,
                url_fp: "url".into(),
                expires_at_unix,
            }),
            url: "https://media.invalid/audio".into(),
            mime_type: "audio/mp4".into(),
            codec: AudioCodec::Aac,
            bitrate: None,
            content_length: None,
            duration: None,
            live: false,
            title: "fixture".into(),
            author: None,
            thumbnail_url: None,
            itag: Some(140),
            adaptive_format_count: 1,
            audio_format_count: 1,
            signature_cipher: false,
            signature_deciphered: false,
            n_present: false,
            n_transformed: false,
            request_headers: Vec::new(),
            challenge_capture: None,
        };
        assert!(ValidatedPlaybackResolution::from_preflight(make_stream(403, None)).is_err());
        let fresh = ValidatedPlaybackResolution::from_preflight(make_stream(206, None))
            .expect("206 preflight is accepted");
        assert_eq!(fresh.expires_at, None);
        let expired = ValidatedPlaybackResolution::from_preflight(make_stream(200, Some(1_000)))
            .expect("200 preflight is accepted");
        assert!(expired.is_expired(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000)));
    }

    #[test]
    fn signature_timestamp_parser_matches_the_player_js_pattern() {
        assert_eq!(
            parse_signature_timestamp("var x={signatureTimestamp: 20347};"),
            Some(20347)
        );
        assert_eq!(parse_signature_timestamp("x.sts:20348;"), Some(20348));
        assert_eq!(parse_signature_timestamp("signatureTimestamp: 1234;"), None);
    }

    #[test]
    fn config_get_retry_statuses_are_limited_to_rate_limit_and_server_errors() {
        assert!(retryable_config_status(
            reqwest::StatusCode::TOO_MANY_REQUESTS
        ));
        assert!(retryable_config_status(
            reqwest::StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(!retryable_config_status(reqwest::StatusCode::FORBIDDEN));
        assert!(!retryable_config_status(reqwest::StatusCode::NOT_FOUND));
    }

    #[test]
    fn player_js_url_accepts_player_asset_and_context_fallback() {
        assert_eq!(
            player_js_url(&json!({"PLAYER_JS_URL":"/s/player/id/base.js"})),
            Some("/s/player/id/base.js".into())
        );
        assert_eq!(
            player_js_url(
                &json!({"WEB_PLAYER_CONTEXT_CONFIGS":{"WEB_PLAYER_CONTEXT_CONFIG_ID_KEVLAR_WATCH":{"jsUrl":"/s/player/id/context.js"}}})
            ),
            Some("/s/player/id/context.js".into())
        );
    }

    #[test]
    fn parses_nested_ytcfg_json_without_truncating_braces_in_strings() {
        let page = r#"<script>ytcfg.set({"INNERTUBE_CONTEXT":{"client":{"clientName":"MWEB"},"extra":{"value":"} and { are text"}},"INNERTUBE_CONTEXT_CLIENT_NAME":2});</script>"#;
        let config = parse_ytcfg(page).expect("ytcfg config should parse");
        assert_eq!(
            config.pointer("/INNERTUBE_CONTEXT/client/clientName"),
            Some(&json!("MWEB"))
        );
        assert_eq!(
            config.pointer("/INNERTUBE_CONTEXT/extra/value"),
            Some(&json!("} and { are text"))
        );
    }

    #[test]
    fn parses_anonymized_loose_ytatn_fixture_with_page_bound_challenge() {
        let page = include_str!("../../tests/fixtures/ytatn_anonymized.html");
        let attestation = parse_yt_at_n(page).expect("initial attestation should parse");
        assert_eq!(
            attestation.pointer("/bgChallenge/globalName"),
            Some(&json!("fixtureGlobal"))
        );
        assert_eq!(
            attestation.pointer("/bgChallenge/program"),
            Some(&json!("ANONYMIZED_PROGRAM"))
        );
    }

    #[test]
    fn detects_gvs_binding_experiment_without_assuming_video_id_binding() {
        assert!(has_video_id_gvs_binding(&json!({
            "WEB_PLAYER_CONTEXT_CONFIGS": {
                "WEB_PLAYER_CONTEXT_CONFIG_ID_KEVLAR_WATCH": {
                    "serializedExperimentFlags": "x=1&html5_generate_content_po_token=true&y=2"
                }
            }
        })));
        assert!(!has_video_id_gvs_binding(&json!({
            "WEB_PLAYER_CONTEXT_CONFIGS": {
                "WEB_PLAYER_CONTEXT_CONFIG_ID_KEVLAR_WATCH": {
                    "serializedExperimentFlags": "x=1&html5_generate_content_po_token=false&y=2"
                }
            }
        })));
        assert!(!has_video_id_gvs_binding(&json!({})));
    }

    #[tokio::test]
    async fn already_cancelled_resolution_stops_before_network_io() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let result = YouTubeStreamResolver::new(crate::music::YouTubeSession::default())
            .resolve(
                &TrackId("not-requested".into()),
                AudioQuality::Auto,
                cancellation,
            )
            .await;
        assert!(matches!(result, Err(StreamError::Cancelled)));
    }

    #[test]
    fn live_broadcasts_are_recognised_from_the_response_and_the_media_url() {
        let live = json!({"videoDetails": {"isLiveContent": true}});
        let vod = json!({"videoDetails": {"isLiveContent": false}});
        assert!(is_live_content(&live));
        assert!(!is_live_content(&vod));
        assert!(!is_live_content(&json!({})));
        assert!(is_live_media_url(
            "https://rr1.googlevideo.com/videoplayback?itag=140&live=1&ei=abc"
        ));
        assert!(!is_live_media_url(
            "https://rr1.googlevideo.com/videoplayback?itag=140&live=0&ei=abc"
        ));
        assert!(!is_live_media_url(
            "https://rr1.googlevideo.com/videoplayback?itag=140&notlive=1"
        ));
    }

    #[test]
    fn largest_thumbnail_picks_the_widest_listed_image() {
        let payload = json!({
            "videoDetails": {
                "thumbnail": {
                    "thumbnails": [
                        {"url": "https://i.example/small.jpg", "width": 120},
                        {"url": "https://i.example/large.jpg", "width": 1280},
                        {"url": "https://i.example/medium.jpg", "width": 480}
                    ]
                }
            }
        });
        assert_eq!(
            super::largest_thumbnail(&payload).as_deref(),
            Some("https://i.example/large.jpg")
        );
        assert_eq!(super::largest_thumbnail(&json!({"videoDetails": {}})), None);
    }

    #[test]
    fn player_js_cache_keeps_the_two_most_recent_versions() {
        let directory =
            std::env::temp_dir().join(format!("crabytube-player-js-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("test directory");
        let base = std::time::SystemTime::now() - std::time::Duration::from_secs(10_000);
        let fingerprint = "f".repeat(64);
        let keys: Vec<String> = ["oldest", "middle", "newest"]
            .iter()
            .map(|url| super::sha256_hex(url.as_bytes()))
            .collect();
        for (index, key) in keys.iter().enumerate() {
            for extension in ["js", "json"] {
                let path = directory.join(format!("{key}-{fingerprint}.{extension}"));
                std::fs::write(&path, b"x").expect("write test file");
                std::fs::File::options()
                    .write(true)
                    .open(&path)
                    .and_then(|file| {
                        file.set_modified(base + std::time::Duration::from_secs(index as u64 * 100))
                    })
                    .expect("set test mtime");
            }
        }
        super::trim_persisted_player_js_cache(&directory);
        let names: Vec<String> = std::fs::read_dir(&directory)
            .expect("read test directory")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 4, "two versions with two files each remain");
        assert!(names.iter().all(|name| !name.starts_with(&keys[0])));
        assert!(names.iter().any(|name| name.starts_with(&keys[1])));
        assert!(names.iter().any(|name| name.starts_with(&keys[2])));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
