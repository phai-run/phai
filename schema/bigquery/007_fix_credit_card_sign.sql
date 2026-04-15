-- Migration 007: fix credit-card debit sign
-- Pluggy returns credit-card purchases with positive amounts.
-- Normalize to negative so the invariant "negative = expense" holds.

UPDATE `{{project_id}}.{{dataset_id}}.transactions` t
SET t.amount = -1 * t.amount,
    t.updated_at = CURRENT_TIMESTAMP()
WHERE t.account_id IN (
    SELECT account_id FROM `{{project_id}}.{{dataset_id}}.accounts` WHERE account_type = 'credit'
)
AND t.tx_type = 'debit'
AND t.amount > 0;
