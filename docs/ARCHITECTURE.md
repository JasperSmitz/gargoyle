# Gargoyle — Architecture

Gargoyle is a single, self-contained, **read-only** monitoring binary for one Debian server. It
collects system metrics and systemd unit/timer state and stores history in an embedded SQLite file.
It serves a JSON API and an embedded web dashboard. It has no external infrastructure.

Key decisions are recorded in [`docs/decisions/`](decisions/). The API contract is in this document
(§6) and in example payloads under [`docs/api/`](api/).

## 1. Components

```
                 ┌──────────────────────────── gargoyle (one process) ───────────────────────────┐
 /proc, /sys ──▶ │ Collector (trait)           Scheduler (tokio tasks)        HTTP (axum)        │
 systemctl   ──▶ │  ├ LinuxCollector  ──────▶  ├ sample system  ─┐            ├ /api/v1/*  (GET) │ ──▶ browser
 journalctl  ──▶ │  └ MockCollector            ├ poll units/timers├─▶ State ──▶├ auth middleware  │
 HTTP/TCP    ──▶ │ Checks (async probes) ────▶ └ run checks     ─┘   (RwLock)  └ embedded UI      │
                 │                                    │                ▲                           │
                 │                                    ▼                │                           │
                 │                              Storage (SQLite) ──────┘  history queries         │
                 │                              raw → 5m rollups, retention, events, timer runs   │
                 └───────────────────────────────────────────────────────────────────────────────┘
```

| Component | Responsibility |
|---|---|
| **Collector** | Reads raw facts: counters, gauges, unit properties. Does no health logic and no storage. Has two implementations: Linux and Mock. |
| **Checks** | Optional generic HTTP/TCP probes declared in config, such as "GET this URL and expect 200". |
| **Core** | Pure logic with no IO: config model and validation, rate calculation from counter deltas, health evaluation, and API DTOs. |
| **Scheduler** | Runs collection on fixed intervals. Computes metrics and health, updates the in-memory `State`, writes to storage and records health transitions as events. |
| **Storage** | SQLite (WAL). Holds the time series (raw and 5-minute rollups), events and timer runs. Also runs the retention job. |
| **HTTP** | axum. Serves read-only JSON endpoints and the embedded static UI, with auth middleware and security headers. |
| **Web UI** | Svelte 5 + Vite + TypeScript SPA with uPlot charts. It polls the API. The built files are embedded into the binary. |

**Live vs. history.** Current values always come from the in-memory `State`, which is cheap and
always fresh. Charts and event logs come from SQLite.

## 2. Repository / workspace layout

```
Cargo.toml                 # workspace (resolver 3, edition 2024)
crates/
  gargoyle-core/           # pure: config, model, health, rates, api DTOs   (deps: serde, toml, humantime-serde, thiserror)
  gargoyle-collector/      # Collector trait, LinuxCollector, MockCollector, parsers + fixtures
  gargoyle/                # binary: cli, scheduler, storage, checks, http, auth, ui embedding
web/                       # Svelte 5 + Vite + TS frontend (pnpm)
config/
  gargoyle.example.toml    # documented production example
  gargoyle.dev.toml        # mock mode, localhost, ./data/dev.db
docs/                      # ARCHITECTURE, ROADMAP, decisions/, api/, deployment docs (M6)
packaging/                 # systemd unit, cargo-deb metadata (M6)
Makefile                   # canonical build/test/lint entry points (see AGENTS.md)
```

Why three crates: `core` stays pure and fast to test. `collector` is reusable by a future remote
agent (§10). The binary holds everything with IO or framework dependencies. Do not add more crates
unless one of these reasons applies.

### Dependency baseline (2026)

| Purpose | Crate | Notes |
|---|---|---|
| Async runtime / HTTP | `tokio`, `axum`, `tower-http` (compression, headers) | Standard, stable stack |
| Storage | `rusqlite` (`bundled`) | Synchronous; called through `spawn_blocking` ([ADR 0002](decisions/0002-storage-sqlite-rusqlite.md)) |
| Config | `serde`, `toml`, `humantime-serde` | Durations written as `"30s"` or `"48h"` |
| CLI / logging | `clap` (derive), `tracing`, `tracing-subscriber` | Logs go to stderr, which journald captures |
| Errors | `thiserror` (libraries), `anyhow` (binary) | |
| Filesystem stats | `rustix` (`fs` feature, `statvfs`) | Avoids unsafe FFI |
| HTTP checks | `reqwest` 0.12 (`default-features = false`, `rustls-tls`) | Only for `[[check]]` probes |
| UI embedding | `rust-embed` | [ADR 0004](decisions/0004-embed-frontend-single-binary.md) |
| Password hashing | `argon2` | [ADR 0005](decisions/0005-auth-and-exposure.md) |
| Frontend | `svelte@5`, `vite`, `typescript`, `uplot` | [ADR 0001](decisions/0001-rust-backend-svelte-frontend.md) |

