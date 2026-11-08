//! DTOs for the `/api/v1` JSON contract (ARCHITECTURE §6, `docs/api/examples/`).
//!
//! Rules that apply to every type here:
//! - keys are camelCase;
//! - optional values are `Option<T>` and serialize as `null` — never add `skip_serializing_if`,
//!   the contract requires the key to be present;
//! - timestamps are unix seconds (`i64`), durations are whole seconds, sizes are bytes.
//!
//! Integer and float fields are typed to match the examples exactly (`84.0` is `f64`, `42` is an
//! integer) so a JSON round trip is lossless.

use serde::{Deserialize, Serialize};

pub use crate::config::CheckKind;
use crate::health::Health;

/// `GET /api/v1/health`. Not to be confused with the [`Health`] state enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// Body of every 4xx/5xx response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    /// Machine-readable code, e.g. `not_found`.
    pub code: String,
    /// Generic human-readable message; details belong in the log, not here (ARCHITECTURE §9.4).
    pub message: String,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ErrorBody {
                code: code.into(),
                message: message.into(),
            },
        }
    }
}

/// `GET /api/v1/overview`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub host: String,
    pub ts: i64,
    pub health: Health,
    pub host_info: HostInfo,
    pub system: SystemSummary,
    pub counts: HealthCounts,
    pub services: Vec<ServiceSummary>,
    pub timers: Vec<TimerSummary>,
    pub checks: Vec<CheckSummary>,
    /// At most 10, newest first.
    pub recent_events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub cpu_model: String,
    pub cores: u32,
    pub boot_time: i64,
    /// Seconds.
    pub uptime: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSummary {
    pub health: Health,
    pub ts: i64,
    pub cpu: CpuSummary,
    pub load: LoadSummary,
    pub memory: MemorySummary,
    pub disks: Vec<DiskSummary>,
    /// Empty means "not available".
    pub net: Vec<NetSummary>,
    /// Empty means "not available".
    pub temps: Vec<TempSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuSummary {
    /// Percent, instantaneous.
    pub usage: f64,
    /// Percent, instantaneous.
    pub iowait: f64,
    pub health: Health,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSummary {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
    pub health: Health,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySummary {
    pub total: u64,
    /// `total - available`.
    pub used: u64,
    pub used_pct: f64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub health: Health,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSummary {
    pub mount: String,
    pub fs_type: String,
    pub total: u64,
    pub used: u64,
    pub used_pct: f64,
    pub health: Health,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetSummary {
    pub iface: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TempSummary {
    pub label: String,
    pub celsius: f64,
    pub health: Health,
}

/// Tally of every target; the system counts as one target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCounts {
    pub healthy: u32,
    pub warning: u32,
    pub critical: u32,
    pub unknown: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSummary {
    pub unit: String,
    pub name: String,
    pub description: String,
    pub health: Health,
    pub message: Option<String>,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    /// When the unit entered its current active state.
    pub since: Option<i64>,
    pub restarts: u32,
    /// Bytes.
    pub memory: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerSummary {
    pub unit: String,
    pub name: String,
    pub service: String,
    pub health: Health,
    pub message: Option<String>,
    pub active: bool,
    /// `None` if the timer has never run.
    pub last_run: Option<LastRun>,
    pub next_run: Option<i64>,
    pub running: bool,
}

/// The most recent run of a timer's service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRun {
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// Seconds.
    pub duration: Option<u64>,
    pub result: Option<String>,
    pub exit_status: Option<i32>,
}

/// One entry of a timer's run history: [`LastRun`] plus where it came from.
///
/// The fields are repeated rather than `#[serde(flatten)]`ed because flatten buffers values and
/// has known issues with numbers and `deny_unknown_fields`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerRun {
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration: Option<u64>,
    pub result: Option<String>,
    pub exit_status: Option<i32>,
    pub source: RunSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunSource {
    Observed,
    Journal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckSummary {
    pub name: String,
    pub kind: CheckKind,
    /// Configured URL or address with userinfo and query string stripped.
    pub target: String,
    pub health: Health,
    pub message: Option<String>,
    pub last_check: Option<i64>,
    pub latency_ms: Option<f64>,
    pub consecutive_failures: u32,
}

/// A health transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: i64,
    pub ts: i64,
    pub target_kind: TargetKind,
    pub target: String,
    /// `None` for a target's first observed state.
    pub from: Option<Health>,
    pub to: Health,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    System,
    Service,
    Timer,
    Check,
}

/// `GET /api/v1/services`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceList {
    pub host: String,
    pub services: Vec<ServiceSummary>,
}

/// `GET /api/v1/services/{unit}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDetail {
    pub host: String,
    pub service: ServiceSummary,
    pub events: Vec<Event>,
}

/// `GET /api/v1/timers`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerList {
    pub host: String,
    pub timers: Vec<TimerSummary>,
}

/// `GET /api/v1/timers/{unit}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerDetail {
    pub host: String,
    pub timer: TimerSummary,
    /// Newest first.
    pub runs: Vec<TimerRun>,
    pub events: Vec<Event>,
}

/// `GET /api/v1/checks`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckList {
    pub host: String,
    pub checks: Vec<CheckSummary>,
}

/// `GET /api/v1/checks/{name}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckDetail {
    pub host: String,
    pub check: CheckSummary,
    pub events: Vec<Event>,
}

/// `GET /api/v1/metrics`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricList {
    pub host: String,
    pub series: Vec<SeriesInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesInfo {
    /// `<metric>[:<label>]`.
    pub key: String,
    pub metric: String,
    /// `None` for unlabelled series.
    pub label: Option<String>,
    pub unit: String,
}

/// `GET /api/v1/metrics/query`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsQuery {
    pub host: String,
    pub from: i64,
    pub to: i64,
    /// Bucket width in seconds.
    pub step: i64,
    pub resolution: Resolution,
    pub series: Vec<SeriesData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    #[serde(rename = "raw")]
    Raw,
    #[serde(rename = "5m")]
    FiveMinutes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesData {
    pub key: String,
    pub unit: String,
    pub points: Vec<Point>,
}

/// `[ts, avg, min, max]`; the values are `null` for an empty bucket so charts show a gap.
pub type Point = (i64, Option<f64>, Option<f64>, Option<f64>);

/// `GET /api/v1/events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventList {
    pub host: String,
    /// Newest first.
    pub events: Vec<Event>,
}
