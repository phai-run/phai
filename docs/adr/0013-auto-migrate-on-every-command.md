---
type: ADR
id: "0013"
title: "Auto-apply pending migrations on every command invocation"
status: active
date: 2026-05-18
---

## Context

[ADR-0006](0006-mirrored-idempotent-migrations.md) establishes *how*
migrations are structured (mirrored, idempotent, append-only, embedded
into the binary). [ADR-0007](0007-atomic-self-update.md) establishes
that the CLI updates itself in place from GitHub Releases without user
intervention.

Together, these decisions create an obligation that ADR-0006 only
gestures at ("Survive automatic self-updates"): **after a self-update,
the user expects the next command to just work** — no `admin migrate`
step, no error about a missing view, no schema-drift surprise. The
user does not know (and should not need to know) that the binary they
just ran is N versions newer than the database it points at.

In practice this is implemented by ~30 command entry points in
`crates/finance-cli/src/main.rs` calling `run_migrations` right after
`open_store`. The pattern is correct but **implicit**: there is no
helper, no lint, no test that enforces it. An audit while writing this
ADR found 10 entry points that open the store *without* running
migrations — a latent bug that only bites on the first command after a
self-update introduces a new migration.

## Decision

**Every CLI command that opens the store must apply pending migrations
before the first read or write.** Specifically:

- The canonical sequence at every command entry point is:
  ```rust
  let (_, config) = load_config().await?;
  let store = open_store(&config).await?;
  run_migrations(store.as_ref(), &config).await?;
  ```
- `run_migrations` is idempotent (it consults
  `applied_migrations` and skips what is already applied), so the cost
  on a fully-migrated database is one cheap SELECT per backend.
- `admin migrate` remains in the CLI surface as a **diagnostic /
  manual-recovery tool**, not as a required pre-step. It is useful for
  scripted provisioning, for testing migrations in isolation, and for
  human troubleshooting — never as a deployment gate.
- New command functions that take a `&FinanceStore` reference inherit
  this invariant from their caller; helpers that operate on an
  already-open store do not re-migrate.

This refines, not supersedes, ADR-0006 — ADR-0006 covers the structure
and parity of migrations; this ADR covers the trigger.

## Options considered

- **Auto-migrate on every command** (chosen): zero-admin UX matching
  ADR-0007's self-update posture; the only sustained cost is one
  SELECT per command on the migrations bookkeeping table.
- **Migrate only on `admin migrate`**: closer to traditional server
  ops, but breaks ADR-0007's "binary updates itself, user does
  nothing" invariant. Every self-update would land a binary that
  could error on its first invocation. Rejected.
- **Migrate on first command after version bump (detect via stored
  version)**: solves the cost concern but adds a state machine and a
  failure mode (corrupted version marker leaves the DB unmigrated).
  The cost it avoids is negligible.
- **Implicit migrate inside `open_store`**: hides a side-effect with
  observable I/O in a constructor; makes the `FinanceStore` trait
  harder to test and harder to reason about (e.g. read-only smoke
  tests). Rejected.

## Consequences

- **Easier**: self-updates ship migrations transparently; users can't
  end up on a broken DB by skipping a step; testing CLI commands
  doesn't need a separate "did you migrate?" preamble.
- **Harder**: every new command entry point must remember the pattern.
  A linter or a `with_store` helper that bundles `open_store +
  run_migrations` could eliminate the foot-gun; until that lands, the
  pattern is enforced by code review.
- **Invariants for the codebase**:
  - Any function in `crates/finance-cli/src/main.rs` named `^report_`,
    `^tx_`, `^rule_`, `^admin_`, `^sync_`, `^report_*`, etc. that
    contains `open_store(&config)` must contain `run_migrations(...)`
    on the next non-blank line — and the call must use the same
    `store.as_ref()` handle.
  - `admin migrate` keeps the existing `--dry-run` and explicit
    behavior; it is never a required pre-step in user-facing docs.
  - Helpers that take an already-open store do **not** call
    `run_migrations` themselves — the outer command owns the
    invariant.
- **Re-evaluation triggers**:
  - A backend where applying-no-op migrations becomes expensive (none
    seen — both SQLite and BigQuery treat the bookkeeping check as
    constant-time).
  - A new command shape where it is genuinely desirable to read the
    pre-migration state (e.g. forensics on a broken upgrade); add an
    explicit `--no-migrate` flag rather than weakening the default.

This ADR was the trigger for fixing 10 entry points in
`crates/finance-cli/src/main.rs` that previously opened the store
without running migrations (`report_review`, `tx_enrich`,
`tx_categorize`, `tx_set_context`, `tx_list_context`, `tx_find`,
`tx_pending`, `tx_set_context_by_desc`, `rule_list`, `rule_inspect`).
