---
type: ADR
id: "0007"
title: "Atomic self-update with SHA-256 verification"
status: superseded
superseded_by: "0017"
date: 2026-02-02
---

## Context

phai ships as a single binary updated frequently via Release Please (see [ADR-0001](0001-single-binary-rust-cli.md)). For the user the upgrade story must be:

- Zero ceremony — no `brew upgrade`, no `cargo install --force`, no version-pinning files.
- Verified — every binary must be cryptographically tied to the published release.
- Crash-safe — a partial download or interrupted swap must not leave the user with no `finance-cli`.
- Non-intrusive — the update check must not slow down the user's actual command.

The user's reading surface is WhatsApp; the binary lives on their laptop. They will not notice "your version is 30 commits behind" prompts and will not run an update command manually. The binary has to handle this itself.

## Decision

**The CLI auto-updates atomically with SHA-256 verification, throttled to once per 24 hours.** The flow:

1. **Throttle**: on every invocation that isn't a `self …` subcommand, check `update-state.json` in the data dir. Skip if the last check was <24h ago, or if `FINANCE_OS_NO_AUTO_UPDATE=1`, or if `FINANCE_OS_UPDATED=<version>` is set (loop prevention).
2. **Probe**: 2-second HTTP timeout against the GitHub Releases API. A slow or failed check never delays the user's real command.
3. **Download**: fetch the platform tarball + its `.sha256` sidecar. Verify the SHA **before** unpacking. Path-traversal guard on every archive entry (reject `..`, reject absolute paths).
4. **Swap**: atomically rename the new binary over the running one. The kernel keeps the old inode alive for the running process; the path now points to the new inode.
5. **Re-exec**: `execv` to replace the process image with the new binary, passing the original `argv` plus `FINANCE_OS_UPDATED=<version>` so the child does not re-check.

`finance-cli self check` and `finance-cli self update` provide manual surfaces; both bypass the 24h throttle.

## Options considered

- **Atomic in-place self-update** (chosen): zero user friction, survives interrupted downloads (the rename is atomic; a partial download is discarded before the swap).
- **Package managers (Homebrew, apt, AUR)**: every platform is a separate maintenance burden; users on systems without a maintained package fall through; release cadence couples to packagers.
- **Prompt the user to upgrade manually**: ignored by humans, defeated by AI agents that run commands non-interactively.
- **Background daemon that updates the binary**: violates [ADR-0001](0001-single-binary-rust-cli.md); adds a moving part that needs its own update story.

## Consequences

- **Easier**: zero-ceremony upgrades; users always run a near-current binary; agent integrations don't need version pinning; security patches reach users automatically.
- **Harder**: any change to the update path is high-stakes — a bug there can brick installations. The code is correspondingly small and conservative.
- **Invariants for the codebase**:
  - SHA-256 of the tarball is verified **before** unpacking. Order is not negotiable.
  - Archive entries with `..` or absolute paths are rejected — no exceptions.
  - The 24h throttle is enforced via `update-state.json`; tests cover both throttled and forced paths.
  - The HTTP probe has a 2-second timeout. Real commands never wait on the network for an update check.
  - `self …` subcommands disable the auto-check to prevent recursive scenarios.
  - The atomic swap relies on POSIX `rename` semantics; Windows support (if added later) needs a different strategy.
- **Re-evaluation triggers**: a platform where atomic rename of a running binary is impossible (Windows added as a first-class target); a security incident requiring a stronger verification chain (e.g. minisign / Sigstore over SHA-256).
