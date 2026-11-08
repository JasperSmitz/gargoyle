//! History queries for `GET /api/v1/metrics` and `GET /api/v1/metrics/query` (ARCHITECTURE §5,
//! §6): tier selection and re-bucketing onto a fixed grid of at most `points` buckets.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use gargoyle_core::api::{Point, Resolution, SeriesData, SeriesInfo};
use gargoyle_core::config::Config;
use rusqlite::{Connection, params};

use super::writer::BUCKET_S;

pub const MAX_KEYS: usize = 12;
pub const DEFAULT_POINTS: i64 = 300;
pub const MIN_POINTS: i64 = 10;
pub const MAX_POINTS: i64 = 1000;
const DEFAULT_RANGE_S: i64 = 3600;
/// Longest range served from `points_raw`; beyond it the 5m tier has more than enough detail.
const RAW_MAX_RANGE_S: i64 = 6 * 3600;
/// Bounds on `from`/`to`. They keep all grid arithmetic far from overflow; a client clock that is
/// somewhat ahead still works.
const MAX_FUTURE_S: i64 = 86_400;
const MAX_RANGE_S: i64 = 10 * 366 * 86_400;
/// Default check interval (§7), for the stored latency of a check no longer in the config.
const DEFAULT_CHECK_INTERVAL_S: i64 = 60;

/// A caller error, answered with 400.
#[derive(Debug, PartialEq, Eq)]
pub struct BadRequest(pub String);

impl fmt::Display for BadRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A validated `/metrics/query` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRequest {
    /// As given, each either exact or ending in a `*` prefix glob.
    pub keys: Vec<String>,
    pub from: i64,
    pub to: i64,
    pub points: i64,
}

impl QueryRequest {
    /// Parses the decoded query-string pairs. Unknown parameters are ignored; `from`, `to` and
    /// `points` may appear at most once.
    pub fn parse(pairs: &[(String, String)], now: i64) -> Result<Self, BadRequest> {
        let bad = |m: String| Err(BadRequest(m));
        let mut keys: Vec<String> = Vec::new();
        let (mut from, mut to, mut points) = (None, None, None);
        for (name, value) in pairs {
            let slot = match name.as_str() {
                "key" => {
                    if value.is_empty() {
                        return bad("key must not be empty".into());
                    }
                    if !keys.contains(value) {
                        keys.push(value.clone());
                    }
                    continue;
                }
                "from" => &mut from,
                "to" => &mut to,
                "points" => &mut points,
                _ => continue,
            };
            let Ok(n) = value.parse::<i64>() else {
                return bad(format!("{name} must be an integer"));
            };
            if slot.replace(n).is_some() {
                return bad(format!("{name} given more than once"));
            }
        }
        if keys.is_empty() {
            return bad("at least one key is required".into());
        }
        if keys.len() > MAX_KEYS {
            return bad(format!("at most {MAX_KEYS} keys are allowed"));
        }
        let to = to.unwrap_or(now);
        let from = from.unwrap_or(to.saturating_sub(DEFAULT_RANGE_S));
        if from >= to {
            return bad("from must be before to".into());
        }
        if from < 0 {
            return bad("from must not be negative".into());
        }
        if to > now.saturating_add(MAX_FUTURE_S) {
            return bad("to is too far in the future".into());
        }
        // Cannot overflow: 0 <= from < to.
        if to - from > MAX_RANGE_S {
            return bad("range is longer than 10 years".into());
        }
        let points = points
            .unwrap_or(DEFAULT_POINTS)
            .clamp(MIN_POINTS, MAX_POINTS);
        Ok(Self {
            keys,
            from,
            to,
            points,
        })
    }
}

/// Which tier to read and how wide the buckets are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub resolution: Resolution,
    /// Bucket width, seconds.
    pub step: i64,
    /// Number of buckets: the grid is `from + i * step` for `i in 0..buckets`.
    pub buckets: i64,
}

/// How often each series is written, in seconds, which is the finest step worth asking for.
#[derive(Debug, Clone)]
pub struct NativeIntervals {
    pub system_s: i64,
    pub units_s: i64,
    pub checks_s: HashMap<String, i64>,
}

impl NativeIntervals {
    pub fn from_config(config: &Config) -> Self {
        Self {
            system_s: secs(config.collector.system_interval),
            units_s: secs(config.collector.units_interval),
            checks_s: config
                .checks
                .iter()
                .map(|c| (c.name.clone(), secs(c.interval)))
                .collect(),
        }
    }

