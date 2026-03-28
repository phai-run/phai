CREATE TABLE IF NOT EXISTS schema_versions (
  version TEXT PRIMARY KEY,
  applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS accounts (
  account_id TEXT PRIMARY KEY,
  owner TEXT NOT NULL,
  account_type TEXT NOT NULL,
  bank TEXT NOT NULL,
  label TEXT NOT NULL,
  pluggy_account_id TEXT,
  pluggy_item_id TEXT,
  status TEXT NOT NULL,
  actor_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS categories (
  category_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  parent_category_id TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  actor_id TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rules (
  rule_id TEXT PRIMARY KEY,
  body TEXT NOT NULL,
  status TEXT NOT NULL,
  actor_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS transactions (
  transaction_id TEXT PRIMARY KEY,
  account_id TEXT,
  transaction_date TEXT NOT NULL,
  description TEXT NOT NULL,
  amount REAL NOT NULL,
  tx_type TEXT NOT NULL,
  category_id TEXT,
  category_source TEXT NOT NULL,
  context TEXT,
  payment_status TEXT NOT NULL,
  source TEXT NOT NULL,
  actor_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_log (
  event_id TEXT PRIMARY KEY,
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  action TEXT NOT NULL,
  actor_id TEXT NOT NULL,
  event_timestamp TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  diff_json TEXT NOT NULL
);

