//! Persistent history in one SQLite file (ARCHITECTURE §5, ADR 0002).
//!
//! One write connection behind a mutex and one read connection behind another, so chart queries
//! never queue behind a rollup (WAL lets them read concurrently). SQLite calls block, so the async
//! methods run them on the blocking pool.

mod backfill;
pub mod query;
pub mod samples;
mod schema;
mod writer;

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Context;
use gargoyle_core::api::{SeriesData, SeriesInfo};
use gargoyle_core::config::StorageConfig;
use rusqlite::Connection;

pub use backfill::backfill_mock;
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

impl Storage {
    /// Opens (creating if needed) the database and its parent directory, and migrates it.
    /// Blocking.
    pub fn open(config: &StorageConfig) -> anyhow::Result<Self> {
        let path = &config.path;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create database directory {}", dir.display()))?;
        }
        let open = |p: &Path| -> rusqlite::Result<(Writer, Connection)> {
            let writer = Writer::new(schema::open_writer(p)?)?;
            // Opened after the writer, which creates the file and the schema.
            let reader = schema::open_reader(p)?;
            Ok((writer, reader))
        };
        let (writer, reader) =
            open(path).with_context(|| format!("cannot open database {}", path.display()))?;
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

/// Whole seconds as `i64`; config validation keeps retentions far below the overflow point.
fn secs(d: std::time::Duration) -> i64 {
    i64::try_from(d.as_secs()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests;
