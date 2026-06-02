---
type: ADR
id: "0025"
title: "Cash-flow basis with credit-card bill explosion is the canonical reporting model"
status: accepted
date: 2026-06-01
---

## Context

phai manages a **family's** finances, where the question that matters is *regime
de caixa* (cash flow): "what entered and left the cash this month?" — not accrual
(*competência contábil*). The defining requirement: a credit-card bill **paid in
May** should surface as **May's individual purchases** (exploded line by line, so
the family can see and track them), not as a single lump "card payment" and not on
the original purchase dates.

The codebase had drifted into **three competing notions of "month"**, none of them
this one, and each report surface picked a different one — the direct cause of the
recurring regressions:

1. **Accrual** — [ADR-0024](0024-cashflow-chart-accrual-source.md) made the cashflow
   chart (`v_cashflow`, CLI `report cashflow-chart`, web `/api/chart`) bucket card
   swipes in their **posting month**. The per-category reports, `v_monthly_spend`
   and the web month-detail did the same.
2. **Cash-basis, checking-only** — `cashflow_month` / `report cashflow` and the
   pulse counted the card *bill payment* on the checking account as one lump in the
   month it was paid.
3. **Billing cycle** — [ADR-0010](0010-card-billing-cycle.md)'s `v_card_summary`
   groups by closing cycle.

Every fix nudged one surface toward one of these and broke another, because there
was **no canonical definition of which month a transaction belongs to**. ADR-0024
directly contradicts the family-cash-flow requirement.

ADR-0024 chose accrual partly because the cash-basis chart rendered **empty** for a
card-only store (no checking account, no bill-payment transaction). A *projected*
cash-flow basis removes that objection: a card purchase always has a home month (its
bill's due month) even with no recorded payment.

## Decision

**A single canonical `cash_month` is derived for every reportable transaction, and
every surface (cashflow chart, CLI reports, web month-detail, pulse) buckets by it.**

- **Non-card accounts** (checking / debit / pix / cash): `cash_month` is the
  transaction's own month — cash moves immediately.
- **Credit cards**: `cash_month` is the month the bill that *contains* the purchase
  is **due/paid**, projected from account metadata — `billing_closing_day` decides
  which cycle the purchase closes in (a charge dated **on or after** the closing day
  belongs to the cycle that closes next month — the closing day is the first day of
  the new cycle, as Nubank's OFX `DTSTART` is inclusive; this corrects the off-by-one
  in `v_card_summary`'s `<= closing_day` boundary), and `billing_due_day` adds a
  one-month roll when the due day precedes the closing day. v1 assumes each bill is
  **paid in full on its due date**; revolving/partial payment (*rotativo*) is explicit
  future work.
- The credit-card bill **payment** transaction stays in `internal_categories`
  (already excluded by every report view), so the exploded purchases **replace** it
  with no double counting.

This lives in one place per backend — the SQL view `v_transactions_cashbasis`
(migration 037), which adds `cash_month` to `v_transactions_reportable`.
`v_cashflow` and `v_monthly_spend` are redefined to group by `cash_month`, and the
`cashflow_reportable` queries in both backends read the cash-basis view. **This ADR
supersedes [ADR-0024](0024-cashflow-chart-accrual-source.md).**

## Options considered

- **Projected cash-flow basis with bill explosion** (chosen): deterministic from
  card metadata, works with no checking account or recorded payment, and matches the
  family's mental model. A single SQL view is the single source of truth, so the
  chart, CLI and web cannot drift. Trade-off: assumes payment on the due date, so it
  is a projection, not the literal cleared-cash date.
- **Actual cash basis (match the real payment transaction)**: reflects the literal
  cash movement, but breaks for card-only stores and needs a reliable link between a
  payment transaction and the swipes it settles (which does not exist in the model).
- **Keep accrual (ADR-0024)**: agrees with the transaction list by posting date, but
  contradicts the family-cash-flow requirement and was itself a source of confusion
  between the chart and the lump-payment cash report.

## Consequences

- A card purchase no longer appears in its posting month; it appears in the month its
  bill is paid. Late-May purchases on a card closing in early June surface in **June**,
  while May shows the already-paid bill's purchases plus checking movements.
- The chart's saldo line (accumulated `net`) becomes a meaningful cash position again,
  and a card-only store renders a populated chart (ADR-0024's original objection is
  resolved).
- `cash_month` uses the `>= closing_day → next cycle` boundary, matching
  `compute_bill_id` and the real Nubank statement boundary. `v_card_summary` still
  uses `<= closing_day` (off by one on the closing day itself); converging it is
  tracked as follow-up. Revolving/partial bills are out of scope for v1.
- `report cashflow` and the pulse are migrated onto the same canonical basis so all
  surfaces agree (separate change).
- No data migration: `cash_month` is a derived view column. Changing a card's
  closing/due day re-buckets automatically on next read.
