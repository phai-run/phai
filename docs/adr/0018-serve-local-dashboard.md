# ADR-0018 — `fin serve`: local HTTP+WebSocket dashboard

**Status:** Accepted  
**Date:** 2026-05-26  
**Deciders:** Felipe R. Broering

---

## Context

The forecast review workflow (ADR-0016) produces proposed templates and forecast records that operators must accept or dismiss. The existing CLI commands (`forecast template list`, `forecast accept`) are efficient for scripting but require remembering subcommand names and re-running commands after each action. An interactive view over the cashflow chart — showing which months are over- or under-committed — was also missing.

---

## Decision

Add `fin serve [--port 8080] [--host 127.0.0.1]` — an HTTP server that:

- Serves a single-page dashboard (`serve_dashboard.html`) embedded in the binary via `include_str!`.
- Exposes a WebSocket endpoint at `/ws` for request-response communication with the dashboard.
- Uses a **store actor** pattern: because `Box<dyn FinanceStore>` is `!Send`, it runs inside `tokio::task::LocalSet` and communicates with the `Send` axum router via a bounded `mpsc::channel`.

### Security constraints (mandatory)

| Constraint | Implementation |
|---|---|
| Localhost-only by default | `--host` defaults to `127.0.0.1`; operator must explicitly pass `0.0.0.0` to expose |
| CSWSH (Cross-Site WebSocket Hijacking) prevention | `ws_handler` checks the `Origin` header; rejects any origin that is not `http://localhost:*`, `http://127.0.0.1:*`, or `null` |
| No unauthenticated remote write | LAN exposure requires explicit `--host` opt-in; Origin check stops browser-based CSRF even on localhost |
| XSS in dashboard HTML | All server-supplied strings rendered via `escHtml()`; template IDs passed through `data-*` attributes + `addEventListener`, never interpolated into `onclick=` |

### Channel sizing

The store actor channel is bounded at 64 messages (`STORE_CHANNEL_CAP`). This is sufficient for local interactive use and provides back-pressure that prevents unbounded memory growth if the dashboard sends requests faster than the store can respond.

### Embedded HTML

The dashboard is a single self-contained HTML file (`serve_dashboard.html`) included at compile time. This avoids a `ServeDir` dependency and keeps the binary self-sufficient. Chart.js is loaded from jsDelivr CDN; this is a known trade-off (requires network on first load) documented here so future contributors can choose to self-host.

---

## Alternatives considered

**Separate `fin-web` binary** — rejected: increases distribution complexity. `fin serve` is an opt-in subcommand; users who do not run it pay zero overhead.

**Server-Sent Events (SSE) instead of WebSocket** — rejected: the dashboard needs bidirectional messaging (request → response with correlation IDs). SSE would require a separate POST channel.

**`tower-http::ServeDir` for static assets** — rejected for now: a single HTML file embedded with `include_str!` is simpler. Switch if the asset count grows.

**Require authentication token** — considered and deferred: the server binds to `127.0.0.1` by default, making it reachable only from the same machine. The Origin check prevents CSRF. A token would add setup friction for the primary use case (local review session). Re-evaluate if multi-user or remote access is ever needed.

---

## Consequences

- `axum 0.8`, `tokio-tungstenite 0.29`, and `tower-http 0.6` are now dependencies of `finance-cli`.
- Three new `FinanceStore` methods are added to the trait and both backends: `list_forecasts`, `get_forecast`, `get_categories`.
- `build_chart_data` is extracted from `report_cashflow_chart` into a public function, enabling reuse.
- `materialise_template_forecasts` is promoted to `pub(crate)` for use by the serve handler.
- All write operations via WebSocket emit `AuditEvent` records.
