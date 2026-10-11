/// The app id. The window reports it, MPRIS names it as the desktop entry, and the desktop entry and
/// the icons of a Linux install carry it in their file names.
pub const APP_ID: &str = "dev.holypanda.CrabyTube";

pub mod app_dirs;
pub mod i18n;
pub mod logging;
pub mod music;
pub mod network;
pub mod notifications;
pub mod playback;
pub mod ui;

pub mod artwork;
pub mod background_portal;
pub mod flatpak;
mod home_cache;
pub mod local_clock;
pub mod memory;
pub mod preferences;
pub mod process_stats;
pub mod runtime_cache;
pub mod single_instance;
pub mod storage;
pub mod temp_cleanup;

pub mod lyrics;
#[cfg(target_os = "linux")]
pub mod mpris;
#[cfg(not(target_os = "linux"))]
#[path = "mpris_stub.rs"]
pub mod mpris;
#[cfg(target_os = "linux")]
pub mod tray;
#[cfg(not(target_os = "linux"))]
#[path = "tray_stub.rs"]
pub mod tray;

pub mod discord;

#[cfg(all(test, target_os = "linux"))]
mod packaging {
    use super::APP_ID;

    #[test]
    fn the_desktop_entry_and_the_icon_carry_the_app_id() {
        let entry = include_str!("../packaging/linux/dev.holypanda.CrabyTube.desktop");
        assert!(entry.lines().any(|line| line == format!("Icon={APP_ID}")));
        assert!(entry.lines().any(|line| line == "Exec=crabytube"));
        // Fails to build when the icon named after the app id is missing.
        let _icon =
            include_bytes!("../packaging/icons/hicolor/256x256/apps/dev.holypanda.CrabyTube.png");
    }
}
