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

### 2. Consolidação de slugs `---` ↔ `-`

**Problema.** Slugificador antigo produzia chaves duplicadas:
`assinaturas:cloud-storage` AND `assinaturas:cloud---storage` (mesma
categoria, duas chaves). 8 famílias afetadas (ver diagnóstico no
histórico).

**Plano.**
1. UPDATE em massa nas tabelas: `categories`, `transactions`, `forecast`,
   `category_budgets`, `rules`.
2. Corrigir o slugificador em `crates/finance-core/src/idempotency.rs`
   (função `category_id`) — substituir " / " → "-" não " / " → "---".
3. Migração idempotente em ambos backends.
4. Teste regression no slugificador.

### 3. Eliminar `outros:geral` como lixeira invisível

**Problema.** `category_source = 'fallback'` aponta para `outros:geral`,
então 68 transações ficam com `category_id != NULL` e somem de
`v_uncategorized`. Usuário pensa que classificou tudo.

**Plano.**
1. Introduzir categoria especial `_revisar` (com underscore prefixed para
   ordenar no topo das views).
2. Fallback agora aponta para `_revisar`.
3. `v_uncategorized` pega tudo onde category_source IN ('fallback',
   'unclassified') OR category_id = '_revisar' OR category_id IS NULL.
4. Migração: UPDATE transactions SET category_id='_revisar' WHERE
   category_source='fallback' AND category_id='outros:geral'.
5. ADR sobre a semântica de category_source.

### 4. Streaming taxonomy

**Problema.** Netflix, HBO, Disney, Prime, YouTube estão em
`moradia:streaming` enquanto outras assinaturas (`assinaturas:apple`,
`assinaturas:cloud-storage`) seguem o pattern `assinaturas:*`.
Inconsistência conceitual.

**Plano.**
1. Migração que cria `assinaturas:streaming` e UPDATE move forecasts +
   transactions + rules de `moradia:streaming` para `assinaturas:streaming`.
2. ADR opcional (depende — pode ir junto com consolidação de slugs).

### 5. Cashback como redução de despesa

**Problema.** Cashback (R$1857 em março) entra como `income` em
`v_cashflow`. É redução de despesa, não receita; distorce taxa de poupança.

**Plano.**
1. Adicionar categoria `cashback` (já existe — está como categoria flat
   EN) à tabela `internal_categories` OU criar um terceiro bucket no
   `v_cashflow` (`expense_reduction`) e subtrair de `expenses`.
2. Atualizar pulse para mostrar cashback como "redução" no MtD.
3. Atualizar regras se houver pattern PT vs EN duplicado.

### 6. Dedup heurística secundária

**Problema.** Em 2026-02-06 Pluggy emitiu duas tx com transaction_ids
diferentes mas mesma date+amount+account+description ("Pagamento recebido"
R$7905.62 duplicado em aline_cartao). Idempotência por pluggy_id não pega.

**Plano.**
1. No `upsert_transactions`, ao receber um lote, hash secundário
   `(transaction_date, amount, account_id, normalize(description))`.
2. Se hash já existe e o existente também é `source='pluggy'`, marcar a
   nova como `dedup_skipped` em audit_events em vez de inserir.
3. Esquema: adicionar coluna `dedup_hash` em transactions (ou ficar em
   metadata_json) com índice.
4. ADR descrevendo a heurística e seu corner case (legítimas duplicatas
   no mesmo dia/conta/valor — passar `--force` ou tag manual?).

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
