//! Health of configured timers (§8, "Timer" row) and the run facts it is judged on.
//!
//! A timer is judged on its own unit (active, overdue) and on the last run of the service it
//! triggers. The service's `ActiveState` is deliberately ignored: between runs a oneshot service is
//! `inactive`, which is normal. Only the `Result` of its last run counts (§8, "Inactive timer
//! services").

use std::time::Duration;

use super::service::not_loaded;
use super::{Assessment, Health, short_duration};
use crate::api::LastRun;
use crate::config::TimerConfig;
use crate::model::{TimerRunRecord, UnitSnapshot};

/// How far `NextElapse` may lie in the past before the timer counts as overdue: 10 minutes.
pub const OVERDUE_GRACE_S: i64 = 600;

/// What is known about a timer's runs, folded from the live service snapshot, stored runs and the
/// journal. The scheduler keeps one per timer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimerRuns {
    /// The most recent run, finished or in progress.
    pub last: Option<LastRun>,
    /// When the most recent successful run ended (its start, if the end is unknown).
    pub last_success: Option<i64>,
}

impl TimerRuns {
    /// Folds in one run, from any source and in any order. A run with the same start as `last` is
    /// the same run: its known fields only fill in the ones `last` is missing (a run in progress
    /// that has since finished), never replace them, exactly as storage merges rows, so live
    /// state and state restored after a restart agree.
    pub fn record(&mut self, run: &LastRun) {
        match &mut self.last {
            Some(last) if last.started_at == run.started_at => {
                last.finished_at = last.finished_at.or(run.finished_at);
                last.result = last.result.take().or_else(|| run.result.clone());
                last.exit_status = last.exit_status.or(run.exit_status);
                last.duration = duration(last.started_at, last.finished_at);
            }
            Some(last) if last.started_at > run.started_at => {}
            _ => self.last = Some(run.clone()),
        }
        if run.result.as_deref() == Some("success") {
            let ended = run.finished_at.unwrap_or(run.started_at);
            self.last_success = Some(self.last_success.map_or(ended, |t| t.max(ended)));
        }
    }
}

/// The service a timer triggers: configured explicitly, else the timer's `Unit=` property, else
/// `<timer stem>.service` (systemd's own default).
pub fn timer_service(cfg: &TimerConfig, timer: Option<&UnitSnapshot>) -> String {
    if let Some(service) = &cfg.service {
        return service.clone();
    }
    timer
        .and_then(|t| t.trigger_unit.clone())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| {
            let stem = cfg.unit.strip_suffix(".timer").unwrap_or(&cfg.unit);
            format!("{stem}.service")
        })
}

/// True while the service's main process is starting, running or stopping. A oneshot runs in
/// `activating`; `active (exited)` is a finished `RemainAfterExit` run.
pub fn service_is_running(svc: &UnitSnapshot) -> bool {
    match svc.active_state.as_str() {
        "activating" | "deactivating" | "reloading" => true,
        "active" => svc.sub_state != "exited",
        _ => false,
    }
}

/// The current or last run of `svc` as systemd reports it, or `None` if it has not run since boot
/// (no `ExecMainStartTimestamp`). While running, its result and exit status are not known yet;
/// systemd still shows the previous run's values then, so they are dropped.
pub fn service_run(svc: &UnitSnapshot) -> Option<LastRun> {
    let started_at = svc.exec_main_start_ts.filter(|&t| t > 0)?;
    if service_is_running(svc) {
        return Some(LastRun {
            started_at,
            finished_at: None,
            duration: None,
            result: None,
            exit_status: None,
        });
    }
    // An exit before the start belongs to an earlier run.
    let finished_at = svc.exec_main_exit_ts.filter(|&t| t >= started_at);
    Some(LastRun {
        started_at,
        finished_at,
        duration: duration(started_at, finished_at),
        result: svc.result.clone(),
        exit_status: svc.exec_main_status,
    })
}

/// A stored or journal run as the API's [`LastRun`].
pub fn run_of(record: &TimerRunRecord) -> LastRun {
    LastRun {
        started_at: record.started_at,
        finished_at: record.finished_at,
        duration: duration(record.started_at, record.finished_at),
        result: record.result.clone(),
        exit_status: record.exit_status,
    }
}

