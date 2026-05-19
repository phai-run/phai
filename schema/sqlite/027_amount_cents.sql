-- Migration 027: add amount_cents INTEGER column to transactions.
--
-- ADR-0003 requires decimal-precise aggregation. SQLite stores `amount`
-- as REAL which loses precision during SUM over many rows. This migration
-- adds a VIRTUAL generated column `amount_cents` that derives exact cents
-- from `amount`. Future views will use SUM(amount_cents) / 100.0 instead
-- of CAST(amount AS REAL).
--
-- VIRTUAL (not STORED) because macOS's SQLite build (Apple's bundled
-- libsqlite3) rejects ALTER TABLE ADD COLUMN ... STORED even in 3.51.0.
-- VIRTUAL is computed on read — functionally identical for our purpose:
-- SUM of integers is exact regardless of storage. VIRTUAL columns can
-- still be indexed if needed.
--
-- BigQuery mirror is 028_amount_cents.sql — NUMERIC is already precise
-- there, but the column exists for cross-backend query compatibility.

ALTER TABLE transactions ADD COLUMN amount_cents INTEGER
  GENERATED ALWAYS AS (CAST(ROUND(amount * 100) AS INTEGER)) VIRTUAL;
