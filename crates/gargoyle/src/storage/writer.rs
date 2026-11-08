//! The single write connection: appending points, rollup, retention and `PRAGMA optimize`.

use std::collections::HashMap;

use rusqlite::{Connection, params};

use super::samples::Sample;

/// Width of a rollup bucket, seconds. Bucket `ts` is its start, a multiple of this.
pub const BUCKET_S: i64 = 300;

/// A bucket is rolled up only once it ended at least this long ago, so a sample whose collection
/// was slow (a call may take up to 20s, ARCHITECTURE §3) still lands in it first.
const ROLLUP_GRACE_S: i64 = 60;

const OPTIMIZE_EVERY_S: i64 = 86_400;

/// Retention in seconds per tier (`[storage]` config).
#[derive(Debug, Clone, Copy)]
pub struct Retention {
    pub raw_s: i64,
    pub rollup_s: i64,
}

/// What one maintenance pass did, for logging and tests.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Maintenance {
    pub buckets_written: usize,
    pub raw_deleted: usize,
    pub rollups_deleted: usize,
    pub series_deleted: usize,
    pub optimized: bool,
}

pub struct Writer {
    conn: Connection,
    host_id: i64,
    /// `key -> series id`. Cleared whenever series rows may have been deleted or rolled back.
    series: HashMap<String, i64>,
    last_optimize: Option<i64>,
}

impl Writer {
    pub fn new(conn: Connection) -> rusqlite::Result<Self> {
        // One host row today; it is named "local" rather than after the hostname so history
        // survives a hostname change (the API shows the collected hostname, §10).
        let host_id = conn.query_row(
            "INSERT INTO hosts (name) VALUES ('local')
             ON CONFLICT (name) DO UPDATE SET name = excluded.name RETURNING id",
            [],
            |r| r.get(0),
        )?;
        Ok(Self {
            conn,
            host_id,
            series: HashMap::new(),
            last_optimize: None,
        })
    }

    pub fn host_id(&self) -> i64 {
        self.host_id
    }

    /// True when no points of either tier exist, e.g. a fresh database.
    pub fn has_no_points(&self) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM points_raw) AND NOT EXISTS (SELECT 1 FROM points_5m)",
            [],
            |r| r.get(0),
        )
    }

    /// Appends raw points in one transaction. A second point for the same series and second
    /// replaces the first.
    pub fn append(&mut self, samples: &[Sample]) -> rusqlite::Result<()> {
        self.in_tx(|tx, ids, host_id| {
            let mut insert = tx.prepare_cached(
                "INSERT OR REPLACE INTO points_raw (series_id, ts, value) VALUES (?1, ?2, ?3)",
            )?;
            for s in samples {
                let id = series_id(tx, ids, host_id, &s.key, s.unit)?;
                insert.execute(params![id, s.ts, s.value])?;
            }
            Ok(())
        })
    }

    /// Runs `f` in a transaction with the series cache. If the transaction fails, ids cached
    /// during it may name rolled-back rows, so the cache is dropped.
    pub fn in_tx<R>(
        &mut self,
        f: impl FnOnce(&Connection, &mut HashMap<String, i64>, i64) -> rusqlite::Result<R>,
    ) -> rusqlite::Result<R> {
        let result = (|| {
            let tx = self.conn.transaction()?;
            let r = f(&tx, &mut self.series, self.host_id)?;
            tx.commit()?;
            Ok(r)
        })();
        if result.is_err() {
            self.series.clear();
        }
        result
    }

    /// Rollup, then retention, then (once a day) `PRAGMA optimize`. Rollup and retention share
    /// one transaction, so raw rows are never deleted unless their buckets were written.
    pub fn maintain(&mut self, now: i64, retention: Retention) -> rusqlite::Result<Maintenance> {
        let mut report = self.in_tx(|tx, ids, host_id| {
            let buckets_written = rollup(tx, host_id, now)?;
            let (raw_deleted, rollups_deleted) = apply_retention(tx, host_id, now, retention)?;
            let series_deleted = prune_series(tx, host_id)?;
            if series_deleted > 0 {
                ids.clear();
            }
            Ok(Maintenance {
                buckets_written,
                raw_deleted,
                rollups_deleted,
                series_deleted,
                optimized: false,
            })
        })?;
        if self
            .last_optimize
            .is_none_or(|t| now - t >= OPTIMIZE_EVERY_S)
        {
            self.conn.execute_batch("PRAGMA optimize")?;
            self.last_optimize = Some(now);
            report.optimized = true;
        }
        Ok(report)
    }
}