    /// Derived from the key's metric, which fixes the tick that writes it.
    pub fn for_key(&self, key: &str) -> i64 {
        if key.starts_with("service.memory:") || key.starts_with("service.restarts:") {
            self.units_s
        } else if let Some(name) = key.strip_prefix("check.latency:") {
            self.checks_s
                .get(name)
                .copied()
                .unwrap_or(DEFAULT_CHECK_INTERVAL_S)
        } else {
            self.system_s
        }
    }
}

fn secs(d: Duration) -> i64 {
    i64::try_from(d.as_secs()).unwrap_or(i64::MAX)
}

/// Raw points are used when `from` is still within raw retention and the range is at most 6h.
/// Steps never go below the native resolution (`native_s`, the slowest write interval among the
/// queried series, or 5 minutes), which would only produce empty buckets between real points;
/// 5m steps are whole multiples of 5m so every bucket aggregates the same number of rollups.
///
/// Arithmetic saturates and `buckets` is clamped to `points`, so even unvalidated extremes can
/// neither overflow nor allocate a huge grid.
pub fn plan(req: &QueryRequest, now: i64, raw_retention_s: i64, native_s: i64) -> Plan {
    let points = req.points.max(1);
    let range = req.to.saturating_sub(req.from).max(1);
    let wanted = div_ceil(range, points);
    let raw = req.from >= now.saturating_sub(raw_retention_s) && range <= RAW_MAX_RANGE_S;
    let (resolution, step) = if raw {
        (Resolution::Raw, wanted.max(native_s.max(1)))
    } else {
        (
            Resolution::FiveMinutes,
            div_ceil(wanted, BUCKET_S).saturating_mul(BUCKET_S),
        )
    };
    Plan {
        resolution,
        step,
        buckets: div_ceil(range, step).clamp(1, points),
    }
}

/// `ceil(a / b)` for `a >= 0`, `b >= 1`, saturating instead of overflowing.
fn div_ceil(a: i64, b: i64) -> i64 {
    a.saturating_add(b - 1) / b
}

struct SeriesRow {
    id: i64,
    key: String,
    unit: String,
}

fn all_series(conn: &Connection, host_id: i64) -> rusqlite::Result<Vec<SeriesRow>> {
    conn.prepare_cached("SELECT id, key, unit FROM series WHERE host_id = ?1 ORDER BY key")?
        .query_map([host_id], |r| {
            Ok(SeriesRow {
                id: r.get(0)?,
                key: r.get(1)?,
                unit: r.get(2)?,
            })
        })?
        .collect()
}

/// Every stored series, sorted by key.
pub fn list(conn: &Connection, host_id: i64) -> rusqlite::Result<Vec<SeriesInfo>> {
    Ok(all_series(conn, host_id)?
        .into_iter()
        .map(|s| {
            let (metric, label) = match s.key.split_once(':') {
                Some((m, l)) => (m.to_owned(), Some(l.to_owned())),
                None => (s.key.clone(), None),
            };
            SeriesInfo {
                key: s.key,
                metric,
                label,
                unit: s.unit,
            }
        })
        .collect())
}

#[derive(Debug)]
pub enum QueryError {
    /// An exact key that names no stored series (`unknown_series`, 400).
    UnknownSeries(String),
    Internal(anyhow::Error),
}

impl From<rusqlite::Error> for QueryError {
    fn from(e: rusqlite::Error) -> Self {
        QueryError::Internal(anyhow::Error::new(e).context("database query failed"))
    }
}

