# Plano: Portar Gaps do Legado para finance-os

## Contexto

O finance-os (Rust/BigQuery) substituiu o finance-analyzer (JS/Python/CSV) como runtime oficial, mas a camada operacional fina do legado nao foi portada. O relatorio de 2026-04-14 identificou 15 gaps organizados em 4 blocos de prioridade. Este plano cobre a implementacao de todos eles em fases incrementais e entregaveis.

A base arquitetural do finance-os e solida: storage trait com dual-backend, migrations, idempotencia, audit log, rules engine. O trabalho e adicionar capacidade operacional sobre essa base.

---

## Fase 1: Resiliencia Pluggy + Snapshots de Conta

### 1A. Auto-rebind Pluggy por itemId [M]
- **Problema**: `fetch_account_details` falha com 404 quando Pluggy roda o account ID. Nao ha fallback.
- **Solucao**: Retry com `GET /accounts?itemId=...` quando 404. Adicionar `pluggyItemId` opcional em `PluggyBindingConfig`.
- **Arquivos**:
  - `crates/finance-core/src/pluggy.rs` — nova fn `fetch_accounts_by_item`, retry no spawn block (~L483)
  - `crates/finance-core/src/storage/mod.rs` — novo trait method `find_account_by_pluggy_item_id`
  - `crates/finance-core/src/storage/local.rs` + `bigquery.rs` — implementacao (SELECT por pluggy_item_id)
- **Teste**: Fixture com pluggyAccountId diferente do config mas mesmo itemId. Assert sync produz registros corretos.

### 1B. Tabela account_snapshots [M]
- **Problema**: `upsert_accounts` sobrescreve saldo. Nao ha historico.
- **Solucao**: Nova tabela `account_snapshots` (snapshot_id, account_id, snapshot_date, balance, credit_limit, currency_code, source, actor_id, idempotency_key, metadata_json, created_at).
- **Arquivos**:
  - `schema/sqlite/012_account_snapshots.sql` + `schema/bigquery/012_account_snapshots.sql`
  - `crates/finance-core/src/models.rs` — novo `AccountSnapshotRecord`
  - `crates/finance-core/src/storage/mod.rs` — novo `insert_account_snapshots`
  - `storage/local.rs` + `bigquery.rs` — implementacao
  - `crates/finance-core/src/migrations.rs` — registrar migration 012
  - `crates/finance-cli/src/main.rs` — inserir snapshots apos upsert_accounts no sync
- **Idempotencia**: `snapshot:{account_id}:{date}:{source}`
- **Teste**: Sync 2x no mesmo dia = idempotente. Sync em dias diferentes = 2 snapshots.

---

## Fase 2: Summary Rico + Shortcuts de Contexto

### 2A. Enriquecer --json-summary [S]
- **Problema**: Summary so tem 8 campos. Pluggy extra (merchant, CNPJ, MCC, payer/receiver) ja esta em metadata_json mas nao e surfaceado.
- **Solucao**: Adicionar `metadataJson`, `txType`, `categorySource`, `dayOfWeek`, `accountLabel` aos structs `SyncSummaryTransaction`/`SyncSummaryPending`.
- **Arquivos**: `crates/finance-cli/src/main.rs` — extender structs e builder
- **Teste**: Atualizar E2E do --json-summary para assert nos campos novos.

### 2B. Shortcuts de contexto [M]
- **Problema**: `tx set-context` exige transaction_id explicito. AI/usuario precisa de atalhos.
- **Solucao**:
  - Novo trait: `find_transactions_by_description(query, limit)`, `latest_uncategorized_transactions(limit)`
  - Novos CLI: `tx find --query "mercado"`, `tx pending --limit 10`, `tx set-context-by-desc --query "mercado" --context "compras"`
- **Arquivos**:
  - `storage/mod.rs` — 2 novos trait methods
  - `storage/local.rs` + `bigquery.rs` — LIKE search + filtered query
  - `main.rs` — 3 novos subcomandos TxCommand
- **Teste**: Sync fixture, `tx find --query "Uber" --json`, assert resultados.

---

## Fase 3: OFX Import + Reconciliacao

### 3A. Parser OFX + Tabelas de Statement [L]
- **Problema**: Nao ha como importar extrato oficial OFX. Pluggy e unica fonte.
- **Solucao**:
  - Novo modulo `crates/finance-core/src/ofx.rs` — parser OFX (state machine para extrair STMTTRN blocks: DTPOSTED, TRNAMT, FITID, NAME/MEMO, LEDGERBAL)
  - Novas tabelas: `statement_summaries` (statement_id, account_id, import_date, source_file, period_start, period_end, ledger_balance, available_balance, currency_code, line_count, ...) e `statement_lines` (line_id, statement_id, account_id, posted_date, amount, fit_id, tx_type, name, memo, match_status, matched_transaction_id, ...)
  - Novo CLI: `sync ofx --file path.ofx --account-id X`
