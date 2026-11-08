//! The API view of a report: grouping, ordering and caps (ARCHITECTURE §6, `LibraryReport`).
//!
//! Counts are capped while parsing (`de`); here every report string is also capped in bytes, with
//! control characters replaced, so the response stays at about 10 MB at worst whatever the report
//! holds.

use std::cmp::Reverse;
use std::collections::HashMap;

use super::{Album, Issue, LinerReport, shown_version};
use crate::api::{
    CodeCount, LibraryAlbum, LibraryFileError, LibraryIssue, LibraryReport, LinerTool,
    ReportSummary, Severity,
};

/// Issues kept, in severity order.
pub const MAX_ISSUES: usize = 500;
/// Paths kept per issue.
pub const MAX_PATHS: usize = 50;
/// Unreadable files kept.
pub const MAX_ERRORS: usize = 100;
/// The longest text kept (message, code, kind, title, artist, tool), in bytes.
pub const MAX_TEXT_BYTES: usize = 1024;
/// The longest path or album id kept, in bytes (Linux `PATH_MAX`).
pub const MAX_PATH_BYTES: usize = 4096;
/// Bytes of issue paths kept over the whole view. Paths go to the worst issues first; once the
/// budget is spent, later issues list fewer paths (`pathsTotal` still counts them all).
pub const PATH_BUDGET_BYTES: usize = 2 * 1024 * 1024;

const ELLIPSIS: &str = "…";

/// Builds the view the API serves. The worst response stays at about 10 MB, whatever the report
/// holds (`"` and `\\` escape to 2 bytes; see `clip`).
pub fn build_view(report: &LinerReport) -> LibraryReport {
    // Stable, so the report's order is kept within a severity. Parsing already did this; a
    // `LinerReport` built by hand may not have.
    let mut ordered: Vec<&Issue> = report.issues.iter().collect();
    ordered.sort_by_key(|i| Reverse(i.severity));
    ordered.truncate(MAX_ISSUES);

    let known: HashMap<&str, &Album> = report.albums.iter().map(|a| (a.id.as_str(), a)).collect();
    let mut albums: Vec<(Severity, LibraryAlbum)> = Vec::new();
    // By the id as shown: ids that only differ beyond the cap share one entry, so `id` stays
    // unique in the response (the UI keys albums by it).
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut library_issues = Vec::new();
    let mut path_budget = PATH_BUDGET_BYTES;
    for issue in ordered {
        let item = view_issue(issue, &mut path_budget);
        let Some(id) = issue.album_id.as_deref() else {
            library_issues.push(item);
            continue;
        };
        let shown = clip(id, MAX_PATH_BYTES);
        let slot = *index.entry(shown).or_insert_with_key(|shown| {
            // Issues arrive worst first, so the first one of an album is its worst.
            let album = empty_album(shown.clone(), known.get(id).copied());
            albums.push((issue.severity, album));
            albums.len() - 1
        });
        albums[slot].1.issues.push(item);
    }
    albums.sort_by(|(sa, a), (sb, b)| sb.cmp(sa).then_with(|| a.id.cmp(&b.id)));

    let s = &report.summary;
    let mut by_code: Vec<CodeCount> = s
        .issues_by_code
        .iter()
        .map(|(code, &count)| CodeCount {
            code: clip(code, MAX_TEXT_BYTES),
            count,
        })
        .collect();
    // The map is sorted by code already; a stable sort by count keeps that as the tie-break.
    by_code.sort_by_key(|c| Reverse(c.count));

    LibraryReport {
        // Only 1.x gets here; anything but a short semver is shown as `1.x`.
        schema_version: shown_version(&report.schema_version, 1),
        tool: LinerTool {
            name: clip(&report.tool.name, MAX_TEXT_BYTES),
            version: clip(&report.tool.version, MAX_TEXT_BYTES),
        },
        min_severity: report.options.min_severity,
        summary: ReportSummary {
            files_scanned: s.files_scanned,
            audio_files: s.audio_files,
            unreadable_files: s.unreadable_files,
            albums: s.albums,
            issue_counts: s.issues_by_severity,
            issues_hidden: s.issues_hidden,
            by_code,
        },
        albums: albums.into_iter().map(|(_, a)| a).collect(),
        library_issues,
        errors: report
            .errors
            .iter()
            .take(MAX_ERRORS)
            .map(|e| LibraryFileError {
                path: clip(&e.path, MAX_PATH_BYTES),
                kind: clip(&e.kind, MAX_TEXT_BYTES),
                message: clip(&e.message, MAX_TEXT_BYTES),
            })
            .collect(),
        issues_total: report.issues_total,
        issues_truncated: report.issues_total > MAX_ISSUES as u64,
        errors_total: report.errors_total,
        errors_truncated: report.errors_total > MAX_ERRORS as u64,
    }
}

