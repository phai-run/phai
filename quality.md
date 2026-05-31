# Quality Audit — phai (`serve` + general)

> Audit date: 2026-05-30 · Last updated: 2026-05-31
> Scope: full codebase review with focus on the `phai serve` web interface.
>
> Legend:
> - 🔴 Critical — must fix (data integrity, security, crash risk)
> - 🟠 High — should fix (security hardening, reliability)
> - 🟡 Medium — could fix (UX, completeness, observability)
> - 🟢 Low — nice-to-have (cosmetic, future-proofing)
> - ⚪ Resolved / Obsolete — no longer applies

---

## 🔴 Critical

### 1. Silent audit-event data loss via `unwrap_or_default()`

**File:** `crates/phai-cli/src/serve.rs` (old line 136)

```rust
let diff = serde_json::to_value(&record).unwrap_or_default();
```

If `serde_json::to_value(&record)` fails, the `AuditEvent` is still inserted — but its `diff_json` is silently set to `{}` (empty object). The write succeeds, but the audit trail loses the entire payload.

**Why this is critical:** The architecture guarantees "every write is an event" and the audit log is the foundation for corrections (ADR-0004). A silent empty diff makes the audit event useless — you can't reconstruct what was written.

**Status:** ✅ Fixed — PR #116 (released in v5.1.2). The `serve_dashboard.html` was deleted and the web app rewritten as a React SPA. The `?` operator now propagates the serialization error instead of swallowing it.

---

### 2. ~~No Subresource Integrity (SRI) on external CDN scripts~~ ⚪ Obsolete

**File:** `crates/phai-cli/src/serve_dashboard.html` (deleted)

The old dashboard loaded Chart.js from jsdelivr without `integrity` hashes. The new web app (`crates/phai-cli/web/`) bundles all dependencies via pnpm + Vite — no CDN scripts exist anymore.

**Status:** ⚪ Obsolete — `serve_dashboard.html` deleted in the web-app rewrite. Chart.js is now an npm dependency bundled at build time.

---

### 3. Store actor crash takes down the entire server

**File:** `crates/phai-cli/src/serve.rs:1252-1260`

```rust
let local = LocalSet::new();
local.spawn_local(async move {
    let store = open_store(&actor_config).await?;
    run_migrations(store.as_ref(), &actor_config).await?;
    store_actor_loop(store, actor_config, store_rx).await;
    Ok::<_, anyhow::Error>(())
});
```

The store actor runs inside `LocalSet`. If `open_store` or `run_migrations` returns `Err`, the task exits silently. The HTTP server stays alive but every `/api` request returns "store actor indisponível" permanently — there is no restart logic.

**Impact:** A single unhandled DB error (e.g., corrupt SQLite, disk full) makes the dashboard permanently non-functional until manual restart.

**Fix:** Wrap the actor initialisation + loop in a restart-with-backoff so transient failures (disk full that resolves, DB lock released) self-heal. Use a shared sender (`Arc<RwLock<Sender>>`) so a fresh channel is plumbed on each restart attempt.

**Status:** TODO

---

## 🟠 High

### 4. No security headers on HTTP responses

**File:** `crates/phai-cli/src/serve.rs` (axum router, lines 1267-1292)

The server sends no security headers:
- No `Content-Security-Policy` → XSS risk from injected content
- No `X-Content-Type-Options: nosniff` → MIME sniffing risk
- No `X-Frame-Options: DENY` → clickjacking (though low-risk on localhost)
- No `Referrer-Policy` → potential data leakage in referrers
- No `Permissions-Policy` → browser feature abuse surface

Note: a `guard_origin` middleware (same-origin check) was added in the rewrite, which mitigates CSRF. But the headers above are still missing.

**Fix:** Add a Tower layer with baseline security headers via a simple middleware function.

**Status:** TODO

---

### 5. `null` origin allowed in origin check

**File:** `crates/phai-cli/src/serve.rs:1418`

