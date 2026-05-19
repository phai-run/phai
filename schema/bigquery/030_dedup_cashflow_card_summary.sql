-- Migration 030: dedup parity for cashflow / card summary (BigQuery no-op).
--
-- Mirror of schema/sqlite/029_dedup_cashflow_card_summary.sql. BigQuery's
-- aggregate views were never rewritten in migration 029 (NUMERIC is exact
-- by construction), so they still target v_transactions_reportable and the
-- legacy/manual ↔ Pluggy dedup filter is already enforced. No rewrite is
-- needed; this file exists for cross-backend numbering parity.

SELECT 1;
