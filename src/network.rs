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
