//! `GET /api/v1/library` and the library in the overview and events (M7.2), driven by real polls
//! of report files in a temp dir.

use gargoyle_core::api::{LibraryResponse, Severity};
use gargoyle_core::health::Health;

use super::*;
use crate::library::{Reader, tick};

const DEV_REPORT: &str = include_str!("../../../../../config/liner-report.dev.json");
const SCHEMA_2: &str =
    include_str!("../../../../gargoyle-core/tests/fixtures/liner/schema-2.0.0.json");

/// The dev config with `[library] report` in `dir` (holding `contents`, if any), and a database
/// in `dir` when `storage` is set.
fn library_shared(dir: &tempfile::TempDir, contents: Option<&str>, storage: bool) -> Shared {
    let path = dir.path().join("report.json");
    if let Some(text) = contents {
        std::fs::write(&path, text).unwrap();
    }
    let mut config = Config::from_toml_str(DEV_CONFIG).unwrap();
    config
        .library
        .as_mut()
        .expect("dev config has [library]")
        .report = path;
    if storage {
        config.storage.path = dir.path().join("g.db");
        let storage = crate::storage::Storage::open(&config.storage).unwrap();
        Shared::new(config, Arc::new(MockCollector::new(1))).with_storage(storage)
    } else {
        Shared::new(config, Arc::new(MockCollector::new(1)))
    }
}

async fn polled(shared: &Shared, polls: usize) -> Router {
    let path = shared.config.library.as_ref().unwrap().report.clone();
    let mut reader = Reader::new(path);
    for _ in 0..polls {
        tick(shared, &mut reader).await;
    }
    router(shared.clone())
}

async fn library(router: Router) -> LibraryResponse {
    let (status, body) = get_json(router, "/api/v1/library").await;
    assert_eq!(status, StatusCode::OK);
    assert_same_shape(&body, &example("library.json"), "library");
    serde_json::from_value(body).unwrap()
}

#[tokio::test]
async fn unconfigured_matches_the_example_and_changes_nothing() {
    let shared = Shared::new(Config::default(), Arc::new(MockCollector::new(1)));
    system_tick(shared.clone()).await;
    let router = router(shared);
    let (status, body) = get_json(router.clone(), "/api/v1/library").await;
    assert_eq!(status, StatusCode::OK);
    let mut expected = example("library-unconfigured.json");
    expected["host"] = serde_json::json!("gargoyle-mock");
    assert_eq!(body, expected);

    let (_, overview) = get_json(router, "/api/v1/overview").await;
    assert_eq!(overview["library"], Value::Null);
    let o: Overview = serde_json::from_value(overview).unwrap();
    let total = o.counts.healthy + o.counts.warning + o.counts.critical + o.counts.unknown;
    assert_eq!(total, 1, "the system only");
}

#[tokio::test]
async fn configured_before_the_first_poll() {
    let dir = tempfile::tempdir().unwrap();
    let shared = library_shared(&dir, Some(DEV_REPORT), false);
    let r = library(polled(&shared, 0).await).await;
    assert!(r.configured);
    let l = r.library.unwrap();
    assert_eq!(l.health, Health::Unknown);
    assert_eq!(l.message.as_deref(), Some("report not read yet"));
    assert_eq!((l.checked_at, l.generated_at, l.age), (None, None, None));
    assert_eq!(
        (l.issue_counts, l.unreadable_files, l.albums),
        (None, None, None)
    );
    assert_eq!(l.name, "Music library");
    assert_eq!(l.max_age, 26 * 3600);
    assert!(r.report.is_none());
}

