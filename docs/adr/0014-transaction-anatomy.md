---
type: ADR
id: "0014"
title: "Transaction anatomy separates raw, human, merchant, purpose, and trace fields"
status: active
date: 2026-05-19
---

## Context

Bank and Pluggy descriptions are often optimized for statement processing, not for family financial analysis. A single `description` field could not safely serve rules, audit/debug, reports, human labels, merchant grouping, and subjective purchase intent. The old `context` field made this worse because it mixed human notes with classifier rationale.

## Decision

**Transactions now keep raw bank text in `raw_description`, short human labels in `description`, cleaned establishment names in `merchant_name`, subjective intent in `purpose`, and technical classifier/debug rationale in `classifier_trace`.** The legacy `context` column remains temporarily for compatibility, but new behavior should use the explicit fields.

## Options considered

- **Explicit columns for each semantic role** (chosen): makes rules, reports, enrichment, and human edits independent and queryable.
- **Keep everything in `metadata_json`**: avoids schema churn but breaks SQL rules, filters, and operational debugging.
- **Rename `context` to a broader notes field**: still competes semantically with purchase purpose and keeps technical/user text mixed.

## Consequences

- Reports can prefer human labels with `description -> merchant_name -> raw_description` fallback without losing auditability.
- Rules and technical search match `raw_description`, so user-edited labels cannot silently change classification.
- Enrichment may fill `merchant_name` and `classifier_trace`, while `purpose` stays human-authored unless explicitly provided.
- Future cleanup should remove the deprecated `context` column after downstream tools have migrated.
