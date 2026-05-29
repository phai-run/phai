#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import sys
import time
import tomllib
from pathlib import Path

from google.auth.transport.requests import AuthorizedSession
from google.oauth2 import service_account

BIGQUERY_SCOPE = "https://www.googleapis.com/auth/bigquery"
DRIVE_READONLY_SCOPE = "https://www.googleapis.com/auth/drive.readonly"
MAX_POLL_ATTEMPTS = 30
POLL_INTERVAL_SECONDS = 1


def default_config_path() -> Path:
    config_root = os.environ.get("FINANCE_OS_CONFIG_DIR")
    if config_root:
        return Path(config_root) / "config.toml"
    return Path.home() / ".config" / "finance-os" / "config.toml"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Create a Google Sheets-backed override layer for phai BigQuery reads.",
    )
    parser.add_argument("--sheet-url", required=True, help="Google Sheets URL")
    parser.add_argument(
        "--sheet-range",
        default="category_overrides!A:E",
        help="Sheet tab/range used as the external table source",
    )
    parser.add_argument(
        "--external-table",
        default="sheet_category_overrides_raw",
        help="External table name inside the dataset",
    )
    parser.add_argument(
        "--config-file",
        type=Path,
        default=default_config_path(),
        help="phai config.toml path",
    )
    parser.add_argument("--project-id", help="Override project_id from config.toml")
    parser.add_argument("--dataset-id", help="Override dataset_id from config.toml")
    parser.add_argument(
        "--service-account-path",
        type=Path,
        help="Override service_account_path from config.toml",
    )
    parser.add_argument(
        "--execute",
        action="store_true",
        help="Run the SQL against BigQuery instead of only printing it",
    )
    return parser.parse_args()


def load_config(path: Path) -> dict:
    try:
        with path.open("rb") as fh:
            return tomllib.load(fh)
    except FileNotFoundError as exc:
        raise SystemExit(f"config.toml not found: {path}") from exc


def required_arg(cli_value: str | None, config: dict, key: str) -> str:
    value = cli_value or config.get(key)
    if not value:
        raise SystemExit(f"missing `{key}`; pass --{key.replace('_', '-')} or set it in config.toml")
    return str(value)


def sql_string(value: str) -> str:
    return "'" + value.replace("\\", "\\\\").replace("'", "\\'") + "'"


def qualified_table(project_id: str, dataset_id: str, table_name: str) -> str:
    return f"`{project_id}.{dataset_id}.{table_name}`"


