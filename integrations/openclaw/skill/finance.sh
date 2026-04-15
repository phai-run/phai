#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="${FORD_WORKSPACE_ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"

resolve_runtime_root() {
  if [[ -n "${FINANCE_OS_RUNTIME_ROOT:-}" ]]; then
    if [[ -x "$FINANCE_OS_RUNTIME_ROOT/bin/finance-cli" || -x "$FINANCE_OS_RUNTIME_ROOT/target/release/finance-cli" ]]; then
      printf '%s\n' "$FINANCE_OS_RUNTIME_ROOT"
      return
    fi
  fi

  local candidates=(
    "$WORKSPACE_ROOT/.finance-os-runtime"
    "$WORKSPACE_ROOT/../finance-os-runtime"
    "$WORKSPACE_ROOT"
    "${HOME:-}/.local/share/finance-os/runtime"
    "${HOME:-}/.finance-os/runtime"
    "${HOME:-}/finance-os"
  )

  local root
  for root in "${candidates[@]}"; do
    if [[ -n "$root" && ( -x "$root/bin/finance-cli" || -x "$root/target/release/finance-cli" ) ]]; then
      printf '%s\n' "$root"
      return
    fi
  done

  echo "finance-cli runtime not found" >&2
  exit 1
}

RUNTIME_ROOT="$(resolve_runtime_root)"

if [[ -x "$RUNTIME_ROOT/bin/finance-cli" ]]; then
  BIN_PATH="$RUNTIME_ROOT/bin/finance-cli"
  DEFAULT_CONFIG_DIR="$RUNTIME_ROOT/config"
  DEFAULT_DATA_DIR="$RUNTIME_ROOT/data"
elif [[ -x "$RUNTIME_ROOT/target/release/finance-cli" ]]; then
  BIN_PATH="$RUNTIME_ROOT/target/release/finance-cli"
  DEFAULT_CONFIG_DIR="${FINANCE_OS_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/finance-os}"
  DEFAULT_DATA_DIR="${FINANCE_OS_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/finance-os}"
else
  echo "finance-cli binary not found" >&2
  exit 1
fi

export FINANCE_OS_CONFIG_DIR="${FINANCE_OS_CONFIG_DIR:-$DEFAULT_CONFIG_DIR}"
export FINANCE_OS_DATA_DIR="${FINANCE_OS_DATA_DIR:-$DEFAULT_DATA_DIR}"

if [[ -f "$FINANCE_OS_CONFIG_DIR/pluggy.env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$FINANCE_OS_CONFIG_DIR/pluggy.env"
  set +a
fi

args=("$@")

if [[ "${1:-}" == "sync" && "${2:-}" == "pluggy" ]]; then
  has_pluggy_config=0
  has_accounts_csv=0
  for arg in "${args[@]}"; do
    case "$arg" in
      --pluggy-config|--pluggy-config=*)
        has_pluggy_config=1
        ;;
      --accounts-csv|--accounts-csv=*)
        has_accounts_csv=1
        ;;
    esac
  done
  if [[ "$has_pluggy_config" -eq 0 ]]; then
    args+=(--pluggy-config "$WORKSPACE_ROOT/finance/pluggy-config.json")
  fi
  if [[ "$has_accounts_csv" -eq 0 ]]; then
    args+=(--accounts-csv "$WORKSPACE_ROOT/finance/data/contas.csv")
  fi
fi

exec "$BIN_PATH" "${args[@]}"
