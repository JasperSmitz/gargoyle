//! HTTP layer: the `/api/v1` routes and the embedded UI.

use std::net::SocketAddr;

use anyhow::Context;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State as AxumState};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use gargoyle_core::api::{
    ApiError, Event, EventList, HealthResponse, MetricList, MetricsQuery, Overview, ServiceDetail,
    ServiceList, TargetKind, TimerDetail, TimerList, TimerRun,
};
use gargoyle_core::config::is_valid_unit_name;

use crate::scheduler::{Shared, now};
use crate::storage::{EventFilter, NativeIntervals, QueryError, QueryRequest};
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

/// `GET /events`: default and maximum `limit`.
const EVENTS_LIMIT: i64 = 100;
const EVENTS_MAX: i64 = 500;
/// `GET /timers/{unit}`: default and maximum `limit` (runs).
const RUNS_LIMIT: i64 = 50;
const RUNS_MAX: i64 = 500;
/// Events in a service or timer detail, and in the overview.
const DETAIL_EVENTS: i64 = 50;
const RECENT_EVENTS: i64 = 10;

/// Only GET routes are registered, so every other method gets 405 (axum answers HEAD via GET).
pub fn router(shared: Shared) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/overview", get(overview))
        .route("/api/v1/services", get(services))
        .route("/api/v1/services/{unit}", get(service))
        .route("/api/v1/timers", get(timers))
        .route("/api/v1/timers/{unit}", get(timer))
        .route("/api/v1/events", get(events))
        .route("/api/v1/metrics", get(metrics))
        .route("/api/v1/metrics/query", get(metrics_query))
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
    let mut overview = shared.read(|s| s.overview(&shared.config, now()));
    overview.recent_events = events_or_empty(
        &shared,
        EventFilter {
            limit: RECENT_EVENTS,
            ..EventFilter::default()
        },
    )
    .await;
    Json(overview)
}

async fn services(AxumState(shared): AxumState<Shared>) -> Json<ServiceList> {
    Json(shared.read(|s| ServiceList {
        host: s.host().to_owned(),
        services: s.services_view(&shared.config, now()),
    }))
}

/// `unit` arrives percent-decoded. It is validated before use even though it is only compared
/// against config, so a malformed name gets a clear 400 instead of a misleading 404.
fn unit_param(unit: Result<Path<String>, PathRejection>) -> Option<String> {
    unit.ok().map(|Path(u)| u).filter(|u| is_valid_unit_name(u))
}

fn invalid_unit() -> Response {
    error(StatusCode::BAD_REQUEST, "bad_request", "invalid unit name")
}

/// The newest events of one target, for its detail page.
fn target_events(kind: TargetKind, target: &str) -> EventFilter {
    EventFilter {
        kind: Some(kind),
        target: Some(target.to_owned()),
        limit: DETAIL_EVENTS,
        ..EventFilter::default()
    }
}

async fn service(
    AxumState(shared): AxumState<Shared>,
    unit: Result<Path<String>, PathRejection>,
) -> Response {
    let Some(unit) = unit_param(unit) else {
        return invalid_unit();
    };
    let Some(cfg) = shared.config.services.iter().find(|s| s.unit == unit) else {
        return error(StatusCode::NOT_FOUND, "not_found", "unknown service");
    };
    let events = events_or_empty(&shared, target_events(TargetKind::Service, &unit)).await;
    let detail = shared.read(|s| ServiceDetail {
        host: s.host().to_owned(),
        service: s.service_view(&shared.config, cfg, now()),
        events,
    });
    Json(detail).into_response()
}

async fn timers(AxumState(shared): AxumState<Shared>) -> Json<TimerList> {
    Json(shared.read(|s| TimerList {
        host: s.host().to_owned(),
        timers: s.timers_view(&shared.config, now()),
    }))
}

