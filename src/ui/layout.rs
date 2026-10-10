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

pub fn clamped_window(width: f32, height: f32) -> (f32, f32) {
    (width.max(MIN_WIDTH), height.max(MIN_HEIGHT))
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