fn duration(started_at: i64, finished_at: Option<i64>) -> Option<u64> {
    finished_at.and_then(|f| u64::try_from(f - started_at).ok())
}

/// Evaluates one timer at `now`.
///
/// - The timer unit not loaded is critical, as for services.
/// - Not `active` (stopped or disabled) is a warning.
/// - While its service runs, the last run's result is not judged (its outcome is not known yet).
/// - Otherwise: never run is unknown; a last run whose result is not `success` is critical; an
///   unknown result is unknown.
/// - With `max_age`, once the timer has run, no success within that age is a warning, also while
///   running, so a job hung in `activating` does not stay healthy.
/// - `NextElapse` more than [`OVERDUE_GRACE_S`] in the past is a warning (overdue).
///
/// Collector errors and staleness are not decided here; the caller reports those as
/// [`Assessment::unknown`].
pub fn evaluate_timer(
    cfg: &TimerConfig,
    timer: &UnitSnapshot,
    running: bool,
    runs: &TimerRuns,
    now: i64,
) -> Assessment {
    if let Some(a) = not_loaded(timer) {
        return a;
    }
    let mut parts = Vec::new();
    if timer.active_state != "active" {
        parts.push(Assessment::new(
            Health::Warning,
            format!("timer {} ({})", timer.active_state, timer.sub_state),
        ));
    }
    match &runs.last {
        None if !running => parts.push(Assessment::unknown("never run")),
        None => {}
        Some(run) => {
            if !running {
                parts.push(last_result(run));
            }
            if let Some(max_age) = cfg.max_age {
                parts.push(success_age(runs.last_success, max_age, now));
            }
        }
    }
    if let Some(next) = timer.next_elapse_ts
        && now.saturating_sub(next) > OVERDUE_GRACE_S
    {
        parts.push(Assessment::new(
            Health::Warning,
            format!("overdue: next run was due {} ago", ago(now - next)),
        ));
    }
    Assessment::combine(&parts)
}

/// `last run failed: exit-code (status 1)` / `last run failed: signal`.
fn last_result(run: &LastRun) -> Assessment {
    match (run.result.as_deref(), run.exit_status) {
        (Some("success"), _) => Assessment::healthy(),
        (Some("exit-code"), Some(status)) => Assessment::new(
            Health::Critical,
            format!("last run failed: exit-code (status {status})"),
        ),
        (Some(result), _) => {
            Assessment::new(Health::Critical, format!("last run failed: {result}"))
        }
        (None, _) => Assessment::unknown("last run result unknown"),
    }
}

fn success_age(last_success: Option<i64>, max_age: Duration, now: i64) -> Assessment {
    let limit = i64::try_from(max_age.as_secs()).unwrap_or(i64::MAX);
    match last_success {
        Some(ts) if now.saturating_sub(ts) <= limit => Assessment::healthy(),
        Some(ts) => Assessment::new(
            Health::Warning,
            format!(
                "last success {} ago (warning > {})",
                ago(now - ts),
                short_duration(max_age)
            ),
        ),
        None => Assessment::new(
            Health::Warning,
            format!(
                "no successful run recorded within {}",
                short_duration(max_age)
            ),
        ),
    }
}

