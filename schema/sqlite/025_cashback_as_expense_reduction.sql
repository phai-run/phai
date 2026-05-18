-- Migration 025: surface cashback as expense-reduction in v_cashflow.
--
-- Cashback events arrive from Pluggy as positive amounts on the credit
-- account ("Resgate de Cashback +R$ 1.117,18"). Before this migration
-- they fell into v_cashflow.income, indistinguishable from a salary
-- credit. That over-states income, and under-states the household's
-- effective spend — the cashback is a refund on past purchases, not new
-- money entering the system.
--
-- The cleanest model is: cashback is *negative expense*. v_cashflow
-- gains an `expense_reduction` column that sums cashback events
-- separately, and `expenses` is reported net of it. Income stays clean
-- (only actual money inflows: salary, transfers in, etc.).
--
-- We bucket by category_id = 'cashback' (the leaf slug Pluggy emits).
-- If a user later renames cashback to a custom slug they'd need a rule
-- that maps to category_id 'cashback' — same pattern as internal_categories.

DROP VIEW IF EXISTS v_cashflow;

CREATE VIEW v_cashflow AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  CAST(ROUND(SUM(
    CASE
      WHEN CAST(amount AS REAL) > 0 AND COALESCE(category_id, '') != 'cashback'
        THEN CAST(amount AS REAL)
      ELSE 0
    END
  ), 2) AS TEXT) AS income,
  CAST(ROUND(SUM(
    CASE
      WHEN CAST(amount AS REAL) < 0 THEN ABS(CAST(amount AS REAL))
      ELSE 0
    END
  ), 2) AS TEXT) AS expenses_gross,
  CAST(ROUND(SUM(
    CASE
      WHEN CAST(amount AS REAL) > 0 AND COALESCE(category_id, '') = 'cashback'
        THEN CAST(amount AS REAL)
      ELSE 0
    END
  ), 2) AS TEXT) AS expense_reduction,
  -- `expenses` keeps the old contract for back-compat (gross expenses),
  -- callers that want the net should subtract expense_reduction.
  CAST(ROUND(SUM(
    CASE
      WHEN CAST(amount AS REAL) < 0 THEN ABS(CAST(amount AS REAL))
      ELSE 0
    END
  ), 2) AS TEXT) AS expenses,
  -- `net` is reported using the cashback-adjusted view: income includes
  -- salaries etc. but not cashback; expenses include the gross outflow.
  -- The relationship is: net = income - (expenses - expense_reduction).
  CAST(ROUND(SUM(CAST(amount AS REAL)), 2) AS TEXT) AS net
FROM transactions
WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1;
