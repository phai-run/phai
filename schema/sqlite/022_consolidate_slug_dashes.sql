-- Migration 022: collapse `---` (triple-dash) category slugs to `-`.
--
-- Older versions of the slugifier rendered `" / "` literally — three chars,
-- each replaced by a dash — producing keys like `assinaturas:cloud---storage`,
-- `transporte:pedagio---estacionamento`, `outros:pix---rateios`, etc. The
-- current slugifier collapses runs of non-alphanumeric chars to a single dash,
-- but the legacy rows persist and fragment every "top category" aggregation
-- (the same logical category appears as two distinct keys).
--
-- This migration is purely a data fix. It updates every place a slug can
-- appear, then consolidates the `categories` table so we don't keep two PK
-- rows pointing at the same logical category.
--
-- Order matters: rewrite the references (transactions / forecast / budgets)
-- BEFORE touching the `categories` PKs, otherwise we'd briefly have
-- references pointing at non-existent keys.

UPDATE transactions
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

UPDATE forecast
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

UPDATE category_budgets
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

UPDATE category_budgets
SET subcategory_id = REPLACE(subcategory_id, '---', '-')
WHERE subcategory_id LIKE '%---%';

UPDATE internal_categories
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

-- Drop categories rows that have a `---` slug AND a canonical `-` twin
-- already present. The transactions/forecast/budgets above have already
-- been retargeted at the canonical key, so this DELETE is safe.
DELETE FROM categories
WHERE category_id LIKE '%---%'
  AND EXISTS (
    SELECT 1 FROM categories c2
    WHERE c2.category_id = REPLACE(categories.category_id, '---', '-')
  );

-- For categories where only the `---` variant existed, rename it in place.
UPDATE categories
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

-- Same for parent_category_id (categories can self-reference).
UPDATE categories
SET parent_category_id = REPLACE(parent_category_id, '---', '-')
WHERE parent_category_id LIKE '%---%';
