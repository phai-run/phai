# Finance OS

[![CI](https://github.com/feliperun/finance-os/actions/workflows/ci.yml/badge.svg)](https://github.com/feliperun/finance-os/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A personal finance CLI and data pipeline built in Rust. Syncs bank transactions from [Pluggy](https://pluggy.ai), stores them in BigQuery (production) or SQLite (local development), and provides reporting views for budgeting, cashflow analysis, and forecast tracking.

> **Brazilian banking focus:** Pluggy is a Brazilian open-finance aggregator. The data model and reporting are designed for Brazilian accounts (checking, credit cards, investments), but the architecture is backend-agnostic and can be adapted for other providers.

## Install

### Download a release (macOS ARM)

```bash
curl -fsSL https://github.com/feliperun/finance-os/releases/latest/download/finance-cli-aarch64-apple-darwin.tar.gz | tar xz
chmod +x finance-cli
./finance-cli --version
```

To make it available as `finance-cli` from any directory, move it into a PATH directory (the auto-update will overwrite this file in place when a new release is published):

```bash
mv finance-cli ~/.local/bin/   # or /usr/local/bin, or any directory in your $PATH
finance-cli --version
```

> Note: if you previously installed an older version under a different name (e.g. `finance`), make sure your shell resolves to the new binary — `which finance-cli` should point to the location you just moved it to.

### Build from source

```bash
git clone https://github.com/feliperun/finance-os.git
cd finance-os
cargo build --release
./target/release/finance-cli --version
```

The CLI checks for updates automatically (at most once every 24 hours). Run `finance self check` to see if a newer release is available.

## Features

- **Dual-backend storage** -- BigQuery for production, SQLite for local development
- **Bank sync** via Pluggy API with automatic pagination and idempotent imports
- **Reporting views** -- daily pulse, monthly spend, cashflow, forecast vs actual, card summary, uncategorized transactions
- **Audit trail** -- append-only event log for every write operation
- **Financial precision** -- all amounts use `rust_decimal::Decimal`, never floating-point
- **Idempotent operations** -- safe to re-run syncs and imports without duplicating data
- **Legacy import** -- migrate from CSV-based finance tracking

## Prerequisites

- Rust 1.90+ (`rustup update stable`)
- A [Pluggy](https://pluggy.ai) account with API credentials
- For BigQuery backend: a GCP project with BigQuery API enabled and a service account key

## Quick Start (Local Backend)

```bash
# Build
cargo build --release

# Configure
./target/release/finance-cli auth setup \
  --backend local \
  --actor-id your-name

# Run migrations
./target/release/finance-cli admin migrate

# Sync from Pluggy (using a fixture for testing)
./target/release/finance-cli sync pluggy \
  --pluggy-config examples/pluggy-config.example.json \
  --fixture examples/pluggy_fixture.json

# View recent transactions
./target/release/finance-cli report daily-pulse --days 30
```

## BigQuery Backend Setup

1. Create a GCP project and enable the BigQuery API
2. Create a dataset (e.g. `finance_os`)
3. Create a service account with **BigQuery Data Editor** and **BigQuery Job User** roles
4. Download the service account JSON key

```bash
./target/release/finance-cli auth setup \
  --backend bigquery \
  --actor-id your-name \
  --project-id your-gcp-project \
  --dataset-id finance_os \
  --service-account-path /path/to/service-account.json

./target/release/finance-cli admin migrate
```

Optional private workflow: use a Google Sheet as a category/context override source for BigQuery reads. See [docs/google-sheets-overrides.md](docs/google-sheets-overrides.md).

## Pluggy Configuration

Create a `pluggy-config.json` mapping your internal account IDs to Pluggy account IDs (see `examples/pluggy-config.example.json`):

```json
{
  "syncStartDate": "2025-01-01",
  "accounts": [
    { "id": "my-checking", "pluggyAccountId": "uuid-from-pluggy" }
  ]
}
```

Set your Pluggy credentials as environment variables:

```bash
export PLUGGY_CLIENT_ID=your-client-id
export PLUGGY_CLIENT_SECRET=your-client-secret
```

## Environment Variables

| Variable | Description |
|---|---|
| `FINANCE_OS_CONFIG_DIR` | Config directory (default follows platform conventions: `~/.config/finance-os` on Linux, `~/Library/Application Support/finance-os` on macOS) |
| `FINANCE_OS_DATA_DIR` | Data directory (default follows platform conventions: `~/.local/share/finance-os` on Linux, `~/Library/Application Support/finance-os` on macOS). Houses `finance-os.db` (SQLite backend) and `update-state.json` (auto-update throttle). |
| `FINANCE_OS_NO_AUTO_UPDATE` | Set to `1` to disable automatic update checks |
| `PLUGGY_CLIENT_ID` | Pluggy API client ID |
| `PLUGGY_CLIENT_SECRET` | Pluggy API client secret |

## CLI Commands

```
finance auth setup          Configure backend and credentials
finance admin migrate       Apply pending database migrations
finance admin import-legacy Import from legacy CSV files
finance sync pluggy         Sync transactions from Pluggy
finance report daily-pulse  Recent transactions
finance report monthly-spend Monthly expenses by category
finance report cashflow     Monthly income/expenses/net
finance report forecast-vs-actual  Budget vs actual comparison
finance report card-summary Credit card statement summary
finance report card-closed-insights Closed-bill insights (categories, recurring, subscriptions, installments)
finance report ofx-consistency Transaction-by-transaction consistency check against an OFX file
finance report uncategorized Transactions needing categorization
finance tx upsert-manual    Add a manual transaction
finance tx categorize       Assign category to a transaction
finance tx set-context      Add context to a transaction
finance forecast upsert     Create or update a forecast entry
finance rule upsert         Create or update a classification rule
finance account upsert      Create or update an account
finance self check          Check for available updates
finance self update         Download and install the latest release
```

All report commands support `--json` for machine-readable output.

`finance sync pluggy` supports summary outputs for automation:
- `--json-summary` for structured integrations
- `--notify-summary` for human-readable notification text generated by Finance OS

Finance OS also standardizes user-facing transaction naming through `display_label` (context-first) and emoji category hints in report outputs. See `FINANCE_OS.md` for cross-agent behavior conventions.

Rules are stored in the database and applied during Pluggy sync. Supported rule formats:

```bash
finance rule upsert \
  --rule-id bill_payment \
  --body 'if description contains "pagamento de fatura" then category credit-card-payment'

finance rule upsert \
  --rule-id own_transfer \
  --body 'if description contains "pix no credito" then category transfer-internal'
```

Use database-backed rules or private config for user-specific patterns. Do not commit personal classification rules to shared code or migrations.

## Private Local Setup

For day-to-day use, keep private files outside the open-source repo and mount them into a local gitignored overlay.

Recommended layout:

```text
finance-os/                  # open-source repo
  .private/                  # gitignored local overlay
    runtime -> ~/finance-os-configs/runtime/<runtime-name>
    pluggy.env -> ~/finance-os-configs/pluggy/pluggy.env
    pluggy-config.json -> /private/path/to/pluggy-config.json
    contas.csv -> /private/path/to/contas.csv
```

Create the local links with:

```bash
scripts/setup-private-links.sh \
  --configs-root "$HOME/finance-os-configs" \
  --runtime your-runtime \
  --pluggy-config "/private/path/to/pluggy-config.json" \
  --accounts-csv "/private/path/to/contas.csv"
```

Then run sync through the overlay:

```bash
scripts/local-sync.sh --from 2026-03-01 --to 2026-04-01
```

This keeps secrets, service-account paths, Pluggy credentials, and user-specific rules out of the shared repository.

## Architecture

```
crates/
  finance-core/     Shared domain logic, storage trait, models
  finance-cli/      CLI application (clap-based)
schema/
  sqlite/           SQLite migration files
  bigquery/         BigQuery migration files (with template substitution)
integrations/
  openclaw/         Example AI assistant integration (skill + shell wrapper)
```

The `FinanceStore` trait abstracts over both backends. Migrations are embedded in the binary at compile time via `include_str!`.

## AI Assistant Integration

The `integrations/openclaw/` directory contains an example integration for [OpenClaw](https://openclaw.io), an AI assistant framework. It shows how to expose `finance-cli` commands as a skill that an AI agent can invoke — syncing transactions, categorizing spending, and generating reports through natural language.

The pattern (a shell wrapper + a skill definition file) can be adapted for other AI assistant frameworks.

## Testing

```bash
cargo test --workspace
```

E2E tests run against SQLite using temporary directories. No external services required.

## Release Process

Releases are managed by Release Please on pushes to `main`.

- Use Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`).
- Use `!` or a `BREAKING CHANGE:` footer for breaking changes.
- Release Please opens a release PR that updates `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, and tags the release after merge.
- CI builds and attaches a `finance-cli-aarch64-apple-darwin.tar.gz` + `.sha256` to each release.

### Auto-Update

The CLI checks for updates at most once every 24 hours before running commands.
This check is **silent** — it only prints to stderr when a download is in progress or
when an error occurs. It never blocks your command.

**Skip the auto-check:**

```bash
FINANCE_OS_NO_AUTO_UPDATE=1 finance report daily-pulse
```

Or disable it permanently in your shell profile:

```bash
export FINANCE_OS_NO_AUTO_UPDATE=1
```

**Manual update commands:**

```bash
finance self check          # Report current vs latest version
finance self update         # Download and install the latest release
```

The updater replaces the running binary in-place on macOS (atomic `rename(2)` +
`execv`). After a successful update your original command re-runs with the new
binary automatically.

**Runtime binary layout:** The updater discovers the current executable via
`std::env::current_exe()`. The recommended path for OpenClaw wrapper users is:

```text
~/.local/share/finance-os/runtime/bin/finance-cli
```

## Contributing

Contributions are welcome. Please open an issue before submitting large changes so we can discuss the approach. For bug fixes and small improvements, a pull request is sufficient.

- Keep new commands idempotent where possible
- Add a migration file for any schema changes (both `schema/sqlite/` and `schema/bigquery/`)
- E2E tests against the SQLite backend are preferred over unit tests that mock the store

## License

MIT
