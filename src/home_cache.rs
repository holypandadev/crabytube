//! Small, anonymous Home snapshot so repeat launches can paint immediately.
//! It contains public catalog metadata only: no cookies, account data, or raw API payloads.

use crate::music::HomeShelf;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SHELVES: usize = 32;
const MAX_ITEMS_PER_SHELF: usize = 80;
static SAVE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u8,
    shelves: Vec<HomeShelf>,
}

/// One snapshot per catalog language, so switching language never shows the old one.
fn path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!(
            "crabytube/home-v1-{}.json",
            crate::music::content_language()
        ))
}

fn read(path: &Path) -> Option<Vec<HomeShelf>> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() == 0 || metadata.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let mut snapshot: Snapshot = serde_json::from_slice(&bytes).ok()?;
    if snapshot.version != 1 {
        return None;
    }
    snapshot.shelves.truncate(MAX_SHELVES);
    for shelf in &mut snapshot.shelves {
        shelf.items.truncate(MAX_ITEMS_PER_SHELF);
    }
    (!snapshot.shelves.is_empty()).then_some(snapshot.shelves)
}

/// Loads the last public Home feed. Callers should revalidate it from the service in the
/// background and keep showing this snapshot if that refresh fails.
pub fn load() -> Option<Vec<HomeShelf>> {
    read(&path())
}

pub fn save(shelves: &[HomeShelf]) {
    if shelves.is_empty() {
        return;
    }
    let mut shelves = shelves.to_vec();
    shelves.truncate(MAX_SHELVES);
    for shelf in &mut shelves {
        shelf.items.truncate(MAX_ITEMS_PER_SHELF);
    }
    let snapshot = Snapshot {
        version: 1,
        shelves,
    };
    let Ok(bytes) = serde_json::to_vec(&snapshot) else {
        return;
    };
    if bytes.len() as u64 > MAX_BYTES {
        return;
    }

    save_at(&path(), &snapshot);
}

fn save_at(path: &Path, snapshot: &Snapshot) {
    // All background Home refreshes target the same .tmp name. Serialize the
    // replace so overlapping refreshes cannot publish a truncated snapshot.
    let _guard = SAVE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Ok(bytes) = serde_json::to_vec(snapshot) else {
        return;
    };
    if bytes.len() as u64 > MAX_BYTES {
        return;
    }
    let path = path.to_path_buf();
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, bytes).is_ok() {
        let _ = std::fs::rename(temporary, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{HomeItem, HomeItemKind};

    #[test]
    fn public_home_snapshot_round_trips() {
        let path =
            std::env::temp_dir().join(format!("crabytube-home-cache-{}.json", std::process::id()));
        let value = Snapshot {
            version: 1,
            shelves: vec![HomeShelf {
                title: "Mixes".into(),
                items: vec![HomeItem {
                    title: "Salsa Mix".into(),
                    subtitle: "Public playlist".into(),
                    thumbnail_url: Some("https://example.invalid/art.jpg".into()),
                    kind: HomeItemKind::Mix,
                    browse_id: Some("RDAMVMfixture".into()),
                    browse_target: None,
                    track: None,
                    duration: None,
                }],
            }],
        };
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(read(&path).unwrap(), value.shelves);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unknown_home_cache_version_is_ignored() {
        let path = std::env::temp_dir().join(format!(
            "crabytube-home-cache-invalid-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, br#"{"version":2,"shelves":[]}"#).unwrap();
        assert!(read(&path).is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn concurrent_home_writes_leave_one_complete_atomic_snapshot() {
        let path =
            std::env::temp_dir().join(format!("crabytube-home-race-{}.json", std::process::id()));
        let make_snapshot = |title: &str| Snapshot {
            version: 1,
            shelves: vec![HomeShelf {
                title: title.into(),
                items: vec![],
            }],
        };
        let first_path = path.clone();
        let first = std::thread::spawn(move || save_at(&first_path, &make_snapshot("first")));
        let second_path = path.clone();
        let second = std::thread::spawn(move || save_at(&second_path, &make_snapshot("second")));
        first.join().unwrap();
        second.join().unwrap();
        let saved = read(&path).expect("one complete snapshot must remain");
        assert!(matches!(saved[0].title.as_str(), "first" | "second"));
        let _ = std::fs::remove_file(path);
    }
}
