//! Middleware applied to every response, including errors and 401s (ARCHITECTURE §9.5).

use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

/// §9.5. The UI is one script and one stylesheet from `/assets/`, and its favicon is a `data:`
/// SVG swapped per health. No inline styles: the UI sets styles only through the CSSOM (Svelte
/// `style:`, uPlot), which CSP allows; a static `style="..."` attribute would be blocked.
pub const CSP: &str = "default-src 'self'; img-src 'self' data:; frame-ancestors 'none'; \
                       base-uri 'none'; form-action 'none'; object-src 'none'";
const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), payment=(), usb=()";
const NO_STORE: &str = "no-store";
const ALLOW: &str = "GET, HEAD";

/// Covers the slowest legitimate request (a large metrics query, or a wrong password's 1s
/// delay behind other verifications) with room to spare.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Each `layer` wraps the ones added before it: the headers are outermost, so even a 405, a
/// 413 or a timeout gets them, and methods are checked before bodies, auth and routing.
pub fn apply(router: Router) -> Router {
    router
        .layer(middleware::from_fn_with_state(REQUEST_TIMEOUT, timeout))
        .layer(middleware::from_fn(reject_bodies))
        .layer(middleware::from_fn(only_get_and_head))
        .layer(middleware::from_fn(security_headers))
}

fn is_api(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

async fn security_headers(req: Request, next: Next) -> Response {
    let api = is_api(req.uri().path());
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    set(headers, header::CONTENT_SECURITY_POLICY, CSP);
    set(headers, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(headers, header::REFERRER_POLICY, "no-referrer");
    set(headers, header::X_FRAME_OPTIONS, "DENY");
    set(
        headers,
        HeaderName::from_static("cross-origin-opener-policy"),
        "same-origin",
    );
    set(
        headers,
        HeaderName::from_static("cross-origin-resource-policy"),
        "same-origin",
    );
    set(
        headers,
        HeaderName::from_static("permissions-policy"),
        PERMISSIONS_POLICY,
    );
    if api {
        set(headers, header::CACHE_CONTROL, NO_STORE);
    }
    resp
}

fn set(headers: &mut HeaderMap, name: HeaderName, value: &'static str) {
    headers.insert(name, HeaderValue::from_static(value));
}

/// Read-only by construction (§9.1): this answers before routing or auth, so the 405 is the
/// same on every path, with or without credentials.
async fn only_get_and_head(req: Request, next: Next) -> Response {
    if req.method() == Method::GET || req.method() == Method::HEAD {
        next.run(req).await
    } else {
        (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, HeaderValue::from_static(ALLOW))],
        )
            .into_response()
    }
}

/// No endpoint reads a body, so any announced body is refused instead of being received.
async fn reject_bodies(req: Request, next: Next) -> Response {
    let headers = req.headers();
    let has_body = headers.contains_key(header::TRANSFER_ENCODING)
        || headers
            .get(header::CONTENT_LENGTH)
            .is_some_and(|v| v.as_bytes() != b"0");
    if has_body {
        StatusCode::PAYLOAD_TOO_LARGE.into_response()
    } else {
        next.run(req).await
    }
}

async fn timeout(State(limit): State<Duration>, req: Request, next: Next) -> Response {
    match tokio::time::timeout(limit, next.run(req)).await {
        Ok(resp) => resp,
        Err(_) => {
            tracing::warn!(?limit, "request timed out");
            super::error(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "request timed out",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn slow_requests_time_out() {
        let app = Router::new()
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    "late"
                }),
            )
            .layer(middleware::from_fn_with_state(
                Duration::from_millis(20),
                timeout,
            ));
        let req = Request::builder().uri("/slow").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn api_paths() {
        for p in ["/api", "/api/", "/api/v1/health", "/api/v1/nope"] {
            assert!(is_api(p), "{p}");
        }
        for p in ["/", "/apix", "/assets/api.js", "/index.html"] {
            assert!(!is_api(p), "{p}");
        }
    }
}
