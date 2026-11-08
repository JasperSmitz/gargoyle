//! Liner's JSON report (schema 1.x): input DTOs, parsing and the API view (ADR 0007).
//!
//! Gargoyle only reads the report file Liner writes; it never runs Liner. The DTOs here cover the
//! fields Gargoyle uses and follow Liner's consumer rules: unknown fields are ignored, and issue
//! `code` and error `kind` are open sets (plain strings). `severity` and `scope` are closed
//! enums, as Liner's schema fixes them for a major version. Per-track data (`albums[].discs`),
//! `issues[].details` and the absolute `root` are never deserialized.
//!
//! Arrays that can be long (`issues`, `paths`, `errors`) are cut to what the view keeps while
//! parsing (`de`), so a report within [`MAX_REPORT_BYTES`] cannot take many times its size in
//! memory.
//!
//! Errors never carry report content or the file path, so their `Display` can go to the API
//! as is. The detail for the log (serde's own message, which may quote values) is separate.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

use crate::api::{IssueScope, Severity, SeverityCounts};

mod de;
mod time;
mod view;

pub use de::MAX_CODES;
use de::{Capped, ProbedVersion, WorstIssues};
pub use time::parse_rfc3339;
pub use view::{
    MAX_ERRORS, MAX_ISSUES, MAX_PATH_BYTES, MAX_PATHS, MAX_TEXT_BYTES, PATH_BUDGET_BYTES,
    build_view,
};

/// The largest report Gargoyle reads (ADR 0007): about 40 times a real 1148-file report.
pub const MAX_REPORT_BYTES: u64 = 32 * MIB;

const MIB: u64 = 1024 * 1024;

