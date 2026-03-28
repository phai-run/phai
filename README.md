# Finance OS

A personal finance CLI and data pipeline built in Rust. Syncs bank transactions from [Pluggy](https://pluggy.ai), stores them in BigQuery (production) or SQLite (local development), and provides reporting views for budgeting, cashflow analysis, and forecast tracking.

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
| `FINANCE_OS_CONFIG_DIR` | Config directory (default: `~/.config/finance-os`) |
| `FINANCE_OS_DATA_DIR` | Data directory (default: `~/.local/share/finance-os`) |
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
finance report uncategorized Transactions needing categorization
finance tx upsert-manual    Add a manual transaction
finance tx categorize       Assign category to a transaction
finance tx set-context      Add context to a transaction
finance forecast upsert     Create or update a forecast entry
finance rule upsert         Create or update a classification rule
finance account upsert      Create or update an account
```

All report commands support `--json` for machine-readable output.

## Architecture

```
crates/
  finance-core/     Shared domain logic, storage trait, models
  finance-cli/      CLI application (clap-based)
schema/
  sqlite/           SQLite migration files
  bigquery/         BigQuery migration files (with template substitution)
integrations/
  ford/             Example workspace integration (skill + shell wrapper)
```

The `FinanceStore` trait abstracts over both backends. Migrations are embedded in the binary at compile time via `include_str!`.

## Testing

```bash
cargo test --workspace
```

E2E tests run against SQLite using temporary directories. No external services required.

## License

MIT
