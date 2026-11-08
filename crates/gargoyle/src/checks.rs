//! Generic HTTP/TCP probes (ARCHITECTURE §3 step 3, §9.7). Health is judged in
//! `gargoyle_core::health::evaluate_check`; this module only runs one probe and reports how it
//! went.
//!
//! An HTTP probe is one GET with the check's timeout. Redirects are never followed (a 3xx is just
//! a status), the response body is never read (the response is dropped once the status line and
//! headers are in), and proxies from the environment are ignored, so a probe only ever talks to
//! the configured target. Connections are not pooled, so every probe measures a fresh connect.
//!
//! The configured URL can carry credentials (userinfo, query), so it is never logged or put in a
//! failure reason: `reqwest` errors print their URL, so reasons are built from the error kind and
//! the innermost cause only, and logs use `CheckConfig::display_target`.

use std::error::Error as _;
use std::io;
use std::time::{Duration, Instant};

use anyhow::Context;
use gargoyle_core::config::{CheckConfig, CheckKind};
use gargoyle_core::health::ProbeOutcome;
use tokio::net::TcpStream;

/// Runs probes. One per process: the HTTP client inside is meant to be shared.
#[derive(Clone)]
pub struct Prober {
    client: reqwest::Client,
}

impl Prober {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .pool_max_idle_per_host(0)
            .user_agent(concat!("gargoyle/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("cannot set up the HTTP client for checks")?;
        Ok(Self { client })
    }

    pub async fn probe(&self, cfg: &CheckConfig) -> ProbeOutcome {
        match cfg.kind {
            CheckKind::Http => self.http(cfg).await,
            CheckKind::Tcp => tcp(cfg).await,
        }
    }

    async fn http(&self, cfg: &CheckConfig) -> ProbeOutcome {
        let Some(url) = cfg.url.as_deref() else {
            return failure("no URL configured", None);
        };
        let started = Instant::now();
        // Userinfo in the URL is sent as basic auth by reqwest.
        let result = self.client.get(url).timeout(cfg.timeout).send().await;
        let latency_ms = ms_since(started);
        match result {
            Ok(response) => {
                let status = response.status();
                // Dropping the response without reading the body closes the connection.
                drop(response);
                let expected = match cfg.expect_status {
                    Some(code) => status.as_u16() == code,
                    None => status.is_success(),
                };
                if expected {
                    ProbeOutcome::Success { latency_ms }
                } else {
                    let want = cfg
                        .expect_status
                        .map_or_else(|| "2xx".to_owned(), |c| c.to_string());
                    failure(
                        format!("unexpected status {} (expected {want})", status.as_u16()),
                        Some(latency_ms),
                    )
                }
            }
            Err(e) => failure(http_reason(&e, cfg.timeout), None),
        }
    }
}

async fn tcp(cfg: &CheckConfig) -> ProbeOutcome {
    let Some(address) = cfg.address.as_deref() else {
        return failure("no address configured", None);
    };
    let started = Instant::now();
    match tokio::time::timeout(cfg.timeout, TcpStream::connect(address)).await {
        Err(_) => failure(timed_out(cfg.timeout), None),
        Ok(Err(e)) => failure(io_reason(&e), None),
        Ok(Ok(stream)) => {
            let latency_ms = ms_since(started);
            drop(stream);
            ProbeOutcome::Success { latency_ms }
        }
    }
}

fn failure(reason: impl Into<String>, latency_ms: Option<f64>) -> ProbeOutcome {
    ProbeOutcome::Failure {
        reason: reason.into(),
        latency_ms,
    }
}

fn ms_since(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn timed_out(limit: Duration) -> String {
    if limit.subsec_millis() == 0 {
        format!("timed out after {}s", limit.as_secs())
    } else {
        format!("timed out after {}ms", limit.as_millis())
    }
}

/// A reason for a failed request that never includes the URL: `reqwest::Error`'s own `Display`
/// does, so only its kind and its innermost cause (an IO, DNS or TLS error) are used.
fn http_reason(e: &reqwest::Error, timeout: Duration) -> String {
    if e.is_timeout() {
        return timed_out(timeout);
    }
    if e.is_builder() {
        return "invalid URL".to_owned();
    }
    let mut cause: Option<&(dyn std::error::Error + 'static)> = e.source();
    let mut innermost = None;
    while let Some(c) = cause {
        if let Some(io) = c.downcast_ref::<io::Error>()
            && io.kind() != io::ErrorKind::Other
        {
            return io_reason(io);
        }
        innermost = Some(c);
        cause = c.source();
    }
    let what = if e.is_connect() {
        "connection failed"
    } else {
        "request failed"
    };
    match innermost {
        Some(c) => format!("{what}: {c}"),
        None => what.to_owned(),
    }
}

fn io_reason(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::ConnectionRefused => "connection refused".to_owned(),
        io::ErrorKind::ConnectionReset => "connection reset".to_owned(),
        io::ErrorKind::ConnectionAborted => "connection aborted".to_owned(),
        io::ErrorKind::TimedOut => "timed out".to_owned(),
        io::ErrorKind::HostUnreachable => "host unreachable".to_owned(),
        io::ErrorKind::NetworkUnreachable => "network unreachable".to_owned(),
        _ => e.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::response::{IntoResponse, Redirect};
    use axum::routing::get;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    use super::*;

    /// What the test server saw.
    #[derive(Default)]
    pub struct Seen {
        /// Requests to `/landing`, the redirect target.
        pub landing: AtomicUsize,
        /// The last `Authorization` header and request query string.
        pub auth: std::sync::Mutex<Option<String>>,
        pub query: std::sync::Mutex<Option<String>>,
    }

    /// A local HTTP server: `/ok` 200, `/fail` 500, `/slow` (2s), `/redirect` → `/landing`,
    /// `/auth` records the `Authorization` header and query.
    pub async fn http_server() -> (SocketAddr, Arc<Seen>) {
        let seen = Arc::new(Seen::default());
        let app =
            Router::new()
                .route("/ok", get(|| async { "ok" }))
                .route(
                    "/fail",
                    get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "no") }),
                )
                .route(
                    "/slow",
                    get(|| async {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        "late"
                    }),
                )
                .route(
                    "/redirect",
                    get(|| async { Redirect::temporary("/landing") }),
                )
                .route(
                    "/landing",
                    get(|State(seen): State<Arc<Seen>>| async move {
                        seen.landing.fetch_add(1, Ordering::SeqCst);
                        "landed"
                    }),
                )
                .route(
                    "/auth",
                    get(
                        |State(seen): State<Arc<Seen>>,
                         headers: HeaderMap,
                         uri: axum::http::Uri| async move {
                            *seen.auth.lock().unwrap() = headers
                                .get(header::AUTHORIZATION)
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_owned);
                            *seen.query.lock().unwrap() = uri.query().map(str::to_owned);
                            "ok".into_response()
                        },
                    ),
                )
                .with_state(seen.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (addr, seen)
    }

