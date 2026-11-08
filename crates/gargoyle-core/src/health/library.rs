//! Health of the music library from its Liner report (§8, "Library" row; ADR 0007).
//!
//! Never critical: metadata problems are not outages.

use super::{Assessment, Health, is_stale, short_duration};
use crate::api::SeverityCounts;
use crate::config::{LibraryConfig, WarnOn};
use crate::liner::ReadFailure;

/// What health needs from a valid report. Counts are Liner's `summary`, not counted from arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportFacts {
    /// Unix seconds.
    pub generated_at: i64,
    pub issue_counts: SeverityCounts,
    pub unreadable_files: u64,
    pub albums: u64,
}

/// The outcome of the last read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryStatus<'a> {
    NotRead,
    Failed(&'a ReadFailure),
    Read(ReportFacts),
}

/// Evaluates the library at `now`. `checked_at` is when the last poll completed.
///
/// - not read yet, no poll within 3× the interval, or a failed read: unknown;
/// - otherwise warning for any of: older than `max_age` (exactly `max_age` is fine; a future
///   `generated_at` is age 0), an issue at or above `warn_on`, unreadable files (unless
///   `warn_on = never`); the reasons are joined in that order;
/// - otherwise healthy.
pub fn evaluate_library(
    cfg: &LibraryConfig,
    status: LibraryStatus<'_>,
    checked_at: Option<i64>,
    now: i64,
) -> Assessment {
    if status == LibraryStatus::NotRead {
        return Assessment::unknown("report not read yet");
    }
    if is_stale(checked_at, now, cfg.interval) {
        let since = checked_at.map_or(0, |ts| now.saturating_sub(ts));
        return Assessment::unknown(format!("no result for {since}s"));
    }
    let facts = match status {
        LibraryStatus::NotRead => return Assessment::unknown("report not read yet"),
        LibraryStatus::Failed(failure) => return Assessment::unknown(failure.to_string()),
        LibraryStatus::Read(facts) => facts,
    };

    let mut reasons = Vec::new();
    let age = now.saturating_sub(facts.generated_at).max(0);
    let max_age = i64::try_from(cfg.max_age.as_secs()).unwrap_or(i64::MAX);
    if age > max_age {
        reasons.push(format!(
            "report is {} old (max {})",
            age_text(age),
            short_duration(cfg.max_age)
        ));
    }
    let c = facts.issue_counts;
    let issues_warn = match cfg.warn_on {
        WarnOn::Error => c.error > 0,
        WarnOn::Warning => c.error > 0 || c.warning > 0,
        WarnOn::Info => c.error > 0 || c.warning > 0 || c.info > 0,
        WarnOn::Never => false,
    };
    if issues_warn {
        reasons.push(issues_text(c, facts.albums));
    }
    if facts.unreadable_files > 0 && cfg.warn_on != WarnOn::Never {
        reasons.push(plural(
            facts.unreadable_files,
            "unreadable file",
            "unreadable files",
        ));
    }
    if reasons.is_empty() {
        Assessment::healthy()
    } else {
        Assessment::new(Health::Warning, reasons.join("; "))
    }
}

/// Whole minutes below 1h, whole hours below 48h, else whole days.
fn age_text(age_s: i64) -> String {
    if age_s < 3600 {
        format!("{}m", age_s / 60)
    } else if age_s < 48 * 3600 {
        format!("{}h", age_s / 3600)
    } else {
        format!("{}d", age_s / 86_400)
    }
}

