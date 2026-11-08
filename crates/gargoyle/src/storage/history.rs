//! The `events` and `timer_runs` tables (ARCHITECTURE §5): health transitions and timer runs.

use gargoyle_core::api::{Event, LastRun, RunSource, TargetKind, TimerRun};
use gargoyle_core::health::{Health, TimerRuns, Transition, UnknownHealth};
use gargoyle_core::model::TimerRunRecord;
use rusqlite::{Connection, Row, params};

/// Newest-first event selection. `since` is inclusive, `until` exclusive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventFilter {
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub kind: Option<TargetKind>,
    pub target: Option<String>,
    pub limit: i64,
}

pub fn insert_events(
    conn: &Connection,
    host_id: i64,
    ts: i64,
    events: &[Transition],
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO events (host_id, ts, target_kind, target, from_state, to_state, message)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for e in events {
        stmt.execute(params![
            host_id,
            ts,
            kind_str(e.kind),
            e.target,
            e.from.map(Health::as_str),
            e.to.as_str(),
            e.message,
        ])?;
    }
    Ok(())
}

/// Inserts runs, or completes the stored row of the same run: the same service and
/// `InvocationID`, or else the same service and start.
///
/// A value already stored is never overwritten, only missing ones are filled in: a run's facts do
/// not change once known, and this keeps the first source (`observed` or `journal`) and its start
/// time, and makes a repeated insert, e.g. after a restart, a no-op. Matching on the invocation
/// first matters because the journal's start can differ from systemd's by about a second.
pub fn upsert_runs(
    conn: &Connection,
    host_id: i64,
    runs: &[TimerRunRecord],
    source: RunSource,
) -> rusqlite::Result<()> {
    let mut by_invocation = conn.prepare_cached(
        "UPDATE timer_runs SET
           finished_at = coalesce(finished_at, ?4),
           result = coalesce(result, ?5),
           exit_status = coalesce(exit_status, ?6)
         WHERE host_id = ?1 AND service = ?2 AND invocation_id = ?3",
    )?;
    let mut insert = conn.prepare_cached(
        "INSERT INTO timer_runs (host_id, timer, service, invocation_id, started_at, finished_at,
                                 result, exit_status, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (host_id, service, started_at) DO UPDATE SET
           invocation_id = coalesce(invocation_id, excluded.invocation_id),
           finished_at = coalesce(finished_at, excluded.finished_at),
           result = coalesce(result, excluded.result),
           exit_status = coalesce(exit_status, excluded.exit_status)",
    )?;
    for r in runs {
        if let Some(id) = &r.invocation_id
            && by_invocation.execute(params![
                host_id,
                r.service,
                id,
                r.finished_at,
                r.result,
                r.exit_status
            ])? > 0
        {
            continue;
        }
        insert.execute(params![
            host_id,
            r.timer,
            r.service,
            r.invocation_id,
            r.started_at,
            r.finished_at,
            r.result,
            r.exit_status,
            source_str(source),
        ])?;
    }
    Ok(())
}

/// Events matching `filter`, newest first.
///
/// Pages hold whole seconds only, so that `until = <oldest ts returned>` (exclusive) is a
/// gap-free cursor even though one tick logs all its transitions with the same `ts`:
/// - if the events after the first `limit` continue the last second on the page, that second is
///   left for the next page, so the page can be shorter than `limit`;
/// - if the first `limit` events are all one second, the whole second is returned, so the page
///   can be longer than `limit` (bounded by the number of targets).
///
/// A client has seen everything when a page comes back empty.
pub fn events(
    conn: &Connection,
    host_id: i64,
    filter: &EventFilter,
) -> rusqlite::Result<Vec<Event>> {
    let limit = usize::try_from(filter.limit).unwrap_or(0);
    let mut rows = select_events(
        conn,
        host_id,
        filter,
        filter.since,
        filter.until,
        filter.limit.saturating_add(1),
    )?;
    if rows.len() > limit {
        let split_ts = rows[limit].ts;
        if rows.first().is_some_and(|e| e.ts != split_ts) {
            rows.truncate(limit);
            rows.retain(|e| e.ts != split_ts);
        } else {
            // `split_ts` is within [since, until), so this stays inside the filter. LIMIT -1 is
            // SQLite for "no limit".
            rows = select_events(
                conn,
                host_id,
                filter,
                Some(split_ts),
                Some(split_ts + 1),
                -1,
            )?;
        }
    }
    Ok(rows)
}

