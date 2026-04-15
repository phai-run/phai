-- Migration 008: exclude internal movements from aggregate reports
-- Credit-card bill payments are transfers between own accounts, not real expenses.
-- Category: credit-card-payment

DROP VIEW IF EXISTS v_monthly_spend;
DROP VIEW IF EXISTS v_cashflow;
DROP VIEW IF EXISTS v_forecast_vs_actual;
DROP VIEW IF EXISTS v_card_summary;

CREATE VIEW v_monthly_spend AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  CAST(ROUND(SUM(ABS(CAST(amount AS REAL))), 2) AS TEXT) AS expenses,
  COUNT(*) AS expense_count
FROM transactions
WHERE CAST(amount AS REAL) < 0
  AND COALESCE(category_id, '') NOT IN ('credit-card-payment')
GROUP BY 1, 2, 3;

CREATE VIEW v_cashflow AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  CAST(ROUND(SUM(CASE WHEN CAST(amount AS REAL) > 0 THEN CAST(amount AS REAL) ELSE 0 END), 2) AS TEXT) AS income,
  CAST(ROUND(SUM(CASE WHEN CAST(amount AS REAL) < 0 THEN ABS(CAST(amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS expenses,
  CAST(ROUND(SUM(CAST(amount AS REAL)), 2) AS TEXT) AS net
FROM transactions
WHERE COALESCE(category_id, '') NOT IN ('credit-card-payment')
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
    AND COALESCE(category_id, '') NOT IN ('credit-card-payment')
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
  AND COALESCE(t.category_id, '') NOT IN ('credit-card-payment')
GROUP BY 1, 2;
