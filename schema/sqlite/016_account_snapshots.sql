CREATE TABLE IF NOT EXISTS account_snapshots (
  snapshot_id       TEXT NOT NULL PRIMARY KEY,
  account_id        TEXT NOT NULL REFERENCES accounts(account_id),
  snapshot_date     TEXT NOT NULL,
  balance           TEXT,
  credit_limit      TEXT,
  currency_code     TEXT,
  source            TEXT NOT NULL,
  actor_id          TEXT NOT NULL,
  idempotency_key   TEXT NOT NULL UNIQUE,
  metadata_json     TEXT NOT NULL DEFAULT '{}',
  created_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_account_snapshots_account_date
  ON account_snapshots(account_id, snapshot_date DESC);
