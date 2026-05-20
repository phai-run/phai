-- Migration 033: transaction anatomy redesign.
--
-- Split the old overloaded `description`/`context` pair into:
--   raw_description   original bank/Pluggy text, queryable and stable
--   description       short human description of what was bought
--   merchant_name     cleaned merchant name
--   purpose           optional human intent/context
--   classifier_trace  technical trace from rules/enrichment
--
-- SQLite cannot drop NOT NULL from `description`, so this migration rebuilds
-- the transactions table and then recreates the semantic reporting views.

DROP VIEW IF EXISTS v_card_open_now;
DROP VIEW IF EXISTS v_card_summary;
DROP VIEW IF EXISTS v_forecast_vs_actual;
DROP VIEW IF EXISTS v_cashflow;
DROP VIEW IF EXISTS v_monthly_spend;
DROP VIEW IF EXISTS v_daily_pulse;
DROP VIEW IF EXISTS v_uncategorized;
DROP VIEW IF EXISTS v_transactions_reportable;
DROP VIEW IF EXISTS v_transactions_effective;

CREATE TABLE IF NOT EXISTS transactions_new (
  transaction_id TEXT PRIMARY KEY,
  account_id TEXT,
  transaction_date TEXT NOT NULL,
  raw_description TEXT NOT NULL DEFAULT '',
  description TEXT,
  merchant_name TEXT,
  purpose TEXT,
  amount TEXT NOT NULL,
  amount_cents INTEGER GENERATED ALWAYS AS (CAST(ROUND(amount * 100) AS INTEGER)) VIRTUAL,
  tx_type TEXT NOT NULL,
  category_id TEXT,
  category_source TEXT NOT NULL,
  context TEXT,
  classifier_trace TEXT,
  payment_status TEXT NOT NULL,
  source TEXT NOT NULL,
  actor_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  enrichment_attempted_at TEXT
);

INSERT OR IGNORE INTO transactions_new (
  transaction_id,
  account_id,
  transaction_date,
  raw_description,
  description,
  merchant_name,
  purpose,
  amount,
  tx_type,
  category_id,
  category_source,
  context,
  classifier_trace,
  payment_status,
  source,
  actor_id,
  idempotency_key,
  metadata_json,
  created_at,
  updated_at,
  enrichment_attempted_at
)
SELECT
  transaction_id,
  account_id,
  transaction_date,
  COALESCE(NULLIF(description, ''), ''),
  CASE
    WHEN source = 'pluggy' THEN NULL
    ELSE description
  END,
  NULL,
  NULL,
  amount,
  tx_type,
  category_id,
  category_source,
  context,
  context,
  payment_status,
  source,
  actor_id,
  idempotency_key,
  metadata_json,
  created_at,
  updated_at,
  enrichment_attempted_at
FROM transactions;

DROP TABLE IF EXISTS transactions;
ALTER TABLE transactions_new RENAME TO transactions;

CREATE INDEX IF NOT EXISTS idx_transactions_date ON transactions(transaction_date);
CREATE INDEX IF NOT EXISTS idx_transactions_account ON transactions(account_id);
CREATE INDEX IF NOT EXISTS idx_transactions_category ON transactions(category_id);
CREATE INDEX IF NOT EXISTS idx_transactions_raw_description ON transactions(raw_description);

