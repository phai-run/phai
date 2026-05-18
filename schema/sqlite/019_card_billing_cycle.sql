-- Migration 019: redefine v_card_summary to bucket by billing cycle, not calendar month.
--
-- A credit-card purchase on day D for an account with billing_closing_day = C
-- belongs to the bill that CLOSES on the next C-th day after D:
--   - if D <= C, the purchase lands on the bill closing this month (cycle_ref = this month)
--   - if D > C,  the purchase lands on the bill closing next month (cycle_ref = next month)
--
-- This matches how Brazilian credit cards actually bill the customer and how
-- users mentally segment purchases. The previous view grouped by
-- strftime('%Y-%m', transaction_date) which placed a March-28th purchase on
-- the closing-day-3 card into "March" even though the user pays it in April.
--
-- For accounts whose metadata_json has no billing_closing_day (corporate
-- meal-voucher cards like Flash that bill differently), we fall back to the
-- calendar month so the data still groups cleanly.
--
-- Internal-category exclusion now reads from the internal_categories table
-- (added in 010) instead of the hardcoded 'credit-card-payment' literal so
-- bill payments classified under the Portuguese taxonomy
-- (financeiro:pagamento-de-fatura-de-cartao) are also excluded.

DROP VIEW IF EXISTS v_card_summary;

CREATE VIEW v_card_summary AS
WITH cycles AS (
  SELECT
    t.transaction_id,
    t.transaction_date,
    t.amount,
    t.account_id,
    t.payment_status,
    t.category_id,
    a.account_type,
    CASE
      WHEN COALESCE(NULLIF(json_extract(a.metadata_json, '$.billing_closing_day'), ''), '') = ''
        THEN strftime('%Y-%m', t.transaction_date)
      WHEN CAST(strftime('%d', t.transaction_date) AS INTEGER)
           <= CAST(json_extract(a.metadata_json, '$.billing_closing_day') AS INTEGER)
        THEN strftime('%Y-%m', t.transaction_date)
      ELSE strftime('%Y-%m', date(t.transaction_date, 'start of month', '+1 month'))
    END AS cycle_ref
  FROM transactions t
  JOIN accounts a ON a.account_id = t.account_id
)
SELECT
  cycle_ref AS month_ref,
  account_id,
  CAST(ROUND(SUM(CASE WHEN CAST(amount AS REAL) < 0 THEN ABS(CAST(amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS total_charges,
  CAST(ROUND(SUM(CASE WHEN payment_status IN ('pending', 'em_aberto', 'parcial') THEN ABS(CAST(amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS open_amount,
  COUNT(CASE WHEN CAST(amount AS REAL) < 0 THEN 1 END) AS transaction_count
FROM cycles
WHERE account_type = 'credit'
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1, 2;

-- v_card_open_now is introduced separately in migration 020.
