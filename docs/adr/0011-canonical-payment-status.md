---
type: ADR
id: "0011"
title: "Canonical payment_status vocabulary"
status: accepted
date: 2026-05-18
---

## Context

The `transactions.payment_status` column accumulated five overlapping values
over time:

- `pago` / `posted` — duplicate PT/EN tokens for "finalised on a closed bill"
- `em_aberto` / `pending` — duplicate PT/EN tokens for "on the current open bill"
- `parcial` — Pluggy's marker for "future parcela of an installment chain"
- `confirmed` / `unconfirmed` — internal aliases the manual-entry path emitted

The May 2026 audit ([PLAN.md](../../PLAN.md)) found a March 2026 cutover
in the data: until February rows used `pago`/`parcial`, March mixed all
five at once, April standardised on `pending`/`posted`. The mix was
caused by an enrichment-code change that didn't migrate existing rows.

The mix has a concrete behavioural cost: `v_card_summary.open_amount`
summed `payment_status IN ('pending', 'em_aberto', 'parcial')`. That
folded future installments (`parcial`) into the "open right now" total
of a card's bill, inflating immediate obligations with money that does
not yet belong to any closed bill.

## Decision

**Three canonical values, normalised at sync time and via a one-shot
data migration.**

| Canonical | Meaning |
|---|---|
| `posted` | Charge is finalised on a closed bill (credit) or settled in the account (checking). |
| `pending` | Charge is on the current open cycle, awaiting closure. |
| `installment` | Future parcela of an installment chain. Does not belong to any bill yet. |

The Pluggy ingestion path (`crates/finance-core/src/pluggy.rs`) routes raw
status through `normalize_payment_status()`, which maps the legacy tokens
(`pago`/`em_aberto`/`parcial`/`confirmed`/`unconfirmed`) onto the canonical
trio and passes anything else through unchanged.

`v_card_summary` is rewritten so `open_amount` sums only `pending`. A new
sibling column `installments_future` surfaces the parcela exposure as a
separate signal. `v_card_open_now` carries the new column through.

`CardSummaryRow` gains an `installments_future: Decimal` field. The pulse
renders parcelas inline next to the open bill: `Aline Nubank · R$ 4.463
(venceu 10/mai) · +R$ 1.230 em parcelas`.

## Options considered

- **Three canonical values** (chosen): minimal vocabulary that captures
  the three states that actually drive UX. Each value answers a different
  user question ("how much do I owe now?" vs "what am I committed to?").
- **Two values (`open` / `closed`)**: rejected — collapsing `pending` and
  `installment` is what got us here. The user needs to see them apart.
- **Keep the bilingual aliases**: rejected — every consumer would have to
  re-implement the predicate. The duplication is a footgun.
- **Drop `parcial` rows entirely (model installments as a separate table)**:
  considered, deferred. `installments.rs` already detects chains; a future
  ADR can move parcelas to a dedicated table and reduce `payment_status` to
  a binary `posted`/`pending`. Out of scope here.

## Consequences

- The Rust-side `is_open_card_payment_status` matches only `pending`
  (with PT/legacy fallbacks for tolerance during the migration window).
  Reports stay correct against rolling deployments where the migration
  hasn't been applied yet.
- `installments_future` is now visible on every consumer of
  `v_card_summary` / `cards_open_now()`. The pulse uses it; an agent can
  query it via `report card-summary --raw`.
- The legacy aliases are still tolerated on read so we don't break if a
  third party (Pluggy version, manual import) emits them. We do not emit
  them.
- The `parcial` → `installment` mapping is a heuristic: it assumes every
  Pluggy `parcial` row IS an installment. If Pluggy ever overloads that
  string for another meaning, we'd lose that signal. We accept the risk
  because it matches every row in the production dataset audited.
- A migration that's been released should not be edited. If a follow-up
  reveals a row that should have stayed `posted` but became `installment`,
  add another migration to correct it — do not amend 021/022.
