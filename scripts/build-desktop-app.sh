#!/usr/bin/env bash
# Build the native macOS desktop shell (Pake/Tauri WKWebView) that wraps the
# local `phai serve` app. Produces `Phai.app` (+ zip + dmg) pointing at the
# fixed pairing URL.
#
# This runs in CI (and locally), never on the end user's machine — the compiled
# app ships as a release asset (ADR-0001 keeps the user's install pure-Rust,
# ADR-0039 introduces the shell). Requires: node/npx, a Rust toolchain, and the
# macOS system WebKit — all present on the release runner. The app is UNSIGNED
# (ADR-0039): first launch is right-click → Open.
#
# Usage: scripts/build-desktop-app.sh [OUT_DIR]
#   OUT_DIR  where to leave Phai.app / .app.zip / .dmg (default: ./dist-desktop)
#
# Env overrides:
#   PHAI_APP_URL     URL the shell loads       (default http://phai.localhost, ADR-0040)
#   PHAI_APP_NAME    app + bundle name         (default Phai)
#   PHAI_APP_ID      bundle identifier         (default run.phai.desktop)
#   PAKE_VERSION     pinned pake-cli version   (default 3.13.1)
#   PAKE_MULTI_ARCH  "1" → universal Intel+ARM (default off; CI sets it)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-$REPO_ROOT/dist-desktop}"
APP_URL="${PHAI_APP_URL:-http://phai.localhost}"
APP_NAME="${PHAI_APP_NAME:-Phai}"
APP_ID="${PHAI_APP_ID:-run.phai.desktop}"
PAKE_VERSION="${PAKE_VERSION:-3.13.1}"
ICON="$REPO_ROOT/crates/phai-cli/assets/Phai.icns"

if [[ ! -f "$ICON" ]]; then
  echo "error: icon not found at $ICON" >&2
  exit 1
fi

multi_arch_flag=()
if [[ "${PAKE_MULTI_ARCH:-}" == "1" ]]; then
  multi_arch_flag=(--multi-arch)
fi

mkdir -p "$OUT_DIR"
workdir="$(mktemp -d)"
mnt="$(mktemp -d)"
cleanup() {
  hdiutil detach "$mnt" -quiet 2>/dev/null || true
  rm -rf "$workdir" "$mnt"
}
trap cleanup EXIT

echo "→ building $APP_NAME.app  (url=$APP_URL, id=$APP_ID, pake=$PAKE_VERSION, multi_arch=${PAKE_MULTI_ARCH:-0})"
(
  cd "$workdir"
  # Chromeless native window: --hide-title-bar keeps the traffic lights but
  # drops browser chrome, so it reads as a native app, not a web view.
  npx --yes "pake-cli@$PAKE_VERSION" "$APP_URL" \
    --name "$APP_NAME" \
    --identifier "$APP_ID" \
    --icon "$ICON" \
    --hide-title-bar \
    --width 1200 \
    --height 820 \
    ${multi_arch_flag[@]+"${multi_arch_flag[@]}"}
)

# Pake emits a .dmg installer; the .app lives inside it.
dmg="$(find "$workdir" -maxdepth 2 -name '*.dmg' -type f | head -1)"
if [[ -z "$dmg" ]]; then
  echo "error: pake did not produce a .dmg" >&2
  exit 1
fi

hdiutil attach "$dmg" -nobrowse -mountpoint "$mnt" -quiet
app_in_dmg="$(find "$mnt" -maxdepth 1 -name "$APP_NAME.app" -type d | head -1)"
if [[ -z "$app_in_dmg" ]]; then
  echo "error: no $APP_NAME.app inside the dmg" >&2
  exit 1
fi

rm -rf "${OUT_DIR:?}/$APP_NAME.app"
ditto "$app_in_dmg" "$OUT_DIR/$APP_NAME.app"
hdiutil detach "$mnt" -quiet
trap 'rm -rf "$workdir" "$mnt"' EXIT

# Zip (for the automated installer to drop into ~/Applications) + dmg (for
# manual drag-install). ditto keeps the bundle + its ad-hoc signature intact.
( cd "$OUT_DIR" && rm -f "$APP_NAME.app.zip" \
    && ditto -c -k --keepParent "$APP_NAME.app" "$APP_NAME.app.zip" )
cp "$dmg" "$OUT_DIR/$APP_NAME.dmg"

echo "✓ $OUT_DIR/$APP_NAME.app"
echo "✓ $OUT_DIR/$APP_NAME.app.zip"
echo "✓ $OUT_DIR/$APP_NAME.dmg"
