-- Migration 029: route v_cashflow and v_card_summary through
-- v_transactions_reportable to honour the legacy/manual ↔ Pluggy dedup
-- filter.
--
-- Migration 028 rewrote these aggregates against `transactions` directly,
-- which is correct for exact-integer SUM but silently dropped the dedup
-- predicate that v_transactions_reportable enforces (manual_* legacy rows
-- shadowed by a Pluggy row within ±7 days at the same account/cents).
-- The result was double-counting of shadowed expenses in cashflow totals
-- and credit-card charges. This migration restores the filter while
-- keeping the integer-cent aggregation.
--
-- The downstream view v_card_open_now depends on v_card_summary, so it is
-- rebuilt as well to keep the dependency chain consistent.
--
-- BigQuery mirror is 030_dedup_cashflow_card_summary.sql (no-op — the BQ
-- views were never rewritten in 029, so the dedup filter still applies via
-- v_transactions_reportable there).

DROP VIEW IF EXISTS v_card_open_now;
DROP VIEW IF EXISTS v_card_summary;
DROP VIEW IF EXISTS v_cashflow;

CREATE VIEW v_cashflow AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  CAST(ROUND(SUM(
    CASE
      WHEN amount_cents > 0 AND COALESCE(category_id, '') != 'cashback'
        THEN amount_cents
      ELSE 0
    END
  ) / 100.0, 2) AS TEXT) AS income,
  CAST(ROUND(SUM(
    CASE
      WHEN amount_cents < 0 THEN ABS(amount_cents)
      ELSE 0
    END
  ) / 100.0, 2) AS TEXT) AS expenses_gross,
  CAST(ROUND(SUM(
    CASE
      WHEN amount_cents > 0 AND COALESCE(category_id, '') = 'cashback'
        THEN amount_cents
      ELSE 0
    END
  ) / 100.0, 2) AS TEXT) AS expense_reduction,
  CAST(ROUND(SUM(
    CASE
      WHEN amount_cents < 0 THEN ABS(amount_cents)
      ELSE 0
    END
  ) / 100.0, 2) AS TEXT) AS expenses,
  CAST(ROUND(SUM(amount_cents) / 100.0, 2) AS TEXT) AS net
FROM v_transactions_reportable
WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1;

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
           <= CAST(json_extract(a.metadata_json, '$.billing_closing_day') AS INTEGER)
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
