-- 040_transaction_tier.sql — ADR-0032.
--
-- Per-transaction commitment-tier override (locked | cancellable | variable).
-- Sparse: only transactions the user manually pins land here; every other
-- transaction's tier is derived at read time (installment/subscription flags).
-- Generic infrastructure only — the rows are user data, the table carries no
-- personal counterparties or labels (AGENTS §1).
--
-- Idempotent.

CREATE TABLE IF NOT EXISTS transaction_tier (
  transaction_id   TEXT PRIMARY KEY,
  tier             TEXT NOT NULL,
  actor_id         TEXT NOT NULL,
  idempotency_key  TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);
