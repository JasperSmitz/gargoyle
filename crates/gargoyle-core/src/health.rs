//! The four-state health model and the rules that produce it (ARCHITECTURE §8).
//!
//! Every rule is a pure function from config, current input and (where a rule needs history) a
//! small piece of previous state, to an [`Assessment`]. The scheduler owns that state and calls
//! these functions; nothing here does IO or reads the clock.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::api::HealthCounts;
use crate::config::Levels;

mod service;
mod system;
mod window;

pub use service::{RESTART_WINDOW_S, RestartTracker, evaluate_service};
pub use system::{
    SystemHealth, disk_used_pct, evaluate_system, load_per_core, memory_used_pct, swap_used_pct,
};
pub use window::Window;

/// Health of a target or aggregate.
///
/// The variant order *is* the severity order (`critical > warning > unknown > healthy`), so the
/// derived `Ord` is what aggregation uses. Do not reorder the variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Healthy,
    Unknown,
    Warning,
    Critical,
}

impl Health {
    /// The worse of two states.
    #[must_use]
    pub fn worst_of(self, other: Health) -> Health {
        self.max(other)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Health::Healthy => "healthy",
            Health::Unknown => "unknown",
            Health::Warning => "warning",
            Health::Critical => "critical",
        }
    }
}

impl std::fmt::Display for Health {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The worst state among `parts`.
///
/// An empty aggregate is `Healthy`: having nothing configured is not a problem to report.
pub fn worst<I: IntoIterator<Item = Health>>(parts: I) -> Health {
    parts.into_iter().max().unwrap_or(Health::Healthy)
}

/// The health of one target and, unless healthy, why.
///
/// ARCHITECTURE §8 describes the result as `(Health, Option<String>)`; this is that pair with
/// names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub health: Health,
    /// `None` exactly when `health` is `Healthy`.
    pub message: Option<String>,
}

impl Assessment {
    pub fn healthy() -> Self {
        Self {
            health: Health::Healthy,
            message: None,
        }
    }

    /// A non-healthy state with its reason. A `Healthy` state drops the message.
    pub fn new(health: Health, message: impl Into<String>) -> Self {
        Self {
            health,
            message: (health != Health::Healthy).then(|| message.into()),
        }
    }

    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new(Health::Unknown, message)
    }

    /// The worst of `parts`, with the messages of every non-healthy part joined by `"; "`, most
    /// severe first. Parts of equal severity keep their order.
    pub fn combine<'a, I: IntoIterator<Item = &'a Assessment>>(parts: I) -> Assessment {
        let mut bad: Vec<&Assessment> = parts
            .into_iter()
            .filter(|a| a.health != Health::Healthy)
            .collect();
        // Stable sort, so equal-severity parts stay in the caller's order.
        bad.sort_by_key(|a| std::cmp::Reverse(a.health));
        match bad.first() {
            None => Assessment::healthy(),
            Some(first) => Assessment {
                health: first.health,
                message: Some(
                    bad.iter()
                        .filter_map(|a| a.message.as_deref())
                        .collect::<Vec<_>>()
                        .join("; "),
                ),
            },
        }
    }
}

/// Classifies a metric against its levels: `value < warning` is healthy, `warning ≤ value <
/// critical` is warning, `value ≥ critical` is critical. A non-finite value is unknown.
pub fn threshold(value: f64, levels: Levels) -> Health {
    if !value.is_finite() {
        Health::Unknown
    } else if value >= levels.critical {
        Health::Critical
    } else if value >= levels.warning {
        Health::Warning
    } else {
        Health::Healthy
    }
}

/// True when the last successful sample is missing or older than 3× the collection interval, in
/// which case the target is `unknown` (§3).
pub fn is_stale(last_sample_ts: Option<i64>, now: i64, interval: Duration) -> bool {
    let limit = i64::try_from(interval.as_secs().saturating_mul(3)).unwrap_or(i64::MAX);
    match last_sample_ts {
        None => true,
        Some(ts) => now.saturating_sub(ts) > limit,
    }
}

/// Counts targets per state for `overview.counts`. The caller passes the system as one target.
pub fn tally<I: IntoIterator<Item = Health>>(parts: I) -> HealthCounts {
    let mut c = HealthCounts::default();
    for h in parts {
        match h {
            Health::Healthy => c.healthy += 1,
            Health::Warning => c.warning += 1,
            Health::Critical => c.critical += 1,
            Health::Unknown => c.unknown += 1,
        }
    }
    c
}

/// A number for a message: truncated (not rounded) to one decimal, so a value just below a
/// threshold never prints as the threshold itself, and without a trailing `.0`.
fn num(v: f64) -> String {
    let t = (v * 10.0).floor() / 10.0;
    if t.fract() == 0.0 {
        format!("{t:.0}")
    } else {
        format!("{t:.1}")
    }
}

