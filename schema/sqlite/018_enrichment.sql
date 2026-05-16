-- Phase 1 enrichment foundation: idempotency column + cnpj cache table.
-- Sqlite lacks "ADD COLUMN IF NOT EXISTS", but the column is guaranteed
-- to be new here because the migration runs once per database (tracked
-- via schema_versions). Subsequent runs are skipped by run_migrations.

ALTER TABLE transactions ADD COLUMN enrichment_attempted_at TEXT;

CREATE TABLE IF NOT EXISTS cnpj_cache (
  cnpj TEXT PRIMARY KEY,
  found INTEGER NOT NULL,
  data_json TEXT,
  fetched_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cnpj_cache_fetched ON cnpj_cache(fetched_at);
