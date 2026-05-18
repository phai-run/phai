-- Migration 024: move `moradia:streaming` → `assinaturas:streaming`.
--
-- The user-level taxonomy had `moradia:streaming` (housing > streaming)
-- alongside `assinaturas:apple`, `assinaturas:cloud-storage`,
-- `assinaturas:ia-produtividade`, etc. Streaming services are subscriptions
-- like every other monthly bill in `assinaturas:*`; living in `moradia` was
-- a historical accident that fragments any "total de assinaturas" view.
--
-- This migration:
-- 1. Inserts `assinaturas:streaming` if missing (idempotent).
-- 2. Re-routes all transactions, forecasts and budgets from
--    `moradia:streaming` → `assinaturas:streaming`.
-- 3. Drops the now-empty `moradia:streaming` category row if it exists.
--
-- Rules table is NOT touched: rules persist the human label
-- ("Moradia" / "Streaming") and slugify at apply time, so the rules a user
-- might have written stay valid. Future syncs that hit those rules will
-- now produce `moradia:streaming` again — the user is expected to edit
-- those rules manually if they want the assinaturas semantic. The
-- migration documents this in the AGENTS playbook (PLAN.md).

INSERT OR IGNORE INTO categories (
  category_id, name, parent_category_id, metadata_json, actor_id, updated_at
) VALUES (
  'assinaturas:streaming',
  'Streaming',
  'assinaturas',
  '{}',
  'migration:024',
  '2026-05-18T00:00:00+00:00'
);

UPDATE transactions
SET category_id = 'assinaturas:streaming'
WHERE category_id = 'moradia:streaming';

UPDATE forecast
SET category_id = 'assinaturas:streaming'
WHERE category_id = 'moradia:streaming';

UPDATE category_budgets
SET category_id = 'assinaturas:streaming'
WHERE category_id = 'moradia:streaming';

DELETE FROM categories
WHERE category_id = 'moradia:streaming';