CREATE VIEW v_transactions_effective AS
SELECT
  transaction_id,
  account_id,
  transaction_date,
  raw_description,
  description,
  merchant_name,
  purpose,
  amount,
  amount_cents,
  tx_type,
  category_id,
  category_source,
  context,
  classifier_trace,
  payment_status,
  source,
  actor_id,
  idempotency_key,
  metadata_json,
  created_at,
  updated_at,
  enrichment_attempted_at,
  CASE
    WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
    WHEN category_id LIKE 'transfer%' THEN '🔁'
    WHEN category_id LIKE 'assinaturas%' THEN '🔂'
    WHEN category_id LIKE 'moradia%' OR category_id LIKE 'casa%' THEN '🏠'
    WHEN category_id LIKE 'alimentacao%' THEN '🍽️'
    WHEN category_id LIKE 'saude%' THEN '🩺'
    WHEN category_id LIKE 'transporte%' OR category_id LIKE 'mobilidade%' THEN '🚗'
    WHEN category_id LIKE 'educacao%' THEN '📚'
    WHEN category_id LIKE 'lazer%' THEN '🎉'
    WHEN category_id LIKE 'investimentos%' THEN '📈'
    WHEN category_id LIKE 'financeiro%' THEN '🧾'
    WHEN category_id IS NULL THEN '❓'
    ELSE '💸'
  END AS display_emoji,
  (
    CASE
      WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
      WHEN category_id LIKE 'transfer%' THEN '🔁'
      WHEN category_id LIKE 'assinaturas%' THEN '🔂'
      WHEN category_id LIKE 'moradia%' OR category_id LIKE 'casa%' THEN '🏠'
      WHEN category_id LIKE 'alimentacao%' THEN '🍽️'
      WHEN category_id LIKE 'saude%' THEN '🩺'
      WHEN category_id LIKE 'transporte%' OR category_id LIKE 'mobilidade%' THEN '🚗'
      WHEN category_id LIKE 'educacao%' THEN '📚'
      WHEN category_id LIKE 'lazer%' THEN '🎉'
      WHEN category_id LIKE 'investimentos%' THEN '📈'
      WHEN category_id LIKE 'financeiro%' THEN '🧾'
      WHEN category_id IS NULL THEN '❓'
      ELSE '💸'
    END
  ) || ' ' || TRIM(COALESCE(NULLIF(description, ''), NULLIF(merchant_name, ''), raw_description)) AS display_label,
  CASE
    WHEN category_id IS NULL OR TRIM(category_id) = '' THEN '❓ sem categoria'
    ELSE
      (
        CASE
          WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
          WHEN category_id LIKE 'transfer%' THEN '🔁'
          WHEN category_id LIKE 'assinaturas%' THEN '🔂'
          WHEN category_id LIKE 'moradia%' OR category_id LIKE 'casa%' THEN '🏠'
          WHEN category_id LIKE 'alimentacao%' THEN '🍽️'
          WHEN category_id LIKE 'saude%' THEN '🩺'
          WHEN category_id LIKE 'transporte%' OR category_id LIKE 'mobilidade%' THEN '🚗'
          WHEN category_id LIKE 'educacao%' THEN '📚'
          WHEN category_id LIKE 'lazer%' THEN '🎉'
          WHEN category_id LIKE 'investimentos%' THEN '📈'
          WHEN category_id LIKE 'financeiro%' THEN '🧾'
          ELSE '💸'
        END
      ) || ' ' || REPLACE(REPLACE(category_id, ':', ' > '), '-', ' ')
  END AS category_display
FROM transactions;

CREATE VIEW v_transactions_reportable AS
SELECT
  t.transaction_id,
  t.account_id,
  t.transaction_date,
  t.raw_description,
  t.description,
  t.merchant_name,
  t.purpose,
  t.amount,
  t.amount_cents,
  t.tx_type,
  t.category_id,
  t.category_source,
  t.context,
  t.classifier_trace,
  t.payment_status,
  t.source,
  t.actor_id,
  t.idempotency_key,
  t.metadata_json,
  t.created_at,
  t.updated_at,
  t.enrichment_attempted_at,
  t.display_emoji,
  t.display_label,
  t.category_display
FROM v_transactions_effective t
WHERE NOT (
  t.source = 'legacy'
  AND t.transaction_id LIKE 'manual_%'
  AND EXISTS (
    SELECT 1
    FROM v_transactions_effective p
    WHERE p.source = 'pluggy'
      AND p.account_id = t.account_id
      AND p.amount_cents = t.amount_cents
      AND p.transaction_date BETWEEN date(t.transaction_date, '-7 day') AND date(t.transaction_date, '+7 day')
  )
);

CREATE VIEW v_daily_pulse AS
SELECT
  transaction_id,
  transaction_date,
  display_label AS description,
  amount,
  category_id,
  source,
  payment_status,
  account_id
FROM v_transactions_reportable
ORDER BY transaction_date DESC, updated_at DESC;

CREATE VIEW v_monthly_spend AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  CAST(ROUND(SUM(ABS(amount_cents)) / 100.0, 2) AS TEXT) AS expenses,
  COUNT(*) AS expense_count
FROM v_transactions_reportable
WHERE amount_cents < 0
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1, 2, 3;

CREATE VIEW v_cashflow AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  CAST(ROUND(SUM(
    CASE
      WHEN amount_cents > 0 AND COALESCE(category_id, '') != 'cashback'
        THEN amount_cents
      ELSE 0
    END
  ) / 100.0, 2) AS TEXT) AS income,
  CAST(ROUND(SUM(
    CASE
      WHEN amount_cents < 0 THEN ABS(amount_cents)
      ELSE 0
    END
  ) / 100.0, 2) AS TEXT) AS expenses_gross,
  CAST(ROUND(SUM(
    CASE
      WHEN amount_cents > 0 AND COALESCE(category_id, '') = 'cashback'
        THEN amount_cents
      ELSE 0
    END
  ) / 100.0, 2) AS TEXT) AS expense_reduction,
  CAST(ROUND(SUM(
    CASE
      WHEN amount_cents < 0 THEN ABS(amount_cents)
      ELSE 0
    END
  ) / 100.0, 2) AS TEXT) AS expenses,
  CAST(ROUND(SUM(amount_cents) / 100.0, 2) AS TEXT) AS net
