---
type: ADR
id: "0004"
title: "Pluggy as the exclusive bank aggregator (Brazil-first)"
status: active
date: 2026-01-08
---

## Context

Finance OS targets Brazilian users first: that's where the author's accounts are, where open finance is most mature, and where the existing-tool gap is largest. Bank-data ingestion is the most operationally sensitive part of the system: a slow, fragile, or generic aggregator forces a worse user experience everywhere downstream.

Candidate aggregators:

- **Pluggy** — Brazilian open-finance aggregator with broad bank coverage, structured installment metadata, HMAC client credentials, JSON-decimal amounts. Pricing aligns with personal use.
- **Belvo** — Latin-America aggregator. Strong on Mexico/Colombia, weaker on Brazilian credit-card nuances.
- **OFX import** — universal but manual; loses real-time, requires per-bank export configuration, no structured installment data.
- **Per-bank scrapers** — fragile, ToS-risky, high maintenance.

A generic "bank-agnostic" abstraction layered over multiple aggregators would be premature: every aggregator has different installment semantics, different category hints, different pagination shapes, and we have no working knowledge of two of them.

## Decision

**Pluggy is the only bank aggregator integrated into Finance OS, accessed through `crates/finance-core/src/pluggy.rs` directly — no generic aggregator trait.** Other ingestion paths exist but are clearly secondary: `admin import-legacy` for one-time CSV imports, `tx upsert-manual` for hand entries.

When (if) a second aggregator becomes necessary, the abstraction emerges from refactoring two concrete clients, not from speculating one.

## Options considered

- **Pluggy directly, no abstraction** (chosen): minimal complexity; Pluggy-specific features (installment markers, structured category hints) surface naturally into the models.
- **Generic `Aggregator` trait with one impl**: premature abstraction. Adds a layer without paying for it, hides Pluggy-specific structure that's actually useful in the rest of the pipeline.
- **Pluggy + Belvo from day one**: not justified — we have no Belvo user. Building two implementations in parallel for a hypothetical user is the wrong cost shape.

## Consequences

- **Easier**: Pluggy-specific richness (installment markers, structured categories) is first-class in models and enrichment; no generic-layer tax on every feature.
- **Harder**: adding a second aggregator later requires a refactor pass, not just a new module. We accept this — the refactor will be cheaper than carrying a premature abstraction for years.
- **Invariants for the codebase**:
  - Pluggy-specific fields can appear on `TransactionRecord.metadata` and in enrichment paths without indirection.
  - Personal classification still does **not** live in `pluggy.rs` — see [ADR-0008](0008-privacy-no-personal-data-in-shared-source.md).
- **Re-evaluation triggers**: a Finance OS user in a country Pluggy doesn't cover; sustained Pluggy API instability; or a second aggregator integration becoming a real need (not hypothetical).