fn select_events(
    conn: &Connection,
    host_id: i64,
    filter: &EventFilter,
    since: Option<i64>,
    until: Option<i64>,
    limit: i64,
) -> rusqlite::Result<Vec<Event>> {
    conn.prepare_cached(
        "SELECT id, ts, target_kind, target, from_state, to_state, message FROM events
         WHERE host_id = ?1
           AND (?2 IS NULL OR ts >= ?2) AND (?3 IS NULL OR ts < ?3)
           AND (?4 IS NULL OR target_kind = ?4) AND (?5 IS NULL OR target = ?5)
         ORDER BY ts DESC, id DESC LIMIT ?6",
    )?
    .query_map(
        params![
            host_id,
            since,
            until,
            filter.kind.map(kind_str),
            filter.target,
            limit,
        ],
        event_row,
    )?
    .collect()
}

/// The last logged state of every target, to restore transition baselines at startup.
pub fn last_states(
    conn: &Connection,
    host_id: i64,
) -> rusqlite::Result<Vec<(TargetKind, String, Health)>> {
    // SQLite takes the bare columns from the row holding max(id).
    let mut stmt = conn.prepare_cached(
        "SELECT target_kind, target, to_state, max(id) FROM events WHERE host_id = ?1
         GROUP BY target_kind, target",
    )?;
    let rows = stmt.query_map([host_id], |r| {
        Ok((
            parse_kind(r, 0)?,
            r.get::<_, String>(1)?,
            parse_health(r, 2)?,
        ))
    })?;
    rows.collect()
}

/// The newest `limit` runs of `timer`, newest first.
pub fn timer_runs(
    conn: &Connection,
    host_id: i64,
    timer: &str,
    limit: i64,
) -> rusqlite::Result<Vec<TimerRun>> {
    conn.prepare_cached(
        "SELECT started_at, finished_at, result, exit_status, source FROM timer_runs
         WHERE host_id = ?1 AND timer = ?2 ORDER BY started_at DESC LIMIT ?3",
    )?
    .query_map(params![host_id, timer, limit], |r| {
        let started_at: i64 = r.get(0)?;
        let finished_at: Option<i64> = r.get(1)?;
        Ok(TimerRun {
            started_at,
            finished_at,
            duration: finished_at.and_then(|f| u64::try_from(f - started_at).ok()),
            result: r.get(2)?,
            exit_status: r.get(3)?,
            source: match r.get_ref(4)?.as_str()? {
                "journal" => RunSource::Journal,
                _ => RunSource::Observed,
            },
        })
    })?
    .collect()
}

/// What storage knows about `timer`'s runs: the newest one and the end of the newest success.
pub fn timer_summary(conn: &Connection, host_id: i64, timer: &str) -> rusqlite::Result<TimerRuns> {
    let last = timer_runs(conn, host_id, timer, 1)?
        .into_iter()
        .next()
        .map(|r| LastRun {
            started_at: r.started_at,
            finished_at: r.finished_at,
            duration: r.duration,
            result: r.result,
            exit_status: r.exit_status,
        });
    let last_success = conn
        .prepare_cached(
            "SELECT max(coalesce(finished_at, started_at)) FROM timer_runs
             WHERE host_id = ?1 AND timer = ?2 AND result = 'success'",
        )?
        .query_row(params![host_id, timer], |r| r.get(0))?;
    Ok(TimerRuns { last, last_success })
}

fn event_row(r: &Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get(0)?,
        ts: r.get(1)?,
        target_kind: parse_kind(r, 2)?,
        target: r.get(3)?,
        from: match r.get_ref(4)?.as_str_or_null()? {
            None => None,
            Some(_) => Some(parse_health(r, 4)?),
        },
        to: parse_health(r, 5)?,
        message: r.get(6)?,
    })
}

pub fn kind_str(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::System => "system",
        TargetKind::Service => "service",
        TargetKind::Timer => "timer",
        TargetKind::Check => "check",
        TargetKind::Library => "library",
    }
}

fn source_str(source: RunSource) -> &'static str {
    match source {
        RunSource::Observed => "observed",
        RunSource::Journal => "journal",
    }
}

fn parse_kind(r: &Row<'_>, idx: usize) -> rusqlite::Result<TargetKind> {
    match r.get_ref(idx)?.as_str()? {
        "system" => Ok(TargetKind::System),
        "service" => Ok(TargetKind::Service),
        "timer" => Ok(TargetKind::Timer),
        "check" => Ok(TargetKind::Check),
        "library" => Ok(TargetKind::Library),
        other => Err(invalid(idx, format!("unknown target kind {other:?}"))),
    }
}

fn parse_health(r: &Row<'_>, idx: usize) -> rusqlite::Result<Health> {
    r.get_ref(idx)?
        .as_str()?
        .parse()
        .map_err(|e: UnknownHealth| invalid(idx, e.to_string()))
}