/// A coarse age, floored: `90s`, `25m`, `27h`, `3d`.
fn ago(secs: i64) -> String {
    match secs {
        s if s < 120 => format!("{s}s"),
        s if s < 120 * 60 => format!("{}m", s / 60),
        s if s < 48 * 3600 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Health::*;

    const NOW: i64 = 1_794_200_000;
    const H: i64 = 3600;

    fn cfg(max_age: Option<u64>) -> TimerConfig {
        TimerConfig {
            unit: "music-sync.timer".into(),
            name: None,
            service: None,
            max_age: max_age.map(Duration::from_secs),
        }
    }

    fn snap(unit: &str, load: &str, active: &str, sub: &str) -> UnitSnapshot {
        UnitSnapshot {
            unit: unit.into(),
            load_state: load.into(),
            active_state: active.into(),
            sub_state: sub.into(),
            description: String::new(),
            active_enter_ts: None,
            n_restarts: None,
            result: None,
            exec_main_status: None,
            exec_main_start_ts: None,
            exec_main_exit_ts: None,
            invocation_id: None,
            memory_current: None,
            last_trigger_ts: None,
            next_elapse_ts: None,
            trigger_unit: None,
        }
    }

    fn timer(active: &str, sub: &str, next: Option<i64>) -> UnitSnapshot {
        UnitSnapshot {
            next_elapse_ts: next,
            ..snap("music-sync.timer", "loaded", active, sub)
        }
    }

    fn run(
        started: i64,
        finished: Option<i64>,
        result: Option<&str>,
        status: Option<i32>,
    ) -> LastRun {
        LastRun {
            started_at: started,
            finished_at: finished,
            duration: duration(started, finished),
            result: result.map(Into::into),
            exit_status: status,
        }
    }

    fn ran(r: LastRun) -> TimerRuns {
        let mut runs = TimerRuns::default();
        runs.record(&r);
        runs
    }

    fn ok_run(ago_s: i64) -> TimerRuns {
        ran(run(
            NOW - ago_s - 40,
            Some(NOW - ago_s),
            Some("success"),
            Some(0),
        ))
    }

    /// (case, max_age, timer, running, runs, health, message)
    type Row = (
        &'static str,
        Option<u64>,
        UnitSnapshot,
        bool,
        TimerRuns,
        Health,
        Option<&'static str>,
    );

    #[test]
    fn timer_rows() {
        let next = Some(NOW + H);
        let active = || timer("active", "waiting", next);
        let rows: Vec<Row> = vec![
            (
                "last run ok",
                None,
                active(),
                false,
                ok_run(H),
                Healthy,
                None,
            ),
            (
                "running, never finished",
                None,
                active(),
                true,
                TimerRuns::default(),
                Healthy,
                None,
            ),
            (
                "running after a failure",
                None,
                active(),
                true,
                ran(run(NOW - H, Some(NOW - H + 5), Some("exit-code"), Some(1))),
                Healthy,
                None,
            ),
            (
                "never run",
                None,
                active(),
                false,
                TimerRuns::default(),
                Unknown,
                Some("never run"),
            ),
            (
                "failed with status",
                None,
                active(),
                false,
                ran(run(NOW - H, Some(NOW - H + 42), Some("exit-code"), Some(1))),
                Critical,
                Some("last run failed: exit-code (status 1)"),
            ),
            (
                "failed by signal",
                None,
                active(),
                false,
                ran(run(NOW - H, Some(NOW - H + 42), Some("signal"), Some(9))),
                Critical,
                Some("last run failed: signal"),
            ),
            (
                "result unknown",
                None,
                active(),
                false,
                ran(run(NOW - H, None, None, None)),
                Unknown,
                Some("last run result unknown"),
            ),
            (
                "timer stopped",
                None,
                timer("inactive", "dead", None),
                false,
                ok_run(H),
                Warning,
                Some("timer inactive (dead)"),
            ),
            (
                "timer stopped and never run",
                None,
                timer("inactive", "dead", None),
                false,
                TimerRuns::default(),
                Warning,
                Some("timer inactive (dead); never run"),
            ),
            (
                "not found",
                None,
                UnitSnapshot {
                    ..snap("music-sync.timer", "not-found", "inactive", "dead")
                },
                false,
                ok_run(H),
                Critical,
                Some("unit not found"),
            ),
            (
                "masked",
                None,
                snap("music-sync.timer", "masked", "inactive", "dead"),
                false,
                ok_run(H),
                Critical,
                Some("unit is masked"),
            ),
            // Overdue: strictly more than 10 minutes.
            (
                "due 10m ago",
                None,
                timer("active", "waiting", Some(NOW - 600)),
                false,
                ok_run(H),
                Healthy,
                None,
            ),
            (
                "due 10m01s ago",
                None,
                timer("active", "waiting", Some(NOW - 601)),
                false,
                ok_run(H),
                Warning,
                Some("overdue: next run was due 10m ago"),
            ),
            (
                "overdue and failed",
                None,
                timer("active", "waiting", Some(NOW - 3 * H)),
                false,
                ran(run(
                    NOW - 30 * H,
                    Some(NOW - 30 * H + 1),
                    Some("timeout"),
                    None,
                )),
                Critical,
                Some("last run failed: timeout; overdue: next run was due 3h ago"),
            ),
            // max_age (26h): age measured from the end of the last success, inclusive limit.
            (
                "success at max_age",
                Some(26 * 3600),
                active(),
                false,
                ok_run(26 * H),
                Healthy,
                None,
            ),
            (
                "success older than max_age",
                Some(26 * 3600),
                active(),
                false,
                ok_run(26 * H + 1),
                Warning,
                Some("last success 26h ago (warning > 26h)"),
            ),
            (
                "failed and no recent success",
                Some(86_400),
                active(),
                false,
                {
                    let mut r = ok_run(3 * 86_400);
                    r.record(&run(NOW - H, Some(NOW - H + 1), Some("exit-code"), Some(2)));
                    r
                },
                Critical,
                Some("last run failed: exit-code (status 2); last success 3d ago (warning > 24h)"),
            ),
            (
                "failed, success never seen",
                Some(86_400),
                active(),
                false,
                ran(run(NOW - H, Some(NOW - H + 1), Some("exit-code"), Some(1))),
                Critical,
                Some(
                    "last run failed: exit-code (status 1); no successful run recorded within 24h",
                ),
            ),
            // max_age is judged while running (a hung job), but not before the first run.
            (
                "running past max_age",
                Some(3600),
                active(),
                true,
                ok_run(5 * H),
                Warning,
                Some("last success 5h ago (warning > 1h)"),
            ),
            (
                "running within max_age",
                Some(26 * 3600),
                active(),
                true,
                ok_run(H),
                Healthy,
                None,
            ),
            (
                "hung after an old success",
                Some(26 * 3600),
                active(),
                true,
                {
                    let mut r = ok_run(3 * 86_400);
                    r.record(&run(NOW - 3 * 86_400 + 60, None, None, None));
                    r
                },
                Warning,
                Some("last success 3d ago (warning > 26h)"),
            ),
            (
                "never run with max_age",
                Some(3600),
                active(),
                false,
                TimerRuns::default(),
                Unknown,
                Some("never run"),
            ),
        ];
        for (case, max_age, t, running, runs, health, msg) in rows {
            let a = evaluate_timer(&cfg(max_age), &t, running, &runs, NOW);
            assert_eq!(a.health, health, "{case}");
            assert_eq!(a.message.as_deref(), msg, "{case}");
        }
    }

    #[test]
    fn inactive_triggered_service_never_counts_against_the_timer() {
        // Between runs the oneshot service is inactive/dead; only the run's Result matters.
        let svc = UnitSnapshot {
            result: Some("success".into()),
            exec_main_status: Some(0),
            exec_main_start_ts: Some(NOW - H),
            exec_main_exit_ts: Some(NOW - H + 30),
            ..snap("music-sync.service", "loaded", "inactive", "dead")
        };
        assert!(!service_is_running(&svc));
        let runs = ran(service_run(&svc).expect("has run"));
        let a = evaluate_timer(
            &cfg(None),
            &timer("active", "waiting", Some(NOW + H)),
            false,
            &runs,
            NOW,
        );
        assert_eq!(a, Assessment::healthy());
    }

    #[test]
    fn running_states() {
        let rows = [
            ("activating", "start", true),
            ("active", "running", true),
            ("deactivating", "stop-sigterm", true),
            ("reloading", "reload", true),
            ("active", "exited", false),
            ("inactive", "dead", false),
            ("failed", "failed", false),
        ];
        for (active, sub, expected) in rows {
            let s = snap("a.service", "loaded", active, sub);
            assert_eq!(service_is_running(&s), expected, "{active} ({sub})");
        }
    }

    #[test]
    fn service_run_from_snapshot() {
        let base = UnitSnapshot {
            result: Some("exit-code".into()),
            exec_main_status: Some(3),
            exec_main_start_ts: Some(100),
            exec_main_exit_ts: Some(142),
            ..snap("a.service", "loaded", "failed", "failed")
        };
        assert_eq!(
            service_run(&base),
            Some(run(100, Some(142), Some("exit-code"), Some(3)))
        );
        // Running: the previous run's result is not this run's.
        let running = UnitSnapshot {
            active_state: "activating".into(),
            sub_state: "start".into(),
            ..base.clone()
        };
        assert_eq!(service_run(&running), Some(run(100, None, None, None)));
        // An exit timestamp older than the start belongs to an earlier run.
        let stale_exit = UnitSnapshot {
            exec_main_exit_ts: Some(90),
            ..base.clone()
        };
        assert_eq!(
            service_run(&stale_exit),
            Some(run(100, None, Some("exit-code"), Some(3)))
        );
        // Not run since boot.
        for start in [None, Some(0)] {
            let never = UnitSnapshot {
                exec_main_start_ts: start,
                ..base.clone()
            };
            assert_eq!(service_run(&never), None);
        }
    }

    #[test]
    fn record_merges_and_keeps_the_newest() {
        let mut runs = TimerRuns::default();
        runs.record(&run(100, None, None, None));
        assert_eq!(runs.last, Some(run(100, None, None, None)));
        // The same run finishes.
        runs.record(&run(100, Some(130), Some("success"), Some(0)));
        assert_eq!(
            runs.last,
            Some(run(100, Some(130), Some("success"), Some(0)))
        );
        assert_eq!(runs.last_success, Some(130));
        // Known facts are kept, as storage keeps them.
        runs.record(&run(100, Some(140), Some("exit-code"), Some(1)));
        assert_eq!(
            runs.last,
            Some(run(100, Some(130), Some("success"), Some(0)))
        );
        // An older run (e.g. from the journal) does not replace it, but a success still counts.
        runs.record(&run(50, Some(60), Some("exit-code"), Some(1)));
        assert_eq!(runs.last.as_ref().map(|r| r.started_at), Some(100));
        runs.record(&run(10, Some(200), Some("success"), Some(0)));
        assert_eq!(runs.last_success, Some(200));
        // A newer run replaces it; a failure keeps the last success.
        runs.record(&run(300, Some(301), Some("exit-code"), Some(1)));
        assert_eq!(runs.last.as_ref().map(|r| r.started_at), Some(300));
        assert_eq!(runs.last_success, Some(200));
        // A success without an end counts from its start.
        runs.record(&run(400, None, Some("success"), None));
        assert_eq!(runs.last_success, Some(400));
    }

    #[test]
    fn service_name_resolution() {
        let t = |trigger: Option<&str>| UnitSnapshot {
            trigger_unit: trigger.map(Into::into),
            ..timer("active", "waiting", None)
        };
        assert_eq!(timer_service(&cfg(None), None), "music-sync.service");
        assert_eq!(
            timer_service(&cfg(None), Some(&t(Some("sync.service")))),
            "sync.service"
        );
        assert_eq!(
            timer_service(&cfg(None), Some(&t(Some("")))),
            "music-sync.service"
        );
        let explicit = TimerConfig {
            service: Some("other.service".into()),
            ..cfg(None)
        };
        assert_eq!(
            timer_service(&explicit, Some(&t(Some("sync.service")))),
            "other.service"
        );
    }

    #[test]
    fn run_of_record() {
        let r = TimerRunRecord {
            timer: "t.timer".into(),
            service: "t.service".into(),
            invocation_id: None,
            started_at: 10,
            finished_at: Some(52),
            result: Some("success".into()),
            exit_status: Some(0),
        };
        assert_eq!(run_of(&r), run(10, Some(52), Some("success"), Some(0)));
        assert_eq!(run_of(&r).duration, Some(42));
    }

    #[test]
    fn ages() {
        let cases = [
            (5, "5s"),
            (119, "119s"),
            (120, "2m"),
            (7199, "119m"),
            (7200, "2h"),
            (172_799, "47h"),
            (172_800, "2d"),
        ];
        for (s, expected) in cases {
            assert_eq!(ago(s), expected);
        }
    }
}
