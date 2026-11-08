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
| M4 Checks & health polish | 🚧 In progress |
| M5 Security hardening | Planned |
| M6 Packaging & docs | Planned |

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

- [ ] A check against `127.0.0.1:9077/api/v1/health` is healthy. A TCP check to a closed port goes
      warning → critical after 3 failures. Both of these are in `gargoyle.dev.toml`.
- [ ] A URL configured with `user:pass@` and a `?token=` never appears in any API response
      (test).
- [ ] Making the collector fail (e.g. a fake systemctl that exits 1) turns the affected targets
      `unknown` with a message within 3 intervals.
- [ ] Every non-healthy state in the UI shows its `message`.

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

- [ ] With auth on, every non-health route returns 401 without credentials, including static
      assets, and `WWW-Authenticate` is set. With correct credentials, polling does not run argon2
      on every request (test via counter or timing).
- [ ] `POST`, `PUT` and `DELETE` on any path return 405.
- [ ] Headers test passes for both API and UI responses.
- [ ] Security review report has no unresolved high or medium findings.

---

## M6 — Packaging, deployment docs & polish

**Goal.** Ready for the user to install on the Debian server. Gargoyle itself performs no
deployment.

**Scope**

- **M6.1** Release profile (LTO, strip, `panic = "abort"`). `make release` builds the frontend and
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

- [ ] `make deb` produces a `.deb`. `dpkg-deb -c` lists the expected files only, and `lintian` has
      no errors (warnings documented).
- [ ] `systemd-analyze verify packaging/gargoyle.service` passes, and `systemd-analyze security`
      exposure ≤ 3.0 (documented).
- [ ] Following DEPLOYMENT.md in a clean Debian 13 container or VM (developer's local machine only)
      gets the dashboard running as the `gargoyle` user. **Nothing is installed on the real
      server by agents.**
- [ ] The config reference test passes, and the README quick start works on a fresh clone.

---

## Later (not scheduled)

- Multi-machine agent/hub (ARCHITECTURE §10).
- Notifications (email, ntfy) on health transitions. This is still read-only toward the host.
- Disk I/O and per-process top-N.
- SSE for live updates if polling ever proves insufficient.
