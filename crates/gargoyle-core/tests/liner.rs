//! The Liner report parser against Liner's own golden report and synthetic variants (ADR 0007).
//! Fixtures and their origin: `tests/fixtures/liner/README.md`.

#![allow(clippy::unwrap_used)] // test code

use std::fs;
use std::path::PathBuf;

use gargoyle_core::api::{IssueScope, Severity, SeverityCounts};
use gargoyle_core::liner::{
    MAX_ERRORS, MAX_ISSUES, MAX_PATHS, ReportError, build_view, parse_report, parse_report_bytes,
};
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/liner")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn error_of(name: &str) -> ReportError {
    parse_report(&fixture(name)).expect_err(name).error
}

#[test]
fn golden_report_parses() {
    let r = parse_report(&fixture("golden-report.json")).unwrap();
    assert_eq!(r.schema_version, "1.0.0");
    assert_eq!(
        (r.tool.name.as_str(), r.tool.version.as_str()),
        ("liner", "0.0.0-test")
    );
    // 2026-11-08T10:00:00Z
    assert_eq!(r.generated_at, 1_794_132_000);
    assert_eq!(r.options.min_severity, Severity::Info);
    let s = &r.summary;
    assert_eq!(
        (
            s.files_scanned,
            s.audio_files,
            s.unreadable_files,
            s.albums,
            s.issues_hidden
        ),
        (11, 8, 2, 3, 0)
    );
    assert_eq!(
        s.issues_by_severity,
        SeverityCounts {
            error: 1,
            warning: 4,
            info: 2
        }
    );
    assert_eq!(s.issues_by_code.len(), 7);
    assert_eq!(r.albums.len(), 3);
    assert_eq!(r.issues.len(), 7);
    assert_eq!(r.errors.len(), 2);
    assert!(
        r.issues
            .iter()
            .any(|i| i.scope == IssueScope::Library && i.album_id.is_none())
    );

    let v = build_view(&r);
    assert_eq!(v.issues_total, 7);
    assert!(!v.issues_truncated && !v.errors_truncated);
    let kept = v.library_issues.len() + v.albums.iter().map(|a| a.issues.len()).sum::<usize>();
    assert_eq!(kept, 7);
    assert_eq!(v.albums[0].issues[0].severity, Severity::Error);
    assert_eq!(v.errors.len(), 2);
}

/// Liner's `root` (the absolute library location) never reaches the view.
#[test]
fn root_is_not_in_the_view() {
    let mut report: Value = serde_json::from_str(&fixture("golden-report.json")).unwrap();
    report["root"] = json!("/srv/secret-root-marker");
    let r = parse_report(&report.to_string()).unwrap();
    let view = serde_json::to_string(&build_view(&r)).unwrap();
    assert!(!view.contains("secret-root-marker"));
}

#[test]
fn later_minor_with_unknown_fields_parses() {
    let r = parse_report(&fixture("schema-1.7.0-unknown-fields.json")).unwrap();
    assert_eq!(r.schema_version, "1.7.0");
    assert!(r.issues.iter().any(|i| i.code == "future-rule"));
    assert!(r.errors.iter().any(|e| e.kind == "future-kind"));
    let v = build_view(&r);
    assert!(
        v.summary
            .by_code
            .iter()
            .any(|c| c.code == "future-rule" && c.count == 1)
    );
}

#[test]
fn pre_release_of_major_1_parses() {
    let mut report: Value = serde_json::from_str(&fixture("golden-report.json")).unwrap();
    report["schema_version"] = json!("1.1.0-rc.1");
    parse_report(&report.to_string()).unwrap();
}

#[test]
fn major_2_is_unsupported() {
    assert_eq!(
        error_of("schema-2.0.0.json"),
        ReportError::Unsupported("2.0.0".into())
    );
    assert_eq!(
        error_of("schema-2.0.0.json").to_string(),
        "unsupported Liner schema 2.0.0"
    );
}

#[test]
fn invalid_variants() {
    assert_eq!(
        error_of("missing-schema-version.json").to_string(),
        "invalid report: schema_version"
    );
    assert_eq!(
        error_of("bad-generated-at.json").to_string(),
        "invalid report: generated_at"
    );
    let e = parse_report(&fixture("unknown-severity.json")).expect_err("fatal is not a severity");
    assert!(
        matches!(e.error, ReportError::Invalid { line, .. } if line > 1),
        "{e:?}"
    );
    // The API message has only the position; serde's detail (for the log) names the value.
    assert!(!e.error.to_string().contains("fatal"));
    assert!(e.detail.contains("fatal"), "{}", e.detail);
}

#[test]
fn truncated_and_non_utf8_reports_are_invalid() {
    let text = fixture("golden-report.json");
    let e = parse_report(&text[..text.len() / 2]).expect_err("truncated");
    assert!(matches!(e.error, ReportError::Invalid { .. }), "{e:?}");
    let mut bytes = text.into_bytes();
    bytes[40] = 0xff;
    let e = parse_report_bytes(&bytes).expect_err("not UTF-8");
    assert!(matches!(e.error, ReportError::Invalid { .. }), "{e:?}");
}

/// More than 500 issues, more than 50 paths on one issue and more than 100 errors, built from
/// the golden report.
#[test]
fn caps_on_a_large_report() {
    let mut report: Value = serde_json::from_str(&fixture("golden-report.json")).unwrap();
    let template = report["issues"][1].clone();
    let issues = report["issues"].as_array_mut().unwrap();
    for i in 0..MAX_ISSUES {
        let mut issue = template.clone();
        issue["code"] = json!(format!("bulk-{i}"));
        issue["severity"] = json!("info");
        issues.push(issue);
    }
    issues[0]["paths"] = json!(
        (0..MAX_PATHS + 25)
            .map(|i| format!("Artist/Album/{i:03}.flac"))
            .collect::<Vec<_>>()
    );
    let error = report["errors"][0].clone();
    let errors = report["errors"].as_array_mut().unwrap();
    errors.extend(std::iter::repeat_n(error, MAX_ERRORS));

    let v = build_view(&parse_report(&report.to_string()).unwrap());
    assert_eq!(v.issues_total, 7 + MAX_ISSUES as u64);
    assert!(v.issues_truncated);
    let kept: Vec<_> = v
        .albums
        .iter()
        .flat_map(|a| &a.issues)
        .chain(&v.library_issues)
        .collect();
    assert_eq!(kept.len(), MAX_ISSUES);
    // Every error and warning of the original report survives the cut.
    assert_eq!(
        kept.iter().filter(|i| i.severity != Severity::Info).count(),
        5
    );
    let wide = kept
        .iter()
        .find(|i| i.paths_total > MAX_PATHS as u64)
        .unwrap();
    assert_eq!((wide.paths.len(), wide.paths_total), (MAX_PATHS, 75));
    assert_eq!(v.errors.len(), MAX_ERRORS);
    assert_eq!(v.errors_total, 2 + MAX_ERRORS as u64);
    assert!(v.errors_truncated);
    // `summary` still describes the whole report.
    assert_eq!(v.summary.unreadable_files, 2);
}
