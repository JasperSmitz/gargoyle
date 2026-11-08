//! Connection setup and the embedded, numbered migrations (ARCHITECTURE §5).
//!
//! `PRAGMA user_version` holds the number of migrations applied. Each migration runs in its own
//! transaction together with the version bump, so a crash leaves the file at a known version.
//! Migrations are append-only: never edit one that has shipped.

use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

/// How long a statement waits for a lock held by another connection (or a `sqlite3` shell)
/// before failing with `SQLITE_BUSY`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const MIGRATIONS: &[&str] = &[
    // 1: the full §5 schema, including the tables M3 fills (events, timer_runs).
    "CREATE TABLE hosts (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
     CREATE TABLE series (id INTEGER PRIMARY KEY, host_id INTEGER NOT NULL REFERENCES hosts(id),
                          key TEXT NOT NULL, unit TEXT NOT NULL, UNIQUE(host_id, key));
     CREATE TABLE points_raw (series_id INTEGER NOT NULL, ts INTEGER NOT NULL, value REAL NOT NULL,
                              PRIMARY KEY (series_id, ts)) WITHOUT ROWID;
     CREATE TABLE points_5m (series_id INTEGER NOT NULL, ts INTEGER NOT NULL,
                             avg REAL NOT NULL, min REAL NOT NULL, max REAL NOT NULL,
                             count INTEGER NOT NULL,
                             PRIMARY KEY (series_id, ts)) WITHOUT ROWID;
     CREATE TABLE events (id INTEGER PRIMARY KEY, host_id INTEGER NOT NULL, ts INTEGER NOT NULL,
                          target_kind TEXT NOT NULL, target TEXT NOT NULL,
                          from_state TEXT, to_state TEXT NOT NULL, message TEXT);
     CREATE INDEX events_ts ON events(host_id, ts);
     CREATE TABLE timer_runs (id INTEGER PRIMARY KEY, host_id INTEGER NOT NULL, timer TEXT NOT NULL,
                              service TEXT NOT NULL, invocation_id TEXT, started_at INTEGER NOT NULL,
                              finished_at INTEGER, result TEXT, exit_status INTEGER,
                              source TEXT NOT NULL,
                              UNIQUE(host_id, service, started_at));
     CREATE INDEX timer_runs_ts ON timer_runs(host_id, timer, started_at);",
    // 2: one row per invocation. The journal and systemd can disagree on a run's start by about
    // a second, so (service, started_at) alone does not catch the same run from both sources.
    "CREATE UNIQUE INDEX timer_runs_invocation ON timer_runs(host_id, service, invocation_id)
       WHERE invocation_id IS NOT NULL;",
];

/// Opens the writer connection: applies pragmas and pending migrations.
pub fn open_writer(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(path)?;
    configure(&conn)?;
    // WAL is persistent in the file, so only the writer needs to switch it on.
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        tracing::warn!(%mode, "SQLite refused WAL mode; readers may block behind writes");
    }
    migrate(&mut conn)?;
    Ok(conn)
}

/// Opens the read connection. `query_only` turns any accidental write on it into an error.
pub fn open_reader(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure(&conn)?;
    conn.pragma_update(None, "query_only", true)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(())
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current > MIGRATIONS.len() {
        // Written by a newer gargoyle: refuse rather than guess at an unknown schema.
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISMATCH),
            Some(format!(
                "database schema version {current} is newer than this build supports ({})",
                MIGRATIONS.len()
            )),
        ));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i + 1)?;
        tx.commit()?;
        tracing::info!(version = i + 1, "applied database migration");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_once_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let conn = open_writer(&path).unwrap();
        let v: usize = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len());
        drop(conn);
        // Reopening applies nothing and keeps the tables.
        let conn = open_writer(&path).unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            tables,
            [
                "events",
                "hosts",
                "points_5m",
                "points_raw",
                "series",
                "timer_runs"
            ]
        );
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let fk: bool = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert!(fk);
    }

    #[test]
    fn upgrades_a_populated_v1_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute_batch(
                "INSERT INTO hosts (id, name) VALUES (1, 'local');
                 INSERT INTO series (id, host_id, key, unit) VALUES (1, 1, 'cpu.usage', 'percent');
                 INSERT INTO points_raw VALUES (1, 100, 1.5);
                 INSERT INTO events (host_id, ts, target_kind, target, from_state, to_state)
                   VALUES (1, 100, 'system', 'cpu', NULL, 'warning');
                 INSERT INTO timer_runs (host_id, timer, service, invocation_id, started_at, source)
                   VALUES (1, 't.timer', 't.service', 'a', 100, 'observed'),
                          (1, 't.timer', 't.service', NULL, 200, 'journal'),
                          (1, 't.timer', 't.service', NULL, 300, 'journal');",
            )
            .unwrap();
        }
        let conn = open_writer(&path).unwrap();
        let v: usize = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT count(*) FROM points_raw"), 1);
        assert_eq!(count("SELECT count(*) FROM events"), 1);
        assert_eq!(count("SELECT count(*) FROM timer_runs"), 3);
        // The new index is in force; several NULL invocations stay allowed.
        let dup = conn.execute(
            "INSERT INTO timer_runs (host_id, timer, service, invocation_id, started_at, source)
             VALUES (1, 't.timer', 't.service', 'a', 999, 'journal')",
            [],
        );
        assert!(dup.is_err());
    }

    #[test]
    fn newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let conn = open_writer(&path).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        drop(conn);
        let err = open_writer(&path).unwrap_err().to_string();
        assert!(err.contains("newer"), "{err}");
    }

    #[test]
    fn reader_cannot_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let _w = open_writer(&path).unwrap();
        let r = open_reader(&path).unwrap();
        assert!(
            r.execute("INSERT INTO hosts (name) VALUES ('x')", [])
                .is_err()
        );
    }
}
