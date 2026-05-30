# Changelog

## [5.0.0](https://github.com/phai-run/phai/compare/v4.1.0...v5.0.0) (2026-05-30)


### ⚠ BREAKING CHANGES

* BigQuery runtimes that rewired v_transactions_effective to a Google Sheet will lose Drive access — queries against a Sheets-backed external table will fail. Move those overrides into rules / private configuration.

### Features

* remove Google Sheets category-override sync ([3752867](https://github.com/phai-run/phai/commit/37528675f267c3ae59bd6dff01e78486752ba76f))


### Bug Fixes

* **storage:** accept both 'ativo' and 'active' forecast status in upcoming_forecasts ([d354325](https://github.com/phai-run/phai/commit/d3543259400366ed0a88e2a0709f95ca54c509e0))
* **storage:** accept both 'ativo' and 'active' forecast status in upcoming_forecasts ([d354325](https://github.com/phai-run/phai/commit/d3543259400366ed0a88e2a0709f95ca54c509e0))
* **storage:** accept both 'ativo' and 'active' forecast status in upcoming_forecasts ([c4caaab](https://github.com/phai-run/phai/commit/c4caaabd0a013784362d607846714a25cb8e037f)), closes [#109](https://github.com/phai-run/phai/issues/109)

## [4.1.0](https://github.com/phai-run/phai/compare/v4.0.0...v4.1.0) (2026-05-29)


### Features

* migrate on-disk identity to phai (finance-os fallback) ([f57ac4a](https://github.com/phai-run/phai/commit/f57ac4abe6fb8c62977dfddd2970115f76754f8d))
* migrate on-disk identity to phai with finance-os fallback ([0f0ae83](https://github.com/phai-run/phai/commit/0f0ae83f0b7bf0314f21b7e456665c8bb5190ccf))


### Bug Fixes

* address code review on identity migration ([5088b3b](https://github.com/phai-run/phai/commit/5088b3bb3997a855afe8fae564cdc7de3a5ec96c))

## [4.0.0](https://github.com/phai-run/phai/compare/v3.2.4...v4.0.0) (2026-05-29)


### ⚠ BREAKING CHANGES

* **cli:** the CLI binary is renamed from `fin` to `phai` and the crates from finance-core/finance-cli to phai-core/phai-cli. Existing installs must reinstall (curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh | bash); the old `fin` command no longer exists.
* the installed binary is now `phai` (was `fin`). Existing users must reinstall; the old `fin` binary is not upgraded in place.

### Features

* **cli:** add phai branding to version and help ([075f704](https://github.com/phai-run/phai/commit/075f704af6c536d06ca6c9f62fd1e45f3624e4e9))
* **site:** publish phai.run landing via GitHub Pages ([1aa1967](https://github.com/phai-run/phai/commit/1aa1967b98c5b6f76cb6f3704821f420c35b0d42))


### Bug Fixes

* **cli:** correct remaining finance-cli/-core brand strings in user-facing output ([9cd4705](https://github.com/phai-run/phai/commit/9cd470517d82a7c2d471856d24f1ff2dd62e25f1))


### Code Refactoring

* rename crates to phai-core/phai-cli and binary to phai ([1fa5f8c](https://github.com/phai-run/phai/commit/1fa5f8cb62b864f182b33ced8999e2dea05b7b77))

## [3.2.4](https://github.com/feliperun/finance-os/compare/v3.2.3...v3.2.4) (2026-05-28)


### Bug Fixes

* **release:** read unprefixed release-please outputs for root package ([#97](https://github.com/feliperun/finance-os/issues/97)) ([17384cc](https://github.com/feliperun/finance-os/commit/17384cc7249bcfb98ca1f118c2a14fb03b93e905))

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
