CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.category_budgets` (
  budget_id STRING NOT NULL,
  category_id STRING NOT NULL,
  subcategory_id STRING,
  month_ref STRING,
  amount NUMERIC NOT NULL,
  alert_threshold_pct INT64 NOT NULL,
  actor_id STRING NOT NULL,
  idempotency_key STRING NOT NULL,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);