fn view_issue(issue: &Issue, path_budget: &mut usize) -> LibraryIssue {
    let mut paths = Vec::new();
    for path in issue.paths.iter().take(MAX_PATHS) {
        let path = clip(path, MAX_PATH_BYTES);
        let Some(left) = path_budget.checked_sub(path.len()) else {
            break;
        };
        *path_budget = left;
        paths.push(path);
    }
    LibraryIssue {
        code: clip(&issue.code, MAX_TEXT_BYTES),
        severity: issue.severity,
        scope: issue.scope,
        message: clip(&issue.message, MAX_TEXT_BYTES),
        paths,
        paths_total: issue.paths_total,
    }
}

/// An album entry without issues yet. An `album_id` that is not among the report's albums still
/// gets one, with no title, artist or counts.
fn empty_album(id: String, album: Option<&Album>) -> LibraryAlbum {
    let text = |t: &Option<String>| t.as_deref().map(|t| clip(t, MAX_TEXT_BYTES));
    LibraryAlbum {
        id,
        title: album.and_then(|a| text(&a.title)),
        album_artist: album.and_then(|a| text(&a.album_artist)),
        disc_count: album.map_or(0, |a| a.disc_count),
        track_count: album.map_or(0, |a| a.track_count),
        issues: Vec::new(),
    }
}

