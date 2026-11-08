//! Health of configured HTTP/TCP checks (§8, "Check" row).

use super::{Assessment, Health, is_stale, short_duration};
use crate::config::CheckConfig;

/// Consecutive failures from which a check is critical; fewer are a warning.
pub const CRITICAL_FAILURES: u32 = 3;

/// The result of one probe.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeOutcome {
    Success {
        latency_ms: f64,
    },
    /// `reason` is a short message such as `"connection refused"` that never contains the
    /// configured URL. `latency_ms` is set when a response arrived but did not match (e.g. an
    /// unexpected status).
    Failure {
        reason: String,
        latency_ms: Option<f64>,
    },
}

impl ProbeOutcome {
    pub fn latency_ms(&self) -> Option<f64> {
        match self {
            ProbeOutcome::Success { latency_ms } => Some(*latency_ms),
            ProbeOutcome::Failure { latency_ms, .. } => *latency_ms,
        }
    }
}

/// The last probe of one check and the current run of failures. The scheduler keeps one per
/// check and feeds it every probe.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CheckTracker {
    /// Unix seconds and outcome of the last completed probe.
    pub last: Option<(i64, ProbeOutcome)>,
    pub consecutive_failures: u32,
}

impl CheckTracker {
    /// A tracker for a check whose last logged state, restored from storage at startup, was
    /// `health`. The failure count lives only in memory, so a check that was critical before a
    /// restart starts at [`CRITICAL_FAILURES`]: if it is still down, its first probe stays critical
    /// instead of logging a spurious critical → warning → critical pair. A success resets the
    /// count as usual. A restored warning is not seeded: it may have been "slow" rather than a
    /// failure, and seeding must never escalate on old information. `last` stays `None`, so the
    /// check is "not yet run" until its first probe.
    pub fn restored(health: Health) -> Self {
        Self {
            last: None,
            consecutive_failures: if health == Health::Critical {
                CRITICAL_FAILURES
            } else {
                0
            },
        }
    }

    pub fn record(&mut self, ts: i64, outcome: ProbeOutcome) {
        self.consecutive_failures = match outcome {
            ProbeOutcome::Success { .. } => 0,
            ProbeOutcome::Failure { .. } => self.consecutive_failures.saturating_add(1),
        };
        self.last = Some((ts, outcome));
    }
}

/// Evaluates one check at `now`.
///
/// - not yet run, or no probe completed within 3× the interval: unknown;
/// - 1–2 consecutive failures: warning; [`CRITICAL_FAILURES`] or more: critical;
/// - success with latency ≥ `latency_warning`: warning; otherwise healthy.
///
/// A failed probe is a result, not a collection error, so it does not count against staleness.
/// Probes are bounded by `timeout ≤ interval`, so a stale check means its runner is stuck.
pub fn evaluate_check(cfg: &CheckConfig, tracker: &CheckTracker, now: i64) -> Assessment {
    let Some((ts, outcome)) = &tracker.last else {
        return Assessment::unknown("not yet run");
    };
    if is_stale(Some(*ts), now, cfg.interval) {
        return Assessment::unknown(format!("no result for {}s", now.saturating_sub(*ts)));
    }
    match outcome {
        ProbeOutcome::Failure { reason, .. } => {
            let n = tracker.consecutive_failures.max(1);
            let health = if n >= CRITICAL_FAILURES {
                Health::Critical
            } else {
                Health::Warning
            };
            let plural = if n == 1 { "" } else { "s" };
            Assessment::new(
                health,
                format!("{reason} ({n} consecutive failure{plural})"),
            )
        }
        ProbeOutcome::Success { latency_ms } => match cfg.latency_warning {
            Some(limit) if *latency_ms >= limit.as_secs_f64() * 1000.0 => Assessment::new(
                Health::Warning,
                format!(
                    "slow: {} ms (warning ≥ {})",
                    latency_ms.round(),
                    limit_text(limit)
                ),
            ),
            _ => Assessment::healthy(),
        },
    }
}

