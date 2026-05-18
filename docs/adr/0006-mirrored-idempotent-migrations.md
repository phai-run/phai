---
type: ADR
id: "0006"
title: "Mirrored idempotent migrations across both backends"
status: active
date: 2026-01-18
---

## Context

Finance OS runs against two backends (see [ADR-0002](0002-financestore-trait-dual-backend.md)) and ships as a single binary that the user re-runs against the same database, possibly across many version upgrades. The migration story must:

1. **Be idempotent.** `admin migrate` on a fully-migrated database is a no-op.
2. **Keep both backends in semantic parity.** A user moving from SQLite to BigQuery (or vice versa) sees the same domain shape.
3. **Survive automatic self-updates.** A user who upgrades five times in a week without thinking should have a healthy database without manual intervention.

SQLite and BigQuery have different SQL dialects: BigQuery uses `CREATE OR REPLACE VIEW`, `MERGE`, and `NUMERIC`; SQLite uses `CREATE VIEW IF NOT EXISTS`, `INSERT … ON CONFLICT`, and `TEXT` decimals. Migrations cannot share files — but they must share *semantics*.

## Decision

**Migrations live in parallel directories — `schema/sqlite/` and `schema/bigquery/` — with shared monotonic numeric prefixes (`001_initial.sql`, `002_add_forecast.sql`, …) and equivalent semantics.** They are embedded into the binary at compile time via `include_str!` and registered in `crates/finance-core/src/migrations.rs`. Every migration is **idempotent by construction**: `CREATE TABLE IF NOT EXISTS`, `CREATE OR REPLACE VIEW`, guarded `ALTER` / backfills.

A migration that has been released to users is **never edited** — corrections land as new numbered migrations.

When backends genuinely cannot match (e.g. SQLite lacks `MERGE`), the divergence is documented in a header comment in the migration file, and the `FinanceStore` impl handles it at the call site.

## Options considered

- **Mirrored directories, same prefix, semantic parity** (chosen): each migration has an obvious sibling; PR review is "do these two files agree?".
- **Shared `.sql` files with dialect templating**: hides divergence under a fragile substitution layer; debugging migrations becomes archaeology.
- **Migration tool (`sqlx-migrate`, `refinery`)**: adds a runtime dep and a CLI step; doesn't solve the cross-backend parity problem; conflicts with embedding migrations into the binary.
- **Drop migrations, regenerate from a schema description**: doesn't fit append-only audit semantics or BigQuery views that depend on row content (e.g. internal-transfer reclassification).

## Consequences

- **Easier**: idempotent upgrades, replay-safe self-updates, transparent diffs in PRs that touch both files, parity by visual inspection.
- **Harder**: every schema change is two files; the discipline to keep semantics matched is on the contributor.
- **Invariants for the codebase**:
  - New migrations land in both `schema/sqlite/` and `schema/bigquery/` in the same PR, with the same numeric prefix.
  - Idempotency is mandatory (`IF NOT EXISTS`, `OR REPLACE`, conditional backfills). A migration that crashes on a fully-migrated database is a bug.
  - Released migrations are immutable. Mistakes land as new migrations.
  - The shared numeric prefix is the parity anchor — never reuse a number across backends with different semantics.
- **Re-evaluation triggers**: a class of schema change where idempotency is fundamentally impossible (none seen so far); or sustained backend divergence that makes the mirrored model dishonest.
