use std::io::{self, Read};
use std::path::Path;
use std::process;

use serde::{Deserialize, Serialize};
use ytdlp_ejs::{JsChallengeRequest, JsChallengeResponse, JsChallengeType, RuntimeType};

mod oxc_preprocessor;

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_PLAYER_JS_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PREPROCESSED_BYTES: u64 = 8 * 1024 * 1024;
/// Layout of the preprocessed cache file. The app names files by version too.
const PREPROCESSED_CACHE_VERSION: u8 = 3;
const PROTOCOL_VERSION: u16 = 2;

#[derive(Serialize, Deserialize)]
struct PreprocessedCache {
    version: u8,
    source_fp: String,
    preprocessed: String,
}

/// Paths keep megabytes of player JavaScript out of the pipe; the helper reads the files itself.
#[derive(Deserialize)]
struct Request {
    protocol_version: u16,
    /// Raw player JavaScript to preprocess.
    player_js_path: Option<String>,
    /// A preprocessed cache file from an earlier run, used when present.
    preprocessed_cache_path: Option<String>,
    /// Fingerprint of the raw source, which a cache file must match.
    source_fp: Option<String>,
    /// Where a newly preprocessed cache file is written.
    output_cache_path: Option<String>,
    signature: Option<String>,
    n: Option<String>,
}

#[derive(Serialize)]
struct Response {
    protocol_version: u16,
    signature: Option<String>,
    n: Option<String>,
    /// True when the helper wrote a new preprocessed cache file to the requested path.
    preprocessed_written: bool,
    peak_rss_kib: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_profile: Option<Vec<MemoryStage>>,
}

#[derive(Serialize)]
struct MemoryStage {
    stage: &'static str,
    rss_kib: Option<u64>,
}

#[derive(Serialize)]
struct Failure {
    error: String,
}

/// Limits this process puts on itself, before it reads the request. The app's timeout for a request
/// is 15 seconds, so the deadline only ends a helper whose app has gone.
const LIMITS: helper_guard::Limits = helper_guard::Limits {
    deadline: std::time::Duration::from_secs(60),
    committed_bytes: 1 << 30,
    cpu_seconds: 60,
};

