# ADR-0028: `phai serve install` — launchd agent + launcher app, not a daemon

- Status: active (partially supersedes ADR-0001's "no daemon" stance)
- Date: 2026-06-12

## Context

ADR-0001 rejected shipping phai as a daemon or a Tauri app: one self-updating
binary, no resident processes of our own. That stance still holds — but for a
non-technical household member the web app must (a) already be running and
(b) be one click away, or it does not exist. "Open a terminal and run
`phai serve`" is a non-starter for that audience.

## Decision

`phai serve install [--port N]` writes two artifacts under `$HOME` (macOS):

1. **launchd agent** `~/Library/LaunchAgents/run.phai.serve.plist` —
   `RunAtLoad` + `KeepAlive`, logs to `~/Library/Logs/phai/serve.log`.
   Supervision belongs to the OS: we still ship zero resident processes of
   our own, so this does not reopen ADR-0001's daemon question. The plist
   points at the absolute path of the current binary — the self-updater
   (ADR-0007) replaces that path atomically, so the agent always launches
   the latest installed version. `PHAI_CONFIG_DIR`/`FINANCE_OS_*` values in
   effect at install time are captured into the plist, freezing which store
   the agent serves.
2. **Launcher app** `~/Applications/Phai.app` — a minimal bundle (Info.plist,
   φ icon, a `/bin/sh` exec of `/usr/bin/open <url>`) so the web app is
   visible in Launchpad/Spotlight and pinnable to the Dock. It is a bookmark
   with an icon, not a runtime: no webview, no Tauri, nothing to update
   beyond the URL it opens (port 80 → `http://phai.localhost/`, otherwise
   `http://localhost:N/`).

`phai serve uninstall` boots the agent out and removes both artifacts.
Install is idempotent (bootout-then-bootstrap; bundle rewritten in place).
Both commands are macOS-only and fail with a clear message elsewhere;
a systemd-user variant is the obvious Linux follow-up when needed.

## Consequences

- "Install phai for a family member" becomes: run the installer, run
  `phai auth setup`, run `phai serve install` — after that the app survives
  reboots and lives in Launchpad with an icon.
- The launcher bundle is unsigned and built locally at install time, so
  Gatekeeper does not quarantine it (no download bit). If macOS hardening
  ever tightens around script-executable bundles, the fallback is generating
  the bundle through `osacompile` instead.
- The φ icon ships inside the binary (`crates/phai-cli/assets/Phai.icns`,
  ~0.8 MB) — the only binary asset in the crate; regenerate from brand assets
  if the mark changes.
- Port changes require re-running `serve install --port N` (rewrites both
  artifacts); the agent label `run.phai.serve` stays singleton by design.
