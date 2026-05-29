---
type: ADR
id: "0022"
title: "On-disk identity migration from finance-os to phai (legacy fallback)"
status: active
date: 2026-05-29
---

## Context

The product was renamed `finance-os` → `phai` (binary `fin` → `phai`, crates
`finance-core`/`finance-cli` → `phai-core`/`phai-cli`), shipped as the breaking
4.0.0 release. The rename deliberately left the **persisted identity**
untouched at the time, because changing it naively would orphan existing users'
data and break their automation:

- Config dir `~/.config/finance-os` (and the macOS-native
  `~/Library/Application Support/finance-os`).
- Data dir `~/.local/share/finance-os` and the local DB file
  `finance-os.local.db`.
- Environment variables `FINANCE_OS_*` (`CONFIG_DIR`, `DATA_DIR`, `UPDATED`,
  `NO_AUTO_UPDATE`, `WHATSAPP_WEBHOOK_URL`/`_TOKEN`, `SKIP_SIG_VERIFY`,
  `BQ_SMOKE`) used by the CLI, cron jobs, and the OpenClaw wrapper.
- Transaction provenance metadata `{"origin": "finance-cli"}` written on manual
  writes.
- The OpenClaw skill deploy identity (`name: finance-os`, `skills/finance-os/`).

These are the last surfaces carrying the old name. They need to migrate without
a destructive, error-prone one-shot move of user data.

## Decision

**Adopt `phai` as the canonical on-disk identity, resolved with a uniform,
non-destructive legacy-fallback rule: prefer the new `phai` name; use the legacy
`finance-os` name only when the new one is absent.** Nothing is moved, copied,
or rewritten on disk.

Concretely:

- **Directories / DB file** (`ConfigPaths::discover`): for the config dir, data
  dir, and DB filename, resolve the `phai` path first and fall back to a
  pre-existing `finance-os` path/`finance-os.local.db` when the `phai` one does
  not exist. Fresh installs get `phai` / `phai.local.db`; existing installs keep
  using their `finance-os` files in place.
- **Environment variables**: introduce `PHAI_*` as the primary names and read
  them via a shared helper (`phai_core::compat::env_var{,_os}`) that falls back
  to the matching `FINANCE_OS_*` name. The self-update re-exec sentinel is now
  set as `PHAI_UPDATED`; readers accept either, so the 3.x→4.x upgrade boundary
  is covered. The OpenClaw wrapper exports **both** `PHAI_*` and `FINANCE_OS_*`
  to the same resolved value so a new or older binary both work.
- **Provenance metadata**: new manual writes record `{"origin": "phai-cli"}`.
  Existing rows keep `finance-cli` (correct history); no code branches on the
  value, so no migration of past rows is needed.
- **OpenClaw skill**: `name: phai`, deploy path `skills/phai/`, with
  `PHAI_RUNTIME_ROOT` and `phai`-named runtime candidates added ahead of the
  legacy ones.

## Options considered

- **Legacy fallback, no data movement** (chosen): zero risk of half-moved data,
  no privileged file operations, deterministic resolution. Cost: the dual-name
  resolution code and `FINANCE_OS_*` env aliases must live on indefinitely (or
  until a future removal ADR).
- **Auto-migrate (move/rename on first run)**: tidy end state, single name on
  disk. Rejected: a move that fails partway (permissions, full disk, symlinked
  dirs, concurrent processes) can corrupt or orphan a user's only financial
  database. The upside (cosmetic) does not justify the downside (data loss).
- **Hard cut, no fallback**: simplest code. Rejected: silently orphans every
  existing user's config and database on upgrade — unacceptable for a 4.0 whose
  whole breaking surface was supposed to be "reinstall the binary".

## Consequences

- **Easier**: existing users upgrade to 4.x with zero data steps and unchanged
  automation; new users get clean `phai` paths and `PHAI_*` env vars.
- **Harder**: the codebase carries permanent dual-name resolution and an env
  compatibility shim (`phai-core::compat`). Removing the `finance-os` fallback
  later is itself a breaking change and needs its own ADR + migration window.
- **Invariant for the codebase**: any new persisted identity (paths, filenames,
  env vars) uses the `phai` name and, if it replaces a `finance-os` predecessor,
  routes reads through the legacy-fallback rule above — never a bare rename that
  ignores existing data.
- **Re-evaluation trigger**: once telemetry/usage indicates no `finance-os`
  installs remain (or after a deprecation window), supersede this ADR to drop
  the legacy fallback and the `FINANCE_OS_*` aliases.
