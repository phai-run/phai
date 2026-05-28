#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Create local symlinks under ./.private/ for private phai files.

Usage:
  scripts/setup-private-links.sh \
    --runtime <runtime-name> \
    --pluggy-config <path> \
    --accounts-csv <path> \
    [--configs-root <path>] \
    [--private-dir <path>]

Example:
  scripts/setup-private-links.sh \
    --configs-root "$HOME/finance-os-configs" \
    --runtime personal-runtime \
    --pluggy-config "$HOME/private-finance/pluggy-config.json" \
    --accounts-csv "$HOME/private-finance/data/contas.csv"
EOF
}

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIGS_ROOT="$HOME/finance-os-configs"
PRIVATE_DIR="$REPO_ROOT/.private"
RUNTIME_NAME=""
PLUGGY_CONFIG_PATH=""
ACCOUNTS_CSV_PATH=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --configs-root)
      CONFIGS_ROOT="$2"
      shift 2
      ;;
    --private-dir)
      PRIVATE_DIR="$2"
      shift 2
      ;;
    --runtime)
      RUNTIME_NAME="$2"
      shift 2
      ;;
    --pluggy-config)
      PLUGGY_CONFIG_PATH="$2"
      shift 2
      ;;
    --accounts-csv)
      ACCOUNTS_CSV_PATH="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$RUNTIME_NAME" || -z "$PLUGGY_CONFIG_PATH" || -z "$ACCOUNTS_CSV_PATH" ]]; then
  usage >&2
  exit 1
fi

RUNTIME_PATH="$CONFIGS_ROOT/runtime/$RUNTIME_NAME"
PLUGGY_ENV_PATH="$CONFIGS_ROOT/pluggy/pluggy.env"

for path in "$RUNTIME_PATH" "$PLUGGY_ENV_PATH" "$PLUGGY_CONFIG_PATH" "$ACCOUNTS_CSV_PATH"; do
  if [[ ! -e "$path" ]]; then
    echo "Required path not found: $path" >&2
    exit 1
  fi
done

mkdir -p "$PRIVATE_DIR"
ln -sfn "$RUNTIME_PATH" "$PRIVATE_DIR/runtime"
ln -sfn "$PLUGGY_ENV_PATH" "$PRIVATE_DIR/pluggy.env"
ln -sfn "$PLUGGY_CONFIG_PATH" "$PRIVATE_DIR/pluggy-config.json"
ln -sfn "$ACCOUNTS_CSV_PATH" "$PRIVATE_DIR/contas.csv"

cat <<EOF
Private links created in $PRIVATE_DIR
- runtime -> $RUNTIME_PATH
- pluggy.env -> $PLUGGY_ENV_PATH
- pluggy-config.json -> $PLUGGY_CONFIG_PATH
- contas.csv -> $ACCOUNTS_CSV_PATH
EOF
