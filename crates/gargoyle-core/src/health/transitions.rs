//! Detecting health transitions, which become `events` rows (ARCHITECTURE §3, §6 "Event").
//!
//! Rules, in order:
//! - **Hold-down:** a new state is logged only once it has been observed on
//!   [`CONFIRMATIONS`] consecutive evaluations, so a value hovering at a threshold does not log an
//!   event every tick. The event carries the time and message of the confirming observation. Live
//!   health in the API is not affected.
//! - A confirmed change from the last logged state is a transition.
//! - The first known state of a target is a transition (`from = null`) only if it is not healthy:
//!   a fresh install should not log one "healthy" event per target, but a target that is already
//!   broken when first seen must show up in the log. A healthy first state is the baseline at once.
//! - `unknown` never becomes a target's *first* state. At startup some metrics are unknown for a
//!   tick (CPU needs two samples). A later change from a known state to `unknown` (collection
//!   failing, stale data) is logged.
//! - Baselines restored from storage (the last logged state per target) are compared against, so
//!   a restart does not log the same state again. Until a target has been observed live, an
//!   `unknown` observation does not count against its restored baseline, for the startup reason
//!   above.

use std::collections::HashMap;

use super::{Assessment, Health};
use crate::api::TargetKind;

/// One health change of one target, at the time it was observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub kind: TargetKind,
    pub target: String,
    /// `None` for the target's first logged state.
    pub from: Option<Health>,
    pub to: Health,
    /// `None` when healthy.
    pub message: Option<String>,
}

/// Consecutive observations of a new state needed before it is logged.
pub const CONFIRMATIONS: u32 = 2;

#[derive(Debug, Clone, Copy, Default)]
struct Entry {
    /// The last logged (or silently accepted healthy first) state.
    logged: Option<Health>,
    /// Observed by this process, as opposed to restored from storage.
    live: bool,
    /// A state different from `logged`, and how many consecutive times it has been seen.
    pending: Option<(Health, u32)>,
}

/// The last known state of every target.
#[derive(Debug, Default)]
pub struct Transitions {
    last: HashMap<(&'static str, String), Entry>,
}

impl Transitions {
    /// Restores a target's last logged state.
    pub fn restore(&mut self, kind: TargetKind, target: &str, health: Health) {
        self.last.insert(
            key(kind, target),
            Entry {
                logged: Some(health),
                ..Entry::default()
            },
        );
    }

