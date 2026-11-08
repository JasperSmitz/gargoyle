# AGENTS.md — conventions for all agents on Gargoyle

Gargoyle is a read-only, single-binary monitoring dashboard for one Debian server. It is written in
Rust with axum and SQLite, and has a Svelte 5 frontend embedded in the binary.

**Read first:** `docs/ARCHITECTURE.md` (design and the API contract), `docs/ROADMAP.md` (milestones
and task IDs), and `docs/decisions/` (ADRs; do not re-litigate them without the coordinator).

## Layout

```
crates/gargoyle-core/       pure logic: config, health, rates, API DTOs   (no IO, no tokio)
crates/gargoyle-collector/  Collector trait, LinuxCollector, MockCollector, parsers, tests/fixtures/
crates/gargoyle/            binary: cli, scheduler, storage, checks, http, auth, embedded UI
web/                        Svelte 5 + Vite + TS (pnpm); built into web/dist and embedded
config/                     gargoyle.dev.toml (mock), gargoyle.example.toml (documented)
docs/                       ARCHITECTURE, ROADMAP, decisions/ (ADRs), api/ (contract examples)
packaging/                  systemd unit, deb metadata (M6)
```

## Commands

Run all commands from the repo root. `Makefile` is the source of truth; until M0.7 lands, use the
underlying commands.

| Command | What it does |
|---|---|
| `make check` | Runs **before every hand-off**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and in `web/`: `pnpm check` (svelte-check), `pnpm test` (vitest), `pnpm format:check` |
| `make test` | Runs the Rust and web tests only |
| `make fmt` | Runs `cargo fmt` and `pnpm format` |
| `make dev` | Starts the backend in mock mode (`config/gargoyle.dev.toml`) and the Vite dev server, which proxies `/api` |
| `make run-local` | Starts the backend with the real Linux collector on this dev machine (read-only) |
| `make web` / `make build` / `make release` | Frontend build / debug binary with embedded UI / optimized binary |
| `make test-live` | Runs the `#[ignore]` smoke tests against the real local host |

## Coding conventions

**Rust**

- Use edition 2024. Share dependencies through `[workspace.dependencies]`.
- Library crates use `thiserror`; the binary uses `anyhow`. Do not use `unwrap()` or `expect()` in
  non-test code unless the reason it cannot fail is stated in the `expect` message.
- Keep logic pure where possible. Health, rates and config validation belong in `gargoyle-core`, and
  parsers are `fn(&str) -> Result<T>`.
- Blocking work (`/proc` reads, subprocesses, SQLite) runs in `spawn_blocking`.
- Log with `tracing`. Never log secrets, such as auth headers or the password hash.
- **TypeScript and Svelte:** use `strict` mode, Svelte 5 runes, and no UI or CSS frameworks. Styles
  use CSS custom properties from one tokens file. Charts use uPlot only. Format with Prettier.
- **API:** the contract is `docs/api/examples/*.json` together with ARCHITECTURE §6. Changing it is
  a contract change, which needs coordinator approval and must update the DTOs, the TS types and
  the docs in the same change.
- **Dependencies:** stick to the baseline in ARCHITECTURE §2. Every new crate or npm package needs a
  one-line justification in your hand-off report.
- **Style:** small modules and clear names. Comments explain *why*, not *what*. Do not leave
  speculative abstractions or dead code behind.

## Testing expectations

- Every change ships with tests. Parsers are tested against fixtures. Health rules are tested
  table-style against ARCHITECTURE §8. HTTP is tested with `tower::ServiceExt::oneshot`, and storage
  is tested against a temp DB.
- Tests must run without root, systemd, the internet or the target server. Use `MockCollector`,
  fixture trees (`LinuxCollector { root }`) or a fake `systemctl` script.
- New fixtures that come from a real host must have hostnames, IPs and usernames scrubbed.
- `make check` must be green before you hand off. If something cannot pass, say so explicitly in
  your report.

## Safety constraints (non-negotiable)

- **Gargoyle is read-only.** Do not add mutating endpoints, service control, `systemctl` verbs
  other than `show`, or command execution driven by config or the API. Commands use an argument
  vector and never run through a shell.
- **Never touch real system configuration**, on this machine or the server. Do not install units,
  edit `/etc`, run `sudo`, or enable or start services. Local test timers and units go under
  `systemctl --user`, and only if a task explicitly requires them.
- **No deployment.** Do not SSH to the server, copy files to it or install packages on it.
  Packaging work is tested in local containers or VMs only.
- **No `git push`, no commits** unless the coordinator explicitly assigns you to. Do not rewrite
  history.
- Never put secrets in the repo, in fixtures or in API responses (see ARCHITECTURE §9).

## Workflow

1. The **coordinator** assigns a task by ID (e.g. `M1.3`) with its scope.
2. The **developer** implements it in the working tree without committing. When `make check` is
   green, the developer reports:
   - a summary,
   - the files changed,
   - any new dependencies with their justification,
   - any deviations from the docs,
   - the test output.
3. The **reviewer**, who must be an independent agent and not the developer, reviews against the
   task, the architecture, this file and the safety constraints. The reviewer runs `make check` and
   returns **PASS**, or **FAIL** with concrete, actionable findings. On FAIL, the work goes back to
   step 2.
   When several developers work concurrently, each gets its own git worktree and branch created by
   the coordinator (`../gargoyle-wt/<task>` on branch `wip/<task>`); stay inside your worktree and
   your assigned files.
4. The **coordinator** integrates and commits, using conventional commit messages
   (`feat(collector): ...`).

When docs and code disagree, raise it with the coordinator instead of silently picking one. When a
change alters the design, update `docs/ARCHITECTURE.md`, or add an ADR in `docs/decisions/NNNN-*.md`,
in the same change.
