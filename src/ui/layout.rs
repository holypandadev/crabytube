//! Pure responsive layout helpers, testable without a GUI.
//!
//! All viewport-dependent geometry lives here so it can be tested without
//! building widgets. Views only call these functions.

use std::ops::Range;

/// Smallest window: phone-sized, so the narrow layout can be used and tested.
pub const MIN_WIDTH: f32 = 360.0;
pub const MIN_HEIGHT: f32 = 560.0;

pub fn default_window_size() -> (f32, f32) {
    if cfg!(target_os = "linux") {
        (1024.0, 768.0)
    } else {
        (1440.0, 900.0)
    }
}

/// Share of the monitor's width the default window may take.
const DEFAULT_WIDTH_SHARE: f32 = 0.9;
/// Share of the monitor's height the default window may take. The window is centred on the whole
/// monitor, and the taskbar (48 px on Windows) takes the bottom of it: with this share the window
/// stays above it on any monitor from 640 px high. With 90 % a 768 or 800 px laptop screen hid the
/// bottom of the player bar.
const DEFAULT_HEIGHT_SHARE: f32 = 0.85;

/// The default window size, limited to part of the monitor so that a first launch fits on a small
/// screen. A size that already fits is kept, and the result never drops below the minimum.
pub fn fit_to_monitor(size: (f32, f32), monitor: (f32, f32)) -> (f32, f32) {
    clamped_window(
        size.0.min(monitor.0 * DEFAULT_WIDTH_SHARE),
        size.1.min(monitor.1 * DEFAULT_HEIGHT_SHARE),
    )
}

pub fn clamped_window(width: f32, height: f32) -> (f32, f32) {
    (width.max(MIN_WIDTH), height.max(MIN_HEIGHT))
}

/// The window size and maximized flag to keep after a resize. A maximized window leaves the saved
/// size alone, so restoring it brings the earlier size back instead of the whole monitor. A size
/// below the minimum, which is what a minimized window reports, changes nothing.
pub fn remembered_geometry(
    saved: ((f32, f32), bool),
    reported: (f32, f32),
    maximized: bool,
) -> ((f32, f32), bool) {
    if reported.0 < MIN_WIDTH || reported.1 < MIN_HEIGHT {
        return saved;
    }
    if maximized {
        (saved.0, true)
    } else {
        (reported, false)
    }
}

/// How to bring a window that opened at its saved size onto the monitor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitAdjustment {
    pub size: (f32, f32),
    /// Where to move the window, when it stands on the monitor at the origin and so can be placed.
    pub position: Option<(f32, f32)>,
}

/// What to change when the saved size is larger than the monitor: the size is cut with
/// `fit_to_monitor` and the window is centred if its corner is on the monitor at the origin. A
/// window that fits, wherever it stands, is left exactly as it is.
pub fn fit_saved_window(
    size: (f32, f32),
    monitor: (f32, f32),
    position: Option<(f32, f32)>,
) -> Option<FitAdjustment> {
    if size.0 <= monitor.0 && size.1 <= monitor.1 {
        return None;
    }
    let size = fit_to_monitor(size, monitor);
    let position = position
        .filter(|(x, y)| (0.0..monitor.0).contains(x) && (0.0..monitor.1).contains(y))
        .map(|_| ((monitor.0 - size.0) / 2.0, (monitor.1 - size.1) / 2.0));
    Some(FitAdjustment { size, position })
}

/// Bounded row window for long vertical lists. The returned range includes
/// overscan rows on both sides so small scroll changes do not churn the view.
pub fn virtual_range(
    total: usize,
    scroll_y: f32,
    viewport_height: f32,
    row_pitch: f32,
    overscan: usize,
) -> Range<usize> {
    if total == 0 || row_pitch <= 0.0 {
        return 0..0;
    }
    let first = (scroll_y.max(0.0) / row_pitch).floor() as usize;
    let visible = (viewport_height.max(0.0) / row_pitch).ceil() as usize;
    first.saturating_sub(overscan).min(total)
        ..first
            .saturating_add(visible)
            .saturating_add(overscan)
            .min(total)
}

