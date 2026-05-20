-- Migration 033: transaction anatomy redesign.
--
-- Mirrors schema/sqlite/033_transaction_anatomy.sql. BigQuery can add the
-- new columns in-place and drop the NOT NULL constraint from `description`,
-- then rebuilds the semantic views so reports use the new human-label
-- fallback: description -> merchant_name -> raw_description.

DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_daily_pulse`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_monthly_spend`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_cashflow`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_forecast_vs_actual`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_uncategorized`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_card_open_now`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_card_summary`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_transactions_reportable`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_transactions_effective`;

ALTER TABLE `{{project_id}}.{{dataset_id}}.transactions`
  ADD COLUMN IF NOT EXISTS raw_description STRING;

ALTER TABLE `{{project_id}}.{{dataset_id}}.transactions`
  ADD COLUMN IF NOT EXISTS merchant_name STRING;

ALTER TABLE `{{project_id}}.{{dataset_id}}.transactions`
  ADD COLUMN IF NOT EXISTS purpose STRING;

ALTER TABLE `{{project_id}}.{{dataset_id}}.transactions`
  ADD COLUMN IF NOT EXISTS classifier_trace STRING;

ALTER TABLE `{{project_id}}.{{dataset_id}}.transactions`
  ALTER COLUMN description DROP NOT NULL;

UPDATE `{{project_id}}.{{dataset_id}}.transactions`
SET raw_description = COALESCE(NULLIF(raw_description, ''), description, '')
WHERE raw_description IS NULL
   OR raw_description = '';

UPDATE `{{project_id}}.{{dataset_id}}.transactions`
SET classifier_trace = context
WHERE classifier_trace IS NULL
  AND context IS NOT NULL;

UPDATE `{{project_id}}.{{dataset_id}}.transactions`
SET description = NULL
WHERE source = 'pluggy'
  AND description = raw_description;

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
    COALESCE(t.raw_description, t.description, '') AS raw_description,
    t.description,
    t.merchant_name,
    t.purpose,
    t.amount,
    t.amount_cents,
    t.tx_type,
    t.category_id,
    t.category_source,
    t.context,
    t.classifier_trace,
    t.payment_status,
    t.source,
    t.actor_id,
    t.idempotency_key,
    t.metadata_json,
    t.created_at,
    t.updated_at,
    t.enrichment_attempted_at
  FROM `{{project_id}}.{{dataset_id}}.transactions` t
  LEFT JOIN selected_splits ss
    ON ss.parent_transaction_id = t.transaction_id
  WHERE ss.split_id IS NULL

  UNION ALL

  SELECT
    sl.split_line_id AS transaction_id,
    t.account_id,
    t.transaction_date,
    COALESCE(t.raw_description, t.description, '') AS raw_description,
    COALESCE(NULLIF(sl.description, ''), t.description) AS description,
    t.merchant_name,
    t.purpose,
    sl.amount,
    CAST(ROUND(sl.amount * 100) AS INT64) AS amount_cents,
    CASE
      WHEN sl.amount > 0 THEN 'credit'
      WHEN sl.amount < 0 THEN 'debit'
      ELSE t.tx_type
    END AS tx_type,
    COALESCE(NULLIF(sl.category_id, ''), t.category_id) AS category_id,
    COALESCE(NULLIF(sl.category_source, ''), 'split') AS category_source,
    COALESCE(NULLIF(sl.context, ''), t.context) AS context,
    COALESCE(NULLIF(sl.context, ''), t.classifier_trace) AS classifier_trace,
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
    GREATEST(t.updated_at, sl.updated_at) AS updated_at,
    t.enrichment_attempted_at
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
  raw_description,
  description,
  merchant_name,
  purpose,
  amount,
  amount_cents,
  tx_type,
  category_id,
  category_source,
  context,
  classifier_trace,
  payment_status,
  source,
  actor_id,
  idempotency_key,
  metadata_json,
  created_at,
  updated_at,
  enrichment_attempted_at,
  CASE
    WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
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
      WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
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
    TRIM(COALESCE(NULLIF(description, ''), NULLIF(merchant_name, ''), raw_description))
  ) AS display_label,
  CASE
    WHEN category_id IS NULL OR TRIM(category_id) = '' THEN '❓ sem categoria'
    ELSE CONCAT(
      CASE
        WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
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

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_transactions_reportable` AS
SELECT
  t.transaction_id,
  t.account_id,
  t.transaction_date,
  t.raw_description,
  t.description,
  t.merchant_name,
  t.purpose,
  t.amount,
  t.amount_cents,
  t.tx_type,
  t.category_id,
  t.category_source,
  t.context,
  t.classifier_trace,
  t.payment_status,
  t.source,
  t.actor_id,
  t.idempotency_key,
  t.metadata_json,
  t.created_at,
  t.updated_at,
  t.enrichment_attempted_at,
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
WHERE amount_cents < 0
  AND COALESCE(category_id, '') NOT IN (
    SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`
  )
GROUP BY 1, 2, 3;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_cashflow` AS
SELECT
  FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
  ROUND(SUM(IF(amount_cents > 0 AND COALESCE(category_id, '') != 'cashback',
    amount_cents, 0)) / 100.0, 2) AS income,
  ROUND(SUM(IF(amount_cents < 0, ABS(amount_cents), 0)) / 100.0, 2) AS expenses_gross,
  ROUND(SUM(IF(amount_cents > 0 AND COALESCE(category_id, '') = 'cashback',
    amount_cents, 0)) / 100.0, 2) AS expense_reduction,
  ROUND(SUM(IF(amount_cents < 0, ABS(amount_cents), 0)) / 100.0, 2) AS expenses,
  ROUND(SUM(amount_cents) / 100.0, 2) AS net
FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable`
WHERE COALESCE(category_id, '') NOT IN (
  SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`
)
GROUP BY 1;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_forecast_vs_actual` AS
WITH tx AS (
  SELECT
    FORMAT_DATE('%Y-%m', transaction_date) AS month_ref,
    COALESCE(account_id, '') AS account_id,
    COALESCE(category_id, '') AS category_id,
    ROUND(SUM(IF(amount_cents < 0, ABS(amount_cents), 0)) / 100.0, 2) AS actual_amount
  FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable`
  WHERE amount_cents < 0
    AND COALESCE(category_id, '') NOT IN (
      SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`
    )
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
WITH cycles AS (
  SELECT
    t.transaction_id,
    t.transaction_date,
    t.amount,
    t.amount_cents,
    t.account_id,
    t.payment_status,
    t.category_id,
    a.account_type,
    CASE
      WHEN COALESCE(NULLIF(JSON_VALUE(a.metadata_json, '$.billing_closing_day'), ''), '') = ''
        THEN FORMAT_DATE('%Y-%m', t.transaction_date)
      WHEN EXTRACT(DAY FROM t.transaction_date)
           <= CAST(JSON_VALUE(a.metadata_json, '$.billing_closing_day') AS INT64)
        THEN FORMAT_DATE('%Y-%m', t.transaction_date)
      ELSE FORMAT_DATE('%Y-%m', DATE_ADD(DATE_TRUNC(t.transaction_date, MONTH), INTERVAL 1 MONTH))
    END AS cycle_ref
  FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable` t
  JOIN `{{project_id}}.{{dataset_id}}.accounts` a
    ON a.account_id = t.account_id
)
SELECT
  cycle_ref AS month_ref,
  account_id,
  ROUND(SUM(IF(amount_cents < 0, ABS(amount_cents), 0)) / 100.0, 2) AS total_charges,
  ROUND(SUM(IF(payment_status = 'pending', ABS(amount_cents), 0)) / 100.0, 2) AS open_amount,
  ROUND(SUM(IF(payment_status = 'installment', ABS(amount_cents), 0)) / 100.0, 2) AS installments_future,
  COUNTIF(amount_cents < 0) AS transaction_count,
  SUM(IF(payment_status = 'pending', ABS(amount_cents), 0)) AS open_amount_cents
FROM cycles
WHERE account_type = 'credit'
  AND COALESCE(category_id, '') NOT IN (
    SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`
  )
GROUP BY 1, 2;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_card_open_now` AS
WITH latest_open AS (
  SELECT
    account_id,
    MAX(month_ref) AS month_ref
  FROM `{{project_id}}.{{dataset_id}}.v_card_summary`
  WHERE open_amount_cents > 0
  GROUP BY account_id
)
SELECT
  cs.month_ref,
  cs.account_id,
  cs.total_charges,
  cs.open_amount,
  cs.installments_future,
  cs.transaction_count
FROM `{{project_id}}.{{dataset_id}}.v_card_summary` cs
JOIN latest_open lo
  ON lo.account_id = cs.account_id
  AND lo.month_ref = cs.month_ref;

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
