-- Migration 026: delete the phantom empty-id row in `accounts`.
--
-- The legacy CSV importer (load_account_registry) used to
-- `row.get("id").cloned().unwrap_or_default()`, producing a registry entry
-- with `account_id = ""` whenever the source CSV had a blank-id line. The
-- bundle then upserted that row into `accounts`. The bug is fixed in
-- `crates/finance-core/src/legacy.rs`, but the artefact persists in any
-- database that ran an import before the fix.
--
-- The phantom has no transactions pointing at it (the user-visible IDs
-- are non-empty), so dropping it is safe. We restrict the DELETE to the
-- exact "source = legacy_accounts_csv AND label is empty" signature so we
-- don't accidentally nuke a legitimate row that someone manually set up
-- with the empty-id slot.

DELETE FROM accounts
WHERE account_id = ''
  AND COALESCE(owner, '') = ''
  AND COALESCE(label, '') = ''
  AND json_extract(metadata_json, '$.source') = 'legacy_accounts_csv';
