-- Migration 036: mark Pluggy same-person transfers as internal movement.
--
-- These rows represent movement between accounts owned by the same person and
-- must not inflate cashflow income/expense aggregates. Existing report views
-- already read from internal_categories, so inserting the category is enough.

CREATE TABLE IF NOT EXISTS internal_categories (
  category_id TEXT PRIMARY KEY
);

INSERT OR IGNORE INTO internal_categories (category_id)
VALUES ('same-person-transfer');
