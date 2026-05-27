# Changelog

## [1.1.1](https://github.com/feliperun/finance-os/compare/finance-core-v1.1.0...finance-core-v1.1.1) (2026-05-27)


### Bug Fixes

* **migrations:** renumber card-summary fix to 035 and register it ([6ee3b85](https://github.com/feliperun/finance-os/commit/6ee3b8584fb67e7949f0f0c0bdc89c0c3a270296))
* **migrations:** renumber card-summary fix to 035 and register it ([8f5b10a](https://github.com/feliperun/finance-os/commit/8f5b10a2dbdf545b11b9300c845f48d0c24ccf17))

## [1.1.0](https://github.com/feliperun/finance-os/compare/finance-core-v1.0.1...finance-core-v1.1.0) (2026-05-26)


### Features

* **serve:** web dashboard com WebSocket API para forecast interativo ([#71](https://github.com/feliperun/finance-os/issues/71)) ([b416273](https://github.com/feliperun/finance-os/commit/b4162739553f45bb6336f1b89f8637e95a038309))

## [1.0.1](https://github.com/feliperun/finance-os/compare/finance-core-v1.0.0...finance-core-v1.0.1) (2026-05-25)


### Bug Fixes

* trigger release-please after non-CC squash merge of [#67](https://github.com/feliperun/finance-os/issues/67) ([2d5f6ba](https://github.com/feliperun/finance-os/commit/2d5f6ba2c42ce03d10c2ecc8fcdc4a2a9f3643fe))

## [1.0.0](https://github.com/feliperun/finance-os/compare/finance-core-v0.3.0...finance-core-v1.0.0) (2026-05-25)


### ⚠ BREAKING CHANGES

* **report:** cashflow agora é cash-basis em contas correntes, com saldo inicial/final ([#50](https://github.com/feliperun/finance-os/issues/50))

### Features

* add BigQuery transaction splits ([44f3fbf](https://github.com/feliperun/finance-os/commit/44f3fbf2e1ff93e865d35ff63c11cd65757b67b7))
* **cards:** group v_card_summary by billing cycle, add v_card_open_now ([6b11616](https://github.com/feliperun/finance-os/commit/6b11616f99213f4b06606156d1def62646985b2d))
* checkpoint entrega 1 groundwork ([aaf88b7](https://github.com/feliperun/finance-os/commit/aaf88b7b5ea5928dc795df453dfb90acd8752737))
* **cli:** add review dashboard and release v0.2.0 ([19be093](https://github.com/feliperun/finance-os/commit/19be093d26d56a6950d5be83d09a4b67d6cd11af))
* **cli:** self-update + port legacy operational features (entregas 2, 4B, 5B) ([#3](https://github.com/feliperun/finance-os/issues/3)) ([8ca2ff3](https://github.com/feliperun/finance-os/commit/8ca2ff39852de12b5844298909d5575b75c27f8e))
* decimal precision — amount_cents column and view rewrite (ADR-0003) ([#31](https://github.com/feliperun/finance-os/issues/31)) ([704114a](https://github.com/feliperun/finance-os/commit/704114af136ae78b9d35298c7375e4f189bcee4a))
* **enrichment:** accept ANTHROPIC_AUTH_TOKEN as alias for ANTHROPIC_API_KEY ([2b54b93](https://github.com/feliperun/finance-os/commit/2b54b931652a1c0e78ef58409c3d22103b7009f6))
* **enrichment:** LLM-driven transaction enrichment pipeline ([#19](https://github.com/feliperun/finance-os/issues/19)) ([d112a37](https://github.com/feliperun/finance-os/commit/d112a3770242550d37187e12d633c5099fd1aba7))
* **enrichment:** richer context — weekend flag, PT-BR hour labels, purchaseDate priority, DDG web search ([84ea169](https://github.com/feliperun/finance-os/commit/84ea1692d0246efd4096d686dca60dec8269a028))
* **forecast:** forecast_template schema + FinanceStore CRUD (ADR-0016 PR 1/5) ([#59](https://github.com/feliperun/finance-os/issues/59)) ([a3cd76a](https://github.com/feliperun/finance-os/commit/a3cd76a436971afa832847aac89598b34ff5d639))
* **forecast:** Layer 1 — installments auto-forecast (ADR-0016 PR 2/5) ([#60](https://github.com/feliperun/finance-os/issues/60)) ([402b46f](https://github.com/feliperun/finance-os/commit/402b46f0b4a2edd0b5bd00ffb303158100b0f2f2))
* **payment-status:** canonicalise to posted/pending/installment ([0771140](https://github.com/feliperun/finance-os/commit/07711404f1b768765b2874459f0dd09a09f18cef))
* proactive pulse, cycle-aware cards, saldo em conta, phone-friendly sync notify ([d48afd1](https://github.com/feliperun/finance-os/commit/d48afd1b18875d6349849690c919e8bf1be7af10))
* **pulse:** proactive closing-plan WhatsApp message + notify webhook ([9a6d0a8](https://github.com/feliperun/finance-os/commit/9a6d0a887b8ece090373a1820dde2c908e0142a2))
* **report:** add data health and scenario diagnostics ([a667c06](https://github.com/feliperun/finance-os/commit/a667c06de168e09e6547aafd4bc3dbfee6e4c559))
* **report:** cashflow agora é cash-basis em contas correntes, com saldo inicial/final ([#50](https://github.com/feliperun/finance-os/issues/50)) ([4a6ca49](https://github.com/feliperun/finance-os/commit/4a6ca490b7ce42b6a0ee968852430a6b25c6cdf9))
* **reports:** add saldo em conta — pulse block + `report balances` ([4d8c351](https://github.com/feliperun/finance-os/commit/4d8c351021862ec7b21b72a08599705dd731402a))
* **reports:** new `report cards` + English help text on all reports ([#13](https://github.com/feliperun/finance-os/issues/13)) ([7bff310](https://github.com/feliperun/finance-os/commit/7bff310e42ae962a318d96ed1ff5351ec030b0d5))
* **tui:** review TUI overhaul — fin shortcut, BigQuery default, bulk mode, optimistic save ([b9e9fc2](https://github.com/feliperun/finance-os/commit/b9e9fc2e237e755ad32cb53ad19127e85ea83ef6))
* **tx:** redesign transaction anatomy and human review workflow ([#36](https://github.com/feliperun/finance-os/issues/36)) ([80631fd](https://github.com/feliperun/finance-os/commit/80631fdce399f0c7c3d01da4fa36b6aa7745f3c2))


### Bug Fixes

* **bigquery:** add debug output to migration runner, fix no-op SQL ([a25c087](https://github.com/feliperun/finance-os/commit/a25c087fe484c6142b990988bb4b6875cbee60ab))
* **bigquery:** add migration 031 to fix amount_cents views for v1.3.0 users ([6ebaf61](https://github.com/feliperun/finance-os/commit/6ebaf6139f3def37ab39ba36a26410bcd7fb2a51))
* **bigquery:** cast NULL dates explicitly so type inference stays DATE ([#64](https://github.com/feliperun/finance-os/issues/64)) ([8564083](https://github.com/feliperun/finance-os/commit/85640838cb8c31825e3addd16adf2fdc97af8583))
* **bigquery:** revert dedup to p.amount = t.amount, add migration 032 ([d50b6cc](https://github.com/feliperun/finance-os/commit/d50b6cca5f1c5cb317633da778ca183e80504640))
* **cashflow:** treat cashback as expense-reduction, not income ([e2c3ea3](https://github.com/feliperun/finance-os/commit/e2c3ea3f34d4c7ea1843ef1ef868d646e0e6f8fc))
* data hygiene sweep — payment_status, slugs, fallback, streaming, cashback, dedup, phantom account, decimal ([cc1d28d](https://github.com/feliperun/finance-os/commit/cc1d28d4241eb3d8bfd21c7e23e63bee181b9dc2))
* **decimal:** SUM cashflow in Rust to honour ADR-0003 on SQLite ([d2c9dbf](https://github.com/feliperun/finance-os/commit/d2c9dbf85bc28d15eea903df8e20a6dc747ce6e0))
* **enrichment:** set max_tokens on Anthropic completion retry agent ([29f3fb5](https://github.com/feliperun/finance-os/commit/29f3fb5735c545e69b1c7f0bcfd6e93af343fd2d))
* **entrega1:** close all 6 review findings + 3 structural gaps ([ecb0610](https://github.com/feliperun/finance-os/commit/ecb061018d5b6872f7d659cd0756294baea85d06))
* **installments:** detect Pluggy installment markers from metadata ([0078424](https://github.com/feliperun/finance-os/commit/0078424b4f3c5f563083e344625587a48a7c8c97))
* **legacy:** drop phantom empty-id row in accounts; harden CSV importer ([864b9f5](https://github.com/feliperun/finance-os/commit/864b9f50a62fc95820f6b8d7b54db48c7bd740d8))
* **pluggy:** correct sign for credit-card credits and FX amounts ([#26](https://github.com/feliperun/finance-os/issues/26)) ([64426e2](https://github.com/feliperun/finance-os/commit/64426e20a568f7e6dcec256f343945ba2c6d6e3c))
* **pluggy:** enrich description with installment marker at sync time ([2a712c9](https://github.com/feliperun/finance-os/commit/2a712c9c620382f2ff6d4607d5b5ee73d06d87f2))
* **pluggy:** estornos sem type=CREDIT agora armazenados como crédito positivo ([068416c](https://github.com/feliperun/finance-os/commit/068416cbc494b30c10e065f00d4463b0da24e9f4))
* **release:** make crate versions explicit for release-please ([d0c7d73](https://github.com/feliperun/finance-os/commit/d0c7d73a85eb24f392a9e84b590c02e586e5d9c8))
* **reports:** zero-variance sign, forecast emoji, Pluggy EN→PT category normalization ([1945f5c](https://github.com/feliperun/finance-os/commit/1945f5c14c9ad63ce50b1218d22518cebaae7206))
* satisfy stable clippy default lint ([70370fd](https://github.com/feliperun/finance-os/commit/70370fdd4f2387a33329ee3ab61ae566ba137682))
* **storage:** add context field to TransactionAnatomyPatch; fix set-context commands ([#46](https://github.com/feliperun/finance-os/issues/46)) ([1877729](https://github.com/feliperun/finance-os/commit/18777291362630f559591d54fa700f4138bdaf0b))
* **sync:** preserve manual category annotations during Pluggy re-sync ([b569ab5](https://github.com/feliperun/finance-os/commit/b569ab57a004f2c6bfe964a594b6464a02e316b3))
* **taxonomy:** consolidate `---` slug duplicates into canonical `-` ([dcb59de](https://github.com/feliperun/finance-os/commit/dcb59de6071d375733d250a993252c0caaf375e2))
* **taxonomy:** move moradia:streaming → assinaturas:streaming ([f31a4bf](https://github.com/feliperun/finance-os/commit/f31a4bff7046aadbd9d4efde374174f9e4ddff65))
* **taxonomy:** route fallback rows from `outros:geral` to `_revisar` ([c4850ad](https://github.com/feliperun/finance-os/commit/c4850ad05b27b6dd96ddf5393337bd71c9fbb907))
* trigger release for card insights corrections ([f673aff](https://github.com/feliperun/finance-os/commit/f673affaaf91953e70a0203880e06633a60327b7))
