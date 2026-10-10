use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{io::AsyncWriteExt, process::Command, sync::Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const EXPIRY_SAFETY: i64 = 60;
/// The requested video plus the next ten queue entries share one helper run.
const MAX_BATCH_VIDEOS: usize = 11;
/// Helper protocol version; the helper and the app must agree.
const PROTOCOL_VERSION: u16 = 2;
const PERSISTED_VERSION: u8 = 1;
const POT_FILE_NAME: &str = "pot.json";

/// Video IDs the playback queue expects next. The UI publishes them, and the next helper
/// run mints content tokens for them too, so track changes need no new helper run.
static UPCOMING_VIDEO_IDS: StdMutex<Vec<String>> = StdMutex::new(Vec::new());

/// Publishes the queue's upcoming video IDs for token batching.
pub fn set_upcoming_video_ids(ids: Vec<String>) {
    if let Ok(mut upcoming) = UPCOMING_VIDEO_IDS.lock() {
        *upcoming = ids;
    }
}

fn upcoming_video_ids() -> Vec<String> {
    UPCOMING_VIDEO_IDS
        .lock()
        .map(|upcoming| upcoming.clone())
        .unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoToken {
    pub value: String,
    pub valid_until: i64,
}

impl PoToken {
    /// The placeholder for a token the request did not ask for: empty, so it is never stored.
    fn not_requested() -> Self {
        Self {
            value: String::new(),
            valid_until: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoTokens {
    pub session: PoToken,
    /// Empty unless the request asked for it (`PoTokenRequest::needs_content`).
    pub content: PoToken,
    /// True when the tokens came from the saved cache rather than a helper run.
    pub reused: bool,
}

#[derive(Clone, Debug)]
pub struct PoTokenRequest {
    pub visitor_data: String,
    pub video_id: String,
    pub user_agent: String,
    pub ytcfg_json: String,
    pub homepage_challenge_json: Option<String>,
    pub event_id: Option<String>,
    /// The content token is bound to the video. Only the VideoId binding uses it, so other
    /// requests skip the BotGuard run for it.
    pub needs_content: bool,
}

/// Which tokens a request still has to mint, given which ones are cached: (session, content).
fn tokens_to_mint(session_cached: bool, content_cached: bool, needs_content: bool) -> (bool, bool) {
    (!session_cached, needs_content && !content_cached)
}

#[derive(Debug, Error)]
pub enum PoTokenError {
    #[error("YouTube session visitor data is unavailable")]
    MissingVisitorData,
    #[error("BotGuard helper could not be started: {0}")]
    Helper(String),
    /// The helper is not in the installation at all.
    #[error("{0}")]
    HelperMissing(String),
    #[error("BotGuard helper returned an invalid response: {0}")]
    Protocol(String),
    #[error("operation cancelled")]
    Cancelled,
}

#[async_trait]
pub trait PoTokenProvider: Send + Sync {
    async fn tokens(
        &self,
        request: PoTokenRequest,
        cancel: CancellationToken,
    ) -> Result<PoTokens, PoTokenError>;

    /// Drops every token bound to `visitor`, after YouTube refused one of them.
    async fn forget_visitor(&self, _visitor: &str) {}
}

#[derive(Default)]
pub struct PoTokenCache {
    sessions: HashMap<String, PoToken>,
    contents: HashMap<(String, String), PoToken>,
}

impl PoTokenCache {
    pub fn get(
        &mut self,
        visitor: &str,
        video: &str,
        now: i64,
    ) -> (Option<PoToken>, Option<PoToken>) {
        self.sessions
            .retain(|_, token| token.valid_until - EXPIRY_SAFETY > now);
        self.contents
            .retain(|_, token| token.valid_until - EXPIRY_SAFETY > now);
        (
            self.sessions.get(visitor).cloned(),
            self.contents
                .get(&(visitor.to_owned(), video.to_owned()))
                .cloned(),
        )
    }

    pub fn insert(&mut self, visitor: String, video: String, tokens: PoTokens) {
        if !tokens.session.value.is_empty() {
            self.sessions.insert(visitor.clone(), tokens.session);
        }
        if !tokens.content.value.is_empty() {
            self.contents.insert((visitor, video), tokens.content);
        }
    }

    fn forget_visitor(&mut self, visitor: &str) {
        self.sessions.remove(visitor);
        self.contents.retain(|(bound, _), _| bound != visitor);
    }

    fn insert_content(&mut self, visitor: &str, video: String, token: PoToken) {
        if !token.value.is_empty() {
            self.contents.insert((visitor.to_owned(), video), token);
        }
    }

    /// Tokens still usable at `now`, in the on-disk format.
    fn to_persisted(&self, now: i64) -> PersistedTokens {
        let live = |token: &PoToken| token.valid_until - EXPIRY_SAFETY > now;
        PersistedTokens {
            version: PERSISTED_VERSION,
            sessions: self
                .sessions
                .iter()
                .filter(|(_, token)| live(token))
                .map(|(visitor, token)| PersistedSession {
                    visitor: visitor.clone(),
                    value: token.value.clone(),
                    valid_until: token.valid_until,
                })
                .collect(),
            contents: self
                .contents
                .iter()
                .filter(|(_, token)| live(token))
                .map(|((visitor, video), token)| PersistedContent {
                    visitor: visitor.clone(),
                    video: video.clone(),
                    value: token.value.clone(),
                    valid_until: token.valid_until,
                })
                .collect(),
        }
    }

    fn from_persisted(persisted: PersistedTokens, now: i64) -> Self {
        let mut cache = Self::default();
        if persisted.version != PERSISTED_VERSION {
            return cache;
        }
        let live = |valid_until: i64| valid_until - EXPIRY_SAFETY > now;
        for session in persisted.sessions {
            if live(session.valid_until) && !session.value.is_empty() {
                cache.sessions.insert(
                    session.visitor,
                    PoToken {
                        value: session.value,
                        valid_until: session.valid_until,
                    },
                );
            }
        }
        for content in persisted.contents {
            if live(content.valid_until) && !content.value.is_empty() {
                cache.contents.insert(
                    (content.visitor, content.video),
                    PoToken {
                        value: content.value,
                        valid_until: content.valid_until,
                    },
                );
            }
        }
        cache
    }
}

/// On-disk token cache (`pot.json`). It holds bearer-like tokens, so it is written 0600.
#[derive(Serialize, Deserialize)]
struct PersistedTokens {
    version: u8,
    sessions: Vec<PersistedSession>,
    contents: Vec<PersistedContent>,
}

#[derive(Serialize, Deserialize)]
struct PersistedSession {
    visitor: String,
    value: String,
    valid_until: i64,
}

#[derive(Serialize, Deserialize)]
struct PersistedContent {
    visitor: String,
    video: String,
    value: String,
    valid_until: i64,
}

fn load_persisted_cache(path: &Path, now: i64) -> PoTokenCache {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PersistedTokens>(&bytes).ok())
        .map(|persisted| PoTokenCache::from_persisted(persisted, now))
        .unwrap_or_default()
}

fn save_persisted_cache(path: &Path, cache: &PoTokenCache, now: i64) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    let data = serde_json::to_vec(&cache.to_persisted(now))
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    write_private_file(&temporary, &data)?;
    std::fs::rename(&temporary, path)
}

#[cfg(unix)]
pub(crate) fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(data)
}

#[cfg(not(unix))]
pub(crate) fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

/// Cache key for one visitor and video pair.
type VideoKey = (String, String);

pub struct BotguardHelper {
    cache: Mutex<PoTokenCache>,
    /// One generation gate per in-flight (visitor, video) pair.
    inflight: Mutex<HashMap<VideoKey, Arc<Mutex<()>>>>,
    snapshot_path: PathBuf,
    pot_path: PathBuf,
}

impl BotguardHelper {
    pub fn new() -> Self {
        let mut path = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
        path.push("crabytube");
        path.push("botguard-snapshot.bin");
        Self::with_snapshot_path(path)
    }

    /// Create a helper with an isolated snapshot path for diagnostics. Tokens persist beside it.
    pub fn with_snapshot_path(snapshot_path: PathBuf) -> Self {
        let pot_path = snapshot_path.with_file_name(POT_FILE_NAME);
        let cache = load_persisted_cache(&pot_path, unix_now());
        Self {
            cache: Mutex::new(cache),
            inflight: Mutex::new(HashMap::new()),
            snapshot_path,
            pot_path,
        }
    }

    /// Runs the helper once for the session token and, when requested, content tokens for
    /// `video_ids`. Returns the requested video's token plus tokens for the other ids.
    async fn generate(
        &self,
        request: &PoTokenRequest,
        needs: (bool, bool),
        video_ids: &[String],
        cancel: CancellationToken,
    ) -> Result<(PoTokens, Vec<(String, PoToken)>), PoTokenError> {
        // Keep the app's heap lean while the helper's large transient allocations run.
        let _helper_window = crate::memory::HelperWindow::enter();
        #[derive(Serialize)]
        struct Request<'a> {
            protocol_version: u16,
            visitor_data: &'a str,
            video_id: &'a str,
            video_ids: Vec<&'a str>,
            user_agent: &'a str,
            ytcfg_json: &'a str,
            homepage_challenge_json: Option<&'a str>,
            event_id: Option<&'a str>,
            snapshot_path: &'a PathBuf,
            need_session: bool,
            need_content: bool,
        }
        #[derive(Deserialize)]
        struct Response {
            protocol_version: u16,
            session_po_token: Option<String>,
            #[serde(default)]
            content_po_tokens: Vec<String>,
            valid_until: i64,
            from_snapshot: bool,
            cold_start_ms: u128,
            token_generation_ms: u128,
            challenge_source: String,
        }

        let (need_session, need_content) = needs;
        let mut command = helper_command()?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|error| PoTokenError::Helper(error.to_string()))?;
        let batch: Vec<&str> = if need_content {
            video_ids.iter().map(String::as_str).collect()
        } else {
            Vec::new()
        };
        let request = serde_json::to_vec(&Request {
            protocol_version: PROTOCOL_VERSION,
            visitor_data: &request.visitor_data,
            video_id: &request.video_id,
            video_ids: batch,
            user_agent: &request.user_agent,
            ytcfg_json: &request.ytcfg_json,
            homepage_challenge_json: request.homepage_challenge_json.as_deref(),
            event_id: request.event_id.as_deref(),
            snapshot_path: &self.snapshot_path,
            need_session,
            need_content,
        })
        .map_err(|error| PoTokenError::Protocol(error.to_string()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| PoTokenError::Helper("stdin unavailable".into()))?;
        // The write is bounded like the wait: a helper that stops reading must not hang the request.
        let written = tokio::select! {
            _ = cancel.cancelled() => return Err(PoTokenError::Cancelled),
            result = tokio::time::timeout(BOTGUARD_HELPER_TIMEOUT, stdin.write_all(&request)) => result,
        };
        written
            .map_err(|_| PoTokenError::Helper("BotGuard helper did not read its input".into()))?
            .map_err(|error| PoTokenError::Helper(error.to_string()))?;
        drop(stdin);
        // A stuck helper is killed when the timed-out future drops the child (kill_on_drop).
        let output = tokio::select! {
            _ = cancel.cancelled() => return Err(PoTokenError::Cancelled),
            result = tokio::time::timeout(BOTGUARD_HELPER_TIMEOUT, child.wait_with_output()) => result
                .map_err(|_| PoTokenError::Helper("BotGuard helper timed out".into()))?
                .map_err(|error| PoTokenError::Helper(error.to_string()))?,
        };
        if !output.status.success() {
            #[derive(Deserialize)]
            struct ErrorResponse {
                error: Option<String>,
            }
            let message = serde_json::from_slice::<ErrorResponse>(&output.stdout)
                .ok()
                .and_then(|e| e.error)
                .unwrap_or_else(|| format!("helper exited with {}", output.status));
            return Err(PoTokenError::Helper(message));
        }
        let response: Response = serde_json::from_slice(&output.stdout)
            .map_err(|error| PoTokenError::Protocol(error.to_string()))?;
        if response.protocol_version != PROTOCOL_VERSION
            || (need_session
                && response
                    .session_po_token
                    .as_ref()
                    .is_none_or(String::is_empty))
            || (need_content
                && (response.content_po_tokens.len() != video_ids.len()
                    || response.content_po_tokens.iter().any(String::is_empty)))
        {
            return Err(PoTokenError::Protocol(
                "missing or unsupported token response".into(),
            ));
        }
        info!(
            from_snapshot = response.from_snapshot,
            cold_start_ms = response.cold_start_ms,
            token_generation_ms = response.token_generation_ms,
            token_expiry = response.valid_until,
            challenge_source = %response.challenge_source,
            session_token_length = response.session_po_token.as_ref().map_or(0, String::len),
            content_tokens = response.content_po_tokens.len(),
            "BotGuard tokens generated"
        );
        let valid_until = response.valid_until;
        let session = PoToken {
            value: response.session_po_token.unwrap_or_default(),
            valid_until,
        };
        let mut content_tokens = response
            .content_po_tokens
            .into_iter()
            .map(|value| PoToken { value, valid_until });
        let content = content_tokens.next().unwrap_or(PoToken {
            value: String::new(),
            valid_until,
        });
        let extra = video_ids
            .iter()
            .skip(1)
            .cloned()
            .zip(content_tokens)
            .collect();
        Ok((
            PoTokens {
                session,
                content,
                reused: false,
            },
            extra,
        ))
    }

    /// The requested video first, then the queue's upcoming entries that have no token yet.
    async fn batch_video_ids(&self, visitor: &str, video: &str, now: i64) -> Vec<String> {
        let mut cache = self.cache.lock().await;
        let mut ids = vec![video.to_owned()];
        for upcoming in upcoming_video_ids() {
            if ids.len() >= MAX_BATCH_VIDEOS {
                break;
            }
            if ids.contains(&upcoming) || cache.get(visitor, &upcoming, now).1.is_some() {
                continue;
            }
            ids.push(upcoming);
        }
        ids
    }
}

impl Default for BotguardHelper {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PoTokenProvider for BotguardHelper {
    async fn forget_visitor(&self, visitor: &str) {
        let mut cache = self.cache.lock().await;
        cache.forget_visitor(visitor);
        if let Err(error) = save_persisted_cache(&self.pot_path, &cache, unix_now()) {
            warn!(%error, "could not save BotGuard tokens");
        }
    }

    async fn tokens(
        &self,
        request: PoTokenRequest,
        cancel: CancellationToken,
    ) -> Result<PoTokens, PoTokenError> {
        if request.visitor_data.is_empty() {
            return Err(PoTokenError::MissingVisitorData);
        }
        // Single-flight per (visitor, video): identical requests wait for one helper run
        // instead of starting two. The cache lock is never held across the helper.
        let key = (request.visitor_data.clone(), request.video_id.clone());
        let gate = {
            let mut inflight = self.inflight.lock().await;
            inflight.retain(|_, gate| Arc::strong_count(gate) > 1);
            Arc::clone(inflight.entry(key.clone()).or_default())
        };
        let _turn = gate.lock().await;

        let now = unix_now();
        let (session, content) =
            self.cache
                .lock()
                .await
                .get(&request.visitor_data, &request.video_id, now);
        let (mint_session, mint_content) =
            tokens_to_mint(session.is_some(), content.is_some(), request.needs_content);
        if let (Some(session), false, false) = (&session, mint_session, mint_content) {
            info!("reused cached BotGuard tokens");
            return Ok(PoTokens {
                session: session.clone(),
                content: content.clone().unwrap_or_else(PoToken::not_requested),
                reused: true,
            });
        }
        // Only a content token is bound to the video, so only it needs the batch of videos.
        let video_ids = if !request.needs_content {
            Vec::new()
        } else if content.is_none() {
            self.batch_video_ids(&request.visitor_data, &request.video_id, now)
                .await
        } else {
            vec![request.video_id.clone()]
        };
        let (generated, extra) = self
            .generate(&request, (mint_session, mint_content), &video_ids, cancel)
            .await?;
        let session =
            session.or_else(|| (!generated.session.value.is_empty()).then_some(generated.session));
        let content = if request.needs_content {
            content.or_else(|| (!generated.content.value.is_empty()).then_some(generated.content))
        } else {
            Some(PoToken::not_requested())
        };
        let (Some(session), Some(content)) = (session, content) else {
            return Err(PoTokenError::Protocol(
                "helper omitted a requested token".into(),
            ));
        };
        if session.valid_until <= now || (request.needs_content && content.valid_until <= now) {
            warn!("BotGuard returned an already expired token");
            return Err(PoTokenError::Protocol("token already expired".into()));
        }
        let tokens = PoTokens {
            session,
            content,
            reused: false,
        };
        let mut cache = self.cache.lock().await;
        cache.insert(
            request.visitor_data.clone(),
            request.video_id.clone(),
            tokens.clone(),
        );
        for (video, token) in extra {
            cache.insert_content(&request.visitor_data, video, token);
        }
        if let Err(error) = save_persisted_cache(&self.pot_path, &cache, now) {
            warn!(%error, "could not save BotGuard tokens");
        }
        Ok(tokens)
    }
}

fn helper_command() -> Result<Command, PoTokenError> {
    // Overrides are for diagnostics builds only.
    #[cfg(feature = "diagnostics")]
    if let Some(path) = std::env::var_os("CRABYTUBE_BOTGUARD_HELPER") {
        return Ok(Command::new(path));
    }
    let name = if cfg!(windows) {
        "botguard-helper.exe"
    } else {
        "botguard-helper"
    };
    if let Ok(current) = std::env::current_exe()
        && let Some(path) = current
            .parent()
            .map(|parent| parent.join(name))
            .filter(|path| path.is_file())
    {
        return Ok(Command::new(path));
    }
    // Never fall back to $PATH: a helper found there would receive our request.
    Err(PoTokenError::HelperMissing(format!(
        "{name} is not installed next to the application executable"
    )))
}

/// Upper bound for one BotGuard helper run before it is killed.
const BOTGUARD_HELPER_TIMEOUT: Duration = Duration::from_secs(20);

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
        .min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::{PersistedTokens, PoToken, PoTokenCache, PoTokens};

    fn pair(expiry: i64) -> PoTokens {
        PoTokens {
            session: PoToken {
                value: "session".into(),
                valid_until: expiry,
            },
            content: PoToken {
                value: "content".into(),
                valid_until: expiry,
            },
            reused: false,
        }
    }

    #[test]
    fn session_and_content_cache_keys_are_independent_and_expire_early() {
        let mut cache = PoTokenCache::default();
        cache.insert("visitor".into(), "video-a".into(), pair(1000));
        assert!(cache.get("visitor", "video-a", 100).0.is_some());
        assert!(cache.get("visitor", "video-b", 100).1.is_none());
        assert!(cache.get("other", "video-a", 100).0.is_none());
        assert!(cache.get("visitor", "video-a", 950).0.is_none());
    }

    #[test]
    fn persisted_tokens_round_trip_and_drop_expired_entries() {
        let mut cache = PoTokenCache::default();
        cache.insert("visitor".into(), "video-a".into(), pair(1000));
        cache.insert_content(
            "visitor",
            "video-b".into(),
            PoToken {
                value: "stale".into(),
                valid_until: 150,
            },
        );

        let persisted = cache.to_persisted(100);
        assert_eq!(persisted.sessions.len(), 1);
        assert_eq!(persisted.contents.len(), 1);

        let mut restored = PoTokenCache::from_persisted(persisted, 100);
        assert!(restored.get("visitor", "video-a", 100).1.is_some());
        assert!(restored.get("visitor", "video-b", 100).1.is_none());
    }

    #[tokio::test]
    async fn batch_covers_the_requested_video_and_uncached_upcoming_entries() {
        let dir = std::env::temp_dir().join(format!("crabytube-pot-batch-{}", std::process::id()));
        let helper = super::BotguardHelper::with_snapshot_path(dir.join("snapshot.bin"));
        helper.cache.lock().await.insert_content(
            "visitor",
            "cached".into(),
            PoToken {
                value: "token".into(),
                valid_until: i64::MAX / 2,
            },
        );
        super::set_upcoming_video_ids(vec!["cached".into(), "next".into(), "after".into()]);

        let ids = helper
            .batch_video_ids("visitor", "now", super::unix_now())
            .await;

        super::set_upcoming_video_ids(Vec::new());
        assert_eq!(ids, vec!["now", "next", "after"]);
    }

    #[test]
    fn unknown_persisted_version_is_ignored() {
        let persisted = PersistedTokens {
            version: 99,
            sessions: Vec::new(),
            contents: Vec::new(),
        };
        let mut restored = PoTokenCache::from_persisted(persisted, 0);
        assert!(restored.get("visitor", "video-a", 0).1.is_none());
    }

    #[test]
    fn a_request_mints_only_the_tokens_it_needs() {
        use super::tokens_to_mint;
        // Visitor-data binding, nothing cached: only the session token is minted.
        assert_eq!(tokens_to_mint(false, false, false), (true, false));
        // Video binding, nothing cached: both tokens are minted.
        assert_eq!(tokens_to_mint(false, false, true), (true, true));
        // Session cached: a request that does not use the content token mints nothing.
        assert_eq!(tokens_to_mint(true, false, false), (false, false));
        // Session cached but the video binding still needs its content token.
        assert_eq!(tokens_to_mint(true, false, true), (false, true));
        assert_eq!(tokens_to_mint(true, true, true), (false, false));
    }
}