async fn timer(
    AxumState(shared): AxumState<Shared>,
    unit: Result<Path<String>, PathRejection>,
    params: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    let Some(unit) = unit_param(unit) else {
        return invalid_unit();
    };
    let Ok(Query(pairs)) = params else {
        return malformed_query();
    };
    let mut limit = None;
    if let Err(e) = parse_ints(&pairs, &mut [("limit", &mut limit)]) {
        return error(StatusCode::BAD_REQUEST, "bad_request", &e);
    }
    let Some(cfg) = shared.config.timers.iter().find(|t| t.unit == unit) else {
        return error(StatusCode::NOT_FOUND, "not_found", "unknown timer");
    };
    let limit = limit.unwrap_or(RUNS_LIMIT).clamp(1, RUNS_MAX);
    let runs: Vec<TimerRun> = match &shared.storage {
        None => Vec::new(),
        Some(storage) => storage
            .timer_runs(unit.clone(), limit)
            .await
            .unwrap_or_else(|e| {
                log_history_error(&e);
                Vec::new()
            }),
    };
    let events = events_or_empty(&shared, target_events(TargetKind::Timer, &unit)).await;
    let detail = shared.read(|s| TimerDetail {
        host: s.host().to_owned(),
        timer: s.timer_view(&shared.config, cfg, now()),
        runs,
        events,
    });
    Json(detail).into_response()
}

/// `GET /events?since=..&until=..&kind=..&target=..&limit=..`: `since` inclusive, `until`
/// exclusive, newest first. A client pages back with `until` = the oldest `ts` it received until a
/// page is empty; pages hold whole seconds only (see `storage::history::events`), so nothing is
/// skipped.
async fn events(
    AxumState(shared): AxumState<Shared>,
    params: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    let Ok(Query(pairs)) = params else {
        return malformed_query();
    };
    let (mut since, mut until, mut limit) = (None, None, None);
    if let Err(e) = parse_ints(
        &pairs,
        &mut [
            ("since", &mut since),
            ("until", &mut until),
            ("limit", &mut limit),
        ],
    ) {
        return error(StatusCode::BAD_REQUEST, "bad_request", &e);
    }
    let (mut target, mut kind) = (None, None);
    for (name, value) in &pairs {
        let repeated = match name.as_str() {
            "target" => target.replace(value.clone()).is_some(),
            "kind" => {
                let Some(k) = parse_kind(value) else {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "bad_request",
                        "kind must be system, service, timer or check",
                    );
                };
                kind.replace(k).is_some()
            }
            _ => false,
        };
        if repeated {
            return error(
                StatusCode::BAD_REQUEST,
                "bad_request",
                &format!("{name} given more than once"),
            );
        }
    }
    let Some(storage) = &shared.storage else {
        return unavailable();
    };
    let filter = EventFilter {
        since,
        until,
        kind,
        target,
        limit: limit.unwrap_or(EVENTS_LIMIT).clamp(1, EVENTS_MAX),
    };
    match storage.events(filter).await {
        Ok(events) => Json(EventList {
            host: shared.read(|s| s.host().to_owned()),
            events,
        })
        .into_response(),
        Err(e) => internal_error(&e),
    }
}

fn parse_kind(value: &str) -> Option<TargetKind> {
    match value {
        "system" => Some(TargetKind::System),
        "service" => Some(TargetKind::Service),
        "timer" => Some(TargetKind::Timer),
        "check" => Some(TargetKind::Check),
        _ => None,
    }
}

/// Parses the named integer parameters, each at most once. Other parameters are ignored.
fn parse_ints(
    pairs: &[(String, String)],
    slots: &mut [(&str, &mut Option<i64>)],
) -> Result<(), String> {
    for (name, value) in pairs {
        let Some((_, slot)) = slots.iter_mut().find(|(n, _)| n == name) else {
            continue;
        };
        let Ok(n) = value.parse::<i64>() else {
            return Err(format!("{name} must be an integer"));
        };
        if slot.replace(n).is_some() {
            return Err(format!("{name} given more than once"));
        }
    }
    Ok(())
}

/// Events for pages whose main content is live data: a history failure is logged and shows as
/// no events rather than failing the page.
async fn events_or_empty(shared: &Shared, filter: EventFilter) -> Vec<Event> {
    let Some(storage) = &shared.storage else {
        return Vec::new();
    };
    storage.events(filter).await.unwrap_or_else(|e| {
        log_history_error(&e);
        Vec::new()
    })
}

fn malformed_query() -> Response {
    error(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "malformed query string",
    )
}

async fn metrics(AxumState(shared): AxumState<Shared>) -> Response {
    let Some(storage) = &shared.storage else {
        return unavailable();
    };
    match storage.series().await {
        Ok(series) => Json(MetricList {
            host: shared.read(|s| s.host().to_owned()),
            series,
        })
        .into_response(),
        Err(e) => internal_error(&e),
    }
}