    /// Records the current state of a target and returns the transition, if this is one.
    pub fn observe(
        &mut self,
        kind: TargetKind,
        target: &str,
        now: &Assessment,
    ) -> Option<Transition> {
        let entry = self.last.entry(key(kind, target)).or_default();
        if now.health == Health::Unknown && !entry.live {
            return None;
        }
        entry.live = true;
        if entry.logged == Some(now.health) {
            entry.pending = None;
            return None;
        }
        if entry.logged.is_none() && now.health == Health::Healthy {
            entry.logged = Some(Health::Healthy);
            entry.pending = None;
            return None;
        }
        let seen = match entry.pending {
            Some((h, n)) if h == now.health => n + 1,
            _ => 1,
        };
        if seen < CONFIRMATIONS {
            entry.pending = Some((now.health, seen));
            return None;
        }
        let from = entry.logged.replace(now.health);
        entry.pending = None;
        Some(Transition {
            kind,
            target: target.to_owned(),
            from,
            to: now.health,
            message: now.message.clone(),
        })
    }
}

fn key(kind: TargetKind, target: &str) -> (&'static str, String) {
    let kind = match kind {
        TargetKind::System => "system",
        TargetKind::Service => "service",
        TargetKind::Timer => "timer",
        TargetKind::Check => "check",
    };
    (kind, target.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use Health::*;

    fn a(h: Health) -> Assessment {
        Assessment::new(h, format!("is {h}"))
    }

    /// Feeds `states` to a fresh tracker (optionally restored) and returns the `(from, to)` of
    /// every transition.
    fn run(restored: Option<Health>, states: &[Health]) -> Vec<(Option<Health>, Health)> {
        let mut t = Transitions::default();
        if let Some(h) = restored {
            t.restore(TargetKind::Service, "x.service", h);
        }
        states
            .iter()
            .filter_map(|&h| t.observe(TargetKind::Service, "x.service", &a(h)))
            .map(|tr| (tr.from, tr.to))
            .collect()
    }

    use Health::{Critical as C, Healthy as H, Unknown as U, Warning as W};

    #[test]
    fn transition_rows() {
        type Row = (
            &'static str,
            Option<Health>,
            &'static [Health],
            &'static [(Option<Health>, Health)],
        );
        let rows: &[Row] = &[
            ("healthy first is silent", None, &[H, H], &[]),
            (
                "broken first is logged once confirmed",
                None,
                &[C, C, C],
                &[(None, C)],
            ),
            ("a single broken observation is not logged", None, &[C], &[]),
            ("warning first", None, &[W, W], &[(None, W)]),
            (
                "unknown first is not a baseline",
                None,
                &[U, H, C, C],
                &[(Some(H), C)],
            ),
            ("unknown then broken", None, &[U, C, C], &[(None, C)]),
            (
                "confirmed changes are logged",
                None,
                &[H, C, C, C, H, H, U, U, H, H],
                &[(Some(H), C), (Some(C), H), (Some(H), U), (Some(U), H)],
            ),
            // Flapping at a threshold: never two in a row, so nothing is logged.
            ("flapping is held down", None, &[H, W, H, W, H, W, H], &[]),
            (
                "a blip back resets the count",
                None,
                &[H, W, H, W, W],
                &[(Some(H), W)],
            ),
            (
                "a different new state restarts the count",
                None,
                &[H, W, C, C],
                &[(Some(H), C)],
            ),
            ("restored, unchanged", Some(C), &[C], &[]),
            ("restored, recovered", Some(C), &[H, H], &[(Some(C), H)]),
            (
                "restored, unknown at startup is ignored",
                Some(C),
                &[U, C, U, U],
                &[(Some(C), U)],
            ),
            ("restored healthy", Some(H), &[H, W, W], &[(Some(H), W)]),
        ];
        for (case, restored, states, expected) in rows {
            assert_eq!(run(*restored, states), *expected, "{case}");
        }
    }

    #[test]
    fn transition_carries_the_confirming_message() {
        let mut t = Transitions::default();
        let obs = |t: &mut Transitions, a: Assessment| t.observe(TargetKind::Timer, "a.timer", &a);
        assert_eq!(obs(&mut t, Assessment::healthy()), None);
        assert_eq!(obs(&mut t, Assessment::new(Critical, "boom 1")), None);
        let tr = obs(&mut t, Assessment::new(Critical, "boom 2")).expect("confirmed");
        assert_eq!(
            tr,
            Transition {
                kind: TargetKind::Timer,
                target: "a.timer".into(),
                from: Some(Healthy),
                to: Critical,
                message: Some("boom 2".into()),
            }
        );
        assert_eq!(obs(&mut t, Assessment::healthy()), None);
        let back = obs(&mut t, Assessment::healthy()).expect("confirmed");
        assert_eq!((back.from, back.message), (Some(Critical), None));
    }

    #[test]
    fn targets_are_kept_apart_by_kind_and_name() {
        let mut t = Transitions::default();
        let mut twice = |kind, target: &str| {
            t.observe(kind, target, &a(Warning));
            t.observe(kind, target, &a(Warning)).is_some()
        };
        assert!(twice(TargetKind::System, "cpu"));
        assert!(twice(TargetKind::System, "load"));
        assert!(twice(TargetKind::Service, "cpu"));
        assert!(!twice(TargetKind::System, "cpu"));
    }
}
