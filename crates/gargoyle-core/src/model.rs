//! Raw collector output (ARCHITECTURE §4).
//!
//! These are plain data: cumulative counters and gauges exactly as the host reports them, with no
//! rates and no health. They live in core rather than in `gargoyle-collector` so that the pure
//! rate and health code can consume them without depending on the collector. They derive
//! `Serialize` so a future remote agent can ship them as-is (§10).
//!
//! Units are fixed per field and documented on it: sizes are **bytes**, timestamps are **unix
//! seconds**, CPU times are **jiffies** (`USER_HZ` ticks, normally 1/100 s, as in `/proc/stat`).

use serde::{Deserialize, Serialize};

/// Mostly static facts about the host, read once at startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    /// `PRETTY_NAME` from os-release, e.g. `Debian GNU/Linux 13 (trixie)`.
    pub os: String,
    /// Kernel release, e.g. `6.12.0-1-amd64`.
    pub kernel: String,
    pub cpu_model: String,
    /// Logical CPUs.
    pub cores: u32,
    /// Unix seconds.
    pub boot_time: i64,
}

/// One system sample. Counters are cumulative since boot; `core::rates` derives per-second values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemSnapshot {
    /// When the sample was taken, unix seconds.
    pub ts: i64,
    /// Seconds since boot. A decrease between samples means the host rebooted.
    pub uptime_s: f64,
    pub cpu: CpuCounters,
    /// 1-, 5- and 15-minute load averages (not divided by core count).
    pub load: [f64; 3],
    pub memory: MemoryInfo,
    pub disks: Vec<DiskUsage>,
    pub net: Vec<NetCounters>,
    /// Empty when the host has no readable sensors.
    pub temps: Vec<TempReading>,
}

/// CPU time counters from `/proc/stat`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuCounters {
    /// The aggregate `cpu` line (sum over all cores).
    pub total: CpuTimes,
    /// The `cpuN` lines, in order.
    pub per_core: Vec<CpuTimes>,
}

/// Cumulative CPU time per state, in jiffies.
///
/// Guest time is already included in `user`/`nice` by the kernel, so it is not tracked separately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl CpuTimes {
    /// Sum of all states, in jiffies.
    pub fn total(&self) -> u64 {
        self.user
            .saturating_add(self.nice)
            .saturating_add(self.system)
            .saturating_add(self.idle)
            .saturating_add(self.iowait)
            .saturating_add(self.irq)
            .saturating_add(self.softirq)
            .saturating_add(self.steal)
    }
}

/// Memory gauges from `/proc/meminfo`, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryInfo {
    /// `MemTotal`.
    pub total: u64,
    /// `MemAvailable`; used memory is `total - available`.
    pub available: u64,
    /// `SwapTotal`; 0 when the host has no swap.
    pub swap_total: u64,
    /// `SwapFree`.
    pub swap_free: u64,
}

/// Usage of one mounted filesystem (statvfs), in bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskUsage {
    /// Mount point, unescaped (`\040` already turned into a space).
    pub mount: String,
    pub fs_type: String,
    pub total: u64,
    pub used: u64,
    /// Available to unprivileged users; `used + avail` can be less than `total` (reserved blocks).
    pub avail: u64,
}

/// Cumulative traffic of one network interface from `/proc/net/dev`, in bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetCounters {
    pub iface: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// One temperature sensor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TempReading {
    /// `<chip>/<sensor label>`, e.g. `coretemp/Package id 0`.
    pub label: String,
    pub celsius: f64,
}

/// A unit the collector should report on. Names are validated at config load (§4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitRequest {
    pub unit: String,
    /// Timers only: the triggered service when configured explicitly. When `None`, the collector
    /// uses the timer's `Unit=` property, falling back to `<timer stem>.service`.
    pub service: Option<String>,
}

impl UnitRequest {
    pub fn new(unit: impl Into<String>) -> Self {
        Self {
            unit: unit.into(),
            service: None,
        }
    }
}

