-- Migration 032: fix v_transactions_reportable dedup (SQLite no-op).
--
-- The correlated-subquery issue only affects BigQuery. SQLite views were
-- already correct in migration 028. This file exists for cross-backend
-- numbering parity.

-- (no-op)
