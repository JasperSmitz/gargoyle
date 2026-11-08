# Deploying Gargoyle

This guide installs Gargoyle on a Debian server, either as a `.deb` or by hand. Gargoyle is one
binary with an embedded web UI. It runs as the unprivileged system user `gargoyle`, keeps its
history in `/var/lib/gargoyle/gargoyle.db` and serves the dashboard on `127.0.0.1:9077`.

Commands prefixed with `sudo` run on the server. Build commands run on your build machine, which
can also be the server.

Contents: [Requirements](#requirements) · [Building](#building) ·
[Installing the .deb](#installing-the-deb) · [Manual install](#manual-install-without-the-deb) ·
[Configuring](#configuring-services-timers-and-checks) ·
[Authentication](#enabling-authentication) · [Remote access](#reverse-proxy) ·
[Journal access](#journal-access-for-timer-history) ·
[Music library report](#music-library-report-liner) · [Upgrade](#upgrading) ·
[Backup](#backup) · [Uninstall](#uninstalling) · [Troubleshooting](#troubleshooting)

## Requirements

- **Debian 12 (bookworm) or newer**, x86_64. Other systemd distributions work too if they meet
  the next two points.
- **systemd 252 or newer.** Gargoyle reads unit state with `systemctl show --timestamp=unix`,
  which older versions lack. Check with `systemctl --version`.
- **glibc 2.34 or newer** for the standard build (Debian 12 has 2.36, Debian 13 has 2.41). The
  static musl build has no glibc requirement.
- About 10 MB of disk for the binary, plus the database, whose size grows with the retention
  settings and the number of disks, interfaces, services and checks.

## Building

The build needs Rust (1.85 or newer, with `cargo`), Node.js and `pnpm`. From a clone of the
repository:

```sh
make release
```

This builds the frontend into `web/dist`, then the optimized binary with the UI embedded:
`target/x86_64-unknown-linux-gnu/release/gargoyle`. The binary is self-contained apart from
glibc; copy it anywhere.

The release profile uses fat LTO, one codegen unit and stripped symbols. It deliberately keeps
`panic = "unwind"`: Gargoyle catches a panic in a collector call, a database write or a request
handler and carries on, which `panic = "abort"` would turn into a crash of the whole service.

### glibc compatibility

A glibc binary runs only on systems whose glibc is at least as new as the symbol versions it
was linked against. Building on a rolling distribution (such as Arch, glibc 2.44 at the time of
writing) is fine today: the release binary requires at most `GLIBC_2.34`. Its only newer
references (`GLIBC_2.39`: `pidfd_spawnp`, `pidfd_getpid`, used by Rust's process spawning) are
weak, so they are skipped on an older glibc rather than refusing to start. Check a build
yourself:

```sh
objdump -T target/x86_64-unknown-linux-gnu/release/gargoyle | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -3
readelf -V target/x86_64-unknown-linux-gnu/release/gargoyle | grep -A30 .gnu.version_r   # WEAK flags
```

Any non-weak version above `GLIBC_2.36` will not run on Debian 12. Three ways to avoid that:

1. **Build on Debian.** Build on the server itself, or in a Debian 12 container or VM, with the
   commands above. This always works.
2. **Build a static musl binary**, which has no glibc dependency at all:

   ```sh
   rustup target add x86_64-unknown-linux-musl     # or your distribution's musl Rust target package
   # plus a musl C compiler for the bundled SQLite and ring: Debian `musl-tools`, Arch `musl`
   make release TARGET=x86_64-unknown-linux-musl
   # -> target/x86_64-unknown-linux-musl/release/gargoyle
   ```

   musl's memory allocator is slower than glibc's, which does not matter at Gargoyle's load.
3. Use the `.deb`, whose `Depends: libc6 (>= 2.34)` makes `apt` refuse an incompatible system
   instead of failing at startup.

### Building the .deb

```sh
cargo install --locked cargo-deb     # once; installs into ~/.cargo/bin
make deb                             # -> target/debian/gargoyle_<version>-1_amd64.deb
```

`make deb` runs `make release` first. Inspect the result with `dpkg-deb -c` and `dpkg-deb -I`
(on non-Debian systems: `ar x <file>.deb && tar -tvf data.tar.xz`). The package contains:

| Path | Notes |
|---|---|
| `/usr/bin/gargoyle` | The binary. |
| `/etc/gargoyle/gargoyle.toml` | Config (conffile, so `apt` keeps your edits on upgrade); `root:gargoyle`, mode `0640`. |
| `/usr/lib/systemd/system/gargoyle.service` | The hardened unit from `packaging/gargoyle.service`. |
| `/usr/share/doc/gargoyle/` | `DEPLOYMENT.md`, `CONFIG.md`, `copyright`. |

## Installing the .deb

Copy the package to the server, then:

```sh
sudo apt install ./gargoyle_0.1.0-1_amd64.deb
```

The package:

- creates the system user and group `gargoyle` (no login shell, no home directory);
- makes `/etc/gargoyle` (mode `0750`) and `/etc/gargoyle/gargoyle.toml` (mode `0640`) readable by
  group `gargoyle` only, with `dpkg-statoverride` so upgrades keep that;
- enables and starts `gargoyle.service`. systemd creates `/var/lib/gargoyle` (owned by
  `gargoyle`, mode `0750`) on first start.

Starting immediately is safe: the shipped config uses the real collector, listens on loopback
only and lists no services. Check it is running:

```sh
systemctl status gargoyle
curl -s http://127.0.0.1:9077/api/v1/health      # {"status":"ok","version":"0.1.0"}
```

Then [configure](#configuring-services-timers-and-checks) what to watch. To look at the dashboard
from your own machine before setting up remote access, use an SSH tunnel:
`ssh -L 9077:127.0.0.1:9077 <server>` and open <http://127.0.0.1:9077>.

## Manual install (without the .deb)

```sh
# 1. User and group
sudo adduser --system --group --no-create-home --home /nonexistent gargoyle

# 2. Binary (built as above, copied to the server)
sudo install -m 0755 gargoyle /usr/local/bin/gargoyle

# 3. Config, readable by root and the gargoyle group only
sudo install -d -m 0750 -o root -g gargoyle /etc/gargoyle
sudo install -m 0640 -o root -g gargoyle gargoyle.example.toml /etc/gargoyle/gargoyle.toml

# 4. Unit, with ExecStart pointing at /usr/local/bin
sed 's|/usr/bin/gargoyle|/usr/local/bin/gargoyle|' gargoyle.service \
  | sudo tee /etc/systemd/system/gargoyle.service >/dev/null
sudo systemctl daemon-reload
sudo systemctl enable --now gargoyle
```

`gargoyle.example.toml` is `config/gargoyle.example.toml` and `gargoyle.service` is
`packaging/gargoyle.service` from the repository. The state directory needs no manual step:
the unit's `StateDirectory=gargoyle` creates `/var/lib/gargoyle` with the right owner.

## Configuring services, timers and checks

Edit `/etc/gargoyle/gargoyle.toml`, validate it, and restart:

```sh
sudoedit /etc/gargoyle/gargoyle.toml
sudo -u gargoyle gargoyle --config /etc/gargoyle/gargoyle.toml check-config
sudo systemctl restart gargoyle
```

Running `check-config` as the `gargoyle` user also proves the service can read the file (and a
`password_hash_file`). Every key is described in [CONFIG.md](CONFIG.md). The common additions:

```toml
[[service]]
unit = "navidrome.service"
name = "Navidrome"
memory_warning_mb = 2048

[[service]]
unit = "nightly-backup.service"     # a oneshot with RemainAfterExit=yes
expect = "active"

[[timer]]
unit = "music-sync.timer"
name = "Music sync"
max_age = "26h"                     # warn when the daily run is missing

[[check]]
name = "Navidrome HTTP"
kind = "http"
url = "http://127.0.0.1:4533/ping"

[[check]]
name = "Game port"
kind = "tcp"
address = "127.0.0.1:25565"
```

Find unit names with `systemctl list-units --type=service` and `systemctl list-timers --all`.
Gargoyle only ever runs `systemctl show` and `journalctl` with fixed read-only arguments; it
cannot start, stop or change anything.

## Enabling authentication

Needed whenever the dashboard is reachable by anyone but you on the server itself, including
through a reverse proxy. Generate the hash with a long random password (20+ characters, from a
password manager):

```sh
# Create the file with its final owner and mode first; tee keeps both when it writes.
sudo install -m 0640 -o root -g gargoyle /dev/null /etc/gargoyle/password
# Ask for the sudo password now, so it does not race hash-password's prompt below.
sudo -v
gargoyle hash-password | sudo tee /etc/gargoyle/password >/dev/null   # prompts twice, no echo
```

Then in `/etc/gargoyle/gargoyle.toml`:

```toml
[auth]
mode = "basic"
username = "admin"
password_hash_file = "/etc/gargoyle/password"
```

and `sudo systemctl restart gargoyle`. Every page, asset and API endpoint now asks for the
password, except `/api/v1/health`, which shows only the version (for uptime monitors). Changing
the password also takes a restart.

Basic auth sends the password with every request, so outside the server use it only over TLS
(a reverse proxy) or a VPN.

## Reverse proxy

Keep Gargoyle on `127.0.0.1:9077` and put a reverse proxy in front for remote access. The proxy
terminates TLS; Gargoyle has none of its own.

**Use a proxy with a client write timeout** (ideally one that also buffers responses) for
anything beyond your LAN. Gargoyle's
HTTP server has no write timeout, so a client that sends requests but never reads the responses
keeps a connection slot, and 256 such clients make the dashboard unreachable (see
[KNOWN_ISSUES](KNOWN_ISSUES.md)). A proxy that reads Gargoyle's (small) responses into its own
buffers and drops slow clients absorbs this; a write timeout at least limits how long each such client holds a
connection.

Gargoyle sets its own security headers, including a strict Content-Security-Policy. Do not add a
second CSP in the proxy, and do not let it inject scripts or styles into pages; either breaks the
UI (see [Troubleshooting](#troubleshooting)). The proxy passes the `Authorization` header through
unchanged by default, in both examples.

### Caddy

Caddy obtains and renews the certificate itself. `/etc/caddy/Caddyfile`:

```caddyfile
{
	servers {
		timeouts {
			read_header 10s
			write 30s
			idle 2m
		}
	}
}

gargoyle.example.com {
	reverse_proxy 127.0.0.1:9077
}
```

Caddy streams responses rather than buffering them, so a client that stops reading can still
hold a Caddy→Gargoyle connection for a large response (such as a long history range). The
`write` timeout limits that to 30 s, which is the mitigation here. Most Gargoyle responses fit in
the kernel's socket buffers, so in practice the effect is small. nginx below buffers fully.

### nginx

```nginx
server {
    # `listen ... http2` works on Debian 12's nginx 1.22; nginx >= 1.25.1 prefers a separate
    # `http2 on;` and only logs a deprecation warning for this form.
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name gargoyle.example.com;

    ssl_certificate     /etc/letsencrypt/live/gargoyle.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/gargoyle.example.com/privkey.pem;

    client_header_timeout 10s;
    client_body_timeout   10s;
    send_timeout          30s;   # drop clients that stop reading

    location / {
        proxy_pass http://127.0.0.1:9077;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_buffering on;      # the default; read responses fully, then serve the client
        proxy_read_timeout 35s;  # Gargoyle times out requests after 30s
    }
}
```

### Auth in the proxy instead

You can leave `auth.mode = "none"` and put authentication (basic auth, SSO, forward auth) in
the proxy. Then anyone with a shell on the server can still reach `127.0.0.1:9077` directly,
which is usually acceptable on a single-user machine.

### Tailscale or WireGuard

On a VPN the network can be the access control, with no proxy:

- **Tailscale Serve** (recommended): leave Gargoyle on loopback and run
  `sudo tailscale serve --bg 9077`. Tailscale provides HTTPS on the tailnet name
  (`https://<machine>.<tailnet>.ts.net`). Add `[auth]` if not everyone on the tailnet should
  see it.
- **Listen on the VPN address**: set `listen = "100.64.0.10:9077"` (your Tailscale or WireGuard
  address) and either configure `[auth]` or set `allow_unauthenticated_remote = true`. Gargoyle
  refuses a non-loopback address without one of the two. The address must exist when Gargoyle
  starts, so order it after the VPN with a drop-in (`sudo systemctl edit gargoyle`):

  ```ini
  [Unit]
  After=tailscaled.service
  Wants=tailscaled.service
  ```

  (`wg-quick@wg0.service` for WireGuard.) The traffic is encrypted by the VPN, so Basic auth
  without TLS is acceptable there.

## Journal access for timer history

Gargoyle shows each timer's past runs. Runs that happen while it is running are always recorded.
To also import runs from before it started (`collector.journal_backfill`, on by default), it
needs to read the system journal, which takes membership of the `systemd-journal` group. That
group can read **all** system logs, so it is opt-in. Grant it to the service only, with a
drop-in:

```sh
sudo systemctl edit gargoyle
```

```ini
[Service]
SupplementaryGroups=systemd-journal
```

```sh
sudo systemctl restart gargoyle
journalctl -u gargoyle | grep -i journal     # "imported timer runs from the journal"
```

Without it, the log shows a single `journal backfill failed` warning at startup, which is
harmless. Set `journal_backfill = false` to silence it.

## Music library report (Liner)

Gargoyle can show the metadata health of a music library: missing or duplicate track numbers,
multi-disc albums without disc tags, split albums, unreadable files and so on. The checks are
done by Liner, a separate read-only linter (its own repository and README). Liner writes a JSON report;
Gargoyle only **reads that file**. It never runs Liner. The library card is never critical:
metadata problems show as warnings, and a missing, unreadable or unsupported report shows as
unknown ([ADR 0007](decisions/0007-liner-report-integration.md)).

The recommended setup runs Liner on the server, against the library copy Navidrome serves
(`/srv/music` below), from a daily systemd timer. Gargoyle then monitors both the timer (did the
scan run?) and the report (what did it find?).

### 1. Install Liner

Build Liner on your build machine (see its README; a release build needs at most `GLIBC_2.34`,
so it runs on Debian 12) and install the binary on the server:

```sh
sudo install -m 0755 liner /usr/local/bin/liner
```

### 2. A system user and the scan service

Use a static system user, **not** `DynamicUser=yes`. With `DynamicUser=yes`, systemd puts
`StateDirectory=` under `/var/lib/private/`, which only root can enter, so the `gargoyle` user
could not read the report through the `/var/lib/liner` link.

```sh
sudo adduser --system --group --no-create-home --home /nonexistent liner
```

The library must be readable by `liner`; world-readable files are enough. If they are not, add
`liner` to a group that can read them (for example `sudo adduser liner navidrome`).

`/etc/systemd/system/liner-scan.service`:

```ini
[Unit]
Description=Liner music library metadata report

[Service]
Type=oneshot
User=liner
Group=liner
# /var/lib/liner, owned by liner, readable by gargoyle; the report is written with mode 0644.
StateDirectory=liner
StateDirectoryMode=0755
UMask=0022
ExecStart=/usr/local/bin/liner scan /srv/music --format json \
    --output /var/lib/liner/report.json --fail-on never --config /dev/null
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
PrivateNetwork=yes
NoNewPrivileges=yes
CapabilityBoundingSet=
Nice=10
IOSchedulingClass=idle
```

- **`--fail-on never` is required.** Without it Liner exits 1 whenever the library has an
  error-level issue or an unreadable file (its default is `--fail-on error`), and the timer shows
  as failed although the scan worked. Real failures (an unreadable library
  root, an unwritable report) still exit 2 and still fail the unit.
- Do not add `--min-severity`: issues it hides are hidden from Gargoyle too. (The Library page
  says how many were hidden.)
- `--config /dev/null` keeps the run independent of any Liner user config.
- Liner writes the report to a temporary file and renames it into place, so Gargoyle never reads
  a half-written report. The report must be outside the library; `--output` refuses otherwise.
- With `ProtectHome=yes` the library must not be under `/home`. If it is, use
  `ProtectHome=read-only` instead.

### 3. The timer

`/etc/systemd/system/liner-scan.timer`:

```ini
[Unit]
Description=Daily Liner music library report

[Timer]
OnCalendar=*-*-* 05:00
RandomizedDelaySec=30m
Persistent=true

[Install]
WantedBy=timers.target
```

Pick an `OnCalendar=` time after your daily library sync has normally finished, so the report
describes the synced state. Then:

```sh
sudo systemd-analyze verify /etc/systemd/system/liner-scan.service /etc/systemd/system/liner-scan.timer
sudo systemctl daemon-reload
sudo systemctl enable --now liner-scan.timer
sudo systemctl start liner-scan.service          # first report now, instead of tomorrow
ls -l /var/lib/liner/report.json                 # -rw-r--r-- 1 liner liner ...
sudo -u gargoyle head -c 200 /var/lib/liner/report.json   # proves gargoyle can read it
```

### 4. Configure Gargoyle

Add to `/etc/gargoyle/gargoyle.toml` (every key is described in
[CONFIG.md](CONFIG.md#library)):

```toml
[library]
report = "/var/lib/liner/report.json"
name = "Music library"
max_age = "26h"        # a daily timer plus its random delay, with margin
warn_on = "error"      # or "warning" to be told about every inconsistency

[[timer]]
unit = "liner-scan.timer"
name = "Library scan"
max_age = "26h"
```

Validate and restart as in [Configuring](#configuring-services-timers-and-checks). The
dashboard gets a **Library** entry in the navigation and a library card on the overview. The
`[[timer]]` entry turns critical if a scan fails; the library card turns warning if the report
gets older than `max_age`, so a timer that silently stopped is noticed too.

### Alternative: scan elsewhere and copy the report

If the master copy of the library lives on another machine (a desktop that syncs to the server),
run Liner there and copy only the report to the server, for example from a timer or cron job on
that machine:

```sh
liner scan ~/Music --format json --output ~/.cache/liner-report.json --fail-on never
rsync --chmod=F644 ~/.cache/liner-report.json server:/var/lib/liner/report.json
```

On the server, create `/var/lib/liner` once (`sudo install -d -m 0755 -o <your user> /var/lib/liner`)
so your user can write it and `gargoyle` can read it. Do not use `rsync --inplace`: plain rsync
writes a temporary file and renames it, like Liner. The age Gargoyle shows is the age of the
scan (`generated_at`), not of the copy, so a copy job that keeps sending an old report is
noticed too. The report then describes the desktop's copy, so paths match the server's only if
the two libraries are laid out the same way.

## Upgrading

Back up the database first (next section). Then:

- **.deb**: `sudo apt install ./gargoyle_<new>_amd64.deb`. The service restarts by itself. If you
  edited `/etc/gargoyle/gargoyle.toml` and the new version's example changed too, `apt` asks
  which to keep; keep yours, and compare with the new example later.
- **Manual**: replace the binary (`sudo install -m 0755 gargoyle /usr/local/bin/gargoyle`), then
  `sudo systemctl restart gargoyle`.

The database schema migrates automatically at startup. An older version refuses to open a
database written by a newer one ("database schema version N is newer than this build
supports"), so to downgrade, restore the backup taken before the upgrade.

## Backup

The database is the only state; the config lives in `/etc/gargoyle`. Either take a consistent
copy while Gargoyle runs (requires the `sqlite3` package):

```sh
sudo sqlite3 /var/lib/gargoyle/gargoyle.db ".backup '/root/gargoyle-$(date +%F).db'"
```

or stop it and copy the files (including `-wal` and `-shm` if present):

```sh
sudo systemctl stop gargoyle
sudo cp -a /var/lib/gargoyle/. /root/gargoyle-backup/
sudo systemctl start gargoyle
```

To restore: stop Gargoyle, replace `/var/lib/gargoyle/gargoyle.db` with the backup (and delete
any `gargoyle.db-wal` and `gargoyle.db-shm`), `sudo chown gargoyle:gargoyle` it, and start.
Losing the database loses only history.

## Uninstalling

- **.deb**: `sudo apt remove gargoyle` stops the service and removes the program, keeping the
  config and the database. `sudo apt purge gargoyle` also removes `/var/lib/gargoyle` and the
  shipped config. Files you added to `/etc/gargoyle` (such as `password`) are left; delete them
  by hand. As is usual for Debian packages, the `gargoyle` system user and group are kept (files
  may still belong to them, and a reinstall reuses them). To remove them too, after the purge:
  `sudo rm -rf /etc/gargoyle && sudo deluser --system gargoyle && sudo delgroup --system gargoyle`.
- **Manual**:

  ```sh
  sudo systemctl disable --now gargoyle
  sudo rm /etc/systemd/system/gargoyle.service /usr/local/bin/gargoyle
  sudo rm -r /etc/systemd/system/gargoyle.service.d     # if you made drop-ins
  sudo systemctl daemon-reload
  sudo rm -r /var/lib/gargoyle /etc/gargoyle
  sudo deluser --system gargoyle
  sudo delgroup --system gargoyle
  ```

## Troubleshooting

Start with the log: `journalctl -u gargoyle -e`. Gargoyle logs to stderr, which journald
captures; `RUST_LOG=debug` (set with `systemctl edit gargoyle`, `Environment=RUST_LOG=debug`)
shows more.

| Symptom | Cause and fix |
|---|---|
| Fails to start: `config error(s)` | The config is invalid; every problem is listed. Run `check-config` as above. Unknown keys (typos) are errors too. |
| Fails to start: permission denied reading the config or password file | The file must be readable by group `gargoyle`: `sudo chown root:gargoyle <file> && sudo chmod 0640 <file>`, and `/etc/gargoyle` must be `0750 root:gargoyle`. |
| Fails to start: cannot open the database | `storage.path` must be inside a directory the `gargoyle` user can write. With the unit's `ProtectSystem=strict`, that is only `/var/lib/gargoyle`; a different path also needs `ReadWritePaths=` in a drop-in. |
| Fails to start: `refusing to expose the dashboard without authentication` | `listen` is not loopback and `[auth]` is off. Enable auth, use loopback plus a proxy, or set `allow_unauthenticated_remote = true` on a VPN-only address. |
| Fails to start: address already in use | Another process has the port: `sudo ss -ltnp 'sport = :9077'`. Change `listen` or stop the other process. A VPN address that does not exist yet gives "cannot assign requested address"; see [Tailscale](#tailscale-or-wireguard). |
| A service shows **not found** | The unit name is wrong or not loaded: compare with `systemctl list-units --all <name>`. Names are case-sensitive and need their suffix (`.service`). |
| All services show **unknown** and the log has `systemctl` errors | `systemctl show` failed. Check `systemctl --version` is 252 or newer (`--timestamp=unix`). If you changed the unit, make sure it still allows `AF_UNIX` (D-Bus) and does not set `PrivateUsers=`. |
| Timers have no history from before startup; log says `journal backfill failed` | The service cannot read the journal. See [Journal access](#journal-access-for-timer-history). |
| The library card shows **report not found** or **cannot read report: permission denied** | Check the path in `library.report`, and that `sudo -u gargoyle head -c1 <path>` works: every directory on the way needs `x` for others (or the `gargoyle` group) and the file `r`. A Liner unit with `DynamicUser=yes` hides its `StateDirectory=` from other users; use a static user as in [Music library report](#music-library-report-liner). |
| The library card shows **unsupported Liner schema 2.0.0** | Liner was upgraded to a report format this Gargoyle does not know. Upgrade Gargoyle, or keep the older Liner until a Gargoyle release supports it. |
| The library card shows **report is 3d old** | The scan has not run. Look at the `liner-scan.timer` card and at `journalctl -u liner-scan.service`. |
| A `/home` or other disk is missing | Auto-detection skips pseudo filesystems and lists each device once. List mount points explicitly in `system.disks`. |
| The page is blank or unstyled; the browser console reports CSP violations | Something is adding inline scripts or styles: a proxy that injects content, or a second `Content-Security-Policy` header from the proxy. Remove it; Gargoyle's own policy allows only its own files. Browser extensions can cause the same messages and are harmless. |
| The browser keeps asking for the password | Wrong user name or password, or the hash does not match. Generate a new hash and restart. Many failed attempts in a short time make Gargoyle refuse new logins for a few seconds. |
| `401` behind the proxy although the password is right | The proxy strips or replaces the `Authorization` header (for example its own basic auth). Use auth in only one place. |
