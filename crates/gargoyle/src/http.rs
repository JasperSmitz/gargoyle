//! HTTP layer: the `/api/v1` routes and the embedded UI.

use std::net::SocketAddr;

use anyhow::Context;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State as AxumState};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use gargoyle_core::api::{ApiError, HealthResponse, Overview, ServiceDetail, ServiceList};
use gargoyle_core::config::is_valid_unit_name;

use crate::scheduler::{Shared, now};
use crate::ui;

pub async fn serve(listen: SocketAddr, shared: Shared) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("cannot listen on {listen}"))?;
    tracing::info!(%listen, version = env!("CARGO_PKG_VERSION"), "gargoyle listening");
    axum::serve(listener, router(shared))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP server failed")?;
    tracing::info!("shut down");
    Ok(())
}

/// Only GET routes are registered, so every other method gets 405 (axum answers HEAD via GET).
pub fn router(shared: Shared) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/overview", get(overview))
        .route("/api/v1/services", get(services))
        .route("/api/v1/services/{unit}", get(service))
        .route("/api", get(api_not_found))
        .route("/api/", get(api_not_found))
        .route("/api/{*rest}", get(api_not_found))
        .route("/", get(ui::index))
        .route("/{*path}", get(ui::asset))
        .with_state(shared)
}

