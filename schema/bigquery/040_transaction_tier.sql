-- 040_transaction_tier.sql — ADR-0032.
--
-- Per-transaction commitment-tier override (locked | cancellable | variable),
-- mirrors schema/sqlite/040_transaction_tier.sql. Sparse: only manually pinned
-- transactions land here; every other tier is derived at read time. Generic
-- infrastructure only — no personal data (AGENTS §1).
--
-- Idempotent.

CREATE TABLE IF NOT EXISTS `{{project_id}}.{{dataset_id}}.transaction_tier` (
  transaction_id   STRING NOT NULL,
  tier             STRING NOT NULL,
  actor_id         STRING NOT NULL,
  idempotency_key  STRING NOT NULL,
  updated_at       TIMESTAMP NOT NULL
);
