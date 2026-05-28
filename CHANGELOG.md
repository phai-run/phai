# Changelog

## [3.2.3](https://github.com/feliperun/finance-os/compare/v3.2.2...v3.2.3) (2026-05-28)


### Bug Fixes

* **release:** collapse workspace to a single version ([#93](https://github.com/feliperun/finance-os/issues/93)) ([633f8c0](https://github.com/feliperun/finance-os/commit/633f8c03b9c014fb9bf60b20cf1b986a40a05ea8))
* **release:** drive crate versions via simple strategy + extra-files ([#95](https://github.com/feliperun/finance-os/issues/95)) ([254ed03](https://github.com/feliperun/finance-os/commit/254ed038b20afabae6a3df0a961162a0130ddeb1))
* **release:** use literal crate versions for release-please rust updater ([#94](https://github.com/feliperun/finance-os/issues/94)) ([e887f77](https://github.com/feliperun/finance-os/commit/e887f771dd94055ce14ed4484c65b60dcb3f2ffb))

## 0.3.0 - 2026-05-12

### Features

- Add BigQuery-only transaction splits with split lines and receipt items.
- Add deterministic CLI commands for split preview, apply, show, and clear.
- Add split candidate and item price reports for assistant workflows.
- Update assistant integration docs for split-aware Finance OS reporting.

### Infrastructure

- Configure Release Please for future Rust workspace releases.
