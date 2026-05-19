-- Migration 028: rewrite views to use amount_cents for exact decimal math.
--
-- ADR-0003 requires decimal-precise aggregation. Before this migration,
-- views used SUM(CAST(amount AS REAL)) which accumulates floating-point
-- errors over many rows. With amount_cents (migration 027), we can SUM
-- exact integer cents and divide by 100.0 only at the end.
--
-- Views rewritten:
--   v_transactions_effective  — adds amount_cents column to projection
--   v_transactions_reportable — amount_cents comparison (was CAST AS REAL =)
--   v_monthly_spend           — SUM(ABS(amount_cents)) / 100.0
--   v_cashflow                — all aggregations via amount_cents
--   v_forecast_vs_actual      — actual_total via SUM(ABS(amount_cents))
--   v_card_summary            — aggregations via amount_cents + open_amount_cents
--   v_card_open_now           — open_amount_cents > 0 (was CAST AS REAL > 0)
--   v_daily_pulse, v_uncategorized — passthrough (rebuild for dependency chain)
--
-- BigQuery mirror is 029_amount_cents_views.sql (no-op — NUMERIC is precise).
--
-- All views preserve their output column names and TEXT types for
-- backward compatibility with Rust consumers.

-- ═══════════════════════════════════════════════════════════════════════════
-- Tear down in dependency order (leaf views first)
-- ═══════════════════════════════════════════════════════════════════════════

DROP VIEW IF EXISTS v_card_open_now;
DROP VIEW IF EXISTS v_card_summary;
DROP VIEW IF EXISTS v_forecast_vs_actual;
DROP VIEW IF EXISTS v_cashflow;
DROP VIEW IF EXISTS v_monthly_spend;
DROP VIEW IF EXISTS v_daily_pulse;
DROP VIEW IF EXISTS v_uncategorized;
DROP VIEW IF EXISTS v_transactions_reportable;
DROP VIEW IF EXISTS v_transactions_effective;

-- ═══════════════════════════════════════════════════════════════════════════
-- 1. v_transactions_effective — add amount_cents to projection
-- ═══════════════════════════════════════════════════════════════════════════

