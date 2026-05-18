-- Migration 020: redefine v_card_summary to bucket by billing cycle, not calendar month.
--
-- See schema/sqlite/019_card_billing_cycle.sql for the rationale. Mirrors the
-- SQLite migration under monotonic numbering shared across backends.

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_card_summary` AS
WITH cycles AS (
  SELECT
    t.transaction_id,
    t.transaction_date,
    t.amount,
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
  ROUND(SUM(IF(amount < 0, ABS(amount), 0)), 2) AS total_charges,
  ROUND(SUM(IF(payment_status IN ('pending', 'em_aberto', 'parcial'), ABS(amount), 0)), 2) AS open_amount,
  COUNTIF(amount < 0) AS transaction_count
FROM cycles
WHERE account_type = 'credit'
  AND COALESCE(category_id, '') NOT IN (
    SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`
  )
GROUP BY 1, 2;

-- v_card_open_now is introduced separately in migration 021.
