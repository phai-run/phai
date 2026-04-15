#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Run Finance OS sync using the local ./.private/ overlay.

Usage:
  scripts/local-sync.sh [sync pluggy args...]

Required local links:
  .private/runtime
  .private/pluggy.env
  .private/pluggy-config.json
  .private/contas.csv
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRIVATE_DIR="$REPO_ROOT/.private"
FINANCE_BIN="${FINANCE_BIN:-$REPO_ROOT/target/release/finance-cli}"

for path in \
  "$PRIVATE_DIR/runtime" \
  "$PRIVATE_DIR/pluggy.env" \
  "$PRIVATE_DIR/pluggy-config.json" \
  "$PRIVATE_DIR/contas.csv"
do
  if [[ ! -e "$path" ]]; then
    echo "Missing required private link: $path" >&2
    echo "Run scripts/setup-private-links.sh first." >&2
    exit 1
  fi
done

if [[ ! -x "$FINANCE_BIN" ]]; then
  echo "finance-cli binary not found or not executable: $FINANCE_BIN" >&2
  echo "Build it with: cargo build --release" >&2
  exit 1
fi

set -a
source "$PRIVATE_DIR/pluggy.env"
set +a

export FINANCE_OS_CONFIG_DIR="$PRIVATE_DIR/runtime"

exec "$FINANCE_BIN" sync pluggy \
  --pluggy-config "$PRIVATE_DIR/pluggy-config.json" \
  --accounts-csv "$PRIVATE_DIR/contas.csv" \
  "$@"
