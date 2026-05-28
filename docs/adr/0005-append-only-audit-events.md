---
type: ADR
id: "0005"
title: "Append-only `AuditEvent` log on every write"
status: active
date: 2026-01-12
---

## Context

A personal-finance database is consulted daily for years. When a report disagrees with intuition ("I don't remember paying that"), the user needs to answer two questions:

1. What is the state right now?
2. How did it get this way?

Without an audit log, the second question is unanswerable — you correct the row and the prior state is gone. Over years this corrodes trust: every "wait, what was that?" requires guesswork.

Mutable-only models are also a poor fit for AI agents writing to the database. An agent that miscategorizes 200 transactions needs to be reversible by inspection, not by recovery from backup.

## Decision

**Every write operation in phai emits an `AuditEvent` into an append-only `audit_events` table.** Events carry: a v7 UUID (chronologically sortable), the `actor_id`, an action name in `entity.verb` grammar (`tx.categorize`, `sync.pluggy`, `split.apply`, …), the affected `entity` + `entity_id`, and a JSON `payload` containing before/after snapshots where relevant.

The `FinanceStore` trait does not magically log writes. **Callers are responsible** for pairing a write with `insert_audit_events`. This is enforced by code review and by tests that assert on the audit row alongside the mutated state.

## Options considered

- **Append-only events, callers explicitly log** (chosen): callers know the semantic action ("categorize" vs "set-context" vs "split-apply"); the trait does not have to guess.
- **Automatic logging inside the trait**: would either log too generically ("upsert") losing semantic meaning, or require every method to take a synthetic `action` parameter, which is the same thing with extra ceremony.
- **No audit log, rely on `git log` of overrides Sheet**: doesn't cover writes that happen entirely inside the database (rule changes, splits, manual entries).
- **Event sourcing as the primary model (events are the only source)**: too expensive to build and maintain for personal-scale data; the append-only log alongside a current-state table gives us most of the benefit at far less cost.

## Consequences

- **Easier**: replay, audit ("when did this category change?"), debugging agent writes, reconstructing derived state from raw events, distinguishing user/agent actors.
- **Harder**: every new write op needs a paired audit-event insert; reviewers must check for it; the `audit_events` table grows monotonically (acceptable at personal scale — millions of rows are cheap).
- **Invariants for the codebase**:
  - Every public write method on a domain has a matching `AuditEvent` insertion in the same logical operation.
  - Action names follow `entity.verb`. Adding a new domain means picking a name that fits.
  - Tests on write paths assert on `audit_events` content, not just on the mutated state.
  - The `audit_events` table is never `UPDATE`d or `DELETE`d in normal operation. Schema changes are additive.
- **Re-evaluation triggers**: audit-event storage growth becoming a real cost (very unlikely at personal scale); or a need for event-sourcing-style projections that justifies a deeper change.
