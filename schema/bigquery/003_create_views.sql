CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_daily_pulse` AS
SELECT
  transaction_id,
  transaction_date,
  description,
  amount,
  category_id,
  source,
  payment_status,
  account_id
FROM `{{project_id}}.{{dataset_id}}.transactions`
ORDER BY transaction_date DESC, updated_at DESC;

