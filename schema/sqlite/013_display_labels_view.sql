DROP VIEW IF EXISTS v_daily_pulse;
DROP VIEW IF EXISTS v_monthly_spend;
DROP VIEW IF EXISTS v_cashflow;
DROP VIEW IF EXISTS v_forecast_vs_actual;
DROP VIEW IF EXISTS v_card_summary;
DROP VIEW IF EXISTS v_uncategorized;
DROP VIEW IF EXISTS v_transactions_effective;

CREATE VIEW v_transactions_effective AS
SELECT
  transaction_id,
  account_id,
  transaction_date,
  description,
  amount,
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
    WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR CAST(amount AS REAL) > 0 THEN '💰'
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
          WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR CAST(amount AS REAL) > 0 THEN '💰'
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
          WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR CAST(amount AS REAL) > 0 THEN '💰'
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
          WHEN category_id LIKE 'receitas%' OR category_id = 'salario' OR CAST(amount AS REAL) > 0 THEN '💰'
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
FROM v_transactions_effective
ORDER BY transaction_date DESC, updated_at DESC;

CREATE VIEW v_monthly_spend AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  COALESCE(category_id, 'sem-categoria') AS category_id,
  COALESCE(account_id, 'sem-conta') AS account_id,
  CAST(ROUND(SUM(ABS(CAST(amount AS REAL))), 2) AS TEXT) AS expenses,
  COUNT(*) AS expense_count
FROM v_transactions_effective
WHERE CAST(amount AS REAL) < 0
  AND COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1, 2, 3;

CREATE VIEW v_cashflow AS
SELECT
  strftime('%Y-%m', transaction_date) AS month_ref,
  CAST(ROUND(SUM(CASE WHEN CAST(amount AS REAL) > 0 THEN CAST(amount AS REAL) ELSE 0 END), 2) AS TEXT) AS income,
  CAST(ROUND(SUM(CASE WHEN CAST(amount AS REAL) < 0 THEN ABS(CAST(amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS expenses,
  CAST(ROUND(SUM(CAST(amount AS REAL)), 2) AS TEXT) AS net
FROM v_transactions_effective
WHERE COALESCE(category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1;

CREATE VIEW v_forecast_vs_actual AS
WITH monthly_actuals AS (
  SELECT
    strftime('%Y-%m', transaction_date) AS month_ref,
    COALESCE(account_id, '') AS account_id,
    COALESCE(category_id, '') AS category_id,
    ROUND(SUM(ABS(CAST(amount AS REAL))), 2) AS actual_total
  FROM v_transactions_effective
  WHERE CAST(amount AS REAL) < 0
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

CREATE VIEW v_card_summary AS
SELECT
  strftime('%Y-%m', t.transaction_date) AS month_ref,
  t.account_id,
  CAST(ROUND(SUM(CASE WHEN CAST(t.amount AS REAL) < 0 THEN ABS(CAST(t.amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS total_charges,
  CAST(ROUND(SUM(CASE WHEN t.payment_status IN ('pending', 'em_aberto', 'parcial') THEN ABS(CAST(t.amount AS REAL)) ELSE 0 END), 2) AS TEXT) AS open_amount,
  COUNT(CASE WHEN CAST(t.amount AS REAL) < 0 THEN 1 END) AS transaction_count
FROM v_transactions_effective t
JOIN accounts a ON a.account_id = t.account_id
WHERE a.account_type = 'credit'
  AND COALESCE(t.category_id, '') NOT IN (SELECT category_id FROM internal_categories)
GROUP BY 1, 2;

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
FROM v_transactions_effective
WHERE category_id IS NULL
   OR category_source IN ('unclassified', 'fallback');
