-- Migration 009: fix remaining credit-card transactions with wrong sign
-- Migration 007 only caught tx_type='debit'. Some Pluggy transactions arrived
-- without a type field, so tx_type was inferred as 'credit' from the positive
-- amount — but they are actually purchases. Negate all positive amounts on
-- credit accounts except genuine credits (cashback, refunds).

UPDATE `{{project_id}}.{{dataset_id}}.transactions` t
SET t.amount = -1 * t.amount,
    t.tx_type = 'debit',
    t.updated_at = CURRENT_TIMESTAMP()
WHERE t.account_id IN (
    SELECT account_id FROM `{{project_id}}.{{dataset_id}}.accounts` WHERE account_type = 'credit'
)
AND t.amount > 0
AND COALESCE(t.category_id, '') NOT IN ('credit-card-payment', 'cashback', 'refund');
