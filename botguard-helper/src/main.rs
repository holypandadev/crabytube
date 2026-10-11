use std::{
    fs,
    io::{self, Read},
    path::PathBuf,
    time::{Instant, UNIX_EPOCH},
};

use rustypipe_botguard::Botguard;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_SNAPSHOT_FILES: usize = 8;
const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

#[derive(Deserialize)]
struct Request {
    protocol_version: u16,
    visitor_data: String,
    video_id: String,
    /// Content tokens to mint in this run; empty means only `video_id`.
    #[serde(default)]
    video_ids: Vec<String>,
    user_agent: String,
    ytcfg_json: String,
    homepage_challenge_json: Option<String>,
    event_id: Option<String>,
    snapshot_path: PathBuf,
    need_session: bool,
    need_content: bool,
}

#[derive(Serialize)]
struct Response {
    protocol_version: u16,
    session_po_token: Option<String>,
    /// One content token per requested video, in request order.
    content_po_tokens: Vec<String>,
    valid_until: i64,
    from_snapshot: bool,
    cold_start_ms: u128,
    token_generation_ms: u128,
    challenge_source: &'static str,
}

#[derive(Serialize)]
struct Failure {
    error: String,
}

/// Limits this process puts on itself, before it reads the request. The app's timeout for a request
/// is 20 seconds, so the deadline only ends a helper whose app has gone.
const LIMITS: helper_guard::Limits = helper_guard::Limits {
    deadline: std::time::Duration::from_secs(60),
    committed_bytes: 1 << 30,
    cpu_seconds: 60,
};

/// The BotGuard VM runs V8 through deno_core. The main thread of a Windows process has 1 MB of
/// stack, about what V8 allows itself by default, so the helper runs on a thread with the same 32
/// MiB as the JS challenge helper.
const WORKER_STACK_BYTES: usize = 32 * 1024 * 1024;

fn main() {
    helper_guard::confine(LIMITS);
    let result = match std::thread::Builder::new()
        .name("botguard".into())
        .stack_size(WORKER_STACK_BYTES)
        .spawn(run_worker)
    {
        Ok(worker) => worker
            .join()
            .unwrap_or_else(|_| Err("BotGuard worker panicked".into())),
        Err(error) => Err(format!("worker thread: {error}")),
    };
    if let Err(message) = result {
        let _ = serde_json::to_writer(io::stdout().lock(), &Failure { error: message });
        println!();
        std::process::exit(1);
    }
}

/// Runs the helper's one request on a current-thread runtime, on the worker thread.
fn run_worker() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("async runtime: {e}"))?;
    runtime.block_on(run())
}

async fn run() -> Result<(), String> {
    let raw = read_bounded(io::stdin().lock(), MAX_REQUEST_BYTES)?;
    let request: Request =
        serde_json::from_slice(&raw).map_err(|e| format!("invalid request: {e}"))?;
    drop(raw);
    if request.protocol_version != 2 {
        return Err("unsupported protocol version".into());
    }
    if request.visitor_data.is_empty() || request.video_id.is_empty() {
        return Err("visitor_data and video_id are required".into());
    }
    let has_homepage_challenge = request.homepage_challenge_json.is_some();
    if let Some(parent) = request.snapshot_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("snapshot directory: {e}"))?;
    }

    let cold = Instant::now();
    let snapshot_path = snapshot_path_for_challenge(
        &request.snapshot_path,
        request.homepage_challenge_json.as_deref(),
    );
    let builder = Botguard::builder()
        .snapshot_path_opt(Some(&snapshot_path))
        .user_agent(&request.user_agent);
    let builder = if let Some(challenge_json) = request.homepage_challenge_json {
        builder.homepage_challenge(request.ytcfg_json, challenge_json)
    } else {
        drop(request.ytcfg_json);
        builder
    };
    let mut botguard = builder
        .init()
        .await
        .map_err(|e| format!("BotGuard initialization: {e}"))?;
    drop(request.user_agent);
    drop(request.event_id);
    if std::env::var_os("CRABYTUBE_NETWORK_DIAGNOSTICS").is_some() {
        let mut client = reqwest::Client::builder();
        if std::env::var_os("CRABYTUBE_FORCE_IPV4").is_some() {
            client = client.local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        }
        if std::env::var_os("CRABYTUBE_HTTP1_ONLY").is_some() {
            client = client.http1_only();
        }
        if let Ok(client) = client.build()
            && let Ok(response) = client.get("https://api64.ipify.org").send().await
            && let Ok(address) = response.text().await
            && let Ok(parsed) = address.parse::<std::net::IpAddr>()
        {
            let digest = Sha256::digest(parsed.to_string().as_bytes());
            let fingerprint = digest[..4]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let proxy = [
                "HTTPS_PROXY",
                "https_proxy",
                "HTTP_PROXY",
                "http_proxy",
                "ALL_PROXY",
                "all_proxy",
            ]
            .iter()
            .any(|name| std::env::var_os(name).is_some());
            eprintln!(
                "network_identity_helper address_family={} proxy={} egress_fp={fingerprint}",
                if parsed.is_ipv4() { "ipv4" } else { "ipv6" },
                if proxy { "yes" } else { "no" }
            );
        }
    }
    let cold_start_ms = cold.elapsed().as_millis();
    let from_snapshot = botguard.is_from_snapshot();
    let mint = Instant::now();
    let session_po_token = if request.need_session {
        Some(
            botguard
                .mint_token(&request.visitor_data)
                .await
                .map_err(|e| format!("session token: {e}"))?,
        )
    } else {
        None
    };
    let content_ids = if request.video_ids.is_empty() {
        vec![request.video_id.clone()]
    } else {
        request.video_ids.clone()
    };
    let mut content_po_tokens = Vec::new();
    if request.need_content {
        for video_id in &content_ids {
            content_po_tokens.push(
                botguard
                    .mint_token(video_id)
                    .await
                    .map_err(|e| format!("content token: {e}"))?,
            );
        }
    }
    let token_generation_ms = mint.elapsed().as_millis();
    let valid_until = botguard.valid_until().unix_timestamp();
    let saved = botguard.write_snapshot().await;
    if saved && let Some(parent) = snapshot_path.parent() {
        trim_snapshot_cache(parent, &snapshot_path);
    }
    eprintln!(
        "botguard cold_start_ms={cold_start_ms} snapshot={from_snapshot} snapshot_written={saved} token_generation_ms={token_generation_ms}"
    );
    serde_json::to_writer(
        io::stdout().lock(),
        &Response {
            protocol_version: 2,
            session_po_token,
            content_po_tokens,
            valid_until,
            from_snapshot,
            cold_start_ms,
            token_generation_ms,
            challenge_source: if has_homepage_challenge {
                "homepage"
            } else {
                "legacy"
            },
        },
    )
    .map_err(|e| e.to_string())?;
    println!();
    Ok(())
}

