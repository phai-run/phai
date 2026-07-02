-- 041_plan_scenario.sql — ADR-0037.
--
-- Named what-if planning scenarios. A scenario is a set of typed deltas
-- (`plan_change`) layered over the live forecast baseline at read time —
-- nothing is copied, so the projection never goes stale. Promotion turns
-- the deltas into real forecast/template writes.
-- Generic infrastructure only — rows are user data; the schema carries no
-- personal counterparties or labels (AGENTS §1).
--
-- Idempotent.

CREATE TABLE IF NOT EXISTS plan_scenario (
  scenario_id      TEXT PRIMARY KEY,
  name             TEXT NOT NULL,
  description      TEXT,
  status           TEXT NOT NULL,            -- 'ativo'|'arquivado'|'promovido'
  promoted_at      TEXT,
  metadata_json    TEXT NOT NULL DEFAULT '{}',
  actor_id         TEXT NOT NULL,
  idempotency_key  TEXT NOT NULL UNIQUE,
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS plan_change (
  change_id           TEXT PRIMARY KEY,
  scenario_id         TEXT NOT NULL,
  kind                TEXT NOT NULL,         -- 'add_one_shot'|'adjust_amount'|'skip_forecast'|'end_template'|'hypothetical_installment'
  target_forecast_id  TEXT,                  -- adjust_amount | skip_forecast
  target_template_id  TEXT,                  -- end_template
  month               TEXT,                  -- 'YYYY-MM' (add_one_shot)
  effective_from      TEXT,                  -- 'YYYY-MM' (end_template | hypothetical_installment)
  amount              TEXT,                  -- signed decimal-as-string (positive = inflow)
  months_count        INTEGER,               -- hypothetical_installment
  description         TEXT,
  category_id         TEXT,
  account_id          TEXT,
  status              TEXT NOT NULL,         -- 'ativo'|'orfao'|'aplicado'
  payload_json        TEXT NOT NULL DEFAULT '{}',
  actor_id            TEXT NOT NULL,
  idempotency_key     TEXT NOT NULL UNIQUE,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_plan_change_scenario
  ON plan_change(scenario_id, status);
