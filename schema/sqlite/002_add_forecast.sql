CREATE TABLE IF NOT EXISTS forecast (
  forecast_id TEXT PRIMARY KEY,
  due_date TEXT NOT NULL,
  description TEXT NOT NULL,
  amount REAL NOT NULL,
  category_id TEXT,
  account_id TEXT,
  status TEXT NOT NULL,
  recurrence TEXT,
  actor_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