/// Resolves the keys, plans the grid and reads every matched series on it. Series come in request
/// order; a glob expands to its matches sorted by key (possibly none). An exact key that does not
/// exist is a bad request.
pub fn run(
    conn: &Connection,
    host_id: i64,
    req: &QueryRequest,
    now: i64,
    raw_retention_s: i64,
    intervals: &NativeIntervals,
) -> Result<(Plan, Vec<SeriesData>), QueryError> {
    let all = all_series(conn, host_id)?;
    let mut matched: Vec<&SeriesRow> = Vec::new();
    for key in &req.keys {
        let found: Vec<&SeriesRow> = match key.strip_suffix('*') {
            Some(prefix) => all.iter().filter(|s| s.key.starts_with(prefix)).collect(),
            None => {
                let exact: Vec<_> = all.iter().filter(|s| &s.key == key).collect();
                if exact.is_empty() {
                    return Err(QueryError::UnknownSeries(key.clone()));
                }
                exact
            }
        };
        for s in found {
            if !matched.iter().any(|m| m.id == s.id) {
                matched.push(s);
            }
        }
    }

    let native_s = matched
        .iter()
        .map(|s| intervals.for_key(&s.key))
        .max()
        .unwrap_or(intervals.system_s);
    let plan = plan(req, now, raw_retention_s, native_s);

    // `to` is inclusive so a sample taken in the very second of the request is shown; it falls
    // into the last bucket.
    let sql = match plan.resolution {
        Resolution::Raw => {
            "SELECT min((ts - ?2) / ?3, ?5 - 1) AS b, avg(value), min(value), max(value)
             FROM points_raw WHERE series_id = ?1 AND ts >= ?2 AND ts <= ?4 GROUP BY b"
        }
        // Weighted by count, so a bucket's average equals the average of its raw points. Raw
        // points newer than the newest rollup (not rolled up yet: rollup writes only completed
        // buckets, contiguously) join as count-1 rows, so the right edge of a chart is current.
        Resolution::FiveMinutes => {
            "WITH r AS (
                SELECT ts, avg, min, max, count FROM points_5m
                 WHERE series_id = ?1 AND ts >= ?2 AND ts <= ?4
                UNION ALL
                SELECT ts, value, value, value, 1 FROM points_raw
                 WHERE series_id = ?1 AND ts <= ?4 AND ts >= max(?2, coalesce(
                     (SELECT max(ts) + 300 FROM points_5m WHERE series_id = ?1), ?2))
             )
             SELECT min((ts - ?2) / ?3, ?5 - 1) AS b, sum(avg * count) / sum(count),
                    min(min), max(max)
             FROM r GROUP BY b"
        }
    };
    let mut stmt = conn.prepare_cached(sql)?;
    let mut out = Vec::with_capacity(matched.len());
    for s in matched {
        let mut points: Vec<Point> = (0..plan.buckets)
            .map(|i| (req.from + i * plan.step, None, None, None))
            .collect();
        let rows = stmt.query_map(
            params![s.id, req.from, plan.step, req.to, plan.buckets],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, f64>(3)?,
                ))
            },
        )?;
        for row in rows {
            let (b, avg, min, max) = row?;
            if let Some(p) = usize::try_from(b).ok().and_then(|b| points.get_mut(b)) {
                *p = (p.0, Some(round(avg)), Some(round(min)), Some(round(max)));
            }
        }
        out.push(SeriesData {
            key: s.key.clone(),
            unit: s.unit.clone(),
            points,
        });
    }
    Ok((plan, out))
}

