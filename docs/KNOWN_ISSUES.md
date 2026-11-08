# Known issues & technical debt

Keep entries short; remove them when fixed.

| Area | Issue | Planned |
|---|---|---|
| HTTP | Slow-read clients (send requests, never read responses) keep their connection slot, as hyper has no write timeout; 256 such connections make the dashboard unreachable without credentials. Accepted (M5.5 R1): mitigated by a reverse proxy with a client write timeout (nginx also buffers responses), which DEPLOYMENT.md recommends (with Caddy and nginx examples) for any non-LAN exposure. | — |
| Collector | Per-core CPU counters are indexed by position; CPU hotplug between samples would pair the wrong cores. Rare on a home server. | — |
| Collector | An unparseable optional systemd property logs a warning every units tick (no dedupe). | if noisy |
| Health | iowait is shown but not part of health evaluation. | — |
| Checks | A check's transitions are evaluated only after its own probe. If its loop ever stalled (probes are bounded by `timeout ≤ interval`, so it should not), the API would show `unknown` but no event would be logged. | — |
| Web | Library page: when an album is opened and closed again within the same moment (two synthetic key presses with no delay in a headless browser; not reproduced with a real double-click), a following **Expand all** can leave that album closed. **Collapse all** then **Expand all** recovers. Found in the M7.4 keyboard pass; cause not investigated (likely the async `toggle` event against the `bind:open` getter/setter). | if seen in real use |
| Checks | `https` checks trust only the built-in Mozilla roots (webpki-roots), so a target with a self-signed or internal-CA certificate always fails with an "invalid peer certificate" reason. | if needed |