fn invalid(idx: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> (tempfile::TempDir, Connection, i64) {
        let dir = tempfile::tempdir().unwrap();
        let conn = super::super::schema::open_writer(&dir.path().join("t.db")).unwrap();
        let host: i64 = conn
            .query_row(
                "INSERT INTO hosts (name) VALUES ('local') RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        (dir, conn, host)
    }

    fn rec(id: Option<&str>, start: i64, end: Option<i64>, result: Option<&str>) -> TimerRunRecord {
        TimerRunRecord {
            timer: "t.timer".into(),
            service: "t.service".into(),
            invocation_id: id.map(Into::into),
            started_at: start,
            finished_at: end,
            result: result.map(Into::into),
            exit_status: result.map(|r| i32::from(r != "success")),
        }
    }

    fn runs(conn: &Connection, host: i64) -> Vec<TimerRun> {
        let mut r = timer_runs(conn, host, "t.timer", 100).unwrap();
        r.reverse();
        r
    }

    #[test]
    fn same_invocation_is_one_row_even_if_starts_differ() {
        let (_d, conn, host) = conn();
        // Observed while running, then the journal reports it a second earlier, finished.
        upsert_runs(
            &conn,
            host,
            &[rec(Some("a"), 100, None, None)],
            RunSource::Observed,
        )
        .unwrap();
        upsert_runs(
            &conn,
            host,
            &[rec(Some("a"), 99, Some(130), Some("success"))],
            RunSource::Journal,
        )
        .unwrap();
        let r = runs(&conn, host);
        assert_eq!(r.len(), 1);
        assert_eq!(
            (
                r[0].started_at,
                r[0].finished_at,
                r[0].duration,
                r[0].result.as_deref(),
                r[0].source
            ),
            (
                100,
                Some(130),
                Some(30),
                Some("success"),
                RunSource::Observed
            )
        );
        // And the other way round: journal first, then observed with a shifted start.
        upsert_runs(
            &conn,
            host,
            &[rec(Some("b"), 200, Some(210), Some("exit-code"))],
            RunSource::Journal,
        )
        .unwrap();
        upsert_runs(
            &conn,
            host,
            &[rec(Some("b"), 201, Some(211), Some("success"))],
            RunSource::Observed,
        )
        .unwrap();
        let r = runs(&conn, host);
        assert_eq!(r.len(), 2);
        // Known values are kept, not overwritten.
        assert_eq!(
            (r[1].started_at, r[1].result.as_deref(), r[1].source),
            (200, Some("exit-code"), RunSource::Journal)
        );
    }

    #[test]
    fn same_start_without_invocation_is_one_row() {
        let (_d, conn, host) = conn();
        upsert_runs(
            &conn,
            host,
            &[rec(None, 100, None, None)],
            RunSource::Observed,
        )
        .unwrap();
        upsert_runs(
            &conn,
            host,
            &[rec(Some("a"), 100, Some(120), Some("success"))],
            RunSource::Journal,
        )
        .unwrap();
        // Repeating is a no-op.
        upsert_runs(
            &conn,
            host,
            &[rec(Some("a"), 100, Some(120), Some("success"))],
            RunSource::Observed,
        )
        .unwrap();
        let r = runs(&conn, host);
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].finished_at, r[0].source),
            (Some(120), RunSource::Observed)
        );
        let summary = timer_summary(&conn, host, "t.timer").unwrap();
        assert_eq!(summary.last_success, Some(120));
        assert_eq!(summary.last.map(|l| l.duration), Some(Some(20)));
    }

    #[test]
    fn last_states_are_the_newest_per_target() {
        let (_d, conn, host) = conn();
        let t = |kind, target: &str, from, to| Transition {
            kind,
            target: target.into(),
            from,
            to,
            message: None,
        };
        insert_events(
            &conn,
            host,
            10,
            &[
                t(TargetKind::Service, "a.service", None, Health::Critical),
                t(TargetKind::System, "a.service", None, Health::Warning),
            ],
        )
        .unwrap();
        insert_events(
            &conn,
            host,
            20,
            &[t(
                TargetKind::Service,
                "a.service",
                Some(Health::Critical),
                Health::Healthy,
            )],
        )
        .unwrap();
        let mut states = last_states(&conn, host).unwrap();
        states.sort_by_key(|s| kind_str(s.0));
        assert_eq!(
            states,
            [
                (TargetKind::Service, "a.service".into(), Health::Healthy),
                (TargetKind::System, "a.service".into(), Health::Warning),
            ]
        );
        let e = events(
            &conn,
            host,
            &EventFilter {
                kind: Some(TargetKind::Service),
                limit: 10,
                ..EventFilter::default()
            },
        )
        .unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[0].from, e[0].to),
            (Some(Health::Critical), Health::Healthy)
        );
        assert_eq!(e[1].from, None);
    }
}
