# 0006 — Parse /proc and /sys directly instead of using `sysinfo`

**Status:** Accepted · 2026-11-08

## Context

Gargoyle needs CPU, memory, load, uptime, disks, network and temperatures on Linux only. The
`sysinfo` crate covers these across platforms.

## Decision

Write small, pure parsers for `/proc/stat`, `/proc/meminfo`, `/proc/loadavg`, `/proc/uptime`,
`/proc/self/mountinfo`, `/proc/net/dev` and `/sys/class/{hwmon,thermal}`. `statvfs` comes from
`rustix`. All paths are resolved under a configurable `root` so that tests run against fixture
trees.

- **Testability.** We can feed captured Debian and Arch fixtures to the parsers, including edge
  cases (counter resets, escaped mount paths, missing sensors). `sysinfo` reads the live system
  only.
- **Semantics we control.** For example, used memory is `MemTotal - MemAvailable`, and the
  filesystem filters and network-interface filters are our own choices.
- **Size.** Each parser is tens of lines. The cross-platform abstraction and process scanning in
  `sysinfo` would be unused.
- **Stateless collector.** Returning cumulative counters keeps rate math in pure `core` code, which
  fits [ARCHITECTURE §4](../ARCHITECTURE.md#4-collector-abstraction-gargoyle-collector).

## Consequences

- Gargoyle is Linux-only, which is the product target. The mock collector covers development on
  other machines.
- New metrics need a parser and a fixture, which is a small and reviewable change.