Avoid adding dependencies beyond these without a short justification in the PR or task report.
Explicitly not used: `sysinfo` ([ADR 0006](decisions/0006-hand-rolled-procfs-parsing.md)),
`zbus` ([ADR 0003](decisions/0003-systemd-via-systemctl.md)), `sqlx`, any ORM, and any frontend UI
framework or state library.

## 3. Data flow

1. The **system tick** runs every `collector.system_interval` (default 10s). The scheduler calls
   `collector.system_snapshot()` and receives cumulative counters and gauges. `core::rates` diffs the
   result against the previous snapshot and produces a `SystemMetrics` value with percentages and
   rates. `core::health` evaluates it against the thresholds. The scheduler then updates `State`,
   appends one point per series to `points_raw`, and writes an event for every health transition.
2. The **unit tick** runs every `collector.units_interval` (default 30s). The scheduler calls
   `collector.units(&configured)` once, which is one batched `systemctl show`. It then evaluates
   service and timer health. When it sees a new timer invocation, it inserts a `timer_runs` row.
3. The **check ticks** run per check `interval` (default 60s), each check in its own loop whose
   first probe is delayed by 1s plus a random jitter of up to `min(interval, 10s)`. Each check is
   probed with its timeout (which validation keeps within the interval, so probes of one check
   never overlap). Latency is stored as `check.latency:<name>` whenever the probe got an answer
   (an HTTP response, even with the wrong status, or an accepted TCP connection). Health is then
   evaluated and transitions of that check are logged.
4. The **maintenance tick** runs every 5 minutes. It rolls up completed 5-minute buckets from
   `points_raw` into `points_5m`, then deletes rows past retention. Once a day it runs
   `PRAGMA optimize`.
5. **HTTP** handlers read `State` for current data and query storage for history. No handler ever
   causes a collection or any side effect on the host.

If a collection fails, the error is logged and stored on the target as `message`. Targets whose last
successful sample is older than 3× their interval become `unknown`, so data is never silently stale.
The message of a stale target is the last collection error (`collection failed: ...`) or, without
one, `no data for <n>s`. The system summary has no `message` field, so for system targets the
error only reaches event messages. A failed check probe is a result, not a collection error; a
check goes `unknown` only when no probe completed in 3× its interval (`no result for <n>s`).

## 4. Collector abstraction (`gargoyle-collector`)

The collector is synchronous because reading `/proc` and spawning `systemctl` are both blocking. The
scheduler calls it through `tokio::task::spawn_blocking`. The collector is stateless with respect to
rates: it returns **cumulative** counters, and `core::rates` derives per-second values. This keeps
both implementations trivial and makes rate math testable in pure code.

```rust
pub trait Collector: Send + Sync + 'static {
    fn host_info(&self) -> Result<HostInfo, CollectError>;          // hostname, kernel, os, cpu model, cores, boot time
    fn system_snapshot(&self) -> Result<SystemSnapshot, CollectError>;
    fn units(&self, units: &[UnitRequest]) -> Result<Vec<UnitSnapshot>, CollectError>;
    fn timer_history(&self, timer: &str, service: &str, since: i64)   // journal backfill, optional
        -> Result<Vec<TimerRunRecord>, CollectError> { Ok(vec![]) }
}

pub struct SystemSnapshot {
    pub ts: i64,                       // unix seconds
    pub uptime_s: f64,
    pub cpu: CpuCounters,              // total + per-core jiffies (user,nice,system,idle,iowait,irq,softirq,steal)
    pub load: [f64; 3],
    pub memory: MemoryInfo,            // total, available, swap_total, swap_free (bytes)
    pub disks: Vec<DiskUsage>,         // mount, fs_type, total, used, avail (bytes)
    pub net: Vec<NetCounters>,         // iface, rx_bytes, tx_bytes (cumulative)
    pub temps: Vec<TempReading>,       // label (e.g. "coretemp/Package id 0"), celsius
}

pub struct UnitSnapshot {              // whitelisted systemd properties only, see §9
    pub unit: String,
    pub load_state: String, pub active_state: String, pub sub_state: String,
    pub description: String,
    pub active_enter_ts: Option<i64>,
    pub n_restarts: Option<u32>,
    pub result: Option<String>,        // service Result= (success, exit-code, timeout, ...)
    pub exec_main_status: Option<i32>,
    pub exec_main_start_ts: Option<i64>, pub exec_main_exit_ts: Option<i64>,
    pub invocation_id: Option<String>,
    pub memory_current: Option<u64>,
    // timers only:
    pub last_trigger_ts: Option<i64>, pub next_elapse_ts: Option<i64>, pub trigger_unit: Option<String>,
}
```

**`LinuxCollector { root: PathBuf, systemctl: PathBuf, journalctl: PathBuf, system: SystemConfig }`**

- `root` defaults to `/`. Tests point it at `tests/fixtures/<case>/`, which contains fake `proc/` and
  `sys/` trees. Parsers are pure functions over `&str` and are unit-tested against fixture files.
