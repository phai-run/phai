#!/usr/bin/env bash
#
# phai installer
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh | bash -s -- --prefix=$HOME/.local
#
set -euo pipefail

REPO="phai-run/phai"
ASSET_PREFIX="phai-cli"      # prefix used in GitHub release asset filenames
BINARY_NAME="phai"           # actual binary name inside the tarball and on disk
DEFAULT_PREFIX="${HOME}/.local"
PREFIX="${DEFAULT_PREFIX}"
VERSION="latest"
APP_MODE=0
APP_PORT=4317                # high port → no admin prompt for the user agent

# ─── Parse args ─────────────────────────────────────────────────────────────
for arg in "$@"; do
  case "$arg" in
    --prefix=*) PREFIX="${arg#*=}" ;;
    --version=*) VERSION="${arg#*=}" ;;
    --app) APP_MODE=1 ;;
    --app-port=*) APP_PORT="${arg#*=}" ;;
    --help|-h)
      cat <<EOF
phai installer

Options:
  --prefix=PATH    Install directory (default: \$HOME/.local). Binary goes to PREFIX/bin/${BINARY_NAME}.
  --version=TAG    Specific release tag (e.g. v0.5.1). Defaults to latest.
  --app            Consumer install: also set up the Phai background app + launcher
                   and open the activation screen in the browser (no terminal needed
                   afterwards). Uses a user-level service on a high port (no sudo).
  --app-port=PORT  Port for the --app service (default: ${APP_PORT}).
  -h, --help       Show this help.
EOF
      exit 0
      ;;
    *)
      echo "Unknown option: $arg" >&2
      exit 1
      ;;
  esac
done

# ─── Detect platform ────────────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS-$ARCH" in
  Darwin-arm64)   TARGET="aarch64-apple-darwin" ;;
  Darwin-x86_64)  TARGET="x86_64-apple-darwin" ;;
  *)
    cat <<EOF >&2
phai: unsupported platform: $OS-$ARCH

Currently supported targets:
  Darwin-arm64   (macOS Apple Silicon)
  Darwin-x86_64  (macOS Intel)

Build from source: https://github.com/${REPO}#build-from-source
EOF
    exit 1
    ;;
esac

# ─── Resolve version ────────────────────────────────────────────────────────
if [ "$VERSION" = "latest" ]; then
  TAG="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' \
    | head -1 \
    | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
  if [ -z "$TAG" ]; then
    echo "phai: could not resolve latest release tag" >&2
    exit 1
  fi
else
  TAG="$VERSION"
fi

# ─── Download and verify ────────────────────────────────────────────────────
ASSET="${ASSET_PREFIX}-${TARGET}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "→ Downloading ${BINARY_NAME} ${TAG} for ${TARGET}..."
curl -fsSL -o "${TMPDIR}/${ASSET}"        "${BASE_URL}/${ASSET}"
curl -fsSL -o "${TMPDIR}/${ASSET}.sha256" "${BASE_URL}/${ASSET}.sha256"

echo "→ Verifying SHA-256..."
EXPECTED="$(awk '{print $1}' "${TMPDIR}/${ASSET}.sha256")"
ACTUAL="$(shasum -a 256 "${TMPDIR}/${ASSET}" | awk '{print $1}')"
if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "phai: checksum mismatch (expected $EXPECTED, got $ACTUAL)" >&2
  exit 1
fi

# ─── Install ────────────────────────────────────────────────────────────────
INSTALL_DIR="${PREFIX}/bin"
mkdir -p "$INSTALL_DIR"

echo "→ Installing to ${INSTALL_DIR}/${BINARY_NAME}..."
tar -xzf "${TMPDIR}/${ASSET}" -C "${TMPDIR}"
mv "${TMPDIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

# ─── PATH check ─────────────────────────────────────────────────────────────
case ":$PATH:" in
  *":${INSTALL_DIR}:"*)
    echo "✔ ${INSTALL_DIR} is in your PATH."
    ;;
  *)
    cat <<EOF

⚠ ${INSTALL_DIR} is NOT in your PATH.
   Add this line to your shell rc (.zshrc / .bashrc):

     export PATH="${INSTALL_DIR}:\$PATH"

   Then reload: source ~/.zshrc

EOF
    ;;
esac

# ─── Done ───────────────────────────────────────────────────────────────────
INSTALLED_VERSION="$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null || echo "?")"

if [ "$APP_MODE" = "1" ]; then
  # Consumer flow: install the background service + Phai.app launcher and open
  # the activation screen. From here the user never needs the terminal again —
  # they attach their key file and type the passphrase in the browser.
  echo "→ Setting up the Phai app..."
  "${INSTALL_DIR}/${BINARY_NAME}" serve install --port "${APP_PORT}"
  # Give the launchd agent a moment to bind the port, then open the browser.
  sleep 1
  open "http://localhost:${APP_PORT}/" >/dev/null 2>&1 || true
  cat <<EOF

✓ Phai is installed: ${INSTALLED_VERSION}

  The activation screen should now be open in your browser. If not, open:
    http://localhost:${APP_PORT}/

  Phai also lives in your ~/Applications folder — open it any time from there.

EOF
  exit 0
fi

cat <<EOF

✓ Installed: ${INSTALLED_VERSION}
  Location:  ${INSTALL_DIR}/${BINARY_NAME}

Next steps:
  ${BINARY_NAME} --help            # show available commands
  ${BINARY_NAME} self check        # verify auto-update connectivity

EOF
