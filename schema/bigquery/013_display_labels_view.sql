CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_transactions_effective` AS
SELECT
  transaction_id,
  account_id,
  transaction_date,
  description,
  amount,
  tx_type,
  category_id,
  category_source,
  context,
  payment_status,
  source,
  actor_id,
  idempotency_key,
  metadata_json,
  created_at,
  updated_at,
  CASE
    WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount > 0 THEN '💰'
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
  CONCAT(
    CASE
      WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount > 0 THEN '💰'
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
    END,
    ' ',
    TRIM(COALESCE(NULLIF(context, ''), description))
  ) AS display_label,
  CASE
    WHEN category_id IS NULL OR TRIM(category_id) = '' THEN '❓ sem categoria'
    ELSE CONCAT(
      CASE
        WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount > 0 THEN '💰'
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
      END,
      ' ',
      REGEXP_REPLACE(REPLACE(category_id, ':', ' > '), '-', ' ')
    )
  END AS category_display
FROM `{{project_id}}.{{dataset_id}}.transactions`;

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
FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective`
ORDER BY transaction_date DESC, updated_at DESC;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_monthly_spend` AS
SELECT
  FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  ROUND(SUM(ABS(amount)), 2) AS expenses,
  COUNT(*) AS expense_count
FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective`
WHERE amount < 0
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`)
GROUP BY 1, 2, 3;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_cashflow` AS
SELECT
  FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
  ROUND(SUM(IF(amount > 0, amount, 0)), 2) AS income,
  ROUND(SUM(IF(amount < 0, ABS(amount), 0)), 2) AS expenses,
  ROUND(SUM(amount), 2) AS net
FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective`
WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`)
GROUP BY 1;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_forecast_vs_actual` AS
WITH tx AS (
  SELECT
    FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
    COALESCE(account_id, '') AS account_id,
    COALESCE(category_id, '') AS category_id,
    ROUND(SUM(IF(amount < 0, ABS(amount), 0)), 2) AS actual_amount
  FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective`
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
FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective` t
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
FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective`
WHERE category_id IS NULL
   OR category_source IN ('unclassified', 'fallback');
