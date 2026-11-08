//! The library reader (ARCHITECTURE §3, ADR 0007): polls the Liner report file named by
//! `[library] report` and keeps its API view in `State`. Gargoyle never runs Liner.
//!
//! Every `interval` the file is `stat`ed. It is read and parsed again only when its identity or
//! metadata changed (device, inode, size, mtime, ctime) or the last read failed with an IO error.
//! Liner replaces the report by rename, so a new report always has a new inode; a `chmod` changes
//! ctime, so a fixed permission is picked up too. Only regular files are read, and never more
//! than [`MAX_REPORT_BYTES`]. Health is evaluated on every poll, because the report ages.
//!
//! The configured path is logged (once per distinct error) but never put into a message, so it
//! cannot reach the API (§9.4).

use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gargoyle_core::liner::{MAX_REPORT_BYTES, ReadFailure, build_view, parse_report_bytes};
use rustix::fs::{Mode, OFlags};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::scheduler::{Busy, CallError, Shared, call, now, store_events};
use crate::state::{LibraryPoll, LoadedReport};

/// The first poll waits this long after startup.
const FIRST_POLL_DELAY: Duration = Duration::from_secs(1);

/// The longest one poll is awaited, whatever the interval: parsing a large report may take longer
/// than a short interval, and polls never overlap anyway (the loop awaits each one).
const POLL_LIMIT: Duration = Duration::from_secs(20);

/// Starts the library loop, if `[library]` is configured.
pub fn spawn(shared: &Shared) -> Option<JoinHandle<()>> {
    let cfg = shared.config.library.clone()?;
    let shared = shared.clone();
    Some(tokio::spawn(async move {
        let mut reader = Reader::new(cfg.report);
        let start = tokio::time::Instant::now() + FIRST_POLL_DELAY;
        let mut interval = tokio::time::interval_at(start, cfg.interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            tick(&shared, &mut reader).await;
        }
    }))
}

/// The reader's memory between polls.
pub struct Reader {
    path: PathBuf,
    /// The file behind the current outcome; `None` forces a read on the next poll.
    key: Option<ChangeKey>,
    busy: Busy,
    /// The last error logged, so each distinct error is logged once, not every poll.
    logged: Option<String>,
    /// Polls that read the file (as opposed to finding it unchanged or unreadable).
    reads: u64,
}

impl Reader {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            key: None,
            busy: Busy::default(),
            logged: None,
            reads: 0,
        }
    }

    #[cfg(test)]
    pub fn reads(&self) -> u64 {
        self.reads
    }
}

/// One poll: `stat`, read and parse if changed, record, and log the library's transitions.
pub async fn tick(shared: &Shared, reader: &mut Reader) {
    let path = reader.path.clone();
    let known = reader.key;
    let polled = call(&reader.busy, POLL_LIMIT, move || Ok(poll(&path, known))).await;
    let poll = match polled {
        Ok(Polled::Unchanged) => LibraryPoll::Unchanged,
        Ok(Polled::Read {
            key,
            opened,
            result,
        }) => {
            if opened {
                reader.reads += 1;
            }
            reader.key = match &result {
                Err(f) if f.failure.is_io() => None,
                _ => key,
            };
            LibraryPoll::Read(reader.outcome(result))
        }
        Err(e) => {
            reader.key = None;
            let kind = match e {
                CallError::Panicked => "reader crashed".to_owned(),
                other => other.to_string(),
            };
            LibraryPoll::Read(reader.outcome(Err(Failure::new(ReadFailure::Io(kind)))))
        }
    };
    let ts = now();
    let events = shared.write(|s| {
        s.record_library(ts, poll);
        let target = s.library_target(&shared.config, ts);
        s.transitions(target.into_iter().collect())
    });
    store_events(shared, ts, events).await;
}

impl Reader {
    /// Logs a failure once per distinct message, with the path and the full detail, which the
    /// API never sees.
    fn outcome(
        &mut self,
        result: Result<Box<LoadedReport>, Failure>,
    ) -> Result<Box<LoadedReport>, ReadFailure> {
        match result {
            Ok(report) => {
                if self.logged.take().is_some() {
                    tracing::info!("library report readable again");
                }
                tracing::debug!(
                    generated_at = report.generated_at,
                    issues = report.view.issues_total,
                    "library report read"
                );
                Ok(report)
            }
            Err(f) => {
                let message = f.failure.to_string();
                if self.logged.as_deref() != Some(&message) {
                    tracing::warn!(
                        path = %self.path.display(),
                        error = %f.detail,
                        "cannot use the library report"
                    );
                    self.logged = Some(message);
                }
                Err(f.failure)
            }
        }
    }
}

