---
type: ADR
id: "0026"
title: "One deduped view chain is the single source of truth for all reporting"
status: accepted
date: 2026-06-01
---

## Context

Cash-flow numbers kept regressing because the same logic was implemented in more
than one place. Concretely, the **ofx-vs-pluggy deduplication** (drop an OFX-
imported row when Pluggy already has the same transaction) lived only in the Rust
`FinanceStore::cashflow_reportable` method and in the `serve` transaction-list
handler — *outside* the SQL view chain. So:

- The cashflow **chart** (Rust path) was deduped and showed May expenses
  R$ 37.986,62.
- `report monthly-spend` and anything reading `v_monthly_spend` / `v_cashflow`
  (the view path) were **not** ofx-deduped and showed R$ 41.892,72 for the same
  month.

Two sources of truth for "what counts" guarantee drift. The question "why do we
need these views at all?" has a precise answer: the views are how SQLite and
BigQuery share one definition of dedup + classification + `cash_month`
bucketing. The bug was never the views — it was a rule living *next to* them
instead of *inside* them.

## Decision

**Every report and the web UI read the same view chain; no dedup or
classification logic lives in Rust query methods or the web layer.** The chain
is, bottom to top:

```
transactions                         raw rows (Pluggy / OFX / legacy / manual)
  └─ v_transactions_effective        + splits expansion + display labels
       └─ v_transactions_reportable  ← dedup lives HERE: drops legacy-manual and
            │                            ofx rows shadowed by a Pluggy row
            └─ v_transactions_cashbasis   + canonical `cash_month` (ADR-0025)
                 ├─ v_cashflow            monthly income/expenses/net
                 └─ v_monthly_spend       per-category monthly spend
```

- The ofx-vs-pluggy dedup moved into `v_transactions_reportable` (migration 038),
  joining the existing legacy-manual dedup. Both are expressed as **LEFT JOIN
  anti-joins**, not correlated `EXISTS`, because BigQuery cannot de-correlate
  `EXISTS` through the nested cashbasis/cashflow views.
- `cashflow_reportable` is now `SELECT … FROM v_cashflow` in both backends — a
  thin read of the canonical view, with no window/dedup of its own.
- Internal categories (`credit-card-payment`, `transfer-internal`,
  `same-person-transfer`) remain the single exclusion list, applied in the views.

## Options considered

- **One deduped view chain, thin Rust readers** (chosen): a single definition per
  rule, shared by both backends and every surface. Adding a report means reading a
  view, not re-deriving filters.
- **Query raw `transactions` with filters in each report/UI**: rejected — it
  scatters dedup/classification into every call site, multiplying the exact drift
  this ADR removes.
- **Keep dedup in Rust, document it**: rejected — documentation does not stop a
  new SQL consumer from skipping the Rust path.

## Consequences

- `v_cashflow`, `v_monthly_spend`, the chart, the month-detail list and the
  per-category report now agree by construction.
- New reporting must read the view chain. A new dedup/classification rule is added
  to `v_transactions_reportable` (or the category exclusion list), never to a Rust
  query or the web layer.
- The dedup is an anti-join: any future edit must preserve the "kept rows do not
  fan out" property (a kept row has no matching Pluggy row, so its LEFT JOIN is a
  single NULL-extended row).
- The read-only `report duplicates` audit still reads the raw `transactions` table
  (it reports physical duplicates to clean up); the views make those duplicates
  harmless to reports in the meantime.