CREATE VIEW v_transactions_effective AS
SELECT
  transaction_id,
  account_id,
  transaction_date,
  description,
  amount,
  amount_cents,
  tx_type,
  category_id,
  category_source,
  context,
  payment_status,
  source,
  actor_id,
  idempotency_key,
  metadata_json,
  created_at,
  updated_at,
  CASE
    WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
    WHEN category_id LIKE 'transfer%' THEN '🔁'
    WHEN category_id LIKE 'assinaturas%' THEN '🔂'
    WHEN category_id LIKE 'moradia%' OR category_id LIKE 'casa%' THEN '🏠'
    WHEN category_id LIKE 'alimentacao%' THEN '🍽️'
    WHEN category_id LIKE 'saude%' THEN '🩺'
    WHEN category_id LIKE 'transporte%' OR category_id LIKE 'mobilidade%' THEN '🚗'
    WHEN category_id LIKE 'educacao%' THEN '📚'
    WHEN category_id LIKE 'lazer%' THEN '🎉'
    WHEN category_id LIKE 'investimentos%' THEN '📈'
    WHEN category_id LIKE 'financeiro%' THEN '🧾'
    WHEN category_id IS NULL THEN '❓'
    ELSE '💸'
  END AS display_emoji,
  CASE
    WHEN TRIM(COALESCE(context, '')) <> '' THEN
      (
        CASE
          WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
          WHEN category_id LIKE 'transfer%' THEN '🔁'
          WHEN category_id LIKE 'assinaturas%' THEN '🔂'
          WHEN category_id LIKE 'moradia%' OR category_id LIKE 'casa%' THEN '🏠'
          WHEN category_id LIKE 'alimentacao%' THEN '🍽️'
          WHEN category_id LIKE 'saude%' THEN '🩺'
          WHEN category_id LIKE 'transporte%' OR category_id LIKE 'mobilidade%' THEN '🚗'
          WHEN category_id LIKE 'educacao%' THEN '📚'
          WHEN category_id LIKE 'lazer%' THEN '🎉'
          WHEN category_id LIKE 'investimentos%' THEN '📈'
          WHEN category_id LIKE 'financeiro%' THEN '🧾'
          WHEN category_id IS NULL THEN '❓'
          ELSE '💸'
        END
      ) || ' ' || TRIM(context)
    ELSE
      (
        CASE
          WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
          WHEN category_id LIKE 'transfer%' THEN '🔁'
          WHEN category_id LIKE 'assinaturas%' THEN '🔂'
          WHEN category_id LIKE 'moradia%' OR category_id LIKE 'casa%' THEN '🏠'
          WHEN category_id LIKE 'alimentacao%' THEN '🍽️'
          WHEN category_id LIKE 'saude%' THEN '🩺'
          WHEN category_id LIKE 'transporte%' OR category_id LIKE 'mobilidade%' THEN '🚗'
          WHEN category_id LIKE 'educacao%' THEN '📚'
          WHEN category_id LIKE 'lazer%' THEN '🎉'
          WHEN category_id LIKE 'investimentos%' THEN '📈'
          WHEN category_id LIKE 'financeiro%' THEN '🧾'
          WHEN category_id IS NULL THEN '❓'
          ELSE '💸'
        END
      ) || ' ' || TRIM(description)
  END AS display_label,
  CASE
    WHEN category_id IS NULL OR TRIM(category_id) = '' THEN '❓ sem categoria'
    ELSE
      (
        CASE
          WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR amount_cents > 0 THEN '💰'
          WHEN category_id LIKE 'transfer%' THEN '🔁'
          WHEN category_id LIKE 'assinaturas%' THEN '🔂'
          WHEN category_id LIKE 'moradia%' OR category_id LIKE 'casa%' THEN '🏠'
          WHEN category_id LIKE 'alimentacao%' THEN '🍽️'
          WHEN category_id LIKE 'saude%' THEN '🩺'
          WHEN category_id LIKE 'transporte%' OR category_id LIKE 'mobilidade%' THEN '🚗'
          WHEN category_id LIKE 'educacao%' THEN '📚'
          WHEN category_id LIKE 'lazer%' THEN '🎉'
          WHEN category_id LIKE 'investimentos%' THEN '📈'
          WHEN category_id LIKE 'financeiro%' THEN '🧾'
          ELSE '💸'
        END
      ) || ' ' || REPLACE(REPLACE(category_id, ':', ' > '), '-', ' ')
  END AS category_display
FROM transactions;

-- ═══════════════════════════════════════════════════════════════════════════
-- 2. v_transactions_reportable — exact cents comparison
-- ═══════════════════════════════════════════════════════════════════════════

CREATE VIEW v_transactions_reportable AS
SELECT
  t.transaction_id,
  t.account_id,
  t.transaction_date,
  t.description,
  t.amount,
  t.amount_cents,
  t.tx_type,
  t.category_id,
  t.category_source,
  t.context,
  t.payment_status,
  t.source,
  t.actor_id,
  t.idempotency_key,
  t.metadata_json,
  t.created_at,
  t.updated_at,
  t.display_emoji,
  t.display_label,
  t.category_display
FROM v_transactions_effective t
WHERE NOT (
  t.source = 'legacy'
  AND t.transaction_id LIKE 'manual_%'
  AND EXISTS (
    SELECT 1
    FROM v_transactions_effective p
    WHERE p.source = 'pluggy'
      AND p.account_id = t.account_id
      AND p.amount_cents = t.amount_cents
      AND p.transaction_date BETWEEN date(t.transaction_date, '-7 day') AND date(t.transaction_date, '+7 day')
  )
);

-- ═══════════════════════════════════════════════════════════════════════════
-- 3. v_daily_pulse — passthrough, no CAST changes needed
-- ═══════════════════════════════════════════════════════════════════════════

