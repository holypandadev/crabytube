//! Removes temporary files left behind when a cache write was interrupted.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

/// Temp files younger than this may belong to a write in progress, so they are kept.
const MIN_AGE: Duration = Duration::from_secs(10 * 60);

/// Names produced by the atomic writers: `.tmp-*` for artwork, `*.tmp` for Home
/// metadata and player-JS entries, and `.tmpXXXXXX` for streamed audio buffers.
pub fn is_cache_temp_name(name: &str) -> bool {
    name.starts_with(".tmp") || name.ends_with(".tmp")
}

/// Cache directories that receive atomic writes. Only direct children are swept.
pub fn cache_directories() -> Vec<PathBuf> {
    let Some(cache) = crate::app_dirs::cache_dir() else {
        return Vec::new();
    };
    let root = cache.join("crabytube");
    vec![
        root.clone(),
        root.join("artwork"),
        root.join("player-js"),
        root.join("stream"),
    ]
}

/// Deletes stale temp files in `directories` and returns how many were removed.
pub fn sweep_temp_files(directories: &[PathBuf], min_age: Duration) -> usize {
    directories
        .iter()
        .map(|directory| sweep_directory(directory, min_age))
        .sum()
}

fn sweep_directory(directory: &Path, min_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_cache_temp_name(name) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let old_enough = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= min_age);
        if metadata.is_file() && old_enough && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Runs the startup sweep on a background thread so the window opens without waiting for it.
pub fn sweep_in_background() {
    let _ = std::thread::Builder::new()
        .name("crabytube-temp-sweep".into())
        .spawn(|| {
            let removed = sweep_temp_files(&cache_directories(), MIN_AGE);
            if removed > 0 {
                tracing::info!(removed, "removed stale cache temp files");
            }
            // Only the first instance gets here, so no update of a running app is downloading.
            crate::updater::remove_old_downloads();
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_atomic_writer_temp_names_are_recognized() {
        assert!(is_cache_temp_name(".tmp-1-2.png"));
        assert!(is_cache_temp_name("home-v1.json.tmp"));
        assert!(is_cache_temp_name("abc.3.tmp"));
        assert!(is_cache_temp_name(".tmpAb12Cd"));
        assert!(!is_cache_temp_name("cover-0.png"));
        assert!(!is_cache_temp_name("state.json"));
    }

    #[test]
    fn sweep_removes_only_stale_temp_files() {
        let dir =
            std::env::temp_dir().join(format!("crabytube-temp-sweep-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".tmp-1-2.png"), b"x").unwrap();
        std::fs::write(dir.join("home-v1.json.tmp"), b"x").unwrap();
        std::fs::write(dir.join("cover-0.png"), b"x").unwrap();

        // A zero minimum age treats every temp file as stale.
        let removed = sweep_temp_files(std::slice::from_ref(&dir), Duration::ZERO);

        assert_eq!(removed, 2);
        assert!(!dir.join(".tmp-1-2.png").exists());
        assert!(!dir.join("home-v1.json.tmp").exists());
        assert!(dir.join("cover-0.png").is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sweep_keeps_recent_temp_files_that_may_be_in_use() {
        let dir =
            std::env::temp_dir().join(format!("crabytube-temp-recent-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("active.tmp"), b"x").unwrap();

        let removed = sweep_temp_files(std::slice::from_ref(&dir), MIN_AGE);

        assert_eq!(removed, 0);
        assert!(dir.join("active.tmp").is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
