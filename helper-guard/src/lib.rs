//! Limits that a helper process puts on itself before it reads its request. The app kills a
//! helper that runs past its timeout. These limits cover a helper whose app died or hung: the
//! deadline runs on every platform, and Linux adds death with the parent thread and address-space
//! and CPU-time limits. The crate has no unsafe code; rustix wraps the system calls.

use std::time::Duration;

/// Exit status of a helper that reached its deadline.
pub const EXIT_DEADLINE: i32 = 3;
/// Exit status of a helper whose parent had already exited when it started.
pub const EXIT_ORPHANED: i32 = 4;

/// The limits a helper applies to itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Wall-clock time after which the helper exits. It must stay above the app's timeouts for
    /// the helper, or a slow but healthy request would be cut short.
    pub deadline: Duration,
    /// Linux: the most private writable memory, in bytes, that the helper may hold (RLIMIT_DATA).
    /// Address space is not limited: V8 reserves tens of gigabytes it never writes, which would
    /// fail under an address-space limit.
    pub committed_bytes: u64,
    /// Linux: the CPU time, in seconds, that the helper may use.
    pub cpu_seconds: u64,
}

/// Applies the limits. Call it first in `main`, before the request is read.
pub fn confine(limits: Limits) {
    arm_deadline(limits.deadline, || std::process::exit(EXIT_DEADLINE));
    #[cfg(target_os = "linux")]
    linux::confine(limits);
}

/// Runs `on_expiry` on a background thread once `after` has passed. The thread ends with the process.
pub fn arm_deadline(after: Duration, on_expiry: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        on_expiry();
    });
}

#[cfg(target_os = "linux")]
mod linux {
    use rustix::process::{
        Resource, Rlimit, Signal, getppid, set_parent_process_death_signal, setrlimit,
    };

    pub(super) fn confine(limits: super::Limits) {
        // The signal arrives when the thread that started this helper exits, which ends the helper.
        let _ = set_parent_process_death_signal(Some(Signal::KILL));
        // A parent that exited before the signal was set has left this helper to init.
        if getppid().is_some_and(|parent| parent.as_raw_nonzero().get() == 1) {
            std::process::exit(super::EXIT_ORPHANED);
        }
        let limit = |value| Rlimit {
            current: Some(value),
            maximum: Some(value),
        };
        // Best effort: a refused limit leaves the helper with the deadline and the parent's timeout.
        let _ = setrlimit(Resource::Data, limit(limits.committed_bytes));
        let _ = setrlimit(Resource::Cpu, limit(limits.cpu_seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::arm_deadline;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn the_deadline_runs_its_callback_once_the_time_has_passed() {
        let (sender, receiver) = mpsc::channel();
        arm_deadline(Duration::from_millis(20), move || {
            let _ = sender.send(());
        });
        assert!(receiver.recv_timeout(Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn the_deadline_does_not_run_before_its_time() {
        let (sender, receiver) = mpsc::channel();
        arm_deadline(Duration::from_secs(60), move || {
            let _ = sender.send(());
        });
        assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
    }
}
