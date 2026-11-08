//! Mock mode only (M2.4): seven days of fake history on an empty database, so every chart range
//! has something to show right after `make dev`.
//!
//! Raw points are written only for the raw-retention window; older samples go straight into 5m
//! buckets, aggregated here exactly as the rollup would. Everything is one transaction.

use std::collections::HashMap;

use gargoyle_collector::MockCollector;
use gargoyle_core::rates::system_rates;
use rusqlite::params;

use super::Storage;
use super::samples::{self, Sample};
use super::writer::{BUCKET_S, bucket_start, series_id};

const HISTORY_S: i64 = 7 * 86_400;

/// One 5m bucket being accumulated.
struct Acc {
    unit: &'static str,
    start: i64,
    sum: f64,
    min: f64,
    max: f64,
    count: i64,
}

impl Acc {
    fn new(s: &Sample) -> Self {
        Self {
            unit: s.unit,
            start: bucket_start(s.ts),
            sum: s.value,
            min: s.value,
            max: s.value,
            count: 1,
        }
    }

    fn add(&mut self, v: f64) {
        self.sum += v;
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.count += 1;
    }
}

/// Fills the database with `mock` samples every `interval_s` from 7 days before `now` up to (not
/// including) `now`, then rolls up the raw part. Does nothing if any points exist. Returns the
/// number of samples generated. Blocking.
///
/// `mock` keeps advancing its counters, so the live loop continuing with the same collector at
/// `now` sees one smooth series.
pub fn backfill_mock(
    storage: &Storage,
    mock: &MockCollector,
    now: i64,
    interval_s: i64,
) -> anyhow::Result<Option<usize>> {
    let interval_s = interval_s.max(1);
    let mut w = storage
        .inner
        .writer
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !w.has_no_points()? {
        return Ok(None);
    }
    let retention = storage.inner.retention;
    let start = now - HISTORY_S.min(retention.rollup_s);
    // Bucket-aligned, so no 5m bucket is split between the two paths.
    let raw_start = bucket_start(now - retention.raw_s + BUCKET_S - 1);

    let generated = w.in_tx(|tx, ids, host_id| {
        let mut raw = tx.prepare_cached(
            "INSERT OR REPLACE INTO points_raw (series_id, ts, value) VALUES (?1, ?2, ?3)",
        )?;
        let mut rolled = tx.prepare_cached(
            "INSERT OR REPLACE INTO points_5m (series_id, ts, avg, min, max, count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        let mut flush = |ids: &mut HashMap<String, i64>, key: &str, a: &Acc| {
            let id = series_id(tx, ids, host_id, key, a.unit)?;
            rolled
                .execute(params![
                    id,
                    a.start,
                    a.sum / a.count as f64,
                    a.min,
                    a.max,
                    a.count
                ])
                .map(drop)
        };
        let mut open: HashMap<String, Acc> = HashMap::new();
        let mut prev = None;
        let mut generated = 0;
        let mut ts = start;
        while ts < now {
            let snap = mock.snapshot_at(ts);
            let rates = system_rates(prev.as_ref(), &snap);
            for s in samples::system(&snap, rates.cpu, &rates.net) {
                generated += 1;
                if s.ts >= raw_start {
                    let id = series_id(tx, ids, host_id, &s.key, s.unit)?;
                    raw.execute(params![id, s.ts, s.value])?;
                    continue;
                }
                match open.get_mut(&s.key) {
                    Some(a) if a.start == bucket_start(s.ts) => a.add(s.value),
                    Some(a) => {
                        flush(ids, &s.key, a)?;
                        *a = Acc::new(&s);
                    }
                    None => {
                        open.insert(s.key.clone(), Acc::new(&s));
                    }
                }
            }
            prev = Some(snap);
            ts += interval_s;
        }
        for (key, a) in &open {
            flush(ids, key, a)?;
        }
        Ok(generated)
    })?;
    w.maintain(now, retention)?;
    Ok(Some(generated))
}