```rust
origin == "null"
```

The `null` origin is explicitly allowed. This can be triggered by:
- Sandboxed iframes (`<iframe sandbox="allow-scripts">`)
- `data:` URIs loaded directly
- `file://` pages in some browsers

**Context update (2026-05-31):** The WebSocket API was replaced with REST-only endpoints. The attack surface is smaller (no persistent connection), but a malicious page in a sandboxed iframe could still issue `POST /api/events` (review writes) and `POST /api/forecast` if the user visits it while `phai serve` is running.

**Fix:** Remove the `null` origin exception. Only allow explicit localhost origins.

**Status:** TODO

---

### 6. No graceful shutdown

**File:** `crates/phai-cli/src/serve.rs:1318-1324`

```rust
local
    .run_until(async move {
        axum::serve(listener, app)
            .await
            .context("servidor web parou")
    })
    .await?;
```

When the user hits Ctrl+C, the server exits immediately. In-flight store operations may be interrupted, and there's no drain phase for pending oneshot responses.

**Fix:** Listen for SIGINT/SIGTERM and call `axum::serve(...).with_graceful_shutdown(shutdown_signal())`.

**Status:** TODO

---

## 🟡 Medium

### 7. No request/error logging in release builds

The serve command has `log_ops` middleware that logs method, path, status, latency — but **only in debug builds** (gated by `cfg!(debug_assertions)`). In release builds there is zero observability:
- No access log (which endpoint was hit, latency, status)
- No error log (store actor failures are silent)
- No connection log

The store actor errors are silently dropped via `let _ = resp.send(...)`.

**Fix:** Always log errors (`eprintln!` at minimum). Keep the per-request debug log gated but add unconditional ERROR-level output for store failures and panics.

**Status:** TODO

---

### 8. ~~No pagination on forecast/template lists~~ ⚪ Obsolete

**File:** `crates/phai-cli/src/serve_dashboard.html` (deleted)

The old dashboard loaded all forecasts and templates at once. The new web app uses `DEFAULT_TRANSACTIONS_LIMIT = 5000` and `DEFAULT_REVIEW_QUEUE_LIMIT = 200` — pagination is still server-side-only but the React SPA can implement client-side pagination. The API supports `limit` parameters.

**Status:** ⚪ Obsolete — old dashboard deleted. Pagination is a frontend concern in the new SPA.

---

### 9. Missing edit/delete for forecasts in the UI — partially addressed

The old web interface had "Add" but no edit/delete. The new React SPA adds:
- **Drag-and-drop** forecast rescheduling (`POST /api/forecast/move`) — ✅
- **Forecast creation** (`POST /api/forecast`) — ✅
- **Template accept/dismiss** (`POST /api/forecast-template/accept`, `/dismiss`) — ✅

Still missing in the SPA:
- Inline edit of forecast fields (description, amount, category)
- Delete/dismiss an individual forecast (not template)

**Status:** 🟡 Partially addressed — drag-to-reschedule covers the main UX gap. Edit/delete remain as future enhancements.

---

### 10. Missing CLI parity: no budget, card, or pulse views

The CLI has 17 report subcommands. The web dashboard now has:
- Cashflow chart (`PlanningChart.tsx`)
- Month drill-down (`MonthDetail.tsx`)
- Forecast management (templates + creation + drag-to-reschedule)
- Review queue editing

Still missing: budget status, card summary/bills, daily pulse, installments view, uncategorized queue, data health.

**Status:** 🟡 Feature backlog — the SPA architecture makes these straightforward to add as new views.

---

### 11. Channel capacity bottleneck

**File:** `crates/phai-cli/src/serve.rs:38`

```rust
const STORE_CHANNEL_CAP: usize = 64;
```

The mpsc channel has a fixed capacity of 64. Under load, senders block until a slot frees. With the REST API, concurrent requests are serialised through this channel — 64 concurrent requests could exhaust it.

