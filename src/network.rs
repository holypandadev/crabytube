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

/// The most response header bytes accepted over HTTP/2. The HTTP/2 client stops at 16 KiB unless
/// told otherwise, and YouTube's pages send more than that in report-only policy headers; a
/// response past the limit fails as if the network were down.
pub const MAX_RESPONSE_HEADER_BYTES: u32 = 256 * 1024;

pub fn configure_http(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let builder = builder.http2_max_header_list_size(MAX_RESPONSE_HEADER_BYTES);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Serves one HTTP/2 connection whose responses carry a header of `header_bytes`.
    async fn serve_large_headers(header_bytes: usize) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            while let Some(Ok((_request, mut respond))) = connection.accept().await {
                let response = http::Response::builder()
                    .status(200)
                    .header("x-report-only", "a".repeat(header_bytes))
                    .body(())
                    .unwrap();
                let mut body = respond.send_response(response, false).unwrap();
                body.send_data(bytes::Bytes::from_static(b"ok"), true)
                    .unwrap();
            }
        });
        address
    }

    #[tokio::test]
    async fn responses_with_headers_past_sixteen_kib_are_read() {
        let address = serve_large_headers(24 * 1024).await;
        let client = configure_http(reqwest::Client::builder())
            .http2_prior_knowledge()
            .build()
            .unwrap();
        let response = client
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("a 24 KiB header is accepted");
        assert_eq!(response.text().await.unwrap(), "ok");
    }
}
