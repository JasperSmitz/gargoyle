# API contract examples

These files are the **canonical** JSON shapes for `/api/v1`. The prose contract is in
[ARCHITECTURE §6](../ARCHITECTURE.md#6-http-api-contract).

- **Backend:** a contract test deserializes every file into its DTO in `gargoyle-core::api`. It
  then re-serializes and compares, ignoring key order.
- **Frontend:** each file is imported as a typed fixture, so `svelte-check` fails if the TS types
  drift. The files also serve as offline UI fixtures.

| Endpoint | File | Top-level type |
|---|---|---|
| `GET /api/v1/health` | `examples/health.json` | `HealthResponse` (unauthenticated) |
| `GET /api/v1/overview` | `examples/overview.json` | `Overview` |
| `GET /api/v1/services` | `examples/services.json` | `{ host, services: ServiceSummary[] }` |
| `GET /api/v1/services/{unit}` | `examples/service-detail.json` | `{ host, service, events }` |
| `GET /api/v1/timers` | `examples/timers.json` | `{ host, timers: TimerSummary[] }` |
| `GET /api/v1/timers/{unit}` | `examples/timer-detail.json` | `{ host, timer, runs, events }` |
| `GET /api/v1/checks` | `examples/checks.json` | `{ host, checks: CheckSummary[] }` |
| `GET /api/v1/checks/{name}` | `examples/check-detail.json` | `{ host, check, events }` |
| `GET /api/v1/metrics` | `examples/metrics.json` | `{ host, series: SeriesInfo[] }` |
| `GET /api/v1/metrics/query` | `examples/metrics-query.json` | `MetricsQuery` |
| `GET /api/v1/events` | `examples/events.json` | `{ host, events: Event[] }` |
| any 4xx/5xx | `examples/error.json` | `ApiError` |

## Rules

- Keys are camelCase. Timestamps are unix **seconds** (integers). Durations are seconds. Sizes are
  bytes. Rates are bytes per second.
- Optional values are always present as `null`, never omitted. The DTOs must not use
  `skip_serializing_if`.
- Health is `"healthy" | "warning" | "critical" | "unknown"`.
- List order:
  - Services, timers and checks are returned in config order.
  - Events are returned newest first.
  - Runs are returned newest first.
  - Disks are returned in config order, or sorted by mount when auto-detected.
- `{unit}` and `{name}` path segments are percent-encoded by the client. An unknown unit or name
  returns `404` with an `ApiError`.
- Changing any of these files is a contract change. It needs coordinator approval and must update
  the DTOs, the TS types and ARCHITECTURE §6 together.
