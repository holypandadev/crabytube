// Release builds on Windows start without a console window. Diagnostics builds keep theirs,
// because their output goes there.
#![cfg_attr(
    all(windows, not(debug_assertions), not(feature = "diagnostics")),
    windows_subsystem = "windows"
)]

use tracing::error;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    fn mallopt(param: std::ffi::c_int, value: std::ffi::c_int) -> std::ffi::c_int;
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn tune_glibc_allocator() -> [bool; 4] {
    // Favour a small resident footprint (one arena, eager trimming) before the GUI
    // starts creating worker threads and caches. Values set by the launcher win.
    const OPTIONS: [(&str, std::ffi::c_int, std::ffi::c_int); 4] = [
        ("MALLOC_ARENA_MAX", -8, 1),
        ("MALLOC_TRIM_THRESHOLD_", -1, 65_536),
        ("MALLOC_MMAP_THRESHOLD_", -3, 131_072),
        ("MALLOC_TOP_PAD_", -2, 0),
    ];
    let mut applied = [false; 4];
    for (index, (environment, option, value)) in OPTIONS.into_iter().enumerate() {
        if std::env::var_os(environment).is_none() {
            // SAFETY: glibc documents mallopt as process-wide allocator tuning;
            // it is called once on the main thread before the application starts.
            applied[index] = unsafe { mallopt(option, value) != 0 };
        }
    }
    applied
}

/// `--autoplay <videoId>` starts that track once Home loads.
/// `--exit-after <seconds>` closes the window after the given delay.
#[cfg(feature = "diagnostics")]
fn parse_diagnostic_args() {
    let mut args = std::env::args().skip(1);
    let mut video_id = None;
    let mut exit_after = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--scroll-test" => crabytube::ui::enable_diagnostic_scroll(),
            "--scroll-page" => {
                if let Some(page) = args.next() {
                    crabytube::ui::enable_diagnostic_scroll_page(page);
                }
            }
            "--soak" => crabytube::ui::enable_diagnostic_soak(),
            "--screenshots" => {
                if let Some(dir) = args.next() {
                    crabytube::ui::enable_diagnostic_screenshots(dir.into());
                }
            }
            "--autoplay" => video_id = args.next(),
            "--exit-after" => {
                exit_after = args
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(std::time::Duration::from_secs);
            }
            _ => {}
        }
    }
    // An exit timer works without autoplay; an empty ID plays nothing.
    if video_id.is_some() || exit_after.is_some() {
        crabytube::ui::set_diagnostic_autoplay(crabytube::ui::DiagnosticAutoplay {
            video_id: video_id.unwrap_or_default(),
            exit_after,
        });
    }
}

/// The screenshot tour and the churn loop write state, so they run against a throwaway
/// profile unless the directories already point into the temporary directory. The directories
/// come from the XDG variables, except on macOS, where they all hang from the home directory.
/// Returns the directory to remove when the run ends.
#[cfg(feature = "diagnostics")]
fn isolate_diagnostic_profile() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    const VARIABLES: [&str; 1] = ["HOME"];
    #[cfg(not(target_os = "macos"))]
    const VARIABLES: [&str; 4] = [
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
    ];
    let writes_profile = std::env::args().any(|argument| argument == "--screenshots")
        || std::env::var_os("CRABYTUBE_CHURN").is_some();
    if !writes_profile {
        return None;
    }
    let temp = std::env::temp_dir();
    let already_temporary = VARIABLES.iter().all(|name| {
        std::env::var_os(name).is_some_and(|value| std::path::Path::new(&value).starts_with(&temp))
    });
    if already_temporary {
        return None;
    }
    let root = temp.join(format!("crabytube-tour-{}", std::process::id()));
    for name in VARIABLES {
        let directory = root.join(name.to_ascii_lowercase());
        let _ = std::fs::create_dir_all(&directory);
        // SAFETY: runs first in `main`, before any thread exists.
        unsafe { std::env::set_var(name, directory) };
    }
    Some(root)
}

fn main() {
    #[cfg(feature = "diagnostics")]
    let isolated_profile = isolate_diagnostic_profile();

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    let allocator_settings = tune_glibc_allocator();

    crabytube::logging::init();
    // Before any window exists, which takes the id it has then.
    crabytube::updater::identify_process();

    // Both set environment variables, so they run before any thread starts: claiming the single
    // instance starts one.
    crabytube::ui::configure_renderer_from_preferences();
    #[cfg(target_os = "linux")]
    crabytube::playback::output::provide_sound_server_cookie();

    // Diagnostics runs use their own throwaway profile and may start beside a running app.
    #[cfg(not(feature = "diagnostics"))]
    if !crabytube::single_instance::claim() {
        return;
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    tracing::debug!(
        arena_max = allocator_settings[0],
        trim_threshold = allocator_settings[1],
        mmap_threshold = allocator_settings[2],
        top_pad = allocator_settings[3],
        "Applied Linux glibc allocator defaults"
    );

    crabytube::temp_cleanup::sweep_in_background();
    crabytube::storage::tighten_app_folders();

    #[cfg(feature = "diagnostics")]
    parse_diagnostic_args();

    let result = crabytube::ui::run();
    // An update the app has downloaded installs now that it has let go of its files.
    crabytube::updater::run_scheduled_install();
    #[cfg(feature = "diagnostics")]
    if let Some(root) = isolated_profile {
        let _ = std::fs::remove_dir_all(root);
    }
    #[cfg(feature = "diagnostics")]
    if let Some(code) = crabytube::ui::diagnostic_failure_code() {
        std::process::exit(code);
    }
    if let Err(err) = result {
        error!(?err, "application exited with an error");
        std::process::exit(1);
    }
}