/// The identity and metadata of the report file. Any difference means "read it again".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChangeKey {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl ChangeKey {
    fn of(m: &Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            size: m.size(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
        }
    }
}

/// A failed read: the API's reason and the detail for the log.
#[derive(Debug)]
struct Failure {
    failure: ReadFailure,
    /// May include report content (serde's message); log only.
    detail: String,
}

impl Failure {
    fn new(failure: ReadFailure) -> Self {
        let detail = failure.to_string();
        Self { failure, detail }
    }
}

#[derive(Debug)]
enum Polled {
    Unchanged,
    Read {
        key: Option<ChangeKey>,
        /// Whether the file was opened (for the read counter).
        opened: bool,
        result: Result<Box<LoadedReport>, Failure>,
    },
}

/// Blocking: `stat`s `path` and, unless it is the file behind `known`, reads and parses it.
fn poll(path: &Path, known: Option<ChangeKey>) -> Polled {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            return Polled::Read {
                key: None,
                opened: false,
                result: Err(Failure::new(io_failure(&e))),
            };
        }
    };
    let key = ChangeKey::of(&meta);
    if known == Some(key) {
        return Polled::Unchanged;
    }
    let mut opened = false;
    let result = read(path, &meta, &mut opened);
    Polled::Read {
        key: Some(key),
        opened,
        result,
    }
}

/// Reads and parses the report `meta` describes. Checks the type and size before opening, and
/// again on the opened file in case the path was replaced in between.
fn read(path: &Path, meta: &Metadata, opened: &mut bool) -> Result<Box<LoadedReport>, Failure> {
    check_file(meta)?;
    let file = open(path).map_err(|e| Failure::new(io_failure(&e)))?;
    *opened = true;
    let meta = file.metadata().map_err(|e| Failure::new(io_failure(&e)))?;
    check_file(&meta)?;
    let bytes = read_bounded(file, MAX_REPORT_BYTES)
        .map_err(|e| Failure::new(io_failure(&e)))?
        .ok_or_else(|| {
            Failure::new(ReadFailure::TooLarge {
                bytes: meta.len().max(MAX_REPORT_BYTES + 1),
            })
        })?;
    let report = parse_report_bytes(&bytes).map_err(|e| Failure {
        failure: ReadFailure::Report(e.error),
        detail: e.detail,
    })?;
    Ok(Box::new(LoadedReport {
        generated_at: report.generated_at,
        view: build_view(&report),
    }))
}

/// Regular files of at most [`MAX_REPORT_BYTES`] only.
fn check_file(meta: &Metadata) -> Result<(), Failure> {
    if !meta.is_file() {
        return Err(Failure::new(ReadFailure::NotRegularFile));
    }
    if meta.len() > MAX_REPORT_BYTES {
        return Err(Failure::new(ReadFailure::TooLarge { bytes: meta.len() }));
    }
    Ok(())
}

/// Opens for reading without blocking: if the path became a FIFO after the `stat`, `open` would
/// otherwise wait for a writer. `O_NONBLOCK` does not affect reads of a regular file.
fn open(path: &Path) -> io::Result<File> {
    let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOCTTY;
    let fd = rustix::fs::open(path, flags, Mode::empty())?;
    Ok(File::from(fd))
}

/// Reads at most `max` bytes; `None` if there is more, e.g. a file that grew after the `stat`.
fn read_bounded(r: impl Read, max: u64) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    r.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= max).then_some(bytes))
}

