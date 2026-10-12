//! Memory and CPU use of this process, for the status line on Home. Linux reads them from
//! `/proc` and macOS from `proc_pid_rusage`; other systems have no reading, and the status line
//! then shows only its other parts.

use std::time::Instant;

/// Clock ticks per second in `/proc/<pid>/stat` (`USER_HZ`), which is 100 on every Linux ABI. The
/// macOS reading is converted to the same unit.
const CLOCK_TICKS_PER_SECOND: f32 = 100.0;

/// The raw counters behind one reading. CPU is a difference between two samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessSample {
    /// Proportional set size, in KiB, from `/proc/self/smaps_rollup`. On macOS it is the physical
    /// footprint, the figure Activity Monitor calls Memory.
    pub pss_kib: u64,
    /// User plus system time, in clock ticks.
    pub cpu_ticks: u64,
    pub taken: Instant,
}

/// Whether the readings exist here. Elsewhere the status line shows no memory or CPU, and no
/// reader needs to run.
pub const AVAILABLE: bool = cfg!(any(target_os = "linux", target_os = "macos"));

/// What the status line shows: whole mebibytes of memory, and whole percent of one core since
/// the previous sample. Rounding keeps the figure still while the process is idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessReading {
    pub mib: u64,
    /// `None` until a second sample exists.
    pub cpu_percent: Option<u32>,
}

impl ProcessReading {
    pub fn from_samples(sample: &ProcessSample, previous: Option<&ProcessSample>) -> Self {
        Self {
            mib: sample.pss_kib / 1024,
            cpu_percent: previous
                .and_then(|previous| cpu_percent(previous, sample))
                .map(|percent| percent.round() as u32),
        }
    }
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

#[cfg(target_os = "macos")]
pub fn sample() -> Option<ProcessSample> {
    let mut footprint = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    let mut times = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: each call fills the buffer it is given, which has the size of the structure it
    // asks for, and a structure is only read when its call reports success.
    let (footprint, times) = unsafe {
        let status = libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V2,
            footprint.as_mut_ptr().cast(),
        );
        if status != 0 || libc::getrusage(libc::RUSAGE_SELF, times.as_mut_ptr()) != 0 {
            return None;
        }
        (footprint.assume_init(), times.assume_init())
    };
    let micros = |time: libc::timeval| {
        u64::try_from(time.tv_sec)
            .ok()?
            .checked_mul(1_000_000)?
            .checked_add(u64::try_from(time.tv_usec).ok()?)
    };
    Some(ProcessSample {
        pss_kib: footprint.ri_phys_footprint / 1024,
        cpu_ticks: cpu_ticks_from_micros(
            micros(times.ru_utime)?.checked_add(micros(times.ru_stime)?)?,
        ),
        taken: Instant::now(),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn sample() -> Option<ProcessSample> {
    None
}

/// Converts microseconds of CPU time to clock ticks of `1 / CLOCK_TICKS_PER_SECOND` seconds.
#[cfg(any(target_os = "macos", test))]
fn cpu_ticks_from_micros(micros: u64) -> u64 {
    micros / (1_000_000.0 / CLOCK_TICKS_PER_SECOND) as u64
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
    fn microseconds_of_cpu_time_become_clock_ticks() {
        // 2.5 s of CPU is 250 ticks at 100 per second.
        assert_eq!(cpu_ticks_from_micros(2_500_000), 250);
        // Less than a tick rounds down.
        assert_eq!(cpu_ticks_from_micros(9_999), 0);
        assert_eq!(cpu_ticks_from_micros(10_000), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn this_process_reports_memory_and_cpu_time_on_macos() {
        let first = sample().expect("a reading");
        assert!(first.pss_kib > 1024, "footprint of {} KiB", first.pss_kib);
        // Burn a little CPU: the counter must move forward.
        let started = Instant::now();
        let mut sink = 0u64;
        while started.elapsed() < Duration::from_millis(300) {
            sink = sink.wrapping_add(std::hint::black_box(1));
        }
        std::hint::black_box(sink);
        let second = sample().expect("a second reading");
        assert!(second.cpu_ticks > first.cpu_ticks);
        assert!(cpu_percent(&first, &second).is_some());
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

    #[test]
    fn the_status_figures_are_whole_mebibytes_and_percent() {
        let start = Instant::now();
        let first = ProcessSample {
            pss_kib: 71_680,
            cpu_ticks: 1_000,
            taken: start,
        };
        // Three ticks in five seconds is 0.6 % of one core, which shows as 1.
        let second = ProcessSample {
            pss_kib: 71_900,
            cpu_ticks: 1_003,
            taken: start + Duration::from_secs(5),
        };
        assert_eq!(
            ProcessReading::from_samples(&first, None),
            ProcessReading {
                mib: 70,
                cpu_percent: None,
            }
        );
        assert_eq!(
            ProcessReading::from_samples(&second, Some(&first)),
            ProcessReading {
                mib: 70,
                cpu_percent: Some(1),
            }
        );
    }
}