fn main() {
    helper_guard::confine(LIMITS);
    let result = std::thread::Builder::new()
        .name("js-challenge-oxc".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(run)
        .and_then(|thread| {
            thread
                .join()
                .map_err(|_| io::Error::other("JavaScript challenge worker panicked"))?
                .map_err(io::Error::other)
        });

    if let Err(error) = result {
        let failure = Failure {
            error: error.to_string(),
        };
        let _ = serde_json::to_writer(io::stdout().lock(), &failure);
        println!();
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let raw = read_bounded(io::stdin().lock(), MAX_REQUEST_BYTES)?;
    let request: Request = serde_json::from_slice(&raw).map_err(|error| error.to_string())?;
    drop(raw);
    let mut memory_profile = memory_profile_enabled().then(Vec::new);
    record_memory(&mut memory_profile, "request_parsed");

    let response = solve(request, &mut memory_profile)?;
    serde_json::to_writer(io::stdout().lock(), &response).map_err(|error| error.to_string())?;
    println!();
    Ok(())
}

fn solve(
    request: Request,
    memory_profile: &mut Option<Vec<MemoryStage>>,
) -> Result<Response, String> {
    let Request {
        protocol_version,
        player_js_path,
        preprocessed_cache_path,
        source_fp,
        output_cache_path,
        signature,
        n,
    } = request;
    if protocol_version != PROTOCOL_VERSION {
        return Err("unsupported challenge protocol".into());
    }
    let want_signature = signature.is_some();
    let want_n = n.is_some();

    let mut requests = Vec::with_capacity(2);
    if let Some(signature) = signature {
        requests.push(JsChallengeRequest {
            challenge_type: JsChallengeType::Sig,
            challenges: vec![signature],
        });
    }
    if let Some(n) = n {
        requests.push(JsChallengeRequest {
            challenge_type: JsChallengeType::N,
            challenges: vec![n],
        });
    }
    if requests.is_empty() {
        return Err("no signature or n challenge was provided".into());
    }

    record_memory(memory_profile, "preprocess_started");
    let cached = preprocessed_cache_path
        .as_deref()
        .filter(|path| Path::new(path).is_file())
        .map(|path| read_preprocessed_cache(Path::new(path), source_fp.as_deref()))
        .transpose()?;
    let (preprocessed, preprocessed_written) = if let Some(code) = cached {
        (code, false)
    } else {
        let path = player_js_path
            .as_deref()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| "missing player source".to_owned())?;
        let player = read_player_js(Path::new(path))?;
        let code = oxc_preprocessor::preprocess_player(&player)?;
        drop(player);
        record_memory(memory_profile, "preprocess_finished");
        trim_released_allocator_pages();
        record_memory(memory_profile, "allocator_trimmed");
        let written = match (output_cache_path.as_deref(), source_fp.as_deref()) {
            (Some(output), Some(fp)) => {
                write_preprocessed_cache(Path::new(output), fp, &code).is_ok()
            }
            _ => false,
        };
        (code, written)
    };

    record_memory(memory_profile, "runtime_started");
    let mut provider = RuntimeType::QuickJS
        .create_provider(&preprocessed)
        .map_err(|error| error.to_string())?;
    record_memory(memory_profile, "runtime_ready");
    let responses = requests
        .iter()
        .map(|request| {
            match provider.solve_challenges(&request.challenge_type, &request.challenges) {
                Ok(data) => JsChallengeResponse::Result { data },
                Err(error) => JsChallengeResponse::Error {
                    error: error.to_string(),
                },
            }
        })
        .collect::<Vec<_>>();
    record_memory(memory_profile, "solver_finished");

    let mut responses = responses.into_iter();
    let signature = if want_signature {
        Some(response_value(responses.next())?)
    } else {
        None
    };
    let n = if want_n {
        Some(response_value(responses.next())?)
    } else {
        None
    };
    if signature.as_deref().is_some_and(str::is_empty) || n.as_deref().is_some_and(str::is_empty) {
        return Err("challenge solver returned an empty transform".into());
    }
    record_memory(memory_profile, "response_extracted");

    Ok(Response {
        protocol_version: PROTOCOL_VERSION,
        signature,
        n,
        preprocessed_written,
        peak_rss_kib: peak_rss_kib(),
        memory_profile: memory_profile.take(),
    })
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn trim_released_allocator_pages() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }

    // The OXC parser releases its AST before the QuickJS runtime is built.
    // Ask glibc to return now-free pages before the runtime's peak begins.
    let _ = unsafe { malloc_trim(0) };
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn trim_released_allocator_pages() {}

fn response_value(response: Option<JsChallengeResponse>) -> Result<String, String> {
    match response.ok_or_else(|| "challenge response missing".to_owned())? {
        JsChallengeResponse::Result { data } => data
            .into_values()
            .next()
            .ok_or_else(|| "challenge transform missing".into()),
        JsChallengeResponse::Error { error } => Err(error),
    }
}

fn memory_profile_enabled() -> bool {
    matches!(std::env::var("CRABYTUBE_JS_PROFILE").as_deref(), Ok("1"))
}

fn record_memory(profile: &mut Option<Vec<MemoryStage>>, stage: &'static str) {
    if let Some(profile) = profile {
        profile.push(MemoryStage {
            stage,
            rss_kib: current_rss_kib(),
        });
    }
}

fn read_player_js(path: &Path) -> Result<String, String> {
    let length = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .len();
    if length == 0 || length > MAX_PLAYER_JS_BYTES {
        return Err("player source is empty or too large".into());
    }
    std::fs::read_to_string(path).map_err(|error| error.to_string())
}

/// Reads a cache file written by `write_preprocessed_cache`; a mismatch makes the caller
/// fall back to the raw source.
fn read_preprocessed_cache(path: &Path, source_fp: Option<&str>) -> Result<String, String> {
    let length = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .len();
    if length == 0 || length > MAX_PREPROCESSED_BYTES * 2 {
        return Err("preprocessed cache has an unusable size".into());
    }
    let cache: PreprocessedCache =
        serde_json::from_slice(&std::fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if cache.version != PREPROCESSED_CACHE_VERSION
        || source_fp.is_some_and(|fp| fp != cache.source_fp)
        || cache.preprocessed.is_empty()
    {
        return Err("preprocessed cache does not match this player source".into());
    }
    Ok(cache.preprocessed)
}

/// Writes through a temporary file so a crash never leaves a truncated cache behind.
fn write_preprocessed_cache(path: &Path, source_fp: &str, code: &str) -> Result<(), String> {
    if code.len() as u64 > MAX_PREPROCESSED_BYTES {
        return Err("preprocessed player is too large to cache".into());
    }
    let bytes = serde_json::to_vec(&PreprocessedCache {
        version: PREPROCESSED_CACHE_VERSION,
        source_fp: source_fp.to_owned(),
        preprocessed: code.to_owned(),
    })
    .map_err(|error| error.to_string())?;
    let temporary = temporary_beside(path);
    if let Err(error) = std::fs::write(&temporary, bytes) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        error.to_string()
    })
}

