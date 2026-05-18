-- Migration 021: canonicalise payment_status vocabulary.
--
-- Before this migration, `payment_status` was a free-form bag of values
-- emitted by whichever code path created the row:
--   - `pago`/`posted`            — duplicate PT/EN for "finalised on a closed bill"
--   - `em_aberto`/`pending`      — duplicate PT/EN for "on the current open bill"
--   - `parcial`                  — Pluggy's marker for "future parcela of an installment"
--   - `confirmed`/`unconfirmed`  — internal aliases set by manual entry paths
--
-- v_card_summary.open_amount summed `pending + em_aberto + parcial`, which
-- inflated the "open right now" balance with future installments that aren't
-- yet on any bill. ADR-0011 picks three canonical values and aligns the rest
-- of the codebase around them:
--
--   posted      — final on a closed bill (or settled on a checking account)
--   pending     — on the current open cycle, awaiting closure
--   installment — future parcela of an installment chain
--
-- This migration:
-- 1. UPDATEs existing rows to the canonical vocabulary (idempotent on re-run).
-- 2. Rewrites v_card_summary so `open_amount` sums only `pending`, plus a
--    new `installments_future` column that surfaces parcela exposure.
-- 3. Rewrites v_card_open_now to follow the new v_card_summary shape.

UPDATE transactions SET payment_status = 'posted'      WHERE payment_status IN ('pago', 'confirmed');
UPDATE transactions SET payment_status = 'pending'     WHERE payment_status IN ('em_aberto', 'unconfirmed');
UPDATE transactions SET payment_status = 'installment' WHERE payment_status = 'parcial';

DROP VIEW IF EXISTS v_card_open_now;
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
  CAST(ROUND(SUM(CASE WHEN payment_status = 'pending' THEN ABS(CAST(amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS open_amount,
  CAST(ROUND(SUM(CASE WHEN payment_status = 'installment' THEN ABS(CAST(amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS installments_future,
  COUNT(CASE WHEN CAST(amount AS REAL) < 0 THEN 1 END) AS transaction_count
FROM cycles
WHERE account_type = 'credit'
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1, 2;

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
  cs.installments_future,
  cs.transaction_count
FROM v_card_summary cs
JOIN latest_open lo
  ON lo.account_id = cs.account_id
  AND lo.month_ref = cs.month_ref;
