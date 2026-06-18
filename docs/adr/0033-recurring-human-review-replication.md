---
type: ADR
id: "0033"
title: "Recurring human review replication includes category"
status: active
date: 2026-06-18
---

## Context

[ADR-0015](0015-anatomy-replication.md) replicated human-curated `description` and `purpose` from a prior same-merchant transaction, but it intentionally did not replicate category. That left recurring commitments such as rent, school fees, and condominium charges dependent on the next enrichment/rule pass even after the user had already reviewed a prior month.

Those transactions often vary slightly in amount month to month, and old data may not have stable `merchant_name` enrichment. The engine therefore needs a conservative match key, value tolerance, and category policy that improves recurring recognition without creating hidden user-specific rules in source code.

## Decision

**Replicate human-reviewed category alongside `description` and `purpose` for recurring transactions matched by merchant or normalized raw description, using amount tolerance as donor scoring and never overwriting trusted human target values.**

Replication remains direct data propagation, not automatic rule creation. Donors with `description` or `purpose` still qualify because those fields are human-authored. Category-only donors qualify only when their `category_source` is human (`manual` or `enriched:user`). Target category is copied only when it is missing or weak (`unclassified`, `fallback`, `pluggy`).

## Options considered

- **Direct replication with weak-target guard** (chosen): Copies human category, description, and purpose from matching history. It fixes recurring month-to-month cases without adding persistent rules or hardcoded private patterns.
- **Automatic rule creation from every human review**: Would make future syncs recognize recurring charges, but it risks broad merchant-level rules where the user intended a one-off correction.
- **Description/purpose-only replication plus more LLM prompting**: Keeps category untouched, but does not solve the observed bug where next-month recurring expenses remain uncategorized or weakly categorized.

## Consequences

- Recurring transactions can inherit category, description, and purpose from prior human-reviewed months, including when the amount varies within tolerance.
- `FinanceStore::find_anatomy_donors` now returns human category donors as well as anatomy donors, and both SQLite and BigQuery must keep the same donor/candidate semantics.
- The replication audit event records whether description, purpose, and category were replicated, preserving reviewability.
- Re-evaluate this decision if users want explicit persistent rules instead of direct replication, or if category-only donor matching causes false positives for broad merchants.
