//! HTTP layer: the `/api/v1` routes and the embedded UI.

use std::net::SocketAddr;

use anyhow::Context;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use gargoyle_core::api::{ApiError, HealthResponse};

use crate::ui;

pub async fn serve(listen: SocketAddr) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("cannot listen on {listen}"))?;
    tracing::info!(%listen, version = env!("CARGO_PKG_VERSION"), "gargoyle listening");
    axum::serve(listener, router())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP server failed")?;
    tracing::info!("shut down");
    Ok(())
}

/// Only GET routes are registered, so every other method gets 405 (axum answers HEAD via GET).
pub fn router() -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api", get(api_not_found))
        .route("/api/", get(api_not_found))
        .route("/api/{*rest}", get(api_not_found))
        .route("/", get(ui::index))
        .route("/{*path}", get(ui::asset))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

async fn api_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError::new("not_found", "unknown API endpoint")),
    )
        .into_response()
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

    use super::*;

    async fn send(method: Method, uri: &str) -> Response {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        router().oneshot(req).await.unwrap()
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
}
