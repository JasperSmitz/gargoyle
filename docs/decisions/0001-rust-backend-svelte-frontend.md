# 0001 — Rust backend, Svelte 5 + Vite + uPlot frontend

**Status:** Accepted · 2026-11-08

## Context

Gargoyle needs a small, low-overhead daemon for a personal server, and a polished but modest
dashboard. The user wants a Rust backend. The available toolchain is node 26 and pnpm.

## Decision

**Backend:** Rust (edition 2024) with `tokio` and `axum`. This is the mainstream, well-maintained
async HTTP stack in 2026, with good `tower` middleware for auth and headers.

**Frontend:** Svelte 5 (runes) + Vite + TypeScript as a plain SPA. It does not use SvelteKit.
Charts use **uPlot**, and styling is hand-written CSS with custom properties.

- Svelte compiles to small bundles with little runtime and no virtual DOM. A dashboard of about 6
  views does not need React's ecosystem.
- SvelteKit is not used because the app has no SSR and no routing needs that justify it. Routing
  is a hand-written hash router (`#/timers/foo`, ~30 lines). It needs no dependency, and deep links
  work without server-side rewrites.
- uPlot is about 50 KB, very fast for time series with tens of thousands of points, and supports
  min/max bands and gaps. Chart.js and ECharts are heavier and slower for time series.
- There is no CSS framework and no component library. A small design-token CSS file keeps the look
  consistent.

## Consequences

- Single-language backend, memory-safe, small static-ish binary.
- Frontend developers must know Svelte 5 runes. It is still a small surface.
- TS types are hand-maintained against the contract. The example-JSON fixtures in `docs/api/`
  catch drift, so a type generator (ts-rs, specta) is not needed.
