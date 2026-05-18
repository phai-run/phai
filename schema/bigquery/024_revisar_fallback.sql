-- Migration 024: introduce `_revisar` as the canonical fallback category.
-- Mirror of schema/sqlite/023_revisar_fallback.sql.

MERGE `{{project_id}}.{{dataset_id}}.categories` AS target
USING (
  SELECT
    '_revisar' AS category_id,
    'Revisar' AS name,
    CAST(NULL AS STRING) AS parent_category_id,
    JSON '{"reserved": true, "purpose": "fallback bucket — needs triage"}' AS metadata_json,
    'migration:024' AS actor_id,
    TIMESTAMP '2026-05-18 00:00:00 UTC' AS updated_at
) AS source
ON target.category_id = source.category_id
WHEN NOT MATCHED THEN
  INSERT (category_id, name, parent_category_id, metadata_json, actor_id, updated_at)
  VALUES (source.category_id, source.name, source.parent_category_id,
          source.metadata_json, source.actor_id, source.updated_at);

UPDATE `{{project_id}}.{{dataset_id}}.transactions`
SET category_id = '_revisar'
WHERE category_source = 'fallback'
  AND category_id = 'outros:geral';
