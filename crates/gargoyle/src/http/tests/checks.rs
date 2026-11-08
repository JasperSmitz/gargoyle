//! `GET /api/v1/checks` and `/api/v1/checks/{name}` (M4.2), driven by real probes against local
//! test servers.

use std::io::Write;
use std::sync::Mutex;

use gargoyle_core::api::{CheckDetail, CheckList};
use gargoyle_core::health::Health;

use super::*;
use crate::checks::Prober;
use crate::checks::tests::{closed_port, http_server};
use crate::scheduler::check_tick;

/// A config with the given `[[check]]` tables and a database in `dir`.
fn config_with(dir: &tempfile::TempDir, checks: &str) -> Config {
    let mut config = Config::from_toml_str(&format!(
        "[collector]\nmode = \"mock\"\njournal_backfill = false\n{checks}"
    ))
    .unwrap();
    config.storage.path = dir.path().join("g.db");
    config
}

fn shared_with(config: Config) -> Shared {
    let storage = crate::storage::Storage::open(&config.storage).unwrap();
    Shared::new(config, Arc::new(MockCollector::new(1))).with_storage(storage)
}

/// Probes check `index` `n` times.
async fn probe(shared: &Shared, index: usize, n: usize) {
    let prober = Prober::new().unwrap();
    let cfg = shared.config.checks[index].clone();
    for _ in 0..n {
        check_tick(shared, &prober, &cfg).await;
    }
}

#[tokio::test]
async fn checks_before_any_probe_are_unknown() {
    let (status, body) = get_json(router(shared()), "/api/v1/checks").await;
    assert_eq!(status, StatusCode::OK);
    assert_same_shape(&body, &example("checks.json"), "checks");
    let list: CheckList = serde_json::from_value(body).unwrap();
    let names: Vec<_> = list.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["Gargoyle API", "Closed port (demo)"],
        "config order"
    );
    for c in &list.checks {
        assert_eq!(c.health, Health::Unknown);
        assert_eq!(c.message.as_deref(), Some("not yet run"));
        assert_eq!((c.last_check, c.latency_ms), (None, None));
    }
    assert_eq!(list.checks[0].target, "http://127.0.0.1:9077/api/v1/health");
    assert_eq!(list.checks[1].target, "127.0.0.1:9");
}

