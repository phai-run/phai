-- Migration 023: introduce `_revisar` as the canonical fallback category.
--
-- Before this migration, sync paths that couldn't confidently classify a
-- transaction would tag it `category_source = 'fallback'` and drop it into
-- `outros:geral` (a legitimate user-facing category). The legit users of
-- `outros:geral` (manual entries that genuinely belong there) became
-- indistinguishable from the fallback dump — 68 rows of "needs triage"
-- hidden inside a 106-row category.
--
-- The fix is to give the fallback a reserved slug that `slugify` can never
-- naturally produce. `_revisar` starts with an underscore, which the
-- slugifier strips as non-alphanumeric, so no user-typed label collides
-- with it. The `_` also sorts top-of-list in most views, surfacing the
-- needs-triage bucket explicitly.
--
-- This migration:
-- 1. Inserts the `_revisar` category row (idempotent on re-run).
-- 2. Re-routes existing fallback rows from `outros:geral` to `_revisar`.
--    (Rows where `outros:geral` was a deliberate manual classification keep
--    it — only `category_source = 'fallback'` rows move.)
-- 3. v_uncategorized stays unchanged: it already catches both
--    `category_id IS NULL` and `category_source IN ('unclassified',
--    'fallback')`, so `_revisar` rows surface there from the source field.
--    No view changes needed.

INSERT OR IGNORE INTO categories (
  category_id, name, parent_category_id, metadata_json, actor_id, updated_at
) VALUES (
  '_revisar',
  'Revisar',
  NULL,
  '{"reserved": true, "purpose": "fallback bucket — needs triage"}',
  'migration:023',
  '2026-05-18T00:00:00+00:00'
);

UPDATE transactions
SET category_id = '_revisar'
WHERE category_source = 'fallback'
  AND category_id = 'outros:geral';
