-- Migration 010: reference table for internal movement categories
-- Views JOIN against this table. To add a new internal category,
-- just INSERT into this table — no view recreation needed.

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.internal_categories` (
  category_id STRING NOT NULL
);

MERGE `{{project_id}}.{{dataset_id}}.internal_categories` target
USING (
  SELECT 'credit-card-payment' AS category_id UNION ALL
  SELECT 'transfer-internal'
) source
ON target.category_id = source.category_id
WHEN NOT MATCHED THEN INSERT (category_id) VALUES (source.category_id);

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_monthly_spend` AS
SELECT
  FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  ROUND(SUM(ABS(amount)), 2) AS expenses,
  COUNT(*) AS expense_count
FROM `{{project_id}}.{{dataset_id}}.transactions`
WHERE amount < 0
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`)
GROUP BY 1, 2, 3;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_cashflow` AS
SELECT
  FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
  ROUND(SUM(IF(amount > 0, amount, 0)), 2) AS income,
  ROUND(SUM(IF(amount < 0, ABS(amount), 0)), 2) AS expenses,
  ROUND(SUM(amount), 2) AS net
FROM `{{project_id}}.{{dataset_id}}.transactions`
WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`)
GROUP BY 1;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_forecast_vs_actual` AS
WITH tx AS (
  SELECT
    FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
    COALESCE(account_id, '') AS account_id,
    COALESCE(category_id, '') AS category_id,
    ROUND(SUM(IF(amount < 0, ABS(amount), 0)), 2) AS actual_amount
  FROM `{{project_id}}.{{dataset_id}}.transactions`
  WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`)
  GROUP BY 1, 2, 3
)
SELECT
  f.forecast_id,
  COALESCE(FORMAT_DATE('%Y-%m', f.due_date), FORMAT_TIMESTAMP('%Y-%m', f.created_at)) AS month_ref,
  f.due_date,
  f.description,
  f.account_id,
  f.category_id,
  f.amount AS forecast_amount,
  COALESCE(tx.actual_amount, 0) AS actual_amount,
  ROUND(f.amount - COALESCE(tx.actual_amount, 0), 2) AS variance,
  f.status
FROM `{{project_id}}.{{dataset_id}}.forecast` f
LEFT JOIN tx
  ON tx.month_ref = COALESCE(FORMAT_DATE('%Y-%m', f.due_date), FORMAT_TIMESTAMP('%Y-%m', f.created_at))
 AND tx.account_id = COALESCE(f.account_id, '')
 AND tx.category_id = COALESCE(f.category_id, '');

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_card_summary` AS
SELECT
  FORMAT_DATE('%Y-%m', t.transaction_date) AS month_ref,
  t.account_id,
  ROUND(SUM(IF(t.amount < 0, ABS(t.amount), 0)), 2) AS total_charges,
  ROUND(SUM(IF(t.payment_status IN ('pending', 'em_aberto', 'parcial'), ABS(t.amount), 0)), 2) AS open_amount,
  COUNTIF(t.amount < 0) AS transaction_count
FROM `{{project_id}}.{{dataset_id}}.transactions` t
JOIN `{{project_id}}.{{dataset_id}}.accounts` a
  ON a.account_id = t.account_id
WHERE a.account_type = 'credit'
  AND COALESCE(t.category_id, '') NOT IN (SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`)
GROUP BY 1, 2;
