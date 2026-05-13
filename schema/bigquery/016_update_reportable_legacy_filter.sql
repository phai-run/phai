CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_transactions_reportable` AS
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
