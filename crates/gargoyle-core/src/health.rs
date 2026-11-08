//! The four-state health model (ARCHITECTURE §8).

use serde::{Deserialize, Serialize};

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

    #[test]
    fn serializes_lowercase() {
        for h in [Healthy, Unknown, Warning, Critical] {
            let json = serde_json::to_string(&h).unwrap();
            assert_eq!(json, format!("\"{h}\""));
            assert_eq!(serde_json::from_str::<Health>(&json).unwrap(), h);
        }
    }
}
