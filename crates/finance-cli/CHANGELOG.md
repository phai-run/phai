# Changelog

## [3.0.0](https://github.com/feliperun/finance-os/compare/v2.5.0...v3.0.0) (2026-05-26)


### ⚠ BREAKING CHANGES

* **serve:** fin serve no longer accepts --host; it always binds to 127.0.0.1 and advertises meuapp.localhost.

### Bug Fixes

* **serve:** bind dashboard to localhost alias ([a1f1346](https://github.com/feliperun/finance-os/commit/a1f1346b7fe90b7cd8ae68b92c5160f28517dfc6))

## [2.5.0](https://github.com/feliperun/finance-os/compare/v2.4.0...v2.5.0) (2026-05-26)


### Features

* **serve:** web dashboard com WebSocket API para forecast interativo ([#71](https://github.com/feliperun/finance-os/issues/71)) ([b416273](https://github.com/feliperun/finance-os/commit/b4162739553f45bb6336f1b89f8637e95a038309))

## [2.4.0](https://github.com/feliperun/finance-os/compare/v2.3.2...v2.4.0) (2026-05-26)


### Features

* **forecast:** close ADR-0016 operational gaps ([#69](https://github.com/feliperun/finance-os/issues/69)) ([fa55448](https://github.com/feliperun/finance-os/commit/fa55448b4cfaf0e33ce15672ff0317bb906e0986))

## [2.3.2](https://github.com/feliperun/finance-os/compare/v2.3.1...v2.3.2) (2026-05-25)


### Bug Fixes

* trigger release-please after non-CC squash merge of [#67](https://github.com/feliperun/finance-os/issues/67) ([2d5f6ba](https://github.com/feliperun/finance-os/commit/2d5f6ba2c42ce03d10c2ecc8fcdc4a2a9f3643fe))

## [2.3.1](https://github.com/feliperun/finance-os/compare/v2.3.0...v2.3.1) (2026-05-25)


### Bug Fixes

* **cli:** clarify forecast suggest covers envelopes (post-[#64](https://github.com/feliperun/finance-os/issues/64)) ([af7a34b](https://github.com/feliperun/finance-os/commit/af7a34b7605f3fe2b0a3441366f41eae237acd27))

## [2.3.0](https://github.com/feliperun/finance-os/compare/v2.2.0...v2.3.0) (2026-05-25)


### Features

* **forecast:** forecast_template schema + FinanceStore CRUD (ADR-0016 PR 1/5) ([#59](https://github.com/feliperun/finance-os/issues/59)) ([a3cd76a](https://github.com/feliperun/finance-os/commit/a3cd76a436971afa832847aac89598b34ff5d639))
* **forecast:** Layer 1 — installments auto-forecast (ADR-0016 PR 2/5) ([#60](https://github.com/feliperun/finance-os/issues/60)) ([402b46f](https://github.com/feliperun/finance-os/commit/402b46f0b4a2edd0b5bd00ffb303158100b0f2f2))
* **forecast:** Layer 2/3 — suggest/accept/dismiss recurring templates (ADR-0016 PR 3/5) ([#61](https://github.com/feliperun/finance-os/issues/61)) ([fabdf8a](https://github.com/feliperun/finance-os/commit/fabdf8ad07e491112b9fbcc4c7c617371b5f9c80))
* **forecast:** Layer 4 — category envelopes (ADR-0016 PR 4/5) ([#62](https://github.com/feliperun/finance-os/issues/62)) ([3c3fe87](https://github.com/feliperun/finance-os/commit/3c3fe87826705919e21ce18d7896223ca3fe3d06))
* **forecast:** scenario eval + OpenClaw skill triggers (ADR-0016 PR 5/5) ([#63](https://github.com/feliperun/finance-os/issues/63)) ([a03e819](https://github.com/feliperun/finance-os/commit/a03e8190d38f4456520c0890cb4b8c5074b11004))
* **report:** cashflow-chart stacks forecast on bars + projects saldo into future ([#56](https://github.com/feliperun/finance-os/issues/56)) ([4c25ea1](https://github.com/feliperun/finance-os/commit/4c25ea124e69db35ec1e539858ef6079648cd174))

## [2.2.0](https://github.com/feliperun/finance-os/compare/v2.1.0...v2.2.0) (2026-05-24)


### Features

* **report:** add detailed cashflow forecast TUI ([ade6d34](https://github.com/feliperun/finance-os/commit/ade6d34f36f55c5e9df41b85f65e9a5fa2161ce7))

## [2.1.0](https://github.com/feliperun/finance-os/compare/v2.0.0...v2.1.0) (2026-05-24)


### Features

* **report:** add cashflow-chart subcommand with SVG output and forecast overlay ([#52](https://github.com/feliperun/finance-os/issues/52)) ([dc3140f](https://github.com/feliperun/finance-os/commit/dc3140f15516a4fc77e9a4d5a9053491d789b3ee))

## [2.0.0](https://github.com/feliperun/finance-os/compare/v1.6.3...v2.0.0) (2026-05-24)


### ⚠ BREAKING CHANGES

* **report:** cashflow agora é cash-basis em contas correntes, com saldo inicial/final ([#50](https://github.com/feliperun/finance-os/issues/50))

### Features

* **report:** cashflow agora é cash-basis em contas correntes, com saldo inicial/final ([#50](https://github.com/feliperun/finance-os/issues/50)) ([4a6ca49](https://github.com/feliperun/finance-os/commit/4a6ca490b7ce42b6a0ee968852430a6b25c6cdf9))

## [1.6.3](https://github.com/feliperun/finance-os/compare/v1.6.2...v1.6.3) (2026-05-23)


### Bug Fixes

* **cli:** self update looks for fin binary, not finance-cli ([#48](https://github.com/feliperun/finance-os/issues/48)) ([8f33705](https://github.com/feliperun/finance-os/commit/8f3370524b3a393442dd3deacf8f72697b703051))

## [1.6.2](https://github.com/feliperun/finance-os/compare/v1.6.1...v1.6.2) (2026-05-23)


### Bug Fixes

* **storage:** add context field to TransactionAnatomyPatch; fix set-context commands ([#46](https://github.com/feliperun/finance-os/issues/46)) ([1877729](https://github.com/feliperun/finance-os/commit/18777291362630f559591d54fa700f4138bdaf0b))

## [1.6.1](https://github.com/feliperun/finance-os/compare/v1.6.0...v1.6.1) (2026-05-22)


### Bug Fixes

* **cli:** remove dead functions section_header and category_subtotal from human_format ([#44](https://github.com/feliperun/finance-os/issues/44)) ([c622e11](https://github.com/feliperun/finance-os/commit/c622e113672bb956d2572e39c466a125efc6acee))

## [1.6.0](https://github.com/feliperun/finance-os/compare/v1.5.0...v1.6.0) (2026-05-22)


### Features

* **tui:** review TUI overhaul — fin shortcut, BigQuery default, bulk mode, optimistic save ([b9e9fc2](https://github.com/feliperun/finance-os/commit/b9e9fc2e237e755ad32cb53ad19127e85ea83ef6))

## [1.5.0](https://github.com/feliperun/finance-os/compare/v1.4.0...v1.5.0) (2026-05-20)


### Features

* **cli:** speed up review TUI for bulk categorization ([b5f3e22](https://github.com/feliperun/finance-os/commit/b5f3e2215e395264d06e1d7d59db7bec53a39f0b))


### Bug Fixes

* **cli:** fix review tui editing flow ([fd533da](https://github.com/feliperun/finance-os/commit/fd533daa80c738730e6ab804450caad7aa64faf8))

## [1.4.0](https://github.com/feliperun/finance-os/compare/v1.3.2...v1.4.0) (2026-05-20)


### Features

* **tx:** redesign transaction anatomy and human review workflow ([#36](https://github.com/feliperun/finance-os/issues/36)) ([80631fd](https://github.com/feliperun/finance-os/commit/80631fdce399f0c7c3d01da4fa36b6aa7745f3c2))

## [1.3.2](https://github.com/feliperun/finance-os/compare/v1.3.1...v1.3.2) (2026-05-19)


### Bug Fixes

* **reports:** zero-variance sign, forecast emoji, Pluggy EN→PT category normalization ([1945f5c](https://github.com/feliperun/finance-os/commit/1945f5c14c9ad63ce50b1218d22518cebaae7206))

## [1.3.1](https://github.com/feliperun/finance-os/compare/v1.3.0...v1.3.1) (2026-05-19)


### Bug Fixes

* **bigquery:** add migration 031 to fix amount_cents views for v1.3.0 users ([6ebaf61](https://github.com/feliperun/finance-os/commit/6ebaf6139f3def37ab39ba36a26410bcd7fb2a51))

## [1.3.0](https://github.com/feliperun/finance-os/compare/v1.2.0...v1.3.0) (2026-05-19)


### Features

* decimal precision — amount_cents column and view rewrite (ADR-0003) ([#31](https://github.com/feliperun/finance-os/issues/31)) ([704114a](https://github.com/feliperun/finance-os/commit/704114af136ae78b9d35298c7375e4f189bcee4a))

## [1.2.0](https://github.com/feliperun/finance-os/compare/v1.1.0...v1.2.0) (2026-05-18)


### Features

* **payment-status:** canonicalise to posted/pending/installment ([0771140](https://github.com/feliperun/finance-os/commit/07711404f1b768765b2874459f0dd09a09f18cef))


### Bug Fixes

* **cashflow:** treat cashback as expense-reduction, not income ([e2c3ea3](https://github.com/feliperun/finance-os/commit/e2c3ea3f34d4c7ea1843ef1ef868d646e0e6f8fc))
* data hygiene sweep — payment_status, slugs, fallback, streaming, cashback, dedup, phantom account, decimal ([cc1d28d](https://github.com/feliperun/finance-os/commit/cc1d28d4241eb3d8bfd21c7e23e63bee181b9dc2))

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
