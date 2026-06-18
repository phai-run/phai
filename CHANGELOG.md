# Changelog

## [5.14.1](https://github.com/phai-run/phai/compare/v5.14.0...v5.14.1) (2026-06-18)


### Bug Fixes

* **update:** require a verified signature on update (ADR-0017) ([#167](https://github.com/phai-run/phai/issues/167)) ([e206019](https://github.com/phai-run/phai/commit/e20601998482e09e68085c5df328d1b26ba6b285))

## [5.14.0](https://github.com/phai-run/phai/compare/v5.13.0...v5.14.0) (2026-06-18)


### Features

* **update:** confirm minisign signature verification on update ([#165](https://github.com/phai-run/phai/issues/165)) ([6ddd6b9](https://github.com/phai-run/phai/commit/6ddd6b907c0e4efe87f4107dd28b6efbf1f9fcea))

## [5.13.0](https://github.com/phai-run/phai/compare/v5.12.0...v5.13.0) (2026-06-18)


### Features

* per-transaction commitment-tier override + English labels (ADR-0032) ([#161](https://github.com/phai-run/phai/issues/161)) ([c947b5e](https://github.com/phai-run/phai/commit/c947b5ed126bb376e17009a17a043b0fa2ebc194))

## [5.12.0](https://github.com/phai-run/phai/compare/v5.11.0...v5.12.0) (2026-06-18)


### Features

* **web:** commitment tiers + planning goal solver (ADR-0030/0031) ([#159](https://github.com/phai-run/phai/issues/159)) ([f9b72a4](https://github.com/phai-run/phai/commit/f9b72a42f8dee19d03c6abe3a7818619284a5320))

## [5.11.0](https://github.com/phai-run/phai/compare/v5.10.0...v5.11.0) (2026-06-15)


### Features

* **web:** warn before closing with unsynced writes + make sync chip clickable ([#157](https://github.com/phai-run/phai/issues/157)) ([d6dee85](https://github.com/phai-run/phai/commit/d6dee85a78332210842af96242d7d4dcfd5e909a))

## [5.10.0](https://github.com/phai-run/phai/compare/v5.9.2...v5.10.0) (2026-06-15)


### Features

* **serve:** phai serve install --system — root LaunchDaemon on port 80 via admin-auth prompt ([#154](https://github.com/phai-run/phai/issues/154)) ([6bef3c4](https://github.com/phai-run/phai/commit/6bef3c45750df17b30105ca830279fb75d9ca165))
* **serve:** pin Phai.app to the Dock on install ([#155](https://github.com/phai-run/phai/issues/155)) ([cb53fb9](https://github.com/phai-run/phai/commit/cb53fb959997fa1b206bde3d49bcac18e5009c6a))
* **web:** branded loading skeletons for cold start ([#153](https://github.com/phai-run/phai/issues/153)) ([25a8cf8](https://github.com/phai-run/phai/commit/25a8cf8eca5a9bb909e1d22d03b610d0b7ffb8cc))


### Performance Improvements

* **serve:** add TTL read cache to the web bridge ([#152](https://github.com/phai-run/phai/issues/152)) ([fa0ef0a](https://github.com/phai-run/phai/commit/fa0ef0a1bf726563ab65e8f3430c2bfed6c1c5e3))

## [5.9.2](https://github.com/phai-run/phai/compare/v5.9.1...v5.9.2) (2026-06-12)


### Bug Fixes

* **serve:** block DNS rebinding + harden serve bridge and BigQuery search ([253f81e](https://github.com/phai-run/phai/commit/253f81e55ff0a95a0d436156fd07625472e460b3))
* **serve:** block DNS rebinding, stop error leaks, cap event batch ([7d11794](https://github.com/phai-run/phai/commit/7d11794bcb3226b8b7d650f25489c408212da42f))
* **storage:** escape LIKE wildcards in BigQuery search ([55fe27c](https://github.com/phai-run/phai/commit/55fe27c6b6fa1e0f21a5d458ad1974a9e9dee9ad))

## [5.9.1](https://github.com/phai-run/phai/compare/v5.9.0...v5.9.1) (2026-06-12)


### Bug Fixes

* tighten financial invariants ([c58bb20](https://github.com/phai-run/phai/commit/c58bb202aed6bfc5039c484d2325c78995be9cd8))

## [5.9.0](https://github.com/phai-run/phai/compare/v5.8.0...v5.9.0) (2026-06-12)


### Features

* **cli:** MCP server — read-only reports over stdio (phai mcp) ([#146](https://github.com/phai-run/phai/issues/146)) ([f3280ef](https://github.com/phai-run/phai/commit/f3280efdf17ff4f2d32aa188bf6577e12d3e2b36))
* **cli:** phai serve install — launchd agent + Phai.app launcher (macOS) ([#147](https://github.com/phai-run/phai/issues/147)) ([0ef4ee2](https://github.com/phai-run/phai/commit/0ef4ee2283fa913176a2e663d2800cf504198f74))
* **web:** English UI ([#142](https://github.com/phai-run/phai/issues/142)) ([798143d](https://github.com/phai-run/phai/commit/798143dec7ea2caaca04a82fa992ceb237a2f1e1))

## [5.8.0](https://github.com/phai-run/phai/compare/v5.7.0...v5.8.0) (2026-06-11)


### Features

* **web:** metas por subcategoria no plano de guerra com envelopes persistidos ([#140](https://github.com/phai-run/phai/issues/140)) ([4a72843](https://github.com/phai-run/phai/commit/4a72843ac258c85e9c82def1a8bd1d4ce190054e))

## [5.7.0](https://github.com/phai-run/phai/compare/v5.6.1...v5.7.0) (2026-06-11)


### Features

* **web:** opaque sticky sheet headers, larger type, category emojis, level-3 treemap tiles ([#138](https://github.com/phai-run/phai/issues/138)) ([98001d5](https://github.com/phai-run/phai/commit/98001d5681f32dc00c4426866c5502e4e0f27dd6))

## [5.6.1](https://github.com/phai-run/phai/compare/v5.6.0...v5.6.1) (2026-06-10)


### Bug Fixes

* **web:** boot fresh livestore store on schema drift and never skip seeding empty tables ([#136](https://github.com/phai-run/phai/issues/136)) ([763bb81](https://github.com/phai-run/phai/commit/763bb812235705aeb4e0cdce7e0f38564a9a0640))

## [5.6.0](https://github.com/phai-run/phai/compare/v5.5.0...v5.6.0) (2026-06-10)


### Features

* **web:** modo planilha, plano de guerra e dedup de parcelas renomeadas ([#134](https://github.com/phai-run/phai/issues/134)) ([8c0ca52](https://github.com/phai-run/phai/commit/8c0ca5208a0468c353938529dcbd82a23865226e))

## [5.5.0](https://github.com/phai-run/phai/compare/v5.4.1...v5.5.0) (2026-06-03)


### Features

* **web:** dashboard UX/UI redesign — decision hero, chart, cards, filters, category overview ([#129](https://github.com/phai-run/phai/issues/129)) ([74e2b82](https://github.com/phai-run/phai/commit/74e2b8209673d472d6a8fbd222891fe83026f4a5))

## [5.4.1](https://github.com/phai-run/phai/compare/v5.4.0...v5.4.1) (2026-06-03)


### Bug Fixes

* **web:** trustworthy + fast + delightful serve dashboard (data, cards, tx, chart, perf) ([#127](https://github.com/phai-run/phai/issues/127)) ([3c7700d](https://github.com/phai-run/phai/commit/3c7700dad9414007fe337b201b0821b688e54f9e))

## [5.4.0](https://github.com/phai-run/phai/compare/v5.3.0...v5.4.0) (2026-06-03)


### Features

* **report:** CSV export ([941436a](https://github.com/phai-run/phai/commit/941436a91e61d931e29fe02977442389a8fd4f14))

## [5.3.0](https://github.com/phai-run/phai/compare/v5.2.1...v5.3.0) (2026-06-02)


### Features

* **cashflow:** cash-flow basis with bill explosion + single deduped view chain ([#122](https://github.com/phai-run/phai/issues/122)) ([e9f3710](https://github.com/phai-run/phai/commit/e9f37109888b62599d3a5dd9ff6414fd321f6b51))

## [5.2.1](https://github.com/phai-run/phai/compare/v5.2.0...v5.2.1) (2026-05-31)


### Bug Fixes

* **ci:** declare web pnpm workspace package ([6fdd05c](https://github.com/phai-run/phai/commit/6fdd05ccf82bc8f4b51caae621e815a4d600ade2))

## [5.2.0](https://github.com/phai-run/phai/compare/v5.1.3...v5.2.0) (2026-05-31)


### Features

* **serve:** paginated transactions API with offset/hasMore + forecast past-month guard ([d7e3f81](https://github.com/phai-run/phai/commit/d7e3f813a3d919a11e7da252dc1048b3f66b6129))
* **web:** hierarchical category grouping, keyboard shortcuts, chart modes, list performance ([b0d5847](https://github.com/phai-run/phai/commit/b0d5847774fb639078477948b3f23aef90401c53))


### Bug Fixes

* **web:** harden serve ux integration ([9a9e35c](https://github.com/phai-run/phai/commit/9a9e35cb7ea2b7411cb7eee8844ce501ea067fc6))

## [5.1.3](https://github.com/phai-run/phai/compare/v5.1.2...v5.1.3) (2026-05-31)


### Bug Fixes

* **serve:** address quality audit findings [#3](https://github.com/phai-run/phai/issues/3)-[#12](https://github.com/phai-run/phai/issues/12) ([8c09905](https://github.com/phai-run/phai/commit/8c09905a06b8b90a0a2b0bc09907e42238bdda94))

## [5.1.2](https://github.com/phai-run/phai/compare/v5.1.1...v5.1.2) (2026-05-31)


### Bug Fixes

* **serve:** propagate serialization error instead of silently defaulting to empty audit diff ([#116](https://github.com/phai-run/phai/issues/116)) ([29c31b6](https://github.com/phai-run/phai/commit/29c31b6c5e88d223c3c9336b885f27b0f4a35495))

## [5.1.1](https://github.com/phai-run/phai/compare/v5.1.0...v5.1.1) (2026-05-30)


### Bug Fixes

* **cli:** show snapshot timestamp in balances and auto-clear local_db_path on BigQuery setup ([294e750](https://github.com/phai-run/phai/commit/294e75042e001d5076f33c22b53556ecbaab8acf))
* **cli:** show snapshot timestamp in balances and auto-clear local_db_path on BigQuery setup ([3f062d8](https://github.com/phai-run/phai/commit/3f062d8561f16ef195e6e8cbc8bee63d8cc1290f))

## [5.1.0](https://github.com/phai-run/phai/compare/v5.0.0...v5.1.0) (2026-05-30)


### Features

* **forecast:** harden template idempotency with natural keys + self-healing dedup ([#111](https://github.com/phai-run/phai/issues/111)) ([a646679](https://github.com/phai-run/phai/commit/a646679f674bbdbd944b47d32e67899d6bb0425c))

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
