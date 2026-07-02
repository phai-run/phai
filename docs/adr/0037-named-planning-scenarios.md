---
type: ADR
id: "0037"
title: "Named planning scenarios: persisted what-if deltas over the live forecast baseline"
status: accepted
date: 2026-07-02
---

## Context

The spreadsheet habit this replaces: clone the current month's tab three
times into the future, then edit the copies — add a trip, shrink the grocery
budget, pretend a 10× purchase happened. The copies go stale within days
because the underlying data is dynamic; the clone captures a snapshot, not a
plan.

phai already has a *live* baseline: the forecast engine (ADR-0016) detects
installments/subscriptions/fixed bills, materialises `forecast` rows months
ahead, and reconciles them against real transactions on every sync. The
ephemeral what-if (`phai forecast scenario`) answers one recurring-commitment
question read-only, and ADR-0031 sketched a single persisted "plan" of
envelopes, explicitly deferring **Option B — multiple named scenarios** until
someone needed them. This ADR is that need: editable, comparable, promotable
what-if scenarios in both the CLI and the web app.

## Decision

A **scenario is a set of typed deltas persisted over the live baseline** —
nothing is copied, so the projection can never go stale. Two tables
(migration `041`, both backends):

- `plan_scenario` — `scenario_id`, `name`, `status` (`ativo` | `arquivado` |
  `promovido`), audit fields.
- `plan_change` — typed deltas with dedicated columns (no JSON blob):
  | kind | meaning | key fields |
  |---|---|---|
  | `add_one_shot` | one-shot entry in a month | `month`, `amount` |
  | `adjust_amount` | override a forecast's amount | `target_forecast_id`, `amount` (absolute) |
  | `skip_forecast` | drop one forecast occurrence | `target_forecast_id` |
  | `end_template` | stop a recurrence from a month on | `target_template_id`, `effective_from` |
  | `hypothetical_installment` | N monthly parcels | `effective_from`, `amount`, `months_count` |

**Projection** is a pure function,
[`phai_core::scenario::apply_scenario`](../../crates/phai-core/src/scenario.rs)
`(baseline, templates, changes, horizon) → { virtual_forecasts,
monthly_delta, orphaned_change_ids }`, consumed by the CLI (`phai scenario
show|diff`) and the serve bridge (`GET /api/scenario/projection`, which
returns the `/api/chart` shape with the deltas layered on future months).
The web client never re-implements the engine — it renders the served
projection and only does trivial optimistic arithmetic locally.

**Orphans**: a change whose target was realized/removed by the engine is a
no-op, recomputed on every read and surfaced as `orphaned` — reality wins
over the plan. Cleanup is only ever explicit (`phai scenario prune` / the ×
button); nothing is deleted automatically.

**Promotion** (`phai scenario promote`, `POST /api/scenario/promote`)
applies deltas to the real plan in a deterministic order (end-template →
skip → adjust → add → installment). Every write uses a natural idempotency
key `scenario-{scenario_id}-{change_id}` (ADR-0022), so a retry after a
partial BigQuery failure re-executes as a no-op — important because BigQuery
has no multi-statement transaction here. Applied changes flip to
`aplicado`, the scenario to `promovido`, all with `AuditEvent`s.

**Surfaces**: full CLI (`phai scenario create|list|show|diff|add|adjust|
skip|end-template|installment|delete-change|archive|delete|prune|promote`,
all with `--raw`), REST under `/api/scenario/*` (new `Scenarios` read-cache
family; promote also invalidates forecasts/chart/templates), and the web
`ScenarioPanel` + a dashed scenario saldo line with a shaded wedge on the
chart (the ADR-0031 visual language). LiveStore gained `scenarios`,
`scenarioChanges`, `scenarioChartMonths` and `ui.activeScenarioId`
(`STORE_VERSION` 11); scenario/change ids are client-generated and honoured
by the bridge, so offline retries stay idempotent without ack remapping.

## Alternatives rejected

- **`scenario_id` column on `forecast`** — every baseline query
  (`upcoming_forecasts`, chart, reconciler) would need a `scenario_id IS
  NULL` filter; one forgotten filter leaks a hypothesis into the real saldo.
  Three of the five operations are *modifiers* of existing rows anyway, not
  new rows.
- **Copying forecasts per scenario** — reproduces the spreadsheet defect
  this feature exists to fix (stale copies).
- **Porting the projection engine to TS/WASM** — two engines drift; WASM is
  disproportionate. The bridge serves the projection instead.
- **Auto-pruning orphans on sync** — silently mutating a user's plan is
  surprising; lazy detection + explicit prune keeps the user in charge.

## Consequences

- The war-plan goal sliders (ADR-0031) still write envelopes to the
  *baseline*; a live slider simulation and a scenario overlay don't compose
  in v1 (the scenario wins while active). A future revision can capture
  goals as scenario deltas.
- `forecast scenario` (the ephemeral single-commitment what-if) remains for
  agent "can I afford this?" queries; its help points here.
- Relation to ADR-0031: this materialises its "Option B". 0031 stays valid
  for the single plan-of-envelopes; its chart overlay language (dashed line
  + wedge) is reused here.