- **Arquivos**:
  - `crates/finance-core/src/ofx.rs` (novo)
  - `crates/finance-core/src/lib.rs` — registrar modulo
  - `schema/sqlite/013_statement_tables.sql` + `schema/bigquery/013_statement_tables.sql`
  - `models.rs` — `StatementLineRecord`, `StatementSummaryRecord`
  - `storage/mod.rs` — `upsert_statement_summaries`, `upsert_statement_lines`, `statement_lines_by_account`
  - `storage/local.rs` + `bigquery.rs`
  - `migrations.rs`
  - `main.rs` — `SyncCommand::Ofx`
- **Idempotencia**: `ofx:{account_id}:{fit_id}` por linha, `statement:{account_id}:{period_end}:{hash}` por summary
- **Teste**: Fixture OFX sintetica em `examples/`. Unit tests no parser + E2E import idempotente.

### 3B. Reconciliacao OFX vs Pluggy [L]
- **Depende de**: 3A
- **Solucao**:
  - Novo modulo `crates/finance-core/src/reconcile.rs`
  - Algoritmo: para cada statement_line unmatched, buscar candidates em transactions (mesmo account, amount +/- 0.01, date +/- 3 dias, overlap textual). Scoring: exact amount=3, date match=2, text=1. Best acima de threshold.
  - Novo CLI: `sync reconcile --account-id X [--dry-run]`
  - Novo report: `report reconciliation-status --account-id X`
- **Arquivos**:
  - `crates/finance-core/src/reconcile.rs` (novo)
  - `lib.rs` — registrar
  - `storage/mod.rs` — `unmatched_statement_lines`, `candidate_transactions_for_reconcile`, `update_statement_line_match`
  - `storage/local.rs` + `bigquery.rs`
  - `main.rs` — 2 novos subcomandos
- **Teste**: Unit tests com pares mao-na-massa cobrindo exact, fuzzy, no-match, multi-candidate.

### 3C. Diagnostico de saldo stale [S]
- **Depende de**: 1B + 3A
- **Solucao**: Report que compara `account_snapshots.balance` (Pluggy) vs `statement_summaries.ledger_balance` (OFX).
- **Arquivos**:
  - `storage/mod.rs` — `balance_comparison`
  - `models.rs` — `BalanceComparisonRow`
  - `storage/local.rs` + `bigquery.rs`
  - `main.rs` — `report stale-balance`

---

## Fase 4: Forecast por Ocorrencias + Parceladas

### 4A. Modelo de ocorrencias [L]
- **Problema**: Forecast e uma linha so. Nao ha expansao por mes, nem status por ocorrencia, nem matching com tx reais.
- **Solucao**:
  - Nova tabela `forecast_occurrences` (occurrence_id, forecast_id, month_ref, due_date, amount, status, matched_transaction_id, ...)
  - Novo modulo `crates/finance-core/src/forecast.rs` — `expand_forecast(template, from, to)`, `match_occurrence_to_actual(occurrence, transactions)`
  - Novo CLI: `forecast expand --from 2026-01 --to 2026-12`, `forecast match --month 2026-04`, `report forecast-occurrences --month 2026-04`
- **Arquivos**:
  - `crates/finance-core/src/forecast.rs` (novo)
  - `lib.rs`
  - `schema/sqlite/014_forecast_occurrences.sql` + `schema/bigquery/014_forecast_occurrences.sql`
  - `models.rs` — `ForecastOccurrenceRecord`
  - `storage/mod.rs` — `upsert_forecast_occurrences`, `forecast_occurrences_by_month`, `update_occurrence_status`
  - `storage/local.rs` + `bigquery.rs`
  - `migrations.rs`
  - `main.rs`
- **Idempotencia**: `occurrence:{forecast_id}:{month_ref}`
- **Teste**: Unit tests para expansao mensal/bimestral/semanal. E2E expand + match.

### 4B. Tracking de parceladas [M]
- **Depende de**: 4A
- **Solucao**:
  - Metadata em `ForecastRecord.metadata_json`: `parcelas_total`, `parcela_atual`, `parcela_inicio_mes`
  - Fn `parse_installment_description(desc) -> Option<(u32, u32)>` — regex para "X/Y", "Parcela X de Y"
  - Report `report installments [--account-id X]` — chains ativas, current/total, data fim projetada, flag "libera no proximo mes"
- **Arquivos**: `forecast.rs`, `main.rs`, `models.rs` (se precisar struct dedicado)
- **Teste**: Unit tests para parser de parcelas. E2E com fixture de tx parcelada.

---

## Fase 5: Cashflow em Camadas + Orcamentos

### 5A. Cashflow em camadas [M]
- **Depende de**: 4A (para camada planner), opcionalmente 5B (para camada orcamento)
- **Solucao**:
  - View `v_layered_cashflow` com UNION das 4 camadas: confirmado (posted), agendado (pending/scheduled), planner (ocorrencias abertas), orcamento_variavel (budget - actual)
  - Novo model `LayeredCashflowRow { month_ref, layer, income, expenses, net }`
  - Novo CLI: `report layered-cashflow --months 6`
