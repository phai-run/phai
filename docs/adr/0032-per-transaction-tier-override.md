---
type: ADR
id: "0032"
title: "Per-transaction commitment-tier override via a sparse table"
status: proposed
date: 2026-06-18
---

## Context

ADR-0030 introduced `commitment_tier` as a *derived* axis (installment →
locked, subscription → cancellable, else variable) and floated a "category
default tier" as the fallback for everything untemplated. Field use immediately
broke that assumption: a user has many `moradia` transactions where **some are
fixed (rent) and some are not** — a category-level default would mislabel the
rest. The detector also does not reliably classify these as `fixed` (rent
surfaced as a `subscription` candidate), so the forecast-derived fixed set
(ADR-0030) cannot carry the intent either.

What the user actually needs is to pin **specific transactions** to a tier —
including in bulk via the sheet's existing multi-select — and have that survive
across sessions in the system of record (BigQuery for the production runtime,
SQLite locally).

Constraints:

- The tier of most transactions stays derived; only manual pins need storage —
  so the store should be **sparse**, not a column on every transaction row.
- Must not add a field to `TransactionRecord` (it is constructed in many query
  sites across both backends; a column there is high blast-radius).
- Generic infrastructure only — the override rows are user data, but the table
  carries no personal counterparties or labels (ADR-0008).
- Every write emits an `AuditEvent` (ADR-0005); migrations mirror across both
  backends (ADR-0006); amounts stay `Decimal` (n/a here).

## Decision

**Persist per-transaction tier overrides in a dedicated sparse table,
`transaction_tier(transaction_id, tier, …)`, fronted by two new `FinanceStore`
methods — `set_commitment_tier` (upsert, `None` clears) and
`commitment_tier_overrides` (read all pairs for the seed). The bridge applies
the override through the existing human-review write path and exposes it on the
`/api/transactions` seed; the web `commitmentTier` derivation lets a manual
override win over the derived signals.**

Precedence in `commitmentTier`:

1. manual per-transaction override → that tier
2. installment flag → locked
3. subscription flag → cancellable
4. forecast-derived fixed category (ADR-0030) → locked
5. variable

So a subscription inside a "fixed" category stays cancellable unless the user
pins it; a pinned rent transaction is locked while its siblings stay derived.

## Options considered

- **Option A** (chosen): Sparse `transaction_tier` override table + two
  `FinanceStore` methods; `TransactionRecord` untouched; the seed left-joins the
  overrides in.
  *Pros:* zero blast-radius on the many `TransactionRecord` query sites; sparse
  storage matches sparse intent; mirrors the `reviewOverlay` model; per-tx
  granularity the user asked for.
  *Cons:* the transactions seed does a second read (the overrides map); tier
  override is not queryable on the transaction row in SQL without a join.
- **Option B**: `commitment_tier` column on `transactions`, written through the
  review patch.
  *Pros:* single read on the seed; queryable inline.
  *Cons:* touches `TransactionRecord` and every SELECT/upsert/mapper in both
  backends; re-tagging churn; column is null for the overwhelming majority.
- **Option C**: A `merchant → tier` rule in the existing rules engine.
  *Pros:* auto-applies to future recurring charges (next month's rent inherits).
  *Cons:* not per-transaction — the user explicitly rejected category/merchant
  granularity ("nem tudo de moradia quero travar"). Kept as a possible future
  complement, not the primary mechanism.

## Consequences

- The sheet gains a bulk "set tier" action over its existing multi-select; the
  override persists to BigQuery/SQLite and reflects on the sheet, treemap, and
  planning the same way (ADR-0030 reads the same derivation).
- The `/api/transactions` seed reads `commitment_tier_overrides` once per load
  and merges it into the row shape; the web store carries an explicit
  `commitmentTierOverride` per transaction.
- New `FinanceStore` methods must be implemented by every backend (SQLite,
  BigQuery) and every mock; a missing impl fails compilation (caught in CI).
- A migration (`040_transaction_tier`) lands in both `schema/sqlite/` and
  `schema/bigquery/`, idempotent, registered in `migrations.rs`, auto-applied
  on next command (ADR-0013) — including against the production BigQuery
  dataset.
- Re-evaluate toward **Option C** if the user later wants recurring pins to
  auto-propagate to future months without re-tagging.
