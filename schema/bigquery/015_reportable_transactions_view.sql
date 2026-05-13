CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_transactions_reportable` AS
SELECT
  t.transaction_id,
  t.account_id,
  t.transaction_date,
  t.description,
  t.amount,
  t.tx_type,
  t.category_id,
  t.category_source,
  t.context,
  t.payment_status,
  t.source,
  t.actor_id,
  t.idempotency_key,
  t.metadata_json,
  t.created_at,
  t.updated_at,
  t.display_emoji,
  t.display_label,
  t.category_display
FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective` t
WHERE NOT (
  t.source = 'legacy'
  AND STARTS_WITH(t.transaction_id, 'manual_')
  AND EXISTS (
    SELECT 1
    FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective` p
    WHERE p.source = 'pluggy'
      AND p.account_id = t.account_id
      AND p.amount = t.amount
      AND p.transaction_date BETWEEN DATE_SUB(t.transaction_date, INTERVAL 7 DAY)
      AND DATE_ADD(t.transaction_date, INTERVAL 7 DAY)
  )
);

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_daily_pulse` AS
SELECT
  transaction_id,
  transaction_date,
  display_label AS description,
  amount,
  category_id,
  source,
  payment_status,
  account_id
FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable`
ORDER BY transaction_date DESC, updated_at DESC;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_monthly_spend` AS
SELECT
  FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  ROUND(SUM(ABS(amount)), 2) AS expenses,
  COUNT(*) AS expense_count
FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable`
WHERE amount < 0
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`)
GROUP BY 1, 2, 3;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_cashflow` AS
SELECT
  FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
  ROUND(SUM(IF(amount > 0, amount, 0)), 2) AS income,
  ROUND(SUM(IF(amount < 0, ABS(amount), 0)), 2) AS expenses,
  ROUND(SUM(amount), 2) AS net
FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable`
WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`)
GROUP BY 1;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_forecast_vs_actual` AS
WITH tx AS (
  SELECT
    FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
    COALESCE(account_id, '') AS account_id,
    COALESCE(category_id, '') AS category_id,
    ROUND(SUM(IF(amount < 0, ABS(amount), 0)), 2) AS actual_amount
  FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable`
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
FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable` t
JOIN `{{project_id}}.{{dataset_id}}.accounts` a
  ON a.account_id = t.account_id
WHERE a.account_type = 'credit'
  AND COALESCE(t.category_id, '') NOT IN (SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`)
GROUP BY 1, 2;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_uncategorized` AS
SELECT
  transaction_id,
  transaction_date,
  display_label AS description,
  amount,
  account_id,
  category_source,
  payment_status,
  source
FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable`
WHERE category_id IS NULL
   OR category_source IN ('unclassified', 'fallback');
