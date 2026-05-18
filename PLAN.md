# Plano de continuação — auditoria 2026-05-18

> **Origem.** Esta é a memória da auditoria geral feita em 2026-05-18 que
> sintetizou os erros conceituais / arquiteturais do repositório. Use como
> referência se outro agente continuar o trabalho. Os dois primeiros lotes
> (pulse proativo + cycle de cartões + saldo + notify UX) já entraram via
> [PR #27](https://github.com/feliperun/finance-os/pull/27).
>
> Se este arquivo ficar desatualizado, prefira o estado atual do código
> (e o `git log`). Não tente seguir cegamente cada item — releia o
> diagnóstico no histórico de conversa antes de iniciar cada PR.

## O que já foi feito (PR #27)

- **`report daily-pulse`** virou plano de fechamento proativo (cinco blocos:
  mês até hoje + delta T3M, frear neste mês, a vencer, saldo em conta,
  cartões em aberto, ações). Bloco "frear" é robusto a custo fixo lumpy
  (count < 3 ⇒ compara com baseline cheia, não pro-rata).
- **`notify whatsapp`** novo subcomando que POSTa o corpo num webhook
  (`FINANCE_OS_WHATSAPP_WEBHOOK_URL` + opcional bearer token).
- **`v_card_summary` agora agrupa por ciclo de fatura**, não mês civil. Nova
  view `v_card_open_now` retorna o ciclo fechado-com-saldo por cartão.
- **Saldo em conta** aparece no pulse (sempre) e via `report balances`
  (agrupado por owner com totais).
- **`sync --notify-summary`** virou WhatsApp-friendly (era pipe-separated
  CLI log). Inclui saldo quando há tx novas.
- **`FINANCE_OS_NO_AUTO_UPDATE=1`** agora desativa também o force-check do
  `sync pluggy` — destrava testes locais reprodutíveis.
- ADRs 0009 (pulse) e 0010 (card cycle).
- `docs/whatsapp-pulse-cron.md` com receita cron + launchd.

## O que ficou para depois (ordenado por dependência)

### ✅ 1. Normalização de `payment_status` — feito

ADR-0011 + migrations 021 (sqlite) / 022 (bq). Vocabulário canônico:
`posted` / `pending` / `installment`. `v_card_summary.open_amount` agora
soma apenas `pending`; nova coluna `installments_future` surface parcelas
separadamente. Pulse mostra `+R$X em parcelas` ao lado do "em aberto".
Pluggy sync normaliza no ingestion via `normalize_payment_status()`.
`is_open_card_payment_status` no CLI mantém compat com PT/legacy aliases
para tolerar deployment rolling.

### ✅ 2. Consolidação de slugs `---` ↔ `-` — feito

Migrations 022 (sqlite) / 023 (bq) fazem `REPLACE(category_id, '---', '-')`
em transactions, forecast, category_budgets, internal_categories e
categories (com lógica de consolidação para evitar conflito de PK quando
ambos `x-y` e `x---y` existem). Slugifier atual já produz dash único —
regression test em `idempotency.rs` garante que `Bar / Baz` etc. nunca
voltam a gerar `---`.

### ✅ 3. `outros:geral` fallback dump → `_revisar` — feito

Migrations 023 (sqlite) / 024 (bq) introduzem a categoria reservada
`_revisar` e re-rotam rows fallback (`category_source='fallback'` AND
`category_id='outros:geral'`) para ela. `_revisar` começa com `_` que o
slugifier nunca produz (filtra `is_ascii_alphanumeric`), então não
colide com slug de usuário. `v_uncategorized` continua catching ambos
(via predicate em `category_source`).

Limitações: nenhum código emite `category_source='fallback'` hoje (o
caso da auditoria veio de versão histórica). Se um futuro caminho de
código tentar emitir fallback de novo, vale escrever direto em
`_revisar` em vez de em `outros:geral`. Não há ADR específica aqui — o
fix é puramente dado.

### ✅ 4. Streaming taxonomy — feito

Migrations 024 (sqlite) / 025 (bq) criam `assinaturas:streaming` e
movem todas as transactions, forecasts e budgets de `moradia:streaming`
para `assinaturas:streaming`. A categoria `moradia:streaming` é
deletada após a migração.

Limitação: rules persistem o label humano ("Moradia" / "Streaming") e
slugificam no apply, então rules que produzem `moradia:streaming` no
caminho de sincronização vão continuar fazendo isso. O usuário precisa
editar essas rules manualmente para mudar para `assinaturas:streaming`
(via `finance rule upsert`).

### ✅ 5. Cashback como redução de despesa — feito

Migrations 025 (sqlite) / 026 (bq) redefinem `v_cashflow`: cashback
(rows com `category_id='cashback'` e amount positivo) sai do bucket
`income` e vai para uma coluna nova `expense_reduction`. `CashflowRow`
ganha o campo; pulse renderiza "💸 cashback R$X · saídas líquidas R$Y"
quando há cashback no mês. Income agora reflete só inflows reais
(salário, transferências recebidas, etc.).

### ✅ 6. Dedup heurística secundária — feito

`sync pluggy` agora roda `dedup_pluggy_duplicates` antes do upsert.
Fingerprint = `(date, account, signed_amount, normalize(description))`;
match contra rows existentes em `transactions_in_date_range` no mesmo
range de datas que o batch (only `source='pluggy'` para evitar falso
positivo contra entradas manuais). Quando colide com um existente de
ID diferente, a row nova é descartada e um `AuditEvent` `tx.dedup_skipped`
é emitido com `skipped_transaction_id` / `matched_existing_id` /
`fingerprint` no diff. Sem mudança de schema.

Limitação conhecida: gastos legítimos repetidos no mesmo dia (e.g.
duas tarifas idênticas de pedágio na mesma manhã) seriam suprimidos.
Aceitável dado o ratio Pluggy-dupe vs legítimo observado. Se for
problema, adicionar `--no-dedup` no CLI ou marcar manualmente como
não-duplicata via metadata_json.

### 7. Linha-fantasma de `accounts` (account_id vazio)

**Problema.** O importer legacy criou uma row em `accounts` com
`account_id=''` (e todos os outros campos vazios), source =
`legacy_accounts_csv`. Não tem transações ligadas mas suja
`get_accounts()`.

**Plano.**
1. Bug fix em `crates/finance-core/src/legacy.rs` (CSV importer):
   skip rows where `id` é vazio.
2. Migração one-shot:
   `DELETE FROM accounts WHERE account_id='' AND NOT EXISTS (SELECT 1 FROM transactions WHERE account_id='accounts.account_id')`.
3. Teste regression no importer com CSV malformado.

### 8. Replace `CAST(amount AS REAL)` por SQL decimal-safe

**Problema.** ADR-0003 promete Decimal end-to-end. Mas
`v_monthly_spend`, `v_cashflow`, `v_card_summary`, `v_forecast_vs_actual`
fazem `CAST(amount AS REAL)` em SUM/comparison. f64 contamina os relatórios.

**Plano.**
1. SQLite armazena Decimal como TEXT. Para somar precisamente sem REAL:
   usar UDF `sqlite_decimal` ou armazenar centavos como INTEGER em uma
   coluna shadow (`amount_cents`) computada por trigger. Trade-off: o
   UDF não está disponível em `rusqlite` sem feature flag.
2. Solução pragmática: continuar lendo TEXT, fazer agregação no
   aplicativo Rust em `Decimal`. Refatorar as views para retornar TEXT
   e mover SUM para o lado Rust.
3. BigQuery: trocar `IF(amount < 0, ABS(amount), 0)` (que opera em
   NUMERIC nativo — já é decimal-safe) — apenas o display string
   coercion (`CAST(... AS STRING)`) está OK.
4. ADR atualizando 0003 para refletir essa realidade.
5. Trabalho significativo — provavelmente é o último.

## Princípios operacionais ao continuar

- **Cada item vira 1 PR**. Conventional Commits.
- **Migrations são append-only**. Se uma migration nova precisa corrigir
  uma anterior já aplicada (no BQ de dev), adicione uma segunda em vez de
  editar a primeira.
- **Testar contra SQLite local** (e2e). BigQuery é validado por inspeção
  manual + parity review.
- **Sempre rodar antes do commit:**
  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test --workspace
  ```
- **Não silenciar warnings com `#[allow(dead_code)]`** sem motivo
  documentado. Remova o código morto.
- **Privacy**: nenhum nome próprio em código compartilhado. Regras de
  classificação pessoal vão no `rules` table do usuário.
- **CLI auto-update no dev**: exportar `FINANCE_OS_NO_AUTO_UPDATE=1` ao
  invocar `./target/debug/finance-cli` manualmente, ou o binário será
  reescrito pela release publicada.

## Próximo PR sugerido

**`payment_status` normalization** — destrava o significado de
`open_amount`, que cascateia em correções de outras views e no pulse.
Sem isso, "Cartões em aberto" continua misturando cobrança corrente com
parcelas futuras.
