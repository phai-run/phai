-- Migration 036: mark Pluggy same-person transfers as internal movement.
--
-- These rows represent movement between accounts owned by the same person and
-- must not inflate cashflow income/expense aggregates. Existing report views
-- already read from internal_categories, so inserting the category is enough.

MERGE `{{project_id}}.{{dataset_id}}.internal_categories` target
USING (SELECT 'same-person-transfer' AS category_id) source
ON target.category_id = source.category_id
WHEN NOT MATCHED THEN INSERT (category_id) VALUES (source.category_id);
