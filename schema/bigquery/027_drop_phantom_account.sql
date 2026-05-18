-- Migration 027: delete the phantom empty-id row in `accounts`.
-- Mirror of schema/sqlite/026_drop_phantom_account.sql.

DELETE FROM `{{project_id}}.{{dataset_id}}.accounts`
WHERE account_id = ''
  AND COALESCE(owner, '') = ''
  AND COALESCE(label, '') = ''
  AND JSON_VALUE(metadata_json, '$.source') = 'legacy_accounts_csv';
