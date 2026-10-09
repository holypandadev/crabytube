//! Disk-cache accounting and selective cleanup for the Storage settings.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const MAX_ARTWORK_DISK_BYTES: u64 = crate::artwork::MAX_DISK_ARTWORK_BYTES;
pub const MAX_LYRICS_DISK_BYTES: u64 = crate::lyrics::MAX_LYRICS_BYTES;
pub const MAX_HOME_DISK_BYTES: u64 = crate::home_cache::MAX_BYTES;
pub const MAX_PLAYER_JS_DISK_BYTES: u64 = crate::playback::MAX_PERSISTED_PREPROCESSED_BYTES
    * crate::playback::MAX_PERSISTED_PLAYER_JS_ENTRIES as u64;
pub const MAX_BOTGUARD_DISK_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_BOTGUARD_SNAPSHOTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheKind {
    Artwork,
    Lyrics,
    Home,
    PlayerJs,
    BotGuard,
}

impl CacheKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Artwork => "artwork cache",
            Self::Lyrics => "lyrics cache",
            Self::Home => "Home metadata cache",
            Self::PlayerJs => "player JS disk cache",
            Self::BotGuard => "BotGuard disk snapshots",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub artwork_bytes: u64,
    pub lyrics_bytes: u64,
    pub home_bytes: u64,
    pub player_js_bytes: u64,
    pub botguard_bytes: u64,
}

impl CacheUsage {
    pub fn total_bytes(self) -> u64 {
        self.artwork_bytes
            .saturating_add(self.lyrics_bytes)
            .saturating_add(self.home_bytes)
            .saturating_add(self.player_js_bytes)
            .saturating_add(self.botguard_bytes)
    }
}

fn app_cache_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("crabytube")
}

pub fn disk_cache_usage() -> CacheUsage {
    disk_cache_usage_at(&app_cache_root())
}

fn disk_cache_usage_at(root: &Path) -> CacheUsage {
    CacheUsage {
        artwork_bytes: directory_bytes(&root.join("artwork")),
        lyrics_bytes: directory_bytes(&root.join("lyrics")),
        home_bytes: home_snapshots(root)
            .iter()
            .map(|path| file_bytes(path))
            .sum(),
        player_js_bytes: directory_bytes(&root.join("player-js")),
        botguard_bytes: botguard_snapshot_bytes(root),
    }
}

pub fn clear_cache(kind: CacheKind) -> io::Result<u64> {
    clear_cache_at(&app_cache_root(), kind)
}

/// Home snapshots: the legacy `home-v1.json` and one `home-v1-<lang>.json` per language.
fn home_snapshots(root: &Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("home-v1") && name.ends_with(".json"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn clear_cache_at(root: &Path, kind: CacheKind) -> io::Result<u64> {
    let bytes = match kind {
        CacheKind::Artwork => directory_bytes(&root.join("artwork")),
        CacheKind::Lyrics => directory_bytes(&root.join("lyrics")),
        CacheKind::Home => home_snapshots(root)
            .iter()
            .map(|path| file_bytes(path))
            .sum(),
        CacheKind::PlayerJs => directory_bytes(&root.join("player-js")),
        CacheKind::BotGuard => botguard_snapshot_bytes(root),
    };

    match kind {
        CacheKind::Artwork => remove_dir_if_present(&root.join("artwork"))?,
        CacheKind::Lyrics => remove_dir_if_present(&root.join("lyrics"))?,
        CacheKind::Home => {
            for path in home_snapshots(root) {
                remove_file_if_present(&path)?;
            }
        }
        CacheKind::PlayerJs => remove_dir_if_present(&root.join("player-js"))?,
        CacheKind::BotGuard => {
            for entry in botguard_entries(root)? {
                fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(bytes)
}

fn remove_dir_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn file_bytes(path: &Path) -> u64 {
    fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map_or(0, |metadata| metadata.len())
}

fn directory_bytes(path: &Path) -> u64 {
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir()) {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries.flatten().fold(0u64, |total, entry| {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return total;
        };
        if metadata.file_type().is_dir() {
            total.saturating_add(directory_bytes(&path))
        } else if metadata.file_type().is_file() {
            total.saturating_add(metadata.len())
        } else {
            total
        }
    })
}

fn botguard_entries(root: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let directory = match fs::read_dir(root) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut entries = Vec::new();
    for entry in directory {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("botguard-") && name.ends_with(".bin") && entry.file_type()?.is_file() {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn botguard_snapshot_bytes(root: &Path) -> u64 {
    botguard_entries(root)
        .unwrap_or_default()
        .iter()
        .fold(0u64, |total, entry| {
            total.saturating_add(entry.metadata().map_or(0, |metadata| metadata.len()))
        })
}

#[cfg(test)]
mod tests {
    use super::{CacheKind, clear_cache_at, disk_cache_usage_at};
    use std::{fs, path::PathBuf, time::SystemTime};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "crabytube-storage-{label}-{}-{:?}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    #[test]
    fn reports_each_cache_and_clears_only_the_selected_one() {
        let root = temp_root("selective-clear");
        fs::create_dir_all(root.join("artwork")).unwrap();
        fs::create_dir_all(root.join("lyrics")).unwrap();
        fs::create_dir_all(root.join("player-js")).unwrap();
        fs::write(root.join("artwork/cover.png"), b"12345").unwrap();
        fs::write(root.join("lyrics/song.json"), b"123").unwrap();
        fs::write(root.join("home-v1.json"), b"1234567").unwrap();
        fs::write(root.join("player-js/current.json"), b"123456789").unwrap();
        fs::write(root.join("botguard-a.bin"), b"12").unwrap();
        fs::write(root.join("botguard-snapshot.bin"), b"1234").unwrap();
        fs::write(root.join("diagnostics.json"), b"keep").unwrap();

        assert_eq!(
            disk_cache_usage_at(&root),
            super::CacheUsage {
                artwork_bytes: 5,
                lyrics_bytes: 3,
                home_bytes: 7,
                player_js_bytes: 9,
                botguard_bytes: 6,
            }
        );
        assert_eq!(disk_cache_usage_at(&root).total_bytes(), 30);
        assert_eq!(clear_cache_at(&root, CacheKind::PlayerJs).unwrap(), 9);
        assert_eq!(clear_cache_at(&root, CacheKind::BotGuard).unwrap(), 6);
        assert_eq!(disk_cache_usage_at(&root).player_js_bytes, 0);
        assert_eq!(disk_cache_usage_at(&root).botguard_bytes, 0);
        assert_eq!(disk_cache_usage_at(&root).artwork_bytes, 5);
        assert_eq!(fs::read(root.join("diagnostics.json")).unwrap(), b"keep");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_missing_cache_is_a_successful_noop() {
        let root = temp_root("empty");
        assert_eq!(clear_cache_at(&root, CacheKind::Home).unwrap(), 0);
        assert_eq!(clear_cache_at(&root, CacheKind::Artwork).unwrap(), 0);
    }
}
