---
type: ADR
id: "0017"
title: "Release signature verification (minisign) on top of SHA-256"
status: active
date: 2026-05-25
supersedes: "0007"
---

## Context

[ADR-0007](0007-atomic-self-update.md) committed to atomic self-update with SHA-256 verification, with the explicit re-evaluation trigger:

> a security incident requiring a stronger verification chain (e.g. minisign / Sigstore over SHA-256).

The 2026-05 audit re-examined that gate and concluded it should be tripped pre-emptively, not after an incident. SHA-256 alone guarantees **integrity** (the bytes you received match the bytes the publisher uploaded), but it carries **zero authentication** — the digest is fetched from the same GitHub Releases bucket as the tarball. If that bucket is compromised (account takeover, OAuth leak, malicious workflow), an attacker can publish a malicious binary alongside a matching `.sha256` sidecar and every auto-updating CLI instance accepts it without complaint.

Because phai auto-updates with no user confirmation (see ADR-0007 §Decision), the trust boundary is wider than it looks: the same compromise propagates to every laptop running the CLI within 24 hours.

## Decision

**Verify a minisign signature over the release tarball, alongside the existing SHA-256 check.**

Concretely:

1. **Signing**: a release-time signing key (Ed25519, generated locally, secret never leaves the maintainer's machine or — when CI signs — GitHub Actions encrypted secrets) signs each platform tarball at release time. The signature file is published as `<asset>.minisig` alongside the tarball.
2. **Embedded public key**: the corresponding minisign public key is compiled into the CLI binary as a `const PUBLIC_KEY: &str`. Rotation is a code change; the release that rotates the key carries both the old and the new pubkey for one cycle so in-flight updates don't break.
3. **Verification order**: the updater downloads the tarball, the `.sha256` sidecar, and the `.minisig` sidecar. SHA-256 is checked first (cheap integrity gate). Only on success does the minisign check run against the embedded pubkey. Both must pass before unpacking. The path-traversal guards from ADR-0007 still apply at extraction time.
4. **Transition mode**: until the release CI starts producing `.minisig` files, the updater treats a missing sidecar as a soft warning rather than a hard failure (logged once per check), so existing installations continue to update. Once CI is signing reliably for one full release cycle, the warning is promoted to a hard error in a follow-up commit. The behaviour is gated by a single `REQUIRE_SIGNATURE: bool` constant — flipping it is a one-line change.
5. **Emergency override**: `FINANCE_OS_SKIP_SIG_VERIFY=1` lets a maintainer bypass signature verification for diagnostic / break-glass purposes. The existing `FINANCE_OS_NO_AUTO_UPDATE=1` is unchanged.

The verifier itself is the `minisign-verify` crate — verification-only, zero transitive deps, no `openssl` exposure. The keygen / signing side does not live in this binary.

## Options considered

- **Sigstore / cosign keyless**: stronger transparency-log guarantees, but adds OIDC, an upstream service dependency, and ~5 MB of binary weight. Overkill for a single-maintainer project where the trust root is the maintainer's laptop.
- **GPG signatures**: ubiquitous tooling but a heavyweight verifier dependency, large signature files, and a worse UX for ad-hoc key generation. minisign is the smaller, modern equivalent.
- **Require manual `finance-cli self update --verify-key=…`**: defeats the auto-update model (ADR-0007 §Decision).
- **Stay with SHA-256 only**: status quo. Re-evaluated and rejected — the auto-update model already removes user friction, so adding authenticity to the chain should not be deferred to a post-incident reaction.

## Consequences

- **Easier**: trust-root is now the maintainer's signing key (or a tightly-scoped CI secret), not GitHub's bucket. A compromised release bucket alone cannot push a malicious binary.
- **Harder**: a lost signing key blocks new releases until the embedded pubkey is rotated, which itself ships via a CLI that still trusts the old key — so the rotation path needs the dual-key cycle described above.
- **Invariants for the codebase**:
  - SHA-256 is verified before minisign; minisign is verified before unpacking; unpacking still rejects archive path traversal.
  - The embedded pubkey is a single `const` in `update.rs` (or a sibling module). Tests assert it is non-empty and parseable on startup.
  - `REQUIRE_SIGNATURE` defaults to `false` until CI is signing reliably; the auto-update flow logs a one-line warning when a `.minisig` sidecar is missing.
  - `FINANCE_OS_SKIP_SIG_VERIFY=1` is documented in `--help` for `self check` and `self update` and must remain off by default.
- **Re-evaluation triggers**: signing key compromise (forces rotation cycle), addition of a Windows / Linux target where minisign-verify availability changes, a credible alternative emerges in the Sigstore ecosystem.
