---
type: ADR
id: "0008"
title: "Privacy: no personal data in shared source"
status: active
date: 2026-02-14
---

## Context

phai is open source and runs on the author's real money. The two facts are in tension. The natural shortcut — hardcode a counterparty regex, ship a fixture mirroring a real statement, embed an account label in a migration — saves minutes today and is irrecoverable forever:

- Once it lands in a public commit, `git history` is permanent.
- AI agents and code-search tools scrape public repos.
- The information leaks across the personal/professional boundary in ways that are not visible until they are.

The category of "doesn't look sensitive" is exactly where this kind of leak happens. A merchant name as a heuristic, a CNPJ as a hint, a CSV fixture with one too many fields — each is harmless in isolation and unrecoverable in aggregate.

This concern outranks DRY, outranks convenience, and outranks "we'll clean it up later."

## Decision

**No personal data may appear in shared source files of this repository.** Specifically:

- **No hardcoded counterparty names**, account labels, statement fingerprints, or personally-identifying patterns in Rust files.
- **No personal patterns in migrations.** Migrations create generic infrastructure only — tables, views, indexes. They do not embed user-specific classification logic, account numbers, or institution-specific text.
- **No real data in fixtures or tests.** Every committed fixture is synthetic — plausible-but-fake. Real-data bug repros are reproduced locally and translated into synthetic regression tests.
- **Classification belongs in the runtime.** User-specific patterns live in the `rules` table on the user's machine, or in private configuration outside the repository.
- **Bug fixes vs. data fixes.** If a real-user bug requires a data correction, implement the generic engine support in shared code, then apply the private rule/data correction outside this repository.

This is enforced by code review (and reinforced by the privacy section of [AGENTS.md §1](../../AGENTS.md#1-privacy--data-hygiene-hard-rules), which agents read before writing any code).

## Options considered

- **Hard rule, zero exceptions** (chosen): the only policy that doesn't decay. A single "exception just this once" sets a precedent that future contributors interpret as latitude.
- **Allow personal patterns behind a feature flag**: still ships in the binary, still in the repo, still in `git history`. Solves nothing.
- **Private fork with personal data**: defeats the open-source posture and splits the codebase across surfaces that drift apart.
- **Pre-commit hook scanning for PII**: useful as a backstop, not a substitute for the rule. False negatives are inevitable; the rule has to be the primary defense.

## Consequences

- **Easier**: the repo is publishable, forkable, and analyzable by anyone — including AI agents — without leaking the author's finance graph.
- **Harder**: bug repros require a translation step from real data to synthetic. Classification fixes can't be one-line patches; they need engine support + a private rule.
- **Invariants for the codebase**:
  - Counterparty / account label / statement text in a `*.rs` or `schema/**/*.sql` file is a code-review blocker.
  - Tests that depend on a real user's data are restructured to use synthetic fixtures before merge.
  - The `enrichment/heuristics.rs` module contains only generic patterns (currency markers, common merchant suffixes, structured installment text). Anything specific to one bank or one user goes to the `rules` table.
  - `AGENTS.md` reinforces this rule as the first hard rule, so agents writing code in this repo see it before producing a diff.
- **Re-evaluation triggers**: none anticipated. This rule is structural, not pragmatic.
