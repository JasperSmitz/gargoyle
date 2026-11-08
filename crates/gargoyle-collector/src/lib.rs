//! System and systemd collection for Gargoyle (ARCHITECTURE §4).
//!
//! A [`Collector`] reads raw facts — cumulative counters, gauges and unit properties — and nothing
//! else: no rates, no health, no storage. The data types live in [`gargoyle_core::model`] so pure
//! code can use them without this crate; they are re-exported here for convenience.

pub use gargoyle_core::model::{
    CollectError, CpuCounters, CpuTimes, DiskUsage, HostInfo, MemoryInfo, NetCounters,
    SystemSnapshot, TempReading, TimerRunRecord, UnitRequest, UnitSnapshot,
};

/// Source of raw host data.
///
/// Implementations are synchronous because reading `/proc` and spawning `systemctl` both block;
/// the scheduler calls them through `tokio::task::spawn_blocking`. They must be stateless with
/// respect to rates: counters are returned cumulative and `gargoyle_core::rates` diffs them.
pub trait Collector: Send + Sync + 'static {
    /// Hostname, kernel, OS, CPU model, core count and boot time.
    fn host_info(&self) -> Result<HostInfo, CollectError>;

    /// One sample of every system metric.
    fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError>;

    /// Properties of the requested units, plus the triggered service of every requested timer.
    /// Units that do not exist are returned with `load_state = "not-found"`, not as an error.
    fn units(&self, units: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError>;

    /// Past runs of `service` (triggered by `timer`) since `since` (unix seconds), from the
    /// journal. Optional: collectors without a journal, or without permission to read it, return
    /// an empty list.
    fn timer_history(
        &self,
        timer: &str,
        service: &str,
        since: i64,
    ) -> Result<Vec<TimerRunRecord>, CollectError> {
        let _ = (timer, service, since);
        Ok(Vec::new())
    }
}
