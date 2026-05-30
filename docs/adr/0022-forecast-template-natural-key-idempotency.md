---
type: ADR
id: "0022"
title: "Forecast template idempotency via natural keys + self-healing dedup"
status: active
date: 2026-05-30
---

## Context

[ADR-0016](0016-forecast-automation.md) made every `forecast_template` (and the
`forecast` rows it materialises) idempotent by deriving a deterministic
`template_id` from a content hash:

- installments — `installment-<sha8(account_id, base_description, total)>`
- recurring — `<kind>-<sha8(account_id, merchant_key, kind)>`

The upsert deduplicates **solely** on that derived id (`ON CONFLICT(template_id)`
in SQLite, `MERGE ON target.template_id = source.template_id` in BigQuery). The
materialised forecast ids embed the template id (`{template_id}-{n:03}` for
installments, `tpl-{template_id}-{yyyymm}` for recurring), and `forecast refresh`
re-materialises every active template on each run.

This made identity **only as stable as the hash and its inputs**. In production
(May 2026) two `forecast refresh` runs hours apart produced *different* ids for
byte-identical commitments — recovered from the audit log, the forked rows shared
account, description and total yet carried different `template_id` hashes. The
most likely trigger was a binary self-update between the two runs that changed
the hashing; a detector revision (e.g. a recomputed `total`, a normalisation
tweak) would do the same. Because dedup keyed only on the exact id:

- every template forked into a duplicate;
- every materialised forecast forked with it; and
- because `refresh` re-materialises every **active** template, both copies kept
  being re-emitted on every run — ~80 duplicate templates and ~160 duplicate
  forecasts accumulated, roughly doubling several recurring expense lines and
  distorting the cash-flow chart.

The existing `*_is_stable_across_builds` frozen-hash tests prevent *accidental*
drift, but they cannot protect against a *deliberate* future change to the id
scheme, nor against detector-input drift — and there was no safety net once a
duplicate existed.

## Decision

Keep the derived `template_id` as the surrogate primary key, but stop treating it
as the unit of identity. Introduce a **natural key** — a canonical,
hash-independent identity — and use it both to prevent and to heal duplicates.

1. **Natural key.** A delimited tuple of stable fields:
   - installments: `kind | account_id | base_description | total`
   - recurring: `kind | account_id | merchant_pattern | category_id`

   Computed identically from a stored `ForecastTemplateRecord`, from an
   `InstallmentChain`, and from a `RecurringCandidate` (agreement asserted in
   tests). It includes the installment `total` so two genuinely distinct plans at
   the same merchant stay separate, but it does **not** depend on the id hash, so
   it survives any future change to the id-derivation scheme.

2. **Prevention — id reuse.** Before upserting, `refresh` maps each known natural
   key to its canonical `template_id` and reuses it when re-deriving. A drifted
   hash therefore *updates the canonical row in place* instead of inserting a
   fork. The recurring detector also skips proposing a candidate whose natural
   key already exists (replacing the previous id-based skip), preserving the
   "don't re-propose a dismissed merchant" guarantee even across id drift.

3. **Self-healing — dedup pass.** `refresh` starts by collapsing any existing
   duplicates: per natural key the **oldest** template wins (ties broken by id);
   the losers are demoted to `descartado` and their non-realised forecasts set to
   `inativo`, so they stop being projected and re-materialised. Already-`descartado`
   rows never win and are never re-demoted, making the pass idempotent — a no-op
   once the store is clean. Realised forecasts (`realizado`/`effected`) are left
   untouched so a prediction's link to a real transaction is never lost. Every
   demotion/deactivation emits an `AuditEvent` ([ADR-0005](0005-append-only-audit-events.md)).

The id-derivation hashes and their frozen tests are unchanged: this ADR is
additive and does not re-key existing rows.

## Consequences

- `forecast refresh` is now robust to id-scheme changes and detector drift: at
  most one active template can exist per identity, and duplicates that slip in
  are healed on the next run. `RefreshReport` gains `templates_deduped`.
- Healing is conservative (soft demotion via existing statuses, not hard delete),
  so it is reversible and fully audited; no schema migration or `FinanceStore`
  trait change was required.
- The one-time production duplicates from the original incident were cleaned
  separately via direct DML; this code prevents recurrence.
- Future detector layers must derive identity through the natural key rather than
  inventing a new id scheme. If the natural-key definition itself ever changes,
  it must be versioned and reconciled the same way the id is here.

## Alternatives considered

- **Re-key on a semantic column with a DB-level unique constraint.** Cleaner in
  theory but requires a migration in both backends and a `MERGE`/`ON CONFLICT`
  key change — higher blast radius on live data for no extra safety over the
  application-level reconciliation chosen here.
- **Hard-delete duplicates.** Rejected: irreversible on real financial data and
  needs new delete methods on the storage trait. Soft demotion to existing
  terminal statuses achieves the same projection/materialisation effect.
- **Rely only on the frozen-hash tests.** They guard accidental drift but not a
  deliberate id-scheme change or detector-input drift, and offer no remediation
  for duplicates already persisted.
