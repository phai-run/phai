CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.forecast` (
  forecast_id STRING NOT NULL,
  due_date DATE NOT NULL,
  description STRING NOT NULL,
  amount NUMERIC NOT NULL,
  category_id STRING,
  account_id STRING,
  status STRING NOT NULL,
  recurrence STRING,
  actor_id STRING NOT NULL,
  idempotency_key STRING NOT NULL,
  metadata_json JSON,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