FROM v_transactions_reportable
WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1;

CREATE VIEW v_forecast_vs_actual AS
WITH monthly_actuals AS (
  SELECT
    strftime('%Y-%m', transaction_date) AS month_ref,
    COALESCE(account_id, '') AS account_id,
    COALESCE(category_id, '') AS category_id,
    ROUND(SUM(ABS(amount_cents)) / 100.0, 2) AS actual_total
  FROM v_transactions_reportable
  WHERE amount_cents < 0
    AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
  GROUP BY 1, 2, 3
)
SELECT
  f.forecast_id,
  COALESCE(strftime('%Y-%m', f.due_date), strftime('%Y-%m', f.created_at)) AS month_ref,
  f.due_date,
  f.description,
  f.account_id,
  f.category_id,
  f.amount AS forecast_amount,
  CAST(COALESCE(ma.actual_total, 0) AS TEXT) AS actual_amount,
  CAST(ROUND(CAST(f.amount AS REAL) - COALESCE(ma.actual_total, 0), 2) AS TEXT) AS variance,
  f.status
FROM forecast f
LEFT JOIN monthly_actuals ma
  ON ma.month_ref = COALESCE(strftime('%Y-%m', f.due_date), strftime('%Y-%m', f.created_at))
  AND ma.account_id = COALESCE(f.account_id, '')
  AND ma.category_id = COALESCE(f.category_id, '');

CREATE VIEW v_card_summary AS
WITH cycles AS (
  SELECT
    t.transaction_id,
    t.transaction_date,
    t.amount,
    t.amount_cents,
    t.account_id,
    t.payment_status,
    t.category_id,
    a.account_type,
    CASE
      WHEN COALESCE(NULLIF(json_extract(a.metadata_json, '$.billing_closing_day'), ''), '') = ''
        THEN strftime('%Y-%m', t.transaction_date)
      WHEN CAST(strftime('%d', t.transaction_date) AS INTEGER)
           <= CAST(json_extract(a.metadata_json, '$.billing_closing_day') AS INTEGER)
        THEN strftime('%Y-%m', t.transaction_date)
      ELSE strftime('%Y-%m', date(t.transaction_date, 'start of month', '+1 month'))
    END AS cycle_ref
  FROM v_transactions_reportable t
  JOIN accounts a ON a.account_id = t.account_id
)
SELECT
  cycle_ref AS month_ref,
  account_id,
  CAST(ROUND(SUM(CASE WHEN amount_cents < 0 THEN ABS(amount_cents) ELSE 0 END) / 100.0, 2) AS TEXT) AS total_charges,
  CAST(ROUND(SUM(CASE WHEN payment_status = 'pending' THEN ABS(amount_cents) ELSE 0 END) / 100.0, 2) AS TEXT) AS open_amount,
  CAST(ROUND(SUM(CASE WHEN payment_status = 'installment' THEN ABS(amount_cents) ELSE 0 END) / 100.0, 2) AS TEXT) AS installments_future,
  COUNT(CASE WHEN amount_cents < 0 THEN 1 END) AS transaction_count,
  SUM(CASE WHEN payment_status = 'pending' THEN ABS(amount_cents) ELSE 0 END) AS open_amount_cents
FROM cycles
WHERE account_type = 'credit'
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1, 2;

CREATE VIEW v_card_open_now AS
WITH latest_open AS (
  SELECT
    account_id,
    MAX(month_ref) AS month_ref
  FROM v_card_summary
  WHERE open_amount_cents > 0
  GROUP BY account_id
)
SELECT
  cs.month_ref,
  cs.account_id,
  cs.total_charges,
  cs.open_amount,
  cs.installments_future,
  cs.transaction_count
FROM v_card_summary cs
JOIN latest_open lo
  ON lo.account_id = cs.account_id
  AND lo.month_ref = cs.month_ref;

CREATE VIEW v_uncategorized AS
SELECT
  transaction_id,
  transaction_date,
  display_label AS description,
  amount,
  account_id,
  category_source,
  payment_status,
  source
FROM v_transactions_reportable
WHERE category_id IS NULL
   OR category_source IN ('unclassified', 'fallback');
