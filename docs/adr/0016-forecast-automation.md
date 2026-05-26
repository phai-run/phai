---
type: ADR
id: "0016"
title: "Forecast automation: templates, detection layers and reconciliation"
status: accepted
date: 2026-05-25
---

## Context

The `forecast` table today is populated by hand: each row is one due-date /
amount tuple. That worked while we only had a handful of one-shot bills
(IPVA, viagem, matrícula). It does not scale to the real workload — recurring
subscriptions, fixed monthly bills (aluguel, diarista), credit-card
installment chains and variable-but-recurring categories (mercado, terapia)
all need to land in the same table for the cashflow projection (introduced
in [`report cashflow-chart --forecast`](../../crates/finance-cli/src/cashflow_chart.rs))
to be useful.

The end goal is conversational scenario analysis — *"can we afford a new
$X/month recurring activity?"*, *"when do my current installment chains
end?"* — which only works when the forecast table reliably reflects every
known future commitment.

What we already have:

- **Installment detection** in [`finance-core/src/installments.rs`](../../crates/finance-core/src/installments.rs)
  parses *"X/N"* and *"Parcela X de N"* into `InstallmentChain` rows with
  anchor date, total parcelas and remaining.
- **Recurring-merchant detection** in `report card-closed-insights` outputs
  merchants that appeared in ≥N months within the closed bill.
- **`forecast` table** with `forecast_id`, `due_date`, `description`,
  `amount`, `category_id`, `account_id`, `status`, `recurrence` (the
  `recurrence` field is currently informational only).
- **Rules engine** (`enrichment/rules`) categorises new transactions.
- **`report cashflow-chart --forecast`** stacks forecast onto realized bars
  and projects the saldo forward.

What is missing is the **orchestration layer** that detects patterns, lets
the user confirm them as templates, and materialises future forecast rows
deterministically on every sync.

## Decision

**We split forecasts into two concepts — `forecast_template` (the rule that
generates entries) and `forecast` (the materialised instances) — and add a
4-layer automation pipeline (installments → subscriptions → fixed bills →
envelopes) that runs at the end of every `sync pluggy` to keep the
`forecast` table current.**

The single materialised `forecast` row is still what the chart and reports
consume; the new `forecast_template` table is the *source* the orchestrator
regenerates from. This makes "throw away and rebuild" trivial and keeps the
existing `forecast` schema as the consumer-facing canonical table.

## Options considered

- **A** (chosen) — Separate `forecast_template` table; `forecast` stays as
  the consumer-facing instance table with a new optional FK
  `template_id`. Templates carry the rule (cadence, amount, banda,
  status); the orchestrator materialises N months ahead. Pros: clear
  separation; safe regeneration; explicit lineage from template→instance.
  Cons: one more table to maintain in both backends.
- **B** — Add `kind`, `template_id`, `parent_id` columns to the existing
  `forecast` table; a single row plays both roles depending on `kind`.
  Pros: one fewer migration. Cons: conflates "rule" and "instance"; harder
  to reason about idempotency and regeneration.