/// `key` may repeat, which the struct-based `Query` cannot express, so the pairs are parsed by
/// hand.
async fn metrics_query(
    AxumState(shared): AxumState<Shared>,
    params: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    let Ok(Query(pairs)) = params else {
        return malformed_query();
    };
    let Some(storage) = &shared.storage else {
        return unavailable();
    };
    let now = now();
    let req = match QueryRequest::parse(&pairs, now) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::BAD_REQUEST, "bad_request", &e.0),
    };
    let (from, to) = (req.from, req.to);
    let intervals = NativeIntervals::from_config(&shared.config);
    match storage.query(req, now, intervals).await {
        Ok((plan, series)) => Json(MetricsQuery {
            host: shared.read(|s| s.host().to_owned()),
            from,
            to,
            step: plan.step,
            resolution: plan.resolution,
            series,
        })
        .into_response(),
        Err(QueryError::UnknownSeries(key)) => error(
            StatusCode::BAD_REQUEST,
            "unknown_series",
            &format!("unknown series key: {key}"),
        ),
        Err(QueryError::Internal(e)) => internal_error(&e),
    }
}

fn unavailable() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "history storage is not available",
    )
}

fn log_history_error(e: &anyhow::Error) {
    tracing::error!(error = format!("{e:#}"), "history query failed");
}

