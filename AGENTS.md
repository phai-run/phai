# AGENTS.md — phai

> Quick links: [Architecture](docs/ARCHITECTURE.md) · [Abstractions](docs/ABSTRACTIONS.md) · [Vision](docs/VISION.md) · [Getting Started](docs/GETTING-STARTED.md) · [ADRs](docs/adr/README.md) · [Reporting UX rules](REPORTING_UX.md)
>
> *Playbook structure inspired by [tolaria](https://github.com/refactoringhq/tolaria).*

Critical guardrails for this repository — read before writing code, opening a PR, or producing user-facing finance output.

---

## 1. Privacy & data hygiene (hard rules)

These come first because violations are the hardest to undo.

- **No personal data in shared source.** Never hardcode personal counterparties, account labels, statement fingerprints, or production-derived values into Rust files, SQL migrations, fixtures, tests, or docs.
- **Classification belongs in the runtime.** User-specific patterns live in the `rules` table or in private configuration — not in `enrichment/heuristics.rs`, not in migrations, not in tests.
- **Migrations are generic.** Shared migrations under `schema/sqlite/` and `schema/bigquery/` may create infrastructure (tables, views, indexes). They must not embed personal names, account numbers, or institution-specific text.
- **Fixtures are synthetic.** All committed fixtures and test data must be plausible-but-fake. If a real bug needs a real-data repro, reproduce it locally and translate the failure into a synthetic test.
- **Bug fixes vs. data fixes.** If a real-user bug requires a data correction, implement the generic engine support in shared code, then apply the private rule or data fix outside this repository.
- **Reporting UX rules** live in [REPORTING_UX.md](REPORTING_UX.md). Read that before formatting any user-facing finance output.

---

## 2. Task workflow

### 2a. Pick up a task

- Read the issue or task fully, including all comments.
- Check `docs/adr/` for relevant architecture decisions before any structural choice.
- For bug fixes: reproduce first, then write a failing regression test, then fix.

### 2b. Implement

- Branch from `main` and open a PR — small, focused, conventional-commit titles.
- Commit every 20–30 min: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`.
- **Never `--no-verify`.** If a hook blocks, fix the underlying issue.
- Keep changes scoped to the task — no opportunistic refactors in feature PRs.

### 2c. Before declaring done

Run the full local check suite below, then verify the release-readiness checklist further down.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

---

## 3. Development process

### Commits & PRs

- **Conventional Commits are required** — Release Please derives `CHANGELOG.md` and the next version from them.
- One logical change per commit. One bounded scope per PR.
- Use `feat!:` or a `BREAKING CHANGE:` footer for breaking changes.
- PRs targeting `main` go through GitHub Actions CI (fmt, clippy, test, audit, deny). A PR is not ready to merge until CI is green.

### TDD (mandatory for behavior changes)

Red → Green → Refactor → Commit. One cycle per commit.

- Bug fixes: write a failing regression test first, then fix.
- New behavior: add targeted coverage close to the changed code. E2E tests prefer the SQLite backend over mocks.
- Exception: pure docs, formatting, or release-only changes.

**Test quality (Kent Beck's Desiderata):** Isolated · Deterministic · Fast · Behavioral · Structure-insensitive · Specific · Predictive. Fix flaky tests first.

### Check suite (runs on every push / PR)

```bash
cargo fmt --all -- --check       # formatting
cargo clippy --all-targets --all-features -- -D warnings   # lints as errors
cargo test --workspace            # unit + integration + e2e (SQLite backend)
cargo audit                       # security advisories (CI: rustsec/audit-check)
cargo deny check licenses         # license policy
```

CI mirrors this and runs on every PR (`.github/workflows/ci.yml`).

### Code conventions (enforced by clippy + review)

- `anyhow::Result` with `.context()` for error propagation in CLI/storage seams. No `.unwrap()` in production code paths — only in tests and clearly proven invariants.
- **All monetary amounts use `rust_decimal::Decimal`. Never `f64`, never `f32`.** Parse from strings, serialize via the `serde` feature.
- SQL parameters are **bound**, never string-interpolated. The only exception is table identifiers validated against the allowlist in `storage::mod::validate_table_name`.
- Every write operation emits an `AuditEvent` (see [docs/ABSTRACTIONS.md](docs/ABSTRACTIONS.md#audit-events)).
- New migrations are **idempotent** and land in **both** `schema/sqlite/` and `schema/bigquery/` in the same commit. Numbering is monotonic and shared.
- No `#[allow(...)]` to silence clippy. Fix the code.

### Migrations

- Idempotent by construction: `CREATE TABLE IF NOT EXISTS`, `CREATE OR REPLACE VIEW`, guarded backfills.
- The same numeric prefix exists in both backends; semantics must match. If a feature is backend-specific (rare), explain why in the migration header comment.
- Embedded into the binary at compile time via `include_str!`. After adding a file, also register it in `crates/phai-core/src/migrations.rs`.

### Code health gate — Sentrux (mandatory)

[Sentrux](https://github.com/sentrux/sentrux) is the architectural-quality sensor for this repo. It runs on every meaningful change, gives the agent a score to optimize against, and blocks regressions.

```bash
sentrux check .           # CI-friendly; exits 0 if rules pass, 1 if not
sentrux gate --save .     # snapshot the current baseline (before agent edits)
sentrux gate .            # compare current vs baseline; fails on degradation
```

Workflow:

- **Before starting a task on existing files**, run `sentrux gate --save .` to capture the baseline.
- **Before committing**, run `sentrux gate .`. If it reports degradation in any file you touched, refactor — do not commit.
- **Boy Scout Rule**: every file you touch must leave with an equal or better score. If a file is already at the top of its scale, keep it there.
- **New files** must pass `sentrux check .` cleanly — no findings, no warnings.
- **Never silence a rule** to make the gate pass. Fix the code. The gate is a ratchet — only direction is up.

CI runs `sentrux check .` and (on PRs) `sentrux gate .` against the merge base. A failing gate blocks the PR.

### Coverage & dependency gates

- Coverage is a release gate, not a vanity metric. For bug fixes, add a regression test when practical; for new behavior, add coverage close to the changed code.
- `cargo audit` and `cargo deny check licenses` block on new advisories or disallowed licenses. Resolve the finding — never silence it.

### ADRs & docs

ADRs live in `docs/adr/`. Create one in the same commit as the code that implements the decision. Never edit an active ADR — supersede it with a new one. Use `/create-adr` (see `.claude/commands/create-adr.md`).

**When to create an ADR**

- A new dependency that changes the surface area (new backend, new external service, new aggregator).
- A storage strategy or schema convention change.
- A new platform target or distribution channel.
- A core abstraction (new trait, new domain model, change to `FinanceStore`).
- A cross-cutting pattern that future contributors must follow.

**Not for**: bug fixes, refactors that preserve behavior, dependency version bumps, formatting.

After any change that affects: the storage trait, a new command, a data model, a new integration, or a privacy boundary — update `docs/ARCHITECTURE.md` and/or `docs/ABSTRACTIONS.md` in the same commit.

---

## 4. Release-readiness checklist

Before merging or releasing, confirm:

- [ ] `cargo fmt`, `cargo clippy -D warnings`, `cargo test --workspace` all pass locally.
- [ ] `sentrux check .` passes; `sentrux gate .` shows no degradation on touched files.
- [ ] CI is green on the PR (`.github/workflows/ci.yml`).
- [ ] If a write path changed: an `AuditEvent` is emitted and the schema accepts it.
- [ ] If a migration was added: it exists in both `schema/sqlite/` and `schema/bigquery/`, both are idempotent, and `migrations.rs` includes it.
- [ ] If a public CLI flag, subcommand, or report changed: `README.md` is updated.
- [ ] If a structural decision was made: an ADR is in `docs/adr/` and the index in `docs/adr/README.md` is updated.
- [ ] No personal counterparties, account labels, or statement fingerprints in shared code (`grep` your diff before pushing).
- [ ] Conventional Commit title — Release Please parses it.

---

## 5. Reporting UX (when answering finance questions)

The reporting voice and disambiguation rules are documented in [REPORTING_UX.md](REPORTING_UX.md). Highlights:

- `phai` is the single source of truth for operational output.
- Prefer standard reports (`--notify-summary` for text, `--json-summary` / `--raw` for structured) over ad-hoc agent formatting.
- Never invent categories — category assignment must come from rules and effective overrides.
- Card-bill questions: disambiguate "open" vs "closed" before answering (see REPORTING_UX.md §Interaction Consistency).

---

## 6. Reference

### Layout

```
crates/
  phai-core/   Domain models, storage trait, Pluggy client, rules, splits, enrichment
  phai-cli/    Binary, report formatters, auto-update, command surface
    web/       LiveStore + React web app for `phai serve` (built to web/dist, embedded)
schema/
  sqlite/         SQLite migrations (local backend)
  bigquery/       BigQuery migrations (production backend)
integrations/
  openclaw/       AI assistant skill + wrapper
docs/
  adr/            Architecture decision records
  *.md            Architecture, abstractions, vision, getting started
```

### Useful commands

```bash
cargo run -p phai-cli -- --help            # explore CLI
cargo test -p phai-cli                     # E2E tests against SQLite
cargo test -p phai-core                    # core unit tests
cargo deny check licenses                     # license policy
cargo audit                                   # vulnerability scan
```

### Web app (`phai serve`)

The interactive UI is a LiveStore + React (Vite) app under `crates/phai-cli/web/`, embedded
into the binary via `include_dir!("web/dist")`. It is **client-only** (no LiveStore sync
backend); writes flush to the Rust bridge (`/api/*`), which is the BigQuery/SQLite system of
record. The JS build runs only in CI / locally — never on the end user's machine (ADR-0001).

```bash
cd crates/phai-cli/web
pnpm install            # one-time
pnpm typecheck          # tsc
pnpm build              # → web/dist (generated; embedded at compile time)
```

`web/dist` is a **generated artifact and is NOT committed** (it would pollute the source-quality
gate and bloat the repo). `crates/phai-cli/build.rs` guarantees it exists at compile time: CI and
the release workflow run `pnpm build` first; a plain `cargo build` with no web build falls back to
a placeholder page, so `cargo build` stays pure-Rust (no Node on the user's machine — ADR-0001).
After changing `web/src`, run `pnpm build` locally to see it in `phai serve`.

### Versioning & release

Release Please reads Conventional Commits on `main` and opens a release PR with the next version + CHANGELOG. Merging that PR cuts a GitHub Release; CI publishes platform tarballs. The CLI auto-updates from these releases — see [ADR-0007](docs/adr/0007-atomic-self-update.md).
