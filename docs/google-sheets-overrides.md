# Google Sheets Category Overrides

Finance OS keeps shared classification logic in the database and shared codebase, but the actual sheet URL and private categorization workflow should stay outside the repository.

This repository exposes a single read layer, `v_transactions_effective`, that can be replaced in a private BigQuery runtime. In a private environment, you can rewire that view to read an external Google Sheet and override `category_id` plus human transaction fields without mutating the canonical transaction rows.

This override layer is independent from transaction splits. Split flows (`tx split ...`, `report split-candidates`, `report item-prices`) are BigQuery-only runtime features and should not be implemented as shared, user-specific sheet heuristics.

## Expected Sheet Shape

Create a tab such as `category_overrides` with this header row:

| transaction_id | category_id | description | merchant_name | purpose | updated_at | enabled |
|---|---|---|---|---|
| tx-123 | moradia:streaming | HBO Max annual plan | 2026-04-22T10:30:00Z | true |

Notes:

- `transaction_id` must match the Finance OS transaction ID already stored in BigQuery.
- `category_id` must use the canonical Finance OS category key (example: `alimentacao:mercado`).
- Human fields are optional; leave a column blank when the canonical value should stand.
- `updated_at` is optional but recommended. When multiple rows target the same transaction, the latest timestamp wins.
- `enabled` is optional. Empty values are treated as enabled.

## Private Setup

1. Share the sheet with the BigQuery service account configured in your local `config.toml`.
   The querying runtime must also request Google Drive read access; Finance OS does this automatically in the BigQuery backend.
2. Apply migrations so the dataset has the default `v_transactions_effective` and dependent views:

```bash
./target/release/finance-cli admin migrate
```

3. Replace `v_transactions_effective` with a Sheets-backed version:

```bash
python3 scripts/setup_google_sheets_overrides.py \
  --sheet-url "https://docs.google.com/spreadsheets/d/..." \
  --execute
```

By default the script reads:

- `project_id`
- `dataset_id`
- `service_account_path`

from `~/.config/finance-os/config.toml`.

## What Changes

After the script runs:

- `v_transactions_effective` reads transaction overrides from Google Sheets.
- `v_daily_pulse`, `v_monthly_spend`, `v_cashflow`, `v_forecast_vs_actual`, `v_card_summary`, and `v_uncategorized` consume the effective view created by migration `012_effective_transactions_view`.
- CLI reads that already depend on those views will reflect the edited categories without changing shared rules or migrations.

## Ford Split Flow (BigQuery-only)

For assistant/runtime operations tied to itemized transaction splits, use CLI commands directly (not ad-hoc sheet logic):

```bash
bash skills/finance-os/finance.sh tx split preview --transaction-id ID --payload split.json
bash skills/finance-os/finance.sh tx split apply --transaction-id ID --payload split.json
bash skills/finance-os/finance.sh tx split show --transaction-id ID
bash skills/finance-os/finance.sh tx split clear --transaction-id ID
bash skills/finance-os/finance.sh report split-candidates
bash skills/finance-os/finance.sh report item-prices --query "item"
```

Notes:

- Backend `local` should be treated as unsupported for these split flows.
- Keep personal merchant/item patterns in private runtime rules/config, never in shared repository migrations/docs/tests.