/// Details go to the log only; the client gets a generic message.
fn internal_error(e: &anyhow::Error) -> Response {
    log_history_error(e);
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "history query failed",
    )
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
        let (_dir, router) = with_history().await;
        let (status, body) = get_json(router, "/api/v1/overview").await;
        assert_eq!(status, StatusCode::OK);
        let mut ex = example("overview.json");
        // Checks are M4.
        assert_eq!(body["checks"], serde_json::json!([]));
        ex["checks"] = serde_json::json!([]);
        assert_same_shape(&body, &ex, "overview");
        assert_eq!(body["host"], "gargoyle-mock");
        // It also parses as the DTO, as the frontend's types expect.
        let o = serde_json::from_value::<Overview>(body).unwrap();
        assert_eq!(o.timers.len(), 3);
        let total = o.counts.healthy + o.counts.warning + o.counts.critical + o.counts.unknown;
        assert_eq!(total, 1 + 4 + 3, "system, services and timers");
        // backup-fail.service is critical from its first observation.
        assert!(!o.recent_events.is_empty() && o.recent_events.len() <= 10);
        assert!(
            o.recent_events
                .iter()
                .any(|e| e.target == "backup-fail.service" && e.from.is_none())
        );
    }

    #[tokio::test]
    async fn overview_without_storage_has_no_events() {
        let (status, body) = get_json(populated().await, "/api/v1/overview").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["recentEvents"], serde_json::json!([]));
        assert_eq!(body["timers"].as_array().map(Vec::len), Some(3));
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
    async fn service_detail_includes_its_events() {
        let (_dir, router) = with_history().await;
        let (status, body) = get_json(router, "/api/v1/services/backup-fail.service").await;
        assert_eq!(status, StatusCode::OK);
        assert_same_shape(&body, &example("service-detail.json"), "service-detail");
        let d: ServiceDetail = serde_json::from_value(body).unwrap();
        assert_eq!(d.events.len(), 1);
        let e = &d.events[0];
        assert_eq!(
            (e.target_kind, e.target.as_str(), e.from, e.to),
            (
                TargetKind::Service,
                "backup-fail.service",
                None,
                gargoyle_core::health::Health::Critical
            )
        );
        assert_eq!(e.message.as_deref(), Some("failed: exit-code (status 1)"));
    }

    #[tokio::test]
    async fn timers_list_matches_the_example_shape() {
        let (status, body) = get_json(populated().await, "/api/v1/timers").await;
        assert_eq!(status, StatusCode::OK);
        assert_same_shape(&body, &example("timers.json"), "timers");
        let list: TimerList = serde_json::from_value(body).unwrap();
        let units: Vec<_> = list.timers.iter().map(|t| t.unit.as_str()).collect();
        assert_eq!(
            units,
            ["music-sync.timer", "backup.timer", "demo-quick.timer"],
            "config order"
        );
        let t = &list.timers[1];
        assert_eq!(
            (t.name.as_str(), t.service.as_str()),
            ("backup.timer", "backup.service")
        );
        assert!(t.active && t.next_run.is_some());
    }

    #[tokio::test]
    async fn timer_detail_matches_the_example_shape() {
        let (_dir, router) = with_history().await;
        let (status, body) = get_json(router.clone(), "/api/v1/timers/music-sync.timer").await;
        assert_eq!(status, StatusCode::OK);
        let mut ex = example("timer-detail.json");
        if body["events"] == serde_json::json!([]) {
            ex["events"] = serde_json::json!([]);
        }
        assert_same_shape(&body, &ex, "timer-detail");
        let d: TimerDetail = serde_json::from_value(body).unwrap();
        assert_eq!(d.timer.name, "Music sync");
        // The unit tick observed the current run.
        assert_eq!(d.runs.len(), 1);
        assert_eq!(d.runs[0].source, gargoyle_core::api::RunSource::Observed);
        assert_eq!(
            d.timer.last_run.as_ref().map(|r| r.started_at),
            Some(d.runs[0].started_at)
        );

        let (status, _) = get_json(router.clone(), "/api/v1/timers/music-sync.timer?limit=1").await;
        assert_eq!(status, StatusCode::OK);
        for (uri, code) in [
            ("/api/v1/timers/nope.timer", StatusCode::NOT_FOUND),
            ("/api/v1/timers/music-sync", StatusCode::BAD_REQUEST),
            (
                "/api/v1/timers/music-sync.timer?limit=x",
                StatusCode::BAD_REQUEST,
            ),
            (
                "/api/v1/timers/music-sync.timer?limit=1&limit=2",
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let (status, body) = get_json(router.clone(), uri).await;
            assert_eq!(status, code, "{uri}");
            assert_same_shape(&body, &example("error.json"), "error");
        }
    }

    #[tokio::test]
    async fn timer_detail_without_storage_has_no_history() {
        let (status, body) = get_json(populated().await, "/api/v1/timers/backup.timer").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["runs"], serde_json::json!([]));
        assert_eq!(body["events"], serde_json::json!([]));
        assert_eq!(body["timer"]["unit"], "backup.timer");
    }

    /// A router over a fresh database with the given events (ts, target) inserted.
    async fn with_events(events: &[(i64, &str)]) -> (tempfile::TempDir, Router) {
        use gargoyle_core::health::{Health, Transition};
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::from_toml_str(DEV_CONFIG).unwrap();
        config.storage.path = dir.path().join("g.db");
        let storage = crate::storage::Storage::open(&config.storage).unwrap();
        for &(ts, target) in events {
            let t = Transition {
                kind: TargetKind::Service,
                target: target.to_owned(),
                from: Some(Health::Healthy),
                to: Health::Critical,
                message: Some("down".into()),
            };
            storage.insert_events(ts, vec![t]).await.unwrap();
        }
        let shared = Shared::new(config, Arc::new(MockCollector::new(1))).with_storage(storage);
        (dir, router(shared))
    }

    async fn event_list(router: &Router, uri: &str) -> Vec<(i64, i64, String)> {
        let (status, body) = get_json(router.clone(), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        let list: EventList = serde_json::from_value(body).unwrap();
        list.events
            .into_iter()
            .map(|e| (e.id, e.ts, e.target))
            .collect()
    }

    #[tokio::test]
    async fn events_match_the_example_shape() {
        let (_dir, router) = with_events(&[(100, "a.service"), (200, "b.service")]).await;
        let (status, body) = get_json(router, "/api/v1/events").await;
        assert_eq!(status, StatusCode::OK);
        assert_same_shape(&body, &example("events.json"), "events");
        let list: EventList = serde_json::from_value(body).unwrap();
        let ts: Vec<i64> = list.events.iter().map(|e| e.ts).collect();
        assert_eq!(ts, [200, 100], "newest first");
    }

    #[tokio::test]
    async fn events_filter_and_page() {
        let (_dir, router) = with_events(&[
            (100, "a.service"),
            (200, "b.service"),
            (300, "a.service"),
            (300, "b.service"),
            (400, "a.service"),
        ])
        .await;
        let ts = |v: Vec<(i64, i64, String)>| v.into_iter().map(|e| e.1).collect::<Vec<_>>();
        assert_eq!(
            ts(event_list(&router, "/api/v1/events?since=200").await),
            [400, 300, 300, 200]
        );
        assert_eq!(
            ts(event_list(&router, "/api/v1/events?until=300").await),
            [200, 100]
        );
        assert_eq!(
            ts(event_list(&router, "/api/v1/events?since=200&until=400").await),
            [300, 300, 200]
        );
        let a = event_list(&router, "/api/v1/events?target=a.service").await;
        assert!(a.iter().all(|e| e.2 == "a.service"));
        assert_eq!(ts(a), [400, 300, 100]);

        // Paging with until = oldest ts of the page: the page that would split second 300 ends
        // before it, so every event is seen exactly once.
        let page1 = event_list(&router, "/api/v1/events?limit=2").await;
        assert_eq!(ts(page1.clone()), [400]);
        let page2 = event_list(&router, "/api/v1/events?limit=2&until=400").await;
        assert_eq!(ts(page2.clone()), [300, 300]);
        let page3 = event_list(&router, "/api/v1/events?limit=2&until=300").await;
        assert_eq!(ts(page3.clone()), [200, 100]);
        let mut ids: Vec<i64> = [page1, page2, page3]
            .concat()
            .into_iter()
            .map(|e| e.0)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 5);

        // limit is clamped to 1..=500.
        assert_eq!(event_list(&router, "/api/v1/events?limit=0").await.len(), 1);
        assert_eq!(
            event_list(&router, "/api/v1/events?limit=100000")
                .await
                .len(),
            5
        );
    }

    #[tokio::test]
    async fn events_paging_never_splits_a_second() {
        let (_dir, router) = with_events(&[
            (100, "a.service"),
            (200, "a.service"),
            (200, "b.service"),
            (200, "c.service"),
            (300, "a.service"),
            (300, "b.service"),
            (400, "a.service"),
        ])
        .await;
        for limit in [1, 2, 3] {
            let mut seen: Vec<i64> = Vec::new();
            let mut page_ts: Vec<Vec<i64>> = Vec::new();
            let mut uri = format!("/api/v1/events?limit={limit}");
            loop {
                let page = event_list(&router, &uri).await;
                let Some(oldest) = page.last().map(|e| e.1) else {
                    break;
                };
                page_ts.push(page.iter().map(|e| e.1).collect());
                seen.extend(page.iter().map(|e| e.0));
                uri = format!("/api/v1/events?limit={limit}&until={oldest}");
                assert!(page_ts.len() < 10, "paging terminates");
            }
            // Every event exactly once, and no page holds part of a second.
            let mut sorted = seen.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                (1..=7).collect::<Vec<_>>(),
                "limit={limit} {page_ts:?}"
            );
            for (i, a) in page_ts.iter().enumerate() {
                for b in &page_ts[i + 1..] {
                    assert!(
                        a.iter().all(|t| !b.contains(t)),
                        "limit={limit} {page_ts:?}"
                    );
                }
            }
            if limit == 1 {
                assert_eq!(
                    page_ts,
                    [vec![400], vec![300, 300], vec![200, 200, 200], vec![100]],
                    "a page may exceed limit to hold a whole second"
                );
            }
        }
    }

    #[tokio::test]
    async fn events_filter_by_kind() {
        let (_dir, router) = with_events(&[(100, "a.service")]).await;
        let (status, body) = get_json(router.clone(), "/api/v1/events?kind=service").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["events"].as_array().map(Vec::len), Some(1));
        for uri in [
            "/api/v1/events?kind=timer",
            "/api/v1/events?kind=service&target=b.service",
        ] {
            let (status, body) = get_json(router.clone(), uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(body["events"], serde_json::json!([]), "{uri}");
        }
        let (_, body) = get_json(
            router.clone(),
            "/api/v1/events?kind=service&target=a.service",
        )
        .await;
        assert_eq!(body["events"].as_array().map(Vec::len), Some(1));
        for uri in [
            "/api/v1/events?kind=nope",
            "/api/v1/events?kind=Service",
            "/api/v1/events?kind=timer&kind=service",
        ] {
            let (status, body) = get_json(router.clone(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(body["error"]["code"], "bad_request", "{uri}");
        }
    }

    #[tokio::test]
    async fn events_bad_requests_are_json_400() {
        let (_dir, router) = with_events(&[]).await;
        for uri in [
            "/api/v1/events?since=x",
            "/api/v1/events?until=1.5",
            "/api/v1/events?limit=",
            "/api/v1/events?since=1&since=2",
            "/api/v1/events?target=a&target=b",
        ] {
            let (status, body) = get_json(router.clone(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_same_shape(&body, &example("error.json"), "error");
        }
        let (status, body) = get_json(router, "/api/v1/events?since=100&until=50").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["events"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn events_without_storage_is_503() {
        let (status, body) = get_json(router(shared()), "/api/v1/events").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "unavailable");
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

    /// A router with storage: a backfilled week of mock history plus two live ticks. The
    /// `TempDir` must outlive the router.
    async fn with_history() -> (tempfile::TempDir, Router) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::from_toml_str(DEV_CONFIG).unwrap();
        config.storage.path = dir.path().join("g.db");
        let storage = crate::storage::Storage::open(&config.storage).unwrap();
        let mock = MockCollector::new(1);
        crate::storage::backfill_mock(&storage, &mock, now() - 20, 10).unwrap();
        let shared = Shared::new(config, Arc::new(mock)).with_storage(storage);
        system_tick(shared.clone()).await;
        system_tick(shared.clone()).await;
        // Two unit ticks, so first states pass the event hold-down.
        unit_tick(shared.clone()).await;
        unit_tick(shared.clone()).await;
        (dir, router(shared))
    }

    #[tokio::test]
    async fn metrics_list_matches_the_example_shape() {
        let (_dir, router) = with_history().await;
        let (status, body) = get_json(router, "/api/v1/metrics").await;
        assert_eq!(status, StatusCode::OK);
        assert_same_shape(&body, &example("metrics.json"), "metrics");
        let list: MetricList = serde_json::from_value(body).unwrap();
        assert_eq!(list.host, "gargoyle-mock");
        let keys: Vec<&str> = list.series.iter().map(|s| s.key.as_str()).collect();
        for key in [
            "cpu.usage",
            "mem.used",
            "disk.used:/",
            "service.memory:navidrome.service",
        ] {
            assert!(keys.contains(&key), "{key} in {keys:?}");
        }
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
    }

    #[tokio::test]
    async fn metrics_query_matches_the_example_shape() {
        let (_dir, router) = with_history().await;
        let (status, body) = get_json(
            router.clone(),
            "/api/v1/metrics/query?key=cpu.usage&key=mem.used",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_same_shape(&body, &example("metrics-query.json"), "metrics-query");
        let q: MetricsQuery = serde_json::from_value(body).unwrap();
        assert_eq!((q.to - q.from, q.step), (3600, 12));
        assert_eq!(q.resolution, gargoyle_core::api::Resolution::Raw);
        assert_eq!(q.series.len(), 2);
        assert!(q.series.iter().all(|s| s.points.len() == 300));

        let to = now();
        let uri = format!(
            "/api/v1/metrics/query?key=net.rx%3A*&from={}&to={to}&points=100",
            to - 7 * 86_400
        );
        let (status, body) = get_json(router, &uri).await;
        assert_eq!(status, StatusCode::OK);
        let q: MetricsQuery = serde_json::from_value(body).unwrap();
        assert_eq!(q.resolution, gargoyle_core::api::Resolution::FiveMinutes);
        assert!(!q.series.is_empty());
        assert!(q.series.iter().all(|s| s.key.starts_with("net.rx:")));
        assert!(q.series.iter().all(|s| s.points.len() <= 100));
    }

    #[tokio::test]
    async fn metrics_query_bad_requests_are_json_400() {
        let (_dir, router) = with_history().await;
        for uri in [
            "/api/v1/metrics/query",
            "/api/v1/metrics/query?key=cpu.usage&from=abc",
            "/api/v1/metrics/query?key=cpu.usage&from=100&to=100",
            "/api/v1/metrics/query?key=cpu.usage&from=200&to=100",
            "/api/v1/metrics/query?key=cpu.usage&from=0&to=9223372036854775307&points=1000",
            "/api/v1/metrics/query?key=cpu.usage&from=-9223372036854775808&to=9223372036854775807",
            "/api/v1/metrics/query?key=cpu.usage&from=0&to=99999999999999999999",
            "/api/v1/metrics/query?key=a&key=b&key=c&key=d&key=e&key=f&key=g\
             &key=h&key=i&key=j&key=k&key=l&key=m",
        ] {
            let (status, body) = get_json(router.clone(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_same_shape(&body, &example("error.json"), "error");
            assert_eq!(body["error"]["code"], "bad_request", "{uri}");
        }
    }

    #[tokio::test]
    async fn unknown_series_key_has_its_own_error_code() {
        let (_dir, router) = with_history().await;
        let (status, body) = get_json(router, "/api/v1/metrics/query?key=cpu.usage&key=nope").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_same_shape(&body, &example("error.json"), "error");
        assert_eq!(body["error"]["code"], "unknown_series");
    }

    #[tokio::test]
    async fn metrics_without_storage_is_503() {
        for uri in ["/api/v1/metrics", "/api/v1/metrics/query?key=cpu.usage"] {
            let (status, body) = get_json(router(shared()), uri).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
            assert_eq!(body["error"]["code"], "unavailable");
        }
    }
}