- Sources:
  - `/proc/stat` for CPU.
  - `/proc/meminfo` for memory, using `MemAvailable`.
  - `/proc/loadavg` and `/proc/uptime`.
  - `/proc/self/mountinfo` plus `statvfs` for disks. These are the configured mounts, or auto-detected
    real filesystems: tmpfs, overlay, squashfs and other pseudo filesystems are excluded.
  - `/proc/net/dev` for network. Uses the configured interfaces, or all except `lo`, `veth*`,
    `docker*` and `br-*`.
  - `/sys/class/hwmon/*/{name,temp*_input,temp*_label}` for temperatures, falling back to
    `/sys/class/thermal/thermal_zone*`. If no sensors are found, the result is an empty list, not an
    error.
- systemd:
  ```
  systemctl show --no-pager --timestamp=unix -p <whitelist> -- <unit>...
  ```
  This is one process per unit tick and is run with `Command` and an argument vector, never a shell.
  Unit names are validated at config load against `^[A-Za-z0-9:_.@\\-]+\.(service|timer|socket|mount|target|path)$`.
  For each timer, the triggered service (`Unit=` property, or the timer name with `.service`) is
  added to the same call.
  ([ADR 0003](decisions/0003-systemd-via-systemctl.md))
- Journal backfill (`timer_history`) runs
  ```
  journalctl --unit=<service> --output=json --since=@<ts> --no-pager --lines=5000 \
             --output-fields=<fixed list> _PID=1 MESSAGE_ID=<id>...
  ```
  so only PID 1's unit lifecycle messages for that service come back (never `MESSAGE` or the
  service's own output). Runs are rebuilt per `INVOCATION_ID`; the message IDs and fields are
  documented in `crates/gargoyle-collector/src/linux/journal.rs`. Each call is bounded: killed after
  10s, at most 5000 entries and 8 MiB of output. The scheduler's timeout around it should be at
  least 10s. It is optional; if there is no journal or access is denied, the collector logs one
  warning per process and returns an empty list.

**`MockCollector { seed: u64 }`** (`collector.mode = "mock"`) is deterministic and needs no root,
systemd or Linux paths.

- **System:** sine waves plus seeded noise around plausible baselines. Counters increase
  monotonically, and there is an occasional CPU spike so warning states can be seen.
- **Units:** it answers for any configured unit name. The result is derived from a hash of the name
  so the output is stable, and covers running, failed, inactive and not-found. Names containing
  `fail` are always failed, which makes demos predictable.
- **Timers:** it simulates periodic runs with occasional failures.
- **History:** if the DB is empty at startup in mock mode, the scheduler backfills 7 days of mock
  history so charts are useful right away.

## 5. Storage (`crates/gargoyle/src/storage/`)

The database is a single SQLite file (default `/var/lib/gargoyle/gargoyle.db`) with
`journal_mode=WAL`, `synchronous=NORMAL` and `foreign_keys=ON`. Migrations are numbered SQL strings
embedded in the binary and tracked with `PRAGMA user_version`. Writes go through one connection
behind a `Mutex`, and reads use a second connection, all via `spawn_blocking`. At this data rate,
which is a few dozen rows every 10s, no connection pool is needed.

```sql
CREATE TABLE hosts   (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);          -- 1 row today ("local" → hostname)
CREATE TABLE series  (id INTEGER PRIMARY KEY, host_id INTEGER NOT NULL REFERENCES hosts(id),
                      key TEXT NOT NULL, unit TEXT NOT NULL, UNIQUE(host_id, key));
CREATE TABLE points_raw (series_id INTEGER NOT NULL, ts INTEGER NOT NULL, value REAL NOT NULL,
                      PRIMARY KEY (series_id, ts)) WITHOUT ROWID;
CREATE TABLE points_5m  (series_id INTEGER NOT NULL, ts INTEGER NOT NULL,       -- ts = bucket start
                      avg REAL NOT NULL, min REAL NOT NULL, max REAL NOT NULL, count INTEGER NOT NULL,
                      PRIMARY KEY (series_id, ts)) WITHOUT ROWID;
CREATE TABLE events  (id INTEGER PRIMARY KEY, host_id INTEGER NOT NULL, ts INTEGER NOT NULL,
                      target_kind TEXT NOT NULL,      -- system|service|timer|check
                      target TEXT NOT NULL,           -- e.g. "cpu", "nginx.service"
                      from_state TEXT, to_state TEXT NOT NULL, message TEXT);
CREATE INDEX events_ts ON events(host_id, ts);
CREATE TABLE timer_runs (id INTEGER PRIMARY KEY, host_id INTEGER NOT NULL, timer TEXT NOT NULL,
                      service TEXT NOT NULL, invocation_id TEXT, started_at INTEGER NOT NULL,
                      finished_at INTEGER, result TEXT, exit_status INTEGER,
                      source TEXT NOT NULL,           -- observed|journal
                      UNIQUE(host_id, service, started_at));
CREATE INDEX timer_runs_ts ON timer_runs(host_id, timer, started_at);
CREATE UNIQUE INDEX timer_runs_invocation ON timer_runs(host_id, service, invocation_id)
                      WHERE invocation_id IS NOT NULL;                 -- migration 2
```