**Fix:** Increase to 256. The memory overhead is negligible (each `StoreRequest` is a few hundred bytes at most).

**Status:** TODO

---

### 12. No input-length validation on `post_forecast`

**File:** `crates/phai-cli/src/serve.rs:1086-1136`

The `post_forecast` handler accepts `description`, `amount`, `category_id`, `account_id` with no max-length validation. A malformed request could insert extremely long strings into the database.

**Fix:** Validate `description.len() <= 500`, `category_id.len() <= 100`, `account_id.len() <= 100`, and return clear 400 error messages.

**Status:** TODO

---

## 🟢 Low

### 13. No dark mode

The new design tokens (`web/src/design/tokens.css`) define light-theme colors on `:root`. There is no `@media (prefers-color-scheme: dark)` override.

**Fix:** Add dark-mode media query overrides in `tokens.css`.

**Status:** TODO (cosmetic — not a correctness issue)

---

### 14. ~~No offline/CDN-offline fallback~~ ⚪ Obsolete

The old dashboard broke if jsdelivr was unreachable. The new SPA bundles all dependencies via Vite — there is no runtime CDN dependency.

**Status:** ⚪ Obsolete — all assets are bundled at build time.

---

### 15. ~~HTML template baked into binary~~ ✅ Fixed

**File:** `crates/phai-cli/src/serve.rs` (old)

The old 584-line HTML template was compiled into the binary via `include_str!("serve_dashboard.html")`. The new code uses `crate::serve_assets::static_handler` which serves the embedded SPA from the `web/` build output, embedded via `include_dir`.

**Status:** ✅ Fixed — the SPA is embedded at build time via `include_dir` and served through a proper static-handler with MIME types, caching headers, and SPA fallback.

---

### 16. ~~No WebSocket ping/pong heartbeat~~ ⚪ Obsolete

The old WebSocket connection had no application-level heartbeat. The WebSocket API was removed entirely in the rewrite — all communication is now stateless REST over HTTP.

**Status:** ⚪ Obsolete — WebSocket removed. REST is inherently stateless.

---

## Non-`serve` findings

### 17. Residual `unwrap_or_default()` in non-critical display paths

**Files:** `serve.rs:376, 647`

```rust
// Line 376 — ForecastWithMeta::to_json()
let mut value = serde_json::to_value(&self.record).unwrap_or_default();

// Line 647 — debug_assert! amount precision check
.unwrap_or_default();
```

These are in display/debug paths, not the audit trail. The types are all `Serialize` so these should never fail in practice. Low risk but still latent correctness issues.

**Fix:** Use `expect("ForecastRecord serialization is infallible")` to document the invariant, or propagate the error.

**Status:** 🟢 Low priority — not audit-critical.

---

## Summary

| Priority | Count | Area |
|----------|-------|------|
| 🔴 Critical | 1 | Reliability (store actor crash) |
| 🟠 High | 3 | Security headers, Origin bypass, Graceful shutdown |
| 🟡 Medium | 3 | Observability, Channel capacity, Input validation |
| 🟢 Low | 3 | Dark mode, Residual unwraps, Feature backlog |
| ⚪ Resolved/Obsolete | 7 | Rewrite addressed or obsoleted |
| **Total actionable** | **10** | |

---

## Action Plan

1. **[NOW]** Fix #3 (store actor resilience) — restart loop + shared sender
2. **[NEXT]** Fix #5 (remove `null` origin) — one-line change
3. **[NEXT]** Fix #4 (security headers) — middleware layer
4. **[NEXT]** Fix #6 (graceful shutdown) — `with_graceful_shutdown`
5. **[NEXT]** Fix #7 (error logging in release) — unconditional `eprintln!`
6. **[NEXT]** Fix #11 (channel capacity) — 64 → 256
7. **[NEXT]** Fix #12 (input validation) — length checks on `post_forecast`
8. **[BACKLOG]** #9, #10, #13, #17 — feature work
