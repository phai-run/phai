-- 041_plan_scenario.sql — ADR-0037.
-- Mirror of the SQLite version. See that file for design notes.
--
-- Idempotent: CREATE TABLE IF NOT EXISTS only.

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.plan_scenario` (
  scenario_id      STRING NOT NULL,
  name             STRING NOT NULL,
  description      STRING,
  status           STRING NOT NULL,
  promoted_at      TIMESTAMP,
  metadata_json    JSON,
  actor_id         STRING NOT NULL,
  idempotency_key  STRING NOT NULL,
  created_at       TIMESTAMP NOT NULL,
  updated_at       TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.plan_change` (
  change_id           STRING NOT NULL,
  scenario_id         STRING NOT NULL,
  kind                STRING NOT NULL,
  target_forecast_id  STRING,
  target_template_id  STRING,
  month               STRING,
  effective_from      STRING,
  amount              NUMERIC,
  months_count        INT64,
  description         STRING,
  category_id         STRING,
  account_id          STRING,
  status              STRING NOT NULL,
  payload_json        JSON,
  actor_id            STRING NOT NULL,
  idempotency_key     STRING NOT NULL,
  created_at          TIMESTAMP NOT NULL,
  updated_at          TIMESTAMP NOT NULL
);
