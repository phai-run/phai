-- Fix v_card_summary: count only charges, not refunds
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
GROUP BY 1, 2;
