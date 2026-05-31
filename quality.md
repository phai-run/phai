# Quality Audit — phai (`serve` + general)

> Audit date: 2026-05-30
> Scope: full codebase review with focus on the `phai serve` web interface.
>
> Legend:
> - 🔴 Critical — must fix (data integrity, security, crash risk)
> - 🟠 High — should fix (security hardening, reliability)
> - 🟡 Medium — could fix (UX, completeness, observability)
> - 🟢 Low — nice-to-have (cosmetic, future-proofing)

---

## 🔴 Critical

### 1. Silent audit-event data loss via `unwrap_or_default()`

**File:** `crates/phai-cli/src/serve.rs:136`

```rust
let diff = serde_json::to_value(&record).unwrap_or_default();
```

If `serde_json::to_value(&record)` fails (e.g., because a `Decimal` serialization triggers a panic in `serde_json`, or a future field type causes an error), the `AuditEvent` is still inserted — but its `diff_json` is silently set to `{}` (empty object). The write succeeds, but the audit trail loses the entire payload.

**Why this is critical:** The architecture guarantees "every write is an event" and the audit log is the foundation for corrections (ADR-0004). A silent empty diff makes the audit event useless — you can't reconstruct what was written, defeating the purpose of the audit system.

**Fix:** Propagate the error instead of swallowing it. Convert the `serde_json::Error` into an `anyhow::Result::Err` so the upsert+audit transaction fails atomically rather than writing a hollow audit.

**Status:** ✅ Fixed — see PR #116

---

### 2. No Subresource Integrity (SRI) on external CDN scripts

**File:** `crates/phai-cli/src/serve_dashboard.html:7-8`

```html
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.0/dist/chart.umd.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/chartjs-plugin-datalabels@2.2.0/dist/chartjs-plugin-datalabels.min.js"></script>
```

Both `<script>` tags load from jsdelivr without `integrity` and `crossorigin` attributes. If the CDN is compromised or a version is tampered with, the dashboard silently runs arbitrary JS. The dashboard connects to a local WebSocket API that reads/writes financial data — the exploit surface is real.

**Fix:** Add SRI hashes (`integrity="sha384-..." crossorigin="anonymous"`).

**Status:** TODO

---

### 3. Store actor crash takes down the entire server

**File:** `crates/phai-cli/src/serve.rs:646-654`

```rust
local.spawn_local(async move {
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    store_actor_loop(store, store_rx).await;
    Ok::<_, anyhow::Error>(())
});
```

The store actor runs inside `LocalSet`. If the actor task panics or returns `Err`, the entire `LocalSet` unwinds, the axum server stops, and all connected clients drop. There is no restart logic, no supervisor, no fallback mode.

**Impact:** A single unhandled DB error (e.g., corrupt SQLite, disk full) kills the dashboard for everyone immediately.

**Fix:** Wrap the actor in a restart loop or use `tokio::task::JoinSet` with automatic respawn. At minimum, emit a visible error and keep the HTTP server alive so clients can reconnect.

**Status:** TODO

---

## 🟠 High

### 4. No security headers on HTTP responses

**File:** `crates/phai-cli/src/serve.rs` (the axum router)

The server sends no security headers:
- No `Content-Security-Policy` → XSS risk from injected content
- No `X-Content-Type-Options: nosniff` → MIME sniffing risk
- No `X-Frame-Options: DENY` → clickjacking (though low-risk on localhost)
- No `Referrer-Policy` → potential data leakage in referrers
- No `Permissions-Policy` → browser feature abuse surface

**Fix:** Add a Tower layer (`tower-http::set_header`) with baseline security headers, or use `tower-http::compression` + custom middleware.

**Status:** TODO

---

### 5. `null` origin allowed in WebSocket origin check

**File:** `crates/phai-cli/src/serve.rs:734`

```rust
origin == "null"
```

The `null` origin is explicitly allowed. This can be triggered by:
- Sandboxed iframes (`<iframe sandbox="allow-scripts">`)
- `data:` URIs loaded directly
- `file://` pages in some browsers

If a user visits a malicious page that opens a sandboxed iframe and connects to `ws://127.0.0.1:8080/ws`, the origin check passes and the attacker can read/write financial data through the WebSocket API.

**Fix:** Remove the `null` origin exception. Only allow explicit localhost origins.

**Status:** TODO

---

### 6. No graceful shutdown

**File:** `crates/phai-cli/src/serve.rs:667-672`

```rust
local
    .run_until(async move {
        axum::serve(listener, app)
            .await
            .context("servidor web parou")
    })
    .await?;
```

When the user hits Ctrl+C, the server exits immediately. WebSocket connections are dropped without close frames, in-flight store operations may be interrupted, and there's no drain phase.

**Fix:** Listen for SIGINT/SIGTERM and call `axum::serve(...).with_graceful_shutdown(shutdown_signal())`.

**Status:** TODO

---

## 🟡 Medium

### 7. No request/error logging

The serve command prints only a startup message. There is no:
- Access log (which endpoint was hit, latency, status)
- Error log (WebSocket errors, store actor errors)
- Connection log (client connect/disconnect)

This makes debugging production issues impossible. The store actor errors are silently dropped via `let _ = resp.send(...)`.

**Fix:** Add `tracing` instrumentation with at minimum `INFO`-level request logs and `ERROR`-level store failures.

