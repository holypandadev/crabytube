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
    crate::app_dirs::cache_dir()
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
    // Covers are saved as they download, so their folder is measured and removed with no save under
    // way, and downloads that began before are not saved.
    if kind == CacheKind::Artwork {
        let folder = root.join("artwork");
        return crate::artwork::clearing_folder(|| {
            let bytes = directory_bytes(&folder);
            remove_dir_if_present(&folder)?;
            Ok(bytes)
        });
    }
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

/// Creates a folder the app owns, with its parents. On Unix a folder created here is readable only
/// by its owner. A folder that already exists keeps its permissions, because it may be shared with
/// other programs.
pub(crate) fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

/// Makes the app's own folders owner-only where they already exist, as the folders created now are.
/// Only the `crabytube` folders under the config and cache roots change, with the folders below
/// them. Links are not followed. Unix only: other systems have no such modes.
#[cfg(unix)]
pub fn tighten_app_folders() {
    for root in [crate::app_dirs::config_dir(), crate::app_dirs::cache_dir()]
        .into_iter()
        .flatten()
    {
        tighten_private_tree(&root.join("crabytube"));
    }
}

#[cfg(not(unix))]
pub fn tighten_app_folders() {}

/// Sets owner-only permissions on `dir` and on every real folder below it. Files and links are left
/// alone, and a link is not followed.
#[cfg(unix)]
fn tighten_private_tree(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = fs::symlink_metadata(dir) else {
        return;
    };
    if !metadata.file_type().is_dir() {
        return;
    }
    if metadata.permissions().mode() & 0o777 != 0o700 {
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        tighten_private_tree(&entry.path());
    }
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

    #[cfg(unix)]
    #[test]
    fn folders_the_app_creates_are_private_and_existing_ones_keep_their_mode() {
        use super::create_private_dir_all;
        use std::os::unix::fs::PermissionsExt;
        let root = temp_root("private-dirs");
        let nested = root.join("cache").join("stream");
        create_private_dir_all(&nested).unwrap();
        let mode =
            |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&nested), 0o700);
        let shared = root.join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir_all(&shared).unwrap();
        assert_eq!(mode(&shared), 0o755);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn existing_app_folders_become_owner_only_and_links_and_other_folders_stay_as_they_were() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let base = temp_root("tighten-tree");
        let app = base.join("crabytube");
        let artwork = app.join("artwork");
        let outside = base.join("outside");
        fs::create_dir_all(&artwork).unwrap();
        fs::create_dir_all(&outside).unwrap();
        for dir in [&app, &artwork, &outside] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
        }
        // A link inside the app's folders must not lead to the folder it points at.
        symlink(&outside, app.join("linked")).unwrap();

        super::tighten_private_tree(&app);

        let mode =
            |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&app), 0o700);
        assert_eq!(mode(&artwork), 0o700);
        assert_eq!(mode(&outside), 0o755);
        let _ = fs::remove_dir_all(&base);
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
