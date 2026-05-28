#!/usr/bin/env bash
#
# finance-os installer
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/feliperun/finance-os/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/feliperun/finance-os/main/install.sh | bash -s -- --prefix=$HOME/.local
#
set -euo pipefail

REPO="feliperun/finance-os"
ASSET_PREFIX="finance-cli"   # prefix used in GitHub release asset filenames
BINARY_NAME="fin"            # actual binary name inside the tarball and on disk
DEFAULT_PREFIX="${HOME}/.local"
PREFIX="${DEFAULT_PREFIX}"
VERSION="latest"

# ─── Parse args ─────────────────────────────────────────────────────────────
for arg in "$@"; do
  case "$arg" in
    --prefix=*) PREFIX="${arg#*=}" ;;
    --version=*) VERSION="${arg#*=}" ;;
    --help|-h)
      cat <<EOF
finance-os installer

Options:
  --prefix=PATH    Install directory (default: \$HOME/.local). Binary goes to PREFIX/bin/${BINARY_NAME}.
  --version=TAG    Specific release tag (e.g. v0.5.1). Defaults to latest.
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
finance-os: unsupported platform: $OS-$ARCH

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
    echo "finance-os: could not resolve latest release tag" >&2
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
  echo "finance-os: checksum mismatch (expected $EXPECTED, got $ACTUAL)" >&2
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
cat <<EOF

✓ Installed: ${INSTALLED_VERSION}
  Location:  ${INSTALL_DIR}/${BINARY_NAME}

Next steps:
  ${BINARY_NAME} --help            # show available commands
  ${BINARY_NAME} self check        # verify auto-update connectivity

EOF
