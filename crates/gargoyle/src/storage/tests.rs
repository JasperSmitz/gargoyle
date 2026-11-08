//! Storage acceptance tests (ROADMAP M2) against temp databases.

use std::time::{Duration, Instant};

use gargoyle_collector::MockCollector;
use gargoyle_core::api::Resolution;
use gargoyle_core::config::StorageConfig;
use tempfile::TempDir;

use super::*;

/// System every 10s, units every 30s, no checks.
fn iv() -> NativeIntervals {
    NativeIntervals {
        system_s: 10,
        units_s: 30,
        checks_s: std::collections::HashMap::new(),
    }
}

const NOW: i64 = 1_794_200_000;
const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

fn config(dir: &TempDir) -> StorageConfig {
    StorageConfig {
        // A missing parent directory is created.
        path: dir.path().join("sub/dir/g.db"),
        ..StorageConfig::default()
    }
}

fn open(dir: &TempDir) -> Storage {
    Storage::open(&config(dir)).unwrap()
}

fn sample(key: &str, ts: i64, value: f64) -> Sample {
    Sample {
        key: key.into(),
        unit: samples::PERCENT,
        ts,
        value,
    }
}

fn req(keys: &[&str], from: i64, to: i64, points: i64) -> QueryRequest {
    QueryRequest {
        keys: keys.iter().map(|k| (*k).to_owned()).collect(),
        from,
        to,
        points,
    }
}

