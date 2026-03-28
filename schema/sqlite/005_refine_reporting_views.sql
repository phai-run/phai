DROP VIEW IF EXISTS v_monthly_spend;

CREATE VIEW v_monthly_spend AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  ROUND(SUM(ABS(amount)), 2) AS expenses,
  COUNT(*) AS expense_count
FROM transactions
WHERE amount < 0
GROUP BY 1, 2, 3;
