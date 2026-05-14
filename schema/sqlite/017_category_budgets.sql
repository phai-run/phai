CREATE TABLE IF NOT EXISTS category_budgets (
  budget_id TEXT PRIMARY KEY,
  category_id TEXT NOT NULL,
  subcategory_id TEXT,
  month_ref TEXT,
  amount TEXT NOT NULL,
  alert_threshold_pct INTEGER NOT NULL DEFAULT 80,
  actor_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_category_budgets_unique
  ON category_budgets (
    category_id,
    COALESCE(subcategory_id, ''),
    COALESCE(month_ref, '_default')
  );