/// Whitelisted systemd properties of one unit (§9: nothing that can carry secrets).
///
/// The collector returns one snapshot per requested unit **plus** one for the triggered service of
/// every requested timer. A unit that does not exist is still returned, with
/// `load_state = "not-found"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitSnapshot {
    pub unit: String,
    /// `LoadState`: `loaded`, `not-found`, `masked`, `error`, ...
    pub load_state: String,
    /// `ActiveState`: `active`, `inactive`, `failed`, `activating`, `deactivating`, `reloading`.
    pub active_state: String,
    /// `SubState`: `running`, `exited`, `dead`, `waiting`, ...
    pub sub_state: String,
    pub description: String,
    /// `ActiveEnterTimestamp`, unix seconds.
    pub active_enter_ts: Option<i64>,
    /// `NRestarts` (services only): automatic restarts since the unit was last started manually.
    pub n_restarts: Option<u32>,
    /// `Result` (services): `success`, `exit-code`, `signal`, `timeout`, `core-dump`, ...
    pub result: Option<String>,
    /// `ExecMainStatus`: exit code (or signal number) of the main process.
    pub exec_main_status: Option<i32>,
    /// `ExecMainStartTimestamp`, unix seconds.
    pub exec_main_start_ts: Option<i64>,
    /// `ExecMainExitTimestamp`, unix seconds.
    pub exec_main_exit_ts: Option<i64>,
    /// `InvocationID` of the current or last run.
    pub invocation_id: Option<String>,
    /// `MemoryCurrent`, bytes; `None` when unavailable (inactive, or memory accounting off).
    pub memory_current: Option<u64>,
    /// Timers: `LastTriggerUSec`, unix seconds.
    pub last_trigger_ts: Option<i64>,
    /// Timers: `NextElapseUSecRealtime`, unix seconds.
    pub next_elapse_ts: Option<i64>,
    /// Timers: `Unit=`, the service the timer triggers.
    pub trigger_unit: Option<String>,
}

/// A past timer run recovered from the journal (journal backfill).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerRunRecord {
    pub timer: String,
    pub service: String,
    pub invocation_id: Option<String>,
    /// Unix seconds.
    pub started_at: i64,
    /// Unix seconds; `None` if the run has not finished.
    pub finished_at: Option<i64>,
    /// Service `Result`, e.g. `success` or `exit-code`.
    pub result: Option<String>,
    pub exit_status: Option<i32>,
}

/// Why a collection failed.
///
/// Plain data (no boxed `io::Error`) so it can be cloned into `State` and its text shown as a
/// target's `message`. Keep the text free of secrets: it can end up in the log and the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum CollectError {
    /// Reading a file under `/proc` or `/sys` failed.
    #[error("cannot read {path}: {message}")]
    Io { path: String, message: String },
    /// A source had unexpected content.
    #[error("cannot parse {source_name}: {message}")]
    Parse {
        source_name: String,
        message: String,
    },
    /// Running `systemctl`/`journalctl` failed or it exited unsuccessfully.
    #[error("{program} failed: {message}")]
    Command { program: String, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_total_sums_every_state() {
        let t = CpuTimes {
            user: 1,
            nice: 2,
            system: 3,
            idle: 4,
            iowait: 5,
            irq: 6,
            softirq: 7,
            steal: 8,
        };
        assert_eq!(t.total(), 36);
    }

    #[test]
    fn collect_error_messages() {
        let e = CollectError::Io {
            path: "/proc/stat".into(),
            message: "permission denied".into(),
        };
        assert_eq!(e.to_string(), "cannot read /proc/stat: permission denied");
    }

    #[test]
    fn unit_snapshot_round_trips_through_json() {
        let u = UnitSnapshot {
            unit: "a.service".into(),
            load_state: "loaded".into(),
            active_state: "active".into(),
            sub_state: "running".into(),
            description: "A".into(),
            active_enter_ts: Some(1),
            n_restarts: Some(0),
            result: Some("success".into()),
            exec_main_status: Some(0),
            exec_main_start_ts: Some(1),
            exec_main_exit_ts: None,
            invocation_id: Some("abc".into()),
            memory_current: Some(1024),
            last_trigger_ts: None,
            next_elapse_ts: None,
            trigger_unit: None,
        };
        let json = serde_json::to_string(&u).expect("serializes");
        assert_eq!(
            serde_json::from_str::<UnitSnapshot>(&json).expect("deserializes"),
            u
        );
    }
}