A timer run is matched to an existing row by `(service, invocation_id)` first, then by
`(service, started_at)`, because the journal and systemd can disagree on a start by about a second.
A match only fills in missing values (a run in progress that finished), so the first source and
start time are kept and re-inserting after a restart or backfill is a no-op. Journal backfill runs
once after the first successful unit tick (which resolves each timer's `Unit=`), starting from the
newest run stored before startup, or 30 days back.

**Series keys** use the format `<metric>[:<label>]`. Each key has a fixed unit string:

| Key | Unit | Key | Unit |
|---|---|---|---|
| `cpu.usage` | `percent` | `disk.used:<mount>` | `percent` |
| `cpu.iowait` | `percent` | `net.rx:<iface>` / `net.tx:<iface>` | `bytes_per_s` |
| `load.1` / `load.5` / `load.15` | `load` | `temp:<label>` | `celsius` |
| `mem.used` | `percent` | `service.memory:<unit>` | `bytes` |
| `swap.used` | `percent` | `check.latency:<name>` | `ms` |
| | | `service.restarts:<unit>` | `count` |

**Retention and downsampling** (configurable):

| Tier | Resolution | Default retention | Approx. rows/series |
|---|---|---|---|
| `points_raw` | collection interval (10s) | 48h | 17k |
| `points_5m` | 5 min (avg/min/max) | 400d | 115k |

Events and timer runs are kept for 400 days. With about 30 series the database stays in the low
hundreds of MB.

**Queries** pick the tier automatically. A request whose `from` falls within raw retention and whose
range is at most 6h uses `points_raw`; anything else uses `points_5m`. The query then re-buckets in
SQL (`GROUP BY ts / step`) so it returns at most `points` buckets (default 300, maximum 1000). Each
bucket carries `avg`, `min` and `max`.

Query details:

- The grid is anchored at `from` (`from + i·step`), and `to` is inclusive.
- The step is never finer than the native interval of the slowest queried series:
  `system_interval` by default, `units_interval` for `service.memory:*` and `service.restarts:*`, and the check interval
  for `check.latency:*`. On the 5m tier the step is a multiple of 300.
- 5m-tier queries union in the raw rows newer than the last rollup, so the right edge of a chart
  is never empty.
- Values are rounded to 3 decimals.
- Parameter limits: `from >= 0`, `to <= now + 1 day`, `to - from <= 10 years`, and repeated
  `from`, `to` or `points` → 400. A glob that matches nothing returns an empty `series`; only
  unknown exact keys are a 400.

Storage details:

- The single `hosts` row is named `local`, so history survives a hostname change. The API `host`
  field still carries the collected hostname.
- Raw retention deletes whole 5m buckets only, so rollups are never recomputed from partial data.
- Series with no points left in either tier are pruned during maintenance.
- A database that cannot be opened is fatal at startup. Write and maintenance errors at runtime
  are logged and never stop live collection.

## 6. HTTP API (contract)

The API rules apply to every endpoint:

- The base path is `/api/v1`. Every endpoint is **GET** (HEAD is allowed). Any other method returns
  `405`.
- Responses are JSON, UTF-8, `camelCase` keys.
- All timestamps are **unix seconds** (integers). Durations are seconds. Sizes are bytes.
- Health is one of `"healthy" | "warning" | "critical" | "unknown"`.
- Errors use `{ "error": { "code": "not_found", "message": "..." } }` with the matching 4xx/5xx
  status. Codes: `not_found` (404), `bad_request` (400), `unknown_series` (400, an exact metric
  key with no stored series), `unavailable` (503, no history storage), `internal` (500).
- Every top-level response includes `"host": "<hostname>"`. This is reserved for multi-host (§10).
- Optional fields are always present as `null`, never omitted.
- List endpoints wrap their items in an object: `{ "host", "<items>": [...] }`.
- Example payloads live in `docs/api/examples/*.json`, with an index in `docs/api/README.md`. They
  are the canonical shapes, and backend and frontend tests both use them (§11).

| Method & path | Purpose |
|---|---|
| `GET /api/v1/health` | Liveness, never requires auth. Returns `{ "status": "ok", "version": "0.1.0" }`. |
| `GET /api/v1/overview` | Everything needed by the dashboard home page, in one call. Polled every 10s. |
| `GET /api/v1/services` / `GET /api/v1/services/{unit}` | Configured services, or one with recent events. |
| `GET /api/v1/timers` / `GET /api/v1/timers/{unit}?limit=50` | Configured timers, or one with run history. |
| `GET /api/v1/checks` / `GET /api/v1/checks/{name}` | Configured HTTP/TCP checks. |
| `GET /api/v1/metrics` | Available series keys with units and labels. |
| `GET /api/v1/metrics/query?key=..&key=..&from=..&to=..&points=300` | Time series for charts. |
| `GET /api/v1/events?since=..&until=..&target=..&limit=100` | Health transition log, newest first. |

**`GET /api/v1/overview`**
```json
{
  "host": "debian-box", "ts": 1794200000, "health": "warning",
  "hostInfo": { "hostname": "debian-box", "os": "Debian GNU/Linux 13 (trixie)", "kernel": "6.12.0-1-amd64",
                "cpuModel": "Intel(R) N100", "cores": 4, "bootTime": 1793500000, "uptime": 700000 },
  "system": {
    "health": "warning", "ts": 1794199995,
    "cpu":    { "usage": 12.5, "iowait": 0.4, "health": "healthy" },
    "load":   { "one": 0.42, "five": 0.38, "fifteen": 0.31, "health": "healthy" },
    "memory": { "total": 16624951296, "used": 4271000000, "usedPct": 25.7,
                "swapTotal": 1073741824, "swapUsed": 0, "health": "healthy" },
    "disks":  [ { "mount": "/", "fsType": "ext4", "total": 250000000000, "used": 210000000000,
                  "usedPct": 84.0, "health": "warning" } ],
    "net":    [ { "iface": "enp1s0", "rxBps": 120400.0, "txBps": 8800.0 } ],
    "temps":  [ { "label": "coretemp/Package id 0", "celsius": 48.0, "health": "healthy" } ]
  },
  "counts": { "healthy": 5, "warning": 1, "critical": 0, "unknown": 0 },
  "services": [ /* ServiceSummary, see below */ ],
  "timers":   [ /* TimerSummary */ ],
  "checks":   [ /* CheckSummary */ ],
  "recentEvents": [ /* Event, max 10 */ ]
}
```
Before the first system sample, `system.ts` is `0` and every system health is `unknown`. When the
system sample is stale (older than 3× the interval), values are kept but every per-metric `health`
becomes `unknown`. A disk listed in `[system].disks` that is not mounted appears in `disks` with
`fsType: ""`, zero sizes and `health: "critical"`. `memory.used` is `total - available`. `cpu.usage` and `load` are instantaneous values. Health for
cpu and load uses the windowed average (§7). The `net` and `temps` lists may be empty, and an empty
list means "not available" in the UI.

**ServiceSummary** — one item in the `GET /api/v1/services` list:
```json
{ "unit": "navidrome.service", "name": "Navidrome", "description": "Navidrome Music Server",
  "health": "healthy", "message": null,
  "loadState": "loaded", "activeState": "active", "subState": "running",
  "since": 1794000000, "restarts": 0, "memory": 152000000 }
```
`GET /api/v1/services/{unit}` returns `{ "host", "service": ServiceSummary, "events": [Event] }`.

**TimerSummary** — one item in the `GET /api/v1/timers` list:
```json
{ "unit": "music-sync.timer", "name": "Music sync", "service": "music-sync.service",
  "health": "critical", "message": "last run failed: exit-code (status 1)",
  "active": true, "lastRun": { "startedAt": 1794190000, "finishedAt": 1794190042,
                               "duration": 42, "result": "exit-code", "exitStatus": 1 },
  "nextRun": 1794276400, "running": false }
```
`lastRun` is `null` if the timer has never run. `GET /api/v1/timers/{unit}` returns
`{ "host", "timer": TimerSummary, "runs": [TimerRun], "events": [Event] }`, where a `TimerRun` has
the same shape as `lastRun` plus `"source": "observed" | "journal"`.

**CheckSummary**. `GET /api/v1/checks/{name}` returns `{ "host", "check": CheckSummary, "events": [Event] }`.
```json
{ "name": "Music HTTP", "kind": "http", "target": "http://127.0.0.1:4533/ping",
  "health": "healthy", "message": null, "lastCheck": 1794199980, "latencyMs": 3.1,
  "consecutiveFailures": 0 }
```
The `target` field echoes the configured URL or address, with any query string and fragment
stripped. A URL with userinfo (`user:pass@`) is shown as `scheme://host[:port]` only, without its
path. This happens in one place (`CheckConfig::display_target`, `config/target.rs`), which logs
and `Debug` use too; the raw URL only ever goes to the HTTP client. If a URL contains an `@`
anywhere, the display never shows anything before the last `@`, as a backstop to validation (§7). `latencyMs` is that of the last probe if it got an
answer (rounded to 0.1 ms), else `null`. `lastCheck` is when the last probe finished.
`consecutiveFailures` counts failures since the last success; it is `0` before the first probe of
the process. At startup, a check whose last logged state was critical has its count seeded at 3,
because the count is not stored: if the target is still down, the first probe stays critical
instead of logging a spurious critical → warning → critical pair (so the count may include
failures from before the restart). Failure
messages read `<reason> (<n> consecutive failure[s])`, e.g. `connection refused`, `timed out after
5s` or `unexpected status 500 (expected 2xx)`; a slow success reads `slow: 1200 ms (warning ≥ 1s)`.

**Event**
```json
{ "id": 812, "ts": 1794190042, "targetKind": "timer", "target": "music-sync.timer",
  "from": "healthy", "to": "critical", "message": "last run failed: exit-code (status 1)" }
```
`from` is `null` for a target's first observed state. `message` is `null` when healthy.
System targets are `cpu`, `load`, `memory`, `swap`, `disk:<mount>` and `temp:<label>`; services and
timers use the unit name. Transition rules (`core::health::Transitions`):

- **Hold-down:** a new state is logged only after it has been observed on 2 consecutive
  evaluations (system tick for system targets, unit tick for services and timers). A value
  flapping across a threshold every tick therefore logs nothing. **Checks have no hold-down:**
  their consecutive-failure rule (§8) already debounces, and a probe interval can be long, so a
  check's state is logged on the probe that first shows it (warning at failure 1, critical at
  failure 3). The event's `ts` and `message` are
  from the confirming evaluation. Live health in every other endpoint is not delayed.
