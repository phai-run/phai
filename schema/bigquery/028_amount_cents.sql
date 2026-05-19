-- Migration 028: add amount_cents to transactions (BigQuery).
-- Mirror of schema/sqlite/027_amount_cents.sql.
-- BigQuery's NUMERIC is already decimal-safe; this column exists for
-- cross-backend query compatibility. Unlike SQLite's generated column,
-- BigQuery requires the column to be populated explicitly — the Rust
-- upsert and backfill handle this.

ALTER TABLE `{{project_id}}.{{dataset_id}}.transactions`
  ADD COLUMN IF NOT EXISTS amount_cents INT64;

UPDATE `{{project_id}}.{{dataset_id}}.transactions`
SET amount_cents = CAST(ROUND(amount * 100) AS INT64)
WHERE amount_cents IS NULL;
