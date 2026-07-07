---
type: ADR
id: "0040"
title: "Single production daemon on port 80"
status: active
date: 2026-07-07
---

## Context

The desktop shell and household devices had drifted into two simultaneous production roles: a user-scoped `phai serve` on port 4317 for the desktop app, and a root/system daemon on port 80 for LAN access. Each long-running process kept its own in-memory version and passive update status, so a CLI self-update could refresh the binary on disk while one or both daemons still reported stale versions until their next background check or restart.

TCP bind already prevents two live servers on the exact same port, but that happens too late for clean production replacement: a new port-80 daemon should be able to ask the previous production daemon to exit before attempting to bind. High ports are still useful for development, preview and tests, and debug builds must not kill a user's installed production daemon.

This is ADR-worthy because it changes the production topology, the desktop app's backend target, and the cross-cutting process ownership invariant for `phai serve`.

## Decision

**Production has exactly one official `phai serve` per machine: the daemon on port 80.** The desktop app and LAN devices both talk to that daemon, while high ports such as 4317, 4318 and 8080 are reserved for development, preview and tests.

Release builds on Unix acquire a production serve lock before binding port 80. The lock file lives at `data_dir/serve-80.lock`, records the owning PID/version/start time, and allows a new production daemon to take over by sending SIGTERM to the previous live PID, escalating to SIGKILL only if graceful shutdown does not complete quickly. Debug builds and non-port-80 serves do not participate in the singleton and rely only on normal TCP bind behavior. Acquiring the lock itself is serialized by a short-lived `.lock.acquire` guard file; if that guard is older than 30s (its owner was SIGKILLed or the machine lost power before it could clean up), a new instance removes it and proceeds rather than deadlocking forever.

Since binding port 80 always requires root, `phai serve install` now routes *any* install that resolves to port 80 through the root LaunchDaemon path, regardless of whether `--system` was passed — the flag becomes explicit-only for forcing a system install on a non-default port. Only an explicit high `--port` (dev/preview) still installs the no-sudo user agent.

The app-triggered update path performs an on-demand GitHub check before deciding whether to update, so clicking the version in the desktop UI is not constrained by the passive background check interval.

## Options considered

- **Single port-80 production daemon** (chosen): one installed production process serves the local desktop shell, localhost browser access and LAN devices. This removes duplicated long-running version state and gives `phai serve install` one production target. It requires privileged binding on systems that reserve low ports, so the supported production install remains the existing daemon/bootstrap flow.
- **Keep separate desktop port 4317 plus LAN port 80**: preserves the historical no-sudo desktop pairing path, but keeps two production processes, two update states and ambiguity about which process is canonical.
- **Make every port participate in a global singleton**: prevents more accidental concurrent servers, but breaks development and test workflows where multiple high-port serves are intentionally run side by side.
- **Depend only on TCP bind conflicts**: simple, but a replacement daemon cannot gracefully stop a stale port-80 process before bind, so the new process just crashes with `address already in use`.

## Consequences

- `phai serve install` must treat port 80 as the default production target. Explicit high-port installs remain development/preview choices, not canonical production roles.
- The desktop launcher/app must target `http://localhost` / `http://127.0.0.1` / `http://phai.localhost` on implicit port 80 in production, not `:4317`.
- `4317` is no longer a production singleton role. It may still be used by `cargo run -p phai-cli -- serve --port 4317` and similar local development commands.
- Release builds on Unix must acquire `serve-80.lock` before TCP bind and release it best-effort on shutdown only if the lock still belongs to the current PID.
- Debug builds must not acquire the production lock or signal the installed daemon; if debug tries to bind an occupied port, it should fail with the ordinary bind error.
- Automatic takeover makes stale production daemons self-healing, but it intentionally accepts the small initial risk of PID reuse. If that becomes observable, the lock should be extended with process start-time validation.
- The update endpoint remains idempotent and serializes update application so concurrent UI clicks do not trigger multiple downloads/replacements.
