# Gargoyle — Roadmap

Each milestone ends in a working, tested state:

- `make check` is green.
- The independent reviewer gives a PASS.
- The coordinator commits the result.

Task IDs (`M1.3`, etc.) are suggested units of work that can be handed to developers. The "∥" lines
list which tasks can run in parallel. The API contract ([ARCHITECTURE §6](ARCHITECTURE.md#6-http-api-contract)
and `docs/api/examples/`) is fixed in M0. After M0, backend and frontend work proceeds in parallel
against it. Contract changes need a coordinator decision and must update docs, examples and DTOs in
the same change.

## Status

| Milestone | Status |
|---|---|
| M0 Foundation | ✅ Done (2026-11-08) |
| M1 Live dashboard | ✅ Done (2026-11-08) |
| M2 History & charts | ✅ Done (2026-11-08) |
| M3 Timers & events | ✅ Done (2026-11-08) |
| M4 Checks & health polish | ✅ Done (2026-11-08) |
| M5 Security hardening | ✅ Done (2026-11-08) |
| M6 Packaging & docs | ✅ Done, Debian install test pending (2026-11-08) |
| M7 Music library report (Liner) | ✅ Done, server deployment pending (2026-11-08) |

---

## M0 — Foundation

**Goal.** Skeleton repo where every later task has a home and a green build.

**Scope**

- **M0.1** Cargo workspace with three crates (`gargoyle-core`, `gargoyle-collector`, `gargoyle`).
  It uses edition 2024, shared `[workspace.dependencies]`, and `clippy`/`rustfmt` config.
- **M0.2** `gargoyle-core`:
  - Config structs, TOML parsing and validation (§7), including duration parsing.
  - The `Health` enum with ordering and a `worst()` helper.
  - API DTOs for every §6 shape, using `serde(rename_all = "camelCase")`.
- **M0.3** `docs/api/examples/*.json` for every endpoint, plus a contract test that round-trips each
  file through the DTOs.
- **M0.4** Binary:
  - `clap` CLI with `run` (default), `check-config` and `hash-password` (stub is OK).
  - `tracing` logging.
  - axum server serving `GET /api/v1/health`, with the non-loopback-without-auth guard.
- **M0.5** `web/` scaffold:
  - Vite + Svelte 5 + TS, `svelte-check`, `vitest`, `prettier`.
  - Dev proxy from `/api` to `127.0.0.1:9077`.
  - TS types mirroring the DTOs, with typed import of the example JSON files.
- **M0.6** UI embedding via `rust-embed`. Serves `web/dist` (hash routing, so no SPA fallback is needed). A fresh checkout
  without a built frontend still compiles and serves a placeholder page.
- **M0.7** `Makefile` targets (see AGENTS.md), `config/gargoyle.dev.toml`, and `config/gargoyle.example.toml`.

**Parallel.** M0.1 → (M0.2 ∥ M0.5), then M0.3 ∥ M0.4 ∥ M0.6. M0.7 can happen at any point.

**Acceptance**

- [x] On a fresh clone, `make check` runs fmt-check, clippy `-D warnings`, cargo tests, svelte-check,
      vitest and prettier-check, and all pass.
- [x] `cargo run -- --config config/gargoyle.dev.toml` serves `/api/v1/health` →
      `{"status":"ok","version":...}` on 127.0.0.1:9077.
- [x] With `listen = "0.0.0.0:9077"` and auth `none`, startup exits non-zero with a clear message.
- [x] `gargoyle check-config` reports **all** errors in a deliberately broken config.
- [x] Every example JSON round-trips through the Rust DTOs. Each one also type-checks as its TS
      type.

---

## M1 — Live dashboard vertical slice

**Goal.** A usable dashboard showing live system metrics and configured service states. It works
with both the mock collector and the real Linux collector. There is no history yet.

**Scope**

- **M1.1** `Collector` trait and `MockCollector`, covering system and units (§4).
- **M1.2** `LinuxCollector` system part:
  - Parsers for `/proc/stat`, `meminfo`, `loadavg` and `uptime`, plus mounts + statvfs.
  - Fixtures and tests for each.
  - `host_info` from `/etc/os-release`, `/proc/sys/kernel/{hostname,osrelease}` and `/proc/cpuinfo`.
- **M1.3** `LinuxCollector` units part:
  - Batched `systemctl show` with the property whitelist, plus a parser.
  - Fixtures for running, failed, inactive, not-found and oneshot units.
  - Fake-`systemctl` integration test.
- **M1.4** Core:
  - `rates` for CPU% from jiffy deltas, including counter-reset handling.
  - Health rules for the system metrics (with windows) and for services.
  - Overall aggregation and messages.
- **M1.5** Scheduler and in-memory `State`. Endpoints: `/overview`, `/services` and
  `/services/{unit}` (events may be an empty list).
- **M1.6** UI:
  - App shell with header (host, overall health, last-updated), responsive grid, light/dark via
    `prefers-color-scheme`.
  - Overview page with stat cards for CPU, load, memory, swap, disks and uptime, plus a services
    list with health badges and messages.
  - Polls every 10s and shows a stale/offline banner when the API fails.

**Parallel.** M1.1 ∥ M1.2 ∥ M1.3 ∥ M1.4 ∥ M1.6. The UI is built against the example JSON and
`make dev` mock mode. M1.5 integrates them.

**Acceptance**

- [x] `make dev` (mock): the dashboard at the Vite URL shows changing CPU and memory values and
      services in at least three different health states. It updates without reload.
- [x] `make run-local` (Linux collector on the Arch dev box, configured with e.g.
      `NetworkManager.service`, `sshd.service` and one non-existent unit) shows real values. The
      non-existent unit is `critical` with message "unit not found".
- [x] A release build (`make build`) produces one binary that serves the dashboard with no
      `web/` directory present.
- [x] The overview layout works at 360px width with no horizontal scroll, and at desktop width.
- [x] Health rule unit tests cover every row of the §8 table for system and services.

---

## M2 — History & charts

**Goal.** Metrics are persisted, retained and charted.

**Scope**

- **M2.1** Storage module:
  - Migrations, `hosts`/`series`/`points_raw`/`points_5m`.
  - Writer, rollup job, retention job and `PRAGMA optimize`.
- **M2.2** Query layer: tier selection and re-bucketing to `points` with avg/min/max and null gaps.
  Endpoints `/metrics` and `/metrics/query`, including the `*` glob.
- **M2.3** Network rates (`/proc/net/dev`) and temperatures (hwmon, thermal_zone fallback), with
  fixtures. Add them to the overview and series.
- **M2.4** Mock backfill: 7 days of history when the DB is empty in mock mode.
- **M2.5** UI:
  - uPlot chart component with min/max band, gaps, unit-aware axes, tooltip, and theme-aware colors.
  - System page with charts for CPU, load, memory, disk, net and temps.
  - Range picker: 1h / 6h / 24h / 7d / 30d / 1y.
  - Sparklines on the overview cards.

**Parallel.** M2.1 ∥ M2.3 ∥ M2.5. The UI uses the example JSON first and then mock mode. M2.2
depends on M2.1, and M2.4 depends on M2.1.

**Acceptance**

- [x] After restarting, history from before the restart is still present.
- [x] Rollup test: a known raw series produces exact 5m avg/min/max/count. Retention test: rows
      older than retention are deleted.
- [x] `/metrics/query` returns at most `points` buckets, uses `resolution: "raw"` for 1h and `"5m"`
      for 7d, and returns 400 on an unknown key.
- [x] In mock mode every range in the picker renders a sensible chart within 300ms on the dev box.
- [x] On a box without hwmon, the temps section is hidden and there are no errors.
- [x] Simulated DB growth test or estimate: 30 series at 10s for 400d stays under 500 MB.

---

## M3 — Timers, events & history

**Goal.** systemd timers are first-class, and every health transition is logged and browsable.

**Scope**

- **M3.1** Collector:
  - Timer properties (`LastTriggerUSec`, `NextElapseUSecRealtime`, `Unit=`) plus the triggered
    service's run properties, in the same batched call.
  - Fixtures for success, failed, never-run and running cases.
- **M3.2** Core: timer health rules (§8), including `max_age` and overdue.
- **M3.3** Run detection: a new `InvocationID` or `ExecMainStartTimestamp` inserts a `timer_runs`
  row (`source = observed`), and the row is completed on exit.
- **M3.4** Journal backfill (`journalctl -o json`, fixed fields) with fixtures. When permission is
  denied it logs a warning once and continues.
- **M3.5** Events: write a row on every target health transition. Endpoint `/events`. Service and
  timer detail endpoints include events and runs. Add `NRestarts` and service memory series.
- **M3.6** UI:
  - Timers table on the overview: last run (relative time + result), next run (countdown), health.
  - Timer detail page with run history (duration bars, result).
  - Service detail page with state, uptime, restarts, memory chart and events.
  - Global events page with filters.

**Parallel.** M3.1 ∥ M3.2 ∥ M3.6. M3.3, M3.4 and M3.5 follow M3.1 and can run in parallel with
each other.

**Acceptance**

- [x] In mock mode, a timer shows a failed run as `critical` with the exit status. After the next
      successful run it returns to `healthy`, and both transitions appear in `/events`.
- [x] Inactive services triggered by timers never count against the timer's health.
- [x] On the dev box, a real user-created test timer, run with `systemctl --user` or a system timer
      the developer creates locally, shows correct last and next run. (Gargoyle itself creates
      nothing.)
- [x] Restarting gargoyle does not create duplicate `timer_runs` rows (unique constraint test).
- [x] The events page paginates (`until` cursor) and filters by target.

---

## M4 — Generic checks & health polish

**Goal.** Optional HTTP/TCP probes. Health is complete, consistent and explainable everywhere.

**Scope**

- **M4.1** Checks runner:
  - HTTP GET via reqwest (rustls, no redirects, timeout, status match, body discarded).
  - TCP connect with timeout.
  - Per-check intervals, consecutive-failure tracking, a latency series and events.
- **M4.2** Endpoints `/checks` and `/checks/{name}`, plus URL sanitization (strip userinfo and query).
- **M4.3** Staleness: targets with no successful sample in 3× their interval become `unknown`.
  Collector errors are surfaced as `message`.
- **M4.4** UI:
  - Checks section and detail page with a latency chart.
  - Consistent health badge component (color plus icon and text, accessible to colorblind users).
  - Document title and favicon reflect overall health.
  - Empty states when nothing is configured.

**Parallel.** M4.1 ∥ M4.3 ∥ M4.4. M4.2 depends on M4.1.

**Acceptance**

- [x] A check against `127.0.0.1:9077/api/v1/health` is healthy. A TCP check to a closed port goes
      warning → critical after 3 failures. Both of these are in `gargoyle.dev.toml`.
- [x] A URL configured with `user:pass@` and a `?token=` never appears in any API response
      (test).
- [x] Making the collector fail (e.g. a fake systemctl that exits 1) turns the affected targets
      `unknown` with a message within 3 intervals.
- [x] Every non-healthy state in the UI shows its `message`.

---

## M5 — Security hardening

**Goal.** Safe to expose beyond localhost behind a proxy or VPN.

**Scope**

- **M5.1** `[auth] mode = "basic"`:
  - argon2id verify, verified-credential cache, 1s failure delay, constant-time username compare.
  - `password_hash_file` support.
  - `/api/v1/health` exempt.
- **M5.2** `gargoyle hash-password` reads from stdin or a TTY prompt (no echo) and prints a PHC
  string.
- **M5.3** Security headers middleware (§9.5). GET/HEAD only, with 405 otherwise. Request body size
  limit 0. Request timeout.
- **M5.4** Response audit: tests asserting that no API response contains the password hash, config
  paths, `ExecStart` or environment values, and that the systemd property whitelist is enforced in
  one place.
- **M5.5** Security review of the whole codebase by an independent reviewer, using the `/security-review`
  skill or equivalent. Findings must be fixed or explicitly accepted.

**Parallel.** M5.1 ∥ M5.2 ∥ M5.3 ∥ M5.4, then M5.5.

**Acceptance**

- [x] With auth on, every non-health route returns 401 without credentials, including static
      assets, and `WWW-Authenticate` is set. With correct credentials, polling does not run argon2
      on every request (test via counter or timing).
- [x] `POST`, `PUT` and `DELETE` on any path return 405.
- [x] Headers test passes for both API and UI responses.
- [x] Security review report has no unresolved high or medium findings.

---

## M6 — Packaging, deployment docs & polish

**Goal.** Ready for the user to install on the Debian server. Gargoyle itself performs no
deployment.

**Scope**

- **M6.1** Release profile (LTO, strip; `panic = "unwind"` is kept deliberately, because collector-crash recovery and per-connection isolation depend on it). `make release` builds the frontend and
  then the binary, targeting `x86_64-unknown-linux-gnu`. Document glibc-compatibility caveats
  (Debian stable glibc vs Arch); a `musl` build option is acceptable if it is simple.
- **M6.2** `packaging/gargoyle.service`, a hardened unit (§9.6), and `cargo-deb` metadata:
  - Install the binary to `/usr/bin`.
  - Install the example config to `/etc/gargoyle/gargoyle.toml` as a conffile.
  - `postinst` creates the `gargoyle` system user (optionally adding it to `systemd-journal`).
  - State directory is `/var/lib/gargoyle`.
- **M6.3** `docs/DEPLOYMENT.md` covering:
  - Build, `.deb` install, and manual install (no .deb).
  - Configuring services and timers, and enabling auth.
  - Reverse proxy examples (Caddy, nginx) and Tailscale note.
  - Upgrade, backup (copy the DB with `sqlite3 .backup` or stop + copy), uninstall, and
    troubleshooting.
- **M6.4** `docs/CONFIG.md`, a complete config reference generated or checked against the config
  structs (test that every field is documented).
- **M6.5** UI polish:
  - Loading skeletons, keyboard navigation, focus styles.
  - Number and unit formatting review.
  - Automated accessibility checks (e.g. axe-core in vitest/jsdom on key pages) plus a manual
    keyboard/contrast checklist, and a bundle size check (JS < 150 KB gzipped).
- **M6.6** `README.md` with screenshots from mock mode, a feature summary and a quick start.

**Parallel.** M6.1 ∥ M6.4 ∥ M6.5 ∥ M6.6. M6.2 depends on M6.1, and M6.3 depends on M6.2.

**Acceptance**

- [x] `make deb` produces a `.deb`. `dpkg-deb -c` lists the expected files only, and `lintian` has
      no errors (warnings documented).
- [x] `systemd-analyze verify packaging/gargoyle.service` passes, and `systemd-analyze security`
      exposure ≤ 3.0 (documented).
- [ ] **Not verified (no container runtime available on the dev machine):** Following DEPLOYMENT.md in a clean Debian 13 container or VM (developer's local machine only)
      gets the dashboard running as the `gargoyle` user. **Nothing is installed on the real
      server by agents.**
- [x] The config reference test passes, and the README quick start works on a fresh clone.

---

> **Status notes (2026-11-08).** Contents were inspected with `ar`/`tar` because `dpkg-deb` and
> `lintian` are not installed on the Arch dev box, so the lintian result is unknown (expected
> warnings: no changelog, placeholder maintainer address). The unit's exposure score is 1.6. The
> sandbox was exercised in a transient *user* unit with the real Linux collector: services, timers,
> journal backfill and checks all worked. The real Debian install test is still open.

---

## M7 — Music library report (Liner)

**Goal.** The dashboard shows the metadata health of the server's music library from the JSON
report that Liner (a separate tool) writes. Gargoyle reads the file and never runs Liner.
Design: [ADR 0007](decisions/0007-liner-report-integration.md). Contract: ARCHITECTURE §6
(`GET /api/v1/library`, `LibrarySummary`, `overview.library`, `targetKind: "library"`), §7
(`[library]`), §8 (library row and message formats), §9.4, and `docs/api/examples/`
(`library.json`, `library-unconfigured.json`, and the updated `overview.json` and
`events.json`).

**Contract freeze.** M7.0 fixes the contract before any code exists, so backend and frontend
work in parallel. Until M7.1 and M7.2 land, `make check` is **expected to be red** in exactly
these places: the Rust contract test (new example files and fields, the `library` target kind),
`crates/gargoyle-core/tests/config_reference.rs` (CONFIG.md documents `[library]`, which the
config structs do not have yet), and until M7.3 the web contract fixtures. Developers must not
change the contract to make them green. A needed change goes back to the coordinator.

**Scope**

- **M7.0** (architect, done) ADR 0007; the contract files listed above; CONFIG.md `[library]`;
  DEPLOYMENT.md "Music library report (Liner)"; this section.
- **M7.1** Backend, `gargoyle-core` (pure, no IO):
  - Config: `LibraryConfig { report, name, max_age, warn_on, interval }` as
    `Config.library: Option<_>`, with the defaults and validation of §7.
    `config_reference.rs` passes against the new CONFIG.md rows and example.
  - Liner input DTOs (subset, unknown fields ignored, `code`/`kind` as `String`,
    `severity`/`scope` closed enums) and `parse_report(&str) -> Result<LinerReport,
    ReportError>`: schema-major gate before the typed parse, RFC 3339 parser for
    `generated_at` (hand-written, table-tested, no new crate), error values that carry no report
    text (serde_json line and column only).
  - The view builder `LinerReport -> LibraryReport`: grouping, ordering and caps
    (500 issues, 50 paths per issue, 100 errors) per §6. `serde_json` becomes a normal
    dependency of `gargoyle-core`.
  - API DTOs `LibraryResponse`, `LibrarySummary`, `LibraryReport`, `ReportSummary`,
    `SeverityCounts`, `CodeCount`, `LibraryAlbum`, `LibraryIssue`, `LibraryFileError`,
    `LinerTool`, `Severity`, `IssueScope`, `WarnOn`; `Overview.library`;
    `TargetKind::Library`. Contract test entries for both new example files.
  - `health::library`: a pure evaluation returning `(Health, Option<String>)` from config, read
    outcome and `now`, table-tested against every §8 rule and message format: not yet read, each
    unknown cause, stale poll, age boundaries (exactly `max_age` is healthy), future
    `generated_at`, each `warn_on` × severity combination, unreadable with and without `never`,
    combined messages in order, singular and plural.
  - Fixtures under `crates/gargoyle-core/tests/fixtures/liner/`: Liner's golden report
    (`tests/golden/report.json` of the Liner repository, schema 1.0.0) unchanged, with a
    `README.md` naming its origin and the Liner commit it was copied from; synthetic variants
    (unknown fields, issue codes and error kinds; `1.7.0`; `2.0.0` with a different shape;
    missing `schema_version`; bad `generated_at`; unknown `severity`; more than 500 issues and
    50 paths for the caps). Nothing is copied from a real library report.
- **M7.2** Backend, binary (after M7.1):
  - Library reader task (`crates/gargoyle/src/library.rs` or similar), enabled only with
    `[library]`: first poll 1s after start, then every `interval`; `stat` and change key
    (device, inode, size, mtime, ctime); regular files only; 32 MiB cap checked from the
    metadata and enforced on the read; IO errors retried every poll; the previous view dropped
    after a failed read. All IO in `spawn_blocking`; a panic in it is a read failure, not a
    crash. The path is logged at most once per distinct error, never put in a message.
  - `State` holds the summary inputs and the prebuilt `LibraryReport`. Health is evaluated each
    tick. Transitions go through `Transitions` (unit-tick hold-down) as target `library`, kind
    `library`, and are restored from `events` at startup (storage maps `"library"` both ways;
    no migration).
  - `overview`: `library` summary, library in `health` and `counts`. `GET /api/v1/library`
    (GET/HEAD only, `age` computed at response time). `/events?kind=library`.
  - Tests: HTTP via `oneshot` for configured (valid, missing, invalid, `2.0.0` report) and
    unconfigured; reader tests on a temp dir (change detection by rename, `chmod` fix picked up,
    oversized file, a directory, a FIFO is not opened); a transition test (warning logged
    once, restored after restart); the M5.4 response audit extended with a `[library]` whose
    report path and a marker `root` value must appear in no response.
  - `config/gargoyle.dev.toml` gets `[library]` pointing at a new synthetic
    `config/liner-report.dev.json` (several albums, every severity, one library issue, one
    unreadable file; `generated_at` in the past, so mock mode also shows the age warning), and
    `config/gargoyle.example.toml` a commented-out `[library]` block.
- **M7.3** Frontend (parallel with M7.1 and M7.2, against the example files):
  - TS types for every new DTO, `Overview.library`, `TargetKind` `'library'`; fixtures for
    `library.json` and `library-unconfigured.json` in `fixtures.ts`; `client.library()`.
  - Route `#/library`, `targetHref('library', …)`, and an Events kind filter option. The nav
    entry **Library** and the overview card appear only when `overview.library` is not `null`;
    a direct visit to `#/library` without `[library]` shows an empty state pointing to
    CONFIG.md `[library]`.
  - Overview: a library card (name, health badge, message, error/warning/info counts,
    unreadable files, report age as relative time, link to the page), and the library in the
    "needs attention" list when not healthy.
  - Library page (polls `/library` every 60s): header with name, health and message; summary
    tiles (albums, audio files, unreadable files, errors, warnings, info); report age and
    Liner version; a note when `issuesHidden > 0` or anything is truncated; a severity filter
    (all / warning and up / errors only) kept in the URL query; albums as native
    `<details>` elements, collapsed by default, with expand-all and collapse-all controls,
    each summary showing title, album artist, id and per-severity counts; issues with a
    severity chip (icon and text, never color alone), code, message as text and paths
    (shown relative to the album id when they start with it, wrapping at 360px, "and N more"
    from `pathsTotal`); library-scope issues and unreadable files as their own sections;
    `report: null` shows the status message and no tables.
  - Severity colors come from tokens in the single tokens file (error, warning and info chips
    aliased to the health palette); no new npm packages.
  - Tests: vitest for the grouping, filter and path helpers, page states (normal, filtered,
    truncated, hidden issues, `report: null`, unconfigured, loading, offline), nav and card
    visibility; the Library page and the overview with a library card added to
    `a11y.test.ts` and the CSP test; `pnpm size` stays ≤ 150 KiB.
- **M7.4** Docs and verification (after M7.2 and M7.3): README feature list and a mock-mode
  screenshot of the Library page; DEPLOYMENT.md and CONFIG.md checked against the shipped
  behavior (`check-config` with the documented `[library]`); the two Liner units in DEPLOYMENT.md
  checked with `systemd-analyze verify` on copies in a scratch directory (nothing installed,
  no `daemon-reload`); ACCESSIBILITY.md keyboard checklist gains the Library page; KNOWN_ISSUES
  updated if anything is deferred.
- **M7.5** Independent review of the whole milestone, including a security pass over the new
  file-read path (size cap, file type, no path or `root` in responses, messages as text in the
  UI).

**Status (2026-11-08).** M7.0–M7.3 are merged. M7.4 is done: README and screenshots (Library
page, and the overview refreshed now that it has a library card), `check-config` run with the
CONFIG.md and DEPLOYMENT.md `[library]` examples and each validation rule, a real `liner scan`
report served end to end (unreadable file, rename, deletion, `chmod 000` and back), and the two
Liner units verified with `systemd-analyze verify` on scratch copies (clean apart from
`/usr/local/bin/liner` not existing on the dev box). M7.5 is done: an independent review and a
Codex second opinion (FAIL: parse-time memory amplification, per-string response caps, zero
chips on the card, untested poll error paths, `<wbr>` DOM amplification), a fix round, a
re-review (FAIL: page-wide `<wbr>` count, control-character escaping, album count) and a second
fix round. The Library expand-all race found in M7.4 was reproduced in a test and fixed.
The acceptance list below is ticked from those reviews; `make dev` itself was not run, the
same dev config was checked on the embedded build.

**Parallel.** M7.1 → M7.2 (backend developer) ∥ M7.3 (frontend developer), then M7.4, then
M7.5. The frontend develops against the example JSON first and against `make dev` once M7.2
lands.

**Acceptance**

- [x] Without `[library]`, nothing changes: no library in the overview (`library: null`,
      counts unchanged), no nav entry, `/api/v1/library` answers `configured: false`, and no
      file is read.
- [x] `make dev` shows the Library nav entry, the overview card and the page with grouped,
      filterable, collapsible issues from `config/liner-report.dev.json`.
- [x] Liner's golden report parses; a `1.7.0` report with unknown fields and codes parses; a
      `2.0.0` report is `unknown` with `unsupported Liner schema 2.0.0`.
- [x] Health is never `critical` (property or table test over all inputs), and every §8 library
      rule and message format has a test row.
- [x] Replacing the report file (rename) is picked up within one `interval`; an unchanged file
      is not parsed again (test via a parse counter or equivalent); a deleted file turns the
      library `unknown` with `report not found` after the hold-down.
- [x] A report over 32 MiB is refused without being read into memory.
- [x] A library health change appears in `/events` and on the Events page with kind `library`,
      once, and is not logged again after a restart.
- [x] Neither the configured report path nor the report's `root` appears in any API response
      (audit test).
- [x] The Library page works at 360px without horizontal scroll and passes the automated a11y
      and CSP tests; the bundle stays ≤ 150 KiB gzipped.
- [x] `make check` is green.

---

## Later (not scheduled)

- Multi-machine agent/hub (ARCHITECTURE §10).
- Notifications (email, ntfy) on health transitions. This is still read-only toward the host.
- Disk I/O and per-process top-N.
- SSE for live updates if polling ever proves insufficient.