def build_sql(
    project_id: str,
    dataset_id: str,
    external_table: str,
    sheet_url: str,
    sheet_range: str,
) -> str:
    external = qualified_table(project_id, dataset_id, external_table)
    transactions = qualified_table(project_id, dataset_id, "transactions")
    transaction_splits = qualified_table(project_id, dataset_id, "transaction_splits")
    transaction_split_lines = qualified_table(project_id, dataset_id, "transaction_split_lines")
    effective = qualified_table(project_id, dataset_id, "v_transactions_effective")
    return f"""
CREATE OR REPLACE EXTERNAL TABLE {external} (
  transaction_id STRING,
  category_id STRING,
  context STRING,
  updated_at STRING,
  enabled STRING
)
OPTIONS (
  format = 'GOOGLE_SHEETS',
  skip_leading_rows = 1,
  uris = [{sql_string(sheet_url)}],
  sheet_range = {sql_string(sheet_range)}
);

CREATE OR REPLACE VIEW {effective} AS
WITH split_candidates AS (
  SELECT
    s.split_id,
    s.parent_transaction_id,
    ROW_NUMBER() OVER (
      PARTITION BY s.parent_transaction_id
      ORDER BY
        CASE WHEN s.status = 'confirmed' THEN 0 ELSE 1 END,
        s.updated_at DESC,
        s.created_at DESC,
        s.split_id DESC
    ) AS row_priority
  FROM {transaction_splits} s
  WHERE s.status IN ('active', 'confirmed')
    AND EXISTS (
      SELECT 1
      FROM {transaction_split_lines} sl
      WHERE sl.split_id = s.split_id
        AND sl.status IN ('active', 'confirmed')
    )
),
selected_splits AS (
  SELECT split_id, parent_transaction_id
  FROM split_candidates
  WHERE row_priority = 1
),
base_transactions AS (
  SELECT
    t.transaction_id,
    t.account_id,
    t.transaction_date,
    t.description,
    t.amount,
    t.tx_type,
    t.category_id,
    t.category_source,
    t.context,
    t.payment_status,
    t.source,
    t.actor_id,
    t.idempotency_key,
    t.metadata_json,
    t.created_at,
    t.updated_at
  FROM {transactions} t
  LEFT JOIN selected_splits ss
    ON ss.parent_transaction_id = t.transaction_id
  WHERE ss.split_id IS NULL

  UNION ALL

  SELECT
    sl.split_line_id AS transaction_id,
    t.account_id,
    t.transaction_date,
    COALESCE(NULLIF(sl.description, ''), t.description) AS description,
    sl.amount,
    CASE
      WHEN sl.amount > 0 THEN 'credit'
      WHEN sl.amount < 0 THEN 'debit'
      ELSE t.tx_type
    END AS tx_type,
    COALESCE(NULLIF(sl.category_id, ''), t.category_id) AS category_id,
    COALESCE(NULLIF(sl.category_source, ''), 'split') AS category_source,
    COALESCE(NULLIF(sl.context, ''), t.context) AS context,
    t.payment_status,
    t.source,
    COALESCE(NULLIF(sl.actor_id, ''), t.actor_id) AS actor_id,
    COALESCE(NULLIF(sl.idempotency_key, ''), t.idempotency_key) AS idempotency_key,
    JSON_OBJECT(
      'effectiveKind', 'split',
      'parentTransactionId', t.transaction_id,
      'splitId', sl.split_id,
      'splitLineId', sl.split_line_id
    ) AS metadata_json,
    LEAST(t.created_at, sl.created_at) AS created_at,
    GREATEST(t.updated_at, sl.updated_at) AS updated_at
  FROM selected_splits ss
  JOIN {transactions} t
    ON t.transaction_id = ss.parent_transaction_id
  JOIN {transaction_split_lines} sl
    ON sl.split_id = ss.split_id
   AND sl.parent_transaction_id = ss.parent_transaction_id
  WHERE sl.status IN ('active', 'confirmed')
),
sheet_overrides AS (
  SELECT
    TRIM(transaction_id) AS transaction_id,
    NULLIF(TRIM(category_id), '') AS category_id,
    NULLIF(TRIM(context), '') AS context,
    COALESCE(
      SAFE_CAST(NULLIF(TRIM(updated_at), '') AS TIMESTAMP),
      TIMESTAMP '1970-01-01 00:00:00 UTC'
    ) AS override_updated_at
  FROM {external}
  WHERE TRIM(transaction_id) <> ''
    AND (
      NULLIF(TRIM(category_id), '') IS NOT NULL
      OR NULLIF(TRIM(context), '') IS NOT NULL
    )
    AND COALESCE(NULLIF(LOWER(TRIM(enabled)), ''), 'true') NOT IN ('false', '0', 'no', 'n')
  QUALIFY ROW_NUMBER() OVER (
    PARTITION BY TRIM(transaction_id)
    ORDER BY override_updated_at DESC, transaction_id DESC
  ) = 1
)
SELECT
  t.transaction_id,
  t.account_id,
  t.transaction_date,
  t.description,
  t.amount,
  t.tx_type,
  COALESCE(o.category_id, t.category_id) AS category_id,
  CASE
    WHEN o.transaction_id IS NOT NULL AND o.category_id IS NOT NULL THEN 'sheet'
    ELSE t.category_source
  END AS category_source,
  COALESCE(o.context, t.context) AS context,
  t.payment_status,
  t.source,
  t.actor_id,
  t.idempotency_key,
  t.metadata_json,
  t.created_at,
  GREATEST(t.updated_at, COALESCE(o.override_updated_at, t.updated_at)) AS updated_at,
  CASE
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'receitas%'
      OR COALESCE(o.category_id, t.category_id) = 'salario'
      OR t.amount > 0 THEN '💰'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'transfer%' THEN '🔁'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'assinaturas%' THEN '🔂'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'moradia%'
      OR COALESCE(o.category_id, t.category_id) LIKE 'casa%' THEN '🏠'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'alimentacao%' THEN '🍽️'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'saude%' THEN '🩺'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'transporte%'
      OR COALESCE(o.category_id, t.category_id) LIKE 'mobilidade%' THEN '🚗'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'educacao%' THEN '📚'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'lazer%' THEN '🎉'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'investimentos%' THEN '📈'
    WHEN COALESCE(o.category_id, t.category_id) LIKE 'financeiro%' THEN '🧾'
    WHEN COALESCE(o.category_id, t.category_id) IS NULL THEN '❓'
    ELSE '💸'
  END AS display_emoji,
  CONCAT(
    CASE
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'receitas%'
        OR COALESCE(o.category_id, t.category_id) = 'salario'
        OR t.amount > 0 THEN '💰'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'transfer%' THEN '🔁'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'assinaturas%' THEN '🔂'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'moradia%'
        OR COALESCE(o.category_id, t.category_id) LIKE 'casa%' THEN '🏠'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'alimentacao%' THEN '🍽️'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'saude%' THEN '🩺'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'transporte%'
        OR COALESCE(o.category_id, t.category_id) LIKE 'mobilidade%' THEN '🚗'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'educacao%' THEN '📚'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'lazer%' THEN '🎉'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'investimentos%' THEN '📈'
      WHEN COALESCE(o.category_id, t.category_id) LIKE 'financeiro%' THEN '🧾'
      WHEN COALESCE(o.category_id, t.category_id) IS NULL THEN '❓'
      ELSE '💸'
    END,
    ' ',
    TRIM(COALESCE(o.context, t.context, t.description))
  ) AS display_label,
  CASE
    WHEN COALESCE(o.category_id, t.category_id) IS NULL OR TRIM(COALESCE(o.category_id, t.category_id)) = '' THEN '❓ sem categoria'
    ELSE CONCAT(
      CASE
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'receitas%'
          OR COALESCE(o.category_id, t.category_id) = 'salario'
          OR t.amount > 0 THEN '💰'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'transfer%' THEN '🔁'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'assinaturas%' THEN '🔂'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'moradia%'
          OR COALESCE(o.category_id, t.category_id) LIKE 'casa%' THEN '🏠'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'alimentacao%' THEN '🍽️'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'saude%' THEN '🩺'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'transporte%'
          OR COALESCE(o.category_id, t.category_id) LIKE 'mobilidade%' THEN '🚗'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'educacao%' THEN '📚'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'lazer%' THEN '🎉'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'investimentos%' THEN '📈'
        WHEN COALESCE(o.category_id, t.category_id) LIKE 'financeiro%' THEN '🧾'
        ELSE '💸'
      END,
      ' ',
      REGEXP_REPLACE(REPLACE(COALESCE(o.category_id, t.category_id), ':', ' > '), '-', ' ')
    )
  END AS category_display
FROM base_transactions t
LEFT JOIN sheet_overrides o
  ON o.transaction_id = t.transaction_id;
""".strip()


