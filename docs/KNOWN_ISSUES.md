# Known issues & technical debt

Keep entries short; remove them when fixed.

| Area | Issue | Planned |
|---|---|---|
| Packaging | `ProtectHome=yes` hides a separate `/home` partition from auto disk detection (and a configured `/home` would statvfs the tmpfs). Use `ProtectHome=read-only` or document. | M6.2 |
| Collector | Per-core CPU counters are indexed by position; CPU hotplug between samples would pair the wrong cores. Rare on a home server. | — |
| Collector | An unparseable optional systemd property logs a warning every units tick (no dedupe). | if noisy |
| Collector | Requires systemd ≥ 252 (`--timestamp=unix`); Debian 12+ is the documented minimum. | M6.3 doc |
| Health | iowait is shown but not part of health evaluation. | — |
| UI | First load while the backend is offline shows only the banner (no host name). | M6.5 |
| Checks | A check's transitions are evaluated only after its own probe. If its loop ever stalled (probes are bounded by `timeout ≤ interval`, so it should not), the API would show `unknown` but no event would be logged. | — |
| Checks | `https` checks trust only the built-in Mozilla roots (webpki-roots), so a target with a self-signed or internal-CA certificate always fails with an "invalid peer certificate" reason. | if needed |
