use std::net::{IpAddr, Ipv4Addr};
use std::sync::OnceLock;
use std::time::Duration;

/// Browser-like user agent for YouTube requests; lyrics sets its own per request.
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.10 Safari/605.1.1";

/// The client for ordinary requests. One pool keeps connections to the same hosts
/// alive across modules. Tighter timeouts are set on the request; the stream engine
/// keeps its own client because it needs a per-read timeout.
pub fn shared() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            configure_http(reqwest::Client::builder())
                .connect_timeout(Duration::from_secs(6))
                .timeout(Duration::from_secs(20))
                .user_agent(DEFAULT_USER_AGENT)
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| error.clone())
}

pub fn configure_http(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let builder = if std::env::var_os("CRABYTUBE_FORCE_IPV4").is_some() {
        builder.local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
    } else {
        builder
    };
    if std::env::var_os("CRABYTUBE_HTTP1_ONLY").is_some() {
        builder.http1_only()
    } else {
        builder
    }
}

pub async fn observe_network_identity() -> Option<(String, bool, String)> {
    std::env::var_os("CRABYTUBE_NETWORK_DIAGNOSTICS")?;
    let client = configure_http(reqwest::Client::builder()).build().ok()?;
    let address = client
        .get("https://api64.ipify.org")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let parsed = address.parse::<IpAddr>().ok()?;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(parsed.to_string().as_bytes());
    let fingerprint = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
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
    Some((
        if parsed.is_ipv4() { "ipv4" } else { "ipv6" }.into(),
        proxy,
        fingerprint,
    ))
}
