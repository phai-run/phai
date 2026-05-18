-- Migration 023: collapse `---` (triple-dash) category slugs to `-`.
-- Mirror of schema/sqlite/022_consolidate_slug_dashes.sql.

UPDATE `{{project_id}}.{{dataset_id}}.transactions`
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

UPDATE `{{project_id}}.{{dataset_id}}.forecast`
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

UPDATE `{{project_id}}.{{dataset_id}}.category_budgets`
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

UPDATE `{{project_id}}.{{dataset_id}}.category_budgets`
SET subcategory_id = REPLACE(subcategory_id, '---', '-')
WHERE subcategory_id LIKE '%---%';

UPDATE `{{project_id}}.{{dataset_id}}.internal_categories`
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

-- Drop `---` rows that have a canonical `-` twin already present.
DELETE FROM `{{project_id}}.{{dataset_id}}.categories`
WHERE category_id LIKE '%---%'
  AND EXISTS (
    SELECT 1
    FROM `{{project_id}}.{{dataset_id}}.categories` AS c2
    WHERE c2.category_id = REPLACE(
      `{{project_id}}.{{dataset_id}}.categories`.category_id, '---', '-')
  );

-- Rename remaining `---` rows in place.
UPDATE `{{project_id}}.{{dataset_id}}.categories`
SET category_id = REPLACE(category_id, '---', '-')
WHERE category_id LIKE '%---%';

UPDATE `{{project_id}}.{{dataset_id}}.categories`
SET parent_category_id = REPLACE(parent_category_id, '---', '-')
WHERE parent_category_id LIKE '%---%';
