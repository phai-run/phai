-- Migration 026: surface cashback as expense-reduction in v_cashflow.
-- Mirror of schema/sqlite/025_cashback_as_expense_reduction.sql.

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_cashflow` AS
SELECT
  FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
  ROUND(SUM(IF(amount > 0 AND COALESCE(category_id, '') != 'cashback', amount, 0)), 2) AS income,
  ROUND(SUM(IF(amount < 0, ABS(amount), 0)), 2) AS expenses_gross,
  ROUND(SUM(IF(amount > 0 AND COALESCE(category_id, '') = 'cashback', amount, 0)), 2) AS expense_reduction,
  ROUND(SUM(IF(amount < 0, ABS(amount), 0)), 2) AS expenses,
  ROUND(SUM(amount), 2) AS net
FROM `{{project_id}}.{{dataset_id}}.transactions`
WHERE COALESCE(category_id, '') NOT IN (
  SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`
)
GROUP BY 1;