/// `1s`, `2m`, or `250ms` for a sub-second limit.
fn limit_text(limit: std::time::Duration) -> String {
    if limit.subsec_millis() != 0 || limit.as_secs() == 0 {
        format!("{}ms", limit.as_millis())
    } else {
        short_duration(limit)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::config::CheckKind;
    use Health::*;

    const NOW: i64 = 1_794_200_000;

    fn cfg(latency_warning: Option<Duration>) -> CheckConfig {
        CheckConfig {
            name: "c".into(),
            kind: CheckKind::Tcp,
            url: None,
            address: Some("127.0.0.1:1".into()),
            expect_status: None,
            interval: Duration::from_secs(60),
            timeout: Duration::from_secs(5),
            latency_warning,
        }
    }

    fn ok(ms: f64) -> ProbeOutcome {
        ProbeOutcome::Success { latency_ms: ms }
    }

    fn fail() -> ProbeOutcome {
        ProbeOutcome::Failure {
            reason: "connection refused".into(),
            latency_ms: None,
        }
    }

    /// Feeds `outcomes` (one per minute, the last at `NOW`) and evaluates at `NOW`.
    fn after(outcomes: &[ProbeOutcome], latency_warning: Option<Duration>) -> Assessment {
        let mut t = CheckTracker::default();
        let n = outcomes.len() as i64;
        for (i, o) in outcomes.iter().enumerate() {
            t.record(NOW - (n - 1 - i as i64) * 60, o.clone());
        }
        evaluate_check(&cfg(latency_warning), &t, NOW)
    }

    /// `(case, outcomes, latency_warning, expected health, expected message)`.
    type Row = (
        &'static str,
        Vec<ProbeOutcome>,
        Option<Duration>,
        Health,
        Option<&'static str>,
    );

    #[test]
    fn check_rows() {
        let slow = Some(Duration::from_secs(1));
        let rows: Vec<Row> = vec![
            ("not yet run", vec![], None, Unknown, Some("not yet run")),
            ("success", vec![ok(3.1)], None, Healthy, None),
            ("fast with a limit", vec![ok(999.9)], slow, Healthy, None),
            (
                "slow",
                vec![ok(1000.0)],
                slow,
                Warning,
                Some("slow: 1000 ms (warning ≥ 1s)"),
            ),
            (
                "one failure",
                vec![ok(1.0), fail()],
                None,
                Warning,
                Some("connection refused (1 consecutive failure)"),
            ),
            (
                "two failures",
                vec![fail(), fail()],
                None,
                Warning,
                Some("connection refused (2 consecutive failures)"),
            ),
            (
                "three failures",
                vec![fail(), fail(), fail()],
                None,
                Critical,
                Some("connection refused (3 consecutive failures)"),
            ),
            (
                "recovery resets the count",
                vec![fail(), fail(), fail(), ok(2.0)],
                None,
                Healthy,
                None,
            ),
            (
                "count restarts after recovery",
                vec![fail(), fail(), ok(2.0), fail()],
                None,
                Warning,
                Some("connection refused (1 consecutive failure)"),
            ),
        ];
        for (case, outcomes, limit, health, message) in rows {
            let a = after(&outcomes, limit);
            assert_eq!(
                (a.health, a.message.as_deref()),
                (health, message),
                "{case}"
            );
        }
    }

    #[test]
    fn failure_counts_track_outcomes() {
        let mut t = CheckTracker::default();
        t.record(NOW, fail());
        t.record(NOW, fail());
        assert_eq!(t.consecutive_failures, 2);
        t.record(NOW, ok(1.0));
        assert_eq!(t.consecutive_failures, 0);
        assert_eq!(t.last.as_ref().and_then(|(_, o)| o.latency_ms()), Some(1.0));
    }

    #[test]
    fn restored_critical_stays_critical_while_down() {
        let mut t = CheckTracker::restored(Critical);
        assert_eq!(evaluate_check(&cfg(None), &t, NOW).health, Unknown);
        t.record(NOW, fail());
        let a = evaluate_check(&cfg(None), &t, NOW);
        assert_eq!(
            (a.health, a.message.as_deref()),
            (
                Critical,
                Some("connection refused (4 consecutive failures)")
            )
        );
        t.record(NOW, ok(1.0));
        assert_eq!(evaluate_check(&cfg(None), &t, NOW).health, Healthy);

        for h in [Healthy, Warning, Unknown] {
            assert_eq!(CheckTracker::restored(h), CheckTracker::default(), "{h}");
        }
    }

    #[test]
    fn stale_after_three_intervals() {
        let mut t = CheckTracker::default();
        t.record(NOW - 180, ok(1.0));
        assert_eq!(evaluate_check(&cfg(None), &t, NOW).health, Healthy);
        let a = evaluate_check(&cfg(None), &t, NOW + 1);
        assert_eq!(
            (a.health, a.message.as_deref()),
            (Unknown, Some("no result for 181s"))
        );
        // A stale failure is unknown too, not critical.
        t.record(NOW - 600, fail());
        assert_eq!(evaluate_check(&cfg(None), &t, NOW).health, Unknown);
    }

    #[test]
    fn sub_second_limits_print_in_ms() {
        assert_eq!(limit_text(Duration::from_millis(250)), "250ms");
        assert_eq!(limit_text(Duration::from_millis(1500)), "1500ms");
        assert_eq!(limit_text(Duration::from_secs(2)), "2s");
    }
}
