# 0003 — Read systemd state by running `systemctl show` / `journalctl`, not D-Bus (zbus)

**Status:** Accepted · 2026-11-08

## Context

Gargoyle needs unit and timer properties (ActiveState, SubState, Result, NRestarts, timer
last/next trigger) for a handful of configured units, every 30s. Past timer runs come from journald.
There are two options:

- **zbus**, which is pure-Rust D-Bus and talks to `org.freedesktop.systemd1`.
- Running `systemctl show` (and `journalctl -o json`) as subprocesses.

## Decision

Run the subprocesses:

```
systemctl show --no-pager --timestamp=unix -p <whitelist> -- <unit>...
```

This is one batched call per tick. Gargoyle parses the `Key=value` output, which is blank-line
separated per unit, in a pure function.

- **Testability.** Parsers are tested with captured text fixtures. The integration test swaps in a
  fake `systemctl` script through a configurable path. Mocking zbus proxies is much more work.
- **Simplicity.** There are no proxy definitions, no async D-Bus connection lifecycle, and no
  reconnect handling. zbus pulls in a large dependency tree for what is effectively 2 calls every
  30s.
- **journald.** Run history needs `journalctl` (or `libsystemd` FFI) anyway, so a subprocess is
  unavoidable there. Shelling out to systemctl is consistent with that.
- **Cost.** One fork/exec every 30s is negligible.
- **Stability.** `systemctl show` property names and `--timestamp=unix` are a stable, documented
  interface.

## Safety rules (mandatory)

- Use `std::process::Command` with an argument vector, never `sh -c`.
- The only verbs allowed are `show` (systemctl) and read flags (journalctl).
- Unit names are regex-validated at config load. systemctl receives them after `--`; journalctl
  receives a single `--unit=<name>` token. Neither can be parsed as an option.
- A fixed property whitelist is defined in one place. `ExecStart*`, `Environment*` and similar
  properties are never requested.

## Consequences

- Systemd and journal data depend on the binaries being present (always true on Debian).
- If polling ever needs to be sub-second, or needs signal-based change notifications, swap in a
  zbus implementation behind the same `Collector` trait. The trait boundary makes this a local
  change.