def run_bigquery_sql(project_id: str, service_account_path: Path, sql: str) -> dict:
    creds = service_account.Credentials.from_service_account_file(
        service_account_path,
        scopes=[BIGQUERY_SCOPE, DRIVE_READONLY_SCOPE],
    )
    session = AuthorizedSession(creds)
    base_url = f"https://bigquery.googleapis.com/bigquery/v2/projects/{project_id}"
    response = session.post(
        f"{base_url}/queries",
        json={"query": sql, "useLegacySql": False},
        timeout=120,
    )
    response.raise_for_status()
    payload = response.json()
    if payload.get("jobComplete"):
        return payload
    job_id = payload.get("jobReference", {}).get("jobId")
    if not job_id:
        raise RuntimeError("BigQuery returned an incomplete job without a job ID")
    for _ in range(MAX_POLL_ATTEMPTS):
        time.sleep(POLL_INTERVAL_SECONDS)
        poll = session.get(f"{base_url}/queries/{job_id}", timeout=120)
        poll.raise_for_status()
        payload = poll.json()
        if payload.get("jobComplete"):
            return payload
    raise RuntimeError(f"BigQuery job {job_id} did not complete after polling")


def main() -> int:
    args = parse_args()
    config = load_config(args.config_file)
    project_id = required_arg(args.project_id, config, "project_id")
    dataset_id = required_arg(args.dataset_id, config, "dataset_id")
    service_account = Path(
        required_arg(
            str(args.service_account_path) if args.service_account_path else None,
            config,
            "service_account_path",
        )
    )
    sql = build_sql(
        project_id=project_id,
        dataset_id=dataset_id,
        external_table=args.external_table,
        sheet_url=args.sheet_url,
        sheet_range=args.sheet_range,
    )

    if not args.execute:
        print(sql)
        return 0

    payload = run_bigquery_sql(project_id, service_account, sql)
    print(
        json.dumps(
            {
                "projectId": project_id,
                "datasetId": dataset_id,
                "externalTable": args.external_table,
                "sheetRange": args.sheet_range,
                "jobReference": payload.get("jobReference", {}),
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
