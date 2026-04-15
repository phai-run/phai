-- Migration 009: fix remaining credit-card transactions with wrong sign
-- Migration 007 only caught tx_type='debit'. Some Pluggy transactions arrived
-- without a type field, so tx_type was inferred as 'credit' from the positive
-- amount — but they are actually purchases. Negate all positive amounts on
-- credit accounts except genuine credits (cashback, refunds).

UPDATE transactions
SET amount = CAST(-1 * CAST(amount AS REAL) AS TEXT),
    tx_type = 'debit',
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE account_id IN (
    SELECT account_id FROM accounts WHERE account_type = 'credit'
)
AND CAST(amount AS REAL) > 0
AND COALESCE(category_id, '') NOT IN ('credit-card-payment', 'cashback', 'refund');
