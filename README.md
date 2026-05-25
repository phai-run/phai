<div align="center">

# 💸 Finance OS

**Your personal-finance runtime. Sync Brazilian banks, query in SQL, report on WhatsApp.**

[![CI](https://github.com/feliperun/finance-os/actions/workflows/ci.yml/badge.svg)](https://github.com/feliperun/finance-os/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/feliperun/finance-os?label=release&color=blue)](https://github.com/feliperun/finance-os/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)](https://www.rust-lang.org)

</div>

---

Finance OS is a single-binary CLI that turns your bank feed into a **queryable, scriptable, reportable** finance database. It connects to [Pluggy](https://pluggy.ai) (Brazilian open-finance aggregator), normalizes everything into either SQLite (local) or BigQuery (production), and ships with reports designed to be read on your phone — not in a dashboard.

```bash
$ finance-cli report daily-pulse
📊 Pulse · últimos 7 dias

🍽️ Alimentação · R$ 487,30
  • Mercado · R$ 150,00 (13/mai)
  • iFood · R$ 87,30 (12/mai)

🏠 Moradia · R$ 1.200,00
  • Aluguel · R$ 1.200,00 (10/mai)

💰 Entradas · R$ 8.500,00
  • Salário · R$ 5.000,00 (12/mai)

Saldo do período: +R$ 6.012,70 ✅
```

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/feliperun/finance-os/main/install.sh | bash
```

The installer detects your platform (macOS Apple Silicon or Intel), downloads the matching binary into `~/.local/bin/finance-cli`, verifies its SHA-256, and warns if that path isn't in your `$PATH`. To pin a version or change the install dir:

```bash
curl -fsSL https://raw.githubusercontent.com/feliperun/finance-os/main/install.sh \
  | bash -s -- --version=v1.6.0 --prefix=/usr/local
```

Other paths: [build from source](#build-from-source) · [cargo install](#cargo-install)

After install, the binary self-updates: it checks GitHub Releases at most **once every 24h**, downloads, validates SHA-256, atomically replaces itself, and re-execs the command you ran. Zero ceremony.

## Features

- 🏦 **Pluggy sync** — Brazilian open-finance aggregator, automatic pagination, idempotent imports, account snapshots for balance history.
- 🗃 **Dual backend** — SQLite for local/dev (zero setup) or BigQuery for production (multi-device, joinable with Sheets).
- 📊 **Reports built for humans** — WhatsApp-friendly by default, grouped by category, `--raw` flag for agents that want JSON.
- 💰 **Budgeting & forecasting** — category budgets with alerts, installment chain tracking, forecast vs actual.
- 🧾 **Transaction splits** — split a single bank transaction into multiple categorized lines (groceries → food + cleaning + pets).
- 📜 **Audit trail** — append-only event log on every write. Every change is replayable.
- 🎯 **Decimal precision** — `rust_decimal` end-to-end. No floating-point lies on amounts.
- 🤖 **AI-ready** — first-class skill integration for [OpenClaw](https://openclaw.io), Claude, and other agent frameworks via a single wrapper script.
- ⬆️ **Self-updating** — single-binary install, atomic in-place upgrade on every release.

## Quickstart

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/feliperun/finance-os/main/install.sh | bash

# Initialize the local SQLite backend
finance-cli auth setup --backend local --actor-id $USER
finance-cli admin migrate

# Sync from Pluggy
export PLUGGY_CLIENT_ID=your-client-id
export PLUGGY_CLIENT_SECRET=your-client-secret
finance-cli sync pluggy --pluggy-config pluggy-config.json

# Look at the result
finance-cli report daily-pulse
finance-cli report monthly-spend
finance-cli report card-summary
```

See [BigQuery setup](#bigquery-setup) below for the multi-device / Sheets-friendly backend.

## Reports

All reports produce a human-readable output by default and accept `--raw` for JSON (consumed by AI agents, scripts, dashboards). The legacy `--json` flag continues to work.

| Command | What it shows |
|---|---|
| `report daily-pulse` | Recent transactions, grouped by category |
| `report monthly-spend` | Current month broken down by category |
| `report cashflow` | Cash-basis monthly summary for checking accounts; use `--details`, `--forecast`, and `--tui` for the interactive month dashboard |
| `report cashflow-chart` | SVG chart of cash-basis evolution (last N months) with optional `--forecast` overlay |
| `report card-summary` | Current credit-card cycle (open + closed bills) |
| `report card-closed-insights` | What changed in the most recent closed bill |
| `report budget-status` | Budget vs actual per category, with alerts |
| `report installments` | Active parcela chains (X de Y), with projected end |
| `report forecast-vs-actual` | Planned amounts vs what actually happened |
| `report uncategorized` | Transactions still needing a category |
| `report data-health` | Consistency checks across the dataset |

Add `--raw` to any of them for structured output.

## Command surface

<details>
<summary>Click to expand the full command tree</summary>

```text
finance-cli auth setup              Configure backend and credentials
finance-cli admin migrate           Apply pending database migrations
finance-cli admin import-legacy     Import from legacy CSV files
finance-cli sync pluggy             Sync transactions from Pluggy
finance-cli report <subcommand>     See "Reports" above
finance-cli review                  Open the fast terminal review UI (--month/--account-id/--category/--merchant)
finance-cli tx upsert-manual        Add a manual transaction
finance-cli tx categorize           Assign category to a transaction
finance-cli tx set-anatomy          Edit human transaction fields
finance-cli tx set-context          Deprecated alias for setting a human description
finance-cli tx find                 Search transactions by description
finance-cli tx pending              List uncategorized transactions
finance-cli tx pending-human        List missing description, merchant, or purpose fields
finance-cli tx review-human         TUI/OpenClaw review of human fields and category, with queue filters
finance-cli tx set-context-by-desc  Deprecated alias for setting descriptions by raw match
finance-cli tx split <subcommand>   Split a transaction into multiple lines
finance-cli forecast upsert         Create or update a forecast entry
finance-cli forecast refresh        Full pipeline: installments + reconcile + materialise + suggest
finance-cli forecast reconcile      Match active forecasts to recent transactions (sets realizado)
finance-cli rule upsert/list/inspect Classification rule management
finance-cli account upsert          Create or update an account
finance-cli budget upsert/list      Category budget management
finance-cli self check              Check for available updates
finance-cli self update             Force-update to the latest release
```

</details>

## BigQuery setup

```bash
# 1. Create a GCP project + dataset (e.g. finance_os).
# 2. Create a service account with BigQuery Data Editor + Job User roles.
# 3. Download the JSON key.

finance-cli auth setup \
  --backend bigquery \
  --actor-id $USER \
  --project-id your-gcp-project \
  --dataset-id finance_os \
  --service-account-path /path/to/service-account.json

finance-cli admin migrate
```

Optional: use a Google Sheet as a category/human-field override source so your manual classifications survive across machines. See [docs/google-sheets-overrides.md](docs/google-sheets-overrides.md).

## Configuration

| Variable | Description |
|---|---|
| `FINANCE_OS_CONFIG_DIR` | Config directory. Default: `~/Library/Application Support/finance-os` (macOS) or `~/.config/finance-os` (Linux). |
| `FINANCE_OS_DATA_DIR` | Data directory (holds `finance-os.db` and `update-state.json`). Same defaults as above. |
| `FINANCE_OS_NO_AUTO_UPDATE` | Set to `1` to disable automatic update checks. |
| `PLUGGY_CLIENT_ID` / `PLUGGY_CLIENT_SECRET` | Pluggy API credentials. |

## Build from source

```bash
git clone https://github.com/feliperun/finance-os.git
cd finance-os
cargo build --release
./target/release/finance-cli --version
```

Requires Rust 1.90+ (`rustup update stable`).

### cargo install

```bash
cargo install --git https://github.com/feliperun/finance-os.git --bin finance-cli
```

## AI assistant integration

The `integrations/openclaw/` directory contains a shell wrapper + skill definition that exposes `finance-cli` to an AI assistant. The pattern (wrapper + skill markdown) adapts to Claude Skills, OpenClaw, or any frame that exec's commands.

Agents should always invoke reports with `--raw` to get JSON instead of the human-friendly default.

## Architecture

```text
crates/
  finance-core/     Domain logic, storage trait, models, Pluggy client
  finance-cli/      CLI binary, report formatters, auto-update
schema/
  sqlite/           SQLite migrations
  bigquery/         BigQuery migrations
integrations/
  openclaw/         AI assistant skill + wrapper
```

The `FinanceStore` trait abstracts both backends. Migrations are embedded into the binary at compile time via `include_str!`. Decimal arithmetic uses `rust_decimal` throughout.

## Self-update under the hood

On macOS, the updater downloads the latest tarball, validates its SHA-256 against the published `.sha256` asset, then **atomically renames** the new binary over the running one. The kernel keeps the old inode alive for the running process; the path now points to the new inode. We then `execv` to replace the process image with the new binary, passing the original argv plus a `FINANCE_OS_UPDATED=<version>` sentinel that disables auto-check in the child to prevent loops.

The check is gated by:

- A 24-hour throttle (`update-state.json` in your data dir).
- Skip when `FINANCE_OS_NO_AUTO_UPDATE=1`.
- Skip when running a `self ...` subcommand.
- 2-second HTTP timeout on the API check — never delays your real command.

## Security

- Tarball SHA-256 is validated **before** unpacking.
- Path-traversal guard rejects any archive entry containing `..` or absolute paths.
- Auto-update runs unauthenticated against public GitHub releases — no token embedded in the binary.
- See [SECURITY.md](SECURITY.md) for the disclosure policy.

## Contributing

Pull requests welcome. Before large changes, open an issue to discuss the approach.

- Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`).
- Migrations land in **both** `schema/sqlite/` and `schema/bigquery/` and must be idempotent.
- E2E tests prefer the SQLite backend over mocks.
- AGENTS.md guardrails: no personal counterparty names, account labels, or statement fingerprints in shared code.

## License

[MIT](LICENSE).
