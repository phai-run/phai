-- 034_forecast_template.sql — ADR-0016.
-- Mirror of the SQLite version. See that file for design notes.
--
-- Idempotent: CREATE TABLE IF NOT EXISTS + ALTER TABLE ADD COLUMN IF NOT EXISTS
-- so re-running the migration is safe.

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.forecast_template` (
  template_id        STRING NOT NULL,
  kind               STRING NOT NULL,
  description        STRING NOT NULL,
  merchant_pattern   STRING,
  category_id        STRING,
  account_id         STRING,
  amount             NUMERIC NOT NULL,
  amount_lower       NUMERIC,
  amount_upper       NUMERIC,
  cadence            STRING NOT NULL,
  next_due_day       INT64,
  start_date         DATE NOT NULL,
  end_date           DATE,
  remaining_count    INT64,
  source             STRING NOT NULL,
  confidence         FLOAT64,
  status             STRING NOT NULL,
  metadata_json      JSON,
  actor_id           STRING NOT NULL,
  idempotency_key    STRING NOT NULL,
  created_at         TIMESTAMP NOT NULL,
  updated_at         TIMESTAMP NOT NULL
);

ALTER TABLE `{{project_id}}.{{dataset_id}}.forecast`
  ADD COLUMN IF NOT EXISTS template_id STRING;

ALTER TABLE `{{project_id}}.{{dataset_id}}.forecast`
  ADD COLUMN IF NOT EXISTS realized_transaction_id STRING;

ALTER TABLE `{{project_id}}.{{dataset_id}}.forecast`
  ADD COLUMN IF NOT EXISTS realized_at TIMESTAMP;
