-- Migration 025: move `moradia:streaming` → `assinaturas:streaming`.
-- Mirror of schema/sqlite/024_streaming_to_assinaturas.sql.

MERGE `{{project_id}}.{{dataset_id}}.categories` AS target
USING (
  SELECT
    'assinaturas:streaming' AS category_id,
    'Streaming' AS name,
    'assinaturas' AS parent_category_id,
    JSON '{}' AS metadata_json,
    'migration:025' AS actor_id,
    TIMESTAMP '2026-05-18 00:00:00 UTC' AS updated_at
) AS source
ON target.category_id = source.category_id
WHEN NOT MATCHED THEN
  INSERT (category_id, name, parent_category_id, metadata_json, actor_id, updated_at)
  VALUES (source.category_id, source.name, source.parent_category_id,
          source.metadata_json, source.actor_id, source.updated_at);

UPDATE `{{project_id}}.{{dataset_id}}.transactions`
SET category_id = 'assinaturas:streaming'
WHERE category_id = 'moradia:streaming';

UPDATE `{{project_id}}.{{dataset_id}}.forecast`
SET category_id = 'assinaturas:streaming'
WHERE category_id = 'moradia:streaming';

UPDATE `{{project_id}}.{{dataset_id}}.category_budgets`
SET category_id = 'assinaturas:streaming'
WHERE category_id = 'moradia:streaming';

DELETE FROM `{{project_id}}.{{dataset_id}}.categories`
WHERE category_id = 'moradia:streaming';
