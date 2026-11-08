# 0002 — Embedded SQLite via rusqlite, two-tier retention

**Status:** Accepted · 2026-11-08

## Context

Gargoyle has to store a few dozen time series at 10s resolution, plus events and timer runs. The
requirement is no external database (no Postgres, Prometheus or Influx) and a single file that is
easy to back up.

## Decision

- **SQLite** in WAL mode, accessed with **`rusqlite`** (`bundled` feature, so it does not depend on
  the system libsqlite version). Calls are wrapped in `tokio::task::spawn_blocking`. One writer
  connection is held behind a `Mutex`, with one read connection.
- Migrations are plain SQL strings embedded in the binary and versioned with `PRAGMA user_version`.
- Retention has two tiers: raw points for 48h, and 5-minute avg/min/max rollups for 400d. Queries
  re-bucket in SQL to at most N points.

**Alternatives considered**

| Option | Why not |
|---|---|
| `sqlx` | Async SQLite in sqlx still uses a blocking worker internally. Its compile-time-checked macros need a DB at build time or an offline cache. It brings more machinery for no gain at this scale. |
| `redb`/`sled`/custom files | We would have to write our own time-bucketing and aggregation. SQL `GROUP BY` does it for free and is inspectable with the `sqlite3` CLI. |
| Embedded TSDB crates | Immature or heavy. Our volume, about 3 points/s, is trivial for SQLite. |
| Round-robin (RRD-style) only | Fixed-size, but it loses flexibility (re-bucketing, adding series) and the ability to query events. |

## Consequences

- A single file at `/var/lib/gargoyle/gargoyle.db`. Back it up with `sqlite3 .backup`.
- The DB is estimated to stay well under 500 MB at the default retention.
- Concurrency is trivially safe, and there is no pool to tune.
