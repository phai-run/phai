-- 034_forecast_template.sql — ADR-0016.
--
-- `forecast_template` carries the *rule* that generates forecast instances
-- (one row per recurring expectation: an installment chain, a detected
-- subscription, a fixed monthly bill, or a per-category envelope).
-- The existing `forecast` table stays as the *instance* table consumed by
-- the chart / reports; two new optional columns link each instance back to
-- its template and to the realized transaction (when reconciled).
--
-- Idempotent. No data is migrated — pre-existing manual forecasts simply
-- have NULL template_id and continue to behave as one-shot entries.

CREATE TABLE IF NOT EXISTS forecast_template (
  template_id        TEXT PRIMARY KEY,
  kind               TEXT NOT NULL,            -- 'installment'|'subscription'|'fixed'|'envelope'
  description        TEXT NOT NULL,
  merchant_pattern   TEXT,
  category_id        TEXT,
  account_id         TEXT,
  amount             TEXT NOT NULL,            -- signed magnitude (positive=inflow)
  amount_lower       TEXT,
  amount_upper       TEXT,
  cadence            TEXT NOT NULL,            -- 'monthly'|'weekly'|'one-shot'|'card-cycle'
  next_due_day       INTEGER,
  start_date         TEXT NOT NULL,
  end_date           TEXT,
  remaining_count    INTEGER,
  source             TEXT NOT NULL,            -- 'detected'|'manual'
  confidence         REAL,
  status             TEXT NOT NULL,            -- 'ativo'|'pausado'|'descartado'
  metadata_json      TEXT NOT NULL DEFAULT '{}',
  actor_id           TEXT NOT NULL,
  idempotency_key    TEXT NOT NULL UNIQUE,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_forecast_template_kind_status
  ON forecast_template(kind, status);

CREATE INDEX IF NOT EXISTS idx_forecast_template_account
  ON forecast_template(account_id);

-- Forecast instances gain a link to their generator template and to the
-- realised transaction that closed them. Both are nullable so existing
-- rows are unaffected.
ALTER TABLE forecast ADD COLUMN template_id TEXT;
ALTER TABLE forecast ADD COLUMN realized_transaction_id TEXT;
ALTER TABLE forecast ADD COLUMN realized_at TEXT;

CREATE INDEX IF NOT EXISTS idx_forecast_template_id
  ON forecast(template_id);
CREATE INDEX IF NOT EXISTS idx_forecast_realized_transaction_id
  ON forecast(realized_transaction_id);
