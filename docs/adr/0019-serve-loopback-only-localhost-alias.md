---
type: ADR
id: "0019"
title: "`fin serve` uses loopback-only binding with a localhost alias"
status: accepted
date: 2026-05-26
---

## Context

ADR-0018 introduced `fin serve` with a `--host` option that could expose the
dashboard outside the local machine. The intended primary use is a local browser
session, and write operations are available through the WebSocket API. Keeping a
remote bind option in the default CLI surface creates an easy path to accidental
LAN exposure.

Browsers also treat `*.localhost` as local loopback context, so the dashboard can
use a stable friendly URL without mDNS or LAN discovery.

## Decision

**`fin serve` binds only to `127.0.0.1` and presents
`http://meuapp.localhost:<port>` as the browser URL.** The CLI keeps `--port`
but removes `--host`; `meuapp.localhost` is accepted as a WebSocket Origin.

## Options considered

- **Loopback-only bind with localhost alias** (chosen): keeps the dashboard local
  and supports secure-context browser APIs without HTTPS.
- **Keep `--host` opt-in exposure**: flexible for LAN use, but makes accidental
  unauthenticated remote access easier.
- **Use mDNS `.local` discovery**: useful for LAN apps, but not needed for a
  same-machine dashboard and incompatible with the `.localhost` intent.

## Consequences

- `fin serve` is local-only by construction.
- LAN access requires a future explicit design with authentication and a new ADR.
- Local clients may use `/api` for a simple status check and `/ws` for dashboard
  operations.
