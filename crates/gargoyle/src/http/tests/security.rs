//! M5: auth on every route, security headers on every response, method/body limits, and the
//! response audit (no secrets or local paths in any API response).

use std::time::{Duration, Instant};

use axum::http::HeaderValue;

use super::*;
use crate::auth::test_support::{basic, cheap_hash};
use crate::auth::{Authenticator, CHALLENGE};

const PASSWORD: &str = "c0rrect-h0rse";

fn authenticator(delay: Duration) -> Arc<Authenticator> {
    Arc::new(
        Authenticator::new("admin", &cheap_hash(PASSWORD))
            .unwrap()
            .with_failure_delay(delay),
    )
}

async fn request(router: &Router, method: Method, uri: &str, auth: Option<&str>) -> Response {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(auth) = auth {
        req = req.header(header::AUTHORIZATION, auth);
    }
    router
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Every kind of path: API (known, unknown, malformed), UI (index, asset, unknown) and odd
/// spellings near the exempt health route.
const PROTECTED: [&str; 16] = [
    "/",
    "/index.html",
    "/favicon.svg",
    "/assets/index-3f9a1c.js",
    "/nope",
    "/api",
    "/api/",
    "/api/v1",
    "/api/v1/overview",
    "/api/v1/services",
    "/api/v1/services/nginx.service",
    "/api/v1/metrics/query?key=cpu.usage",
    "/api/v1/nope",
    "/api/v1/health/",
    "/api/v1/health/x",
    "//api/v1/health",
];

#[tokio::test]
async fn with_auth_every_route_but_health_needs_credentials() {
    let app = app(shared(), Some(authenticator(Duration::ZERO)));
    for method in [Method::GET, Method::HEAD] {
        for uri in PROTECTED {
            let resp = request(&app, method.clone(), uri, None).await;
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
            assert_eq!(
                resp.headers()[header::WWW_AUTHENTICATE],
                CHALLENGE,
                "{method} {uri}"
            );
        }
        let resp = request(&app, method.clone(), "/api/v1/health", None).await;
        assert_eq!(resp.status(), StatusCode::OK, "{method}");
    }
    // The 401 body is a regular ApiError, for UI paths too.
    for uri in ["/", "/api/v1/overview"] {
        let resp = request(&app, Method::GET, uri, None).await;
        assert_eq!(resp.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(json(resp).await["error"]["code"], "unauthorized", "{uri}");
    }
    // Health is open even with wrong credentials (a probe may send stale ones).
    let wrong = basic("admin", "nope");
    let resp = request(&app, Method::GET, "/api/v1/health", Some(&wrong)).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn wrong_credentials_are_refused() {
    let app = app(shared(), Some(authenticator(Duration::ZERO)));
    for auth in [
        basic("admin", "nope"),
        basic("root", PASSWORD),
        basic("admin", &format!("{PASSWORD} ")),
        format!("Bearer {PASSWORD}"),
        "Basic".to_owned(),
        "garbage".to_owned(),
    ] {
        let resp = request(&app, Method::GET, "/api/v1/overview", Some(&auth)).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{auth}");
        assert_eq!(resp.headers()[header::WWW_AUTHENTICATE], CHALLENGE);
    }
}

/// ROADMAP M5 acceptance: polling with correct credentials runs argon2 once, not per request.
#[tokio::test]
async fn polling_with_correct_credentials_runs_argon2_once() {
    let auth = authenticator(Duration::ZERO);
    let app = app(populated_shared().await, Some(Arc::clone(&auth)));
    let good = basic("admin", PASSWORD);
    for _ in 0..3 {
        for uri in [
            "/",
            "/api/v1/overview",
            "/api/v1/services",
            "/api/v1/timers",
        ] {
            let resp = request(&app, Method::GET, uri, Some(&good)).await;
            assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        }
    }
    assert_eq!(auth.argon2_runs(), 1);
}

#[tokio::test]
async fn failures_are_delayed_but_a_missing_header_is_not() {
    let delay = Duration::from_millis(200);
    let app = app(shared(), Some(authenticator(delay)));

    let start = Instant::now();
    let wrong = basic("admin", "nope");
    let resp = request(&app, Method::GET, "/", Some(&wrong)).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(start.elapsed() >= delay, "{:?}", start.elapsed());

    // A browser's first request carries no credentials: the prompt should appear at once.
    let start = Instant::now();
    let resp = request(&app, Method::GET, "/", None).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(start.elapsed() < delay, "{:?}", start.elapsed());
}

fn assert_security_headers(resp: &Response, what: &str) {
    let h = resp.headers();
    let csp = h[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    for directive in [
        "default-src 'self'",
        "img-src 'self' data:",
        "frame-ancestors 'none'",
        "base-uri 'none'",
        "form-action 'none'",
        "object-src 'none'",
    ] {
        assert!(csp.contains(directive), "{what}: {csp}");
    }
    // Scripts and styles fall back to 'self': nothing inline is allowed.
    for absent in ["script-src", "style-src", "unsafe"] {
        assert!(!csp.contains(absent), "{what}: {csp}");
    }
    assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{what}");
    assert_eq!(h[header::REFERRER_POLICY], "no-referrer", "{what}");
    assert_eq!(h[header::X_FRAME_OPTIONS], "DENY", "{what}");
    assert_eq!(h["cross-origin-opener-policy"], "same-origin", "{what}");
    assert_eq!(h["cross-origin-resource-policy"], "same-origin", "{what}");
    assert!(h.contains_key("permissions-policy"), "{what}");
    assert!(
        !h.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        "{what}: no CORS"
    );
}

#[tokio::test]
async fn every_response_has_the_security_headers() {
    let open = router(populated_shared().await);
    let cases = [
        (Method::GET, "/api/v1/health", StatusCode::OK),
        (Method::GET, "/api/v1/overview", StatusCode::OK),
        (Method::HEAD, "/api/v1/services", StatusCode::OK),
        (Method::GET, "/api/v1/nope", StatusCode::NOT_FOUND),
        (
            Method::GET,
            "/api/v1/events?limit=x",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::GET,
            "/api/v1/events",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (Method::GET, "/", StatusCode::OK),
        (Method::GET, "/nope.js", StatusCode::NOT_FOUND),
        (
            Method::POST,
            "/api/v1/health",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (Method::DELETE, "/", StatusCode::METHOD_NOT_ALLOWED),
    ];
    for (method, uri, status) in cases {
        let resp = request(&open, method.clone(), uri, None).await;
        assert_eq!(resp.status(), status, "{method} {uri}");
        assert_security_headers(&resp, &format!("{method} {uri}"));
        let cache = resp.headers().get(header::CACHE_CONTROL);
        if uri.starts_with("/api") {
            assert_eq!(cache.unwrap(), "no-store", "{method} {uri}");
        } else {
            assert_ne!(cache, Some(&HeaderValue::from_static("no-store")), "{uri}");
        }
    }
    // The UI keeps its own cache policy.
    let resp = request(&open, Method::GET, "/", None).await;
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-cache");

    let guarded = app(shared(), Some(authenticator(Duration::ZERO)));
    for uri in ["/", "/api/v1/overview"] {
        let resp = request(&guarded, Method::GET, uri, None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_security_headers(&resp, &format!("401 {uri}"));
    }
}

/// ROADMAP M5 acceptance: mutating methods are 405 on every path, before routing and before
/// auth, so the answer does not depend on credentials.
#[tokio::test]
async fn non_get_methods_are_405_everywhere_with_or_without_auth() {
    let open = router(shared());
    let guarded = app(shared(), Some(authenticator(Duration::ZERO)));
    let methods = [
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
        Method::OPTIONS,
        Method::TRACE,
        Method::CONNECT,
    ];
    for router in [&open, &guarded] {
        for method in &methods {
            for uri in [
                "/api/v1/health",
                "/api/v1/overview",
                "/api/v1/nope",
                "/",
                "/x",
            ] {
                let resp = request(router, method.clone(), uri, None).await;
                assert_eq!(
                    resp.status(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{method} {uri}"
                );
                assert_eq!(resp.headers()[header::ALLOW], "GET, HEAD");
            }
        }
    }
}

#[tokio::test]
async fn request_bodies_are_refused() {
    let app = router(shared());
    let with = |name: header::HeaderName, value: &str| {
        Request::get("/api/v1/health")
            .header(name, value)
            .body(Body::empty())
            .unwrap()
    };
    for req in [
        with(header::CONTENT_LENGTH, "5"),
        with(header::TRANSFER_ENCODING, "chunked"),
    ] {
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
    let resp = app
        .clone()
        .oneshot(with(header::CONTENT_LENGTH, "0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

async fn populated_shared() -> Shared {
    let shared = shared();
    system_tick(shared.clone()).await;
    system_tick(shared.clone()).await;
    unit_tick(shared.clone()).await;
    shared
}

/// M5.4 response audit. A config full of sensitive values (password hash, storage path,
/// credentials in a check URL) is served with auth on; every API endpoint, including detail
/// pages and error responses, is fetched, and no response may contain any of them, nor
/// anything that looks like systemd command lines or environment.
#[tokio::test]
async fn no_api_response_contains_secrets_or_local_paths() {
    let dir = tempfile::Builder::new()
        .prefix("gargoyle-audit-dbdir")
        .tempdir()
        .unwrap();
    let hash = cheap_hash(PASSWORD);
    let mut config = Config::from_toml_str(&format!(
        "{DEV_CONFIG}\n\
         [[check]]\nname = \"Secret\"\nkind = \"http\"\n\
         url = \"http://audituser:auditpw@127.0.0.1:9/p4th?token=audittoken#auditfrag\"\n"
    ))
    .unwrap();
    config.auth.mode = gargoyle_core::config::AuthMode::Basic;
    config.auth.password_hash = Some(hash.clone());
    config.storage.path = dir.path().join("audit.db");
    let storage = crate::storage::Storage::open(&config.storage).unwrap();
    let mock = MockCollector::new(1);
    crate::storage::backfill_mock(&storage, &mock, now() - 20, 10).unwrap();
    let shared = Shared::new(config.clone(), Arc::new(mock)).with_storage(storage);
    system_tick(shared.clone()).await;
    system_tick(shared.clone()).await;
    unit_tick(shared.clone()).await;
    unit_tick(shared.clone()).await;
    let auth = Arc::new(
        Authenticator::new(&config.auth.username, &hash)
            .unwrap()
            .with_failure_delay(Duration::ZERO),
    );
    let app = app(shared, Some(auth));
    let good = basic("admin", PASSWORD);

    let mut uris: Vec<String> = [
        "/api/v1/health",
        "/api/v1/overview",
        "/api/v1/services",
        "/api/v1/timers",
        "/api/v1/checks",
        "/api/v1/events",
        "/api/v1/events?limit=500",
        "/api/v1/metrics",
        "/api/v1/metrics/query?key=cpu.usage",
        "/api/v1/metrics/query?key=disk.used_pct:*",
        // Error paths echo nothing from the request or the config.
        "/api/v1/nope",
        "/api/v1/services/unknown.service",
        "/api/v1/services/bad%20name",
        "/api/v1/checks/unknown",
        "/api/v1/events?kind=x",
        "/api/v1/metrics/query?key=nope",
    ]
    .map(str::to_owned)
    .to_vec();
    uris.extend(
        config
            .services
            .iter()
            .map(|s| format!("/api/v1/services/{}", s.unit)),
    );
    uris.extend(
        config
            .timers
            .iter()
            .map(|t| format!("/api/v1/timers/{}", t.unit)),
    );
    uris.extend(
        config
            .checks
            .iter()
            .map(|c| format!("/api/v1/checks/{}", c.name.replace(' ', "%20"))),
    );

    let mut bodies = String::new();
    for uri in &uris {
        for auth in [Some(good.as_str()), None] {
            let resp = request(&app, Method::GET, uri, auth).await;
            let bytes = to_bytes(resp.into_body(), 1 << 22).await.unwrap();
            bodies.push_str(&String::from_utf8_lossy(&bytes));
            bodies.push('\n');
        }
    }
    // The crawl reached real data.
    assert!(bodies.contains("navidrome.service"), "{bodies}");
    assert!(bodies.contains("\"http://127.0.0.1:9\""), "{bodies}");

    let db_dir = dir.path().display().to_string();
    let forbidden = [
        hash.as_str(),
        "$argon2",
        PASSWORD,
        &db_dir,
        "gargoyle-audit-dbdir",
        "audit.db",
        "/etc/gargoyle",
        "gargoyle.toml",
        "audituser",
        "auditpw",
        "audittoken",
        "auditfrag",
        "p4th",
        "ExecStart",
        "ExecStop",
        "Environment",
        "Credential",
    ];
    for needle in forbidden {
        assert!(!bodies.contains(needle), "{needle:?} in an API response");
    }
}
