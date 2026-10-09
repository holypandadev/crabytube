//! Diagnostics-only timing of `update()` and `view()`. Every five seconds the recorder
//! logs p50, p95 and max per message kind, plus the five slowest kinds.

use std::{
    fmt::{self, Write},
    sync::Mutex,
    time::{Duration, Instant},
};

const REPORT_EVERY: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Recorder {
    updates: Vec<(&'static str, u64)>,
    views: Vec<u64>,
    /// Intervals between consecutive view() calls while interaction is continuous. This is the
    /// frame-time proxy: each interaction-driven frame rebuilds the view.
    frame_gaps: Vec<u64>,
    last_view_start: Option<Instant>,
    last_report: Option<Instant>,
}

/// A gap longer than this is idle time, not a frame.
const MAX_FRAME_GAP: Duration = Duration::from_millis(500);

static RECORDER: Mutex<Option<Recorder>> = Mutex::new(None);

/// Accepts the leading identifier of a `Debug` value and stops at the first other character,
/// so naming a message never formats its (possibly large) payload.
struct NameWriter {
    name: String,
    finished: bool,
}

impl Write for NameWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.finished {
            return Err(fmt::Error);
        }
        for character in text.chars() {
            if character.is_alphanumeric() || character == '_' {
                self.name.push(character);
            } else {
                self.finished = true;
                return Err(fmt::Error);
            }
        }
        Ok(())
    }
}

/// The variant name of a message, for grouping timings.
pub fn message_name<T: fmt::Debug>(message: &T) -> &'static str {
    let mut writer = NameWriter {
        name: String::new(),
        finished: false,
    };
    let _ = write!(writer, "{message:?}");
    // Names are few and reused, so interning them keeps the recorder allocation-free per sample.
    intern(&writer.name)
}

fn intern(name: &str) -> &'static str {
    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let Ok(mut names) = NAMES.lock() else {
        return "unknown";
    };
    if let Some(existing) = names.iter().find(|known| **known == name) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.push(leaked);
    leaked
}

pub fn record_update(name: &'static str, elapsed: Duration) {
    record(|recorder| recorder.updates.push((name, micros(elapsed))));
}

pub fn record_view(started: Instant, elapsed: Duration) {
    record(|recorder| {
        recorder.views.push(micros(elapsed));
        if let Some(previous) = recorder.last_view_start.replace(started) {
            let gap = started.duration_since(previous);
            if gap <= MAX_FRAME_GAP {
                recorder.frame_gaps.push(micros(gap));
            }
        }
    });
}

fn micros(elapsed: Duration) -> u64 {
    elapsed.as_micros().min(u128::from(u64::MAX)) as u64
}

fn record(push: impl FnOnce(&mut Recorder)) {
    let Ok(mut guard) = RECORDER.lock() else {
        return;
    };
    let recorder = guard.get_or_insert_with(|| Recorder {
        last_report: Some(Instant::now()),
        ..Recorder::default()
    });
    push(recorder);
    let due = recorder
        .last_report
        .is_some_and(|last| last.elapsed() >= REPORT_EVERY);
    if due {
        report(recorder);
        recorder.updates.clear();
        recorder.views.clear();
        recorder.frame_gaps.clear();
        recorder.last_report = Some(Instant::now());
    }
}

fn percentile(sorted: &[u64], fraction: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 * fraction).ceil() as usize).clamp(1, sorted.len());
    sorted[index - 1]
}

fn summarize(mut values: Vec<u64>) -> (usize, u64, u64, u64) {
    values.sort_unstable();
    (
        values.len(),
        percentile(&values, 0.50),
        percentile(&values, 0.95),
        values.last().copied().unwrap_or(0),
    )
}

fn report(recorder: &Recorder) {
    let (count, p50, p95, max) = summarize(recorder.views.clone());
    tracing::info!(target: "ui_timing", count, p50_us = p50, p95_us = p95, max_us = max, "view()");
    let (count, p50, p95, max) = summarize(recorder.frame_gaps.clone());
    tracing::info!(target: "ui_timing", count, p50_us = p50, p95_us = p95, max_us = max, "frame interval (view-to-view)");
    let mut kinds: Vec<&'static str> = recorder.updates.iter().map(|(name, _)| *name).collect();
    kinds.sort_unstable();
    kinds.dedup();
    let mut rows: Vec<(u64, &'static str, usize, u64, u64)> = kinds
        .into_iter()
        .map(|kind| {
            let values: Vec<u64> = recorder
                .updates
                .iter()
                .filter(|(name, _)| *name == kind)
                .map(|(_, micros)| *micros)
                .collect();
            let (count, p50, p95, max) = summarize(values);
            (max, kind, count, p50, p95)
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    let all: Vec<u64> = recorder.updates.iter().map(|(_, micros)| *micros).collect();
    let (count, p50, p95, max) = summarize(all);
    tracing::info!(target: "ui_timing", count, p50_us = p50, p95_us = p95, max_us = max, "update()");
    for (max, kind, count, p50, p95) in rows.into_iter().take(5) {
        tracing::info!(target: "ui_timing", kind, count, p50_us = p50, p95_us = p95, max_us = max, "slowest update kind");
    }
}

#[cfg(test)]
mod tests {
    use super::{message_name, percentile, summarize};

    #[derive(Debug)]
    #[allow(dead_code)]
    enum Sample {
        Plain,
        Carries(Vec<u8>),
    }

    #[test]
    fn names_are_the_variant_without_formatting_the_payload() {
        assert_eq!(message_name(&Sample::Plain), "Plain");
        assert_eq!(message_name(&Sample::Carries(vec![1, 2, 3])), "Carries");
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let values = vec![1, 2, 3, 4, 100];
        assert_eq!(percentile(&values, 0.50), 3);
        assert_eq!(percentile(&values, 0.95), 100);
        let (count, _, _, max) = summarize(vec![5, 1, 9]);
        assert_eq!((count, max), (3, 9));
    }
}
