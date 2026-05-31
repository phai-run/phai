# finish.md — phai web rebuild: state & remaining work

> Handoff for the next agent/session. Everything the user asked, what's **done**,
> and what's **left**. Read this top-to-bottom before touching the branch.

## Branches & PRs

| Branch | PR | Scope | State |
|--------|----|-------|-------|
| `feat/remove-google-sheets-sync` | **#104** | Remove Google Sheets category-override sync | MERGEABLE (rebased on main) — **not merged** |
| `feat/web-livestore-scaffold` | **#105** | Rebuild `phai serve` on LiveStore + remove TUIs + the planning-workspace features | green locally — **needs a final push + merge** |

`main` advanced to **4.1.0** (an on-disk identity migration, now ADR-0022) while this work was on 4.0.0. Both branches are rebased/merged onto latest main and mergeable. **The local branch has commits not yet pushed** — run the "Wrap-up" steps below.

Current local HEAD of `feat/web-livestore-scaffold`: see `git log --oneline -12`. Recent commits include autonomous **pi-lens** refactors (`c06cd8b`, `2b4e703`, `ecd0851`) — see "Caveats".

---

## What the user asked (original → improvements)

**Round 1 (done, in #104/#105):**
- Reimplement the `phai serve` web app on the brandkit (`DESIGN.md`), folding in all `phai review` TUI functionality.
- Delete the TUIs (review + cashflow) from the code.
- Delete Google Sheets sync.

**Round 2 — "important improvements" (this is the active work):**
1. Don't ask; pick the recommended approach; parallelize with multiple agents. ✅ (ran bridge ‖ frontend agents)
2. Full **E2E** at the end; validate **all** web features; always verify persistence in **LiveStore and BigQuery**. ⏳ (mostly done — see "Remaining")
3. After all OK: **merge**, **release a version**, **test self-update**. ❌ (not started)
4. May update `DESIGN.md`. ✅
5. Create `family_fin_scratch`. ✅
6. Use full screen width on large monitors. ✅
7. Unified Caixa + Previsão view; clicking a month bar selects the month. ✅
8. Filters (category, unreviewed, subscriptions, installments) on the main UI, each showing the sum of expenses/income. ✅
9. In the bar chart, forecasts complement the expense/income bars in other colors; hover shows a popup of that month's forecasts. ✅
10. Drag-and-drop of forecast expenses (not card installments/subscriptions) to plan future months fast, with real-time totals. ⏳ (code complete + write path proven; **live drag not yet exercised in a browser** — see "Remaining")
11. Whole UI extremely fast / zero-delay (LiveStore). ✅ (client-side reactive sums/filters/selection)

---

## DONE (verified)

**Backend (Rust bridge — `crates/phai-cli/src/serve.rs`, `cashflow_chart.rs`):**
- `GET /api/transactions?months_back&months_ahead&include_reviewed&limit` → window of tx with camelCase rows + `reviewed`/`isInstallment`/`isSubscription` flags.
- `GET /api/forecasts` enriched with `kind` (manual|installment|subscription|fixed|envelope) + `draggable`.
- `GET /api/chart` now includes a canonical **`month` (YYYY-MM)** per bar (not just the display `label`).
- `POST /api/forecast/move {forecastId,dueDate}` — re-dates a manual forecast in place (same id), rejects installments/subscriptions, emits an `AuditEvent`.
- Light theme palette in the CLI/serve path; `phai serve` → `phai.localhost`, **default port 80** (privileged → sudo), **auto-opens the browser**, **logs every `/api` op in debug builds**.

**Frontend (`crates/phai-cli/web/`):**
- Full-width responsive workspace (`max-width: min(1680px,96vw)`, sticky filter rail on wide screens).
- Two views: **Revisão** (tx list + filters + live sums) and **Planejamento** (unified chart + month panel).
- LiveStore-first: reads are reactive queries; writes optimistic → `pendingWrites` (typed flush queue) → bridge. Sums/filters/selection all client-side.
- Chart with stacked **forecast overlay** (hatched future bars) + **hover popover** listing that month's forecasts; **click a bar → selects that month** (keyed by YYYY-MM); the right panel shows that month's transactions + forecasts + totals.
- Hand-rolled drag-and-drop (`lib/dnd.tsx`); manual forecasts draggable, installments/subscriptions locked.
- White theme (`design/tokens.css`), `renderError` boot screen, `storeId` schema-versioned (`phai-s2`).

**Verified live (browser, `phai.localhost` → production BigQuery, read-only):**
- White wide layout; Revisão **1331** tx; live sums recompute instantly on filter toggle (saídas/entradas/líquido).
- Planejamento chart renders; hover popover shows month forecasts; clicking dez/25 selects **2025-12**, panel shows entradas/saídas + **PREVISÕES · 28**; installments locked (◎), manual forecasts show a drag handle (⠿).

**BigQuery write-persistence — PROVEN** against `family_fin_scratch` (via curl on the bridge): `POST /api/forecast/move` re-dated `pediatra_bruno_semestral` 2026-06-15 → 2026-08-20 (same id, no dup); `POST /api/events` set a tx's `category_id`+`merchant_name`. Confirmed by querying the scratch tables.

**Suite:** `cargo fmt`/`clippy -D warnings`/`test --workspace` green; `pnpm typecheck`/`build` green; `sentrux gate` clean (0 cycles, no degradation).

---

## REMAINING (do these, in order)

### 1. Wrap-up the branch (5 min)
```bash
git switch feat/web-livestore-scaffold
( cd crates/phai-cli/web && pnpm build )        # ensure dist is current (gitignored, build.rs embeds it)
cargo build -p phai-cli                          # sanity
git push origin feat/web-livestore-scaffold      # push the unpushed commits (HEAD: 46b6729 + pi-lens commits)
gh pr checks 105                                  # confirm CI green
```

### 2. Finish the live E2E (the one gap)
- **Drag-and-drop, end-to-end in the browser** against `family_fin_scratch` (so writes don't touch prod): start `phai serve` with `FINANCE_OS_CONFIG_DIR=/Users/frb/finance-os-configs/runtime/felipe-scratch` on a fresh port, open it, drag a **manual** forecast (e.g. `pediatra_bruno_semestral`) to another month, and confirm: (a) bars/totals update in the same frame, (b) the SyncChip flushes, (c) the new `due_date` lands in `family_fin_scratch.forecast`.
  - Browser gotcha: after any `pnpm build`, **close the tab to kill the LiveStore SharedWorker before reopening** (a live worker runs stale code and the page hangs at "carregando…"). A fresh tab on a fresh origin always boots.
- Re-confirm Revisão filter sums for **assinaturas** and **parcelas** specifically (only "todas"/"não revisadas" were screenshotted).

### 3. Merge
- Decide order: merge **#104** (sheets) first, then **#105**. Both are `feat!:` (breaking) → Release Please will bump the major.
- Squash or merge per repo norm; ensure ADR index stays correct (0021 LiveStore web app, 0022 identity migration).

### 4. Release a version
- Merging to `main` triggers Release Please → it opens a release PR (CHANGELOG + version bump). Merge that PR to cut the GitHub Release.
- The release workflow (`.github/workflows/release-please.yml`) was updated to **build the web app (pnpm) before `cargo build --release`** so the real SPA is embedded. Verify the release job builds the tarball with the embedded UI.

### 5. Test self-update
- After the release publishes assets: on an **older** installed `phai`, run `phai self update` and confirm it pulls + swaps to the new version (see ADR-0017 / `update.rs`). Verify `phai --version` reflects the new version and `phai serve` shows the new UI.

### 6. Loose ends / cleanup
- **Migration 033 idempotency bug** (flagged via spawn_task, not yet fixed): `schema/bigquery/033_transaction_anatomy.sql` does an unconditional `ALTER COLUMN ... DROP NOT NULL` that errors when re-run on an already-nullable column → blocks clean BigQuery migrate-from-clone. Guard it (INFORMATION_SCHEMA check or scripting EXCEPTION). This is why `family_fin_scratch` was built by copying tables + `schema_versions` (auto-migrate no-op) rather than a clean migrate.
- **Review the pi-lens auto-commits** (`c06cd8b`, `2b4e703`, `ecd0851`) properly — they were genuine improvements (break circular dep via `views/types.ts`, error boundary, event-sourced AddForecast, serve DTO unification) and the suite is green, but they weren't human-authored. The dark-mode part of `2b4e703` was reverted (user requires white).
- Consider whether `phai serve` default port should really be **80** (forces sudo for everyone) or revert to 8080 with `phai.localhost:8080`. Currently 80 per the user's explicit request.
- `family_fin_scratch` exists in `finance-os-frb` for testing — `bq rm -r -f -d finance-os-frb:family_fin_scratch` when done.

---

## How to run locally

**Against production BigQuery (read-only browsing):**
```bash
git switch feat/web-livestore-scaffold
( cd crates/phai-cli/web && pnpm install && pnpm build )
cargo build -p phai-cli                          # debug build = op logging on
echo '<sudo-pass>' | sudo -S env FINANCE_OS_CONFIG_DIR=/Users/frb/finance-os-configs/runtime/felipe-mac \
  ./target/debug/phai serve --port 80            # opens http://phai.localhost
```

**Against the scratch dataset (safe for write tests):**
```bash
FINANCE_OS_CONFIG_DIR=/Users/frb/finance-os-configs/runtime/felipe-scratch ./target/debug/phai serve --port 8090
# config points at finance-os-frb.family_fin_scratch
```

**Config locations** (user's machine): `~/finance-os-configs/runtime/{felipe-mac,felipe-scratch}/config.toml`; service account `~/finance-os-configs/gcp/service-accounts/felipe-mac.json`. Prod = `finance-os-frb.family_fin` (1300+ tx), scratch = `finance-os-frb.family_fin_scratch`.

## Architecture pointers
- Bridge + actor: `crates/phai-cli/src/serve.rs`; static embed: `serve_assets.rs` + `build.rs` (web/dist is generated, **not committed**).
- Chart data: `crates/phai-cli/src/cashflow_chart.rs` (`MonthDatum.month` is the YYYY-MM key).
- Web: `crates/phai-cli/web/src/` — `livestore/schema.ts` (tables/events/materializers, bump `storeId` in `main.tsx` on schema changes), `bridge/{api,sync}.ts`, `views/{Review,Planning,PlanningChart}.tsx`, `views/types.ts` (shared view types), `lib/{dnd,format}.tsx`.
- ADR-0021 (LiveStore web app), ADR-0022 (identity migration). Brand: `DESIGN.md` (light theme canonical + web-app interaction model).

## Known caveats
- BigQuery `/api/chart` is slow (~15s) — the chart query aggregates months. The UI is instant once LiveStore is seeded; only the initial seed waits on BQ.
- After a `pnpm build`, kill the SharedWorker (close tabs) before reloading or the page hangs (stale worker). Not a production issue (fresh users boot clean).
- A breaking LiveStore table-schema change requires bumping `storeId` (`main.tsx`) — the local store is a disposable cache (BigQuery is source of truth).
