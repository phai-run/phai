# AGENTS

Critical guardrails for this repository:

- Never hardcode personal, user-specific, confidential, or production-derived data in shared source files, migrations, fixtures, tests, or docs.
- Classification behavior that depends on a specific user's counterparties, names, account labels, statement text, or private business logic must live in the database `rules` table or in private configuration, not in committed Rust code or shared SQL migrations.
- Shared migrations may create generic infrastructure only. They must not embed private reclassification patterns, personal names, account numbers, or institution-specific statement fingerprints.
- Shared fixtures and tests must use synthetic/example data only.
- If a bug fix requires a user-specific data correction, implement generic engine support in code and apply the private rule or private data fix outside the repository.
- For assistant behavior and reporting UX conventions, read `FINANCE_OS.md` before proposing or formatting user-facing finance outputs.
