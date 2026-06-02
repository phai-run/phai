<!--
  Título do PR: Conventional Commit (feat:, fix:, refactor:, docs:, chore:…).
  Release Please usa o título pra derivar CHANGELOG e versão.
-->

## O que muda

<!-- O quê e, principalmente, por quê. Qual problema/dor isso resolve? -->

## Como testar

<!-- Passos pra um revisor reproduzir. Comandos, rota da UI, ou cenário. -->

## Notas pro revisor

<!-- Trade-offs, decisões, o que ficou de fora, follow-ups. Opcional. -->

## Impacto

- [ ] Mudou flag/subcomando/relatório da CLI → `README.md` atualizado
- [ ] Decisão estrutural → ADR em `docs/adr/` (+ índice) na mesma mudança
- [ ] Migration → idêntica em `schema/sqlite/` e `schema/bigquery/`, idempotente, registrada em `migrations.rs`
- [ ] Mudou caminho de escrita → emite `AuditEvent`
- [ ] Mudança visível na web → screenshot/GIF anexado

## Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `sentrux gate .` sem degradação · `sentrux check .` limpo
- [ ] Web (se aplicável): `pnpm typecheck && pnpm build && pnpm test`
- [ ] **Sem dados pessoais, segredos ou credenciais** — contrapartes, contas, OFX, totais reais (rodei `git diff` antes de subir)
