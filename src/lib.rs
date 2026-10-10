pub mod i18n;
pub mod music;
pub mod network;
pub mod notifications;
pub mod playback;
pub mod ui;

pub mod artwork;
mod home_cache;
pub mod local_clock;
pub mod memory;
pub mod preferences;
pub mod process_stats;
pub mod runtime_cache;
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
