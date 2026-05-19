---
type: ADR
id: "0012"
title: "Exact decimal aggregation via `amount_cents` companion column"
status: active
date: 2026-05-18
---

## Context

[ADR-0003](0003-rust-decimal-end-to-end.md) bans `f64` on the money path
and stores amounts as `TEXT` (SQLite) or `NUMERIC` (BigQuery). That keeps
single-row values exact, but it does not solve aggregation.

BigQuery `NUMERIC` is exact under `SUM` and `ROUND`. SQLite, however, has
no decimal type. Aggregate views historically used
`SUM(CAST(amount AS REAL))`, which converts each row to IEEE-754 double
before adding. With thousands of rows over many months this accumulates
sub-cent drift that surfaces as off-by-one-cent differences between the
SQLite cashflow report and a hand-computed total — exactly the trust
failure ADR-0003 set out to prevent.

The CLI reads the SQLite backend in local mode, so this drift is visible
to users despite ADR-0003 nominally being in force.

## Decision

**Add an `amount_cents INTEGER` column to `transactions` and aggregate
through it. Convert to `Decimal::new(cents, 2)` only at the final
boundary.**

- **SQLite**: `amount_cents` is a `VIRTUAL` generated column derived from
  `CAST(ROUND(amount * 100) AS INTEGER)`. (Apple's bundled libsqlite3
  rejects `STORED` even on 3.51.0; `VIRTUAL` is functionally identical
  for our use case because `SUM` of integers is exact regardless of
  storage.) All aggregate views — `v_cashflow`, `v_monthly_spend`,
  `v_card_summary`, `v_forecast_vs_actual.actual_total` — `SUM(amount_cents)`
  and divide by `100.0` at the end. ORDER BY / WHERE comparisons that
  used `CAST(amount AS REAL)` switch to `amount_cents` (integer compare).
- **BigQuery**: the column exists for cross-backend schema parity and is
  populated by the upsert. The BQ views are *not* rewritten — `NUMERIC`
  is already exact under `SUM`.
- **Rust read path**: `LocalStore::cashflow` reads `amount_cents` from
  `v_transactions_reportable`, accumulates `i64` per month, and emits
  `Decimal::new(cents, 2)` at the end. This skips per-row Decimal parsing
  and is both faster *and* exact.
- The Rust `TransactionRecord::amount_cents` field is **write-side only**:
  it carries cents into the BigQuery upsert and is left `None` on read
  paths. Consumers that need cents query the view directly.

This ADR refines, not supersedes, ADR-0003: `Decimal` remains the
boundary type and the in-memory representation; `amount_cents` is a
storage-layer optimisation for exact `SUM` on a backend without native
decimal aggregation.

## Options considered

- **Integer cents in storage, `Decimal` at the boundary** (chosen): all of
  the precision benefit, none of the API change. ADR-0003 still holds —
  cents are an implementation detail of the SQLite path.
- **Push `SUM` into Rust for every aggregate**: works but spreads the
  same logic across many storage methods. Was used as the interim fix in
  the `cashflow` path before this ADR; now consolidated behind the views.
- **Accept REAL drift, round at the edge**: rejected. ADR-0003's invariant
  is exact equality on decimals — masking error with `round_dp(2)` only
  hides the failure mode until the next aggregation.
- **Migrate SQLite to `bigdecimal` / extension**: heavier, harder to ship
  in a single-binary CLI; no upside over an `INTEGER` companion column.

## Consequences

- **Easier**: SUM-based reports (`cashflow`, `card-summary`, `monthly-spend`,
  `forecast-vs-actual.actual_total`) are exact on the SQLite backend.
  ORDER BY on signed amount uses integer comparison (no REAL cast in the
  hot path).
- **Harder**: every new view that aggregates amounts must remember to use
  `amount_cents`, not `CAST(amount AS REAL)`. A `CAST(amount AS REAL)` in
  a new migration is a code-review blocker. Existing exceptions live in
  `v_forecast_vs_actual.variance` (forecast.amount has no cents column —
  documented inline) and may be revisited if forecast becomes a hot
  aggregation path.
- **Re-evaluation triggers**: a future SQLite release with native
  `NUMERIC` support, or a backend swap that removes SQLite from the
  read path.
