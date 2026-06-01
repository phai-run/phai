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

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_transactions_cashbasis` AS
SELECT
  t.*,
  CASE
    WHEN a.account_type = 'credit'
         AND COALESCE(NULLIF(JSON_VALUE(a.metadata_json, '$.billing_closing_day'), ''), '') != ''
    THEN FORMAT_DATE(
      '%Y-%m',
      DATE_ADD(
        DATE_TRUNC(t.transaction_date, MONTH),
        INTERVAL (
          (CASE
             WHEN EXTRACT(DAY FROM t.transaction_date)
                  >= CAST(JSON_VALUE(a.metadata_json, '$.billing_closing_day') AS INT64)
             THEN 1 ELSE 0
           END)
          +
          (CASE
             WHEN COALESCE(NULLIF(JSON_VALUE(a.metadata_json, '$.billing_due_day'), ''), '') != ''
                  AND CAST(JSON_VALUE(a.metadata_json, '$.billing_due_day') AS INT64)
                      < CAST(JSON_VALUE(a.metadata_json, '$.billing_closing_day') AS INT64)
             THEN 1 ELSE 0
           END)
        ) MONTH
      )
    )
    ELSE FORMAT_DATE('%Y-%m', t.transaction_date)
  END AS cash_month
FROM `{{project_id}}.{{dataset_id}}.v_transactions_reportable` t
LEFT JOIN `{{project_id}}.{{dataset_id}}.accounts` a
  ON a.account_id = t.account_id;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_monthly_spend` AS
SELECT
  cash_month AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  ROUND(SUM(ABS(amount)), 2) AS expenses,
  COUNT(*) AS expense_count
FROM `{{project_id}}.{{dataset_id}}.v_transactions_cashbasis`
WHERE amount_cents < 0
  AND COALESCE(category_id, '') NOT IN (
    SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`
  )
GROUP BY 1, 2, 3;

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_cashflow` AS
SELECT
  cash_month AS month_ref,
  ROUND(SUM(IF(amount_cents > 0 AND COALESCE(category_id, '') != 'cashback',
    amount_cents, 0)) / 100.0, 2) AS income,
  ROUND(SUM(IF(amount_cents < 0, ABS(amount_cents), 0)) / 100.0, 2) AS expenses_gross,
  ROUND(SUM(IF(amount_cents > 0 AND COALESCE(category_id, '') = 'cashback',
    amount_cents, 0)) / 100.0, 2) AS expense_reduction,
  ROUND(SUM(IF(amount_cents < 0, ABS(amount_cents), 0)) / 100.0, 2) AS expenses,
  ROUND(SUM(amount_cents) / 100.0, 2) AS net
FROM `{{project_id}}.{{dataset_id}}.v_transactions_cashbasis`
WHERE COALESCE(category_id, '') NOT IN (
  SELECT category_id FROM `{{project_id}}.{{dataset_id}}.internal_categories`
)
GROUP BY 1;
