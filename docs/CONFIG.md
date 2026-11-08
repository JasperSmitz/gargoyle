# Gargoyle configuration reference

Gargoyle reads one TOML file, `/etc/gargoyle/gargoyle.toml` by default, or the path given with
`gargoyle --config <path>`. The file is read once at startup, so restart Gargoyle after a change
(`systemctl restart gargoyle`).

- **Every section and every key is optional** unless the tables below mark it *required*. An empty
  file is a valid config: the real Linux collector on `127.0.0.1:9077`, without auth, watching no
  services.
- **Unknown keys are errors.** A typo such as `listne` stops startup instead of being silently
  ignored.
- **Validation reports every problem at once.** Run `gargoyle --config <path> check-config` to
  check a file without starting Gargoyle. It prints each error with its location (for example
  `service[1].unit`) and exits non-zero.
- **Durations** are strings such as `"500ms"`, `"30s"`, `"5m"`, `"48h"` or `"400d"`.

This page lists every key. A test (`crates/gargoyle-core/tests/config_reference.rs`) reads the
key list from the config structs and fails when a key is missing from this page, when the page
lists a key that does not exist, or when the example below is not a valid config.

## Complete example

Every key, with a typical value. Defaults are listed in the tables below.

```toml
[server]
listen = "127.0.0.1:9077"
allow_unauthenticated_remote = false

[auth]
mode = "basic"
username = "admin"
password_hash_file = "/etc/gargoyle/password"
# password_hash = "$argon2id$v=19$m=19456,t=2,p=1$..."   # alternative to password_hash_file

[storage]
path = "/var/lib/gargoyle/gargoyle.db"
raw_retention = "48h"
rollup_retention = "400d"

[collector]
mode = "linux"
system_interval = "10s"
units_interval = "30s"
journal_backfill = true

[system]
disks = ["/", "/srv"]
interfaces = ["eth0"]
temperatures = true

[thresholds]
cpu         = { warning = 85.0, critical = 95.0, window = "2m" }
load        = { warning = 1.5, critical = 3.0, window = "5m" }
memory      = { warning = 85.0, critical = 95.0 }
swap        = { warning = 50.0, critical = 90.0 }
disk        = { warning = 80.0, critical = 90.0 }
temperature = { warning = 75.0, critical = 90.0 }
disk_mounts = { "/srv" = { warning = 90.0, critical = 95.0 } }

[[service]]
unit = "navidrome.service"
name = "Navidrome"
expect = "running"
memory_warning_mb = 2048

[[service]]
unit = "backup-mount.service"
expect = "active"

[[timer]]
unit = "music-sync.timer"
name = "Music sync"
service = "music-sync.service"
max_age = "26h"

[[check]]
name = "Music HTTP"
kind = "http"
url = "http://127.0.0.1:4533/ping"
expect_status = 200
interval = "60s"
timeout = "5s"
latency_warning = "1s"

[[check]]
name = "Game port"
kind = "tcp"
address = "127.0.0.1:25565"

[library]
report = "/var/lib/liner/report.json"
name = "Music library"
max_age = "26h"
warn_on = "error"
interval = "60s"
```

## `[server]`

