-- Migration 030: dedup parity placeholder for SQLite (no-op).
--
-- Companion to schema/bigquery/030_dedup_cashflow_card_summary.sql so the
-- numeric prefix in both backends stays aligned (AGENTS.md §3 Migrations).
-- The actual dedup rewrite for SQLite landed in migration 029; this file
-- exists purely so the SQLite ledger does not skip 030 while BigQuery
-- carries one.

SELECT 1;