    /// A port with nothing listening on it (bound once, then released).
    pub async fn closed_port() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    }

    pub fn http_check(url: &str) -> CheckConfig {
        CheckConfig {
            name: "web".into(),
            kind: CheckKind::Http,
            url: Some(url.into()),
            address: None,
            expect_status: None,
            interval: Duration::from_secs(60),
            timeout: Duration::from_secs(5),
            latency_warning: None,
        }
    }

    pub fn tcp_check(address: &str) -> CheckConfig {
        CheckConfig {
            kind: CheckKind::Tcp,
            url: None,
            address: Some(address.into()),
            ..http_check("http://unused")
        }
    }

    fn reason(o: &ProbeOutcome) -> &str {
        match o {
            ProbeOutcome::Failure { reason, .. } => reason,
            ProbeOutcome::Success { .. } => panic!("expected a failure, got {o:?}"),
        }
    }

    #[tokio::test]
    async fn http_status_rules() {
        let (addr, _) = http_server().await;
        let p = Prober::new().unwrap();
        let ok = p.probe(&http_check(&format!("http://{addr}/ok"))).await;
        assert!(
            matches!(ok, ProbeOutcome::Success { latency_ms } if latency_ms > 0.0),
            "{ok:?}"
        );

        let fail = p.probe(&http_check(&format!("http://{addr}/fail"))).await;
        assert_eq!(reason(&fail), "unexpected status 500 (expected 2xx)");
        assert!(fail.latency_ms().is_some(), "a response arrived");

        let mut want_500 = http_check(&format!("http://{addr}/fail"));
        want_500.expect_status = Some(500);
        assert!(matches!(
            p.probe(&want_500).await,
            ProbeOutcome::Success { .. }
        ));

        let mut want_204 = http_check(&format!("http://{addr}/ok"));
        want_204.expect_status = Some(204);
        assert_eq!(
            reason(&p.probe(&want_204).await),
            "unexpected status 200 (expected 204)"
        );
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        let (addr, seen) = http_server().await;
        let p = Prober::new().unwrap();
        let o = p
            .probe(&http_check(&format!("http://{addr}/redirect")))
            .await;
        assert_eq!(reason(&o), "unexpected status 307 (expected 2xx)");
        assert_eq!(seen.landing.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn slow_responses_time_out() {
        let (addr, _) = http_server().await;
        let p = Prober::new().unwrap();
        let mut c = http_check(&format!("http://{addr}/slow"));
        c.timeout = Duration::from_millis(200);
        let started = Instant::now();
        let o = p.probe(&c).await;
        assert_eq!(reason(&o), "timed out after 200ms");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(o.latency_ms(), None);
    }

    #[tokio::test]
    async fn closed_ports_are_refused() {
        let addr = closed_port().await;
        let p = Prober::new().unwrap();
        let http = p.probe(&http_check(&format!("http://{addr}/"))).await;
        assert_eq!(reason(&http), "connection refused");
        let tcp = p.probe(&tcp_check(&addr.to_string())).await;
        assert_eq!(reason(&tcp), "connection refused");
    }

    #[tokio::test]
    async fn tcp_connect_succeeds_on_an_open_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let o = Prober::new()
            .unwrap()
            .probe(&tcp_check(&addr.to_string()))
            .await;
        assert!(matches!(o, ProbeOutcome::Success { .. }), "{o:?}");
    }

    #[tokio::test]
    async fn the_body_is_never_read() {
        // Headers promise a huge body that never comes; the probe must not wait for it.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = tokio::io::AsyncReadExt::read(&mut sock, &mut buf).await;
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000000\r\n\r\npartial")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        let mut c = http_check(&format!("http://{addr}/"));
        c.timeout = Duration::from_secs(2);
        let started = Instant::now();
        let o = Prober::new().unwrap().probe(&c).await;
        assert!(matches!(o, ProbeOutcome::Success { .. }), "{o:?}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn userinfo_is_sent_as_basic_auth_and_kept_out_of_reasons() {
        let (addr, seen) = http_server().await;
        let p = Prober::new().unwrap();
        let o = p
            .probe(&http_check(&format!(
                "http://user:pass@{addr}/auth?token=abc"
            )))
            .await;
        assert!(matches!(o, ProbeOutcome::Success { .. }), "{o:?}");
        // base64("user:pass")
        assert_eq!(
            seen.auth.lock().unwrap().as_deref(),
            Some("Basic dXNlcjpwYXNz")
        );
        assert_eq!(seen.query.lock().unwrap().as_deref(), Some("token=abc"));

        // Failure reasons for the same kind of URL carry no part of it.
        let closed = closed_port().await;
        let o = p
            .probe(&http_check(&format!(
                "http://user:pass@{closed}/x?token=abc"
            )))
            .await;
        let r = reason(&o);
        assert!(!r.contains("pass") && !r.contains("token"), "{r}");
    }

    #[test]
    fn timeouts_print_whole_seconds_or_ms() {
        assert_eq!(timed_out(Duration::from_secs(5)), "timed out after 5s");
        assert_eq!(
            timed_out(Duration::from_millis(1500)),
            "timed out after 1500ms"
        );
    }
}
