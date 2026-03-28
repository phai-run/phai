CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_monthly_spend` AS
SELECT
  FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  ROUND(SUM(ABS(amount)), 2) AS expenses,
  COUNT(*) AS expense_count
FROM `{{project_id}}.{{dataset_id}}.transactions`
WHERE amount < 0
GROUP BY 1, 2, 3;
