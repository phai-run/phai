# Web usability audit — 2026-06-24 (audit item 9)

Scope: re-triage the 2026-06-02 / 2026-06-03 web-review backlog against the
current `crates/phai-cli/web/src/` code. The redesign that addressed most of
that backlog shipped in **v5.5.0** (PR #129). This audit confirms which items
are genuinely closed in the current tree and flags what remains.

Method: read the relevant sources, ran `pnpm typecheck` (clean) and `pnpm test`
(all passing). Status legend: **FIXED** / **STILL OPEN** / **CANNOT VERIFY**
(needs live runtime / backend data).

Backlog item ids (A1–E2, B1–B3, C1–C5, D1–D4) reference the original
`web-june2026-backlog` notes.

---

## Theme 1 — Data correctness in the UI

| Item | Status | Evidence |
| --- | --- | --- |
| A1 "June inflated" (current-month bar double-counting envelopes) | **FIXED (backend)** | Root cause was in Rust `cashflow_chart.rs` envelope netting, not the web layer. The web chart consumes `ChartMonthView.forecastOutflowsRemaining` as-is. `views/chart/model.ts:64` `buildModel` simply maps `inflows`/`outflows`/`forecast*Remaining`; no double-count logic lives in JS. |
| A2 "saldo line" (closing-balance anchored to snapshots) | **FIXED** | Backend fix (`checking_balance_at` anchor). Web rendering is correct: `views/chart/model.ts:70-75` picks `projectedClosingBalance` for future months, `closingBalance` otherwise; `views/PlanningChart.tsx:781-806` renders a solid purple balance line for realized months, a dashed line for the forecast tail, and dots per month. |
| Card navigation (click card → view its transactions) | **FIXED** | `views/CardsPanel.tsx:91-94` opens `CardDetailPanel`; `views/cards/CardDetailPanel.tsx:194-211` "view card transactions →" calls `onViewTransactions(accountId)`, wired in `Dashboard.tsx:431-433` to set `accountFilter` and switch to the sheet view. |
| Month-sum / billing-cycle correctness | **FIXED / CANNOT FULLY VERIFY** | The web `computeMonthSums` / `transactionsForMonth` (`lib/derivations.ts:475-498`) are pure and unit-tested (`lib/__tests__/consistency.test.ts` sections b/c). Card billing-cycle explosion is computed server-side (`/api/cards`); the web only renders `cycleMonth` (`CardsPanel.tsx:177`). Cycle-vs-posted correctness can only be confirmed against the live BigQuery runtime. |

**Conclusion:** the data-correctness backlog is effectively closed in the web
layer. The remaining risk lives in the Rust bridge / BigQuery views, which this
web-only audit cannot exercise.

---

## Theme 2 — Broken filters

| Item | Status | Evidence |
| --- | --- | --- |
| E1 "Parcelas" (installments) filter shows nothing | **FIXED** | `isInstallment` is seeded and the predicate works: `lib/derivations.ts:245`. Covered by `consistency.test.ts` "filtered sum <= total" + the dedicated filter tests. |
| E2 "Não revisadas" (unreviewed) zeroes the list | **FIXED** | `reviewed` is seeded; predicate `lib/derivations.ts:252`. Covered by `consistency.test.ts:455-473`. |
| Filter predicates combine correctly (intersection not union) | **FIXED** | `lib/derivations.ts:244-278` is a single short-circuit `Array.filter`; `consistency.test.ts:409-419` asserts intersection semantics. |
| Filter-function drift across views | **STILL OPEN (maintainability) — partially fixed in this PR** | See finding below. |

### Finding 2a — `uncategorizedOnly` was missing from the shared derivation (fixed here)

The redesign added an "uncategorized" filter chip
(`views/month/MonthFilters.tsx:224-230`, schema field
`livestore/schema.ts:198`), but the canonical `TxFilters` /
`filterTransactions` / `hasActiveFilters` in `lib/derivations.ts` did **not**
include it. Each view bolted the predicate on separately, so the chip had **zero
shared-test coverage**. AGENTS-documented invariant: "every view must derive its
state through these functions so UI behaviour and test assertions stay in sync."

**Fixed in this PR:** added `uncategorizedOnly` to `TxFilters`,
`filterTransactions`, and `hasActiveFilters` (`lib/derivations.ts`), rewired
`PlanilhaView.tsx` to pass it through the shared function (dropping its
post-filter bolt-on), and added two regression tests (plain + overlay-aware) in
`consistency.test.ts`.

### Finding 2b — `MonthDetail.tsx` reimplements the entire filter inline (STILL OPEN)

`views/MonthDetail.tsx:128-173` does **not** call `filterTransactions` at all —
it hand-rolls a parallel copy of every predicate (installments, subscriptions,
tier, unreviewed, uncategorized, account, owner, category, text), and a parallel
`hasFilters` at `:215-224`. It is currently in sync with the shared function,
but it is untested drift: a future change to `filterTransactions` (e.g. a new
predicate) will silently not apply in the categories view.

**Recommendation (follow-up, not done here to keep this PR low-risk):** refactor
`MonthDetail` to consume `filterTransactions` + `hasActiveFilters`, mirroring the
now-aligned `PlanilhaView`. This removes the third copy of the filter logic and
brings the categories view under the consistency suite.

---

## Theme 3 — Card bill "open" vs "closed" disambiguation (REPORTING_UX)

**Status: FIXED.**

- `views/CardsPanel.tsx:116-117,159-168` renders an explicit `OPEN` / `CLOSED` /
  `SETTLED` badge per tile, with disambiguating tooltips
  ("Open bill — still leaving your cash", "Bill closed for the selected month",
  "No bill in the selected month"), state-coloured accents, and a separate
  "em aberto" open-balance line (`:183-185`).
- `views/cards/CardDetailPanel.tsx:51-56,98-104` restates the same OPEN/CLOSED/
  SETTLED state and shows `bill` vs `open` as distinct figures, plus
  `cycle MM/YY` and `due DD/MM`. This matches the REPORTING_UX §"open vs closed"
  rule (closed total vs open amount surfaced separately).

Minor nit (not blocking): the open-balance line mixes English chrome with the
Portuguese literal "em aberto" (`CardsPanel.tsx:184`), while the rest of the
card UI is English. Cosmetic only.

Minor nit: `CardsPanel.tsx:44` returns `null` on `/api/cards` failure (silent
empty panel, by design — "non-critical panel"). A user whose card data fails to
load sees nothing and no explanation. Consider a small inline error note.

---

## Theme 4 — General UX (loading / error / empty / a11y)

**Status: FIXED.**

- **Loading states:** `Dashboard.tsx` renders `HeroSkeleton`, `ChartSkeleton`,
  `ListSkeleton` while seeding (`:249-256,312-314,380-383`);
  `CardsPanel.tsx:45-55` shows `CardGridSkeleton`. D4 ("loading flash / wrong
  chart before seed") is addressed by gating on `loading` before render.
- **Error states:** `components/ErrorBoundary.tsx` is wired in `App.tsx`;
  `Dashboard.tsx:248` shows `ErrorNote` on seed error.
- **Empty states:** `Dashboard.tsx:384-411` renders "No cash data" with a
  ↻ retry button; `CardsPanel.tsx:60` hides the section when there are no real
  cards.
- **Accessibility / keyboard nav:** the chart is a focusable
  `role="application"` with `←→/Home/End` month navigation
  (`PlanningChart.tsx:80-115`), regression-tested in
  `views/chart/__tests__/ForecastKeyboard.test.tsx`. Card tiles and installment
  rows are `role="button"` with `tabIndex={0}` and Enter/Space handlers
  (`CardsPanel.tsx:137-144`, `CardDetailPanel.tsx:237-252`). The view-mode
  switcher uses `role="tablist"`/`role="tab"`/`aria-selected`
  (`Dashboard.tsx:342-377`).
- **Scroll collapse (D1) / compact strip (D2):** reimplemented as a
  `position:fixed` fade-in driven by scroll hysteresis
  (`Dashboard.tsx:215-233,261-289`) — no layout-shift oscillation.

---

## Prioritized recommendations

1. **(Med) Collapse the third filter copy.** Refactor `MonthDetail.tsx` to use
   `filterTransactions` + `hasActiveFilters` (Finding 2b). Removes drift risk and
   pulls the categories view under the consistency suite. ~1 file, mechanical.
2. **(Low) Card-panel error note.** Replace the silent `return null` on
   `/api/cards` failure with a small inline note so failures are visible
   (`CardsPanel.tsx:44`).
3. **(Low) i18n consistency.** Replace "em aberto" with "open" on the card tile
   (`CardsPanel.tsx:184`) to match the otherwise-English card chrome.
4. **(Info) Billing-cycle correctness** can only be re-verified against the live
   BigQuery runtime (`phai-test` at :4317). Out of scope for a web-only audit.

## Changes made in this PR

- `lib/derivations.ts`: added `uncategorizedOnly` to `TxFilters`,
  `filterTransactions`, `hasActiveFilters`.
- `views/planilha/PlanilhaView.tsx`: route the uncategorized filter through the
  shared function; dropped the post-filter bolt-on and the redundant
  `hasSheetFilters` clause.
- `lib/__tests__/consistency.test.ts`: +2 regression tests for `uncategorizedOnly`
  (plain + overlay-aware).

Gates: `pnpm typecheck` clean; `pnpm test` 259 passing (was 257). No `.rs` files
touched, so the Rust suite was not required.