/// `(warning ≥ 80%)` / `(critical ≥ 90%, 2m average)`: the threshold that was crossed.
fn crossed(health: Health, levels: Levels, unit: &str, window: Option<Duration>) -> String {
    let limit = if health == Health::Critical {
        levels.critical
    } else {
        levels.warning
    };
    let window = window
        .map(|w| format!(", {} average", short_duration(w)))
        .unwrap_or_default();
    format!("({health} ≥ {limit}{unit}{window})")
}

/// `120s` → `2m`, `3600s` → `1h`, `90s` → `90s`.
fn short_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s != 0 && s % 3600 == 0 {
        format!("{}h", s / 3600)
    } else if s != 0 && s % 60 == 0 {
        format!("{}m", s / 60)
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Health::*;

    #[test]
    fn severity_order_matches_architecture() {
        assert!(Critical > Warning);
        assert!(Warning > Unknown);
        assert!(Unknown > Healthy);
    }

    #[test]
    fn worst_picks_most_severe() {
        let cases: &[(&[Health], Health)] = &[
            (&[], Healthy),
            (&[Healthy], Healthy),
            (&[Healthy, Unknown], Unknown),
            (&[Unknown, Warning, Healthy], Warning),
            (&[Warning, Critical, Unknown], Critical),
            (&[Critical, Healthy], Critical),
        ];
        for (parts, expected) in cases {
            assert_eq!(worst(parts.iter().copied()), *expected, "parts: {parts:?}");
        }
    }

    #[test]
    fn worst_of_is_symmetric() {
        let all = [Healthy, Unknown, Warning, Critical];
        for a in all {
            for b in all {
                assert_eq!(a.worst_of(b), b.worst_of(a));
            }
        }
    }

    fn levels(warning: f64, critical: f64) -> Levels {
        Levels { warning, critical }
    }

    #[test]
    fn threshold_boundaries() {
        let l = levels(80.0, 90.0);
        let cases = [
            (0.0, Healthy),
            (79.99, Healthy),
            (80.0, Warning),
            (89.99, Warning),
            (90.0, Critical),
            (100.0, Critical),
            (f64::NAN, Unknown),
            (f64::INFINITY, Unknown),
        ];
        for (value, expected) in cases {
            assert_eq!(threshold(value, l), expected, "value {value}");
        }
    }

    #[test]
    fn staleness_is_three_intervals() {
        let i = Duration::from_secs(10);
        let cases = [
            (None, true),
            (Some(1000), false),
            (Some(970), false),
            (Some(969), true),
            (Some(i64::MIN), true),
        ];
        for (last, expected) in cases {
            assert_eq!(is_stale(last, 1000, i), expected, "last {last:?}");
        }
    }

    #[test]
    fn tally_counts_each_state() {
        let c = tally([
            Healthy, Healthy, Warning, Critical, Unknown, Unknown, Unknown,
        ]);
        assert_eq!(
            c,
            HealthCounts {
                healthy: 2,
                warning: 1,
                critical: 1,
                unknown: 3
            }
        );
        assert_eq!(tally([]), HealthCounts::default());
    }

    #[test]
    fn assessment_new_drops_message_when_healthy() {
        assert_eq!(Assessment::new(Healthy, "x"), Assessment::healthy());
        assert_eq!(Assessment::new(Warning, "x").message.as_deref(), Some("x"));
    }

    #[test]
    fn combine_orders_messages_by_severity() {
        let parts = [
            Assessment::healthy(),
            Assessment::new(Warning, "w1"),
            Assessment::unknown("u"),
            Assessment::new(Critical, "c"),
            Assessment::new(Warning, "w2"),
        ];
        let c = Assessment::combine(&parts);
        assert_eq!(c.health, Critical);
        assert_eq!(c.message.as_deref(), Some("c; w1; w2; u"));
        assert_eq!(
            Assessment::combine(&[Assessment::healthy()]),
            Assessment::healthy()
        );
        assert_eq!(Assessment::combine(&[]), Assessment::healthy());
    }

    #[test]
    fn number_formatting() {
        let cases = [
            (84.0, "84"),
            (84.37, "84.3"),
            (89.99, "89.9"),
            (2.1, "2.1"),
            (0.0, "0"),
        ];
        for (v, expected) in cases {
            assert_eq!(num(v), expected);
        }
    }

    #[test]
    fn crossed_names_the_threshold() {
        let l = levels(80.0, 90.0);
        assert_eq!(crossed(Warning, l, "%", None), "(warning ≥ 80%)");
        assert_eq!(
            crossed(Critical, l, "%", Some(Duration::from_secs(120))),
            "(critical ≥ 90%, 2m average)"
        );
        assert_eq!(short_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(short_duration(Duration::from_secs(90)), "90s");
    }

    #[test]
    fn serializes_lowercase() {
        for h in [Healthy, Unknown, Warning, Critical] {
            let json = serde_json::to_string(&h).unwrap();
            assert_eq!(json, format!("\"{h}\""));
            assert_eq!(serde_json::from_str::<Health>(&json).unwrap(), h);
        }
    }
}