/// An IO error as its kind (`permission denied`), never with the path.
fn io_failure(e: &io::Error) -> ReadFailure {
    match e.kind() {
        io::ErrorKind::NotFound => ReadFailure::NotFound,
        kind => ReadFailure::Io(kind.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use gargoyle_collector::MockCollector;
    use gargoyle_core::config::Config;
    use gargoyle_core::health::Health;

    use super::*;
    use crate::storage::Storage;

    /// Liner's golden report, as a template for test reports.
    const GOLDEN: &str =
        include_str!("../../gargoyle-core/tests/fixtures/liner/golden-report.json");

    /// The golden report with `generated_at` an hour ago and `albums` set, so a test can tell
    /// two versions apart.
    fn report(albums: u64) -> String {
        let mut v: serde_json::Value = serde_json::from_str(GOLDEN).unwrap();
        v["generated_at"] = serde_json::json!(rfc3339(now() - 3600));
        v["summary"]["albums"] = serde_json::json!(albums);
        v.to_string()
    }

    /// Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (civil_from_days, the inverse of the parser's).
    fn rfc3339(ts: i64) -> String {
        let (days, secs) = (ts.div_euclid(86_400), ts.rem_euclid(86_400));
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            secs / 3600,
            secs % 3600 / 60,
            secs % 60
        )
    }

    #[test]
    fn rfc3339_helper_round_trips() {
        for ts in [0, 1_794_132_000, now()] {
            assert_eq!(gargoyle_core::liner::parse_rfc3339(&rfc3339(ts)), Some(ts));
        }
    }

    fn shared_for(path: &Path, storage: Option<&tempfile::TempDir>) -> Shared {
        let mut config = Config::from_toml_str(&format!(
            "[collector]\nmode = \"mock\"\njournal_backfill = false\n\
             [library]\nreport = {:?}\n",
            path.display().to_string()
        ))
        .unwrap();
        let shared_config = |config: Config| Shared::new(config, Arc::new(MockCollector::new(1)));
        match storage {
            None => shared_config(config),
            Some(dir) => {
                config.storage.path = dir.path().join("g.db");
                let storage = Storage::open(&config.storage).unwrap();
                shared_config(config).with_storage(storage)
            }
        }
    }

    /// `(health, message)` of the library now.
    fn health(shared: &Shared) -> (Health, Option<String>) {
        let l = shared
            .read(|s| s.library_view(&shared.config, now()))
            .expect("configured");
        (l.health, l.message)
    }

    fn albums(shared: &Shared) -> Option<u64> {
        shared
            .read(|s| s.library_report())
            .map(|r| r.summary.albums)
    }

    /// Writes `text` next to `path` and renames it over `path`, as Liner does.
    fn replace(path: &Path, text: &str) {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text).unwrap();
        std::fs::rename(&tmp, path).unwrap();
    }

    #[tokio::test]
    async fn rename_is_picked_up_and_an_unchanged_file_is_not_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        replace(&path, &report(3));
        let shared = shared_for(&path, None);
        let mut reader = Reader::new(path.clone());

        assert_eq!(
            health(&shared),
            (Health::Unknown, Some("report not read yet".into()))
        );
        tick(&shared, &mut reader).await;
        assert_eq!(reader.reads(), 1);
        assert_eq!(albums(&shared), Some(3));
        // The golden report has an error and unreadable files.
        assert_eq!(health(&shared).0, Health::Warning);

        for _ in 0..3 {
            tick(&shared, &mut reader).await;
        }
        assert_eq!(reader.reads(), 1, "unchanged: not read again");
        let checked = shared
            .read(|s| s.library_view(&shared.config, now()))
            .and_then(|l| l.checked_at);
        assert!(checked.is_some());

        replace(&path, &report(4));
        tick(&shared, &mut reader).await;
        assert_eq!(reader.reads(), 2);
        assert_eq!(albums(&shared), Some(4));
    }

    #[tokio::test]
    async fn a_fixed_permission_is_picked_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        replace(&path, &report(3));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            eprintln!("running as root: permissions are not enforced, skipping");
            return;
        }
        let shared = shared_for(&path, None);
        let mut reader = Reader::new(path.clone());
        tick(&shared, &mut reader).await;
        tick(&shared, &mut reader).await;
        assert_eq!(
            health(&shared),
            (
                Health::Unknown,
                Some("cannot read report: permission denied".into())
            )
        );
        assert_eq!(albums(&shared), None);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        tick(&shared, &mut reader).await;
        assert_eq!(albums(&shared), Some(3));
    }

    #[tokio::test]
    async fn an_oversized_file_is_refused_without_opening_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let file = File::create(&path).unwrap();
        // Sparse: no disk space or memory used.
        file.set_len(MAX_REPORT_BYTES + 1).unwrap();
        // Unreadable as well: opening it would fail with "permission denied" instead (unless
        // the tests run as root, where the read counter still shows it was not opened).
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let shared = shared_for(&path, None);
        let mut reader = Reader::new(path.clone());
        tick(&shared, &mut reader).await;
        assert_eq!(
            health(&shared),
            (
                Health::Unknown,
                Some("report too large (33 MiB, max 32 MiB)".into())
            )
        );
        assert_eq!(reader.reads(), 0);
        assert_eq!(albums(&shared), None);
    }

    #[test]
    fn reads_are_bounded() {
        let data = [b'x'; 100];
        assert_eq!(
            read_bounded(&data[..], 100).unwrap().map(|b| b.len()),
            Some(100)
        );
        assert_eq!(read_bounded(&data[..], 99).unwrap(), None);
        assert_eq!(read_bounded(&data[..0], 0).unwrap(), Some(Vec::new()));
    }

    #[tokio::test]
    async fn a_directory_is_not_a_report() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_for(dir.path(), None);
        let mut reader = Reader::new(dir.path().to_owned());
        tick(&shared, &mut reader).await;
        assert_eq!(
            health(&shared),
            (Health::Unknown, Some("report is not a regular file".into()))
        );
        assert_eq!(reader.reads(), 0);
    }

    fn mkfifo(path: &Path) {
        rustix::fs::mknodat(
            rustix::fs::CWD,
            path,
            rustix::fs::FileType::Fifo,
            Mode::from_raw_mode(0o644),
            0,
        )
        .unwrap();
    }

    /// A FIFO without a writer would block `open` (and a read) forever.
    #[tokio::test]
    async fn a_fifo_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        mkfifo(&path);
        let shared = shared_for(&path, None);
        let mut reader = Reader::new(path.clone());
        tick(&shared, &mut reader).await;
        assert_eq!(
            health(&shared),
            (Health::Unknown, Some("report is not a regular file".into()))
        );
        assert_eq!(reader.reads(), 0);
    }

    /// The path turned into a FIFO between the `stat` and the `open`: the non-blocking open
    /// returns at once, and the opened file is checked again.
    #[test]
    fn a_fifo_swapped_in_after_the_stat_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let regular = dir.path().join("regular.json");
        std::fs::write(&regular, GOLDEN).unwrap();
        let meta = std::fs::metadata(&regular).unwrap();
        let fifo = dir.path().join("fifo.json");
        mkfifo(&fifo);
        let mut opened = false;
        let e = read(&fifo, &meta, &mut opened).expect_err("not a regular file");
        assert_eq!(e.failure, ReadFailure::NotRegularFile);
    }

    /// `(target, from, to, message)` of every stored event, oldest first.
    type EventRow = (String, Option<String>, String, Option<String>);

    fn stored_events(dir: &tempfile::TempDir) -> Vec<EventRow> {
        let conn = rusqlite::Connection::open(dir.path().join("g.db")).unwrap();
        conn.prepare(
            "SELECT target, from_state, to_state, message FROM events
             WHERE target_kind = 'library' ORDER BY id",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    /// ROADMAP M7 acceptance: a health change is logged once (after the hold-down), not again
    /// after a restart, and a deleted report turns the library unknown with `report not found`.
    #[tokio::test]
    async fn transitions_are_logged_once_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        replace(&path, &report(3));
        let start = || async {
            let shared = shared_for(&path, Some(&dir));
            crate::scheduler::restore(&shared).await.unwrap();
            (shared, Reader::new(path.clone()))
        };
        let warning = (
            "library".to_owned(),
            None,
            "warning".to_owned(),
            Some("1 error, 4 warnings, 2 info in 3 albums; 2 unreadable files".to_owned()),
        );

        let (shared, mut reader) = start().await;
        tick(&shared, &mut reader).await;
        assert!(stored_events(&dir).is_empty(), "held down");
        tick(&shared, &mut reader).await;
        tick(&shared, &mut reader).await;
        assert_eq!(stored_events(&dir), std::slice::from_ref(&warning));

        drop(shared);
        let (shared, mut reader) = start().await;
        for _ in 0..3 {
            tick(&shared, &mut reader).await;
        }
        assert_eq!(
            stored_events(&dir),
            std::slice::from_ref(&warning),
            "nothing new after the restart"
        );

        std::fs::remove_file(&path).unwrap();
        tick(&shared, &mut reader).await;
        // Live health is not held down; the event is.
        assert_eq!(
            health(&shared),
            (Health::Unknown, Some("report not found".into()))
        );
        assert_eq!(albums(&shared), None, "the old report is dropped");
        assert_eq!(stored_events(&dir).len(), 1);
        tick(&shared, &mut reader).await;
        assert_eq!(
            stored_events(&dir),
            [
                warning,
                (
                    "library".to_owned(),
                    Some("warning".to_owned()),
                    "unknown".to_owned(),
                    Some("report not found".to_owned())
                )
            ]
        );
        // A missing file is retried every poll, and comes back as soon as it is there.
        replace(&path, &report(3));
        tick(&shared, &mut reader).await;
        assert_eq!(albums(&shared), Some(3));
    }

    #[tokio::test]
    async fn without_library_config_nothing_is_spawned_or_read() {
        let shared = Shared::new(Config::default(), Arc::new(MockCollector::new(1)));
        assert!(spawn(&shared).is_none());
        let o = shared.read(|s| s.overview(&shared.config, now()));
        assert_eq!(o.library, None);
        assert!(
            shared
                .read(|s| s.library_target(&shared.config, now()))
                .is_none()
        );
    }

    #[tokio::test]
    async fn the_spawned_loop_reads_after_a_second() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        replace(&path, &report(3));
        let shared = shared_for(&path, None);
        let handle = spawn(&shared).expect("configured");
        for _ in 0..300 {
            if albums(&shared).is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        handle.abort();
        assert_eq!(albums(&shared), Some(3));
    }
}
