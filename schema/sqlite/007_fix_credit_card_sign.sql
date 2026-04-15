-- Migration 007: fix credit-card debit sign
-- Pluggy returns credit-card purchases with positive amounts.
-- Normalize to negative so the invariant "negative = expense" holds.

UPDATE transactions
SET amount = CAST(-1 * CAST(amount AS REAL) AS TEXT),
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE account_id IN (
    SELECT account_id FROM accounts WHERE account_type = 'credit'
)
AND tx_type = 'debit'
AND CAST(amount AS REAL) > 0;