- **C** — No template table; the orchestrator is purely procedural and
  re-derives everything from heuristics on each run. Pros: no schema
  change. Cons: no place for user-confirmed adjustments ("yes, this Spotify
  charge is recurring; ignore that one-off Uber"); every run risks
  re-suggesting the same dismissed candidates.

## Schema

### New table: `forecast_template`

```sql
CREATE TABLE IF NOT EXISTS forecast_template (
  template_id        TEXT PRIMARY KEY,
  kind               TEXT NOT NULL,            -- 'installment' | 'subscription'
                                               -- | 'fixed' | 'envelope'
  description        TEXT NOT NULL,
  merchant_pattern   TEXT,                     -- normalized merchant key
                                               -- (null for envelopes)
  category_id        TEXT,
  account_id         TEXT,                     -- target account (cards for
                                               -- installments, checking for
                                               -- fixed bills)
  amount             TEXT NOT NULL,            -- expected magnitude (signed:
                                               -- positive = inflow,
                                               -- negative = outflow — per
                                               -- the new sign convention,
                                               -- see ADR-? below)
  amount_lower       TEXT,                     -- variation band lower bound
  amount_upper       TEXT,                     -- variation band upper bound
  cadence            TEXT NOT NULL,            -- 'monthly' | 'weekly' |
                                               -- 'one-shot' | 'card-cycle'
  next_due_day       INTEGER,                  -- day-of-month for monthly
  start_date         TEXT NOT NULL,
  end_date           TEXT,                     -- null = open-ended
  remaining_count    INTEGER,                  -- null = open-ended; non-null
                                               -- for installments
  source             TEXT NOT NULL,            -- 'detected' | 'manual'
  confidence         REAL,                     -- 0..1 — only when detected
  status             TEXT NOT NULL,            -- 'ativo' | 'pausado' |
                                               -- 'descartado'
  metadata_json      TEXT NOT NULL DEFAULT '{}',
  actor_id           TEXT NOT NULL,
  idempotency_key    TEXT NOT NULL,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);
CREATE INDEX idx_forecast_template_kind_status
  ON forecast_template(kind, status);
```

### `forecast` table — additive fields

```sql
ALTER TABLE forecast ADD COLUMN template_id TEXT;   -- FK to forecast_template
ALTER TABLE forecast ADD COLUMN realized_transaction_id TEXT; -- FK
                                                              -- to transactions
ALTER TABLE forecast ADD COLUMN realized_at TEXT;             -- when matched
```

Both fields are nullable so existing rows stay intact. New automation
populates `template_id` so we can regenerate cleanly; reconciliation
populates `realized_transaction_id` and `realized_at` when a transaction
matches.

### Status lifecycle

```
template:  ativo ─── (user pause) ──→ pausado
                ╰── (auto-end via end_date or remaining_count) ──→ descartado

instance:  ativo ─── (matching tx found) ──→ realizado
                ╰── (user manual) ─────────→ descartado
```

## The 4 layers (ordered by determinism)

| # | Kind | Trigger | Confidence | What it generates |
|---|---|---|---|---|
| 1 | `installment` | `InstallmentChain` detected on new tx | 100% | One forecast per remaining parcela, on the card's cycle day |
| 2 | `subscription` | Same merchant ≥3 of last 6 months, amount variance ≤10% | High | Monthly forecast, amount = median, capped at `end_date` if any |
| 3 | `fixed` | Same category+merchant ≥3 months, variance ≤30% | Medium | Monthly forecast, amount = avg, `amount_lower`/`upper` = 1σ band |
| 4 | `envelope` | Category-only (no merchant), regular monthly spend | Low | Per-category monthly envelope (analogous to `budget-status`) |

Layers 1 and 4 require no human confirmation (deterministic). Layers 2 and 3
go through a `fin forecast suggest` review queue first.

## Pipeline

```
sync pluggy
  ├─ download transactions (existing)
  ├─ apply rules / categorise (existing)
  ├─ refresh installment templates       ← Layer 1
  │   - for each InstallmentChain ≥1 remaining:
  │       upsert template (kind='installment'),
  │       materialise N forecasts ahead
  ├─ reconcile forecasts                  ← match instances to actuals
  │   - for each ativo forecast with due_date ≤ today:
  │       find candidate tx (account_id, ±3 days, amount within 5%)
  │       on match: set status='realizado', realized_transaction_id,
  │                 realized_at; emit audit event
  ├─ refresh subscription / fixed templates  ← Layers 2–3
  │   - for each ativo template:
  │       ensure forecasts exist for next 6 months
  └─ summarise (existing notify path)
```

The pipeline is idempotent at every step:

- Templates have stable `idempotency_key`s (e.g.
  `installment-{merchant_hash}-{anchor_yyyymm}` for layer 1).
- Materialised forecasts have stable keys
  (`tpl-{template_id}-{yyyymm}`).
- Reconciliation only updates `status='ativo'` rows.

## New surface

### CLI

- `fin forecast refresh` — runs the orchestrator on demand (sync wraps this).
- `fin forecast suggest` — lists detected subscription/fixed candidates
  with confidence; interactive accept/dismiss → seeds templates.
- `fin forecast templates list|show|pause|resume|delete` — manage templates.
- `fin scenario add-recurring --amount … --description … --start …` — adds
  a hypothetical template scoped to a scenario session; `cashflow-chart`
  re-projects with the overlay. (Builds on the existing `report scenario`.)

### Reporting

- `report cashflow-chart --forecast` (already shipped) automatically
  picks up the richer forecast set — no chart change required.
- `report installments` (already shipped) keeps showing parcela chains.
- New `report forecast-coverage` — what fraction of the next 6 months
  of expected spending is captured by templates (sanity gauge for the user
  to know when the forecast base is "complete enough" to trust simulations).

### OpenClaw skill (future PR)

- *"can we afford a new $X/month recurring expense starting in N months?"*
  → `scenario add-recurring --amount X --description … --start YYYY-MM`
  + `cashflow-chart --forecast` projection diff.
- *"when do my current installment chains end?"* → query templates with
  `kind='installment'` and report `end_date`s grouped by month.

## Consequences

- **Easier**: the chart's projection finally reflects everything the user
  has committed to, not just hand-entered bills. Simulation becomes
  meaningful. Replaying the orchestrator from scratch is safe (delete
  detected forecasts, re-run sync, same output).
- **Harder**: the recurrence engine now has to be right. Mis-detecting an
  Uber as recurring or missing a subscription distorts the projection. We
  mitigate this by routing layers 2 and 3 through a manual confirmation
  queue and treating layer 1 (deterministic) as the only auto-apply path.
- **New invariants**:
  - Every materialised `forecast` row from layers 1–4 carries a
    `template_id`; user-created one-off forecasts have `template_id = NULL`.
  - Reconciliation never deletes; only flips `status`.
  - Templates have stable idempotency keys; running the pipeline twice
    must produce identical state.
  - Audit events fire on every template create/pause/delete and every
    reconciliation (existing `AuditEvent` infra).
- **Triggers re-evaluation**:
  - If the heuristic for layers 2–3 produces too many false positives or
    misses common patterns, we may need a richer signal (e.g. fuzzy
    merchant clustering, weekly cadence).
  - If users want to share templates across multiple `accounts.owner`
    values, the template schema may grow an `owner` column and the
    orchestrator may need per-owner scoping (analogous to the `--owner`
    filtering in `tx review-human`).

## Rollout plan

1. **This ADR** lands first (no code).
2. **PR 1 — schema**: add the two migrations (sqlite + bigquery), register
   them in `migrations.rs`, expose the new `forecast_template` CRUD methods
   on `FinanceStore`. No CLI surface yet.
3. **PR 2 — Layer 1 (installments)**: detector → template upsert →
   materialiser → reconciliation. Triggered manually via `fin forecast
   refresh`; hooked into `sync pluggy` once we are confident.
4. **PR 3 — Layer 2 (subscriptions)**: detector + `fin forecast suggest`
   interactive queue. Layer 3 lands in the same PR if straightforward.
5. **PR 4 — Layer 4 (envelopes)**: bridge from `budget-status` to forecast
   instances.
6. **PR 5 — `scenario add-recurring`** + OpenClaw skill wiring for natural
   dialogue.

Each PR is small enough to ship under the existing release-please /
sentrux gates without surprising regressions.
