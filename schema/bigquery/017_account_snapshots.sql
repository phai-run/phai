CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.account_snapshots` (
  snapshot_id       STRING  NOT NULL,
  account_id        STRING  NOT NULL,
  snapshot_date     DATE    NOT NULL,
  balance           NUMERIC,
  credit_limit      NUMERIC,
  currency_code     STRING,
  source            STRING  NOT NULL,
  actor_id          STRING  NOT NULL,
  idempotency_key   STRING  NOT NULL,
  metadata_json     JSON,
  created_at        TIMESTAMP NOT NULL
);
