<div align="center">

# φ phai

**finanças da casa, inteligência de verdade.**

Rules-first, LLM-neutral personal-finance agent. Terminal-first, built in Rust.

[![CI](https://github.com/phai-run/phai/actions/workflows/ci.yml/badge.svg)](https://github.com/phai-run/phai/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/phai-run/phai?label=release&color=A78BFA)](https://github.com/phai-run/phai/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)](https://www.rust-lang.org)

</div>

---

## φ + fi + ai = phai

One word, three parts:

- **φ** — *phi*, the golden ratio. Proportion, equilibrium, the number that keeps things in balance.
- **fi** — *finanças*. Household money: real expenses, real income, real life.
- **ai** — intelligence. An agent that reads, organizes, and anticipates.

phai is a deterministic layer that puts an LLM **on rails**: **rules first, AI second.** It connects to [Pluggy](https://pluggy.ai) (Brazilian open-finance aggregator), normalizes everything into SQLite (local) or BigQuery (production), and turns your bank feed into a **queryable, scriptable, reportable** finance database. It is not a dashboard and not a "5 tips to save money" app — it informs, it doesn't cheer.

```text
$ phai report daily-pulse
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
curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh | bash
```

> `phai.run/install.sh` is coming soon; use the GitHub raw URL above until DNS is live.

The installer detects your platform (macOS Apple Silicon or Intel), downloads the matching binary into `~/.local/bin/phai`, verifies its SHA-256, and warns if that path isn't in your `$PATH`. To pin a version or change the install dir:

```bash
curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh \
  | bash -s -- --version=v1.6.0 --prefix=/usr/local
```

Other paths: [build from source](#build-from-source) · [cargo install](#cargo-install)

After install, the binary self-updates: it checks GitHub Releases at most **once every 24h**, downloads, validates SHA-256, atomically replaces itself, and re-execs the command you ran. Zero ceremony.

## Quickstart

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh | bash

# Initialize the local SQLite backend
phai auth setup --backend local --actor-id $USER
phai admin migrate

# Sync from Pluggy
export PLUGGY_CLIENT_ID=your-client-id
export PLUGGY_CLIENT_SECRET=your-client-secret
phai sync pluggy --pluggy-config pluggy-config.json

# Look at the result
phai report daily-pulse
phai report monthly-spend
phai report card-summary
```

See [BigQuery setup](#bigquery-setup) below for the multi-device / Sheets-friendly backend.

## Why phai

- 🏦 **Pluggy sync** — Brazilian open-finance aggregator, automatic pagination, idempotent imports, account snapshots for balance history.
- 🗃 **Dual backend** — SQLite for local/dev (zero setup) or BigQuery for production (multi-device, joinable with Sheets).
- 📐 **Rules first, AI second** — classification comes from deterministic rules and effective overrides. The LLM reads and proposes; it never silently decides.
- 🔌 **LLM-neutral** — a single wrapper script exposes phai to [OpenClaw](https://openclaw.io), Claude, or any agent framework that exec's commands. No model lock-in.
- 📊 **Reports built for humans** — readable in 80 columns, grouped by category, `--raw` for agents that want JSON.
- 💰 **Budgeting & forecasting** — category budgets with alerts, installment chain tracking, forecast vs actual.
- 🧾 **Transaction splits** — split a single bank transaction into multiple categorized lines (groceries → food + cleaning + pets).
- 📜 **Audit trail** — append-only event log on every write. Every change is replayable.
- 🎯 **Decimal precision** — `rust_decimal` end-to-end. No floating-point lies on amounts.
- ⬆️ **Self-updating** — single-binary install, atomic in-place upgrade on every release.

## Reports

All reports produce a human-readable output by default and accept `--raw` for JSON (consumed by AI agents, scripts, dashboards). The legacy `--json` flag continues to work.

| Command | What it shows |
|---|---|
| `report daily-pulse` | Recent transactions, grouped by category |
| `report monthly-spend` | Current month broken down by category |
| `report cashflow` | Cash-basis monthly summary for checking accounts; use `--details` and `--forecast` to expand the breakdown |
| `report cashflow-chart` | SVG chart of cash-basis evolution (last N months) with optional `--forecast` overlay and `--scenario-amount` what-if line |
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
phai auth setup              Configure backend and credentials
phai admin migrate           Apply pending database migrations
phai admin import-legacy     Import from legacy CSV files
phai sync pluggy             Sync transactions from Pluggy
phai report <subcommand>     See "Reports" above
phai serve                   Start the web app — the interactive surface for review and forecasts
phai tx upsert-manual        Add a manual transaction
phai tx categorize           Assign category to a transaction
phai tx set-anatomy          Edit human transaction fields
phai tx set-context          Deprecated alias for setting a human description
phai tx find                 Search transactions by description
phai tx pending              List uncategorized transactions
phai tx pending-human        List missing description, merchant, or purpose fields
phai tx review-human         Headless review of human fields and category (--json/--summary/--transaction-id), with queue filters
phai tx set-context-by-desc  Deprecated alias for setting descriptions by raw match
phai tx split <subcommand>   Split a transaction into multiple lines
phai forecast upsert         Create or update a forecast entry
phai forecast refresh        Full pipeline: installments + reconcile + materialise + suggest
phai forecast refresh-installments  Layer 1 only: detect parcela chains and materialise
phai forecast reconcile      Match active forecasts to recent transactions (sets realizado)
phai forecast suggest        List detected recurring candidates awaiting accept/dismiss
phai forecast accept         Accept a proposed template and materialise next N months
phai forecast dismiss        Dismiss a proposed template so the detector skips it
phai forecast scenario       What-if: project balance with a hypothetical recurring commitment
phai rule upsert/list/inspect Classification rule management
phai account upsert          Create or update an account
phai budget upsert/list      Category budget management
phai serve [--port 8080]  Local web dashboard for forecast review (WebSocket API)
phai self check              Check for available updates
phai self update             Force-update to the latest release
```

</details>

## BigQuery setup

```bash
# 1. Create a GCP project + dataset (e.g. phai).
# 2. Create a service account with BigQuery Data Editor + Job User roles.
# 3. Download the JSON key.

phai auth setup \
  --backend bigquery \
  --actor-id $USER \
  --project-id your-gcp-project \
  --dataset-id phai \
  --service-account-path /path/to/service-account.json

phai admin migrate
```

Optional: use a Google Sheet as a category/human-field override source so your manual classifications survive across machines. See [docs/google-sheets-overrides.md](docs/google-sheets-overrides.md).

## Configuration

| Variable | Description |
|---|---|
| `FINANCE_OS_CONFIG_DIR` | Config directory. Default: `~/Library/Application Support/finance-os` (macOS) or `~/.config/finance-os` (Linux). |
| `FINANCE_OS_DATA_DIR` | Data directory (holds `finance-os.db` and `update-state.json`). Same defaults as above. |
| `FINANCE_OS_NO_AUTO_UPDATE` | Set to `1` to disable automatic update checks. |
| `PLUGGY_CLIENT_ID` / `PLUGGY_CLIENT_SECRET` | Pluggy API credentials. |

> The on-disk config/data paths and `FINANCE_OS_*` environment variables retain their current names so existing installs keep working. A migration to phai-named paths is tracked separately.

## Build from source

```bash
git clone https://github.com/phai-run/phai.git
cd phai
cargo build --release
./target/release/phai --version
```

Requires Rust 1.90+ (`rustup update stable`).

### cargo install

```bash
cargo install --git https://github.com/phai-run/phai.git --bin phai
```

## AI assistant integration

The `integrations/openclaw/` directory contains a shell wrapper + skill definition that exposes `phai` to an AI assistant. The pattern (wrapper + skill markdown) adapts to Claude Skills, OpenClaw, or any frame that exec's commands — phai stays LLM-neutral.

Agents should always invoke reports with `--raw` to get JSON instead of the human-friendly default.

## Architecture

```text
crates/
  phai-core/        Domain logic, storage trait, models, Pluggy client
  phai-cli/         CLI binary, report formatters, auto-update
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

## Links

- Repo — [github.com/phai-run/phai](https://github.com/phai-run/phai)
- Brand & design — [DESIGN.md](DESIGN.md)
- Getting started — [docs/GETTING-STARTED.md](docs/GETTING-STARTED.md)
- Architecture — [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Site — `phai.run` *(coming soon)*

## License

[MIT](LICENSE).
