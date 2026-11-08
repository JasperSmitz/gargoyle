//! The API view of a report: grouping, ordering and caps (ARCHITECTURE §6, `LibraryReport`).

use std::cmp::Reverse;
use std::collections::HashMap;

use super::{Album, Issue, LinerReport};
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

/// Builds the view the API serves. The worst response stays at a few MB, whatever the report
/// size.
pub fn build_view(report: &LinerReport) -> LibraryReport {
    // Stable, so the report's order is kept within a severity.
    let mut ordered: Vec<&Issue> = report.issues.iter().collect();
    ordered.sort_by_key(|i| Reverse(i.severity));
    ordered.truncate(MAX_ISSUES);

    let known: HashMap<&str, &Album> = report.albums.iter().map(|a| (a.id.as_str(), a)).collect();
    let mut albums: Vec<(Severity, LibraryAlbum)> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut library_issues = Vec::new();
    for issue in ordered {
        let item = view_issue(issue);
        let Some(id) = issue.album_id.as_deref() else {
            library_issues.push(item);
            continue;
        };
        let slot = *index.entry(id).or_insert_with(|| {
            // Issues arrive worst first, so the first one of an album is its worst.
            albums.push((issue.severity, empty_album(id, known.get(id).copied())));
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
            code: code.clone(),
            count,
        })
        .collect();
    // The map is sorted by code already; a stable sort by count keeps that as the tie-break.
    by_code.sort_by_key(|c| Reverse(c.count));

    LibraryReport {
        schema_version: report.schema_version.clone(),
        tool: LinerTool {
            name: report.tool.name.clone(),
            version: report.tool.version.clone(),
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
                path: e.path.clone(),
                kind: e.kind.clone(),
                message: e.message.clone(),
            })
            .collect(),
        issues_total: count(report.issues.len()),
        issues_truncated: report.issues.len() > MAX_ISSUES,
        errors_total: count(report.errors.len()),
        errors_truncated: report.errors.len() > MAX_ERRORS,
    }
}

fn view_issue(issue: &Issue) -> LibraryIssue {
    LibraryIssue {
        code: issue.code.clone(),
        severity: issue.severity,
        scope: issue.scope,
        message: issue.message.clone(),
        paths: issue.paths.iter().take(MAX_PATHS).cloned().collect(),
        paths_total: count(issue.paths.len()),
    }
}

/// An album entry without issues yet. An `album_id` that is not among the report's albums still
/// gets one, with no title, artist or counts.
fn empty_album(id: &str, album: Option<&Album>) -> LibraryAlbum {
    LibraryAlbum {
        id: id.to_owned(),
        title: album.and_then(|a| a.title.clone()),
        album_artist: album.and_then(|a| a.album_artist.clone()),
        disc_count: album.map_or(0, |a| a.disc_count),
        track_count: album.map_or(0, |a| a.track_count),
        issues: Vec::new(),
    }
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
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
        issues[0].paths = (0..MAX_PATHS + 7).map(|i| format!("p{i}")).collect();
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
}
