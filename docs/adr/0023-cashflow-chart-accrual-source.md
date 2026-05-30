---
type: ADR
id: "0023"
title: "Cashflow chart sources from `v_cashflow` (accrual) with a net-derived saldo line"
status: accepted
date: 2026-05-30
---

## Context

The cashflow chart (the CLI `report cashflow-chart` and the web `/api/chart`,
both built by `build_chart_data`) read realized bars from
`FinanceStore::cashflow_month`. That method is **cash-basis, restricted to
`account_type = 'checking'`**: it drops credit-card swipes and counts the card
*bill payment* on the checking account as the cash event (see ADR-0010/0012 for
the surrounding card-cycle and exact-cents work). The saldo line came from
`checking_balance_at`, anchored on `account_snapshots`.

This is correct for "evolution of the checking-account cash balance", but it
diverges from everything else the product shows:

- The per-category reports, `v_monthly_spend`, and the web month-detail view all
  use `v_transactions_reportable` directly — **accrual**, all accounts, card
  swipes included, with only `internal_categories`
  (`credit-card-payment`, `transfer-internal`) excluded.
- A card-only store (no checking account, no snapshots — a common real state)
  renders an **all-zero chart** while the transaction list right below it shows
  real spend. The bars contradicted the list.

The `v_cashflow` view already encodes exactly the accrual basis we want
(`income`/`expenses`/`net` per month over `v_transactions_reportable`, internal
categories excluded) but was unused by production code — only asserted in tests.

## Decision

**The cashflow chart sources realized bars from `v_cashflow` (accrual, all
reportable accounts) and derives the saldo line from the accumulated monthly
`net`, not from checking snapshots.** Both the CLI chart and the web chart share
this basis.

- A new trait method `FinanceStore::cashflow_reportable()` reads `v_cashflow`
  (implemented in both the SQLite and BigQuery backends) and returns per-month
  `CashflowRow`s ordered oldest-first, with `opening_balance` / `closing_balance`
  left `None`.
- `build_chart_data` keys those rows by `YYYY-MM` for the bars, and derives the
  saldo line by accumulating `net`: the line is anchored on the sum of `net` for
  all months *before* the window (so it reflects prior history instead of
  starting at zero), advances on realized `net` for the solid segment, and
  additionally absorbs the forecast remainder for the dashed projection.
- Forecast remaining (the hatched bar tops and the dashed projection) is
  unchanged — still `upcoming_forecasts` over the rest-of-month / future window.

The cash-basis path is **retained, not removed**: `cashflow`, `cashflow_month`,
and `checking_balance_at` still back `report cashflow` (the single-month
cash-basis summary) and forecast commands, which intentionally answer "what
moved through the checking account".

## Options considered

- **Chart on `v_cashflow` accrual + net-derived saldo** (chosen): the chart
  agrees with the per-category reports and the transaction list, and works with
  no checking account or snapshots. The saldo line becomes "accumulated result /
  household cash position derived from flows", not "checking-account balance".
- **Keep checking-only cash-basis**: financially precise for checking evolution,
  but produces an empty, misleading chart for card-centric stores and disagrees
  with the list it sits above.
- **Hybrid: accrual bars, snapshot-anchored saldo**: mixes an accrual bar series
  with a cash-basis line in one chart — internally inconsistent (a card swipe
  would move a bar in its posting month but not the checking-balance line until
  the bill is paid), and still empty without snapshots.

## Consequences

- The chart now counts card swipes in their posting month; it matches
  `v_cashflow` and the month-detail totals exactly.
- The saldo line no longer represents a bank balance. With no pre-window history
  it starts at zero and tracks cumulative net; it is a flow-derived position, not
  a snapshot-anchored balance.
- `report cashflow` and forecast balance anchoring keep their cash-basis
  semantics; only the chart changed. CLI and web charts now agree.
- No schema or migration change: `v_cashflow` already exists in both backends.
  The change is a new read method plus the `build_chart_data` rewrite.
- Future work: if a snapshot-anchored "real bank balance" line is wanted later,
  it should be an explicit, separately-labelled series rather than reusing the
  saldo line (it would otherwise reintroduce the accrual/cash-basis mix).
