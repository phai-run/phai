## Summary

<!-- Brief description of the change. -->

## Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets --all-features` passes
- [ ] `cargo test --workspace` passes
- [ ] New migrations are idempotent and present in both `schema/sqlite/` and `schema/bigquery/`
- [ ] Write operations emit audit events
- [ ] No hardcoded personal data, secrets, or credentials
