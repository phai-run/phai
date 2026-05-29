---
type: ADR
id: "0021"
title: "Web app on LiveStore (client-only) bridged to the store"
status: accepted
date: 2026-05-29
---

## Context

`phai serve` ([ADR-0018](0018-serve-local-dashboard.md), [ADR-0019](0019-serve-loopback-only-localhost-alias.md)) shipped a hand-rolled single-file HTML dashboard driven by a bespoke WebSocket protocol. In parallel, two terminal UIs (the review-queue TUI and the cashflow TUI, on `ratatui`/`crossterm`) carried the interactive review experience. The review TUI is being discontinued; `phai serve` is its migration target. We want one interactive surface, brought to the [DESIGN.md](../../DESIGN.md) brand, that absorbs the TUI's review workflow.

The constraint is [ADR-0001](0001-single-binary-rust-cli.md): phai is a single statically-linked Rust binary — no daemon, no sidecar, and **no build step on the user's machine**. `phai serve` is an on-demand (not resident) process, which is compatible; the dashboard HTML is embedded at compile time.

phai's system of record is the `FinanceStore` (SQLite local / BigQuery production), populated by Pluggy syncs. It is **not** event-sourced.

LiveStore (livestore.dev) is a local-first, event-sourced, reactive SQLite data layer for the browser. Its own multi-client sync requires a sync backend speaking an under-documented, still-churning (0.3→0.4) WebSocket protocol — impractical to reimplement in Rust, and a Node sync sidecar would break the single-binary model.

## Decision

**Rebuild the `phai serve` UI as a client-only LiveStore + React (Vite) app, embedded in the binary, bridged to the existing store over a plain Rust REST API.**

- **Client-only LiveStore.** The app runs entirely in the browser: OPFS-persisted, event-sourced, reactive. **No LiveStore sync backend is configured.** Source lives under `crates/phai-cli/web/`.
- **The Rust bridge is the system-of-record seam.** `phai serve` exposes `/api/*`: reads project `FinanceStore` data into LiveStore (seed events); writes (a review submitted, a forecast upserted) are committed locally for an instant reactive UI, queued in a `pendingWrites` table, and flushed by a background client task to `POST /api/events`, which applies them via the existing domain functions (`apply_human_review`, forecast upsert) with an `AuditEvent`. This replaces the bespoke WebSocket protocol with REST.
- **Embedded bundle.** The built SPA (`web/dist`) is embedded with `include_dir!` and served by `phai serve`; `web/dist` is committed so `cargo build` stays pure-Rust. The JS build (Vite, pnpm) runs only in CI / locally — never on the end user's machine.
- **Isolation preserved.** Loopback-only binding and the `meuapp.localhost` alias from [ADR-0019](0019-serve-loopback-only-localhost-alias.md) are kept. Responses carry `Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy: credentialless` so LiveStore's OPFS worker gets a `crossOriginIsolated` context while still allowing cross-origin no-cors subresources (web fonts).

## Options considered

- **Client-only LiveStore + Rust bridge** (chosen): keeps the single binary, gives a local-first reactive UX, and reuses the audited write paths. Trade-off: not "real" LiveStore multi-device sync, and BigQuery→client updates are poll-based (no server push for Pluggy-driven changes).
- **LiveStore sync backend in Rust**: true sync, but the protocol is under-documented and unstable across versions — high risk, ongoing maintenance burden.
- **Node sync sidecar (`@livestore/sync-cf` etc.)**: real sync with less protocol work, but a resident Node runtime breaks ADR-0001's single-binary install. Rejected.
- **Keep the hand-rolled HTML + WebSocket dashboard**: no new toolchain, but no offline-capable local-first model, weaker UX, and it duplicates rather than replaces the TUI.

## Consequences

- **Easier**: a brandable, reactive, offline-tolerant UI; the review workflow leaves the terminal; audited writes are reused unchanged.
- **Harder / new**: an in-repo JS/TS toolchain (Vite, pnpm) and a CI step that builds the SPA and fails if committed `web/dist` is stale; the binary grows by a few MB (React + wa-sqlite WASM).
- **Poll, not push**: changes that land in BigQuery out-of-band (nightly Pluggy sync) surface on the next client refresh, not via live push. A push path would need a future design + ADR.
- **Re-evaluation trigger**: if real multi-device sync becomes a requirement, revisit the sync-backend options (and ADR-0001).
- Follow-on: the review/cashflow TUIs, the `report cashflow --tui` flag, and the `report review` static export are removed in a subsequent change; `ratatui`/`crossterm` drop from the dependency graph.
