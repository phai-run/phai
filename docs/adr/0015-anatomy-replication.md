---
type: ADR
id: "0015"
title: "Anatomy replication propagates human-curated description and purpose from prior same-merchant transactions"
status: superseded
date: 2026-05-21
superseded_by: "0033"
---

## Context

When transactions arrive from Pluggy, `description` and `purpose` are always `NULL`. LLM enrichment sets `merchant_name` (and `category_id`), but `description` (short human label) and `purpose` (subjective intent) require manual curation via `finance tx set-anatomy` or `finance tx review-human`.

For recurring merchants — subscriptions, regular restaurants, frequent stores — the human-curated anatomy is the same or nearly the same across occurrences. Requiring manual re-entry on every new transaction is friction without value.

## Decision

**Add a replication step that copies `description` and `purpose` from a prior high-trust transaction with the same `merchant_name` to any new transaction that is missing those fields. If `merchant_name` is not available yet, use the exact normalized `raw_description` as a conservative fallback key.**

Replication is offered in two modes:

1. **Inline, post-enrichment**: immediately after the enrichment pipeline writes `merchant_name`, `try_replicate_anatomy` runs as a best-effort step inside `apply_auto_decision` and `apply_decision`. Errors are logged but do not abort enrichment.

2. **Batch CLI command**: `finance tx replicate-anatomy [--dry-run] [--limit N]` processes all categorized transactions that have `merchant_name` but are missing `description` or `purpose`. Supports `--dry-run` for inspection before applying.

### Donor selection

The engine calls `find_anatomy_donors(match_key, exclude_id)`:
- Matches on `LOWER(TRIM(merchant_name))` when available.
- Falls back to `LOWER(TRIM(raw_description))` when `merchant_name` is missing, which supports older local data that has human-curated anatomy but was not merchant-enriched.
- Returns up to 5 candidates ordered by `transaction_date DESC`.
- Requires the donor to have at least one non-blank `description` or `purpose`.
- No `category_source` filter — `description` and `purpose` are exclusively set by humans (via `set-anatomy` or `review-human`), so any transaction that has them set is a valid donor regardless of how its category was determined.

From the candidates, `select_donor` picks the best match by score:
- +2 same `category_id` as the target.
- +1 amount within ±20% of the target.
- Ties broken by recency (most-recent first).

Only `NULL` target fields are filled — existing values are never overwritten.

### Tracking

Each replication emits an `anatomy_replicated` audit event with `donor_id`, `description_replicated`, and `purpose_replicated` in `diff_json`. The `classifier_trace` (LLM reasoning) is not touched.

## Options considered

- **Replicate based on raw_description token** (rejected): `raw_description` varies per transaction; `merchant_name` is the LLM-cleaned, stable identifier — a far better grouping key.
- **Extend the LLM prompt to emit description and purpose** (future option): would require schema changes to `EnrichmentResult` and prompt re-engineering. Replication is a simpler and privacy-safe first step.
- **Replicate in a separate async background job** (rejected): adds operational complexity; the inline post-enrichment hook adds negligible latency and keeps the pipeline self-contained.

## Consequences

- `FinanceStore` gains two new methods: `find_anatomy_donors` and `replicable_anatomy_candidates`. Both backends (SQLite, BigQuery) implement them.
- `finance-core` gains `enrichment::replication` module with pure, unit-tested logic.
- The enrichment pipeline gains a best-effort anatomy propagation step with no change to the LLM call or the `EnrichmentResult` schema.
- Transactions with no prior same-merchant or same-raw-description history still require manual `set-anatomy` — replication only helps when history exists.
- Anatomy replication does NOT create rules, modify `category_source`, or affect `classifier_trace`. It is purely a fill-in-the-gaps convenience for recurring merchants.