**Status:** TODO

---

### 8. No pagination on forecast/template lists

**File:** `crates/phai-cli/src/serve_dashboard.html`

The dashboard loads all forecasts and templates at once with no pagination. With 1000+ forecast records, this would be slow and memory-heavy.

**Fix:** Add `limit`/`offset` parameters to the `list_forecasts` and `list_forecast_templates` store methods, expose them through the WebSocket API, and add pagination controls in the UI.

**Status:** TODO

---

### 9. Missing edit/delete for forecasts in the UI

The web interface has "Add" but no way to:
- Edit an existing forecast (change amount, date, category)
- Delete or dismiss a forecast
- Mark a forecast as "realized"

Users must drop to the CLI for these operations.

**Fix:** Add edit/delete/dismiss buttons to the forecasts table, with corresponding WebSocket messages.

**Status:** TODO

---

### 10. Missing CLI parity: no budget, card, or pulse views

The CLI has 17 report subcommands. The web dashboard only has:
- Cashflow chart (with click-to-drill transactions)
- Forecast templates (accept/dismiss)
- Forecast list
- Manual forecast creation

Missing: budget status, card summary/bills, daily pulse, installments, uncategorized queue, data health, and more.

This is not a bug but a significant completeness gap between the CLI and web experience.

**Status:** TODO (feature backlog)

---

### 11. Channel capacity bottleneck

**File:** `crates/phai-cli/src/serve.rs:36`

```rust
const STORE_CHANNEL_CAP: usize = 64;
```

The mpsc channel has a fixed capacity of 64. If 64 concurrent WebSocket requests are in-flight and a 65th arrives, the sender blocks until a slot frees. WebSocket clients will experience timeout-like behavior with no error message.

**Fix:** Either increase to 256+, use a bounded channel with `try_send` and error feedback, or switch to a `tokio::sync::Semaphore`-based concurrency limiter.

**Status:** TODO

---

### 12. No input-length validation on `upsert_forecast`

**File:** `crates/phai-cli/src/serve.rs:426-471`

The `upsert_forecast` handler accepts `description`, `amount`, `category_id`, `account_id` with no max-length validation. A malformed request could insert extremely long strings into the database, causing storage issues or UI rendering problems.

**Fix:** Validate `description.len() <= 500`, `category_id.len() <= 100`, etc., and return clear error messages.

**Status:** TODO

---

## 🟢 Low

### 13. No dark mode

The CSS uses hardcoded light-theme colors in `:root`. No dark mode support.

**Fix:** Add `@media (prefers-color-scheme: dark)` overrides.

**Status:** TODO

---

### 14. No offline/CDN-offline fallback

If jsdelivr is unavailable (airplane mode, corporate firewall), the dashboard renders a blank page because Chart.js fails to load. The rest of the UI (templates, forecasts, add form) also breaks because `Chart.register(ChartDataLabels)` fails.

**Fix:** Bundle Chart.js with the binary via `include_str!` + base64 data URI, or detect CDN load failure and show a graceful degraded mode with just the tabular data.

**Status:** TODO

---

### 15. HTML template baked into binary

**File:** `crates/phai-cli/src/serve.rs:639`

```rust
Html(include_str!("serve_dashboard.html"))
```

The 584-line HTML template is compiled into the binary. Users cannot customize the UI without rebuilding from source. For advanced users who want to theme or extend the dashboard, there's no override mechanism.

**Fix:** Look for `$PHAI_CONFIG_DIR/dashboard.html` at startup, fall back to the embedded one.

**Status:** TODO

---

### 16. No WebSocket ping/pong heartbeat

The WebSocket connection has no application-level heartbeat. If a TCP connection silently breaks (mobile sleep, NAT timeout), neither the client nor server will notice until the next message send.

**Fix:** Enable `axum`'s automatic ping/pong or implement application-level keepalive.

**Status:** TODO

---

## Non-`serve` findings

### 17. `unwrap_or_default()` used in error-handling-sensitive paths (general)

**Files:** `serve.rs:317,327,354,543`

```rust
serde_json::to_string(&resp).unwrap_or_default().into()
```

In WebSocket message serialization, these should never fail (the types are all `Serialize`), but if they do, the client receives an empty string with no error. This is low-risk today but a latent correctness issue.

**Fix:** Same approach as #1 — either explicitly handle (log + error response) or use `expect("serialization of WsResponse is infallible")` to document the invariant.

**Status:** TODO (follow-up to #1)

---

## Summary

| Priority | Count | Area |
|----------|-------|------|
| 🔴 Critical | 3 | Data integrity, Security, Reliability |
| 🟠 High | 3 | Security headers, Origin bypass, Graceful shutdown |
| 🟡 Medium | 6 | Observability, UX completeness, Input validation |
| 🟢 Low | 4 | Cosmetic, Robustness, Extensibility |
| **Total** | **16** | |

---

## Action Plan

1. **[NOW]** Fix #1 (silent audit data loss) — one-line change with regression test
2. **[Next]** Fix #2 (SRI on CDN scripts) — add integrity hashes
3. **[Next]** Fix #5 (remove `null` origin) — one-line change
4. **[Next]** Fix #3 (store actor crash resilience) — restart loop
5. **[Then]** #6 (graceful shutdown), #4 (security headers), #7 (logging)
6. **[Backlog]** #8-16 — feature work
