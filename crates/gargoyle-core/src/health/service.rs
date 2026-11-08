//! Health of configured services (§8, "Service" rows).

use super::{Assessment, Health};
use crate::config::{Expect, ServiceConfig};
use crate::model::UnitSnapshot;

/// How far back an `NRestarts` increase still counts as a warning: one hour.
pub const RESTART_WINDOW_S: i64 = 3600;

const MIB: u64 = 1024 * 1024;

/// Remembers when a service's `NRestarts` last went up, which is the only history the service
/// rules need. The scheduler keeps one per service and feeds it every unit tick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestartTracker {
    last_count: Option<u32>,
    last_increase_ts: Option<i64>,
}

impl RestartTracker {
    /// Records the `NRestarts` value seen at `ts`.
    ///
    /// The first value seen is only a baseline: restarts that happened before Gargoyle started
    /// have no timestamp. A decrease (the unit was restarted by hand, which resets the counter)
    /// is a new baseline too.
    pub fn observe(&mut self, ts: i64, n_restarts: Option<u32>) {
        let Some(n) = n_restarts else {
            return;
        };
        if self.last_count.is_some_and(|prev| n > prev) {
            self.last_increase_ts = Some(ts);
        }
        self.last_count = Some(n);
    }

    /// True if `NRestarts` went up within [`RESTART_WINDOW_S`] before `now`.
    pub fn restarted_recently(&self, now: i64) -> bool {
        self.last_increase_ts
            .is_some_and(|ts| now.saturating_sub(ts) <= RESTART_WINDOW_S)
    }
}

/// Evaluates one service. `restarted_recently` comes from its [`RestartTracker`].
///
/// A unit whose `LoadState` is not `loaded` is critical for every `expect`: a misspelt or removed
/// unit is a configuration problem worth seeing even when any state would otherwise be fine.
/// Collector errors and staleness are not decided here; the caller reports those as
/// [`Assessment::unknown`].
pub fn evaluate_service(
    cfg: &ServiceConfig,
    unit: &UnitSnapshot,
    restarted_recently: bool,
) -> Assessment {
    if unit.load_state != "loaded" {
        return if unit.load_state == "not-found" {
            Assessment::new(Health::Critical, "unit not found")
        } else {
            Assessment::new(Health::Critical, format!("unit is {}", unit.load_state))
        };
    }
    if unit.active_state == "failed" {
        return Assessment::new(Health::Critical, failed_message(unit));
    }

    let state = format!("{} ({})", unit.active_state, unit.sub_state);
    let base = match (cfg.expect, unit.active_state.as_str()) {
        // §8: for `any`, everything but failed is healthy and there are no warnings.
        (Expect::Any, _) => return Assessment::healthy(),
        (Expect::Running, "active") if unit.sub_state == "running" => Assessment::healthy(),
        // e.g. a oneshot that exited: up as far as systemd cares, but nothing is running.
        (Expect::Running, "active") => {
            Assessment::new(Health::Warning, format!("{state}, expected running"))
        }
        (Expect::Active, "active") => Assessment::healthy(),
        (_, "inactive") => Assessment::new(Health::Critical, state),
        // activating, deactivating, reloading, and any state newer systemd versions add.
        _ => Assessment::new(Health::Warning, state),
    };

    let mut parts = vec![base];
    if restarted_recently {
        let total = unit
            .n_restarts
            .map(|n| format!(" ({n} since last manual start)"))
            .unwrap_or_default();
        parts.push(Assessment::new(
            Health::Warning,
            format!("restarted in the last hour{total}"),
        ));
    }
    if let (Some(limit_mb), Some(bytes)) = (cfg.memory_warning_mb, unit.memory_current)
        && bytes > limit_mb.saturating_mul(MIB)
    {
        parts.push(Assessment::new(
            Health::Warning,
            format!("memory {} MB (warning > {limit_mb} MB)", bytes / MIB),
        ));
    }
    Assessment::combine(&parts)
}

