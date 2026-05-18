---
type: ADR
id: "0003"
title: "`rust_decimal::Decimal` end-to-end for monetary amounts"
status: active
date: 2025-12-22
---

## Context

Floating-point arithmetic on money is a well-known foot-gun: `0.1 + 0.2 ≠ 0.3`, totals drift by cents, balances don't reconcile with the bank. In a personal-finance system the user reads daily, a one-cent drift is *observable* — and erodes trust faster than any UI bug.

Pluggy returns amounts as JSON strings. BigQuery has a native `NUMERIC` type (38-digit precision). SQLite has no decimal type but stores strings reliably. The cheap path — parse JSON numbers as `f64`, normalize on display — is structurally unsound: the rounding error has already happened by the time the value is in memory.

## Decision

**All monetary amounts use `rust_decimal::Decimal` from the API boundary to the database row. `f64` and `f32` are banned for money.** Specifically:

- Parsing Pluggy / BigQuery JSON: `decimal_from_str` (no intermediate `f64`).
- Arithmetic: `Decimal`'s operator overloads — no `as f64` casts.
- SQLite storage: `TEXT` (decimal-as-string), bound through explicit conversion.
- BigQuery storage: `NUMERIC`, serialized as a JSON string in REST payloads.
- Human format: locale-aware formatting applied only at the very last presentation step (`crates/finance-cli/src/human_format.rs`).

`rust_decimal::Decimal` is enabled with the `serde` feature so models serialize/deserialize cleanly across the JSON boundaries (CLI `--raw` output, BigQuery REST, audit-event payloads).

## Options considered

- **`rust_decimal::Decimal`** (chosen): native Rust, mature, fast enough, plays well with `serde` and `rusqlite`. Precision is sufficient for any plausible personal-finance value.
- **`bigdecimal`**: arbitrary precision but heavier, slower, and adds friction to BigQuery `NUMERIC` mapping (which has a known precision/scale).
- **Integer cents (`i64`)**: simplest model, but loses precision once tax/installment math introduces fractions of a cent, and complicates multi-currency handling.
- **`f64` everywhere**: rejected on the merits — see Context.

## Consequences

- **Easier**: trust in totals; reconciliation with bank statements; safe arithmetic in views and reports; clean BigQuery round-tripping.
- **Harder**: arithmetic syntax is slightly more verbose; one external crate to keep current; serialization needs the `serde` feature.
- **Invariants for the codebase**:
  - `f64` / `f32` appearing on an amount path is a code-review blocker.
  - `as f64` on a `Decimal` is banned. If a chart library needs a float, convert at the chart boundary, not in the domain layer.
  - Tests on amounts use exact `Decimal` equality, not epsilon comparisons.
- **Re-evaluation triggers**: a hard performance ceiling from `Decimal` arithmetic in a hot path (none observed); or a backend that cannot store decimals losslessly (not a constraint of our current backends).