- A target's first state is logged (with `from: null`) only if it is not healthy, and `unknown` is
  never a first state (CPU is unknown for one tick after every start).
- At startup the last logged state of each target is restored from `events`, so a restart does not
  log an unchanged state again. Until a target has been observed live, `unknown` does not count
  against that restored state.
- A target that disappears (an unmounted auto-detected disk, a removed sensor, a unit removed from
  config) is no longer evaluated, so its last event stays its final word; nothing marks it gone.

**Nullable fields** (always present, may be `null`): `Event.{from,message}`,
`ServiceSummary.{message,since,memory}`, `TimerSummary.{message,lastRun,nextRun}`,
`TimerRun.{finishedAt,duration,result,exitStatus}` (a run in progress), `CheckSummary.{message,lastCheck,latencyMs}`,
`SeriesInfo.label`, and point `avg/min/max`. Every other field is non-null.

**`GET /api/v1/metrics`**: returns
`{ "host", "series": [ { "key": "disk.used:/", "metric": "disk.used", "label": "/", "unit": "percent" } ] }`.
`label` is `null` for unlabelled series.

**`GET /api/v1/metrics/query`**

- `key` is required and may be repeated, up to 12 keys. It also accepts a trailing `*` prefix glob,
  such as `net.rx:*`.