/// The parsed subset of a Liner report.
#[derive(Debug, Clone, PartialEq)]
pub struct LinerReport {
    pub schema_version: String,
    pub tool: Tool,
    /// `generated_at` as unix seconds.
    pub generated_at: i64,
    pub options: Options,
    pub summary: Summary,
    pub albums: Vec<Album>,
    /// The worst [`MAX_ISSUES`], in severity order (error first), report order within one.
    pub issues: Vec<Issue>,
    /// The number of issues in the report.
    pub issues_total: u64,
    /// The first [`MAX_ERRORS`].
    pub errors: Vec<FileError>,
    /// The number of errors in the report.
    pub errors_total: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Tool {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Options {
    pub min_severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Summary {
    pub files_scanned: u64,
    pub audio_files: u64,
    pub unreadable_files: u64,
    pub albums: u64,
    pub issues_by_severity: SeverityCounts,
    /// At most [`MAX_CODES`] codes; a report with more is invalid.
    #[serde(deserialize_with = "de::codes")]
    pub issues_by_code: BTreeMap<String, u64>,
    pub issues_hidden: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Album {
    pub id: String,
    pub title: Option<String>,
    pub album_artist: Option<String>,
    pub disc_count: u32,
    pub track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "RawIssue")]
pub struct Issue {
    pub code: String,
    pub severity: Severity,
    pub scope: IssueScope,
    pub message: String,
    /// The first [`MAX_PATHS`].
    pub paths: Vec<String>,
    /// The number of paths in the report.
    pub paths_total: u64,
    pub album_id: Option<String>,
}

#[derive(Deserialize)]
struct RawIssue {
    code: String,
    severity: Severity,
    scope: IssueScope,
    message: String,
    paths: Capped<String, MAX_PATHS>,
    album_id: Option<String>,
}

impl From<RawIssue> for Issue {
    fn from(raw: RawIssue) -> Self {
        Self {
            code: raw.code,
            severity: raw.severity,
            scope: raw.scope,
            message: raw.message,
            paths: raw.paths.kept,
            paths_total: raw.paths.total,
            album_id: raw.album_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FileError {
    pub path: String,
    pub kind: String,
    pub message: String,
}

/// The report as it is on disk; `generated_at` is checked separately so a bad timestamp gets its
/// own message.
#[derive(Deserialize)]
struct RawReport {
    schema_version: String,
    tool: Tool,
    generated_at: String,
    options: Options,
    summary: Summary,
    albums: Vec<Album>,
    issues: WorstIssues,
    errors: Capped<FileError, MAX_ERRORS>,
}

/// Only the version, read before the typed parse.
#[derive(Deserialize)]
struct VersionProbe {
    schema_version: Option<ProbedVersion>,
}

/// Why a report could not be used. `Display` is the API message: it never contains report
/// content (except a well-formed schema version) or the file path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReportError {
    /// Not JSON, or not the v1 shape. Only serde's position is kept: its message can quote values.
    #[error("invalid report (line {line}, column {column})")]
    Invalid { line: usize, column: usize },
    /// A field that is present but unusable.
    #[error("invalid report: {0}")]
    InvalidField(&'static str),
    /// A numeric schema major other than 1.
    #[error("unsupported Liner schema {0}")]
    Unsupported(String),
}

impl ReportError {
    fn from_serde(e: &serde_json::Error) -> Self {
        ReportError::Invalid {
            line: e.line(),
            column: e.column(),
        }
    }
}

/// A report that failed to parse: the API error, and serde's full message for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFailure {
    pub error: ReportError,
    /// May quote report content; log it, never put it in a response.
    pub detail: String,
}

impl From<ReportError> for ParseFailure {
    fn from(error: ReportError) -> Self {
        let detail = error.to_string();
        Self { error, detail }
    }
}

/// Parses a Liner report. The schema major is checked first, so a `2.x` report is reported as
/// unsupported even if it would not match the v1 shape.
pub fn parse_report(text: &str) -> Result<LinerReport, ParseFailure> {
    let serde_failure = |e: serde_json::Error| ParseFailure {
        error: ReportError::from_serde(&e),
        detail: e.to_string(),
    };
    let probe: VersionProbe = serde_json::from_str(text).map_err(serde_failure)?;
    let version = match probe.schema_version {
        Some(ProbedVersion::Text(v)) => v,
        _ => return Err(ReportError::InvalidField("schema_version").into()),
    };
    match schema_major(&version) {
        None => return Err(ReportError::InvalidField("schema_version").into()),
        Some(1) => {}
        Some(major) => return Err(ReportError::Unsupported(shown_version(&version, major)).into()),
    }
    let raw: RawReport = serde_json::from_str(text).map_err(serde_failure)?;
    let generated_at =
        parse_rfc3339(&raw.generated_at).ok_or(ReportError::InvalidField("generated_at"))?;
    Ok(LinerReport {
        schema_version: raw.schema_version,
        tool: raw.tool,
        generated_at,
        options: raw.options,
        summary: raw.summary,
        albums: raw.albums,
        issues: raw.issues.kept,
        issues_total: raw.issues.total,
        errors: raw.errors.kept,
        errors_total: raw.errors.total,
    })
}

/// [`parse_report`] over raw file bytes: invalid UTF-8 is an invalid report at the position of
/// the first bad byte.
pub fn parse_report_bytes(bytes: &[u8]) -> Result<LinerReport, ParseFailure> {
    match std::str::from_utf8(bytes) {
        Ok(text) => parse_report(text),
        Err(e) => {
            let valid = &bytes[..e.valid_up_to()];
            let line = valid.iter().filter(|&&b| b == b'\n').count() + 1;
            let line_start = valid.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
            let column = valid.len() - line_start + 1;
            let error = ReportError::Invalid { line, column };
            Err(ParseFailure {
                detail: format!("{error}: invalid UTF-8"),
                error,
            })
        }
    }
}

/// The numeric major of `1.2.3`, `1.0.0-rc.1` or `2`; `None` when it is missing or not a number.
fn schema_major(version: &str) -> Option<u64> {
    let major = version.split('.').next().unwrap_or_default();
    if major.is_empty() || !major.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    major.parse().ok()
}

/// The version as shown in `unsupported Liner schema <version>`: as written when it looks like a
/// semver (short, `[0-9A-Za-z.+-]` only), else just the major, so the message cannot carry
/// arbitrary report text.
fn shown_version(version: &str, major: u64) -> String {
    let plain = version.len() <= 32
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'));
    if plain {
        version.to_owned()
    } else {
        format!("{major}.x")
    }
}

/// Why the library has no usable report. `Display` is the health message (ARCHITECTURE §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadFailure {
    NotFound,
    /// Another IO error; the text is the error kind (`permission denied`), never the path.
    Io(String),
    NotRegularFile,
    TooLarge {
        bytes: u64,
    },
    Report(ReportError),
}

impl ReadFailure {
    /// IO failures are retried on every poll; the others only when the file changes.
    pub fn is_io(&self) -> bool {
        matches!(self, ReadFailure::NotFound | ReadFailure::Io(_))
    }
}

impl fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadFailure::NotFound => f.write_str("report not found"),
            ReadFailure::Io(kind) => write!(f, "cannot read report: {kind}"),
            ReadFailure::NotRegularFile => f.write_str("report is not a regular file"),
            ReadFailure::TooLarge { bytes } => write!(
                f,
                "report too large ({} MiB, max {} MiB)",
                bytes.div_ceil(MIB),
                MAX_REPORT_BYTES / MIB
            ),
            ReadFailure::Report(e) => e.fmt(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_majors() {
        let rows = [
            ("1.0.0", Some(1)),
            ("1.7.0", Some(1)),
            ("1.0.0-rc.1", Some(1)),
            ("1", Some(1)),
            ("2.0.0", Some(2)),
            ("10.1", Some(10)),
            ("", None),
            ("v1.0.0", None),
            (".1", None),
            ("-1.0", None),
            ("1a.0", None),
            ("99999999999999999999999.0", None),
        ];
        for (version, major) in rows {
            assert_eq!(schema_major(version), major, "{version:?}");
        }
    }

    #[test]
    fn shown_versions() {
        assert_eq!(shown_version("2.0.0", 2), "2.0.0");
        assert_eq!(shown_version("2.0.0-beta+x", 2), "2.0.0-beta+x");
        assert_eq!(shown_version("2.<script>", 2), "2.x");
        assert_eq!(shown_version(&format!("3.{}", "0".repeat(40)), 3), "3.x");
    }

    #[test]
    fn version_gate_messages() {
        let rows = [
            ("not json", "invalid report (line 1, column 2)"),
            ("[]", "invalid report (line 1, column 2)"),
            ("{}", "invalid report: schema_version"),
            (r#"{"schema_version": 1}"#, "invalid report: schema_version"),
            (
                r#"{"schema_version": "x.1"}"#,
                "invalid report: schema_version",
            ),
            (
                r#"{"schema_version": "2.0.0"}"#,
                "unsupported Liner schema 2.0.0",
            ),
            (
                r#"{"schema_version": "0.9.0"}"#,
                "unsupported Liner schema 0.9.0",
            ),
            (
                r#"{"schema_version": "1.0.0"}"#,
                "invalid report (line 1, column 27)",
            ),
        ];
        for (text, message) in rows {
            let e = parse_report(text).expect_err(text);
            assert_eq!(e.error.to_string(), message, "{text}");
        }
    }

    #[test]
    fn invalid_utf8_has_a_position() {
        let e = parse_report_bytes(b"{\n  \"a\": \"\xff\"}").expect_err("invalid");
        assert_eq!(e.error, ReportError::Invalid { line: 2, column: 9 });
    }

    #[test]
    fn read_failure_messages() {
        let rows = [
            (ReadFailure::NotFound, "report not found"),
            (
                ReadFailure::Io("permission denied".into()),
                "cannot read report: permission denied",
            ),
            (ReadFailure::NotRegularFile, "report is not a regular file"),
            (
                ReadFailure::TooLarge { bytes: 40 * MIB },
                "report too large (40 MiB, max 32 MiB)",
            ),
            (
                ReadFailure::TooLarge {
                    bytes: MAX_REPORT_BYTES + 1,
                },
                "report too large (33 MiB, max 32 MiB)",
            ),
            (
                ReadFailure::Report(ReportError::Invalid {
                    line: 12,
                    column: 5,
                }),
                "invalid report (line 12, column 5)",
            ),
            (
                ReadFailure::Report(ReportError::InvalidField("generated_at")),
                "invalid report: generated_at",
            ),
            (
                ReadFailure::Report(ReportError::Unsupported("2.0.0".into())),
                "unsupported Liner schema 2.0.0",
            ),
        ];
        for (failure, message) in rows {
            assert_eq!(failure.to_string(), message);
        }
        assert!(ReadFailure::NotFound.is_io() && ReadFailure::Io(String::new()).is_io());
        assert!(!ReadFailure::NotRegularFile.is_io());
    }
}
