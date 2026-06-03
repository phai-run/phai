-- Migration 039: converge v_card_summary's cycle boundary with cash_month.
--
-- v_card_summary used `day <= closing_day → current cycle`, but the canonical
-- cash_month (migration 037) and `compute_bill_id` use `day < closing_day →
-- current cycle` (a charge ON the closing day is the first of the next cycle —
-- Nubank's OFX DTSTART is inclusive). The card panel (cards_open_now →
-- v_card_summary) and the cashflow chart (cash_month) therefore disagreed on a
-- charge dated exactly on the closing day. This aligns v_card_summary to the
-- same `< closing_day` boundary so every card surface shares one cycle rule.
-- See ADR-0025/0026. Only the boundary changes; totals and columns are identical.

DROP VIEW IF EXISTS v_card_open_now;
DROP VIEW IF EXISTS v_card_summary;

CREATE VIEW v_card_summary AS
WITH cycles AS (
  SELECT
    t.transaction_id,
    t.transaction_date,
    t.amount,
    t.amount_cents,
    t.account_id,
    t.payment_status,
    t.category_id,
    a.account_type,
    CASE
      WHEN COALESCE(NULLIF(json_extract(a.metadata_json, '$.billing_closing_day'), ''), '') = ''
        THEN strftime('%Y-%m', t.transaction_date)
      WHEN CAST(strftime('%d', t.transaction_date) AS INTEGER)
           < CAST(json_extract(a.metadata_json, '$.billing_closing_day') AS INTEGER)
        THEN strftime('%Y-%m', t.transaction_date)
      ELSE strftime('%Y-%m', date(t.transaction_date, 'start of month', '+1 month'))
    END AS cycle_ref
  FROM v_transactions_reportable t
  JOIN accounts a ON a.account_id = t.account_id
)
SELECT
  cycle_ref AS month_ref,
  account_id,
  CAST(ROUND(SUM(CASE WHEN amount_cents < 0 THEN ABS(amount_cents) ELSE 0 END) / 100.0, 2) AS TEXT) AS total_charges,
  CAST(ROUND(SUM(CASE WHEN payment_status = 'pending' THEN ABS(amount_cents) ELSE 0 END) / 100.0, 2) AS TEXT) AS open_amount,
  CAST(ROUND(SUM(CASE WHEN payment_status = 'installment' THEN ABS(amount_cents) ELSE 0 END) / 100.0, 2) AS TEXT) AS installments_future,
  COUNT(CASE WHEN amount_cents < 0 THEN 1 END) AS transaction_count,
  SUM(CASE WHEN payment_status = 'pending' THEN ABS(amount_cents) ELSE 0 END) AS open_amount_cents
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
  WHERE open_amount_cents > 0
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
