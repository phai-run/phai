CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.transaction_splits` (
  split_id STRING NOT NULL,
  parent_transaction_id STRING NOT NULL,
  payload_hash STRING NOT NULL,
  status STRING NOT NULL,
  source STRING NOT NULL,
  notes STRING,
  actor_id STRING NOT NULL,
  idempotency_key STRING NOT NULL,
  metadata_json JSON,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.transaction_split_lines` (
  split_line_id STRING NOT NULL,
  split_id STRING NOT NULL,
  parent_transaction_id STRING NOT NULL,
  line_index INT64 NOT NULL,
  description STRING NOT NULL,
  amount NUMERIC NOT NULL,
  category_id STRING,
  category_source STRING NOT NULL,
  context STRING,
  status STRING NOT NULL,
  actor_id STRING NOT NULL,
  idempotency_key STRING NOT NULL,
  metadata_json JSON,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.receipt_items` (
  receipt_item_id STRING NOT NULL,
  parent_transaction_id STRING NOT NULL,
  split_id STRING,
  split_line_id STRING,
  item_index INT64 NOT NULL,
  description STRING NOT NULL,
  quantity NUMERIC,
  unit STRING,
  unit_price NUMERIC,
  total_price NUMERIC,
  code STRING,
  store_name STRING,
  status STRING NOT NULL,
  actor_id STRING NOT NULL,
  idempotency_key STRING NOT NULL,
  metadata_json JSON,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.split_review_policies` (
  policy_id STRING NOT NULL,
  name STRING NOT NULL,
  match_type STRING NOT NULL,
  match_value STRING NOT NULL,
  min_abs_amount NUMERIC,
  status STRING NOT NULL,
  actor_id STRING NOT NULL,
  idempotency_key STRING NOT NULL,
  metadata_json JSON,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_transactions_effective` AS
WITH split_candidates AS (
  SELECT
    s.split_id,
    s.parent_transaction_id,
    ROW_NUMBER() OVER (
      PARTITION BY s.parent_transaction_id
      ORDER BY
        CASE
          WHEN s.status = 'confirmed' THEN 0
          ELSE 1
        END,
        s.updated_at DESC,
        s.created_at DESC,
        s.split_id DESC
    ) AS row_priority
  FROM `{{project_id}}.{{dataset_id}}.transaction_splits` s
  WHERE s.status IN ('active', 'confirmed')
    AND EXISTS (
      SELECT 1
      FROM `{{project_id}}.{{dataset_id}}.transaction_split_lines` sl
      WHERE sl.split_id = s.split_id
        AND sl.status IN ('active', 'confirmed')
    )
),
selected_splits AS (
  SELECT
    split_id,
    parent_transaction_id
  FROM split_candidates
  WHERE row_priority = 1
),
base_transactions AS (
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
    t.updated_at
  FROM `{{project_id}}.{{dataset_id}}.transactions` t
  LEFT JOIN selected_splits ss
    ON ss.parent_transaction_id = t.transaction_id
  WHERE ss.split_id IS NULL

  UNION ALL

  SELECT
    sl.split_line_id AS transaction_id,
    t.account_id,
    t.transaction_date,
    COALESCE(NULLIF(sl.description, ''), t.description) AS description,
    sl.amount,
    CASE
      WHEN sl.amount > 0 THEN 'credit'
      WHEN sl.amount < 0 THEN 'debit'
      ELSE t.tx_type
    END AS tx_type,
    COALESCE(NULLIF(sl.category_id, ''), t.category_id) AS category_id,
    COALESCE(NULLIF(sl.category_source, ''), 'split') AS category_source,
    COALESCE(NULLIF(sl.context, ''), t.context) AS context,
    t.payment_status,
    t.source,
    COALESCE(NULLIF(sl.actor_id, ''), t.actor_id) AS actor_id,
    COALESCE(NULLIF(sl.idempotency_key, ''), t.idempotency_key) AS idempotency_key,
    JSON_OBJECT(
      'effectiveKind', 'split',
      'parentTransactionId', t.transaction_id,
      'splitId', sl.split_id,
      'splitLineId', sl.split_line_id
    ) AS metadata_json,
    LEAST(t.created_at, sl.created_at) AS created_at,
    GREATEST(t.updated_at, sl.updated_at) AS updated_at
  FROM selected_splits ss
  JOIN `{{project_id}}.{{dataset_id}}.transactions` t
    ON t.transaction_id = ss.parent_transaction_id
  JOIN `{{project_id}}.{{dataset_id}}.transaction_split_lines` sl
    ON sl.split_id = ss.split_id
   AND sl.parent_transaction_id = ss.parent_transaction_id
  WHERE sl.status IN ('active', 'confirmed')
)
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
FROM base_transactions;

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