/// `4 errors, 7 warnings in 95 albums`: every non-zero count, worst first.
fn issues_text(c: SeverityCounts, albums: u64) -> String {
    let parts: Vec<String> = [
        (c.error, "error", "errors"),
        (c.warning, "warning", "warnings"),
        (c.info, "info", "info"),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, one, many)| plural(n, one, many))
    .collect();
    format!(
        "{} in {}",
        parts.join(", "),
        plural(albums, "album", "albums")
    )
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::liner::ReportError;

    const NOW: i64 = 1_794_200_000;
    const H: i64 = 3600;

    fn cfg(warn_on: WarnOn) -> LibraryConfig {
        LibraryConfig {
            report: PathBuf::from("r.json"),
            name: "Music library".into(),
            max_age: Duration::from_secs(26 * 3600),
            warn_on,
            interval: Duration::from_secs(60),
        }
    }

    fn counts(error: u64, warning: u64, info: u64) -> SeverityCounts {
        SeverityCounts {
            error,
            warning,
            info,
        }
    }

    fn facts(age: i64, c: SeverityCounts, unreadable: u64, albums: u64) -> ReportFacts {
        ReportFacts {
            generated_at: NOW - age,
            issue_counts: c,
            unreadable_files: unreadable,
            albums,
        }
    }

    fn eval(warn_on: WarnOn, f: ReportFacts) -> (Health, Option<String>) {
        let a = evaluate_library(&cfg(warn_on), LibraryStatus::Read(f), Some(NOW - 5), NOW);
        (a.health, a.message)
    }

    fn w(msg: &str) -> (Health, Option<String>) {
        (Health::Warning, Some(msg.to_owned()))
    }

    const OK: (Health, Option<String>) = (Health::Healthy, None);

    #[test]
    fn unknown_causes() {
        let c = cfg(WarnOn::Error);
        let failures = [
            ReadFailure::NotFound,
            ReadFailure::Io("permission denied".into()),
            ReadFailure::NotRegularFile,
            ReadFailure::TooLarge { bytes: 40 << 20 },
            ReadFailure::Report(ReportError::Invalid {
                line: 12,
                column: 5,
            }),
            ReadFailure::Report(ReportError::InvalidField("generated_at")),
            ReadFailure::Report(ReportError::Unsupported("2.0.0".into())),
        ];
        for f in &failures {
            let a = evaluate_library(&c, LibraryStatus::Failed(f), Some(NOW), NOW);
            assert_eq!(a.health, Health::Unknown, "{f:?}");
            assert_eq!(a.message, Some(f.to_string()));
        }
        let rows = [
            (LibraryStatus::NotRead, None, NOW, "report not read yet"),
            // Not read yet wins even when no poll ever completed.
            (
                LibraryStatus::NotRead,
                None,
                NOW + 3600,
                "report not read yet",
            ),
            (
                LibraryStatus::Read(facts(0, counts(0, 0, 0), 0, 1)),
                Some(NOW - 181),
                NOW,
                "no result for 181s",
            ),
            (
                LibraryStatus::Failed(&failures[0]),
                Some(NOW - 200),
                NOW,
                "no result for 200s",
            ),
        ];
        for (status, checked, now, msg) in rows {
            let a = evaluate_library(&c, status, checked, now);
            assert_eq!(
                (a.health, a.message.as_deref()),
                (Health::Unknown, Some(msg))
            );
        }
        // Exactly 3× the interval is not stale yet.
        let a = evaluate_library(
            &c,
            LibraryStatus::Read(facts(0, counts(0, 0, 0), 0, 1)),
            Some(NOW - 180),
            NOW,
        );
        assert_eq!(a.health, Health::Healthy);
    }

    #[test]
    fn age_rows() {
        let none = counts(0, 0, 0);
        let rows = [
            (0, OK),
            (26 * H, OK),
            (26 * H + 1, w("report is 26h old (max 26h)")),
            (27 * H + 59 * 60, w("report is 27h old (max 26h)")),
            (48 * H - 1, w("report is 47h old (max 26h)")),
            (48 * H, w("report is 2d old (max 26h)")),
            (3 * 86_400 + 5 * H, w("report is 3d old (max 26h)")),
            // A future report counts as age 0.
            (-3600, OK),
        ];
        for (age, expected) in rows {
            assert_eq!(
                eval(WarnOn::Error, facts(age, none, 0, 3)),
                expected,
                "age {age}"
            );
        }
        // Minutes, with a short max_age.
        let mut c = cfg(WarnOn::Error);
        c.max_age = Duration::from_secs(60);
        for (age, msg) in [
            (61, "report is 1m old (max 1m)"),
            (45 * 60 + 30, "report is 45m old (max 1m)"),
            (3599, "report is 59m old (max 1m)"),
            (3600, "report is 1h old (max 1m)"),
        ] {
            let a = evaluate_library(
                &c,
                LibraryStatus::Read(facts(age, none, 0, 1)),
                Some(NOW),
                NOW,
            );
            assert_eq!(a.message.as_deref(), Some(msg), "age {age}");
        }
    }

    #[test]
    fn warn_on_by_severity() {
        use WarnOn::*;
        // (warn_on, error, warning, info) → warns
        let rows = [
            (Error, 1, 0, 0, true),
            (Error, 0, 1, 0, false),
            (Error, 0, 0, 1, false),
            (Warning, 1, 0, 0, true),
            (Warning, 0, 1, 0, true),
            (Warning, 0, 0, 1, false),
            (Info, 1, 0, 0, true),
            (Info, 0, 1, 0, true),
            (Info, 0, 0, 1, true),
            (Never, 1, 0, 0, false),
            (Never, 0, 1, 0, false),
            (Never, 0, 0, 1, false),
        ];
        for warn_on in [Error, Warning, Info, Never] {
            assert_eq!(
                eval(warn_on, facts(0, counts(0, 0, 0), 0, 5)),
                OK,
                "{warn_on:?} clean"
            );
        }
        for (warn_on, e, wa, i, warns) in rows {
            let got = eval(warn_on, facts(0, counts(e, wa, i), 0, 5));
            let expected = if warns {
                let n = [(e, "error"), (wa, "warning"), (i, "info")]
                    .into_iter()
                    .find(|(n, _)| *n > 0)
                    .map(|(n, s)| format!("{n} {s}"))
                    .unwrap_or_default();
                w(&format!("{n} in 5 albums"))
            } else {
                OK
            };
            assert_eq!(got, expected, "{warn_on:?} {e}/{wa}/{i}");
        }
    }

    #[test]
    fn unreadable_files() {
        let none = counts(0, 0, 0);
        assert_eq!(
            eval(WarnOn::Error, facts(0, none, 1, 3)),
            w("1 unreadable file")
        );
        assert_eq!(
            eval(WarnOn::Info, facts(0, none, 2, 3)),
            w("2 unreadable files")
        );
        assert_eq!(eval(WarnOn::Never, facts(0, none, 2, 3)), OK);
        // `never` still warns about age.
        assert_eq!(
            eval(WarnOn::Never, facts(30 * H, counts(4, 0, 0), 2, 3)),
            w("report is 30h old (max 26h)")
        );
    }

    #[test]
    fn messages_combine_in_order_with_plurals() {
        let rows = [
            (
                facts(3 * 86_400, counts(4, 7, 0), 2, 95),
                "report is 3d old (max 26h); 4 errors, 7 warnings in 95 albums; 2 unreadable files",
            ),
            (
                facts(0, counts(2, 4, 1), 1, 95),
                "2 errors, 4 warnings, 1 info in 95 albums; 1 unreadable file",
            ),
            (
                facts(0, counts(1, 0, 2), 0, 1),
                "1 error, 2 info in 1 album",
            ),
            (
                facts(0, counts(1, 1, 0), 0, 0),
                "1 error, 1 warning in 0 albums",
            ),
            (
                facts(27 * H, counts(0, 0, 0), 1, 3),
                "report is 27h old (max 26h); 1 unreadable file",
            ),
        ];
        for (f, msg) in rows {
            assert_eq!(eval(WarnOn::Error, f), w(msg));
        }
    }

    /// Never critical, over a grid of every input dimension.
    #[test]
    fn never_critical() {
        let failure = ReadFailure::NotFound;
        for warn_on in [WarnOn::Error, WarnOn::Warning, WarnOn::Info, WarnOn::Never] {
            let c = cfg(warn_on);
            for checked in [None, Some(NOW), Some(NOW - 10_000)] {
                assert_ne!(
                    evaluate_library(&c, LibraryStatus::Failed(&failure), checked, NOW).health,
                    Health::Critical
                );
                assert_ne!(
                    evaluate_library(&c, LibraryStatus::NotRead, checked, NOW).health,
                    Health::Critical
                );
                for age in [-1, 0, 26 * H, 26 * H + 1, 1_000_000] {
                    for n in [0, 1, u64::MAX] {
                        for unreadable in [0, 1, 5] {
                            let f = facts(age, counts(n, n / 2, n / 3), unreadable, n);
                            let a = evaluate_library(&c, LibraryStatus::Read(f), checked, NOW);
                            assert_ne!(a.health, Health::Critical, "{f:?}");
                            assert_eq!(a.health == Health::Healthy, a.message.is_none());
                        }
                    }
                }
            }
        }
    }
}
