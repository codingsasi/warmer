use isahc::HttpClient;
use isahc::config::{Configurable, RedirectPolicy, SslOption};
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// When true, requests force HTTP/1.1 instead of negotiating HTTP/2.
/// Set once at startup from the resolved config; read on every request.
pub static FORCE_HTTP1: AtomicBool = AtomicBool::new(false);

static HTTP_CLIENT: OnceLock<HttpClient> = OnceLock::new();

/// Shared HTTP client with unlimited connection pool per host.
/// isahc's defaults are browser-like (~6 connections per host), which caps real
/// concurrency well below the requested `--concurrent` level in a load test.
pub fn http_client() -> &'static HttpClient {
    HTTP_CLIENT.get_or_init(|| {
        HttpClient::builder()
            .max_connections(0)
            .max_connections_per_host(0)
            .connect_timeout(Duration::from_secs(30))
            .tcp_keepalive(Duration::from_secs(60))
            .ssl_options(
                SslOption::DANGER_ACCEPT_INVALID_CERTS
                    | SslOption::DANGER_ACCEPT_REVOKED_CERTS
                    | SslOption::DANGER_ACCEPT_INVALID_HOSTS,
            )
            .redirect_policy(RedirectPolicy::Follow)
            .build()
            .expect("failed to build shared HttpClient")
    })
}
