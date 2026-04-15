# Plano de Ajustes Pendentes da Entrega 1

## Origem

Em 2026-04-14 foi consolidado um plano para portar gaps operacionais do legado para o `finance-os`.
A primeira entrega recomendada era:

1. `1A` Auto-rebind Pluggy por `itemId`
2. `2A` Enriquecer `sync pluggy --json-summary`
3. `6A` Inspecao de regras e contextos

Esta branch contem a tentativa dessa Entrega 1.

## O que ja entrou na branch

- Rebind Pluggy por `itemId` em `crates/finance-core/src/pluggy.rs`
- Enriquecimento de `--json-summary` em `crates/finance-cli/src/main.rs`
- Novos comandos:
  - `rule list`
  - `rule inspect`
  - `tx list-context`
- Expansao do storage para suportar os novos caminhos:
  - `all_rules`
  - `transactions_with_context`
  - `count_uncategorized`
- Testes E2E e unitarios atualizados

## Estado atual

`cargo test --workspace` passa em verde (28 testes: 12 e2e + 13 unit finance-core + 3 unit cli).
Todos os 6 findings abaixo foram fechados no followup da branch, alem de 3 lacunas estruturais
(dead API removida, teste de rebind real, teste de divergencia/colisao). Entrega 1 pronta.

## Findings (todos FECHADOS)

### P1

1. [FECHADO] `pluggyItemId` do config sobrepoe o valor do CSV sem validacao
   - Arquivo: `crates/finance-core/src/pluggy.rs`
   - Impacto: um 404 pode rebinder para o item errado e importar dados de outra conta para o `account_id` interno esperado.

2. [FECHADO] Dois bindings podem convergir para o mesmo `pluggy_account_id` final
   - Arquivo: `crates/finance-core/src/pluggy.rs`
   - Impacto: o mesmo conjunto de transacoes pode ser materializado em duas tasks, e o ultimo upsert vence de forma nao deterministica.

### P2

3. [FECHADO] `needsContextCount` do `--json-summary` continua truncado em 100
   - Arquivo: `crates/finance-cli/src/main.rs`
   - Impacto: backlog real de pendencias pode ser subreportado.

4. [FECHADO] Leitura de timestamps no backend BigQuery pode cair silenciosamente em `Utc::now()`
   - Arquivos: `crates/finance-core/src/storage/bigquery.rs`, `crates/finance-core/src/models.rs`
   - Impacto: `rule list` e `rule inspect` podem mostrar timestamps incorretos em BigQuery.

5. [FECHADO] `rule list --status` aceita typo como se fosse um resultado vazio legitimo
   - Arquivo: `crates/finance-cli/src/main.rs`
   - Impacto: UX enganosa para um comando de inspecao.

6. [FECHADO] `tx list-context` em modo texto nao mostra `transaction_id`
   - Arquivo: `crates/finance-cli/src/main.rs`
   - Impacto: a saida texto nao e acionavel sem rerodar em JSON.

## Lacunas estruturais observadas

- [FECHADO] O metodo `find_account_by_pluggy_item_id` foi removido do trait e das
  duas implementacoes. A fonte de verdade para resolucao de binding agora e apenas
  `pluggy-config.json` + `contas.csv`, validados cruzadamente.
- [FECHADO] Teste de rebind real no HTTP path (`404 -> GET /accounts?itemId=...`)
  implementado com wiremock em `pluggy::tests::http_rebind_via_item_id_on_404`.
  Cobre auth, 404 do account original, fallback por itemId, e fetch de transacoes.
- [PENDENTE] Testes BigQuery nao rodam porque requerem credenciais GCP. Fica como
  follow-up para quando o ambiente de CI tiver secrets do sandbox.

## Plano de implementacao recomendado

### Etapa 1: fechar os P1 do rebind Pluggy

1. Unificar a resolucao de `pluggyItemId`
   - Validar config, CSV e historico persistido antes do fallback.
   - Se houver divergencia entre fontes, falhar explicitamente.
   - Remover precedencia silenciosa.

2. Validar unicidade do `pluggy_account_id` final resolvido
   - Resolver todos os bindings primeiro.
   - Antes de buscar transacoes, detectar colisao entre bindings.
   - Em caso de colisao, abortar com erro claro listando os bindings envolvidos.

3. Integrar de fato o storage ao fluxo
   - Decidir se `find_account_by_pluggy_item_id` deve participar da resolucao.
   - Se sim, usar esse historico como mais uma fonte valida.
   - Se nao, remover/refatorar o metodo para evitar API morta.

### Etapa 2: corrigir o contrato do json-summary

1. Separar total real de itens retornados
   - `needsContextCount` deve refletir o total real.
   - Adicionar campo de truncamento, por exemplo `needsContextReturnedCount` e `needsContextTruncated`.

2. Adicionar query de contagem no storage
   - Nao reaproveitar o slice limitado como contagem real.

### Etapa 3: corrigir timestamps BigQuery

1. Ajustar parsing
   - Aceitar formato RFC3339 e o formato textual retornado pelo BigQuery, ou
   - Normalizar o SQL para emitir timestamps em formato RFC3339.

2. Aplicar o ajuste em todos os caminhos novos
   - `all_rules`
   - `find_account_by_pluggy_item_id`
   - Qualquer outro ponto novo que use `parse_datetime_or_now`

### Etapa 4: fechar UX dos comandos novos

1. Validar `rule list --status`
   - Migrar o argumento para enum validado por `clap`
   - Estados sugeridos: `active`, `disabled`, `all`

2. Tornar `tx list-context` texto acionavel
   - Incluir `transaction_id` no output texto
   - Preferencia: primeira coluna

### Etapa 5: endurecer cobertura

1. Adicionar testes de rebind real
   - 404 no lookup por `accountId`
   - fallback por `itemId`
   - conflito config vs CSV
   - colisao de dois bindings no mesmo `pluggy_account_id`

2. Adicionar testes de summary
   - backlog maior que 100
   - verificacao de truncamento sinalizado

3. Adicionar testes focados de BigQuery
   - pelo menos para parsing/mapeamento de timestamps
   - idealmente tambem para os novos paths do adapter

## Ordem recomendada

1. Corrigir os dois `P1` do rebind
2. Corrigir `needsContextCount`
3. Corrigir timestamps BigQuery
4. Validar `rule list --status`
5. Ajustar `tx list-context`
6. Ampliar cobertura
7. Rodar `cargo test --workspace`

## Criterio de pronto

- Nenhum dos 6 findings permanece aberto
- Rebind falha em conflito ou ambiguidade, em vez de seguir silenciosamente
- `needsContextCount` reflete o total real
- BigQuery nao reescreve timestamps para `now`
- `rule list` rejeita status invalido
- `tx list-context` texto mostra `transaction_id`
- Existe cobertura para o fallback real de rebind e para os pontos criticos de BigQuery
- `cargo test --workspace` passa
