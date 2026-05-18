# Architecture Decision Records

This folder contains Architecture Decision Records (ADRs) for Finance OS — the record of *why* each structural choice was made, so future contributors don't have to re-derive it from `git log`.

## Format

Each ADR is a markdown file with YAML frontmatter. Use the template below (also available at [`0000-template.md`](0000-template.md)).

```markdown
---
type: ADR
id: "NNNN"
title: "Short decision title"
status: proposed        # proposed | active | superseded | retired
date: YYYY-MM-DD
superseded_by: "NNNN"  # only if status: superseded
---

## Context
What situation led to this decision? What forces and constraints are at play?

## Decision
**What was decided.** State it clearly in one or two sentences — bold so it stands out.

## Options considered
- **Option A** (chosen): brief description — pros / cons
- **Option B**: brief description — pros / cons
- **Option C**: brief description — pros / cons

## Consequences
What becomes easier or harder as a result?
What are the positive and negative ramifications?
What would trigger re-evaluation of this decision?
```

### Status lifecycle

```
proposed → active → superseded
                 ↘ retired      (decision no longer relevant, not replaced)
```

## Rules

- One decision per file.
- File name: `NNNN-short-title.md`, monotonic numbering.
- Once `active`, **never edit** — supersede with a new ADR instead.
- When superseded: update `status: superseded`, add `superseded_by: "NNNN"`, and link from the new ADR's Context.
- [ARCHITECTURE.md](../ARCHITECTURE.md) reflects the current state (active decisions only). When an ADR supersedes another, update ARCHITECTURE.md in the same commit.

## Index

| ID | Title | Status |
|----|-------|--------|
| [0001](0001-single-binary-rust-cli.md) | Single-binary Rust CLI as the product surface | active |
| [0002](0002-financestore-trait-dual-backend.md) | `FinanceStore` trait with SQLite + BigQuery backends | active |
| [0003](0003-rust-decimal-end-to-end.md) | `rust_decimal::Decimal` end-to-end for monetary amounts | active |
| [0004](0004-pluggy-as-exclusive-aggregator.md) | Pluggy as the exclusive bank aggregator (Brazil-first) | active |
| [0005](0005-append-only-audit-events.md) | Append-only `AuditEvent` log on every write | active |
| [0006](0006-mirrored-idempotent-migrations.md) | Mirrored idempotent migrations across both backends | active |
| [0007](0007-atomic-self-update.md) | Atomic self-update with SHA-256 verification | active |
| [0008](0008-privacy-no-personal-data-in-shared-source.md) | Privacy: no personal data in shared source | active |
| [0009](0009-proactive-pulse-and-closing-plan.md) | Pulse as a proactive closing-plan, not a retrospective transaction list | active |
| [0010](0010-card-billing-cycle.md) | `v_card_summary` groups by billing cycle, not calendar month | active |
| [0011](0011-canonical-payment-status.md) | Canonical `payment_status` vocabulary (`posted`/`pending`/`installment`) | active |

---

*The structure of these docs — the AGENTS.md workflow, the `docs/` layout, and the ADR format — is inspired by [tolaria](https://github.com/refactoringhq/tolaria).*