- `from` and `to` are unix seconds. The defaults are `to = now` and `from = to - 3600`.
- `points` defaults to 300 and is clamped to the range 10–1000.

```json
{ "host": "debian-box", "from": 1794196400, "to": 1794200000, "step": 12, "resolution": "raw",
  "series": [ { "key": "cpu.usage", "unit": "percent",
                "points": [ [1794196400, 10.2, 8.0, 14.1], [1794196412, null, null, null] ] } ] }
```
Each point is `[ts, avg, min, max]`, and empty buckets are `null` so charts show gaps. The
`resolution` field is either `"raw"` or `"5m"`. An unknown exact key returns `400` with code
`unknown_series`; other invalid parameters return `400` with `bad_request`.

**`GET /api/v1/events`** returns `{ "host", "events": [Event] }`, newest first (by `ts`, then
`id`). Parameters, all optional and each at most once:

- `since` (inclusive) and `until` (exclusive), unix seconds;
- `kind`: one of `system`, `service`, `timer`, `check`;
- `target`: exact match, combinable with `kind`;
- `limit`: default 100, clamped to 1–500.

A malformed, unknown-valued or repeated parameter returns 400 `bad_request`; unknown parameter
names are ignored.

**Paging.** Pages hold whole seconds only, because one evaluation logs all its transitions with
the same `ts`. To page back, pass `until` = the oldest `ts` of the previous page, with the same
other parameters. That cursor is exclusive, so a page never ends part-way through a second:

- if the events after the first `limit` continue the page's last second, that second moves to the
  next page, so a page may hold **fewer** than `limit` events while more exist;
- if the first `limit` events all share one second, that whole second is returned, so a page may
  hold **more** than `limit` events (at most one per target).

The end of the list is an **empty page**, not a short one. `GET /api/v1/timers/{unit}` takes `limit` for runs (default 50, max 500). Service, timer and
check details include that target's newest 50 events. `{name}` in `/checks/{name}` is
percent-decoded (`Music%20HTTP`); an unknown name is a 404. The overview includes the newest 10.

