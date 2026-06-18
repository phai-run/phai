---
type: ADR
id: "0030"
title: "Commitment tiers: a derived controllability axis for planning"
status: proposed
date: 2026-06-18
---

## Context

The product goal driving this work is blunt: help the user **stop spending more
than they earn**. The lever for that is not "spend less on everything" — it is
"spend less on the part you actually control". Expenses fall on a spectrum of
controllability:

- **Immovable commitments** — rent/housing, therapy, school. Recurring,
  indefinite, non-negotiable in the short term. The amount may vary month to
  month (a utility bill), but the commitment does not.
- **Time-boxed commitments** — credit-card installment chains. Locked, but they
  expire on a known date and free cash as they end.
- **Cancellable recurring** — subscriptions. Recurring and indefinite, but the
  user can cancel at will. Medium cut margin.
- **Discretionary variable** — groceries, leisure, one-offs. The largest cut
  margin; where a budget actually bites.

phai already encodes this distinction once, on the **forecast template**:
[`forecast_template.kind`](../../schema/sqlite/034_forecast_template.sql) is one
of `installment | subscription | fixed | envelope` (ADR-0016). But `kind` lives
on the *generator* of a future expectation, not on realized transactions, and it
is not surfaced as a filter/grouping axis anywhere. The three views that need
this axis — the sheet (`planilha`), the categorias treemap, and the planning
mode (`plano`) — would each have to re-derive "how controllable is this row?"
independently, which guarantees drift.

Constraints:

- **Privacy (ADR-0008).** Which merchant is "fixed" vs "discretionary" is
  user-specific classification. It must stay in templates/rules/runtime data —
  never hardcoded into shared Rust, migrations, fixtures, or tests.
- **No amount coupling.** A "fixed" expense whose amount varies (e.g. energy
  bill) must stay classified as fixed without re-tagging every month. So the
  classification cannot be pinned to an individual transaction's value.
- **Single source.** Sheet, treemap, and planning must read the *same* tier for
  the same money, or the three views will disagree.

## Decision

**Introduce `commitment_tier` — a derived, three-value controllability axis
(`locked | cancellable | variable`) collapsed from `forecast_template.kind`,
computed once in a shared derivation and attached to each `TxView`. It is
derived, not persisted: a transaction's tier comes from its linked forecast
template (via reconciliation), falling back to a category default, then to
`variable`. The sheet filter, the treemap (colour + grouping lens), and the
planning grouping all read this one field.**

Tier mapping:

| `commitment_tier` | from `forecast_template.kind` | cut margin |
|---|---|---|
| `locked` 🔒 | `fixed` + `installment` | none short-term; installments self-expire |
| `cancellable` ✂️ | `subscription` | full — cancel at will |
| `variable` 🎚️ | `envelope` / no template | max — budget bites here |

Four `kind`s collapse to three tiers because the planning question is "can I cut
this?", and `fixed`/`installment` answer it the same way ("not now") — they
differ only in *when they end*, which the chart already reads from
`end_date`/`remaining_count`, not from the tier.

## Options considered

- **Option A** (chosen): Derived three-tier field on `TxView`, computed in one
  shared derivation from the linked template's `kind`, with a category-default
  and `variable` fallback for untemplated transactions.
  *Pros:* single source consumed by all three views; no migration; honours the
  privacy boundary (classification stays in templates/rules); decoupled from
  per-transaction amount; collapses to the 3 tiers planning actually wants.
  *Cons:* depends on the tx→template reconciliation link existing; untemplated
  spend needs a fallback heuristic; tier is recomputed in the client, not
  queryable directly in SQL.
- **Option B**: New persisted `tier` column on `transactions`, set by the rules
  engine on classification.
  *Pros:* explicit, queryable in SQL, one read.
  *Cons:* migration in both backends; duplicates information already in `kind`
  and risks drift between the two; re-tagging churn when amounts/merchants
  shift; pushes user-specific classification closer to shared schema.
- **Option C**: Keep the four `kind`s as-is and let each view map them ad hoc.
  *Pros:* no new abstraction.
  *Cons:* every view re-implements the 4→3 collapse and the untemplated
  fallback → exactly the drift this ADR exists to prevent.

## Consequences

- The sheet gains a tier filter, the treemap gains tier-colouring and a
  "controllability" grouping lens, and planning groups cuts by tier — all from
  one derived field. This is the shared substrate ADR-0031 builds the plan on.
- **Untemplated transactions need a defined fallback.** A row with no linked
  template resolves to its category's default tier, else `variable`. The
  fallback table is itself user data (category → default tier), not hardcoded.
- **Mixed-tier categories** (e.g. `lazer` holding both a subscription and a
  one-off) force a rule in the treemap's category lens: tint by the dominant
  tier or split the tile. ADR-0031's chart sidesteps this by offering a
  tier-first lens where the tier *is* the partition.
- The reconciliation link `transaction ↔ forecast_template` (ADR-0016's
  `realized_transaction_id`) becomes load-bearing for classification accuracy;
  weak reconciliation degrades tiering to the category fallback.
- Re-evaluate if a fourth tier is needed (e.g. splitting "annual" commitments
  out of `locked`), or if SQL-side tier queries become a requirement (→ revisit
  Option B as a materialised companion, like ADR-0012's `amount_cents`).
