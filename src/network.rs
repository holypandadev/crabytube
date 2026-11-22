use std::future::Future;
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

/// The same client bound to IPv4, for the second try at a media server that does not answer.
pub fn shared_ipv4() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            ipv4_only(configure_http(reqwest::Client::builder()))
                .connect_timeout(Duration::from_secs(6))
                .timeout(Duration::from_secs(20))
                .user_agent(DEFAULT_USER_AGENT)
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| error.clone())
}

/// Binds a client to IPv4, so its connections never take an IPv6 route.
pub fn ipv4_only(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder.local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// Whether every client is bound to IPv4 already, by the troubleshooting switch.
pub fn ipv4_forced() -> bool {
    std::env::var_os("CRABYTUBE_FORCE_IPV4").is_some()
}

/// How long a media server gets to answer before the same request also goes out over IPv4. Some
/// media servers accept a connection over IPv6 and then never answer, while they answer over IPv4
/// at once; the connection having been made, nothing else falls back.
pub const MEDIA_HEDGE_DELAY: Duration = Duration::from_millis(1_500);

/// How long a media server that answered only over IPv4 is asked over IPv4 first.
const IPV4_PREFERENCE: Duration = Duration::from_secs(30 * 60);

/// The media servers that answered only over IPv4, and when each did.
fn ipv4_nodes() -> &'static std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>
{
    static NODES: OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    > = OnceLock::new();
    NODES.get_or_init(Default::default)
}

/// The machine behind a media host: `rr3---sn-abc.googlevideo.com` and `rr1---sn-abc…` are doors
/// to the same one.
fn media_node(host: &str) -> &str {
    host.split_once("---").map_or(host, |(_, node)| node)
}

/// Whether `host` answered only over IPv4 a while ago, so it is asked over IPv4 first.
pub fn prefers_ipv4(host: &str) -> bool {
    let Ok(mut nodes) = ipv4_nodes().lock() else {
        return false;
    };
    nodes.retain(|_, since| since.elapsed() < IPV4_PREFERENCE);
    nodes.contains_key(media_node(host))
}

/// Notes that `host` answered over IPv4 while the first request to it did not.
pub fn prefer_ipv4(host: &str) {
    if let Ok(mut nodes) = ipv4_nodes().lock() {
        nodes.insert(media_node(host).to_owned(), std::time::Instant::now());
    }
}

/// Forgets that `host` is asked over IPv4 first, after it failed there too.
pub fn forget_ipv4(host: &str) {
    if let Ok(mut nodes) = ipv4_nodes().lock() {
        nodes.remove(media_node(host));
    }
}

/// Sends a request and, when it has not answered within `delay` or has failed, the alternative
/// one beside it; the first answer wins. Returns the answer and whether it is the alternative's.
/// With both failed, the error is the first request's.
pub async fn hedged<F, G, A>(
    first: F,
    alternative: A,
    delay: Duration,
) -> (Result<reqwest::Response, reqwest::Error>, bool)
where
    F: Future<Output = Result<reqwest::Response, reqwest::Error>>,
    G: Future<Output = Result<reqwest::Response, reqwest::Error>>,
    A: FnOnce() -> G,
{
    let mut first = std::pin::pin!(first);
    let mut first_error = match tokio::time::timeout(delay, &mut first).await {
        Ok(Ok(response)) => return (Ok(response), false),
        Ok(Err(error)) => Some(error),
        Err(_) => None,
    };
    let mut second = std::pin::pin!(alternative());
    if first_error.is_none() {
        tokio::select! {
            result = &mut first => match result {
                Ok(response) => return (Ok(response), false),
                Err(error) => first_error = Some(error),
            },
            result = &mut second => match result {
                Ok(response) => return (Ok(response), true),
                // The alternative failed first: the first request still has its own time.
                Err(_) => return (first.await, false),
            },
        }
    }
    match second.await {
        Ok(response) => (Ok(response), true),
        Err(error) => (Err(first_error.unwrap_or(error)), false),
    }
}

/// Sends a media request to `url`'s server: over IPv4 at once when the server answered only there
/// lately, otherwise with `primary` and, should it not answer in time, with `ipv4` beside it.
/// `request` builds the request for a client.
pub async fn send_media<R>(
    url: &reqwest::Url,
    primary: &reqwest::Client,
    ipv4: &reqwest::Client,
    request: R,
) -> Result<reqwest::Response, reqwest::Error>
where
    R: Fn(&reqwest::Client) -> reqwest::RequestBuilder,
{
    let host = url.host_str().unwrap_or_default();
    if ipv4_forced() {
        return request(primary).send().await;
    }
    if prefers_ipv4(host) {
        let result = request(ipv4).send().await;
        if result.is_err() {
            forget_ipv4(host);
        }
        return result;
    }
    let first = async {
        // Diagnostics only: CRABYTUBE_STALL_MEDIA=1 makes the first request never answer, as a
        // server reached over a broken route does.
        #[cfg(feature = "diagnostics")]
        if std::env::var_os("CRABYTUBE_STALL_MEDIA").is_some() {
            std::future::pending::<()>().await;
        }
        request(primary).send().await
    };
    let (result, by_ipv4) = hedged(first, || request(ipv4).send(), MEDIA_HEDGE_DELAY).await;
    if by_ipv4 {
        tracing::warn!("media server answered over IPv4 only; asking it over IPv4 for a while");
        prefer_ipv4(host);
    }
    result
}

pub fn configure_http(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let builder = builder.http2_max_header_list_size(MAX_RESPONSE_HEADER_BYTES);
    let builder = if ipv4_forced() {
        ipv4_only(builder)
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

    /// Accepts connections and never answers them, as a server reached over a broken route does.
    async fn serve_silence() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        address
    }

    fn plain_client() -> reqwest::Client {
        reqwest::Client::builder()
            .http2_prior_knowledge()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn a_server_that_never_answers_gives_way_to_the_alternative_after_the_delay() {
        let silent = serve_silence().await;
        let answering = serve_large_headers(16).await;
        let client = plain_client();
        let started = std::time::Instant::now();
        let (result, by_alternative) = hedged(
            client.get(format!("http://{silent}/")).send(),
            || client.get(format!("http://{answering}/")).send(),
            Duration::from_millis(150),
        )
        .await;
        assert!(by_alternative);
        assert_eq!(result.unwrap().text().await.unwrap(), "ok");
        // The answer came with the alternative, long before the first request's own limit.
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_prompt_answer_never_starts_the_alternative() {
        let answering = serve_large_headers(16).await;
        let client = plain_client();
        let (result, by_alternative) = hedged(
            client.get(format!("http://{answering}/")).send(),
            || async { panic!("the alternative was started") },
            Duration::from_secs(2),
        )
        .await;
        assert!(!by_alternative);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn a_first_request_that_fails_is_tried_again_and_its_error_kept_when_both_fail() {
        let answering = serve_large_headers(16).await;
        let client = plain_client();
        // Port 9 of the loopback address refuses connections.
        let refused = "http://127.0.0.1:9/";
        let (result, by_alternative) = hedged(
            client.get(refused).send(),
            || client.get(format!("http://{answering}/")).send(),
            Duration::from_secs(2),
        )
        .await;
        assert!(by_alternative && result.is_ok());
        let (result, by_alternative) = hedged(
            client.get(refused).send(),
            || client.get("http://127.0.0.1:9/again").send(),
            Duration::from_secs(2),
        )
        .await;
        assert!(!by_alternative);
        assert!(result.unwrap_err().url().unwrap().path() == "/");
    }

    #[test]
    fn a_node_is_asked_over_ipv4_first_by_any_of_its_doors_until_it_fails_there() {
        let host = "rr3---sn-test-node.googlevideo.com";
        assert!(!prefers_ipv4(host));
        prefer_ipv4(host);
        assert!(prefers_ipv4("rr1---sn-test-node.googlevideo.com"));
        assert!(!prefers_ipv4("rr3---sn-other-node.googlevideo.com"));
        forget_ipv4("rr5---sn-test-node.googlevideo.com");
        assert!(!prefers_ipv4(host));
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