/// A temporary name beside `path` that only this process uses, so two writers never share one.
fn temporary_beside(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}

fn read_bounded<R: Read>(reader: R, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut reader = reader.take(max_bytes.saturating_add(1) as u64);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > max_bytes {
        return Err(format!("challenge request exceeds {max_bytes} byte limit"));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
    })
}

#[cfg(target_os = "linux")]
fn current_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
    })
}

#[cfg(not(target_os = "linux"))]
fn peak_rss_kib() -> Option<u64> {
    None
}

#[cfg(not(target_os = "linux"))]
fn current_rss_kib() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::{Request, read_bounded, response_value, solve, temporary_beside};
    use std::collections::HashMap;

    #[test]
    fn a_temporary_file_is_named_for_its_writer_beside_the_target() {
        let target = std::path::Path::new("/cache/player-abc.json");
        let temporary = temporary_beside(target);
        assert_eq!(temporary.parent(), target.parent());
        assert_ne!(temporary, target);
        assert!(
            temporary
                .to_string_lossy()
                .contains(&std::process::id().to_string())
        );
    }
    use std::io::Cursor;
    use ytdlp_ejs::JsChallengeResponse;

    #[test]
    fn request_input_is_bounded_without_rejecting_the_limit() {
        assert_eq!(read_bounded(Cursor::new(b"12345"), 5).unwrap(), b"12345");
        let error = read_bounded(Cursor::new(b"123456"), 5).unwrap_err();
        assert!(error.contains("exceeds 5 byte limit"));
    }

    #[test]
    fn response_value_extracts_a_success_without_round_tripping_json() {
        let data = HashMap::from([("challenge".to_owned(), "transformed".to_owned())]);
        assert_eq!(
            response_value(Some(JsChallengeResponse::Result { data })).unwrap(),
            "transformed"
        );
    }

    #[test]
    fn response_value_preserves_solver_errors_and_rejects_empty_results() {
        let error = response_value(Some(JsChallengeResponse::Error {
            error: "solver failed".into(),
        }))
        .unwrap_err();
        assert_eq!(error, "solver failed");
        assert!(
            response_value(Some(JsChallengeResponse::Result {
                data: HashMap::new(),
            }))
            .is_err()
        );
    }

    #[test]
    fn empty_player_source_is_rejected_before_preprocessing() {
        let path = std::env::temp_dir().join(format!("js-helper-empty-{}.js", std::process::id()));
        std::fs::write(&path, b"").unwrap();
        let mut memory_profile = None;
        let result = solve(
            Request {
                protocol_version: 2,
                player_js_path: Some(path.to_string_lossy().into_owned()),
                preprocessed_cache_path: None,
                source_fp: None,
                output_cache_path: None,
                signature: Some("challenge".into()),
                n: None,
            },
            &mut memory_profile,
        );
        let _ = std::fs::remove_file(&path);
        assert!(matches!(result, Err(error) if error == "player source is empty or too large"));
    }
}
