-- Migration 037: cash-flow basis canonical model (`cash_month`).
--
-- Establishes a single canonical "cash month" per reportable transaction so
-- every surface (cashflow chart, CLI reports, web month-detail, pulse) buckets
-- the same way and stops drifting apart:
--   * non-card accounts: the transaction's own month — cash moves immediately;
--   * credit cards: the month the bill that CONTAINS the purchase is DUE/PAID,
--     derived from `billing_closing_day` (which cycle the purchase closes in)
--     plus `billing_due_day` (a one-month roll when the due day precedes the
--     closing day). This is the family's regime de caixa — a bill paid in May
--     surfaces its individual purchases as May outflows, not as a lump payment
--     and not on the original purchase dates.
--
-- The credit-card bill *payment* transaction stays in `internal_categories`
-- (already excluded by every report view), so the exploded purchases replace
-- it without double counting.
--
-- `v_cashflow` and `v_monthly_spend` are redefined to group by `cash_month`
-- instead of the posting month. A charge dated on or after the closing day
-- belongs to the cycle that closes next month (the closing day is the first day
-- of the new cycle — Nubank's OFX DTSTART is inclusive); this corrects the
-- off-by-one in `v_card_summary`'s `<= closing_day` boundary. See ADR-0025,
-- which supersedes ADR-0024 (accrual posting-month bucketing).

DROP VIEW IF EXISTS v_cashflow;
DROP VIEW IF EXISTS v_monthly_spend;
DROP VIEW IF EXISTS v_transactions_cashbasis;

CREATE VIEW v_transactions_cashbasis AS
SELECT
  t.*,
  CASE
    WHEN a.account_type = 'credit'
         AND COALESCE(NULLIF(json_extract(a.metadata_json, '$.billing_closing_day'), ''), '') != ''
    THEN strftime(
      '%Y-%m',
      date(
        t.transaction_date,
        'start of month',
        CAST(
          (CASE
             WHEN CAST(strftime('%d', t.transaction_date) AS INTEGER)
                  >= CAST(json_extract(a.metadata_json, '$.billing_closing_day') AS INTEGER)
             THEN 1 ELSE 0
           END)
          +
          (CASE
             WHEN COALESCE(NULLIF(json_extract(a.metadata_json, '$.billing_due_day'), ''), '') != ''
                  AND CAST(json_extract(a.metadata_json, '$.billing_due_day') AS INTEGER)
                      < CAST(json_extract(a.metadata_json, '$.billing_closing_day') AS INTEGER)
             THEN 1 ELSE 0
           END)
          AS TEXT
        ) || ' months'
      )
    )
    ELSE strftime('%Y-%m', t.transaction_date)
  END AS cash_month
FROM v_transactions_reportable t
LEFT JOIN accounts a ON a.account_id = t.account_id;

CREATE VIEW v_monthly_spend AS
SELECT
  cash_month AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  CAST(ROUND(SUM(ABS(amount_cents)) / 100.0, 2) AS TEXT) AS expenses,
  COUNT(*) AS expense_count
FROM v_transactions_cashbasis
WHERE amount_cents < 0
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1, 2, 3;

CREATE VIEW v_cashflow AS
SELECT
  cash_month AS month_ref,
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
FROM v_transactions_cashbasis
WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1;
