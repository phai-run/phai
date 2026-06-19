# macOS notarization (enable when we get a Developer ID)

The graphical installer (`installer/Instalar Phai.command` → `install.sh --app`) works
today, but the `.command` is **unsigned**, so on first run macOS Gatekeeper shows
"cannot verify the developer". The free workaround is **right-click → Open** once
(see [README](../README.md#activate-on-another-machine-no-terminal--adr-0034)).

To remove that prompt for good we need an Apple **Developer ID**, which requires a
paid **Apple Developer Program** membership (**$99/yr USD**). A free Apple ID can
only sign for local development (7-day expiry, own devices) and **cannot notarize**.

This doc is the ready-to-run recipe. Nothing here is wired into CI yet — enabling it
is a deliberate, one-time step once the account exists.

## Decision when enabled

Stop shipping the bare `.command` as the consumer entry point. Ship a **signed +
notarized `.pkg`** as the graphical installer release asset. A `.pkg` can be
double-clicked with zero Gatekeeper friction once notarized, and its `postinstall`
script runs `phai serve install` exactly like `install.sh --app` does. Scripts
(`.command`) cannot be notarized — only bundles/installers can — which is why the
artifact type changes.

## One-time account setup

1. Enroll in the Apple Developer Program ($99/yr): <https://developer.apple.com/programs/>.
2. In Keychain Access → Certificate Assistant, create a CSR; in the Developer portal
   create two certificates:
   - **Developer ID Application** — signs the binary/app bundle.
   - **Developer ID Installer** — signs the `.pkg`.
3. Create an **App Store Connect API key** (Users and Access → Integrations → Keys,
   role: Developer). Download the `AuthKey_XXXX.p8`. Note the **Key ID** and **Issuer ID**.
   (notarytool with an API key avoids storing an Apple ID password.)

## CI secrets to add (repo → Settings → Secrets and variables → Actions)

| Secret | What |
|---|---|
| `MACOS_CERT_P12` | base64 of the exported Developer ID certs (`.p12`) |
| `MACOS_CERT_PASSWORD` | password for the `.p12` |
| `MACOS_NOTARY_KEY_P8` | base64 of the App Store Connect `AuthKey_XXXX.p8` |
| `MACOS_NOTARY_KEY_ID` | the key id |
| `MACOS_NOTARY_ISSUER_ID` | the issuer id |
| `MACOS_SIGN_IDENTITY` | e.g. `Developer ID Application: Felipe … (TEAMID)` |
| `MACOS_INSTALLER_IDENTITY` | e.g. `Developer ID Installer: Felipe … (TEAMID)` |

Then set repo **variable** `MACOS_NOTARIZE = true` to switch the gated job on.

## Local steps (also the body of the CI job)

```bash
# 1. import the signing cert into a throwaway keychain
echo "$MACOS_CERT_P12" | base64 -d > cert.p12
security create-keychain -p ci build.keychain
security import cert.p12 -k build.keychain -P "$MACOS_CERT_PASSWORD" -T /usr/bin/codesign
security list-keychains -s build.keychain
security unlock-keychain -p ci build.keychain
security set-key-partition-list -S apple-tool:,apple: -s -k ci build.keychain

# 2. sign the binary with a hardened runtime (required for notarization)
codesign --force --options runtime --timestamp \
  --sign "$MACOS_SIGN_IDENTITY" target/release/phai

# 3. build a component pkg that installs phai and runs `serve install` postinstall
#    (see scripts/pkg/ — payload = the signed binary into /usr/local/bin,
#     postinstall = `"$2/usr/local/bin/phai" serve install --port 4317` as the user)
pkgbuild --root pkgroot --scripts scripts/pkg/scripts \
  --identifier run.phai.cli --version "$VERSION" \
  --install-location / phai-component.pkg
productbuild --sign "$MACOS_INSTALLER_IDENTITY" \
  --package phai-component.pkg "Instalar Phai.pkg"

# 4. notarize + staple
echo "$MACOS_NOTARY_KEY_P8" | base64 -d > notary.p8
xcrun notarytool submit "Instalar Phai.pkg" \
  --key notary.p8 --key-id "$MACOS_NOTARY_KEY_ID" --issuer "$MACOS_NOTARY_ISSUER_ID" \
  --wait
xcrun stapler staple "Instalar Phai.pkg"

# 5. upload "Instalar Phai.pkg" as a release asset (alongside the tarballs)
```

## CI job to paste into `.github/workflows/release-please.yml`

Add as a sibling job to `release-assets`, gated so it is inert until the variable
is set:

```yaml
  macos-installer:
    needs: release-please
    if: ${{ needs.release-please.outputs.phai-cli-released == 'true' && vars.MACOS_NOTARIZE == 'true' }}
    runs-on: macos-14
    steps:
      - uses: actions/checkout@v4
        with: { ref: ${{ needs.release-please.outputs.phai-cli-tag }} }
      # … build web + cargo build --release (same as release-assets) …
      # … then the "Local steps" above, env-mapped from the secrets …
      - name: Upload installer
        run: gh release upload "${{ needs.release-please.outputs.phai-cli-tag }}" "Instalar Phai.pkg" --clobber
        env: { GH_TOKEN: ${{ github.token }} }
```

Until `vars.MACOS_NOTARIZE` is `true`, this job is skipped and the free
right-click → Open path remains the consumer flow.
