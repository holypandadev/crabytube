//! Flatpak sandbox detection. Flatpak places `/.flatpak-info` at the root of every sandbox.

/// Whether CrabyTube runs inside a Flatpak sandbox.
#[cfg(target_os = "linux")]
pub fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Flatpak exists on Linux only.
#[cfg(not(target_os = "linux"))]
pub fn in_flatpak() -> bool {
    false
}
