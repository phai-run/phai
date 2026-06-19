---
type: ADR
id: "0034"
title: "Self-contained encrypted invite for multi-device activation"
status: active
date: 2026-06-19
---

## Context

phai is a single Rust binary that reads and writes a shared BigQuery dataset
([ADR-0001](0001-single-binary-rust-cli.md)). Today, pointing a second machine at
that dataset means running `phai auth setup --backend bigquery` with a project id,
dataset id, actor id, and a path to a Google service-account JSON key — a terminal
flow that assumes the operator understands GCP credentials.

The owner wants a non-technical household member to use phai on their own Mac to
**read, recategorize, and simulate** against the same dataset, without ever touching
Pluggy sync (the owner keeps the data current from their own machine / openclaw).
That person should not see GCP, JSON keys, or a terminal. They should paste one key
the owner generates into a Settings field, and be done.

BigQuery authenticates by service-account key ([bigquery.rs](../../crates/phai-core/src/storage/bigquery.rs)).
Writes (recategorization) require the credential on the second machine — there is no
read-only-token shortcut short of standing up a broker service. For a two-person
household, hosting a token broker is unjustified infrastructure.

## Decision

**Ship a self-contained, passphrase-encrypted invite blob. The owner generates it with
`phai invite create`; the household member pastes it (plus a passphrase) into the web
Settings panel, which decrypts it, writes the BigQuery config, and activates the app.**

The invite is a string `PHAI1E-<base64url>` whose plaintext payload is:

```json
{ "v": 1, "project_id": "...", "dataset_id": "phai", "actor_id": "esposa",
  "role": "rw", "service_account": { ...full SA JSON... }, "label": "MacBook Esposa" }
```

Properties of the decision:

- **Self-contained.** The blob *embeds* the service-account key. No broker, no network
  dependency at activation time, works offline. The blob therefore **is** a credential
  and is treated as a secret in all UX and docs.
- **Always encrypted.** Every invite is sealed with a passphrase the owner chooses and
  communicates out of band. KDF is **Argon2id**; AEAD is **XChaCha20-Poly1305**. The
  envelope carries the Argon2 parameters, a random 16-byte salt, and a random 24-byte
  nonce, followed by the ciphertext. Wrong passphrase or any tampering fails decryption.
  Derived key material is zeroized after use.
- **Dedicated, scoped service account.** The embedded key is a *separate* GCP service
  account (`phai-family`) granted only **BigQuery Data Editor + Job User** — never the
  owner's admin key. The member can recategorize and simulate but cannot run schema
  migrations or drop tables. The owner continues to migrate from their own machine.
- **Consumer profile without Pluggy.** The activated machine never syncs. `PLUGGY_*`
  configuration is absent and not required; `phai sync` is simply unused there.
- **Activation surface is the web app.** A first run with no usable config boots phai
  into an activation mode that serves a Settings/activation screen instead of the
  dashboard. On success it writes the config + key and re-initializes against BigQuery.

## Options considered

- **Self-contained encrypted invite** (chosen): One string + passphrase, no infra,
  works offline. Cost: the blob is a long-lived credential; rotation means issuing a new
  invite and (if needed) rotating the dedicated SA key in GCP.
- **Token broker / short-lived credentials**: A hosted service mints expiring,
  revocable tokens. Strictly more secure and revocable, but requires standing up and
  operating infrastructure for two users — disproportionate.
- **Reuse the owner's existing service account**: Fastest to build, but the member would
  inherit the owner's full powers (including migrate/drop). Rejected on least-privilege.
- **Plain (unencrypted) base64 invite**: Simplest for the member, but anyone who sees the
  string owns the dataset. Rejected; encryption is mandatory.

## Consequences

- A new `phai invite create` command and an `invite` envelope module live in
  `phai-core`; the CLI is owner-only by virtue of needing a real service-account key.
- New crypto dependencies enter the workspace (`argon2`, `chacha20poly1305`, `base64`,
  `zeroize`). They are audited RustCrypto crates and must pass `cargo audit` /
  `cargo deny check licenses`.
- A later change adds the bridge activation endpoint (`/api/activate`, `/api/status`),
  the unconfigured serve mode, and the web Settings UI; another adds a consumer installer
  profile. Those are tracked as follow-up work, not this ADR.
- Invite rotation/revocation is manual: re-issue an invite, and rotate the dedicated SA
  key in GCP to invalidate a leaked blob. Re-evaluate this decision if the household grows
  beyond a handful of devices or needs revocation without GCP key rotation — at which point
  a broker (the rejected option) becomes worth its infrastructure cost.