/// The id of series `key`, creating it on first use. A changed unit is updated in place.
pub fn series_id(
    conn: &Connection,
    cache: &mut HashMap<String, i64>,
    host_id: i64,
    key: &str,
    unit: &str,
) -> rusqlite::Result<i64> {
    if let Some(&id) = cache.get(key) {
        return Ok(id);
    }
    let id = conn
        .prepare_cached(
            "INSERT INTO series (host_id, key, unit) VALUES (?1, ?2, ?3)
             ON CONFLICT (host_id, key) DO UPDATE SET unit = excluded.unit RETURNING id",
        )?
        .query_row(params![host_id, key, unit], |r| r.get(0))?;
    cache.insert(key.to_owned(), id);
    Ok(id)
}

/// Start of the bucket containing `ts`.
pub fn bucket_start(ts: i64) -> i64 {
    ts.div_euclid(BUCKET_S) * BUCKET_S
}

/// Rolls completed 5m buckets of every series into `points_5m`.
///
/// Each series continues from its newest existing bucket, which is recomputed (it may have been
/// written before all its points arrived). `INSERT OR REPLACE` makes a repeated or interrupted
/// pass harmless, so no separate watermark needs to be persisted.
fn rollup(conn: &Connection, host_id: i64, now: i64) -> rusqlite::Result<usize> {
    let end = bucket_start(now - ROLLUP_GRACE_S);
    let series: Vec<(i64, Option<i64>)> = conn
        .prepare_cached(
            "SELECT s.id, (SELECT max(ts) FROM points_5m WHERE series_id = s.id)
             FROM series s WHERE s.host_id = ?1",
        )?
        .query_map([host_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut stmt = conn.prepare_cached(
        "INSERT OR REPLACE INTO points_5m (series_id, ts, avg, min, max, count)
         SELECT series_id, ts - (ts % ?4) AS bucket, avg(value), min(value), max(value), count(*)
         FROM points_raw WHERE series_id = ?1 AND ts >= ?2 AND ts < ?3
         GROUP BY bucket",
    )?;
    let mut written = 0;
    for (id, newest) in series {
        written += stmt.execute(params![id, newest.unwrap_or(i64::MIN), end, BUCKET_S])?;
    }
    Ok(written)
}

/// Deletes points older than retention, plus events and timer runs past the rollup retention
/// (they are kept as long as the 5m history, §5).
fn apply_retention(
    conn: &Connection,
    host_id: i64,
    now: i64,
    retention: Retention,
) -> rusqlite::Result<(usize, usize)> {
    // Whole buckets only: if the cutoff split a bucket, the next rollup would recompute that
    // bucket (when it is a stopped series' newest) from its surviving half and overwrite the
    // correct rollup. Config guarantees raw_s >= 10m, so the open bucket is never touched.
    let raw_cutoff = bucket_start(now - retention.raw_s);
    let rollup_cutoff = now - retention.rollup_s;
    let ids: Vec<i64> = conn
        .prepare_cached("SELECT id FROM series WHERE host_id = ?1")?
        .query_map([host_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    // Per series, so each delete is a range scan of the primary key instead of a table scan.
    let mut raw = conn.prepare_cached("DELETE FROM points_raw WHERE series_id = ?1 AND ts < ?2")?;
    let mut rolled =
        conn.prepare_cached("DELETE FROM points_5m WHERE series_id = ?1 AND ts < ?2")?;
    let (mut raw_deleted, mut rollups_deleted) = (0, 0);
    for id in ids {
        raw_deleted += raw.execute(params![id, raw_cutoff])?;
        rollups_deleted += rolled.execute(params![id, rollup_cutoff])?;
    }
    conn.prepare_cached("DELETE FROM events WHERE host_id = ?1 AND ts < ?2")?
        .execute(params![host_id, rollup_cutoff])?;
    conn.prepare_cached("DELETE FROM timer_runs WHERE host_id = ?1 AND started_at < ?2")?
        .execute(params![host_id, rollup_cutoff])?;
    Ok((raw_deleted, rollups_deleted))
}

/// Deletes series with no points left in either tier (a removed service, an unmounted disk),
/// so `/metrics` stops listing them. A series is created in the same transaction as its first
/// point, so this never removes one that is still being written.
fn prune_series(conn: &Connection, host_id: i64) -> rusqlite::Result<usize> {
    conn.prepare_cached(
        "DELETE FROM series WHERE host_id = ?1
           AND NOT EXISTS (SELECT 1 FROM points_raw WHERE series_id = series.id)
           AND NOT EXISTS (SELECT 1 FROM points_5m WHERE series_id = series.id)",
    )?
    .execute([host_id])
}

#[cfg(test)]
impl Writer {
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
