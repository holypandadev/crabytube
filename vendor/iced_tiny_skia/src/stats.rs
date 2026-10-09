//! Optional per-frame render counters.
//! When `CRABYTUBE_RENDER_STATS` is set (to any value), one line per presented
//! frame is printed to stderr; otherwise nothing is printed.
use std::sync::OnceLock;
use std::time::Duration;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CRABYTUBE_RENDER_STATS").is_some())
}

/// Prints one presented frame's damage count, layer count and draw time.
pub(crate) fn frame(damage_rects: usize, layers: usize, draw: Duration) {
    if enabled() {
        eprintln!(
            "render-stats damage={damage_rects} layers={layers} draw_us={}",
            draw.as_micros()
        );
    }
}
