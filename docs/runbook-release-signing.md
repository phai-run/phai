# Runbook: release signing (minisign)

How the release tarballs are signed, where the signing key lives, and how to
recover or rotate it. The decision and rationale are in
[ADR-0017](adr/0017-release-signature-verification.md); this runbook is the
operational counterpart.

## The keypair

A single minisign keypair signs every release tarball.

| Part | Where it lives | Role |
| --- | --- | --- |
| **Public key** | embedded in [`crates/phai-cli/src/update.rs`](../crates/phai-cli/src/update.rs) as `SIGNING_PUBLIC_KEY` (the base64 line only, parsed with `PublicKey::from_base64`) | the updater verifies every downloaded tarball against it |
| **Secret key** | GitHub repo secret `MINISIGN_SECRET_KEY` | the release workflow signs with it |
| **Secret key (backup)** | macOS login keychain, item `phai-minisign-key` | local recovery copy |

The secret key is **passwordless** (generated with `minisign -G -W`) so CI signing
never prompts. Treat the GitHub secret as the source of truth; the keychain item
is the human-held backup. The plaintext `phai.key`/`phai.pub` files must never be
committed (`*.key` is gitignored).

## How a release gets signed

`release-please.yml` → `release-assets` job (one run per target):

1. builds the binary, packages `phai-cli-<target>.tar.gz` + `.sha256`;
2. `Sign tarball (minisign)` writes `$MINISIGN_SECRET_KEY` to a temp file and runs
   `minisign -S` to produce `phai-cli-<target>.tar.gz.minisig`;
3. uploads tarball + `.sha256` + `.minisig` to the GitHub Release.

The updater ([`update.rs`](../crates/phai-cli/src/update.rs)) downloads the
tarball, checks the SHA-256, then verifies the `.minisig` against the embedded
public key. While `REQUIRE_SIGNATURE = false` a missing sidecar only warns; once
flipped to `true` a missing/invalid signature is a hard failure.

## Recover the secret key from the keychain

When you need the secret key again — e.g. to re-set the GitHub secret on a new
repo, or after rotating CI credentials:

```bash
security find-generic-password -s phai-minisign-key -w | base64 -d > phai.key
chmod 600 phai.key
```

Inspect (metadata only, no secret printed):

```bash
security find-generic-password -s phai-minisign-key
```

Re-store / update the keychain item (base64 keeps the two-line key byte-exact):

```bash
security add-generic-password -a "$USER" -s phai-minisign-key \
  -D "minisign secret key" -j "phai release signing (ADR-0017)" -U \
  -w "$(base64 -i phai.key)"
```

Always `rm -f phai.key phai.pub` once the secret is back in the keychain / GitHub
secret — never leave plaintext on disk.

## Re-set the GitHub secret

```bash
gh secret set MINISIGN_SECRET_KEY < phai.key
```

Confirm it exists (value is never shown):

```bash
gh secret list | grep MINISIGN
```

## Rotate the signing key

Rotation is a deliberate, staged operation because a too-eager `REQUIRE_SIGNATURE`
plus a key mismatch bricks updates.

1. Generate a fresh passwordless keypair: `minisign -G -W -p phai.pub -s phai.key`.
2. Store the new secret key in the keychain and the GitHub secret (above).
3. Embed the **new** public key in `SIGNING_PUBLIC_KEY`; land it with
   `REQUIRE_SIGNATURE = false` so the transition is non-breaking.
4. Cut one release and confirm `phai self update` verifies it end-to-end.
5. Only then flip `REQUIRE_SIGNATURE = true` in a follow-up.

Existing installs carry the old public key until they update once onto the build
that embeds the new key — that first hop must remain verifiable under the old key
or be allowed by the transition window, so never rotate the key and flip
`REQUIRE_SIGNATURE` in the same release.

## Verify the chain locally

You can prove the GitHub secret and the embedded public key match without CI:

```bash
# recover the secret key (see above), then:
echo "test" > /tmp/sigtest.txt
minisign -S -s phai.key -m /tmp/sigtest.txt -x /tmp/sigtest.txt.minisig
minisign -V -p phai.pub -m /tmp/sigtest.txt   # → "Signature ... verified"
rm -f /tmp/sigtest.txt /tmp/sigtest.txt.minisig phai.key
```

If `minisign -S` prompts for a password, the secret key is **not** passwordless —
regenerate with `-W`, or CI signing will hang.
