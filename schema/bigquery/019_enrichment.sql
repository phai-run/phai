ALTER TABLE `{{project_id}}.{{dataset_id}}.transactions`
  ADD COLUMN IF NOT EXISTS enrichment_attempted_at TIMESTAMP;
