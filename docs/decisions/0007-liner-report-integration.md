# 0007 — Surface the Liner music-library report by reading its JSON file

**Status:** Accepted · 2026-11-08 (M7)

## Context

Liner (a separate project and repository) is a read-only music metadata linter. `liner scan
--format json` writes one versioned JSON report (schema `1.x`, Liner ADR 0003): a summary,
the albums, the issues (each with a stable `code`, a `severity` of `info|warning|error`, a
`scope` of `file|album|library`, an `album_id` and root-relative `paths`) and `errors[]`, the
files that could not be read. The owner wants the state of the server's music library, the copy
Navidrome serves, on the Gargoyle dashboard next to the services and timers.

The questions to settle: how Gargoyle gets the data, how far it trusts and parses it, what
"health" means for metadata, and what reaches the API.

## Decision

**Gargoyle reads a report file. It never links, runs or schedules Liner.**

- A new optional config section `[library]` names the report path (`report`, required in the
  section), a display `name` (default `"Music library"`), `max_age` (default `"26h"`),
  `warn_on` (`error|warning|info|never`, default `error`) and the poll `interval` (default
  `"60s"`). Without the section the feature is off: no polling, no API data, no UI.
- Producing the report is the admin's job, recommended as a systemd timer on the server
  (`liner-scan.timer`, with `--fail-on never`) that Gargoyle monitors with an ordinary
  `[[timer]]` entry ([DEPLOYMENT.md](../DEPLOYMENT.md#music-library-report-liner)). So the timer
  shows whether the scan ran, and the library target shows what it found. Running Liner from
  Gargoyle would break the "no command execution driven by config" rule (ARCHITECTURE §9.1)
  and make a slow scan of thousands of files part of Gargoyle's process.

**Reading** (binary, `spawn_blocking`):

- Every `interval`, `stat` the file. It is read and parsed again only when its identity or
  metadata changed (device, inode, size, mtime, ctime). Liner replaces the report by rename, so
  every new report changes the inode. A `chmod` changes ctime, so a fixed permission is picked
  up. A read that failed with an IO error (missing, permission denied) is retried on every
  poll. Health is evaluated on every poll anyway, because the report's age keeps growing.
- Not a regular file, or larger than **32 MiB**: refused without reading. The read is bounded
  as well (`take(32 MiB + 1)`), in case the file grows while it is read. A real report of 1148
  files is about 0.8 MB, so 32 MiB allows libraries about 40 times larger.
- The data is parsed once per change into the API view (below), which is kept in `State`.
  Requests never touch the file.
- After a failed read or parse, the previous report is **dropped**, not kept: the API shows
  `report: null` and the reason. Keeping it would show data the file no longer contains. The
  rename-based write means a reader never sees a half-written report from Liner itself. A copy
  tool that writes in place (`rsync --inplace`) can cause one failed poll, which heals on the
  next.

**Parsing** (pure, `gargoyle-core`):

- Gargoyle's own serde DTOs cover only the fields it uses. They follow Liner's consumer rules:
  unknown fields are ignored, and issue `code`, error `kind`, `format` and `tag_types` are
  plain strings, so new values in a later 1.x are accepted. `severity` and `scope` are closed
  enums, as Liner's schema states for a major version. Per-track data (`albums[].discs`) and
  `issues[].details` are not deserialized into anything; `root` is not read (see below).
- **Version gate first:** `schema_version` is read before the typed parse. Major `1` (any
  `1.x.y`, including pre-release suffixes) is accepted. Another numeric major gives
  `unsupported Liner schema 2.0.0`, even if the rest would not match the v1 DTOs. A missing or
  non-numeric major is an invalid report.
- `generated_at` is RFC 3339 (`YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)`, Liner writes `Z`).
  It is parsed by a small pure function in `gargoyle-core` rather than with a date crate,
  because nothing else in Gargoyle needs calendar code.
- Counts come from Liner's `summary` (`issues_by_severity`, `unreadable_files`, `albums`),
  which is contractual, not from counting the arrays.
- Errors reaching the API never contain report content or the file path: `report not found`,
  `cannot read report: permission denied`, `report too large (40 MiB, max 32 MiB)`,
  `invalid report (line 12, column 5)` (from serde_json's position, never its message text,
  which can quote values), `invalid report: generated_at`, `unsupported Liner schema 2.0.0`.
  The full error goes to the log, with the path.
- `gargoyle-core` gains `serde_json` as a normal dependency (it was a dev-dependency; it is
  already a workspace dependency of the binary). The parser stays `fn(&str) -> Result<_, _>`.
- A contract test parses Liner's own golden report (`tests/golden/report.json` in the Liner
  repository), copied into `crates/gargoyle-core/tests/fixtures/liner/` with a note on its
  origin, plus synthetic variants (unknown fields and codes, schema `1.7.0`, `2.0.0`, bad
  `generated_at`, unknown severity).

**Health** (pure, `gargoyle-core::health::library`, table-tested; ARCHITECTURE §8):

- `unknown`: configured, but not read yet; the file is missing, unreadable, too large or
  invalid; the schema major is not 1; or no poll completed within 3× `interval`.
- `warning`, for any of: the report is older than `max_age` (by `generated_at`); an issue
  count at or above `warn_on` is non-zero; `errors[]` is non-empty, unless `warn_on = never`
  (Liner itself treats unreadable files as error severity).
- `healthy` otherwise.
- **Never `critical`.** Metadata problems are not outages. Navidrome being down is the
  service's job to report.
- The message joins the reasons with `; `, in this order: age, issues, unreadable files. For
  example `report is 3d old (max 26h); 4 errors, 7 warnings in 95 albums; 2 unreadable files`.
- A report filtered with `--min-severity` hides issues from Gargoyle as well. The API exposes
  `minSeverity` and `issuesHidden` so the UI can say so; health does not guess.

**Events and overview:** the library is one more target, with `targetKind: "library"` and the
fixed target name `library`. Its transitions follow the unit-tick rules (2-evaluation hold-down,
restored from `events` at startup). It counts in `overview.health` and `overview.counts`. The
events table needs no migration (`target_kind` is free text), and there is **no new table**:
no history of issue counts in M7.

**API** (additive contract change, approved by the coordinator; ARCHITECTURE §6):
`GET /api/v1/library` returns `{ host, configured, library: LibrarySummary | null, report:
LibraryReport | null }`. `overview.library` carries the same `LibrarySummary` (or `null` when
not configured). The view:

- issues are grouped per album (id, title, album artist, disc and track counts) plus
  `libraryIssues` (scope `library`, `album_id: null`) and `errors[]`;
- issues are ordered by severity (error first), keeping Liner's order within a severity, and
  albums by their worst issue, then id; the **first 500 issues** in that order are kept
  (`issuesTotal`, `issuesTruncated`), at most **50 paths per issue** (`pathsTotal`) and the
  **first 100 errors** (`errorsTotal`, `errorsTruncated`). These arrays are cut while parsing
  (a capped serde visitor), not after, so a report within 32 MiB cannot take many times that in
  memory; `issues_by_code` is limited to 1000 codes. Every report string is cut to 1 KiB (4 KiB
  for paths and album ids) with `…`, and paths share a 2 MiB budget over the view (ARCHITECTURE
  §6). The worst response is then a few MB, and a normal one tens of KB. `State` holds the view
  behind an `Arc`, so a request does not copy it;
- issue counts per code come as a list sorted by count, then code (codes are data and are not
  camelCased as keys);
- `message` texts are passed through as plain text (Liner guarantees one line with control
  characters escaped), and the UI never renders them as HTML.

**What is not exposed:** the configured report path (a config path, §9.4; messages never name
it) and Liner's `root` (the absolute library location on the host). The UI does not need either:
every path in the report is relative to `root`. This also keeps the M5.4 response audit simple:
it gains the report path and a marker `root` value as strings that must never appear. Exposing
`root` later would be an additive change.

## Alternatives considered

- **Run `liner scan` from Gargoyle on a schedule.** Rejected: command execution from config,
  long blocking work in the monitor, and Liner's dependencies and failure modes inside Gargoyle.
- **Link Liner as a library crate.** Rejected: couples releases and pulls the audio tag reader
  into a monitoring binary. The JSON schema is the designed integration point (Liner ADR 0003).
- **Store issue counts as series** (`library.issues:error`) for charts. Deferred: the report
  changes once a day, so a chart has little to show, and it would add series semantics for a
  value that is not sampled. Events already record when health changed.
- **Expose the full report unchanged.** Rejected: per-track data is most of the file's size,
  it would pin Gargoyle's API to Liner's schema, and it includes `root`.
- **Keep the last good report after a failed read.** Rejected for M7, as above. Revisit if
  copying the report over the network makes transient failures common.

## Consequences

- One more config section, endpoint, page and target kind. Clients that switch on
  `targetKind` must handle `library`.
- A Liner `2.0` report shows as `unknown` until Gargoyle learns the new schema. That is a
  deliberate, visible failure rather than a silently wrong parse.
- The report file must be readable by the `gargoyle` user. The Liner service must therefore
  not use `DynamicUser=yes` together with `StateDirectory=`: systemd then puts the directory
  under `/var/lib/private/`, which is `0700 root`, and no other user can reach the report
  through the `/var/lib/liner` symlink. DEPLOYMENT.md uses a static `liner` system user.
