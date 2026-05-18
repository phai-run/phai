# Changelog

## [1.1.0](https://github.com/feliperun/finance-os/compare/v1.0.1...v1.1.0) (2026-05-18)


### Features

* **cards:** group v_card_summary by billing cycle, add v_card_open_now ([6b11616](https://github.com/feliperun/finance-os/commit/6b11616f99213f4b06606156d1def62646985b2d))
* **notify:** always-on saldo in pulse; phone-readable sync notify ([7c91a3c](https://github.com/feliperun/finance-os/commit/7c91a3c682d8d88685cd8cfa7b95574f35d6d8df))
* proactive pulse, cycle-aware cards, saldo em conta, phone-friendly sync notify ([d48afd1](https://github.com/feliperun/finance-os/commit/d48afd1b18875d6349849690c919e8bf1be7af10))
* **pulse:** proactive closing-plan WhatsApp message + notify webhook ([9a6d0a8](https://github.com/feliperun/finance-os/commit/9a6d0a887b8ece090373a1820dde2c908e0142a2))
* **reports:** add saldo em conta — pulse block + `report balances` ([4d8c351](https://github.com/feliperun/finance-os/commit/4d8c351021862ec7b21b72a08599705dd731402a))


### Bug Fixes

* **pulse:** clippy unnecessary_sort_by on Rust 1.95 ([2a1aaec](https://github.com/feliperun/finance-os/commit/2a1aaecc7e5273594cd75ccf8ab6089e29d821fd))

## [1.0.1](https://github.com/feliperun/finance-os/compare/v1.0.0...v1.0.1) (2026-05-18)


### Bug Fixes

* **installments:** detect Pluggy installment markers from metadata ([0078424](https://github.com/feliperun/finance-os/commit/0078424b4f3c5f563083e344625587a48a7c8c97))
* **installments:** surface Pluggy metadata for data synced before the fix ([84030be](https://github.com/feliperun/finance-os/commit/84030be604aead51cd088f4aa071fad67632790c))
* **pluggy:** correct sign for credit-card credits and FX amounts ([#26](https://github.com/feliperun/finance-os/issues/26)) ([64426e2](https://github.com/feliperun/finance-os/commit/64426e20a568f7e6dcec256f343945ba2c6d6e3c))

## [1.0.0](https://github.com/feliperun/finance-os/compare/v0.11.0...v1.0.0) (2026-05-17)


### ⚠ BREAKING CHANGES

* `sync pluggy` now makes a GitHub Releases API call on every invocation. Pass FINANCE_OS_NO_AUTO_UPDATE=1 to opt out if needed.

### Features

* enforce latest version before pluggy sync ([c6a7bf2](https://github.com/feliperun/finance-os/commit/c6a7bf280d7239be15d9abe28f547c79d960c27a))


### Bug Fixes

* **reports:** correct three silent bugs in budget-status, forecast, and cards ([cb45863](https://github.com/feliperun/finance-os/commit/cb4586382152fc860ececf6b4d0e78d0e0e00f81))

## [0.11.0](https://github.com/feliperun/finance-os/compare/v0.10.0...v0.11.0) (2026-05-17)


### Features

* **cards:** add --installments-only filter to report cards ([24820b9](https://github.com/feliperun/finance-os/commit/24820b9839f1a9a24bbacbdfb27dfa0d552e34c9))

## [0.10.0](https://github.com/feliperun/finance-os/compare/v0.9.0...v0.10.0) (2026-05-16)


### Features

* **enrichment:** LLM-driven transaction enrichment pipeline ([#19](https://github.com/feliperun/finance-os/issues/19)) ([d112a37](https://github.com/feliperun/finance-os/commit/d112a3770242550d37187e12d633c5099fd1aba7))

## [0.9.0](https://github.com/feliperun/finance-os/compare/v0.8.0...v0.9.0) (2026-05-15)


### Features

* **release:** macOS Intel build alongside Apple Silicon ([#17](https://github.com/feliperun/finance-os/issues/17)) ([6cb35ad](https://github.com/feliperun/finance-os/commit/6cb35ad81d1284a7c2ea7a439afcf6c28d3e7d76))

## [0.8.0](https://github.com/feliperun/finance-os/compare/v0.7.0...v0.8.0) (2026-05-15)


### Features

* **reports:** subcategory breakdown + --all flag for report cards ([#15](https://github.com/feliperun/finance-os/issues/15)) ([274bb3d](https://github.com/feliperun/finance-os/commit/274bb3db3833dfe476b23f9212950c9d0f5b7865))

## [0.7.0](https://github.com/feliperun/finance-os/compare/v0.6.0...v0.7.0) (2026-05-15)


### Features

* **reports:** new `report cards` + English help text on all reports ([#13](https://github.com/feliperun/finance-os/issues/13)) ([7bff310](https://github.com/feliperun/finance-os/commit/7bff310e42ae962a318d96ed1ff5351ec030b0d5))

## [0.6.0](https://github.com/feliperun/finance-os/compare/v0.5.1...v0.6.0) (2026-05-14)


### Features

* install.sh, README facelift, WhatsApp-friendly reports (start with daily-pulse) ([#11](https://github.com/feliperun/finance-os/issues/11)) ([c64a10a](https://github.com/feliperun/finance-os/commit/c64a10aa6645ccd03e8a902d756f3b440edaca4c))

## [0.5.1](https://github.com/feliperun/finance-os/compare/v0.5.0...v0.5.1) (2026-05-14)


### Bug Fixes

* **update:** use dedicated long-timeout client for binary download ([#9](https://github.com/feliperun/finance-os/issues/9)) ([57ca255](https://github.com/feliperun/finance-os/commit/57ca2558290c4fc0ddba67d220048a01099e557a))

## [0.5.0](https://github.com/feliperun/finance-os/compare/v0.4.1...v0.5.0) (2026-05-14)


### Features

* **cli:** add --json output to `finance self check` ([#7](https://github.com/feliperun/finance-os/issues/7)) ([f991505](https://github.com/feliperun/finance-os/commit/f991505e19e01b05292fd87ab9ca2ae25bb62427))

## [0.4.1](https://github.com/feliperun/finance-os/compare/v0.4.0...v0.4.1) (2026-05-14)


### Bug Fixes

* **install:** update repo owner from feliperbroering to feliperun ([#5](https://github.com/feliperun/finance-os/issues/5)) ([14474d8](https://github.com/feliperun/finance-os/commit/14474d8a957becfabbf4df6990565dfdcbbab947))

## [0.4.0](https://github.com/feliperun/finance-os/compare/v0.3.1...v0.4.0) (2026-05-14)


### Features

* **cli:** self-update + port legacy operational features (entregas 2, 4B, 5B) ([#3](https://github.com/feliperun/finance-os/issues/3)) ([8ca2ff3](https://github.com/feliperun/finance-os/commit/8ca2ff39852de12b5844298909d5575b75c27f8e))

## [0.3.1](https://github.com/feliperun/finance-os/compare/v0.3.0...v0.3.1) (2026-05-13)


### Bug Fixes

* **release:** make crate versions explicit for release-please ([d0c7d73](https://github.com/feliperun/finance-os/commit/d0c7d73a85eb24f392a9e84b590c02e586e5d9c8))
* trigger release for card insights corrections ([f673aff](https://github.com/feliperun/finance-os/commit/f673affaaf91953e70a0203880e06633a60327b7))
