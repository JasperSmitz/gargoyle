# 0004 — Embed the built frontend in the binary (rust-embed)

**Status:** Accepted · 2026-11-08

## Context

Installation should be a single file plus a config file. There should be no separate web root to
keep in sync with the binary version.

## Decision

- `pnpm build` writes `web/dist/`, and `rust-embed` embeds it into the `gargoyle` binary at compile
  time.
- The server serves assets with the correct MIME type. Hashed assets get
  `Cache-Control: public, max-age=31536000, immutable`, and `index.html` gets `no-cache`. The UI uses hash
  routing, so there is no SPA fallback.
- A fresh checkout without a built frontend (such as a backend-only dev, or CI without node) must
  still `cargo build` and `cargo test`. In that case the server serves a small built-in placeholder
  page that says "frontend not built — run `make web`". The implementer chooses the mechanism, such
  as a `build.rs` that creates an empty `web/dist` or a cargo feature. Keep it minimal.
- In development the UI runs under the Vite dev server with hot reload and proxies `/api` to the
  backend. The embedded copy is only used for release and integration runs.

## Consequences

- Release builds must run `make web` first. `make build` and `make release` enforce this order.
- Binary size grows by the gzipped-equivalent asset size, about 100–200 KB.
