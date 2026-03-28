CREATE VIEW IF NOT EXISTS v_daily_pulse AS
SELECT
  transaction_id,
  transaction_date,
  description,
  amount,
  category_id,
  source,
  payment_status,
  account_id
FROM transactions
ORDER BY transaction_date DESC, updated_at DESC;

