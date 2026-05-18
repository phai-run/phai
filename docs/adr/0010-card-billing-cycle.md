---
type: ADR
id: "0010"
title: "v_card_summary groups by billing cycle, not calendar month"
status: accepted
date: 2026-05-18
---

## Context

Credit-card statements in Brazil close on a per-account *closing day* (the
`billing_closing_day` field already present in `accounts.metadata_json`).
Purchases between two consecutive closing days form one bill, and that bill
is paid on the following due day.

The previous `v_card_summary` view grouped credit-card transactions by
`strftime('%Y-%m', transaction_date)`. The consequence: a purchase made on
March 28th, on a card with closing-day 3, landed in the "March" bucket even
though the user pays for it in April. Card-summary reports drifted out of
sync with the actual billing reality.

The pulse message ([ADR-0009](0009-proactive-pulse-and-closing-plan.md))
needed a clear "open bill per card" signal. Without cycle-aware grouping
that signal was either incomplete or wrong.

## Decision

**`v_card_summary` redefines `month_ref` to mean the closing cycle, not the
calendar month.** A new sibling view `v_card_open_now` returns at most one
row per card — the most recent closed cycle that still has open balance —
which is the actionable "what do you owe right now" surface.

`FinanceStore` gains `cards_open_now()` reading the new view. The pulse
consumes it. `card_summary(month_ref)` continues to exist with the same
signature, but the `month_ref` parameter now refers to the cycle.

For accounts whose `metadata_json` has no `billing_closing_day` (corporate
meal-voucher cards like Flash that bill on a different model), the view
falls back to the calendar month so they still aggregate cleanly.

## Options considered

- **Redefine v_card_summary in place** (chosen): one view per concept, the
  semantics of `month_ref` shifts from "calendar month" to "cycle month",
  which is what the user actually thinks about. Callers that passed
  `current_month` keep working but answer a more correct question.
- **Add a separate v_card_cycle view, keep v_card_summary calendar-based**:
  rejected — two views with overlapping meaning is exactly the kind of
  ambiguity FINANCE_OS.md "disambiguation rules" was created to combat.
- **Compute cycles in application code per-call**: rejected — would
  duplicate SQL logic in both backends and force every consumer to know
  about closing days. Views are the right boundary.

## Consequences

- A purchase's `month_ref` no longer matches `strftime('%Y-%m', date)`. Any
  ad-hoc SQL that joined v_card_summary to a calendar-month dimension will
  need adjustment.
- The internal-category exclusion now reads from the `internal_categories`
  table (introduced in migration 010) instead of the hardcoded
  `credit-card-payment` literal. This catches the Portuguese taxonomy
  (`financeiro:pagamento-de-fatura-de-cartao`, `pagamento-recebido`, etc.)
  that the previous filter missed.
- `payment_status IN ('pending', 'em_aberto', 'parcial')` is still the
  `open_amount` filter. This is acknowledged as an open bug — `parcial`
  mixes installment-future-charges with currently-open-balance — and is
  tracked separately as the payment-status normalisation work.
- BigQuery had migration 020 applied with the cycle-based v_card_summary
  but an incorrect `v_card_open_now` (returned the accruing cycle instead
  of the open one). Migration 021 (BQ) and 020 (sqlite) fix the
  `v_card_open_now` definition idempotently via CREATE OR REPLACE / DROP +
  CREATE.
- The pulse's `card_due_label` now derives the due date from the cycle's
  `month_ref` + `billing_due_day` and labels overdue cycles as `"venceu
  DD/mmm"` so the message surfaces obligations the user has already
  missed rather than silently rolling them to the next cycle.
