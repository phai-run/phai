---
type: ADR
id: "0020"
title: "Single workspace version: one number for the whole product"
status: active
date: 2026-05-28
---

## Context

Since [#65](https://github.com/feliperun/finance-os/pull/65), release-please tracked
`crates/finance-cli` and `crates/finance-core` as two independent packages, each with
its own version line (CLI on `3.x`, core on `1.x`) and its own release tag
(`vX.Y.Z` vs `finance-core-vX.Y.Z`).

In release-please manifest mode each package only counts commits that touch files under
its own path. A core-only commit therefore bumped `finance-core` but never the CLI, and
the CLI's path-dependency pin (`finance-core = { ..., version = "1.1.1" }`) drifted
behind the released core version. This produced a recurring class of bugs patched across
#81, #87, #89–#92:

- the `version` pin on the internal path dependency falling out of sync;
- two separate release PRs (CLI vs core) for what is a single shipped artifact;
- `install.sh` and the self-updater needing bespoke logic to skip `finance-core-*` tags,
  because GitHub could flag a core release as `latest`.

`finance-core` is not published to crates.io — the only release artifact is the `fin`
binary tarball. The two-version split bought nothing and cost a steady stream of remedies.

## Decision

**Collapse the workspace to a single version, sourced from `workspace.package.version`,
owned by release-please as one root package.** Both crates inherit `version.workspace = true`;
the internal path dependency carries no `version` requirement; there is one release line
(`vX.Y.Z`) and one release PR. `finance-core` no longer has an independent version — at the
collapse it adopts the CLI's `3.x` line.

## Options considered

- **Single workspace version** (chosen): one root release-please package (path `.`) so
  every commit anywhere bumps the one number; `version.workspace = true` in both crates;
  no `version` on the internal path dep. Pros: eliminates the drift bug class entirely; one
  PR, one tag, one number. Cons: one-time discontinuity (core `1.1.2` → `3.2.2`); historical
  `finance-core-v*` tags remain in the repo (harmless, never `latest` again).
- **Keep two packages, sync via `extra-files`**: retain core's `1.x` line but mirror the CLI
  version into core's manifest. Pros: preserves core's history line. Cons: still two release
  PRs and tags; `extra-files` mirroring is fragile and does not fix commit-attribution.
- **Status quo (linked-versions plugin)**: what #87 attempted. Cons: `merge: false` still
  produced separate PRs and could not unify two different version lines — the bug persisted.

## Consequences

- Easier: any commit in the workspace cuts a release for the whole product; the path-dep pin
  can never drift; `install.sh` and the self-updater use `/releases/latest` directly, dropping
  the `finance-core-*` filtering hack and its tests.
- Harder / watch for: a one-time version discontinuity for `finance-core`; release-please
  output keys in `.github/workflows/release-please.yml` are now path-prefixed by `.`
  (`.--release_created`, `.--tag_name`).
- Re-evaluation trigger: if `finance-core` is ever published to crates.io independently, the
  single-version model must be revisited (a published crate needs its own semver contract).
