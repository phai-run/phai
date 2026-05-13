DROP VIEW IF EXISTS v_transactions_reportable;

CREATE VIEW v_transactions_reportable AS
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
FROM v_transactions_effective t
WHERE NOT (
  t.source = 'legacy'
  AND t.transaction_id LIKE 'manual_%'
  AND EXISTS (
    SELECT 1
    FROM v_transactions_effective p
    WHERE p.source = 'pluggy'
      AND p.account_id = t.account_id
      AND CAST(p.amount AS REAL) = CAST(t.amount AS REAL)
      AND p.transaction_date BETWEEN date(t.transaction_date, '-7 day') AND date(t.transaction_date, '+7 day')
  )
);
