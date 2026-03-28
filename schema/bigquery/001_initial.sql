CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.schema_versions` (
  version STRING NOT NULL,
  applied_at TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.accounts` (
  account_id STRING NOT NULL,
  owner STRING NOT NULL,
  account_type STRING NOT NULL,
  bank STRING NOT NULL,
  label STRING NOT NULL,
  pluggy_account_id STRING,
  pluggy_item_id STRING,
  status STRING NOT NULL,
  actor_id STRING NOT NULL,
  idempotency_key STRING NOT NULL,
  metadata_json JSON,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.categories` (
  category_id STRING NOT NULL,
  name STRING NOT NULL,
  parent_category_id STRING,
  metadata_json JSON,
  actor_id STRING NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.rules` (
  rule_id STRING NOT NULL,
  body STRING NOT NULL,
  status STRING NOT NULL,
  actor_id STRING NOT NULL,
  idempotency_key STRING NOT NULL,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.transactions` (
  transaction_id STRING NOT NULL,
  account_id STRING,
  transaction_date DATE NOT NULL,
  description STRING NOT NULL,
  amount NUMERIC NOT NULL,
  tx_type STRING NOT NULL,
  category_id STRING,
  category_source STRING NOT NULL,
  context STRING,
  payment_status STRING NOT NULL,
  source STRING NOT NULL,
  actor_id STRING NOT NULL,
  idempotency_key STRING NOT NULL,
  metadata_json JSON,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.audit_log` (
  event_id STRING NOT NULL,
  entity_type STRING NOT NULL,
  entity_id STRING NOT NULL,
  action STRING NOT NULL,
  actor_id STRING NOT NULL,
  event_timestamp TIMESTAMP NOT NULL,
  idempotency_key STRING NOT NULL,
  diff_json JSON NOT NULL
);