| Key | Type | Default | Description |
|---|---|---|---|
| `server.listen` | address | `"127.0.0.1:9077"` | `ip:port` to serve the dashboard and API on. Port 0 is rejected. A non-loopback address (such as `0.0.0.0:9077`) requires `auth.mode = "basic"` or `allow_unauthenticated_remote = true`. For remote access, prefer keeping this on loopback behind a reverse proxy with TLS ([DEPLOYMENT.md](DEPLOYMENT.md#reverse-proxy)). |
| `server.allow_unauthenticated_remote` | bool | `false` | Allows a non-loopback `listen` with `auth.mode = "none"`. Only for addresses on a network that is itself the access control, such as a Tailscale or WireGuard interface. |

## `[auth]`

| Key | Type | Default | Description |
|---|---|---|---|
| `auth.mode` | `"none"` \| `"basic"` | `"none"` | `"basic"` puts HTTP Basic auth (single user) in front of every page, asset and API endpoint, except `/api/v1/health`, which reveals only the version. |
| `auth.username` | string | `"admin"` | User name. Must not be empty when `mode = "basic"`. |
| `auth.password_hash` | string | none | The argon2id PHC string printed by `gargoyle hash-password` (starts with `$argon2id$`). With `mode = "basic"`, set exactly one of `password_hash` and `password_hash_file`. |
| `auth.password_hash_file` | path | none | A file holding only the PHC string (surrounding whitespace is ignored), read once at startup. Keeps the hash out of the main config. Make it `root:gargoyle` mode `0640`; Gargoyle logs a warning if it is world-readable. An unreadable file or invalid hash stops startup. |

Use a long random password (20+ characters from a password manager). Gargoyle limits guessing to
about 10 attempts per second in total, which is harmless to such a password and not to a short
one. Wrong passwords are answered after a 1 s delay. Basic auth sends the password with every
request, so outside localhost use it only behind TLS or on a VPN. See
[DEPLOYMENT.md](DEPLOYMENT.md#enabling-authentication) for the setup steps.

## `[storage]`

| Key | Type | Default | Description |
|---|---|---|---|
| `storage.path` | path | `"/var/lib/gargoyle/gargoyle.db"` | The SQLite database. Its directory must be writable by the `gargoyle` user; the systemd unit's `StateDirectory=` provides `/var/lib/gargoyle`. A directory Gargoyle creates gets mode `0750`, and the database files `0640`. |
| `storage.raw_retention` | duration | `"48h"` | How long full-resolution samples are kept. At least `10m`, and not longer than `rollup_retention`. |
| `storage.rollup_retention` | duration | `"400d"` | How long 5-minute rollups (average, minimum and maximum) are kept. At least `1s`. |

## `[collector]`

| Key | Type | Default | Description |
|---|---|---|---|
| `collector.mode` | `"linux"` \| `"mock"` | `"linux"` | `"linux"` reads `/proc`, `/sys`, `systemctl show` and `journalctl`. `"mock"` generates demo data and needs no systemd; it is for development. |
| `collector.system_interval` | duration | `"10s"` | How often system metrics are sampled. At least `1s`. |
| `collector.units_interval` | duration | `"30s"` | How often service and timer states are polled. At least `1s`. |
| `collector.journal_backfill` | bool | `true` | Imports past runs of the configured timers' services from the journal after startup (up to 30 days back, or from the newest run already stored). This needs read access to the system journal (the `systemd-journal` group, see [DEPLOYMENT.md](DEPLOYMENT.md#journal-access-for-timer-history)). Without it nothing is imported, and run history starts when Gargoyle starts. |

## `[system]`

| Key | Type | Default | Description |
|---|---|---|---|
| `system.disks` | list of paths | `[]` | Mount points to monitor, in this order. `[]` auto-detects real filesystems: pseudo and virtual filesystems (tmpfs, overlay, squashfs, ...) are skipped, and each device is reported once. Each entry must be absolute. A configured mount point that does not exist is skipped. |
| `system.interfaces` | list of names | `[]` | Network interfaces to monitor. `[]` means all except `lo`, `veth*`, `docker*` and `br-*`. |
| `system.temperatures` | bool | `true` | Reads temperatures from hwmon and thermal zones when the hardware exposes them. |

## `[thresholds]`

Health levels for system metrics. A value below `warning` is healthy, `warning` up to (but not
including) `critical` is a warning, and `critical` or above is critical. In every table
`warning` must be less than `critical`. A table may set only some of its keys; the rest keep the
defaults, so `cpu = { warning = 70.0 }` is valid. Thresholds are global; the only per-target
override is `disk_mounts`.

| Key | Type | Default | Description |
|---|---|---|---|
| `thresholds.cpu.warning` | number | `85.0` | CPU busy, percent. |
| `thresholds.cpu.critical` | number | `95.0` | CPU busy, percent. |
| `thresholds.cpu.window` | duration | `"2m"` | CPU is averaged over this window before it is evaluated, so short spikes do not flip health. At least `1s`. |
| `thresholds.load.warning` | number | `1.5` | 1-minute load average divided by the number of cores. |
| `thresholds.load.critical` | number | `3.0` | 1-minute load average divided by the number of cores. |
| `thresholds.load.window` | duration | `"5m"` | Averaging window for load, as for CPU. At least `1s`. |
| `thresholds.memory.warning` | number | `85.0` | Memory used (total minus available), percent. |
| `thresholds.memory.critical` | number | `95.0` | Memory used, percent. |
| `thresholds.swap.warning` | number | `50.0` | Swap used, percent. A host without swap is always healthy. |
| `thresholds.swap.critical` | number | `90.0` | Swap used, percent. |
| `thresholds.disk.warning` | number | `80.0` | Disk space used, percent, for every monitored mount. |
| `thresholds.disk.critical` | number | `90.0` | Disk space used, percent. |
| `thresholds.temperature.warning` | number | `75.0` | Each sensor, °C. |
| `thresholds.temperature.critical` | number | `90.0` | Each sensor, °C. |
| `thresholds.disk_mounts.*.warning` | number | `disk.warning` | Per-mount override. Write it as a table keyed by the absolute mount point: `disk_mounts = { "/srv" = { warning = 90.0 } }`. Missing keys come from `thresholds.disk`. |
| `thresholds.disk_mounts.*.critical` | number | `disk.critical` | Per-mount override, as above. |

## `[[service]]`

One `[[service]]` table per systemd service (or other unit) to watch. Each unit may appear once.

| Key | Type | Default | Description |
|---|---|---|---|
| `service.unit` | string | *required* | Unit name: letters, digits and `:_.@-`, ending in `.service`, `.socket`, `.mount`, `.target`, `.path` or `.timer`; at most 255 characters. Template instances such as `getty@tty1.service` are fine; a bare template (`foo@.service`) is not. |
| `service.name` | string | the unit name | Display name in the dashboard. |
| `service.expect` | `"running"` \| `"active"` \| `"any"` | `"running"` | What counts as healthy. `"running"`: active and running (a long-lived daemon). `"active"`: active is enough, for a oneshot with `RemainAfterExit=yes`. `"any"`: only the `failed` state is a problem. A unit that systemd does not know is reported as not found. |
| `service.memory_warning_mb` | integer | none | Warns when the service's cgroup uses more than this much memory, in MiB. |

## `[[timer]]`

One `[[timer]]` table per systemd timer to watch. Each timer may appear once.

| Key | Type | Default | Description |
|---|---|---|---|
| `timer.unit` | string | *required* | A `.timer` unit name (same rules as `service.unit`). |
| `timer.name` | string | the unit name | Display name in the dashboard. |
| `timer.service` | string | the timer's `Unit=` | The `.service` the timer triggers, whose runs and results are shown. Set it only if systemd's `Unit=` property is not the one you want. |
| `timer.max_age` | duration | none | Warns when there has been no successful run for longer than this, for example `"26h"` for a daily timer. Without it, only a failed last run is a problem. |

## `[[check]]`

Optional HTTP and TCP probes. Each check runs on its own schedule. One or two consecutive failures
are a warning; three or more are critical.

| Key | Type | Default | Description |
|---|---|---|---|
| `check.name` | string | *required* | Unique name, not empty, without leading or trailing spaces. |
| `check.kind` | `"http"` \| `"tcp"` | *required* | `"http"` sends a `GET` to `url`. `"tcp"` opens a connection to `address` and closes it. |
| `check.url` | string | *required for http* | `http://` or `https://` URL. Redirects are not followed and the body is discarded. `user:pass@` is sent as Basic auth; percent-encode `/`, `?`, `#` and `@` inside it (`%2F`, `%3F`, `%23`, `%40`), or the URL is rejected. The dashboard and logs show the URL without query string or fragment, and only `scheme://host[:port]` when it has credentials. `https` trusts the built-in Mozilla root certificates only, so a self-signed target always fails. Proxy environment variables are ignored. Not allowed for `tcp`. |
| `check.address` | string | *required for tcp* | `host:port`, for example `"127.0.0.1:25565"`. No `@`, `/` or spaces in the host. Not allowed for `http`. |
| `check.expect_status` | integer | any 2xx | The HTTP status code that counts as success (100–599). Not allowed for `tcp`. |
| `check.interval` | duration | `"60s"` | How often to probe. At least `1s`. |
| `check.timeout` | duration | `"5s"` | How long to wait for an answer. Greater than 0 and not longer than `interval`. |
| `check.latency_warning` | duration | none | Warns when a successful probe takes longer than this. Greater than 0. |

## `[library]`

Optional. Shows a music library's metadata health from the JSON report of
[Liner](DEPLOYMENT.md#music-library-report-liner), a separate tool that Gargoyle never runs: it
only reads the report file. Without this section there is no Library page, overview card or
library target. See [ADR 0007](decisions/0007-liner-report-integration.md).

| Key | Type | Default | Description |
|---|---|---|---|
| `library.report` | path | *required* | The report written by `liner scan --format json --output <file>`. Must be readable by the `gargoyle` user. A relative path is relative to Gargoyle's working directory. The path is never shown in the dashboard or API. Reports with schema version `1.x` are understood; files over 32 MiB are refused. |
| `library.name` | string | `"Music library"` | Display name of the library card and page. Not empty, without leading or trailing spaces. |
| `library.max_age` | duration | `"26h"` | Warns when the report's `generated_at` is older than this, for example because the scan timer stopped. At least `1m`. `"26h"` suits a daily scan. |
| `library.warn_on` | `"error"` \| `"warning"` \| `"info"` \| `"never"` | `"error"` | The lowest issue severity that turns the library to warning. Unreadable files (Liner's `errors[]`) count as errors and also warn, except with `"never"`, which warns only about the report's age. The library is never critical. |
| `library.interval` | duration | `"60s"` | How often Gargoyle checks the file. It is read and parsed again only when it changed. At least `1s`. |

## Validation rules in short

- Unit names follow the systemd naming rules above; timers must be `.timer` and a timer's
  `service` must be `.service`.
- Service units, timer units and check names must each be unique.
- In every threshold, `warning < critical`.
- Intervals and windows are at least `1s`; `raw_retention` is at least `10m` and not longer than
  `rollup_retention`; a check's `timeout` is not longer than its `interval`.
- `library.report` must not be empty, `library.name` must not be empty or padded with spaces,
  `library.max_age` is at least `1m` and `library.interval` at least `1s`.
- `server.listen` must be loopback, unless `auth.mode = "basic"` or
  `allow_unauthenticated_remote = true`.
- With `auth.mode = "basic"`: a non-empty `username` and exactly one of `password_hash` (an
  argon2id string) and `password_hash_file`.
