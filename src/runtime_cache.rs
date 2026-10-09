//! WEB_REMIX values kept between launches. Fetching them downloads the music.youtube.com
//! page first, so they live in one private file for six hours. Innertube refusing a request
//! (HTTP 403) drops them.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long saved values stay usable before they are fetched again.
pub const TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// WEB_REMIX values from the music.youtube.com page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebValues {
    pub api_key: String,
    pub client_id: String,
    pub client_version: String,
    pub visitor_data: Option<String>,
    pub context: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Stamped<T> {
    /// Unix seconds when the values were fetched.
    saved_at: u64,
    value: T,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedRuntime {
    #[serde(default)]
    web: Option<Stamped<WebValues>>,
    /// The visitor ID the player requests and the BotGuard tokens are bound to.
    #[serde(default)]
    visitor: Option<Stamped<String>>,
}

/// Serializes read-modify-write cycles on the file within this process.
static FILE_LOCK: Mutex<()> = Mutex::new(());

fn path() -> Option<PathBuf> {
    let mut path = dirs::cache_dir()?;
    path.push("crabytube");
    path.push("runtime.json");
    Some(path)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// A stamp is usable when it is not in the future and is younger than the TTL.
fn is_fresh(saved_at: u64, now: u64) -> bool {
    saved_at <= now && now - saved_at < TTL.as_secs()
}

fn parse(bytes: &[u8]) -> SavedRuntime {
    serde_json::from_slice(bytes).unwrap_or_default()
}

fn read() -> SavedRuntime {
    path()
        .and_then(|path| std::fs::read(path).ok())
        .map(|bytes| parse(&bytes))
        .unwrap_or_default()
}

fn write(saved: &SavedRuntime) {
    let Some(path) = path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(bytes) = serde_json::to_vec(saved) else {
        return;
    };
    // Write beside the target and rename, so a crash never leaves a half-written file.
    let temporary = path.with_extension("json.tmp");
    if crate::playback::pot::write_private_file(&temporary, &bytes).is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}

fn locked<T>(action: impl FnOnce() -> T) -> T {
    let _guard = FILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    action()
}

/// WEB_REMIX values, if saved within the TTL.
pub fn web() -> Option<WebValues> {
    locked(|| {
        read()
            .web
            .filter(|saved| is_fresh(saved.saved_at, now()))
            .map(|saved| saved.value)
    })
}

pub fn store_web(values: WebValues) {
    locked(|| {
        let mut saved = read();
        saved.web = Some(Stamped {
            saved_at: now(),
            value: values,
        });
        write(&saved);
    });
}

/// Visitor ID, if saved within the TTL.
pub fn visitor() -> Option<String> {
    locked(|| {
        read()
            .visitor
            .filter(|saved| is_fresh(saved.saved_at, now()))
            .map(|saved| saved.value)
    })
}

pub fn store_visitor(visitor: &str) {
    locked(|| {
        let mut saved = read();
        saved.visitor = Some(Stamped {
            saved_at: now(),
            value: visitor.to_owned(),
        });
        write(&saved);
    });
}

pub fn forget_visitor() {
    locked(|| {
        let mut saved = read();
        saved.visitor = None;
        write(&saved);
    });
}

pub fn forget_web() {
    locked(|| {
        let mut saved = read();
        saved.web = None;
        write(&saved);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_values_are_fresh_for_six_hours_and_never_from_the_future() {
        let now = 1_000_000;
        assert!(is_fresh(now, now));
        assert!(is_fresh(now - TTL.as_secs() + 1, now));
        assert!(!is_fresh(now - TTL.as_secs(), now));
        assert!(!is_fresh(now + 60, now));
    }

    #[test]
    fn damaged_or_missing_files_read_as_nothing_saved() {
        assert!(parse(b"not json").web.is_none());
        assert!(parse(b"").web.is_none());
        assert!(parse(br#"{"web":null}"#).web.is_none());
    }

    #[test]
    fn saved_values_round_trip() {
        let saved = SavedRuntime {
            visitor: None,
            web: Some(Stamped {
                saved_at: 42,
                value: WebValues {
                    api_key: "key".into(),
                    client_id: "67".into(),
                    client_version: "1.0".into(),
                    visitor_data: Some("visitor".into()),
                    context: serde_json::json!({"client": {}}),
                },
            }),
        };
        let restored = parse(&serde_json::to_vec(&saved).unwrap());
        assert_eq!(restored.web.unwrap().value.api_key, "key");
    }
}
