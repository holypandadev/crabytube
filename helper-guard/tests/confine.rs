//! Linux: the limits and the parent-death signal reach a real helper process. Each test runs this
//! test binary again as the helper, selected by an environment variable and the test's name.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use helper_guard::{Limits, confine};

const ROLE: &str = "HELPER_GUARD_ROLE";
const LIMITS: Limits = Limits {
    deadline: Duration::from_secs(120),
    committed_bytes: 3 << 30,
    cpu_seconds: 300,
};
/// Printed on a line of its own: the test harness has already written the test's name on its line.
const CONFINED: &str = "helper confined";

/// The helper side: confines itself, announces it, and waits for its input to close.
fn run_as_helper() {
    confine(LIMITS);
    println!("\n{CONFINED}");
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
}

/// A helper process. Dropping it kills the process if it still runs, and reaps it either way.
struct Helper {
    child: Child,
    output: BufReader<ChildStdout>,
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts this binary as the helper running the named test. Returns once it has confined itself.
fn start_helper(test: &str) -> Helper {
    let child = Command::new(std::env::current_exe().expect("test binary path"))
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(ROLE, "helper")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("helper starts");
    let mut helper = Helper::owning(child);
    let mut line = String::new();
    loop {
        line.clear();
        let read = helper
            .output
            .read_line(&mut line)
            .expect("helper output reads");
        assert!(read > 0, "the helper ended before it confined itself");
        if line.contains(CONFINED) {
            return helper;
        }
    }
}

impl Helper {
    /// Takes the child's output so the helper can be read. The child is owned from here on.
    fn owning(mut child: Child) -> Helper {
        let output = BufReader::new(child.stdout.take().expect("helper output"));
        Helper { child, output }
    }
}

/// The soft limit that `/proc/<pid>/limits` reports for a resource, such as "Max cpu time".
fn soft_limit(proc_limits: &str, name: &str) -> Option<u64> {
    proc_limits
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
}

#[test]
fn the_helper_applies_its_memory_and_cpu_limits() {
    if std::env::var_os(ROLE).is_some() {
        run_as_helper();
        return;
    }
    let mut helper = start_helper("the_helper_applies_its_memory_and_cpu_limits");
    let proc_limits = std::fs::read_to_string(format!("/proc/{}/limits", helper.child.id()))
        .expect("the helper's limits are readable");
    assert_eq!(
        soft_limit(&proc_limits, "Max data size"),
        Some(LIMITS.committed_bytes)
    );
    assert_eq!(
        soft_limit(&proc_limits, "Max cpu time"),
        Some(LIMITS.cpu_seconds)
    );
    drop(helper.child.stdin.take());
    assert!(helper.child.wait().expect("helper ends").success());
}

#[test]
fn the_helper_dies_when_the_thread_that_started_it_exits() {
    if std::env::var_os(ROLE).is_some() {
        run_as_helper();
        return;
    }
    // The helper is the child of this thread. The thread exits while the helper still waits for input.
    let starter = std::thread::spawn(|| {
        start_helper("the_helper_dies_when_the_thread_that_started_it_exits")
    });
    let mut helper = starter.join().expect("the starting thread ends");
    let started = Instant::now();
    loop {
        if let Some(status) = helper.child.try_wait().expect("helper status") {
            assert_eq!(status.signal(), Some(9), "the helper is killed, not ended");
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the helper outlived its parent thread"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
