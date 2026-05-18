---
type: ADR
id: "0002"
title: "`FinanceStore` trait with SQLite + BigQuery backends"
status: active
date: 2025-12-20
---

## Context

Finance OS has two distinct user shapes:

1. **One person, one machine.** Wants zero setup. SQLite on disk is ideal — no network, no credentials, no cost.
2. **One person, multiple devices, or a Sheets-backed budget.** Needs a shared dataset reachable from anywhere. BigQuery is ideal — joinable with Sheets, free tier covers personal volume, no operational overhead.

The CLI should not know which backend is in use. Reports, audit events, idempotency, and migrations must behave identically.

## Decision

**A single `async` trait, `FinanceStore`, in `crates/finance-core/src/storage/mod.rs`, abstracts all persistence.** Two implementations: `storage::local` (SQLite via `rusqlite`) and `storage::bigquery` (REST + service-account JWT). `open_store(config)` returns `Box<dyn FinanceStore>` based on `AppConfig.backend`. Migrations exist in parallel directories (`schema/sqlite/`, `schema/bigquery/`) with shared numeric prefixes and equivalent semantics.

The trait is intentionally wide (~50 methods). Report queries (`daily_pulse`, `card_summary`, `cashflow`, …) are first-class methods on the trait because they encode business logic that must be parity-tested across backends.

## Options considered

- **Single trait, two impls** (chosen): forces backend parity by construction. Adding a method touches both files in the same PR — review is "do these two implementations agree?", which is a tractable question.
- **A query builder DSL** (`sea-orm`, `diesel`, etc.): saves some duplication, but the cost is an extra abstraction between the developer and SQL when SQL is the natural language of reports. Loses the ability to hand-tune BigQuery's MERGE semantics or SQLite's window functions.
- **SQLite only, BigQuery via export script**: cheaper short-term, but the multi-device user case is real today (Sheets overrides, agent on a different machine).
- **BigQuery only**: forces a GCP project on every user. Rejected — the entry-level experience must be `curl | bash && finance-cli sync pluggy`.

## Consequences

- **Easier**: testing (E2E against SQLite covers semantic correctness; BigQuery parity is a code-review checklist), local dev (no GCP needed), and reasoning (the trait is the contract).
- **Harder**: every new feature has two implementations to write. We accept this cost because it forces parity to be visible at PR time.
- **Invariants for the codebase**:
  - New `FinanceStore` methods land with both impls in the same PR.
  - New migrations land in both `schema/sqlite/` and `schema/bigquery/`, same numeric prefix, equivalent semantics.
  - When BigQuery and SQLite genuinely diverge (e.g. SQLite lacks `MERGE`), the divergence is documented in the migration file header.
- **Re-evaluation triggers**: a third backend with materially different semantics; or sustained pain from feature-by-feature parity that a query builder could eliminate.
