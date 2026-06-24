---
type: ADR
id: "0036"
title: "`phai serve` bridge: loopback + Host/Origin guard as the security boundary, no per-request auth"
status: active
date: 2026-06-24
---

## Context

`phai serve` runs a local HTTP + WebSocket bridge that the embedded LiveStore
web app drives ([serve.rs](../../crates/phai-cli/src/serve.rs);
[ADR-0019](0019-serve-loopback-only-localhost-alias.md),
[ADR-0023](0023-web-app-on-livestore-client-only.md)). The bridge is the
BigQuery/SQLite system of record: its `/api` surface includes state-changing
endpoints — `/api/activate` writes the BigQuery config **and a service-account
key** to disk, `/api/sync` spawns a `phai sync` subprocess, and the forecast
routes (`/api/forecast`, `/api/forecast/delete`, `/api/forecast/move`,
`/api/forecast/settle`, `/api/forecast-template/accept`,
`/api/forecast-template/dismiss`, `/api/events`) mutate persisted financial
state.

None of these endpoints require a per-request credential — no bearer token, no
session cookie, no API key. A security audit (finding **S2**) flagged this: any
local process, or any browser page the user happens to open, can in principle
reach `127.0.0.1:<port>` and drive these routes.

This is an **intentional** design for a single-user, loopback-only finance app,
but the decision and its threat model were never written down. The forces:

- The app is single-user and same-machine by construction. The bridge binds
  only to `127.0.0.1` (`LOCAL_BIND_HOST`), per ADR-0019 — there is no remote
  network surface to authenticate.
- A static, locally-stored auth token would add ceremony (where does the browser
  get it? where is it stored?) without raising the bar against the one attacker
  it could plausibly stop — a malicious local process — because that process
  already has the user's filesystem and could read the token (or the BigQuery
  key, or the SQLite DB) directly.
- The real browser-borne threats are **cross-site request forgery** (a page on
  another origin POSTing to the bridge) and **DNS rebinding** (a remote page
  rebinding its hostname to `127.0.0.1` to slip past the loopback bind). These
  are addressable without per-request auth.

## Decision

**The `phai serve` bridge uses its loopback-only bind plus a `Host` pin, an
`Origin`/CSRF allowlist, and baseline security headers as the security boundary —
not a per-request auth token.** A malicious *local* process on the same machine
is explicitly out of scope, because it already holds the user's filesystem,
config, and credentials.

The boundary is enforced by middleware layered over every `/api` route in `run()`
([serve.rs](../../crates/phai-cli/src/serve.rs)), composed of:

- **Loopback-only bind.** The listener binds `127.0.0.1` (`LOCAL_BIND_HOST`);
  nothing off-host can connect (ADR-0019).
- **`Host` pin (anti-DNS-rebinding).** `guard_origin` rejects any request whose
  `Host` header is not a loopback name. `is_host_allowed` → `host_allowed`
  accepts only `127.0.0.1`, `localhost`, and `phai.localhost` (port stripped); a
  missing `Host` is allowed for HTTP/1.0 / direct-socket integrations, which
  browsers never produce. This closes the rebinding hole that the `Origin` check
  alone misses, since same-origin browser **GET**s carry no `Origin` header.
- **`Origin`/CSRF allowlist.** `is_origin_allowed` permits only origins under
  `http://localhost:`, `http://127.0.0.1:`, `http://phai.localhost:`, or bare
  `http://phai.localhost` (port-80 case). A missing `Origin` (curl, direct
  integration) is allowed. Any cross-site `Origin` is rejected, so a foreign page
  cannot drive the bridge through the user's browser.
