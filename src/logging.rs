//! Where the app's log lines go. On Linux they go to stdout, as they always have. Windows and
//! macOS windowed builds have no terminal to show them, so their lines go to `crabytube.log` in the
//! local data folder, with URLs redacted. A log that has grown past `MAX_LOG_BYTES` is moved to
//! `crabytube.log.1`, which keeps one earlier copy, when the app starts and while it runs: kept in
//! the tray, it may run for weeks.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

/// A log file larger than this is moved aside.
pub const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Installs the global subscriber. Call it once, first thing in `main`.
pub fn init() {
    // Third-party crates log warnings only by default; the app's own info lines stay visible. The
    // PulseAudio client logs an error each time an output is closed, which is the normal end of a
    // stream: a real failure reaches the app as a stream error and is logged there.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,crabytube=info,pulseaudio::client::reactor=off"));
    match open_log_file() {
        Some(file) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .init(),
        None => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
}

/// The log file for this run. `None` on Linux, in diagnostics builds (whose output is read from the
/// console) and when the file cannot be opened. The lines then go to stdout, the default of `fmt()`.
fn open_log_file() -> Option<RedactingFile> {
    if cfg!(any(target_os = "linux", feature = "diagnostics")) {
        return None;
    }
    let directory = crate::app_dirs::data_local_dir()?
        .join("CrabyTube")
        .join("logs");
    fs::create_dir_all(&directory).ok()?;
    RedactingFile::open(directory.join("crabytube.log"), MAX_LOG_BYTES)
}

/// Moves `path` to `path.1` when it is larger than `max_bytes`, replacing the copy there. A failed
/// move is ignored: the log is written to either way.
fn rotate_if_larger(path: &Path, max_bytes: u64) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() <= max_bytes {
        return;
    }
    let mut previous = path.as_os_str().to_owned();
    previous.push(".1");
    let _ = fs::rename(path, previous);
}

/// Writes whole lines with their URLs redacted. A line is redacted before it is written, so a URL
/// that arrives in two pieces is still caught. Past `max_bytes` the file is moved aside and a new
/// one started.
struct RedactingFile {
    file: File,
    path: PathBuf,
    max_bytes: u64,
    /// The length of the file, as far as this writer knows.
    written: u64,
    pending: Vec<u8>,
}

impl RedactingFile {
    fn open(path: PathBuf, max_bytes: u64) -> Option<Self> {
        rotate_if_larger(&path, max_bytes);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        let written = file.metadata().map_or(0, |metadata| metadata.len());
        Some(Self {
            file,
            path,
            max_bytes,
            written,
            pending: Vec::new(),
        })
    }

    /// Moves the full file aside and goes on in a new one. When the move fails, the same file goes
    /// on growing until the next attempt, `max_bytes` later.
    fn start_again(&mut self) {
        let _ = self.file.flush();
        rotate_if_larger(&self.path, self.max_bytes);
        if let Ok(file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            self.file = file;
        }
        self.written = 0;
    }
}

impl Write for RedactingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        while let Some(end) = self.pending.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = redact(&line);
            self.file.write_all(line.as_bytes())?;
            self.written += line.len() as u64;
            if self.written > self.max_bytes {
                self.start_again();
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            self.file.write_all(redact(&rest).as_bytes())?;
        }
        self.file.flush()
    }
}

/// The app's own redaction of URLs, the same one the playback errors use.
fn redact(bytes: &[u8]) -> String {
    crate::playback::redact_urls(&String::from_utf8_lossy(bytes))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("crabytube-log-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_log_within_the_limit_is_left_where_it_is() {
        let dir = scratch("within");
        let path = dir.join("crabytube.log");
        fs::write(&path, vec![b'x'; 100]).unwrap();
        rotate_if_larger(&path, 1024);
        assert_eq!(fs::read(&path).unwrap().len(), 100);
        assert!(!dir.join("crabytube.log.1").exists());
    }

    #[test]
    fn a_log_past_the_limit_moves_aside_and_replaces_the_earlier_copy() {
        let dir = scratch("past");
        let path = dir.join("crabytube.log");
        let earlier = dir.join("crabytube.log.1");
        fs::write(&earlier, b"an older run").unwrap();
        fs::write(&path, vec![b'y'; 2048]).unwrap();
        rotate_if_larger(&path, 1024);
        assert!(!path.exists());
        assert_eq!(fs::read(&earlier).unwrap(), vec![b'y'; 2048]);
    }

    #[test]
    fn a_missing_log_has_nothing_to_move() {
        let dir = scratch("missing");
        rotate_if_larger(&dir.join("crabytube.log"), 1024);
        assert!(!dir.join("crabytube.log.1").exists());
    }

    #[test]
    fn a_log_that_cannot_be_moved_is_kept_and_does_not_stop_the_app() {
        let dir = scratch("blocked");
        let path = dir.join("crabytube.log");
        fs::write(&path, vec![b'z'; 2048]).unwrap();
        // A directory where the earlier copy belongs makes the move fail on every platform.
        fs::create_dir(dir.join("crabytube.log.1")).unwrap();
        rotate_if_larger(&path, 1024);
        assert_eq!(fs::read(&path).unwrap().len(), 2048);
    }

    #[test]
    fn urls_are_redacted_even_when_a_line_arrives_in_pieces() {
        let dir = scratch("redact");
        let path = dir.join("crabytube.log");
        let mut writer = RedactingFile::open(path.clone(), MAX_LOG_BYTES).unwrap();
        writer
            .write_all(b"WARN stream failed: https://rr1.googlevideo.com/videoplayback?")
            .unwrap();
        writer.write_all(b"sig=SECRET&expire=1\n").unwrap();
        writer.write_all(b"INFO next line").unwrap();
        writer.flush().unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("SECRET"), "{text}");
        assert!(text.contains("stream failed: <url>\n"), "{text}");
        assert!(text.ends_with("INFO next line"), "{text}");
    }

    #[test]
    fn a_log_that_grows_past_the_limit_while_the_app_runs_starts_again() {
        let dir = scratch("running");
        let path = dir.join("crabytube.log");
        let mut writer = RedactingFile::open(path.clone(), 100).unwrap();
        for line in 0..30 {
            writeln!(writer, "INFO line {line}").unwrap();
        }
        writer.flush().unwrap();
        let current = fs::read_to_string(&path).unwrap();
        let earlier = fs::read_to_string(dir.join("crabytube.log.1")).unwrap();
        assert!(current.len() <= 100 + 20, "{current}");
        assert!(earlier.len() <= 100 + 20, "{earlier}");
        assert!(current.ends_with("INFO line 29\n"), "{current}");
    }
}
