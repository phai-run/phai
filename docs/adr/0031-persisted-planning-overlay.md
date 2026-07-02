---
type: ADR
id: "0031"
title: "Persisted planning overlay: plan as a first-class layer on the chart"
status: proposed
date: 2026-06-18
---

## Context

The annual chart ([`PlanningChart.tsx`](../../crates/phai-cli/web/src/views/PlanningChart.tsx))
renders realized vs forecast bars and a saldo line (ADR-0024/0025). The planning
mode (`plano`,
[`WarPlanPanel.tsx`](../../crates/phai-cli/web/src/views/plano/WarPlanPanel.tsx))
lets the user drag per-category goal sliders; while dragging,
[`applySimulationToModel`](../../crates/phai-cli/web/src/views/chart/model.ts)
overlays a **live, ephemeral** simulation — forecast outflows shrink from a
chosen month, future balances lift by the accumulated saving. Confirming runs
`saveGoals` → `buildEnvelopeWrites`, persisting the goals as budget-envelope
forecasts (`description: "meta {parent}"`, ADR-0016 `kind=envelope`).

The user's requirement: *"as I cut or simulate cuts, I watch outflows shrink and
cash accumulate; and when I save a plan, the chart keeps showing it — real vs
forecast as today, plus the planned trajectory, so I can see how it ends up and
what still needs to be done to hit the goal."*

The gap: **saving dissolves the plan into the baseline.** The saved envelopes
land in the same `forecasts` table the chart already sums into `fcOuts`, so
after save there is exactly one forecast trajectory again. The before/after
contrast — *"where I was heading"* vs *"where the plan takes me"* vs *"the
goal"* — exists only mid-drag and is lost on save and on reload. There is also
no goal target on the chart and no answer to *"what cut is required to reach
it"*.

Constraints:

- Must reuse the existing forecast/envelope machinery and `applySimulationToModel`
  rather than fork the cash model.
- Must survive reload / cross-session (the live overlay does not).
- Cuts must be sourced from the controllability axis (ADR-0030) so a plan reads
  as "cancel these ✂️, tighten these 🎚️, leave 🔒 untouched".
- No personal data in shared code (ADR-0008); no new `f64` money (ADR-0003).

## Decision

**Persist a plan as a first-class, separable layer — a dated set of cuts/targets
tagged so it is distinguishable from the organic forecast — instead of folding
it into the baseline. The chart computes two cash series in one pass: `baseline`
(forecast *excluding* the plan's tagged envelopes) and `planned` (forecast
*including* them), and renders three trajectories (real · baseline · planned)
plus a goal line, with the wedge between baseline and planned shaded as freed
cash. Add an inverse solver: given a goal (saldo ≥ target by month M), compute
the monthly saving required and flag whether the current plan reaches it.**

Mechanically: plan envelopes carry a discriminator (a `source`/metadata marker)
so `buildModel` can subtract them to reconstruct the baseline. `buildModel`
returns `{ baseline, planned }` coexisting rather than one series replacing the
other; `applySimulationToModel` stays as the live (unsaved) overlay on top of
`planned`. The plan's cut list is grouped by `commitment_tier` (ADR-0030).

## Options considered

- **Option A** (chosen): Plan = the existing envelope/forecast rows, tagged with
  a plan discriminator so the chart can compute `baseline = forecast − plan` and
  `planned = forecast`. One active plan.
  *Pros:* reuses forecast/envelope writes, the bridge, and
  `applySimulationToModel`; minimal schema (a marker, not a table); the plan is a
  removable set; baseline/planned fall out of one pass.
  *Cons:* the chart does a two-pass split; the discriminator must be clean enough
  that organic forecasts never leak into the plan layer; only one plan at a time.
- **Option B**: Dedicated `plans` table (header + line items) separate from
  `forecasts`, with its own bridge endpoint and seed event.
  *Pros:* clean model; named scenarios; multiple concurrent what-ifs.
  *Cons:* new table in both backends; new seed/write surface; more moving parts
  than the single-active-plan requirement justifies today.
- **Option C**: No persistence — keep only the live overlay.
  *Pros:* zero schema.
  *Cons:* fails the requirement; the plan vanishes on reload.
- **Option D**: Snapshot the pre-plan baseline as a frozen series at save time.
  *Pros:* exact before/after.
  *Cons:* the frozen baseline drifts from reality as new realized data lands;
  stores derived data that should be recomputable.

Option A wins on reuse and on matching the actual scope (one active plan
reflected on the chart). B is the documented upgrade path.

## Consequences

- The chart gains a third trajectory, a goal line, and the freed-cash wedge; the
  saved plan stays visible across reloads instead of dissolving. Baseline is
  reconstructed by excluding plan-tagged envelopes — so the discriminator is
  load-bearing and must be set on every plan write and nowhere else.
- `buildModel` changes shape (returns `baseline` + `planned`); call sites and the
  chart's bar/line rendering and legend update accordingly. Bars show the planned
  height solid with a dashed "ghost" cap at the pre-cut height.
- The inverse solver turns the goal into a target monthly saving and a
  reach/short verdict — the *"what still needs to be done"* readout. It inverts
  the same arithmetic as `applySimulationToModel`; no second model.
- Plans compose with ADR-0030: cuts are grouped and coloured by
  `commitment_tier`, tying the sheet/treemap/planning views to the chart's plan
  layer.
- **Re-evaluate → Option B** when the user needs multiple concurrent saved
  scenarios (compare plan A vs plan B), named plans, or plan history. At that
  point migrate the tagged-envelope set into a dedicated `plans` table in both
  backends. *Materialised by
  [ADR-0037](0037-named-planning-scenarios.md) (named planning scenarios,
  `plan_scenario` + `plan_change`); this ADR remains valid for the single
  plan-of-envelopes written by the goal sliders.*
- All plan amounts stay `Decimal` end-to-end (ADR-0003); every plan write emits
  an `AuditEvent` (ADR-0005) like any other forecast write.