#[tokio::test]
async fn healthy_http_and_failing_tcp_checks() {
    let (addr, _) = http_server().await;
    let closed = closed_port().await;
    let dir = tempfile::tempdir().unwrap();
    let shared = shared_with(config_with(
        &dir,
        &format!(
            "[[check]]\nname = \"Web\"\nkind = \"http\"\nurl = \"http://{addr}/ok\"\n\
             [[check]]\nname = \"Game port\"\nkind = \"tcp\"\naddress = \"{closed}\"\n"
        ),
    ));
    probe(&shared, 0, 1).await;

    // ROADMAP M4: warning → critical after 3 failures, and live health is not held down.
    let mut seen = Vec::new();
    for _ in 0..4 {
        probe(&shared, 1, 1).await;
        let c = shared.read(|s| s.checks_view(&shared.config, now())[1].clone());
        seen.push((c.health, c.consecutive_failures));
    }
    assert_eq!(
        seen,
        [
            (Health::Warning, 1),
            (Health::Warning, 2),
            (Health::Critical, 3),
            (Health::Critical, 4)
        ]
    );

    let router = router(shared.clone());
    let (status, body) = get_json(router.clone(), "/api/v1/checks").await;
    assert_eq!(status, StatusCode::OK);
    assert_same_shape(&body, &example("checks.json"), "checks");
    let list: CheckList = serde_json::from_value(body).unwrap();
    let web = &list.checks[0];
    assert_eq!(
        (web.health, web.message.as_deref()),
        (Health::Healthy, None)
    );
    assert!(web.latency_ms.is_some() && web.last_check.is_some());

    // The name is percent-decoded.
    let (status, body) = get_json(router.clone(), "/api/v1/checks/Game%20port").await;
    assert_eq!(status, StatusCode::OK);
    assert_same_shape(&body, &example("check-detail.json"), "check-detail");
    let detail: CheckDetail = serde_json::from_value(body).unwrap();
    assert_eq!(detail.check.target, closed.to_string());
    assert_eq!(detail.check.latency_ms, None);
    assert_eq!(
        detail.check.message.as_deref(),
        Some("connection refused (4 consecutive failures)")
    );
    // Checks have no event hold-down: each state is logged when first seen. Newest first.
    let events: Vec<_> = detail
        .events
        .iter()
        .map(|e| (e.target_kind, e.from, e.to, e.message.as_deref()))
        .collect();
    assert_eq!(
        events,
        [
            (
                TargetKind::Check,
                Some(Health::Warning),
                Health::Critical,
                Some("connection refused (3 consecutive failures)")
            ),
            (
                TargetKind::Check,
                None,
                Health::Warning,
                Some("connection refused (1 consecutive failure)")
            ),
        ]
    );

    // Checks count in the overview.
    let (_, body) = get_json(router.clone(), "/api/v1/overview").await;
    let o: Overview = serde_json::from_value(body).unwrap();
    assert_eq!(o.checks.len(), 2);
    assert_eq!(o.health, Health::Critical);
    assert_eq!(o.counts.critical, 1);
    assert_eq!(o.counts.healthy, 1);

    // Latency is stored for the probe that got a response only.
    let (_, body) = get_json(router, "/api/v1/metrics").await;
    let keys: Vec<_> = body["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["key"].as_str().unwrap().to_owned(), s["unit"].clone()))
        .collect();
    assert!(
        keys.contains(&("check.latency:Web".into(), "ms".into())),
        "{keys:?}"
    );
    assert!(!keys.iter().any(|(k, _)| k == "check.latency:Game port"));
}

#[tokio::test]
async fn unknown_check_is_json_404() {
    for uri in ["/api/v1/checks/nope", "/api/v1/checks/Gargoyle%20APIx"] {
        let (status, body) = get_json(router(shared()), uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_same_shape(&body, &example("error.json"), "error");
        assert_eq!(body["error"]["code"], "not_found");
    }
    let (status, body) = get_json(router(shared()), "/api/v1/checks/Gargoyle%20API").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["check"]["name"], "Gargoyle API");
}

/// Collects formatted log lines.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Installs a process-wide subscriber (down to debug) that writes into the returned buffer, and
/// forwards `log` records (which `reqwest` uses) into it. It has to be global: with a
/// thread-local default, callsites first hit by concurrently running tests can cache "not
/// interested" and drop events. Other tests' lines land in the buffer too, which is harmless.
fn capture_logs() -> Captured {
    use tracing_subscriber::util::SubscriberInitExt;
    let logs = Captured::default();
    let writer = logs.clone();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish()
        .try_init()
        .expect("no other test installs a global subscriber");
    logs
}

/// ROADMAP M4 acceptance: a URL configured with `user:pass@` and a `?token=` never appears in any
/// API response, nor in log lines (captured down to debug, stricter than the default `info`),
/// whether the probe succeeds or fails.
#[tokio::test]
async fn check_url_secrets_never_leave_the_process() {
    const SECRETS: [&str; 4] = ["gargoyleuser", "s3cr3tpw", "tok3nvalue", "fr4gment"];
    let logs = capture_logs();

    let (addr, seen) = http_server().await;
    let closed = closed_port().await;
    let secret_url = |authority: String, path: &str| {
        format!("http://gargoyleuser:s3cr3tpw@{authority}{path}?token=tok3nvalue#fr4gment")
    };
    let dir = tempfile::tempdir().unwrap();
    let shared = shared_with(config_with(
        &dir,
        &format!(
            "[[check]]\nname = \"Good\"\nkind = \"http\"\nurl = \"{}\"\n\
             [[check]]\nname = \"Bad\"\nkind = \"http\"\nurl = \"{}\"\n\
             [[check]]\nname = \"Down\"\nkind = \"http\"\nurl = \"{}\"\n",
            secret_url(addr.to_string(), "/auth"),
            secret_url(addr.to_string(), "/fail"),
            secret_url(closed.to_string(), "/x"),
        ),
    ));
    probe(&shared, 0, 2).await;
    probe(&shared, 1, 3).await;
    probe(&shared, 2, 3).await;
    // The credentials did reach the target, as basic auth and query.
    assert!(seen.auth.lock().unwrap().is_some());
    assert_eq!(
        seen.query.lock().unwrap().as_deref(),
        Some("token=tok3nvalue")
    );

    let router = router(shared.clone());
    let mut bodies = String::new();
    for uri in [
        "/api/v1/overview",
        "/api/v1/checks",
        "/api/v1/checks/Good",
        "/api/v1/checks/Bad",
        "/api/v1/checks/Down",
        "/api/v1/events",
        "/api/v1/events?kind=check",
        "/api/v1/metrics",
        "/api/v1/metrics/query?key=check.latency:*",
        "/api/v1/services",
        "/api/v1/timers",
    ] {
        let resp = send_to(router.clone(), Method::GET, uri).await;
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        bodies.push_str(std::str::from_utf8(&bytes).unwrap());
    }
    // The responses are about these checks at all: their sanitized targets and failures.
    // With userinfo configured, only scheme://host:port is shown, not even the path.
    assert!(bodies.contains(&format!("\"http://{addr}\"")), "{bodies}");
    assert!(!bodies.contains("/auth"), "{bodies}");
    assert!(bodies.contains("unexpected status 500"), "{bodies}");
    assert!(bodies.contains("connection refused"), "{bodies}");

    let logs = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    assert!(logs.contains("health changed"), "{logs}");
    for secret in SECRETS {
        assert!(!bodies.contains(secret), "{secret} in a response: {bodies}");
        assert!(!logs.contains(secret), "{secret} in the logs: {logs}");
    }
}