/// A router over an empty state with the mock collector and default config, for tests that do
/// not care about collected data.
#[cfg(test)]
pub fn mock_router() -> Router {
    let shared = Shared::new(
        gargoyle_core::config::Config::default(),
        std::sync::Arc::new(gargoyle_collector::MockCollector::new(1)),
    );
    router(shared)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

async fn overview(AxumState(shared): AxumState<Shared>) -> Json<Overview> {
    Json(shared.read(|s| s.overview(&shared.config, now())))
}

async fn services(AxumState(shared): AxumState<Shared>) -> Json<ServiceList> {
    Json(shared.read(|s| ServiceList {
        host: s.host().to_owned(),
        services: s.services_view(&shared.config, now()),
    }))
}

/// `unit` arrives percent-decoded. It is validated before use even though it is only compared
/// against config, so a malformed name gets a clear 400 instead of a misleading 404.
async fn service(
    AxumState(shared): AxumState<Shared>,
    unit: Result<Path<String>, PathRejection>,
) -> Response {
    let Ok(Path(unit)) = unit else {
        return error(StatusCode::BAD_REQUEST, "bad_request", "invalid unit name");
    };
    if !is_valid_unit_name(&unit) {
        return error(StatusCode::BAD_REQUEST, "bad_request", "invalid unit name");
    }
    let Some(cfg) = shared.config.services.iter().find(|s| s.unit == unit) else {
        return error(StatusCode::NOT_FOUND, "not_found", "unknown service");
    };
    let detail = shared.read(|s| ServiceDetail {
        host: s.host().to_owned(),
        service: s.service_view(&shared.config, cfg, now()),
        // Events need storage (M2).
        events: Vec::new(),
    });
    Json(detail).into_response()
}

async fn api_not_found() -> Response {
    error(StatusCode::NOT_FOUND, "not_found", "unknown API endpoint")
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(ApiError::new(code, message))).into_response()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, header};
    use serde_json::Value;
    use tower::ServiceExt;

    use std::sync::Arc;

    use gargoyle_collector::MockCollector;
    use gargoyle_core::api::ServiceSummary;
    use gargoyle_core::config::Config;

    use super::*;
    use crate::scheduler::{system_tick, unit_tick};

    const DEV_CONFIG: &str = include_str!("../../../config/gargoyle.dev.toml");

    fn shared() -> Shared {
        let config = Config::from_toml_str(DEV_CONFIG).unwrap();
        Shared::new(config, Arc::new(MockCollector::new(1)))
    }

    /// A router whose state has had two system ticks and one unit tick.
    async fn populated() -> Router {
        let shared = shared();
        system_tick(shared.clone()).await;
        system_tick(shared.clone()).await;
        unit_tick(shared.clone()).await;
        router(shared)
    }

    async fn send(method: Method, uri: &str) -> Response {
        send_to(router(shared()), method, uri).await
    }

    async fn send_to(router: Router, method: Method, uri: &str) -> Response {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        router.oneshot(req).await.unwrap()
    }

    async fn get_json(router: Router, uri: &str) -> (StatusCode, Value) {
        let resp = send_to(router, Method::GET, uri).await;
        let status = resp.status();
        (status, json(resp).await)
    }

    /// Every key of `example` is present in `actual` (recursively, through the first array
    /// element), and no extra keys exist: the response has the documented shape.
    fn assert_same_shape(actual: &Value, example: &Value, path: &str) {
        match (actual, example) {
            (Value::Object(a), Value::Object(e)) => {
                let mut ak: Vec<_> = a.keys().collect();
                let mut ek: Vec<_> = e.keys().collect();
                ak.sort();
                ek.sort();
                assert_eq!(ak, ek, "keys at {path}");
                for (k, ev) in e {
                    assert_same_shape(&a[k], ev, &format!("{path}.{k}"));
                }
            }
            (Value::Array(a), Value::Array(e)) => {
                if let (Some(a0), Some(e0)) = (a.first(), e.first()) {
                    assert_same_shape(a0, e0, &format!("{path}[0]"));
                }
            }
            // Optional fields may legitimately be null on either side.
            (Value::Null, _) | (_, Value::Null) => {}
            (a, e) => assert_eq!(
                std::mem::discriminant(a),
                std::mem::discriminant(e),
                "type at {path}: {a} vs {e}"
            ),
        }
    }

    fn example(name: &str) -> Value {
        let path = format!(
            "{}/../../docs/api/examples/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    async fn json(resp: Response) -> Value {
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn health_returns_status_and_version() {
        let resp = send(Method::GET, "/api/v1/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            json(resp).await,
            serde_json::json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") })
        );
    }

    #[tokio::test]
    async fn head_is_allowed() {
        let resp = send(Method::HEAD, "/api/v1/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn mutating_methods_are_rejected() {
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            for uri in ["/api/v1/health", "/api/v1/nope", "/", "/assets/x.js"] {
                let resp = send(method.clone(), uri).await;
                assert_eq!(
                    resp.status(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{method} {uri}"
                );
            }
        }
    }

    #[tokio::test]
    async fn unknown_api_path_is_json_404() {
        for uri in [
            "/api",
            "/api/",
            "/api/v1/",
            "/api/v1/nope",
            "/api/v2/health",
        ] {
            let resp = send(Method::GET, uri).await;
            assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri}");
            let body = json(resp).await;
            assert_eq!(body["error"]["code"], "not_found", "{uri}");
        }
    }

    #[tokio::test]
    async fn overview_matches_the_example_shape() {
        let (status, body) = get_json(populated().await, "/api/v1/overview").await;
        assert_eq!(status, StatusCode::OK);
        let mut ex = example("overview.json");
        // Not produced until M2 (events), M3 (timers) and M4 (checks).
        for key in ["timers", "checks", "recentEvents"] {
            assert_eq!(body[key], serde_json::json!([]), "{key}");
            ex[key] = serde_json::json!([]);
        }
        assert_same_shape(&body, &ex, "overview");
        assert_eq!(body["host"], "gargoyle-mock");
        // It also parses as the DTO, as the frontend's types expect.
        serde_json::from_value::<Overview>(body).unwrap();
    }

    #[tokio::test]
    async fn overview_before_first_sample_is_unknown() {
        let (status, body) = get_json(router(shared()), "/api/v1/overview").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["health"], "unknown");
        assert_eq!(body["system"]["health"], "unknown");
        assert_eq!(body["services"][0]["message"], "waiting for first sample");
    }

    #[tokio::test]
    async fn services_list_matches_the_example_shape() {
        let (status, body) = get_json(populated().await, "/api/v1/services").await;
        assert_eq!(status, StatusCode::OK);
        assert_same_shape(&body, &example("services.json"), "services");
        let list: ServiceList = serde_json::from_value(body).unwrap();
        let units: Vec<_> = list.services.iter().map(|s| s.unit.as_str()).collect();
        assert_eq!(
            units,
            [
                "navidrome.service",
                "minecraft.service",
                "nginx.service",
                "backup-fail.service"
            ],
            "config order"
        );
        let mut healths: Vec<_> = list.services.iter().map(|s| s.health).collect();
        healths.sort();
        healths.dedup();
        assert!(
            healths.len() >= 3,
            "M1 acceptance: three states, got {healths:?}"
        );
    }

    #[tokio::test]
    async fn service_detail_matches_the_example_shape() {
        let (status, body) =
            get_json(populated().await, "/api/v1/services/backup-fail.service").await;
        assert_eq!(status, StatusCode::OK);
        let mut ex = example("service-detail.json");
        ex["events"] = serde_json::json!([]);
        assert_same_shape(&body, &ex, "service-detail");
        let s: ServiceSummary = serde_json::from_value(body["service"].clone()).unwrap();
        assert_eq!(s.name, "Always failing (demo)");
        assert_eq!(s.message.as_deref(), Some("failed: exit-code (status 1)"));
    }

    #[tokio::test]
    async fn service_detail_name_is_percent_decoded() {
        // `%2D` is `-`.
        let (status, body) =
            get_json(populated().await, "/api/v1/services/backup%2Dfail.service").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["service"]["unit"], "backup-fail.service");
    }

    #[tokio::test]
    async fn unknown_service_is_json_404() {
        let (status, body) = get_json(router(shared()), "/api/v1/services/sshd.service").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_same_shape(&body, &example("error.json"), "error");
        assert_eq!(body["error"]["code"], "not_found");
    }

    #[tokio::test]
    async fn invalid_service_name_is_json_400() {
        for uri in [
            "/api/v1/services/nginx",
            "/api/v1/services/..%2Fetc.service",
            "/api/v1/services/a%20b.service",
            "/api/v1/services/%FF.service",
        ] {
            let (status, body) = get_json(router(shared()), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(body["error"]["code"], "bad_request", "{uri}");
        }
    }
}