/// How far from the end of the search results the next page is asked for, in pixels.
pub const CATALOG_MORE_MARGIN: f32 = 480.0;
/// Height of a top/bottom spacer for `missing` virtual rows. Because spacers
/// live in a spacing-0 inner column, the height is exactly `missing * pitch`
/// with no extra column gap.
pub fn virtual_spacer_height(missing: usize, row_pitch: f32) -> f32 {
    missing as f32 * row_pitch.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_window_is_documented_and_clamped() {
        assert_eq!(MIN_WIDTH, 360.0);
        assert_eq!(MIN_HEIGHT, 560.0);
        assert_eq!(clamped_window(320.0, 400.0), (360.0, 560.0));
    }

    #[test]
    fn the_default_window_leaves_room_on_the_monitor() {
        // A large monitor keeps the default.
        assert_eq!(
            fit_to_monitor((1440.0, 900.0), (2560.0, 1440.0)),
            (1440.0, 900.0)
        );
        // A 1440x900 laptop gets 90 % of the width and 85 % of the height.
        let (width, height) = fit_to_monitor((1440.0, 900.0), (1440.0, 900.0));
        assert!((width - 1296.0).abs() < 0.01 && (height - 765.0).abs() < 0.01);
        // Only the side that does not fit shrinks.
        let (width, height) = fit_to_monitor((1440.0, 900.0), (2560.0, 800.0));
        assert_eq!(width, 1440.0);
        assert!((height - 680.0).abs() < 0.01);
        // Centred on a 768 or 800 px screen, the window ends above a 48 px taskbar.
        for monitor_height in [768.0_f32, 800.0, 900.0] {
            let (_, height) = fit_to_monitor((1440.0, 900.0), (1366.0, monitor_height));
            let bottom = (monitor_height + height) / 2.0;
            assert!(
                bottom <= monitor_height - 48.0,
                "{monitor_height}: {bottom}"
            );
        }
        // A tiny monitor never takes the window below the minimum.
        assert_eq!(
            fit_to_monitor((1440.0, 900.0), (400.0, 300.0)),
            (MIN_WIDTH, MIN_HEIGHT)
        );
    }

    #[test]
    fn a_maximized_window_keeps_the_saved_size() {
        let saved = ((1440.0, 900.0), false);
        // Maximizing reports the whole monitor: the size stays and the flag is raised.
        assert_eq!(
            remembered_geometry(saved, (2560.0, 1392.0), true),
            ((1440.0, 900.0), true)
        );
        // Further resizes while maximized change nothing.
        assert_eq!(
            remembered_geometry(((1440.0, 900.0), true), (1920.0, 1080.0), true),
            ((1440.0, 900.0), true)
        );
        // Restoring reports the normal size and lowers the flag.
        assert_eq!(
            remembered_geometry(((1440.0, 900.0), true), (1300.0, 800.0), false),
            ((1300.0, 800.0), false)
        );
        // A plain resize is kept.
        assert_eq!(
            remembered_geometry(saved, (1000.0, 700.0), false),
            ((1000.0, 700.0), false)
        );
    }

    #[test]
    fn a_minimized_window_changes_nothing() {
        let saved = ((1440.0, 900.0), true);
        assert_eq!(remembered_geometry(saved, (0.0, 0.0), false), saved);
        assert_eq!(
            remembered_geometry(saved, (MIN_WIDTH - 1.0, 900.0), false),
            saved
        );
    }

    #[test]
    fn a_saved_size_that_fits_the_monitor_is_not_touched() {
        let monitor = (2560.0, 1440.0);
        assert_eq!(fit_saved_window((1440.0, 900.0), monitor, None), None);
        // Exactly the monitor still fits, and so does a window standing anywhere.
        assert_eq!(
            fit_saved_window((2560.0, 1440.0), monitor, Some((300.0, 300.0))),
            None
        );
    }

    #[test]
    fn a_saved_size_larger_than_the_monitor_is_cut_and_centred() {
        let monitor = (2560.0, 1440.0);
        let adjustment = fit_saved_window((2600.0, 1392.0), monitor, Some((312.0, 312.0)))
            .expect("larger than the monitor");
        let (width, height) = adjustment.size;
        assert!((width - 2304.0).abs() < 0.01 && (height - 1224.0).abs() < 0.01);
        let (x, y) = adjustment
            .position
            .expect("a window on the monitor is centred");
        assert!((x - 128.0).abs() < 0.01 && (y - 108.0).abs() < 0.01);
    }

    #[test]
    fn an_oversized_window_on_another_monitor_or_unplaceable_is_only_resized() {
        let monitor = (1920.0, 1080.0);
        let beyond = fit_saved_window((2560.0, 1392.0), monitor, Some((2000.0, 100.0)))
            .expect("larger than the monitor");
        assert_eq!(beyond.position, None);
        let unknown = fit_saved_window((2560.0, 1392.0), monitor, None).expect("larger");
        assert_eq!(unknown.position, None);
        // Only the side that does not fit is cut when the other is within its share.
        let tall = fit_saved_window((1200.0, 1500.0), monitor, None).expect("too tall");
        assert_eq!(tall.size.0, 1200.0);
        assert!((tall.size.1 - 918.0).abs() < 0.01);
    }

    #[test]
    fn platform_default_window_size_meets_supported_minimum() {
        let (width, height) = default_window_size();
        assert!(width >= MIN_WIDTH);
        assert!(height >= MIN_HEIGHT);
    }

    #[test]
    fn virtual_lists_keep_rendered_rows_bounded_and_cover_boundaries() {
        for total in [100, 1_000, 10_000] {
            let range = virtual_range(total, 3_600.0, 720.0, 72.0, 8);
            assert!(range.start <= 50);
            assert!(range.contains(&50));
            assert!(range.end <= total);
            assert!(range.len() <= 27);
        }
        assert_eq!(virtual_range(0, 0.0, 700.0, 72.0, 8), 0..0);
        assert_eq!(virtual_range(12, 10_000.0, 700.0, 72.0, 8), 12..12);
        assert_eq!(virtual_range(20, 0.0, 100.0, 72.0, 0), 0..2);
    }

    #[test]
    fn virtual_spacers_are_exact_without_double_spacing() {
        // Spacers live in a spacing-0 inner column, so height is exactly
        // missing * pitch with no extra column gap.
        assert_eq!(virtual_spacer_height(0, 78.0), 0.0);
        assert_eq!(virtual_spacer_height(5, 78.0), 390.0);
        assert_eq!(virtual_spacer_height(10, 76.0), 760.0);
        // Start + visible + end reconstructs the full list height.
        let pitch = 65.0;
        let total = 200;
        for scroll in [0.0, 780.0, 3_900.0, 7_800.0, 15_000.0] {
            let range = virtual_range(total, scroll, 720.0, pitch, 10);
            let rebuilt = virtual_spacer_height(range.start, pitch)
                + (range.len() as f32 * pitch)
                + virtual_spacer_height(total.saturating_sub(range.end), pitch);
            assert_eq!(rebuilt, total as f32 * pitch);
        }
    }
}
