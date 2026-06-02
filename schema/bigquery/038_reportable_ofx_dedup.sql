-- Migration 038: fold the ofx-vs-pluggy dedup into v_transactions_reportable.
--
-- The ofx-shadowed-by-pluggy dedup previously lived ONLY in the Rust
-- `cashflow_reportable` method and the serve transaction list — a second source
-- of truth outside the view chain. That made the cashflow chart (deduped) and
-- per-category report `monthly-spend` (the v_monthly_spend view, not deduped)
-- disagree. Moving the dedup into the base `v_transactions_reportable` view
-- makes EVERY downstream view (v_transactions_cashbasis, v_cashflow,
-- v_monthly_spend, v_card_summary) and every report read one deduped source.
-- See ADR-0026.
--
-- A row from an OFX import is dropped when a Pluggy row exists for the same
-- (transaction_date, account_id, amount_cents, normalised raw_description) —
-- Pluggy is the canonical source. The pre-existing legacy-manual dedup is
-- preserved. Both dedups use LEFT JOIN anti-joins (not correlated EXISTS):
-- BigQuery cannot de-correlate the EXISTS form through the nested
-- cashbasis/cashflow views ("Correlated subqueries ... not supported"). Anti-
-- joins do not fan out kept rows: a kept row has no matching Pluggy row, so its
-- LEFT JOIN yields a single NULL-extended row.

CREATE OR REPLACE VIEW `{{project_id}}.{{dataset_id}}.v_transactions_reportable` AS
SELECT
  t.transaction_id,
  t.account_id,
  t.transaction_date,
  t.raw_description,
  t.description,
  t.merchant_name,
  t.purpose,
  t.amount,
  t.amount_cents,
  t.tx_type,
  t.category_id,
  t.category_source,
  t.context,
  t.classifier_trace,
  t.payment_status,
  t.source,
  t.actor_id,
  t.idempotency_key,
  t.metadata_json,
  t.created_at,
  t.updated_at,
  t.enrichment_attempted_at,
  t.display_emoji,
  t.display_label,
  t.category_display
FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective` t
LEFT JOIN (
  SELECT account_id, amount, transaction_date, 1 AS matched
  FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective`
  WHERE source = 'pluggy'
) legacy_match
  ON t.source = 'legacy'
  AND STARTS_WITH(t.transaction_id, 'manual_')
  AND legacy_match.account_id = t.account_id
  AND legacy_match.amount = t.amount
  AND legacy_match.transaction_date
        BETWEEN DATE_SUB(t.transaction_date, INTERVAL 7 DAY)
        AND DATE_ADD(t.transaction_date, INTERVAL 7 DAY)
LEFT JOIN (
  SELECT
    COALESCE(account_id, '') AS account_key,
    amount_cents,
    transaction_date,
    LOWER(TRIM(raw_description)) AS desc_key,
    1 AS matched
  FROM `{{project_id}}.{{dataset_id}}.v_transactions_effective`
  WHERE source = 'pluggy'
) ofx_match
  ON t.source = 'ofx'
  AND ofx_match.account_key = COALESCE(t.account_id, '')
  AND ofx_match.amount_cents = t.amount_cents
  AND ofx_match.transaction_date = t.transaction_date
  AND ofx_match.desc_key = LOWER(TRIM(t.raw_description))
WHERE legacy_match.matched IS NULL
  AND ofx_match.matched IS NULL;