/// Three decimals is below anything a chart can show for any unit we store, and keeps the
/// response small.
fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(q: &[(&str, &str)]) -> Vec<(String, String)> {
        q.iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    fn parse(q: &[(&str, &str)]) -> Result<QueryRequest, BadRequest> {
        QueryRequest::parse(&pairs(q), 10_000)
    }

    #[test]
    fn defaults() {
        let r = parse(&[("key", "cpu.usage")]).unwrap();
        assert_eq!(
            r,
            QueryRequest {
                keys: vec!["cpu.usage".into()],
                from: 6_400,
                to: 10_000,
                points: 300
            }
        );
    }

    #[test]
    fn points_are_clamped_and_keys_deduplicated() {
        let r = parse(&[("key", "a"), ("key", "a"), ("points", "5")]).unwrap();
        assert_eq!((r.keys.len(), r.points), (1, 10));
        assert_eq!(
            parse(&[("key", "a"), ("points", "5000")]).unwrap().points,
            1000
        );
    }

    #[test]
    fn bad_params() {
        let cases: &[&[(&str, &str)]] = &[
            &[],
            &[("key", "")],
            &[("key", "a"), ("from", "x")],
            &[("key", "a"), ("points", "1.5")],
            &[("key", "a"), ("from", "10"), ("to", "10")],
            &[("key", "a"), ("from", "20"), ("to", "10")],
            &[("key", "a"), ("to", "1"), ("to", "2")],
        ];
        for q in cases {
            assert!(parse(q).is_err(), "{q:?}");
        }
        let min = i64::MIN.to_string();
        let max = i64::MAX.to_string();
        let extremes: &[&[(&str, &str)]] = &[
            &[("key", "a"), ("from", &min), ("to", &max)],
            &[("key", "a"), ("from", "0"), ("to", &max)],
            &[("key", "a"), ("from", "0"), ("to", "9223372036854775307")],
            &[("key", "a"), ("from", &min)],
            &[("key", "a"), ("to", &min)],
            &[("key", "a"), ("from", "-1"), ("to", "100")],
            // More than 10 years.
            &[("key", "a"), ("from", "0"), ("to", "400000000")],
            &[("key", "a"), ("points", &max), ("from", &min)],
        ];
        for q in extremes {
            assert!(
                QueryRequest::parse(&pairs(q), 400_000_000).is_err(),
                "{q:?}"
            );
        }
        assert!(parse(&[("key", "a"), ("points", &min)]).is_ok());
        assert!(
            parse(&[("key", "a"), ("to", "96400")]).is_ok(),
            "a day ahead is fine"
        );
        assert!(parse(&[("key", "a"), ("to", "96401")]).is_err());
        let thirteen: Vec<(String, String)> =
            (0..13).map(|i| ("key".into(), format!("k{i}"))).collect();
        assert!(QueryRequest::parse(&thirteen, 10_000).is_err());
        assert!(QueryRequest::parse(&thirteen[..12], 10_000).is_ok());
    }

    fn req(from: i64, to: i64, points: i64) -> QueryRequest {
        QueryRequest {
            keys: vec!["k".into()],
            from,
            to,
            points,
        }
    }

    const NOW: i64 = 1_794_200_000;
    const RAW_RET: i64 = 48 * 3600;

    #[test]
    fn one_hour_is_raw_and_seven_days_is_5m() {
        let p = plan(&req(NOW - 3600, NOW, 300), NOW, RAW_RET, 10);
        assert_eq!(
            p,
            Plan {
                resolution: Resolution::Raw,
                step: 12,
                buckets: 300
            }
        );
        let p = plan(&req(NOW - 7 * 86_400, NOW, 300), NOW, RAW_RET, 10);
        assert_eq!(p.resolution, Resolution::FiveMinutes);
        assert_eq!(p.step % BUCKET_S, 0);
        assert!(p.buckets <= 300, "{p:?}");
    }

    #[test]
    fn tier_boundaries() {
        // Exactly 6h is still raw; a second more is not.
        assert_eq!(
            plan(&req(NOW - 6 * 3600, NOW, 300), NOW, RAW_RET, 10).resolution,
            Resolution::Raw
        );
        assert_eq!(
            plan(&req(NOW - 6 * 3600 - 1, NOW, 300), NOW, RAW_RET, 10).resolution,
            Resolution::FiveMinutes
        );
        // A short range older than raw retention must come from the rollups.
        let old = NOW - RAW_RET - 60;
        assert_eq!(
            plan(&req(old, old + 600, 300), NOW, RAW_RET, 10).resolution,
            Resolution::FiveMinutes
        );
    }

    /// Defence in depth: even inputs `parse` rejects cannot overflow or exceed `points`.
    #[test]
    fn plan_survives_extremes() {
        for (from, to, points) in [
            (0, i64::MAX - 500, 1000),
            (i64::MIN, i64::MAX, 10),
            (0, i64::MAX, 1),
            (i64::MAX - 1, i64::MAX, 1000),
        ] {
            let p = plan(&req(from, to, points), NOW, RAW_RET, 10);
            assert!(p.buckets >= 1 && p.buckets <= points.max(1), "{p:?}");
            assert!(p.step >= 1, "{p:?}");
        }
    }

    #[test]
    fn steps_never_go_below_native_resolution_and_buckets_never_exceed_points() {
        let p = plan(&req(NOW - 3600, NOW, 1000), NOW, RAW_RET, 10);
        assert_eq!((p.step, p.buckets), (10, 360));
        let p = plan(&req(NOW - 600, NOW, 1000), NOW, RAW_RET, 10);
        assert_eq!(p.step, 10);
        for range in [7, 3599, 3601, 6 * 3600, 86_400, 30 * 86_400, 365 * 86_400] {
            for points in [10, 300, 999, 1000] {
                let p = plan(&req(NOW - range, NOW, points), NOW, RAW_RET, 10);
                assert!(
                    p.buckets <= points && p.buckets >= 1,
                    "{range} {points} {p:?}"
                );
                assert!(p.buckets * p.step >= range, "grid covers the range");
            }
        }
    }
}