fn read_bounded<R: Read>(reader: R, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut reader = reader.take(max_bytes.saturating_add(1) as u64);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > max_bytes {
        return Err(format!("request exceeds {max_bytes} byte limit"));
    }
    Ok(bytes)
}

/// Snapshots are keyed by the challenge without its `program`. The program bytes change on
/// every page load (and EVENT_ID on every launch), so a key that included either would never
/// match on the next launch. The interpreter and global name stay stable per deploy.
fn snapshot_path_for_challenge(base: &std::path::Path, challenge_json: Option<&str>) -> PathBuf {
    let stable = challenge_json
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .map(|mut challenge| {
            if let Some(object) = challenge.as_object_mut() {
                object.remove("program");
            }
            challenge.to_string()
        })
        .unwrap_or_else(|| "legacy".to_owned());
    let digest = Sha256::digest(stable.as_bytes());
    let suffix = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    base.with_file_name(format!("botguard-{suffix}.bin"))
}

fn trim_snapshot_cache(directory: &std::path::Path, current: &std::path::Path) {
    trim_snapshot_cache_to(directory, current, MAX_SNAPSHOT_FILES, MAX_SNAPSHOT_BYTES);
}

fn trim_snapshot_cache_to(
    directory: &std::path::Path,
    current: &std::path::Path,
    max_files: usize,
    max_bytes: u64,
) {
    let current_size = fs::metadata(current).map_or(0, |metadata| metadata.len());
    if current_size > max_bytes {
        let _ = fs::remove_file(current);
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut snapshots = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("botguard-") || !name.ends_with(".bin") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then_some((
                entry.path(),
                metadata.len(),
                metadata.modified().unwrap_or(UNIX_EPOCH),
            ))
        })
        .collect::<Vec<_>>();
    let mut total_bytes = snapshots
        .iter()
        .fold(0u64, |total, (_, bytes, _)| total.saturating_add(*bytes));
    snapshots.sort_by_key(|(_, _, modified)| *modified);
    let mut file_count = snapshots.len();
    for (path, bytes, _) in snapshots {
        if file_count <= max_files && total_bytes <= max_bytes {
            break;
        }
        if path == current {
            continue;
        }
        if fs::remove_file(path).is_ok() {
            file_count = file_count.saturating_sub(1);
            total_bytes = total_bytes.saturating_sub(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{read_bounded, trim_snapshot_cache_to};
    use std::{
        fs,
        io::Cursor,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn request_input_accepts_the_limit_and_rejects_one_byte_over() {
        assert_eq!(read_bounded(Cursor::new(b"1234"), 4).unwrap(), b"1234");
        assert_eq!(
            read_bounded(Cursor::new(b"12345"), 4).unwrap_err(),
            "request exceeds 4 byte limit"
        );
    }

    #[test]
    fn snapshot_cache_enforces_byte_and_file_caps_and_keeps_current() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "crabytube-botguard-trim-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let mut current = PathBuf::new();
        for index in 0..5 {
            let path = directory.join(format!("botguard-{index}.bin"));
            fs::write(&path, [index as u8; 4]).unwrap();
            if index == 4 {
                current = path;
            }
        }
        fs::write(directory.join("unrelated.bin"), b"keep").unwrap();

        trim_snapshot_cache_to(&directory, &current, 3, 10);

        let snapshots = fs::read_dir(&directory)
            .unwrap()
            .flatten()
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("botguard-") && name.ends_with(".bin")
            })
            .collect::<Vec<_>>();
        let bytes = snapshots
            .iter()
            .map(|entry| entry.metadata().unwrap().len())
            .sum::<u64>();
        assert!(snapshots.len() <= 3);
        assert!(bytes <= 10);
        assert!(current.is_file());
        assert!(directory.join("unrelated.bin").is_file());
        fs::remove_dir_all(directory).unwrap();
    }
}