CREATE VIEW v_daily_pulse AS
SELECT
  transaction_id,
  transaction_date,
  display_label AS description,
  amount,
  category_id,
  source,
  payment_status,
  account_id
FROM v_transactions_reportable
ORDER BY transaction_date DESC, updated_at DESC;

-- ═══════════════════════════════════════════════════════════════════════════
-- 4. v_monthly_spend — integer-cent SUM
-- ═══════════════════════════════════════════════════════════════════════════

CREATE VIEW v_monthly_spend AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  CAST(ROUND(SUM(ABS(amount_cents)) / 100.0, 2) AS TEXT) AS expenses,
  COUNT(*) AS expense_count
FROM v_transactions_reportable
WHERE amount_cents < 0
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1, 2, 3;

-- ═══════════════════════════════════════════════════════════════════════════
-- 5. v_cashflow — all aggregations via amount_cents
-- ═══════════════════════════════════════════════════════════════════════════

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
FROM transactions
WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1;

-- ═══════════════════════════════════════════════════════════════════════════
-- 6. v_forecast_vs_actual — exact actual_total from cents
--
-- Note on variance: `forecast.amount` is stored as TEXT and has no
-- amount_cents companion column (forecast is a small, manually curated
-- table, not a high-volume aggregate input). The `variance` expression
-- therefore still goes through CAST AS REAL. Drift is bounded to the row
-- count of `forecast` (typically << 100), so the cents strategy from
-- ADR-0003 is intentionally not extended here. If forecast ever grows
-- into a hot aggregation path, add forecast.amount_cents in a follow-up.
-- ═══════════════════════════════════════════════════════════════════════════

CREATE VIEW v_forecast_vs_actual AS
WITH monthly_actuals AS (
  SELECT
    strftime('%Y-%m', transaction_date) AS month_ref,
    COALESCE(account_id, '') AS account_id,
    COALESCE(category_id, '') AS category_id,
    ROUND(SUM(ABS(amount_cents)) / 100.0, 2) AS actual_total
  FROM v_transactions_reportable
  WHERE amount_cents < 0
    AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
  GROUP BY 1, 2, 3
)
SELECT
  f.forecast_id,
  COALESCE(strftime('%Y-%m', f.due_date), strftime('%Y-%m', f.created_at)) AS month_ref,
  f.due_date,
  f.description,
  f.account_id,
  f.category_id,
  f.amount AS forecast_amount,
  CAST(COALESCE(ma.actual_total, 0) AS TEXT) AS actual_amount,
  CAST(ROUND(CAST(f.amount AS REAL) - COALESCE(ma.actual_total, 0), 2) AS TEXT) AS variance,
  f.status
FROM forecast f
LEFT JOIN monthly_actuals ma
  ON ma.month_ref = COALESCE(strftime('%Y-%m', f.due_date), strftime('%Y-%m', f.created_at))
  AND ma.account_id = COALESCE(f.account_id, '')
  AND ma.category_id = COALESCE(f.category_id, '');

-- ═══════════════════════════════════════════════════════════════════════════
-- 7. v_card_summary — integer-cent aggregations + open_amount_cents
-- ═══════════════════════════════════════════════════════════════════════════

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
  FROM transactions t
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

-- ═══════════════════════════════════════════════════════════════════════════
-- 8. v_card_open_now — exact cents comparison
-- ═══════════════════════════════════════════════════════════════════════════

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

-- ═══════════════════════════════════════════════════════════════════════════
-- 9. v_uncategorized — passthrough, rebuilt for dependency chain
-- ═══════════════════════════════════════════════════════════════════════════

CREATE VIEW v_uncategorized AS
SELECT
  transaction_id,
  transaction_date,
  display_label AS description,
  amount,
  account_id,
  category_source,
  payment_status,
  source
FROM v_transactions_reportable
WHERE category_id IS NULL
   OR category_source IN ('unclassified', 'fallback');
