# 0005 — Localhost by default; optional HTTP Basic auth with argon2id; TLS via proxy

**Status:** Accepted · 2026-11-08

## Context

The dashboard reveals host details, which is low but non-zero sensitivity. It must be safe by
default, while still letting the owner reach it remotely.

## Decision

- **Default bind is `127.0.0.1:9077`.** Gargoyle refuses to start on a non-loopback address with
  `auth.mode = "none"` unless `allow_unauthenticated_remote = true` is set. That override exists for
  VPN-only interfaces such as Tailscale, where the network is the auth.
- **Auth uses HTTP Basic with an argon2id password hash.** It is a single user and is configured as
  a PHC string from `gargoyle hash-password`.
  - The browser's native prompt means no login page, no sessions, no cookies and no CSRF surface.
    This fits a read-only, GET-only app.
  - Successful verification is cached (SHA-256 of the header value, kept in memory) so 10s polling
    does not run argon2 on every request.
  - Failures get a 1s delay.
  - `/api/v1/health` stays open for uptime probes and reveals nothing beyond the version.
- **No built-in TLS.** Basic auth over plain HTTP outside localhost is only acceptable on a VPN. The
  documented setup for internet exposure is Caddy or nginx with TLS in front. This avoids
  certificate management code and `rustls` server configuration in Gargoyle.
- **Bearer tokens are deferred.** They are only needed for the future agent → hub ingest
  (ARCHITECTURE §10).

**Alternatives considered:** session cookies with a login form, which needs CSRF handling, session
storage and a UI. Delegating to the proxy only (forward auth) is still possible: the user can leave
Gargoyle on localhost and put auth in the proxy.

## Consequences

- The logout experience is browser-dependent, which is acceptable for a personal tool.
- There is a single user, with no roles. It is read-only anyway.
