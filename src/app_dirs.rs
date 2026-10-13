//! The folders the app keeps its data in. Every lookup goes through this module. In test builds the
//! folders point at a temporary folder of the test process, so a test run never writes to the
//! user's real cache, configuration, documents or local data folders.

use std::path::PathBuf;

/// The base folder for caches. Tests get a folder of their own.
pub fn cache_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return Some(test_root().join("cache"));
    }
    dirs::cache_dir()
}

/// The base folder for settings and saved state.
pub fn config_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return Some(test_root().join("config"));
    }
    dirs::config_dir()
}

/// The folder that exports and backups are written to.
pub fn document_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return Some(test_root().join("documents"));
    }
    dirs::document_dir()
}

/// The folder for per-machine data, such as the log file on Windows and macOS.
pub fn data_local_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return Some(test_root().join("local-data"));
    }
    dirs::data_local_dir()
}

/// The temporary folder of this test process. Only test builds reach it.
fn test_root() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| std::env::temp_dir().join(format!("crabytube-test-{}", std::process::id())))
        .clone()
}

/// The folder of the running executable, where the helpers are installed beside it. On macOS a
/// launch through a symbolic link names the link, so the path is resolved first; elsewhere the
/// system already gives the real file.
pub fn executable_dir() -> Option<std::path::PathBuf> {
    let executable = std::env::current_exe().ok()?;
    #[cfg(target_os = "macos")]
    let executable = std::fs::canonicalize(&executable).unwrap_or(executable);
    executable.parent().map(std::path::Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_runs_never_resolve_the_real_folders() {
        let temporary = std::env::temp_dir();
        let folders = [
            (cache_dir(), dirs::cache_dir()),
            (config_dir(), dirs::config_dir()),
            (document_dir(), dirs::document_dir()),
            (data_local_dir(), dirs::data_local_dir()),
        ];
        for (in_tests, real) in folders {
            let in_tests = in_tests.expect("a test folder is always available");
            assert!(
                in_tests.starts_with(&temporary),
                "{} is outside the temporary folder",
                in_tests.display()
            );
            if let Some(real) = real {
                assert_ne!(in_tests, real, "a test would write to a real folder");
            }
        }
    }

    #[test]
    fn app_code_reaches_the_folders_only_through_this_module() {
        let mut offenders = Vec::new();
        collect_direct_lookups(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut offenders,
        );
        assert!(
            offenders.is_empty(),
            "use crate::app_dirs instead of dirs:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn only_direct_dirs_calls_are_flagged() {
        assert!(calls_dirs_directly("let dir = dirs::cache_dir();"));
        assert!(calls_dirs_directly("dirs::config_dir()"));
        assert!(!calls_dirs_directly(
            "let dir = crate::app_dirs::cache_dir();"
        ));
        assert!(!calls_dirs_directly("let my_dirs::value = 1;"));
    }

    /// Whether a line uses `dirs::` as a path of its own, rather than inside a longer name such as
    /// `app_dirs::`.
    fn calls_dirs_directly(line: &str) -> bool {
        line.match_indices("dirs::").any(|(index, _)| {
            line[..index]
                .chars()
                .next_back()
                .is_none_or(|previous| !(previous.is_alphanumeric() || previous == '_'))
        })
    }

    fn collect_direct_lookups(directory: &Path, offenders: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_direct_lookups(&path, offenders);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && !path.ends_with("app_dirs.rs")
            {
                let Ok(source) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (number, line) in source.lines().enumerate() {
                    if calls_dirs_directly(line) {
                        offenders.push(format!("{}:{}", path.display(), number + 1));
                    }
                }
            }
        }
    }
}
