//! The player configuration of an earlier launch, kept for six hours. Fetching it downloads the YouTube
//! pages it is read from, so a launch within the window skips those downloads. A saved configuration that
//! YouTube refuses is forgotten and fetched again by the resolver.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long a saved configuration stays usable.
pub const TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// The values a player runtime reads from the pages. The signature timestamp is not saved: the player
/// JavaScript that provides it is cached on its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedPlayerConfig {
    /// Unix seconds when the pages were fetched.
    pub saved_at: u64,
    /// The client profile name, so a configuration of one profile is never used for the other.
    pub profile: String,
    pub endpoint: String,
    pub origin: String,
    pub header_id: String,
    pub version: String,
    pub user_agent: Option<String>,
    pub player_js_url: Option<String>,
    pub context: Value,
    pub botguard_ytcfg_json: String,
    pub botguard_user_agent: String,
    pub homepage_challenge_json: Option<String>,
    pub event_id: Option<String>,
    /// Whether the GVS token is bound to the video ID rather than to the visitor.
    pub gvs_video_id: bool,
}

/// Serializes read-modify-write cycles on the file within this process.
static FILE_LOCK: Mutex<()> = Mutex::new(());

/// The file of `profile`. Each profile has its own, so the one used for live broadcasts does not
/// replace the music profile's. The music profile keeps the name the file has always had.
fn path(profile: &str) -> Option<PathBuf> {
    let mut path = crate::app_dirs::cache_dir()?;
    path.push("crabytube");
    path.push(file_name(profile));
    Some(path)
}

fn file_name(profile: &str) -> String {
    if profile == "WEB_REMIX" {
        return "player-config.json".into();
    }
    let tag: String = profile
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    format!("player-config-{tag}.json")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// A configuration is usable when it is not from the future and younger than the TTL.
pub fn is_fresh(saved_at: u64, now: u64) -> bool {
    saved_at <= now && now - saved_at < TTL.as_secs()
}

/// A damaged file reads as nothing saved, so the pages are fetched again.
fn parse(bytes: &[u8]) -> Option<SavedPlayerConfig> {
    serde_json::from_slice(bytes).ok()
}

/// The saved configuration of `profile`, if it is still fresh.
pub fn load(profile: &str) -> Option<SavedPlayerConfig> {
    let _guard = FILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let bytes = std::fs::read(path(profile)?).ok()?;
    parse(&bytes).filter(|saved| saved.profile == profile && is_fresh(saved.saved_at, now()))
}

/// Saves a configuration, replacing the one on disk. The file is written beside its target and renamed,
/// so a crash never leaves a half-written file behind.
pub fn store(saved: &SavedPlayerConfig) {
    let _guard = FILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(path) = path(&saved.profile) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = crate::storage::create_private_dir_all(parent);
    }
    let Ok(bytes) = serde_json::to_vec(saved) else {
        return;
    };
    let temporary = path.with_extension("json.tmp");
    if crate::playback::pot::write_private_file(&temporary, &bytes).is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}

/// Removes the saved configuration of `profile`, so the next start fetches its pages again.
pub fn forget(profile: &str) {
    let _guard = FILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(path) = path(profile) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(profile: &str, saved_at: u64) -> SavedPlayerConfig {
        SavedPlayerConfig {
            saved_at,
            profile: profile.into(),
            endpoint: "https://music.youtube.com/youtubei/v1/player".into(),
            origin: "https://music.youtube.com".into(),
            header_id: "67".into(),
            version: "1.0".into(),
            user_agent: None,
            player_js_url: Some("/s/player/abc/base.js".into()),
            context: serde_json::json!({"client": {"visitorData": "v"}}),
            botguard_ytcfg_json: "{}".into(),
            botguard_user_agent: "agent".into(),
            homepage_challenge_json: None,
            event_id: None,
            gvs_video_id: true,
        }
    }

    #[test]
    fn a_saved_configuration_is_fresh_for_six_hours_and_never_from_the_future() {
        let now = 1_000_000;
        assert!(is_fresh(now, now));
        assert!(is_fresh(now - TTL.as_secs() + 1, now));
        assert!(!is_fresh(now - TTL.as_secs(), now));
        assert!(!is_fresh(now + 60, now));
    }

    #[test]
    fn a_damaged_file_reads_as_nothing_saved() {
        assert!(parse(b"not json").is_none());
        assert!(parse(b"").is_none());
        assert!(parse(br#"{"saved_at": 1}"#).is_none());
    }

    #[test]
    fn a_saved_configuration_round_trips() {
        let saved = sample("MWEB", 42);
        let restored = parse(&serde_json::to_vec(&saved).unwrap()).expect("it parses back");
        assert_eq!(restored, saved);
    }

    #[test]
    fn store_load_and_forget_keep_one_configuration_per_profile() {
        // The test cache root is a temporary folder, so the real files are never touched.
        let profile = "TEST_PROFILE_ONLY";
        forget(profile);
        assert!(load(profile).is_none());
        store(&sample(profile, now()));
        let loaded = load(profile).expect("the fresh configuration is loaded");
        assert_eq!(loaded.profile, profile);
        assert!(load("ANOTHER_PROFILE").is_none());
        // Another profile's configuration is kept apart and survives this one's forgetting.
        store(&sample("TEST_OTHER_PROFILE", now()));
        forget(profile);
        assert!(load(profile).is_none());
        assert!(load("TEST_OTHER_PROFILE").is_some());
        forget("TEST_OTHER_PROFILE");
    }

    #[test]
    fn each_profile_has_its_own_file_and_the_music_one_keeps_its_name() {
        assert_eq!(file_name("WEB_REMIX"), "player-config.json");
        assert_eq!(file_name("MWEB"), "player-config-mweb.json");
        assert_eq!(file_name("../X Y"), "player-config-xy.json");
    }
}
