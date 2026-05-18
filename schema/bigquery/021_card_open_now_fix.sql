-- Migration 021: fix v_card_open_now semantics. Mirror of
-- schema/sqlite/020_card_open_now_fix.sql.

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_card_open_now` AS
WITH latest_open AS (
  SELECT
    account_id,
    MAX(month_ref) AS month_ref
  FROM `{{project_id}}.{{dataset_id}}.v_card_summary`
  WHERE open_amount > 0
  GROUP BY account_id
)
SELECT
  cs.month_ref,
  cs.account_id,
  cs.total_charges,
  cs.open_amount,
  cs.transaction_count
FROM `{{project_id}}.{{dataset_id}}.v_card_summary` cs
JOIN latest_open lo
  ON lo.account_id = cs.account_id
  AND lo.month_ref = cs.month_ref;