#[tokio::test]
async fn a_valid_report() {
    let dir = tempfile::tempdir().unwrap();
    let shared = library_shared(&dir, Some(DEV_REPORT), false);
    let router = polled(&shared, 1).await;
    let r = library(router.clone()).await;
    let l = r.library.unwrap();
    // The dev report is old, has errors and an unreadable file.
    assert_eq!(l.health, Health::Warning);
    let message = l.message.unwrap();
    assert!(message.starts_with("report is "), "{message}");
    assert!(
        message.ends_with("(max 26h); 2 errors, 5 warnings, 1 info in 6 albums; 1 unreadable file"),
        "{message}"
    );
    let generated = gargoyle_core::liner::parse_rfc3339("2026-11-01T03:15:00Z");
    assert_eq!(l.generated_at, generated);
    let age = l.age.unwrap();
    assert!(
        (now() - generated.unwrap() - age).abs() <= 1,
        "age at response time"
    );
    assert!(l.checked_at.is_some());
    assert_eq!(l.albums, Some(6));

    let report = r.report.unwrap();
    assert_eq!(report.schema_version, "1.0.0");
    let ids: Vec<&str> = report.albums.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "Sample Person/Field Notes",
            "The Examples/Night Drive",
            "Various/Summer Mix"
        ],
        "albums with issues, worst first, then by id"
    );
    assert_eq!(report.library_issues.len(), 1);
    assert_eq!(report.library_issues[0].code, "split-album");
    assert_eq!(report.albums[0].issues[0].severity, Severity::Error);
    assert_eq!(report.errors.len(), 1);
    assert_eq!((report.issues_total, report.issues_truncated), (8, false));

    // The overview carries the same summary and counts the library as one target.
    let (_, body) = get_json(router.clone(), "/api/v1/overview").await;
    let o: Overview = serde_json::from_value(body).unwrap();
    let in_overview = o.library.unwrap();
    assert_eq!(in_overview.health, Health::Warning);
    assert_eq!(in_overview.albums, Some(6));
    let total = o.counts.healthy + o.counts.warning + o.counts.critical + o.counts.unknown;
    assert_eq!(total, 1 + 4 + 3 + 2 + 1);

    // GET and HEAD only.
    let resp = send_to(router.clone(), Method::HEAD, "/api/v1/library").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = send_to(router, Method::POST, "/api/v1/library").await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

/// `(health, message)` of the library and whether a report is present.
async fn status_of(contents: Option<&str>) -> (Health, Option<String>, bool) {
    let dir = tempfile::tempdir().unwrap();
    let shared = library_shared(&dir, contents, false);
    let r = library(polled(&shared, 1).await).await;
    let l = r.library.unwrap();
    // Without a valid report the report fields are null.
    if r.report.is_none() {
        assert_eq!((l.generated_at, l.age, l.issue_counts), (None, None, None));
        assert_eq!((l.unreadable_files, l.albums), (None, None));
        assert!(l.checked_at.is_some());
    }
    (l.health, l.message, r.report.is_some())
}

#[tokio::test]
async fn missing_invalid_and_unsupported_reports_are_unknown() {
    assert_eq!(
        status_of(None).await,
        (Health::Unknown, Some("report not found".into()), false)
    );
    assert_eq!(
        status_of(Some("{\"schema_version\": \"1.0.0\",\n  oops")).await,
        (
            Health::Unknown,
            Some("invalid report (line 2, column 3)".into()),
            false
        )
    );
    assert_eq!(
        status_of(Some(SCHEMA_2)).await,
        (
            Health::Unknown,
            Some("unsupported Liner schema 2.0.0".into()),
            false
        )
    );
}

#[tokio::test]
async fn library_events_can_be_filtered_by_kind() {
    let dir = tempfile::tempdir().unwrap();
    let shared = library_shared(&dir, Some(DEV_REPORT), true);
    let router = polled(&shared, 2).await;
    let (status, body) = get_json(router.clone(), "/api/v1/events?kind=library").await;
    assert_eq!(status, StatusCode::OK);
    let list: EventList = serde_json::from_value(body).unwrap();
    assert_eq!(list.events.len(), 1);
    let e = &list.events[0];
    assert_eq!(
        (e.target_kind, e.target.as_str(), e.from, e.to),
        (TargetKind::Library, "library", None, Health::Warning)
    );
    let (_, body) = get_json(router.clone(), "/api/v1/events?kind=library&target=library").await;
    assert_eq!(body["events"].as_array().map(Vec::len), Some(1));
    let (_, body) = get_json(router, "/api/v1/events?kind=check").await;
    assert_eq!(body["events"], serde_json::json!([]));
}

/// The shipped dev report parses and shows every severity, a library issue and an unreadable
/// file, as ROADMAP M7.2 asks.
#[test]
fn the_dev_report_has_everything_the_page_shows() {
    let report = gargoyle_core::liner::parse_report(DEV_REPORT).unwrap();
    let c = report.summary.issues_by_severity;
    assert!(c.error > 0 && c.warning > 0 && c.info > 0);
    assert!(report.issues.iter().any(|i| i.album_id.is_none()));
    assert_eq!(report.errors.len(), 1);
    assert!(report.albums.len() >= 3);
    assert!(report.generated_at < now() - 26 * 3600);
}
