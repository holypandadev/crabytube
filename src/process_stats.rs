//! Memory and CPU use of this process, for the status line on Home. Linux reads them from
//! `/proc`; other systems have no reading, and the status line then shows only its other parts.

use std::time::Instant;

/// Clock ticks per second in `/proc/<pid>/stat` (`USER_HZ`), which is 100 on every Linux ABI.
const CLOCK_TICKS_PER_SECOND: f32 = 100.0;

/// The raw counters behind one reading. CPU is a difference between two samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessSample {
    /// Proportional set size, in KiB, from `/proc/self/smaps_rollup`.
    pub pss_kib: u64,
    /// User plus system time, in clock ticks, from `/proc/self/stat`.
    pub cpu_ticks: u64,
    pub taken: Instant,
}

/// What the status line shows: memory, and CPU since the previous sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessReading {
    pub pss_kib: u64,
    /// `None` until a second sample exists.
    pub cpu_percent: Option<f32>,
}

#[cfg(target_os = "linux")]
pub fn sample() -> Option<ProcessSample> {
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup").ok()?;
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    Some(ProcessSample {
        pss_kib: parse_pss_kib(&rollup)?,
        cpu_ticks: parse_cpu_ticks(&stat)?,
        taken: Instant::now(),
    })
}

#[cfg(not(target_os = "linux"))]
pub fn sample() -> Option<ProcessSample> {
    None
}

/// Share of one core used between two samples, in percent (one core busy is 100).
pub fn cpu_percent(previous: &ProcessSample, current: &ProcessSample) -> Option<f32> {
    let elapsed = current
        .taken
        .checked_duration_since(previous.taken)?
        .as_secs_f32();
    if elapsed <= 0.0 {
        return None;
    }
    let ticks = current.cpu_ticks.checked_sub(previous.cpu_ticks)?;
    Some(ticks as f32 / CLOCK_TICKS_PER_SECOND / elapsed * 100.0)
}

/// The `Pss:` line of `smaps_rollup`, which is already in KiB.
#[cfg(any(target_os = "linux", test))]
fn parse_pss_kib(rollup: &str) -> Option<u64> {
    rollup
        .lines()
        .find_map(|line| line.strip_prefix("Pss:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()
}

/// utime (field 14) plus stime (field 15) of `/proc/<pid>/stat`. The command name in field 2
/// sits in parentheses and may hold spaces or parentheses, so fields are counted from the last
/// `)`: field 3 (state) is the first one after it.
#[cfg(any(target_os = "linux", test))]
fn parse_cpu_ticks(stat: &str) -> Option<u64> {
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pss_is_read_from_the_rollup_in_kib() {
        let rollup = "Rss:              70000 kB\nPss:              45123 kB\nPss_Dirty:  1 kB\n";
        assert_eq!(parse_pss_kib(rollup), Some(45_123));
        assert_eq!(parse_pss_kib("Rss: 1 kB\n"), None);
    }

    #[test]
    fn cpu_ticks_are_taken_after_a_command_name_with_parentheses() {
        // Fields after the name: state S, ppid, ... utime 900, stime 120.
        let stat = "4242 (crab (y)tube) S 1 4242 4242 0 -1 4194560 1000 0 0 0 900 120 0 0 20 0 3 0";
        assert_eq!(parse_cpu_ticks(stat), Some(1_020));
        assert_eq!(parse_cpu_ticks("4242 (short) S 1"), None);
    }

    #[test]
    fn cpu_percent_is_the_tick_gain_over_the_wall_time() {
        let start = Instant::now();
        let first = ProcessSample {
            pss_kib: 40_000,
            cpu_ticks: 1_000,
            taken: start,
        };
        // 5 s of wall time and 10 s of CPU ticks at 100 per second: 2 % of one core.
        let second = ProcessSample {
            pss_kib: 40_100,
            cpu_ticks: 1_010,
            taken: start + Duration::from_secs(5),
        };
        let percent = cpu_percent(&first, &second).expect("two samples");
        assert!((percent - 2.0).abs() < 0.001, "{percent}");
        assert_eq!(cpu_percent(&second, &first), None, "time must move forward");
    }
}
