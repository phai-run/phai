-- Migration 034: fix v_card_summary double-counting (SQLite no-op).
--
-- The double-counting issue only affects BigQuery (v_card_summary reads
-- from transactions directly). SQLite's card views in migration 028 were
-- already correct. This file exists for cross-backend numbering parity.
--
-- See schema/bigquery/034_fix_card_summary_double_count.sql for the
-- actual fix.

-- (no-op)