/// `failed: exit-code (status 1)`, `failed: signal`, or plain `failed`.
fn failed_message(unit: &UnitSnapshot) -> String {
    match (unit.result.as_deref(), unit.exec_main_status) {
        (Some("exit-code"), Some(status)) => format!("failed: exit-code (status {status})"),
        (Some(result), _) if result != "success" => format!("failed: {result}"),
        _ => "failed".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Health::*;

    fn cfg(expect: Expect, memory_warning_mb: Option<u64>) -> ServiceConfig {
        ServiceConfig {
            unit: "app.service".into(),
            name: None,
            expect,
            memory_warning_mb,
        }
    }

    fn unit(load: &str, active: &str, sub: &str) -> UnitSnapshot {
        UnitSnapshot {
            unit: "app.service".into(),
            load_state: load.into(),
            active_state: active.into(),
            sub_state: sub.into(),
            description: "App".into(),
            active_enter_ts: Some(1),
            n_restarts: Some(0),
            result: Some("success".into()),
            exec_main_status: Some(0),
            exec_main_start_ts: Some(1),
            exec_main_exit_ts: None,
            invocation_id: None,
            memory_current: Some(100 * MIB),
            last_trigger_ts: None,
            next_elapse_ts: None,
            trigger_unit: None,
        }
    }

    fn failed(result: Option<&str>, status: Option<i32>) -> UnitSnapshot {
        UnitSnapshot {
            result: result.map(Into::into),
            exec_main_status: status,
            ..unit("loaded", "failed", "failed")
        }
    }

    /// (expect, memory_warning_mb, memory_current, health, message)
    type MemRow = (
        Expect,
        Option<u64>,
        Option<u64>,
        Health,
        Option<&'static str>,
    );

    /// (expect, unit, restarted_recently, health, message)
    type Row = (Expect, UnitSnapshot, bool, Health, Option<&'static str>);

    fn check(rows: Vec<Row>) {
        for (expect, u, restarted, health, msg) in rows {
            let case = format!(
                "{expect:?} {}/{}/{} restarted={restarted}",
                u.load_state, u.active_state, u.sub_state
            );
            let a = evaluate_service(&cfg(expect, None), &u, restarted);
            assert_eq!(a.health, health, "{case}");
            assert_eq!(a.message.as_deref(), msg, "{case}");
        }
    }

    #[test]
    fn expect_running_rows() {
        use Expect::Running as R;
        check(vec![
            (R, unit("loaded", "active", "running"), false, Healthy, None),
            (
                R,
                unit("loaded", "activating", "start-pre"),
                false,
                Warning,
                Some("activating (start-pre)"),
            ),
            (
                R,
                unit("loaded", "reloading", "reload"),
                false,
                Warning,
                Some("reloading (reload)"),
            ),
            (
                R,
                unit("loaded", "deactivating", "stop-sigterm"),
                false,
                Warning,
                Some("deactivating (stop-sigterm)"),
            ),
            (
                R,
                unit("loaded", "active", "running"),
                true,
                Warning,
                Some("restarted in the last hour (0 since last manual start)"),
            ),
            (
                R,
                unit("loaded", "active", "exited"),
                false,
                Warning,
                Some("active (exited), expected running"),
            ),
            (
                R,
                failed(Some("exit-code"), Some(1)),
                false,
                Critical,
                Some("failed: exit-code (status 1)"),
            ),
            (
                R,
                unit("loaded", "inactive", "dead"),
                false,
                Critical,
                Some("inactive (dead)"),
            ),
            (
                R,
                unit("not-found", "inactive", "dead"),
                false,
                Critical,
                Some("unit not found"),
            ),
            (
                R,
                unit("masked", "inactive", "dead"),
                false,
                Critical,
                Some("unit is masked"),
            ),
            (
                R,
                unit("error", "inactive", "dead"),
                false,
                Critical,
                Some("unit is error"),
            ),
        ]);
    }

    #[test]
    fn expect_active_rows() {
        use Expect::Active as A;
        check(vec![
            (A, unit("loaded", "active", "running"), false, Healthy, None),
            (A, unit("loaded", "active", "exited"), false, Healthy, None),
            (
                A,
                unit("loaded", "activating", "start"),
                false,
                Warning,
                Some("activating (start)"),
            ),
            (
                A,
                unit("loaded", "active", "exited"),
                true,
                Warning,
                Some("restarted in the last hour (0 since last manual start)"),
            ),
            (
                A,
                failed(Some("timeout"), None),
                false,
                Critical,
                Some("failed: timeout"),
            ),
            (
                A,
                unit("loaded", "inactive", "dead"),
                false,
                Critical,
                Some("inactive (dead)"),
            ),
            (
                A,
                unit("not-found", "inactive", "dead"),
                false,
                Critical,
                Some("unit not found"),
            ),
        ]);
    }

    #[test]
    fn expect_any_rows() {
        use Expect::Any as Y;
        check(vec![
            (Y, unit("loaded", "active", "running"), false, Healthy, None),
            (Y, unit("loaded", "inactive", "dead"), false, Healthy, None),
            (
                Y,
                unit("loaded", "activating", "start"),
                false,
                Healthy,
                None,
            ),
            // No warnings for `any`, so restarts do not count either.
            (Y, unit("loaded", "active", "running"), true, Healthy, None),
            (
                Y,
                failed(Some("signal"), Some(9)),
                false,
                Critical,
                Some("failed: signal"),
            ),
            (
                Y,
                unit("not-found", "inactive", "dead"),
                false,
                Critical,
                Some("unit not found"),
            ),
        ]);
    }

    #[test]
    fn failed_messages() {
        let cases = [
            (
                failed(Some("exit-code"), Some(2)),
                "failed: exit-code (status 2)",
            ),
            (failed(Some("exit-code"), None), "failed: exit-code"),
            (failed(Some("core-dump"), Some(11)), "failed: core-dump"),
            (failed(Some("success"), Some(0)), "failed"),
            (failed(None, None), "failed"),
        ];
        for (u, expected) in cases {
            let a = evaluate_service(&cfg(Expect::Running, None), &u, false);
            assert_eq!(a.message.as_deref(), Some(expected));
        }
    }

    #[test]
    fn memory_warning_rows() {
        let with_mem = |bytes: Option<u64>| UnitSnapshot {
            memory_current: bytes,
            ..unit("loaded", "active", "running")
        };
        let cases: &[MemRow] = &[
            (Expect::Running, Some(2048), Some(2048 * MIB), Healthy, None),
            (
                Expect::Running,
                Some(2048),
                Some(2048 * MIB + 1),
                Warning,
                Some("memory 2048 MB (warning > 2048 MB)"),
            ),
            (
                Expect::Active,
                Some(100),
                Some(300 * MIB),
                Warning,
                Some("memory 300 MB (warning > 100 MB)"),
            ),
            (Expect::Running, None, Some(u64::MAX), Healthy, None),
            (Expect::Running, Some(1), None, Healthy, None),
            (Expect::Any, Some(1), Some(300 * MIB), Healthy, None),
        ];
        for (expect, limit, bytes, health, msg) in cases {
            let a = evaluate_service(&cfg(*expect, *limit), &with_mem(*bytes), false);
            let case = format!("{expect:?} limit {limit:?} bytes {bytes:?}");
            assert_eq!(a.health, *health, "{case}");
            assert_eq!(a.message.as_deref(), *msg, "{case}");
        }
    }

    #[test]
    fn warnings_stack_and_critical_wins() {
        let u = UnitSnapshot {
            memory_current: Some(300 * MIB),
            n_restarts: Some(4),
            ..unit("loaded", "activating", "auto-restart")
        };
        let a = evaluate_service(&cfg(Expect::Running, Some(100)), &u, true);
        assert_eq!(a.health, Warning);
        assert_eq!(
            a.message.as_deref(),
            Some(
                "activating (auto-restart); restarted in the last hour (4 since last manual \
                 start); memory 300 MB (warning > 100 MB)"
            )
        );

        let u = UnitSnapshot {
            n_restarts: Some(4),
            ..unit("loaded", "inactive", "dead")
        };
        let a = evaluate_service(&cfg(Expect::Running, None), &u, true);
        assert_eq!(a.health, Critical);
        assert_eq!(
            a.message.as_deref(),
            Some("inactive (dead); restarted in the last hour (4 since last manual start)")
        );
    }

    #[test]
    fn restart_tracker() {
        let mut t = RestartTracker::default();
        t.observe(0, Some(3)); // baseline, not an increase
        assert!(!t.restarted_recently(0));
        t.observe(30, None); // no data changes nothing
        t.observe(60, Some(3));
        assert!(!t.restarted_recently(60));
        t.observe(90, Some(4));
        assert!(t.restarted_recently(90));
        assert!(t.restarted_recently(90 + RESTART_WINDOW_S));
        assert!(!t.restarted_recently(91 + RESTART_WINDOW_S));
        // Manual restart resets the counter: a new baseline, the earlier increase still counts.
        t.observe(120, Some(0));
        assert!(t.restarted_recently(120));
        t.observe(200, Some(1));
        assert!(t.restarted_recently(200 + RESTART_WINDOW_S));
    }
}
