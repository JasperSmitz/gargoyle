//! Persistent history in one SQLite file (ARCHITECTURE §5, ADR 0002).
//!
//! One write connection behind a mutex and one read connection behind another, so chart queries
//! never queue behind a rollup (WAL lets them read concurrently). SQLite calls block, so the async
//! methods run them on the blocking pool.

mod backfill;
pub mod history;
pub mod query;
pub mod samples;
mod schema;
mod writer;

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Context;
use gargoyle_core::api::{Event, RunSource, SeriesData, SeriesInfo, TargetKind, TimerRun};
use gargoyle_core::config::StorageConfig;
use gargoyle_core::health::{Health, TimerRuns, Transition};
use gargoyle_core::model::TimerRunRecord;
use rusqlite::Connection;

pub use backfill::backfill_mock;
pub use history::EventFilter;
pub use query::{NativeIntervals, Plan, QueryError, QueryRequest};
pub use samples::Sample;
pub use writer::{Maintenance, Retention};

use writer::Writer;

#[derive(Clone)]
pub struct Storage {
    inner: Arc<Inner>,
}

struct Inner {
    writer: Mutex<Writer>,
    reader: Mutex<Connection>,
    host_id: i64,
    retention: Retention,
}

/// History is not secret (§9.4) but is nobody else's business: what Gargoyle creates is
/// readable by its own user and group only. Existing files and directories keep the mode the
/// admin gave them.
const DIR_MODE: u32 = 0o750;
const FILE_MODE: u32 = 0o640;

/// Creates `dir` and missing parents with [`DIR_MODE`]; the leaf is set explicitly so the
/// process umask cannot widen it.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.exists() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)?;
    set_mode(dir, DIR_MODE)
}

/// Creates an empty database file with [`FILE_MODE`] (SQLite accepts an empty file as a new
/// database). Returns whether it was created here.
fn create_private_file(path: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)
    {
        Ok(_) => set_mode(path, FILE_MODE).map(|()| true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

impl Storage {
    /// Opens (creating if needed) the database and its parent directory, and migrates it.
    /// Blocking.
    pub fn open(config: &StorageConfig) -> anyhow::Result<Self> {
        let path = &config.path;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            create_private_dir(dir)
                .with_context(|| format!("cannot create database directory {}", dir.display()))?;
        }
        let created = create_private_file(path)
            .with_context(|| format!("cannot create database {}", path.display()))?;
        let open = |p: &Path| -> rusqlite::Result<(Writer, Connection)> {
            let writer = Writer::new(schema::open_writer(p)?)?;
            // Opened after the writer, which creates the file and the schema.
            let reader = schema::open_reader(p)?;
            Ok((writer, reader))
        };
        let (writer, reader) =
            open(path).with_context(|| format!("cannot open database {}", path.display()))?;
        if created {
            // SQLite gives its -wal and -shm files the database file's mode, but set them
            // explicitly in case a version does not.
            for suffix in ["-wal", "-shm"] {
                let mut side = path.as_os_str().to_owned();
                side.push(suffix);
                let side = std::path::PathBuf::from(side);
                if side.exists() {
                    set_mode(&side, FILE_MODE)
                        .with_context(|| format!("cannot restrict {}", side.display()))?;
                }
            }
        }
        tracing::info!(path = %path.display(), "database opened");
        Ok(Self {
            inner: Arc::new(Inner {
                host_id: writer.host_id(),
                writer: Mutex::new(writer),
                reader: Mutex::new(reader),
                retention: Retention {
                    raw_s: secs(config.raw_retention),
                    rollup_s: secs(config.rollup_retention),
                },
            }),
        })
    }

    /// Runs `f` with the write connection on the blocking pool.
    async fn write<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Writer, Retention) -> rusqlite::Result<R> + Send + 'static,
    ) -> anyhow::Result<R> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            // A panic mid-write rolls its transaction back when it unwinds, so the connection is
            // still consistent and a poisoned lock can be reused.
            let mut w = inner.writer.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut w, inner.retention)
        })
        .await
        .context("storage task failed")?
        .context("database write failed")
    }

    /// Runs `f` with the read connection on the blocking pool.
    async fn read<R: Send + 'static, E: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection, i64) -> Result<R, E> + Send + 'static,
    ) -> anyhow::Result<Result<R, E>> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let conn = inner.reader.lock().unwrap_or_else(PoisonError::into_inner);
            f(&conn, inner.host_id)
        })
        .await
        .context("storage task failed")
    }

    pub async fn append(&self, samples: Vec<Sample>) -> anyhow::Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        self.write(move |w, _| w.append(&samples)).await
    }

    /// Rollup, retention and the daily `PRAGMA optimize` (the maintenance tick, §3).
    pub async fn maintain(&self, now: i64) -> anyhow::Result<Maintenance> {
        self.write(move |w, retention| w.maintain(now, retention))
            .await
    }

    pub async fn series(&self) -> anyhow::Result<Vec<SeriesInfo>> {
        Ok(self.read(query::list).await??)
    }

    /// Runs a validated query.
    pub async fn query(
        &self,
        req: QueryRequest,
        now: i64,
        intervals: NativeIntervals,
    ) -> Result<(Plan, Vec<SeriesData>), QueryError> {
        let raw_s = self.inner.retention.raw_s;
        self.read(move |conn, host_id| query::run(conn, host_id, &req, now, raw_s, &intervals))
            .await
            .map_err(QueryError::Internal)?
    }
}

impl Storage {
    /// Logs health transitions observed at `ts`.
    pub async fn insert_events(&self, ts: i64, events: Vec<Transition>) -> anyhow::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        self.write(move |w, _| w.in_tx(|tx, _, host| history::insert_events(tx, host, ts, &events)))
            .await
    }

    /// Inserts or completes timer runs; see [`history::upsert_runs`].
    pub async fn upsert_runs(
        &self,
        runs: Vec<TimerRunRecord>,
        source: RunSource,
    ) -> anyhow::Result<()> {
        if runs.is_empty() {
            return Ok(());
        }
        self.write(move |w, _| w.in_tx(|tx, _, host| history::upsert_runs(tx, host, &runs, source)))
            .await
    }

    pub async fn events(&self, filter: EventFilter) -> anyhow::Result<Vec<Event>> {
        Ok(self
            .read(move |conn, host| history::events(conn, host, &filter))
            .await??)
    }

    /// The last logged state of every target.
    pub async fn last_states(&self) -> anyhow::Result<Vec<(TargetKind, String, Health)>> {
        Ok(self.read(history::last_states).await??)
    }

    pub async fn timer_runs(&self, timer: String, limit: i64) -> anyhow::Result<Vec<TimerRun>> {
        Ok(self
            .read(move |conn, host| history::timer_runs(conn, host, &timer, limit))
            .await??)
    }

    /// The newest run and the newest success of each of `timers`.
    pub async fn timer_summaries(
        &self,
        timers: Vec<String>,
    ) -> anyhow::Result<Vec<(String, TimerRuns)>> {
        Ok(self
            .read(move |conn, host| {
                timers
                    .into_iter()
                    .map(|t| Ok((t.clone(), history::timer_summary(conn, host, &t)?)))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await??)
    }
}

/// Whole seconds as `i64`; config validation keeps retentions far below the overflow point.
fn secs(d: std::time::Duration) -> i64 {
    i64::try_from(d.as_secs()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests;
