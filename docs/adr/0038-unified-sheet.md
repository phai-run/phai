---
type: ADR
id: "0038"
title: "Unified sheet: forecasts and scenario items as inline rows"
status: accepted
date: 2026-07-03
---

## Context

After shipping named planning scenarios (ADR-0037) the web UI had four
separate surfaces for expressing future intent: WarPlan sliders, forecast
cards (`ForecastSection`/`ManualPlannedTransactions`), a standalone
`ScenarioPanel`, and the main `PlanilhaView` sheet. The slider-based
WarPlan approach proved confusing in practice and the scattered placement
made it hard to build intuition about the combined projection.

The spreadsheet mental model the user relied on is: every planned item is
a *row* in the same table as transactions. Sorting, filtering, inserting,
and deleting rows all happen in one place.

## Decision

**Forecasts and scenario items render as inline rows inside `PlanilhaView`,
the single interaction surface for planning.** The following components are
removed:

- `WarPlanPanel` (the slider-based goal solver UI)
- `ForecastSection` / `ManualPlannedTransactions` (dedicated forecast cards)
- Standalone `ScenarioPanel` (replaced by `SheetScenarioBar` in the sheet header)
- The `simulation` prop on `PlanningChart` (WarPlan overlay)
- The `compact` mode on `PlanningChart` (pinned above sliders)

Retained:

- `forecastEnvelopeUpserted` event and `EnvelopeUpsert` API type (reusable for inline amount editing)
- `firstShortfallMonth` / `solveRequiredSaving` shortfall indicator (informational, not slider-driven)
- All scenario persistence, projection, and chart overlay from ADR-0037

### Routing

The sheet uses contextual routing (via `routeSheetDelete`, `routeSheetAmountEdit`, `routeSheetAdd` in `derivations.ts`):

- No active scenario: writes go to baseline forecasts (create / delete / re-amount)
- Active scenario: writes become `plan_change` deltas (add\_one\_shot / skip\_forecast / adjust\_amount / end\_template)

### Sort and filter persistence

User sort/filter preferences are stored in `localStorage`, not LiveStore.
This avoids a `STORE_VERSION` bump for a UI-only concern and survives
store wipes on version changes.

## Options considered

- **Option A (chosen)**: inline rows in PlanilhaView, remove dedicated surfaces
  - Pros: familiar spreadsheet metaphor, single interaction surface, less code
  - Cons: sheet grows more complex; category treemap moves to its own tab

- **Option B**: keep WarPlan + scenario panel, add sheet as a third surface
  - Pros: no demolition
  - Cons: three places to express the same intent, user confusion about which to use

## Consequences

- PlanilhaView is the sole planning surface for baseline and scenario edits.
- Category/transaction browsing remains in MonthDetail under the "categories" tab.
- The WarPlan goal-solver UX is gone; the passive shortfall indicator on the chart remains.
- Future inline amount editing can reuse the envelope-upsert event without changes.
- Supersedes the UI portion of ADR-0031 (the slider overlay on the chart).
