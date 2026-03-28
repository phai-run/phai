# Finance OS

Scaffold inicial do Finance OS v1.

## Objetivo

- BigQuery como fonte de verdade dos dados
- CLI em Rust como engine única de escrita/leitura
- `audit_log` append-only para rastreabilidade
- Google Sheets consumindo views do BigQuery

## Workspace

- `crates/finance-core`: domínio, config, storage, Pluggy e import legacy
- `crates/finance-cli`: comandos `auth`, `admin`, `sync`, `tx`, `forecast`, `rule`, `account`, `report`
- `schema/bigquery`: DDL do BigQuery
- `schema/sqlite`: backend local de desenvolvimento/teste

## Milestone Zero

Entrega funcional mínima:

- `finance auth setup`
- `finance admin migrate`
- `finance sync pluggy --fixture ...`
- `finance report daily-pulse`

## Exemplo rápido

```bash
cd finance-os
cargo run -p finance-cli -- auth setup --backend local --actor-id local-dev
cargo run -p finance-cli -- admin migrate
cargo run -p finance-cli -- sync pluggy --fixture examples/pluggy_fixture.json
cargo run -p finance-cli -- report daily-pulse
```
