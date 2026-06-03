-- Migration 039: converge v_card_summary's cycle boundary with cash_month.
--
-- v_card_summary used `day <= closing_day → current cycle`, but the canonical
-- cash_month (migration 037) and `compute_bill_id` use `day < closing_day →
-- current cycle` (a charge ON the closing day is the first of the next cycle —
-- Nubank's OFX DTSTART is inclusive). The card panel (cards_open_now →
-- v_card_summary) and the cashflow chart (cash_month) therefore disagreed on a
-- charge dated exactly on the closing day. This aligns v_card_summary to the
-- same `< closing_day` boundary so every card surface shares one cycle rule.
-- See ADR-0025/0026. Only the boundary changes; totals and columns are identical.

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
           < CAST(JSON_VALUE(a.metadata_json, '$.billing_closing_day') AS INT64)
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
