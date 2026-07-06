---
type: ADR
id: "0039"
title: "Native macOS desktop shell (Pake/Tauri WKWebView) over the local serve app"
status: accepted
date: 2026-07-06
---

## Context

`phai serve` already runs a local-only web app (LiveStore + React embedded in
the binary, `/api` bridge to the BigQuery/SQLite system of record). The current
"desktop app" is a launcher that opens the app in a Chromium browser's `--app`
mode (ADR-0001 amendment, shipped v5.36.0): chromeless, own Dock icon, but still
the user's browser, and only if a Chromium-family browser is installed. On a
Safari-only Mac it falls back to a browser tab.

The user wants a **real** desktop app: its own icon and window, native chrome,
not "Chrome opening". [Pake](https://github.com/tw93/Pake) builds exactly this —
a thin [Tauri](https://tauri.app) app that wraps a URL in a native **WKWebView**
window. On macOS Tauri/Pake use WKWebView (WebKit), the same engine as Safari.

This is a structural change: it ships a GUI desktop app as a second
distributable, which ADR-0001 ("the CLI is the only product surface, no GUI")
explicitly excluded. ADR-0001's re-evaluation trigger — "the daily reading
surface stops being WhatsApp/CLI" — is met: the web app is now a primary surface,
and a native window is the natural next step.

The make-or-break risk was whether the LiveStore SPA even runs in WKWebView.
LiveStore needs OPFS, web workers, a SharedWorker, wa-sqlite (WASM), and a
**crossOriginIsolated** context (SharedArrayBuffer). A Swift WKWebView probe
against the real SPA + real BigQuery data found:

- `credentialless` (the COEP value `serve_assets.rs` shipped) → **not honored by
  WebKit** → `crossOriginIsolated = false` → no SharedArrayBuffer → wa-sqlite
  breaks. This is why a Safari-only Mac could not run the app.
- `require-corp` → `crossOriginIsolated = true`, SharedArrayBuffer available,
  SharedWorker/OPFS/Worker all present, the SPA mounts and renders real data in
  WKWebView with **zero console errors**.

`require-corp` requires every subresource to be same-origin or carry a CORP
header. The only cross-origin subresource was Google Fonts (Inter, JetBrains
Mono, Space Grotesk), which is exactly why `credentialless` had been chosen.

## Decision

**phai gains a native macOS desktop shell built with Pake/Tauri that loads the
local `phai serve` app in a WKWebView window with its own icon and title.** The
web app, the `/api` bridge, and the launchd service are unchanged — the shell is
a thin native window over the same local server (system of record stays the
Rust binary).

**Enabling change (this ADR's first landed piece):** switch cross-origin
isolation from `cross-origin-embedder-policy: credentialless` to
`require-corp`, and **self-host the fonts** via `@fontsource` (bundled woff2,
same-origin) so there are no cross-origin subresources. This is applied in both
`serve_assets.rs` (production) and `vite.config.ts` (dev), and it also fixes the
app on Safari-only Macs in the current browser flow.

The shell itself lands in later phases: Pake build wired into CI, the `.app`
shipped as a release asset, `phai serve install` laying it into `~/Applications`
and pinning it to the Dock (replacing the Chromium-`--app` shell script), a
fixed loopback port for pairing, and startup/version-drift handling.

Scope of the first cut: **macOS only** (matches `phai serve install`), and the
`.app` is **unsigned** — first launch is right-click → Open → Open, consistent
with `Instalar Phai.command` today (see `docs/notarization.md`). Linux/Windows
shells and notarization are backlog.

## Options considered

- **Pake/Tauri shell over the local server (chosen)** — native window + icon,
  WKWebView, reuses the entire existing web app + bridge. Requires the
  `require-corp` + self-hosted-fonts change. Ships a second (unsigned)
  distributable that depends on the launchd service already running.
- **Keep Chromium `--app` mode only** — zero new distributable, but it is the
  user's browser, needs a Chromium browser installed, and never feels fully
  native. Rejected: does not meet "own window, not Chrome".
- **Full Tauri rewrite** — embed the SPA as Tauri assets, replace the `/api`
  HTTP bridge with Tauri commands. Much larger surface, abandons the
  single-binary serve architecture, and does not use Pake. Rejected as
  disproportionate.

## Consequences

- **Easier**: a genuinely native app surface; the app now runs in any WebKit
  context, so the browser fallback works on Safari-only Macs too.
- **Harder**: a second macOS distributable to build (Pake needs Rust + Node +
  Tauri CLI, CI-only — no build on the user's machine, preserving ADR-0001's
  invariant), keep in version-sync with the CLI, and ship unsigned (Gatekeeper
  friction until notarized).
- **Invariants**: the web app must have **no cross-origin subresources** — all
  assets same-origin or CORP-tagged — or `require-corp` will block them. New
  fonts/images/scripts must be self-hosted (or proxied) going forward.
- **Re-evaluation triggers**: WebKit shipping `credentialless` support (would let
  us relax CORP); a decision to notarize (removes the right-click-open step); or
  extending the shell to Linux/Windows (needs systemd/Windows-service equivalents
  of the launchd pairing).