/// `text` with control characters replaced, or if that is longer than `max` bytes its start (cut
/// on a char boundary) and `…`, `max` bytes at most.
///
/// C0 and C1 controls and U+2028/U+2029 become U+FFFD before measuring: serde_json writes a C0
/// control as `\u0001`, six bytes for one, so the caps would otherwise not bound the response.
/// Now the worst escape is `"` or `\\`, two bytes for one. Only the kept prefix is scanned.
fn clip(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
            char::REPLACEMENT_CHARACTER
        } else {
            c
        };
        if out.len() + c.len_utf8() > max {
            while out.len() + ELLIPSIS.len() > max && out.pop().is_some() {}
            out.push_str(ELLIPSIS);
            return out;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::api::{IssueScope, SeverityCounts};
    use crate::liner::{FileError, Options, Summary, Tool};

    fn issue(code: &str, severity: Severity, album: Option<&str>) -> Issue {
        Issue {
            code: code.into(),
            severity,
            scope: if album.is_some() {
                IssueScope::Album
            } else {
                IssueScope::Library
            },
            message: format!("{code} message"),
            paths: vec![format!("{code}.flac")],
            paths_total: 1,
            album_id: album.map(Into::into),
        }
    }

    fn album(id: &str) -> Album {
        Album {
            id: id.into(),
            title: Some(format!("{id} title")),
            album_artist: None,
            disc_count: 1,
            track_count: 3,
        }
    }

    fn report(issues: Vec<Issue>, albums: Vec<Album>, errors: usize) -> LinerReport {
        LinerReport {
            issues_total: issues.len() as u64,
            errors_total: errors as u64,
            schema_version: "1.0.0".into(),
            tool: Tool {
                name: "liner".into(),
                version: "0.1.0".into(),
            },
            generated_at: 0,
            options: Options {
                min_severity: Severity::Info,
            },
            summary: Summary {
                files_scanned: 10,
                audio_files: 8,
                unreadable_files: 0,
                albums: 2,
                issues_by_severity: SeverityCounts::default(),
                issues_by_code: BTreeMap::from([
                    ("b-code".to_owned(), 2),
                    ("a-code".to_owned(), 2),
                    ("c-code".to_owned(), 5),
                ]),
                issues_hidden: 0,
            },
            albums,
            issues,
            errors: (0..errors)
                .map(|i| FileError {
                    path: format!("bad{i}.flac"),
                    kind: "corrupt".into(),
                    message: "broken".into(),
                })
                .collect(),
        }
    }

    use Severity::{Error as E, Info as I, Warning as W};

    const MIB: usize = 1024 * 1024;

    #[test]
    fn groups_and_orders() {
        let r = report(
            vec![
                issue("i1", I, Some("B")),
                issue("w1", W, Some("A")),
                issue("lib-w", W, None),
                issue("e1", E, Some("B")),
                issue("w2", W, Some("B")),
                issue("orphan", W, Some("Gone")),
                issue("lib-i", I, None),
                issue("w3", W, Some("C")),
            ],
            vec![album("A"), album("B"), album("C")],
            0,
        );
        let v = build_view(&r);
        let shape: Vec<(&str, Vec<&str>)> = v
            .albums
            .iter()
            .map(|a| {
                (
                    a.id.as_str(),
                    a.issues.iter().map(|i| i.code.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("B", vec!["e1", "w2", "i1"]),
                ("A", vec!["w1"]),
                ("C", vec!["w3"]),
                ("Gone", vec!["orphan"]),
            ]
        );
        let lib: Vec<&str> = v.library_issues.iter().map(|i| i.code.as_str()).collect();
        assert_eq!(lib, ["lib-w", "lib-i"]);

        let b = &v.albums[0];
        assert_eq!(
            (b.title.as_deref(), b.disc_count, b.track_count),
            (Some("B title"), 1, 3)
        );
        let gone = &v.albums[3];
        assert_eq!(
            (
                gone.title.as_deref(),
                gone.album_artist.as_deref(),
                gone.disc_count,
                gone.track_count
            ),
            (None, None, 0, 0)
        );
        // Albums without issues are left out.
        assert_eq!(v.albums.len(), 4);
        assert_eq!((v.issues_total, v.issues_truncated), (8, false));
        assert_eq!((v.errors_total, v.errors_truncated), (0, false));

        let codes: Vec<(&str, u64)> = v
            .summary
            .by_code
            .iter()
            .map(|c| (c.code.as_str(), c.count))
            .collect();
        assert_eq!(codes, [("c-code", 5), ("a-code", 2), ("b-code", 2)]);
    }

    #[test]
    fn caps() {
        let mut issues: Vec<Issue> = (0..MAX_ISSUES + 20)
            .map(|i| issue(&format!("i{i}"), I, Some("A")))
            .collect();
        // An error at the end still makes it in: the cap applies after ordering.
        issues.push(issue("late-error", E, None));
        issues[0].paths = (0..MAX_PATHS).map(|i| format!("p{i}")).collect();
        issues[0].paths_total = 57;
        let v = build_view(&report(issues, vec![album("A")], MAX_ERRORS + 1));

        assert_eq!(v.library_issues[0].code, "late-error");
        let kept = v.library_issues.len() + v.albums.iter().map(|a| a.issues.len()).sum::<usize>();
        assert_eq!(kept, MAX_ISSUES);
        assert_eq!((v.issues_total, v.issues_truncated), (521, true));
        let first = &v.albums[0].issues[0];
        assert_eq!(first.paths.len(), MAX_PATHS);
        assert_eq!(first.paths_total, 57);
        assert_eq!(first.paths[49], "p49");
        assert_eq!(v.errors.len(), MAX_ERRORS);
        assert_eq!((v.errors_total, v.errors_truncated), (101, true));
        assert_eq!(v.errors[0].path, "bad0.flac");
    }

    #[test]
    fn clip_cuts_on_a_char_boundary() {
        assert_eq!(clip("short", 8), "short");
        assert_eq!(clip("exactly8", 8), "exactly8");
        assert_eq!(clip("123456789", 8), "12345…");
        // `é` is 2 bytes and `…` 3: a cut inside an `é` backs off to its start.
        assert_eq!(clip("abéééé", 6), "ab…");
        assert_eq!(clip("abéééé", 7), "abé…");
        assert_eq!(clip("abéééé", 8), "abé…");
        for max in 3..20 {
            assert!(clip(&"ü".repeat(20), max).len() <= max);
        }
    }

    #[test]
    fn clip_replaces_control_characters() {
        assert_eq!(clip("a\u{1}b\tc\nd", 64), "a\u{FFFD}b\u{FFFD}c\u{FFFD}d");
        assert_eq!(clip("\u{7f}\u{85}\u{9f}", 64), "\u{FFFD}".repeat(3));
        assert_eq!(clip("x\u{2028}y\u{2029}", 64), "x\u{FFFD}y\u{FFFD}");
        // Measured after the replacement: each U+FFFD is 3 bytes.
        assert_eq!(clip(&"\u{1}".repeat(4), 8), "\u{FFFD}…");
        assert_eq!(clip("é\u{2028}", 64), "é\u{FFFD}");
    }

    /// Re-review M7.5 R2: 500 issues whose code, message, album id and path are all control
    /// characters serialized to 30.7 MB, because serde_json writes each as `\u0001` (6 bytes).
    /// Replaced, every kept string serializes to at most 2× its byte cap (`"` and `\\`): per issue
    /// 2 × (1 KiB code + 1 KiB message + 4 KiB album id + 4 KiB path) = 20 KiB, so 500 issues stay
    /// under 10 MiB plus framing. Plain control characters stay at 1×, about 5 MiB.
    #[test]
    fn control_characters_do_not_inflate_the_response() {
        for (fill, bound) in [('\u{1}', 6 * MIB), ('"', 11 * MIB), ('\\', 11 * MIB)] {
            let text = |n: usize| fill.to_string().repeat(n);
            let issues = (0..MAX_ISSUES)
                .map(|i| {
                    let mut e = issue("c", E, Some(&format!("{i}{}", text(MAX_PATH_BYTES))));
                    e.code = text(MAX_TEXT_BYTES);
                    e.message = text(MAX_TEXT_BYTES);
                    e.paths = vec![text(MAX_PATH_BYTES)];
                    e
                })
                .collect();
            let v = build_view(&report(issues, vec![], 0));
            assert_eq!(v.albums.len(), MAX_ISSUES);
            assert!(v.albums.iter().all(|a| a.issues[0].paths.len() == 1));
            let len = serde_json::to_string(&v).unwrap().len();
            assert!(len < bound, "{fill:?}: {len} bytes");
        }
    }

    /// Every report string is capped, so one huge field cannot make every response huge.
    #[test]
    fn long_strings_are_clipped() {
        let huge = "x".repeat(MAX_PATH_BYTES * 4);
        let mut long = issue("c", E, Some(&huge));
        long.code = huge.clone();
        long.message = huge.clone();
        long.paths = vec![huge.clone(); 3];
        let mut r = report(vec![long], vec![album(&huge)], 1);
        r.schema_version = format!("1.{huge}");
        r.tool.name = huge.clone();
        r.tool.version = huge.clone();
        r.albums[0].title = Some(huge.clone());
        r.albums[0].album_artist = Some(huge.clone());
        r.errors[0].path = huge.clone();
        r.errors[0].kind = huge.clone();
        r.errors[0].message = huge.clone();
        r.summary.issues_by_code = BTreeMap::from([(huge.clone(), 1)]);

        let v = build_view(&r);
        assert_eq!(v.schema_version, "1.x");
        let text = [
            &v.tool.name,
            &v.tool.version,
            &v.albums[0].issues[0].code,
            &v.albums[0].issues[0].message,
            v.albums[0].title.as_ref().unwrap(),
            v.albums[0].album_artist.as_ref().unwrap(),
            &v.errors[0].kind,
            &v.errors[0].message,
            &v.summary.by_code[0].code,
        ];
        for t in text {
            assert_eq!(t.len(), MAX_TEXT_BYTES);
            assert!(t.ends_with(ELLIPSIS));
        }
        let paths = [
            &v.albums[0].id,
            &v.albums[0].issues[0].paths[0],
            &v.errors[0].path,
        ];
        for p in paths {
            assert_eq!(p.len(), MAX_PATH_BYTES);
            assert!(p.ends_with(ELLIPSIS));
        }
        // The album was still found by its full id.
        assert_eq!(v.albums[0].track_count, 3);
        assert!(serde_json::to_string(&v).unwrap().len() < 32 * 1024);
    }

    /// Album ids that only differ past the cap become one entry, so ids stay unique.
    #[test]
    fn clipped_album_ids_stay_unique() {
        let base = "a".repeat(MAX_PATH_BYTES);
        let (one, two) = (format!("{base}1"), format!("{base}2"));
        let r = report(
            vec![issue("e", E, Some(&one)), issue("w", W, Some(&two))],
            vec![],
            0,
        );
        let v = build_view(&r);
        assert_eq!(v.albums.len(), 1);
        assert_eq!(v.albums[0].issues.len(), 2);
    }

    #[test]
    fn paths_share_a_byte_budget() {
        let path = "p".repeat(MAX_PATH_BYTES);
        let per_issue = MAX_PATHS * MAX_PATH_BYTES;
        let issues: Vec<Issue> = (0..MAX_ISSUES)
            .map(|i| {
                let mut issue = issue(&format!("i{i}"), W, None);
                issue.paths = vec![path.clone(); MAX_PATHS];
                issue.paths_total = 60;
                issue
            })
            .collect();
        let v = build_view(&report(issues, vec![], 0));
        let kept: usize = v
            .library_issues
            .iter()
            .flat_map(|i| &i.paths)
            .map(String::len)
            .sum();
        assert_eq!(kept, PATH_BUDGET_BYTES);
        let full = PATH_BUDGET_BYTES / per_issue;
        assert_eq!(v.library_issues[0].paths.len(), MAX_PATHS);
        assert_eq!(v.library_issues[full + 1].paths.len(), 0);
        assert_eq!(v.library_issues[full + 1].paths_total, 60);
    }
}
