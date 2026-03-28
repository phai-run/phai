CREATE VIEW IF NOT EXISTS v_monthly_spend AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  ROUND(SUM(CASE WHEN amount < 0 THEN ABS(amount) ELSE 0 END), 2) AS expenses,
  SUM(CASE WHEN amount < 0 THEN 1 ELSE 0 END) AS expense_count
FROM transactions
GROUP BY 1, 2, 3;

CREATE VIEW IF NOT EXISTS v_cashflow AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  ROUND(SUM(CASE WHEN amount > 0 THEN amount ELSE 0 END), 2) AS income,
  ROUND(SUM(CASE WHEN amount < 0 THEN ABS(amount) ELSE 0 END), 2) AS expenses,
  ROUND(SUM(amount), 2) AS net
FROM transactions
GROUP BY 1;

CREATE VIEW IF NOT EXISTS v_forecast_vs_actual AS
SELECT
  f.forecast_id,
  COALESCE(strftime('%Y-%m', f.due_date), strftime('%Y-%m', f.created_at)) AS month_ref,
  f.due_date,
  f.description,
  f.account_id,
  f.category_id,
  f.amount AS forecast_amount,
  ROUND(COALESCE(SUM(CASE
    WHEN strftime('%Y-%m', t.transaction_date) = COALESCE(strftime('%Y-%m', f.due_date), strftime('%Y-%m', f.created_at))
     AND COALESCE(t.account_id, '') = COALESCE(f.account_id, '')
     AND COALESCE(t.category_id, '') = COALESCE(f.category_id, '')
    THEN ABS(t.amount) ELSE 0 END), 0), 2) AS actual_amount,
  ROUND(f.amount - COALESCE(SUM(CASE
    WHEN strftime('%Y-%m', t.transaction_date) = COALESCE(strftime('%Y-%m', f.due_date), strftime('%Y-%m', f.created_at))
     AND COALESCE(t.account_id, '') = COALESCE(f.account_id, '')
     AND COALESCE(t.category_id, '') = COALESCE(f.category_id, '')
    THEN ABS(t.amount) ELSE 0 END), 0), 2) AS variance,
  f.status
FROM forecast f
LEFT JOIN transactions t ON 1 = 1
GROUP BY 1, 2, 3, 4, 5, 6, 7, 10;

CREATE VIEW IF NOT EXISTS v_card_summary AS
SELECT
  strftime('%Y-%m', t.transaction_date) AS month_ref,
  t.account_id,
  ROUND(SUM(CASE WHEN t.amount < 0 THEN ABS(t.amount) ELSE 0 END), 2) AS total_charges,
  ROUND(SUM(CASE WHEN t.payment_status IN ('pending', 'em_aberto', 'parcial') THEN ABS(t.amount) ELSE 0 END), 2) AS open_amount,
  COUNT(*) AS transaction_count
FROM transactions t
JOIN accounts a ON a.account_id = t.account_id
WHERE a.account_type = 'credit'
GROUP BY 1, 2;

CREATE VIEW IF NOT EXISTS v_uncategorized AS
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