- **Arquivos**:
  - `schema/sqlite/015_layered_cashflow_view.sql` + BigQuery
  - `models.rs`, `storage/mod.rs`, backends, `main.rs`, `migrations.rs`
- **Abordagem incremental**: shipar primeiro so com confirmado + agendado. Adicionar planner apos 4A. Adicionar budget apos 5B.

### 5B. Orcamentos por categoria [M]
- **Solucao**:
  - Nova tabela `category_budgets` (budget_id, category_id, month_ref nullable, amount, alert_threshold_pct, ...)
  - UNIQUE em (category_id, month_ref)
  - Report `report budget-status --month 2026-04` — budget, actual (de v_monthly_spend), usage%, projected%, alert
  - CLI: `budget upsert --category-id alimentacao --amount 3000 [--month 2026-04]`
- **Arquivos**:
  - `schema/sqlite/016_category_budgets.sql` + BigQuery
  - `models.rs` — `CategoryBudgetRecord`
  - `storage/mod.rs`, backends, `main.rs`, `migrations.rs`
- **Teste**: E2E upsert budget + sync + report budget-status.

---

## Fase 6: UX, Export, Governanca

### 6A. Inspecao de regras/contextos [S]
- **Solucao**:
  - `rule list [--status active] [--json]`
  - `rule inspect --rule-id X [--json]`
  - `tx list-context [--limit 50] [--json]`
- **Arquivos**: `storage/mod.rs` (novo `all_rules`, `transactions_with_context`), backends, `main.rs`

### 6B. Export CSV / Google Sheets [M]
- **Solucao**:
  - Novo modulo `crates/finance-core/src/export.rs` — serializa qualquer Vec<T: Serialize> para CSV
  - Sheets opcional via Google Sheets API v4 + service account
  - CLI: `report export --report monthly-spend --month 2026-04 --format csv|sheets`
- **Fase 1**: So CSV. Sheets como follow-up.

### 6C. Card open cycle / fatura parcial [M]
- **Solucao**:
  - Novo modulo `crates/finance-core/src/card.rs` — `compute_card_cycle(account, date) -> (start, end)` usando `billing_closing_day` de metadata
  - Report: `report card-cycle --account-id X [--month 2026-04]` — datas do ciclo, txs no ciclo, total parcial, comparacao OFX se disponivel
  - Trait method: `transactions_in_range(account_id, from, to)`
- **Arquivos**: `card.rs` (novo), `lib.rs`, `storage/mod.rs`, backends, `main.rs`

---

## Grafo de Dependencias

```
1A (rebind)     ─── standalone
1B (snapshots)  ─── standalone
2A (summary)    ─── standalone
2B (shortcuts)  ─── standalone
3A (OFX)        ─── standalone
3B (reconcile)  ─── depende 3A
3C (stale bal)  ─── depende 1B + 3A
4A (occurrences)─── standalone
4B (installments)── depende 4A
5A (layered cf) ─── depende 4A, opcionalmente 5B
5B (budgets)    ─── standalone
6A (inspect)    ─── standalone
6B (export)     ─── standalone
6C (card cycle) ─── standalone (enhanced by 3A)
```

## Ordem de Entrega Recomendada

1. **Entrega 1** (valor imediato, risco baixo): 1A + 2A + 6A
2. **Entrega 2** (melhora integracao AI): 1B + 2B
3. **Entrega 3** (confianca financeira): 3A → 3B → 3C
4. **Entrega 4** (planejamento): 4A → 4B
5. **Entrega 5** (visao completa): 5B → 5A
6. **Entrega 6** (polish): 6B + 6C

## Migrations Planejadas

| # | Conteudo |
|---|---|
| 012 | `account_snapshots` |
| 013 | `statement_summaries` + `statement_lines` |
| 014 | `forecast_occurrences` |
| 015 | `v_layered_cashflow` (confirmado + agendado) |
| 016 | `category_budgets` |
| 017 | `v_layered_cashflow` v2 (+ planner + budget) |

## Novos Modulos

| Modulo | Proposito |
|---|---|
| `src/ofx.rs` | Parser OFX |
| `src/reconcile.rs` | Reconciliacao OFX vs Pluggy |
| `src/forecast.rs` | Expansao forecast, occurrences, installments |
| `src/export.rs` | Export CSV/Sheets |
| `src/card.rs` | Ciclo de cartao de credito |

## Verificacao

Cada fase deve:
1. `cargo test --workspace` passa
2. E2E test especifico para o novo feature (padrao: TempDir, auth setup, migrate, fixture seed, comando novo, assert)
3. Migrations aplicam limpo em SQLite e BigQuery
4. Idempotencia verificada (operacao 2x = mesmo resultado)
5. Nenhum dado pessoal em fixtures/tests/migrations (AGENTS.md)
