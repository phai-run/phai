-- Migration 035: fix v_card_summary double-counting of card charges.
--
-- v_card_summary previously read from `transactions` directly, which
-- admits both the legacy manual entry and its Pluggy-synced equivalent
-- for the same negative charge. `total_charges` and `transaction_count`
-- only consider amount_cents < 0, so duplicate debits (not duplicated
-- "Pagamento recebido" credits) inflated both totals and the count.
--
-- Switching to `v_transactions_reportable` ensures:
--   1. Legacy manual entries deduped against Pluggy equivalents.
--   2. Consistent filtering with every other report view.
--

DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_card_open_now`;
DROP VIEW IF EXISTS `{{project_id}}.{{dataset_id}}.v_card_summary`;

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