fn count(s: &Storage, sql: &str) -> i64 {
    let w = s.inner.writer.lock().unwrap();
    w.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

/// `(ts, avg, min, max, count)` of the rollups of `key`, oldest first.
fn rollups(s: &Storage, key: &str) -> Vec<(i64, f64, f64, f64, i64)> {
    let w = s.inner.writer.lock().unwrap();
    w.conn()
        .prepare(
            "SELECT p.ts, p.avg, p.min, p.max, p.count FROM points_5m p
             JOIN series s ON s.id = p.series_id WHERE s.key = ?1 ORDER BY p.ts",
        )
        .unwrap()
        .query_map([key], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[tokio::test]
async fn history_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    s.append(vec![
        sample("cpu.usage", NOW - 20, 10.0),
        sample("cpu.usage", NOW - 10, 20.0),
    ])
    .await
    .unwrap();
    drop(s);

    let s = open(&dir);
    let (plan, series) = s
        .query(req(&["cpu.usage"], NOW - 60, NOW, 300), NOW, iv())
        .await
        .unwrap();
    assert_eq!(plan.resolution, Resolution::Raw);
    let values: Vec<f64> = series[0].points.iter().filter_map(|p| p.1).collect();
    assert_eq!(values, [10.0, 20.0]);
    // The series id is reused, not duplicated.
    s.append(vec![sample("cpu.usage", NOW, 30.0)])
        .await
        .unwrap();
    assert_eq!(count(&s, "SELECT count(*) FROM series"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM hosts"), 1);
}

#[tokio::test]
async fn rollup_is_exact_idempotent_and_skips_the_open_bucket() {
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    let b0 = writer::bucket_start(NOW) - 3 * writer::BUCKET_S;
    // Bucket b0: 1, 2, ..., 30 every 10s. Bucket b0+300: -5 and 5. Current bucket: one point.
    let mut points: Vec<Sample> = (0..30)
        .map(|i| sample("x", b0 + i * 10, f64::from(i as i32 + 1)))
        .collect();
    points.push(sample("x", b0 + 300, -5.0));
    points.push(sample("x", b0 + 599, 5.0));
    points.push(sample("x", writer::bucket_start(NOW), 99.0));
    s.append(points).await.unwrap();

    let m = s.maintain(NOW).await.unwrap();
    assert_eq!(m.buckets_written, 2);
    let expected = vec![(b0, 15.5, 1.0, 30.0, 30), (b0 + 300, 0.0, -5.0, 5.0, 2)];
    assert_eq!(rollups(&s, "x"), expected);

    // Running again changes nothing.
    s.maintain(NOW).await.unwrap();
    assert_eq!(rollups(&s, "x"), expected);

    // A late point in the newest rolled-up bucket is picked up by the next pass.
    s.append(vec![sample("x", b0 + 310, 15.0)]).await.unwrap();
    s.maintain(NOW).await.unwrap();
    assert_eq!(rollups(&s, "x")[1], (b0 + 300, 5.0, -5.0, 15.0, 3));

    // After a restart, rollup continues where it left off once the open bucket completes. The
    // empty bucket b0+600 stays absent.
    drop(s);
    let s = open(&dir);
    s.maintain(NOW + 300).await.unwrap();
    let r = rollups(&s, "x");
    assert_eq!(r.len(), 3, "{r:?}");
    assert_eq!(r[2], (writer::bucket_start(NOW), 99.0, 99.0, 99.0, 1));
}

#[tokio::test]
async fn retention_deletes_old_rows_only() {
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    let raw_ret = 48 * HOUR;
    let rollup_ret = 400 * DAY;
    // Raw rows go in whole buckets: the bucket holding `NOW - raw_ret` is kept entirely.
    let cutoff = writer::bucket_start(NOW - raw_ret);
    assert_ne!(cutoff, NOW - raw_ret);
    s.append(vec![
        sample("x", cutoff - 10, 1.0),
        sample("x", cutoff, 2.0),
        sample("x", NOW - raw_ret - 10, 3.0),
    ])
    .await
    .unwrap();
    {
        let w = s.inner.writer.lock().unwrap();
        let c = w.conn();
        for ts in [NOW - rollup_ret - 300, NOW - rollup_ret + 300] {
            c.execute(
                "INSERT INTO points_5m VALUES ((SELECT id FROM series), ?1, 1, 1, 1, 1)",
                [ts],
            )
            .unwrap();
            c.execute(
                "INSERT INTO events (host_id, ts, target_kind, target, to_state)
                 VALUES (1, ?1, 'system', 'cpu', 'healthy')",
                [ts],
            )
            .unwrap();
            c.execute(
                "INSERT INTO timer_runs (host_id, timer, service, started_at, source)
                 VALUES (1, 'a.timer', 'a.service', ?1, 'observed')",
                [ts],
            )
            .unwrap();
        }
    }
    let m = s.maintain(NOW).await.unwrap();
    assert_eq!((m.raw_deleted, m.rollups_deleted), (1, 1));
    assert_eq!(count(&s, "SELECT min(ts) FROM points_raw"), cutoff);
    let min5m = NOW - rollup_ret + 300;
    assert_eq!(count(&s, "SELECT min(ts) FROM points_5m"), min5m);
    assert_eq!(count(&s, "SELECT min(ts) FROM events"), min5m);
    assert_eq!(count(&s, "SELECT min(started_at) FROM timer_runs"), min5m);
    assert!(m.optimized, "the first pass optimizes");
    assert!(
        !s.maintain(NOW + 300).await.unwrap().optimized,
        "then once a day"
    );
    assert!(s.maintain(NOW + DAY).await.unwrap().optimized);
}

/// One backfilled mock database shared by the query tests.
async fn backfilled() -> (TempDir, Storage, usize) {
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    let mock = MockCollector::with_boot_time(7, NOW - 9 * DAY);
    let n = backfill_mock(&s, &mock, NOW, 10).unwrap().unwrap();
    (dir, s, n)
}

#[tokio::test]
async fn mock_backfill_fills_seven_days_once_and_quickly() {
    let started = Instant::now();
    let (_dir, s, n) = backfilled().await;
    let elapsed = started.elapsed();
    // The 5s target is for `make dev`; leave headroom for slow CI machines.
    assert!(
        elapsed < Duration::from_secs(20),
        "backfill took {elapsed:?}"
    );
    assert!(n > 7 * 8640 * 10, "{n} samples");

    // Raw only within raw retention (bucket-aligned), 5m rollups for the whole week.
    let oldest_raw = count(&s, "SELECT min(ts) FROM points_raw");
    assert!((NOW - 48 * HOUR..NOW - 48 * HOUR + 300).contains(&oldest_raw));
    let oldest_5m = count(&s, "SELECT min(ts) FROM points_5m");
    assert!(oldest_5m <= NOW - 7 * DAY + 300, "{oldest_5m}");
    let r = rollups(&s, "mem.used");
    // Contiguous buckets from a week ago up to the last completed one, with full counts.
    assert_eq!(
        r.len() as i64,
        (writer::bucket_start(NOW - 60) - r[0].0) / 300
    );
    assert!(r.windows(2).all(|w| w[1].0 - w[0].0 == 300));
    // The first bucket is partial: a week before NOW is not bucket-aligned.
    assert!(r[1..].iter().all(|b| b.4 == 30), "{r:?}");
    assert!(r.iter().all(|b| b.2 <= b.1 && b.1 <= b.3));

    // A second run (the next start) leaves the data alone.
    let mock = MockCollector::with_boot_time(7, NOW - 9 * DAY);
    assert_eq!(backfill_mock(&s, &mock, NOW + 60, 10).unwrap(), None);
}

#[tokio::test]
async fn query_resolution_buckets_and_keys() {
    let (_dir, s, _) = backfilled().await;

    let (plan, series) = s
        .query(req(&["cpu.usage"], NOW - HOUR, NOW, 300), NOW, iv())
        .await
        .unwrap();
    assert_eq!((plan.resolution, plan.step), (Resolution::Raw, 12));
    let pts = &series[0].points;
    assert_eq!(pts.len(), 300);
    assert_eq!(pts[0].0, NOW - HOUR);
    assert!(pts.windows(2).all(|w| w[1].0 - w[0].0 == 12));
    // Points are 10s apart, so every 12s bucket holds one or two.
    for p in pts {
        let (avg, min, max) = (p.1.unwrap(), p.2.unwrap(), p.3.unwrap());
        assert!(min <= avg && avg <= max, "{p:?}");
    }

    let (plan, series) = s
        .query(
            req(&["mem.used", "disk.used:*"], NOW - 7 * DAY, NOW, 300),
            NOW,
            iv(),
        )
        .await
        .unwrap();
    assert_eq!(plan.resolution, Resolution::FiveMinutes);
    let keys: Vec<&str> = series.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["mem.used", "disk.used:/", "disk.used:/srv"]);
    for sd in &series {
        assert!(sd.points.len() <= 300);
        // The whole week has data except possibly the not-yet-rolled-up tail.
        let filled = sd.points.iter().filter(|p| p.1.is_some()).count();
        assert!(filled >= sd.points.len() - 1, "{} {filled}", sd.key);
    }

    // Unknown exact keys are errors; a glob without matches is just empty.
    let err = s
        .query(req(&["cpu.usage", "nope"], NOW - HOUR, NOW, 300), NOW, iv())
        .await
        .unwrap_err();
    assert!(
        matches!(err, QueryError::UnknownSeries(ref k) if k == "nope"),
        "{err:?}"
    );
    let (_, series) = s
        .query(req(&["nope:*"], NOW - HOUR, NOW, 300), NOW, iv())
        .await
        .unwrap();
    assert!(series.is_empty());

    let list = s.series().await.unwrap();
    let disk = list.iter().find(|s| s.key == "disk.used:/srv").unwrap();
    assert_eq!(
        (disk.metric.as_str(), disk.label.as_deref()),
        ("disk.used", Some("/srv"))
    );
    let cpu = list.iter().find(|s| s.key == "cpu.usage").unwrap();
    assert_eq!(
        (cpu.metric.as_str(), cpu.label.as_deref(), cpu.unit.as_str()),
        ("cpu.usage", None, "percent")
    );
}

#[tokio::test]
async fn bucket_values_are_exact() {
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    let from = NOW - HOUR;
    // 12s buckets: offsets 0 and 10 share bucket 0, offset 20 is bucket 1, bucket 2 is empty.
    s.append(vec![
        sample("x", from, 1.0),
        sample("x", from + 10, 3.0),
        sample("x", from + 20, 8.0),
        // `to` is inclusive and lands in the last bucket.
        sample("x", NOW, 4.0),
    ])
    .await
    .unwrap();
    let (_, series) = s
        .query(req(&["x"], from, NOW, 300), NOW, iv())
        .await
        .unwrap();
    let p = &series[0].points;
    assert_eq!(p[0], (from, Some(2.0), Some(1.0), Some(3.0)));
    assert_eq!(p[1], (from + 12, Some(8.0), Some(8.0), Some(8.0)));
    assert_eq!(p[2], (from + 24, None, None, None));
    assert_eq!(p[299], (from + 299 * 12, Some(4.0), Some(4.0), Some(4.0)));

    // The 5m tier weights each rollup by its count.
    let old = writer::bucket_start(NOW - 10 * DAY);
    {
        let w = s.inner.writer.lock().unwrap();
        let c = w.conn();
        for (ts, avg, min, max, n) in [(old, 10, 5, 20, 3), (old + 300, 2, 0, 4, 1)] {
            c.execute(
                "INSERT INTO points_5m VALUES ((SELECT id FROM series), ?1, ?2, ?3, ?4, ?5)",
                [ts, avg, min, max, n],
            )
            .unwrap();
        }
    }
    let (plan, series) = s
        .query(req(&["x"], old, old + HOUR, 10), NOW, iv())
        .await
        .unwrap();
    assert_eq!((plan.resolution, plan.step), (Resolution::FiveMinutes, 600));
    assert_eq!(series[0].points[0], (old, Some(8.0), Some(0.0), Some(20.0)));
    assert_eq!(series[0].points[1], (old + 600, None, None, None));
}

/// ROADMAP M2: 30 series at 10s for 400 days must stay under 500 MB. Writes a sample of each tier
/// in the real write pattern (all series interleaved per timestamp, non-integral values), measures
/// bytes per row and extrapolates to the default retention: 48h raw plus 400d of 5m rollups.
#[tokio::test]
async fn database_growth_estimate_is_under_500_mb() {
    const SERIES: i64 = 30;
    const SAMPLE_TS: i64 = 2000;
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    let size = |s: &Storage| {
        count(
            s,
            "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
        )
    };

    let base = size(&s);
    for t in 0..SAMPLE_TS {
        let batch = (0..SERIES)
            .map(|i| {
                sample(
                    &format!("series.{i}:label"),
                    NOW + t * 10,
                    (t * i) as f64 * 0.37 + 0.123,
                )
            })
            .collect();
        s.append(batch).await.unwrap();
    }
    let raw_bytes = size(&s) - base;

    let base = size(&s);
    {
        let w = s.inner.writer.lock().unwrap();
        let c = w.conn();
        c.execute_batch("BEGIN").unwrap();
        for t in 0..SAMPLE_TS {
            for id in 1..=SERIES {
                let v = (t * id) as f64 * 0.37 + 0.123;
                c.execute(
                    "INSERT INTO points_5m VALUES (?1, ?2, ?3, ?4, ?5, 30)",
                    rusqlite::params![id, NOW + t * 300, v, v - 1.1, v + 1.7],
                )
                .unwrap();
            }
        }
        c.execute_batch("COMMIT").unwrap();
    }
    let rollup_bytes = size(&s) - base;

    let rows = (SERIES * SAMPLE_TS) as f64;
    let per_raw = raw_bytes as f64 / rows;
    let per_rollup = rollup_bytes as f64 / rows;
    let raw_rows = (SERIES * 48 * 360) as f64;
    let rollup_rows = (SERIES * 400 * 288) as f64;
    let estimate_mb = (per_raw * raw_rows + per_rollup * rollup_rows) / 1e6;
    println!(
        "{per_raw:.1} B/raw row, {per_rollup:.1} B/5m row -> {estimate_mb:.0} MB for 30 series, \
         48h raw + 400d 5m"
    );
    assert!(estimate_mb < 500.0, "{estimate_mb} MB");
}

/// A series that stops reporting keeps an exact last rollup while retention eats its raw rows,
/// whatever the phase of the maintenance ticks.
#[tokio::test]
async fn retention_never_corrupts_a_stopped_series_last_rollup() {
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    let t = writer::bucket_start(NOW);
    s.append(
        (0..30)
            .map(|i| sample("gone", t + i * 10, f64::from(i as i32 + 1)))
            .collect(),
    )
    .await
    .unwrap();
    // Unaligned phase, 5 min apart, until well past retention of the whole bucket.
    let mut now = t + 300 + 77;
    while now <= t + 48 * HOUR + 1200 {
        s.maintain(now).await.unwrap();
        assert_eq!(
            rollups(&s, "gone"),
            [(t, 15.5, 1.0, 30.0, 30)],
            "at {}",
            now - t
        );
        now += 300;
    }
    assert_eq!(count(&s, "SELECT count(*) FROM points_raw"), 0);
}

#[tokio::test]
async fn series_without_points_are_pruned_and_can_return() {
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    s.append(vec![
        sample("old", NOW - 3 * DAY, 1.0),
        sample("new", NOW - 10, 2.0),
    ])
    .await
    .unwrap();
    // The old point is rolled up, then its raw row expires; the rollup keeps the series.
    s.maintain(NOW).await.unwrap();
    assert_eq!(s.series().await.unwrap().len(), 2);
    // Once the rollup expires too, the series goes.
    let m = s.maintain(NOW + 400 * DAY).await.unwrap();
    assert_eq!(m.series_deleted, 2);
    assert!(s.series().await.unwrap().is_empty());
    // The writer's id cache was cleared, so writing the key again recreates it.
    s.append(vec![sample("old", NOW + 400 * DAY, 3.0)])
        .await
        .unwrap();
    let keys: Vec<String> = s
        .series()
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.key)
        .collect();
    assert_eq!(keys, ["old"]);
    assert_eq!(count(&s, "SELECT count(*) FROM points_raw"), 1);
}

/// 5m-tier queries include raw points not rolled up yet, weighted like rollups of count 1.
#[tokio::test]
async fn five_minute_queries_include_the_unrolled_tail() {
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    let open_bucket = writer::bucket_start(NOW);
    let prev = open_bucket - 300;
    s.append(vec![
        sample("x", prev - 300, 10.0),
        sample("x", prev, 1.0),
        sample("x", prev + 10, 3.0),
        sample("x", open_bucket + 5, 8.0),
    ])
    .await
    .unwrap();
    // Maintenance just after `prev` ended, inside the grace: only `prev - 300` is rolled up.
    s.maintain(open_bucket + 30).await.unwrap();
    assert_eq!(rollups(&s, "x").len(), 1);

    let from = open_bucket - DAY;
    let (plan, series) = s
        .query(req(&["x"], from, NOW, 300), NOW, iv())
        .await
        .unwrap();
    assert_eq!((plan.resolution, plan.step), (Resolution::FiveMinutes, 300));
    let p = &series[0].points;
    // The grid is aligned with the buckets here: from is a bucket start.
    let at = |ts: i64| p[usize::try_from((ts - from) / 300).unwrap()];
    assert_eq!(
        at(prev - 300),
        (prev - 300, Some(10.0), Some(10.0), Some(10.0))
    );
    assert_eq!(at(prev), (prev, Some(2.0), Some(1.0), Some(3.0)));
    assert_eq!(
        *p.last().unwrap(),
        (open_bucket, Some(8.0), Some(8.0), Some(8.0))
    );

    // After the rollup catches up, the answer is the same and nothing is counted twice.
    s.maintain(open_bucket + 90).await.unwrap();
    let (_, again) = s
        .query(req(&["x"], from, NOW, 300), NOW, iv())
        .await
        .unwrap();
    assert_eq!(again[0].points, *p);
}

#[tokio::test]
async fn step_floor_follows_the_slowest_queried_series() {
    let dir = tempfile::tempdir().unwrap();
    let s = open(&dir);
    s.append(vec![
        sample("cpu.usage", NOW - 10, 1.0),
        sample("service.memory:a.service", NOW - 30, 2.0),
        sample("service.restarts:a.service", NOW - 30, 0.0),
        sample("check.latency:web", NOW - 60, 3.0),
    ])
    .await
    .unwrap();
    let step = |keys: &'static [&'static str]| {
        let s = s.clone();
        async move {
            s.query(req(keys, NOW - HOUR, NOW, 1000), NOW, iv())
                .await
                .unwrap()
                .0
                .step
        }
    };
    assert_eq!(step(&["cpu.usage"]).await, 10);
    assert_eq!(step(&["cpu.usage", "service.memory:*"]).await, 30);
    assert_eq!(step(&["cpu.usage", "service.restarts:a.service"]).await, 30);
    assert_eq!(step(&["check.latency:web"]).await, 60);
}