- **Baseline security headers.** `security_headers` is the outermost layer, so
  the headers ride on every response even when an inner layer short-circuits. It
  sets `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
  `Referrer-Policy: no-referrer`, `Permissions-Policy: interest-cohort=()`, and a
  Content-Security-Policy:

  ```
  default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:;
  font-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self';
  frame-ancestors 'none'
  ```

  `frame-ancestors 'none'` (with `X-Frame-Options: DENY`) blocks clickjacking;
  `object-src 'none'` and `base-uri 'self'` cut common injection vectors;
  `form-action 'self'` keeps form posts same-origin.

## Options considered

- **Loopback bind + Host/Origin guard + security headers, no per-request auth**
  (chosen): zero credential ceremony for a same-machine single user; defends the
  realistic threats (remote access, rebinding, cross-site CSRF). Cost: offers no
  defense against a malicious local process — accepted, because such a process
  already owns everything the bridge could protect.
- **Static local auth token (bearer / cookie)**: the browser fetches a token the
  CLI prints or writes to a file, then sends it on every `/api` call. Adds real
  ceremony and a token-distribution problem, yet a local attacker can read the
  token from disk/process memory just as easily as the data — so it raises
  ceremony without raising the security floor. Rejected as the *boundary*; noted
  below as optional defense-in-depth.
- **OS-level auth (Unix socket + peer credential / `SO_PEERCRED`)**: genuinely
  scopes access to the user's processes, but breaks the `http://phai.localhost`
  browser model (ADR-0019), needs a localhost↔socket shim, and still does not
  distinguish the user's own benign processes from a malicious one running as the
  same user. Disproportionate for a single-user app.
- **Bind to a random high port + secret in the URL**: capability-style secrecy,
  but the URL leaks via `Referer`, shell history, and process listings, and it
  fights the stable `phai.localhost` URL. Rejected.

## Consequences

**Protected (in scope, defended):**

- **Remote network access** — loopback-only bind means nothing off the machine
  can reach the bridge (ADR-0019).
- **DNS rebinding** — the `Host` pin (`is_host_allowed`/`host_allowed`) rejects
  any non-loopback `Host`, covering the read path that the `Origin` check cannot.
- **Cross-site browser CSRF** — the `Origin` allowlist (`is_origin_allowed`)
  rejects state-changing requests carrying a foreign `Origin`.
- **Clickjacking / framing / sniffing** — `security_headers` (CSP
  `frame-ancestors 'none'`, `X-Frame-Options: DENY`, `nosniff`).

**Explicitly accepted (out of scope):**

- **A malicious *local* process on the same machine.** Any process running as the
  user can connect to `127.0.0.1:<port>` and call `/api/sync`, `/api/forecast`,
  even `/api/activate`. We do not defend against this, because that process
  already has the user's filesystem, the BigQuery service-account key, and the
  SQLite database — an in-process auth token would not change its reach. The
  threat model treats the local user account as the trust boundary.

**Invariants the codebase must hold:**

- Every `/api` route stays behind the `guard_origin` and `security_headers`
  layers in `run()`. A new route added outside that layered router would bypass
  the boundary — new endpoints must be registered inside the guarded `Router`.
- The bind address stays `LOCAL_BIND_HOST` (`127.0.0.1`); exposing the bridge to
  a non-loopback interface would invalidate this entire model and requires a new
  ADR with an authentication design (see ADR-0019).
- The loopback allowlists in `host_allowed` and `is_origin_allowed` must stay in
  sync with the hosts the app is actually served on (`LOCAL_APP_HOST`).

**Future hardening (non-blocking):**

- A double-submit **CSRF token** (or a custom-header requirement on
  state-changing routes) would add defense-in-depth against any future gap in the
  `Origin` check, without changing the trust boundary. This is optional and not
  required by this decision.
- If `phai serve` ever grows multi-user, LAN, or daemon exposure, this ADR is
  superseded: that path needs real per-request authentication and a fresh threat
  model. Related: [ADR-0028](0028-launchd-agent-and-launcher-app.md),
  [ADR-0029](0029-serve-system-daemon-admin-auth.md), and the multi-device
  activation flow in [ADR-0034](0034-self-contained-encrypted-invite.md) (which
  introduced `/api/activate` and treats the invite blob — not a bridge token — as
  the secret).

**Re-evaluation triggers:** any non-loopback bind; a shift to multi-user or
shared-machine use; or an audit finding that a same-user local process is no
longer an acceptable trust boundary.
