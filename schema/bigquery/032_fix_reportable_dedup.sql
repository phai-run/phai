-- Migration 032: fix v_transactions_reportable dedup filter.
--
-- Migration 029/031 changed the dedup comparison from `p.amount = t.amount`
-- to `p.amount_cents = t.amount_cents`. BigQuery cannot de-correlate the
-- EXISTS subquery when the predicate references a column that is partially
-- computed (amount_cents for split rows). NUMERIC equality is exact on
-- BigQuery, so reverting to `p.amount = t.amount` is safe and fixes the
-- correlated-subquery error affecting v_daily_pulse, v_monthly_spend, etc.

DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_daily_pulse`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_monthly_spend`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_forecast_vs_actual`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_uncategorized`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_card_open_now`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_card_summary`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_transactions_reportable`;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_transactions_reportable` AS
SELECT
  t.transaction_id,
  t.account_id,
  t.transaction_date,
  t.description,
  t.amount,
  t.amount_cents,
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
WHERE amount_cents < 0
  AND COALESCE(category_id, '') NOT IN (
    SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`
  )
GROUP BY 1, 2, 3;

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
  FROM `{{project_id}}.{{dataset_id}}.transactions` t
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