**Static UI.** `/` serves `index.html` with `Cache-Control: no-cache`. Paths under `/assets/`
(Vite's content-hashed output) are served with `Cache-Control: immutable`; any other embedded file
uses `no-cache`. Unknown
paths return 404. The UI uses hash routing (`/#/timers/x`), so no fallback rewrite is needed.

Updates use polling (overview every 10s, charts every 60s) instead of SSE or WebSockets. At this
scale polling is simpler, works through any proxy and has no connection state.

## 7. Configuration

Configuration is a single TOML file, chosen with `gargoyle --config <path>` (default
`/etc/gargoyle/gargoyle.toml`). It is validated completely at startup: an invalid config prints all
errors and exits non-zero. `gargoyle check-config` validates and exits. All sections are optional
except where stated, and the defaults are shown below.

```toml
[server]
listen = "127.0.0.1:9077"               # non-loopback requires [auth] or allow_unauthenticated_remote
allow_unauthenticated_remote = false

[auth]
mode = "none"                           # "none" | "basic"
username = "admin"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$..."   # from `gargoyle hash-password`
# password_hash_file = "/etc/gargoyle/password"        # alternative, keeps hash out of main file

[storage]
path = "/var/lib/gargoyle/gargoyle.db"
raw_retention = "48h"
rollup_retention = "400d"

[collector]
mode = "linux"                          # "linux" | "mock"
system_interval = "10s"
units_interval = "30s"
journal_backfill = true                 # import past timer runs from journald when permitted

[system]
disks = []                              # [] = auto-detect real filesystems; or ["/", "/srv"]
interfaces = []                         # [] = auto (non-loopback, non-virtual)
temperatures = true

[thresholds]                            # warning/critical; window = averaging window before evaluation
cpu       = { warning = 85.0, critical = 95.0, window = "2m" }   # percent
load      = { warning = 1.5,  critical = 3.0,  window = "5m" }   # 1-min load per core
memory    = { warning = 85.0, critical = 95.0 }                  # percent used
swap      = { warning = 50.0, critical = 90.0 }
disk      = { warning = 80.0, critical = 90.0 }
temperature = { warning = 75.0, critical = 90.0 }               # celsius
disk_mounts = { "/srv" = { warning = 90.0, critical = 95.0 } }  # optional per-mount override

[[service]]
unit = "navidrome.service"              # required
name = "Navidrome"                      # display name; default = unit
expect = "running"                      # "running" (default) | "active" (e.g. oneshot RemainAfterExit) | "any"
memory_warning_mb = 2048                # optional

[[service]]
unit = "minecraft.service"
name = "Minecraft"

[[timer]]
unit = "music-sync.timer"               # required
name = "Music sync"
# service = "music-sync.service"        # default: timer's Unit= property
max_age = "26h"                         # optional: warn if no successful run within this age

[[check]]
name = "Music HTTP"                     # required, unique
kind = "http"                           # "http" | "tcp"
url = "http://127.0.0.1:4533/ping"      # http: GET only, no redirects followed, body not stored
expect_status = 200                     # default: any 2xx
interval = "60s"
timeout = "5s"
latency_warning = "1s"

[[check]]
name = "Game port"
kind = "tcp"
address = "127.0.0.1:25565"
```

Thresholds are global. The only per-target override is `disk_mounts`.

Validation rules:

- Unit names must match the systemd naming regex (§4).
- Check names must be unique.
- A check `url` must start with `http://` or `https://` and contain no whitespace, control
  characters or `\`. It may contain at most one `@`, which must end the userinfo; an `@` in the
  path, query or fragment is rejected. The part after the userinfo must be `host[:port]` (letters,
  digits, `-`, `.`, `_`, or a bracketed IPv6 address; a numeric port). Together these reject every
  URL whose userinfo is ambiguous, such as a password with an unencoded `/`, `?`, `#` or `@`,
  where the URL parser would end the userinfo inside the password and display it as host or path.
  Such characters must be percent-encoded (`%2F`, `%3F`, `%23`, `%40`). The error message never
  echoes the URL. A check `address` is `host:port` with no `@`, `/` or whitespace in the host.
- `warning < critical`.
- Intervals must be at least `1s`. `raw_retention` must be at least `10m`, and not exceed
  `rollup_retention`.
- `listen` must be loopback unless auth is configured or `allow_unauthenticated_remote = true`.

## 8. Health model

There are four states, ordered **critical > warning > unknown > healthy**. Every aggregate is the
worst of its parts in this order.

| Target | healthy | warning | critical | unknown |
|---|---|---|---|---|
| Metric threshold (cpu, load, memory, swap, disk, temperature) | value < warning | warning ≤ value < critical | value ≥ critical | no sample within 3× interval |
| Service (`expect = running`) | active/running | activating, reloading, deactivating; `NRestarts` increased in last hour; memory > `memory_warning_mb` | failed; inactive/dead; LoadState ≠ loaded | collector error, stale |
| Service (`expect = active`) | ActiveState = active | as above | failed or inactive | as above |
| Service (`expect = any`) | not failed | — | failed | as above |
| Timer | timer active and last run `Result=success` (or currently running: the result is not judged while running, but `max_age` still is) | timer not active (stopped/disabled); last success older than `max_age`; `NextElapse` > 10 min in the past | last run result ≠ success | never run; stale |
| Check | success and latency < `latency_warning` | 1–2 consecutive failures; slow | ≥ 3 consecutive failures | not yet run; no probe completed within 3× interval |

- **Windowed metrics.** CPU and load are evaluated on the mean over `window`. The values are kept in
  a small ring buffer in `State`, not queried from the DB. All other metrics are evaluated on the
  latest sample.
- **Inactive timer services.** A timer's service being `inactive` is normal and never counts against
  it. Timer health uses only the `Result` of its last run.
- **Overall status.** `system.health` is the worst of the system metrics. `overview.health` is the
  worst of system, services, timers and checks. `counts` tallies every target, with the system
  counted as one target.
- **Message.** Every non-healthy target carries a short human-readable `message` explaining why,
  such as `"disk / at 84% (warning ≥ 80%)"`.
- **Implementation.** Health evaluation lives in `gargoyle-core::health` as pure functions that take
  config, current input and previous state, and return `(Health, Option<String>)`. These functions
  are exhaustively unit-tested.

## 9. Security model

1. **Read-only by construction.** There are no mutating endpoints and no `POST` routes, and the
   router only registers GET handlers. Gargoyle never calls `systemctl` verbs other than `show` and
   never calls `journalctl` with anything except read flags. Neither the config nor the API can
   cause command execution. Command arguments are fixed vectors and unit names are validated with a
   regex. `systemctl` gets the `--` separator before unit names; `journalctl` gets the unit as the
   single token `--unit=<name>`. Both keep a name from being read as an option.
2. **Localhost by default.** The default listen address is `127.0.0.1:9077`. Binding a non-loopback
   address without auth is refused at startup unless explicitly overridden.
3. **Auth (optional).** HTTP Basic auth checks the password against an argon2id hash. A successful
   check is cached in memory as the SHA-256 of the presented header, so argon2 does not run on every
   poll. The cache is cleared on restart. Failed attempts get a 1s delay, and comparisons are
   constant-time. `/api/v1/health` is exempt. TLS is provided by a reverse proxy (Caddy or nginx) or
   a private network (Tailscale or WireGuard); Gargoyle does not terminate TLS.
   ([ADR 0005](decisions/0005-auth-and-exposure.md))
4. **No secrets in responses.**
   - Only whitelisted systemd properties are fetched. `ExecStart`, `Environment`, `EnvironmentFiles`
     and similar are **never** fetched.
   - Check URLs are echoed without query and fragment, and without anything but
     `scheme://host[:port]` when they have userinfo (§6); logs and the config's `Debug` output
     use the same function. URLs with ambiguous userinfo are rejected at load (§7). Probe failure reasons are built from the error
     kind, never from the client's error text, which contains the URL.
   - Config values such as the password hash and paths are never serialized into the API.
   - Error messages returned to clients are generic. Details go only to the log.
5. **Headers.**
   - The CSP is `default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'`.
   - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, no CORS.
6. **Least privilege.**
   - Gargoyle runs as the dedicated system user `gargoyle`. It does not need root.
   - Reading the journal of system units requires the `systemd-journal` group. This is optional,
     and only journal backfill needs it.
   - The systemd unit is hardened: `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`,
     `NoNewPrivileges=yes`, `StateDirectory=gargoyle`, `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`,
     `CapabilityBoundingSet=` (empty), `SystemCallFilter=@system-service`.
7. **Outbound probes.** Checks only reach targets written in the config file by the admin. They
   never follow redirects, never read or store response bodies, ignore proxy settings from the
   environment (so credentials in a URL go only to the target) and do not reuse connections.

## 10. Multi-machine path (design only — do not build yet)

The future model is **agent → hub push**:

- A small `gargoyle-agent` binary reuses `gargoyle-collector` and `gargoyle-core`. It samples
  locally and `POST`s batches to the hub at `/api/v1/ingest`, authenticated with a per-agent bearer
  token. This would be the only write endpoint, would be separately feature-gated, and would accept
  metrics, never commands. Agents keep a small local buffer for when the hub is unreachable.
- The hub is today's `gargoyle` binary with the ingest route enabled. The local collector is simply
  "the hub's own agent".

The groundwork is already in place now, at no extra cost:

- `hosts` table, plus `host_id` on every stored row.
- `host` field on every API response.
- Collector output is plain serializable data (`SystemSnapshot` and `UnitSnapshot` derive `Serialize`).

Later additions would be `GET /api/v1/hosts` and a `?host=` query parameter on existing endpoints.
The default is the local host, so the v1 contract remains backward compatible.

## 11. Testing strategy

| Layer | How |
|---|---|
| `gargoyle-core` | Unit tests for every health rule, the rate math (including counter wrap and reset after reboot), and config validation (valid and invalid examples). |
| Parsers (`gargoyle-collector`) | Pure parsers tested against fixture files in `crates/gargoyle-collector/tests/fixtures/`. Fixtures are captured from real Debian and Arch hosts. They cover missing hwmon, many cores, odd mount names with `\040`, and `systemctl show` output for running, failed, not-found and timer units. |
| `LinuxCollector` | Integration test with `root` set to a fixture tree, and a fake `systemctl` script set via the `systemctl` path field. One `#[ignore]` smoke test runs against the real host (`make test-live`). |
| Storage | Tests against a temp or in-memory DB covering migrations, inserts, rollup correctness, retention deletes, tier selection and bucketing. |
| HTTP | `tower::ServiceExt::oneshot` against the router with `MockCollector` and a temp DB. Covers status codes, auth on and off, 405 on POST, and security headers. **Contract test:** each `docs/api/examples/*.json` must deserialize into the Rust DTO and round-trip without losing fields. |
| Frontend | `svelte-check` plus `vitest` for formatting, health colors and API client parsing. Example JSON files are imported as typed fixtures, which also checks the TS types against the contract. |
| Manual / E2E | `make dev` runs the backend in mock mode alongside the Vite dev server (which proxies `/api`). Each milestone ends with a short manual checklist (see ROADMAP). |

Tests never require root, systemd, network access (except localhost), or the target server.
