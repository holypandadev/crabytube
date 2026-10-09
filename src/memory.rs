//! Memory hygiene around BotGuard helper runs. A helper is a separate process, but its
//! output and the app's own transient buffers compete for the same memory budget.

use std::sync::atomic::{AtomicUsize, Ordering};

static HELPER_WINDOWS: AtomicUsize = AtomicUsize::new(0);

/// Marks the time a helper process is alive. While one is, the UI shrinks the in-memory
/// artwork cache to a small budget; the evicted images are still on disk.
pub struct HelperWindow(());

impl HelperWindow {
    pub fn enter() -> Self {
        HELPER_WINDOWS.fetch_add(1, Ordering::SeqCst);
        trim_heap();
        Self(())
    }
}

impl Drop for HelperWindow {
    fn drop(&mut self) {
        HELPER_WINDOWS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn helper_running() -> bool {
    HELPER_WINDOWS.load(Ordering::SeqCst) > 0
}

/// Returns freed heap pages to the OS (glibc only).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn trim_heap() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> std::ffi::c_int;
    }
    // SAFETY: malloc_trim is a glibc allocator call that takes no pointers and is
    // safe to call from any thread at any time.
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn trim_heap() {}

#[cfg(test)]
mod tests {
    use super::HelperWindow;

    #[test]
    fn helper_window_counts_nested_runs_and_clears_on_drop() {
        let first = HelperWindow::enter();
        let second = HelperWindow::enter();
        assert!(super::helper_running());
        drop(first);
        assert!(super::helper_running());
        drop(second);
    }
}
