-- Migration 006: harden schema for financial precision and performance
-- Must drop all views before recreating tables they reference.

DROP VIEW IF EXISTS v_daily_pulse;
DROP VIEW IF EXISTS v_monthly_spend;
DROP VIEW IF EXISTS v_cashflow;
DROP VIEW IF EXISTS v_forecast_vs_actual;
DROP VIEW IF EXISTS v_card_summary;
DROP VIEW IF EXISTS v_uncategorized;

-- 1. Recreate transactions with TEXT amount (was REAL — float precision risk)
CREATE TABLE IF NOT EXISTS transactions_new (
  transaction_id TEXT PRIMARY KEY,
  account_id TEXT,
  transaction_date TEXT NOT NULL,
  description TEXT NOT NULL,
  amount TEXT NOT NULL,
  tx_type TEXT NOT NULL,
  category_id TEXT,
  category_source TEXT NOT NULL,
  context TEXT,
  payment_status TEXT NOT NULL,
  source TEXT NOT NULL,
  actor_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

INSERT OR IGNORE INTO transactions_new
  SELECT transaction_id, account_id, transaction_date, description,
         CAST(ROUND(amount, 2) AS TEXT), tx_type, category_id, category_source,
         context, payment_status, source, actor_id, idempotency_key,
         metadata_json, created_at, updated_at
  FROM transactions;

DROP TABLE IF EXISTS transactions;
ALTER TABLE transactions_new RENAME TO transactions;

-- 2. Recreate forecast with TEXT amount
CREATE TABLE IF NOT EXISTS forecast_new (
  forecast_id TEXT PRIMARY KEY,
  due_date TEXT NOT NULL,
  description TEXT NOT NULL,
  amount TEXT NOT NULL,
  category_id TEXT,
  account_id TEXT,
  status TEXT NOT NULL,
  recurrence TEXT,
  actor_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

INSERT OR IGNORE INTO forecast_new
  SELECT forecast_id, due_date, description,
         CAST(ROUND(amount, 2) AS TEXT), category_id, account_id,
         status, recurrence, actor_id, idempotency_key,
         metadata_json, created_at, updated_at
  FROM forecast;

DROP TABLE IF EXISTS forecast;
ALTER TABLE forecast_new RENAME TO forecast;

-- 3. Add indices for common query patterns
CREATE INDEX IF NOT EXISTS idx_transactions_date ON transactions(transaction_date);
CREATE INDEX IF NOT EXISTS idx_transactions_account ON transactions(account_id);
CREATE INDEX IF NOT EXISTS idx_transactions_category ON transactions(category_id);
CREATE INDEX IF NOT EXISTS idx_forecast_due_date ON forecast(due_date);

-- 4. Recreate all views

CREATE VIEW v_daily_pulse AS
SELECT
  transaction_id,
  transaction_date,
  description,
  amount,
  category_id,
  source,
  payment_status,
  account_id
FROM transactions
ORDER BY transaction_date DESC, updated_at DESC;

CREATE VIEW v_monthly_spend AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  CAST(ROUND(SUM(ABS(CAST(amount AS REAL))), 2) AS TEXT) AS expenses,
  COUNT(*) AS expense_count
FROM transactions
WHERE CAST(amount AS REAL) < 0
GROUP BY 1, 2, 3;

CREATE VIEW v_cashflow AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  CAST(ROUND(SUM(CASE WHEN CAST(amount AS REAL) > 0 THEN CAST(amount AS REAL) ELSE 0 END), 2) AS TEXT) AS income,
  CAST(ROUND(SUM(CASE WHEN CAST(amount AS REAL) < 0 THEN ABS(CAST(amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS expenses,
  CAST(ROUND(SUM(CAST(amount AS REAL)), 2) AS TEXT) AS net
FROM transactions
GROUP BY 1;

CREATE VIEW v_forecast_vs_actual AS
WITH monthly_actuals AS (
  SELECT
    strftime('%Y-%m', transaction_date) AS month_ref,
    COALESCE(account_id, '') AS account_id,
    COALESCE(category_id, '') AS category_id,
    ROUND(SUM(ABS(CAST(amount AS REAL))), 2) AS actual_total
  FROM transactions
  WHERE CAST(amount AS REAL) < 0
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
SELECT
  strftime('%Y-%m', t.transaction_date) AS month_ref,
  t.account_id,
  CAST(ROUND(SUM(CASE WHEN CAST(t.amount AS REAL) < 0 THEN ABS(CAST(t.amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS total_charges,
  CAST(ROUND(SUM(CASE WHEN t.payment_status IN ('pending', 'em_aberto', 'parcial') THEN ABS(CAST(t.amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS open_amount,
  COUNT(CASE WHEN CAST(t.amount AS REAL) < 0 THEN 1 END) AS transaction_count
FROM transactions t
JOIN accounts a ON a.account_id = t.account_id
WHERE a.account_type = 'credit'
GROUP BY 1, 2;

CREATE VIEW v_uncategorized AS
SELECT
  transaction_id,
  transaction_date,
  description,
  amount,
  account_id,
  category_source,
  payment_status,
  source
FROM transactions
WHERE category_id IS NULL
   OR category_source IN ('unclassified', 'fallback');
