-- Migration 020: fix v_card_open_now semantics.
--
-- The previous v_card_open_now (introduced in 019) returned the cycle that
-- was STILL accruing charges. The user-facing intent is the opposite —
-- "what do you owe right now?" — which is the most recent CLOSED cycle
-- that has an open balance.
--
-- This redefinition addresses the only release where 019 might already be
-- applied (dev/local). SQLite has no DROP-then-CREATE OR REPLACE, so we
-- drop explicitly.

DROP VIEW IF EXISTS v_card_open_now;

CREATE VIEW v_card_open_now AS
WITH latest_open AS (
  SELECT
    account_id,
    MAX(month_ref) AS month_ref
  FROM v_card_summary
  WHERE CAST(open_amount AS REAL) > 0
  GROUP BY account_id
)
SELECT
  cs.month_ref,
  cs.account_id,
  cs.total_charges,
  cs.open_amount,
  cs.transaction_count
FROM v_card_summary cs
JOIN latest_open lo
  ON lo.account_id = cs.account_id
  AND lo.month_ref = cs.month_ref;
