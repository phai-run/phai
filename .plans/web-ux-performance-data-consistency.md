# Plano: `phai serve` UX, performance e consistencia de dados

Status: revisao de integracao concluida; pronto para merge
Branch: `codex/web-ux-performance-data-plan`
Baseline Sentrux: salvo com `sentrux gate --save .`
Data: 2026-05-31

## Objetivo

Transformar o `phai serve` em uma interface de operacao diaria rapida, consistente
com o BigQuery como fonte da verdade, e capaz de carregar um ano inteiro de
transacoes sem travar. A LiveStore/SQLite no browser continua sendo o cache
reativo local; todas as escritas precisam voltar pela bridge Rust, com auditoria.

## Guardrails

- Nao commitar OFX, caminhos privados, nomes de contas, contrapartes reais,
  fingerprints de extrato ou totais derivados de dados reais.
- Os OFX privados do operador serao usados apenas localmente como oraculo de
  validacao final.
- Qualquer correcao de dado real deve ser feita como suporte generico no motor
  ou por regra/configuracao privada fora do repo.
- Toda mudanca de escrita precisa manter `AuditEvent`.
- Migrations, se necessarias, devem existir em SQLite e BigQuery com o mesmo
  prefixo e serem registradas em `migrations.rs`.
- Antes de commit/PR: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --workspace`, `sentrux gate .`, `sentrux check .`, e checks web.

## Code review atual

1. Risco de truncamento de dados no web.
   `GET /api/transactions` tem hard cap de 5000 linhas e o frontend tambem pede
   `limit: 5000`. Isso viola a meta de carregar um ano inteiro se o volume real
   passar desse limite. A resposta ja sinaliza `truncated`, mas a UI nao consome
   esse campo.

2. Lista de transacoes ainda e O(n) renderizado no DOM.
   `MonthDetail` filtra, agrupa e renderiza todos os rows expandidos do mes em
   um unico componente. Para muitos rows, `AnimatePresence` + `height: auto` por
   grupo tende a gerar layout work custoso.

3. Agrupamento por categoria nao entende pai/subcategoria.
   Hoje a chave de agrupamento e o `categoryId` completo. Isso mostra total por
   chave, mas nao separa `categoria` e `subcategoria`, nem mostra subtotal das
   subcategorias dentro da categoria pai.

4. Recategorizacao e forecast move ainda dependem de modal/drag parcial.
   Transacao abre modal para editar categoria; nao ha selecao por teclado, comando
   rapido, nem drop target por categoria. Forecast ja arrasta para o grafico, mas
   nao tem atalho de teclado nem regra explicita para meses futuros/mes aberto.

5. Grafico principal mistura muitos sinais visuais em pouco espaco.
   Barras de entrada/saida, forecast hachurado, linha de saldo e micro-labels
   competem. Falta seletor explicito de modo para despesas: barras vs linha, com
   paleta contrastante e legenda orientada a tarefa.

6. Falta harness web de regressao.
   O pacote web tem `typecheck` e `build`, mas nao ha testes unitarios para
   derivacoes (agrupamento, subtotais, modelo do grafico), nem teste Playwright
   para interacoes criticas.

## Estrategia de PRs

Cada item abaixo deve ser um PR separado. Dentro de cada PR, preferir um commit
convencional unico quando o escopo for coeso; quebrar em commits menores se
passar de 20-30 minutos ou se misturar Rust, schema e UI.

### PR 1: Harness de qualidade web e fixtures sinteticas

Escopo:
- Adicionar teste unitario web com Vitest para funcoes puras de derivacao.
- Adicionar fixtures sinteticas de transacoes/forecasts com volumes altos.
- Extrair derivacoes hoje embutidas em componentes para modulos testaveis:
  filtros, soma por mes, agrupamento por categoria, parsing categoria/subcategoria.
- Adicionar Playwright para smoke do `phai serve`/SPA quando rodando em dev.

Fora de escopo:
- Sem mudanca visual significativa.
- Sem dados reais.

Testes:
- `cd crates/phai-cli/web && pnpm typecheck && pnpm build && pnpm test`
- Teste unitario com pelo menos 12 meses, 20k transacoes sinteticas e overlays.
- Playwright smoke: carrega dashboard, seleciona mes, abre grupo, abre modal,
  verifica que nao ha erro no console.
- `cargo test -p phai-cli serve`

Paralelizavel:
- Um agente pode montar Vitest/fixtures.
- Outro pode extrair helpers puros de `MonthDetail`/`PlanningChart`.

### PR 2: Contrato de dados completo para um ano sem truncamento silencioso

Escopo:
- Trocar `GET /api/transactions` de limite fixo silencioso para contrato paginado
  ou streaming por mes.
- A UI deve carregar o ano completo e mostrar estado de sincronizacao por janela.
- LiveStore deve receber seed incremental, sem apagar tudo antes de terminar a
  janela inteira.
- Expor e tratar explicitamente qualquer `truncated`/incomplete state.

Fora de escopo:
- Virtualizacao visual da lista.

Testes:
- Rust: `load_transactions_window` retorna pagina consistente, ordenada e sem
  duplicar IDs em duas paginas.
- Rust: limite excedido nunca produz soma silenciosamente incompleta.
- Web: seed incremental preserva rows existentes ate a nova janela estar pronta.
- Web: fixture 20k rows carrega sem bloqueio longo no main thread.
- `cargo test -p phai-cli serve`
- `cd crates/phai-cli/web && pnpm test && pnpm build`

Paralelizavel:
- Um agente faz bridge Rust/API.
- Outro faz LiveStore materializers e consumo incremental.

### PR 3: Performance de lista nativa

Escopo:
- Virtualizar rows dentro de grupos ou usar `content-visibility`/janela manual,
  mantendo headers de categoria sempre baratos.
- Remover animacoes de altura em listas grandes; usar transicoes constantes.
- Memoizar componentes de row e mover handlers para callbacks estaveis.
- Debounce/transicao para busca textual e filtros.
- Preservar acessibilidade de teclado e foco.

Fora de escopo:
- Novas operacoes de recategorizacao.

Testes:
- Web unit: filtros e ordenacao iguais antes/depois.
- Playwright/perf: com fixture grande, alternar filtro e expandir grupo fica
  abaixo de orcamento definido no teste.
- Teste manual com profiler: scroll sem long tasks perceptiveis.
- `pnpm typecheck && pnpm build && pnpm test`

Paralelizavel:
- Nao recomendado dividir muito; toca a experiencia central de lista.

### PR 4: Agrupamento categoria/subcategoria com subtotais

Escopo:
- Definir parser canonico para `categoria:subcategoria[:...]`.
- Agrupar despesas por categoria pai, mostrando total da categoria.
- Dentro de cada categoria, agrupar por subcategoria e mostrar subtotal.
- Manter entradas agrupadas separadamente.
- Mostrar contagem por categoria e subcategoria.
- Garantir que overlays de recategorizacao atualizem os subtotais na mesma frame.

Fora de escopo:
- Drag/drop para recategorizar.

Testes:
- Unit: `alimentacao:mercado`, `alimentacao:restaurante`, `moradia` e
  `sem-categoria` geram hierarquia correta.
- Unit: subtotais batem com soma dos filhos e total geral.
- Unit: overlay muda transacao de subcategoria e atualiza ambos os grupos.
- Playwright: lista exibe total da categoria e subtotais das subcategorias.
- `cargo test --workspace` se houver mudanca Rust.

Paralelizavel:
- Um agente pode fazer modelo/testes.
- Outro pode fazer renderizacao, depois integrar.

### PR 5: Grafico principal legivel e modo despesas barras/linha

Escopo:
- Criar seletor segmentado: `Caixa`, `Despesas barras`, `Despesas linha`.
- No modo despesas, permitir alternar barras vs linha sem recarregar dados.
- Usar cores contrastantes e consistentes para despesas realizadas, forecast,
  selecionado e hover.
- Reduzir micro-labels permanentes; mover detalhes para tooltip/summary.
- Adicionar eixo/grade minima e legenda clara.
- Preservar navegacao por teclado entre meses.

Fora de escopo:
- Mudanca da semantica financeira de `build_chart_data`.

Testes:
- Unit: modelo de serie de despesas usa outflows + forecast outflows corretos.
- Unit: modo linha e modo barras produzem pontos/colunas para os mesmos meses.
- Playwright screenshot desktop/mobile para cada modo.
- Axe/ARIA basico: seletor e grafico acessiveis por teclado.
- `pnpm typecheck && pnpm build && pnpm test`

Paralelizavel:
- Um agente trabalha modelo de dados/testes.
- Outro trabalha componentes visuais.

### PR 6: Recategorizacao rapida por teclado e drag/drop

Escopo:
- Introduzir selecao ativa de transacao na lista.
- Atalhos: abrir categoria, aplicar categoria recente, mover selecao, salvar.
- Drag de transacao para header de categoria/subcategoria.
- Drop target para criar/mover para subcategoria existente.
- Batch opcional para selecao multipla se isso cair naturalmente no modelo.
- Escrita continua usando `reviewSubmitted` e `/api/events`.

Fora de escopo:
- Criar regras automaticas persistentes.

Testes:
- Unit: comando de recategorizacao gera patch minimo correto.
- Unit: overlay otimista altera grupo/subtotal imediatamente.
- Rust: batch review continua isolando falhas e emitindo auditoria.
- Playwright: recategorizar por teclado e por drag/drop, verificar pending chip
  e grupo final.
- `cargo test -p phai-cli serve`
- `pnpm test && pnpm build`

Paralelizavel:
- Um agente faz atalhos/selecao.
- Outro faz DnD de transacoes e drop targets.

### PR 7: Forecast move por teclado, regras de mes aberto/futuro

Escopo:
- Definir regra central: forecast manual pode ir para mes corrente aberto ou
  meses futuros; parcelas/assinaturas continuam bloqueadas.
- Adicionar atalhos para mover forecast selecionado para mes anterior/proximo
  permitido e para escolher mes.
- UI deve explicar bloqueio por estado/tooltip acessivel, nao so cursor.
- Bridge Rust deve validar a mesma regra que a UI.
- Corrigir qualquer divergencia de payload/serde entre `forecastCreated`,
  `api.createForecast` e resposta Rust.

Fora de escopo:
- Recorrencia/template nova.

Testes:
- Rust: mover para mes passado fechado falha; mes corrente/futuro passa.
- Rust: installment/subscription falha.
- Web unit: atalho gera `forecastMoved` com data preservando dia quando valido.
- Playwright: mover por teclado e por drag/drop.
- `cargo test -p phai-cli serve`
- `pnpm test && pnpm build`

Paralelizavel:
- Um agente faz validacao Rust.
- Outro faz UI/atalhos.

### PR 8: Consistencia financeira web vs fonte da verdade

Escopo:
- Criar comando/script local de auditoria que compara:
  - totais de OFX privado parseado localmente;
  - transacoes efetivas do store para os mesmos meses;
  - resposta `/api/transactions`;
  - somas exibidas/derivadas pela UI.
- Nao gravar dados reais no repo; o script recebe caminhos via argumento.
- Corrigir inconsistencias genericas encontradas em importacao, dedupe,
  categoria interna ou janelas de data.
- Documentar no plano/runbook como executar a auditoria local.

Fora de escopo:
- Committing dos OFX ou snapshots reais.

Testes:
- Fixtures OFX sinteticas com casos: debito, credito, estorno, parcela,
  duplicidade, transacao no limite do mes.
- Auditoria sintetica falha com discrepancia de centavos, IDs faltantes ou
  transacoes extras.
- Auditoria real local: os meses dos OFX privados batem 100% ou geram lista
  precisa de divergencias para correcao generica/privada.
- `cargo test --workspace`

Paralelizavel:
- Um agente cria auditoria e fixtures sinteticas.
- Outro investiga/corrige discrepancias depois que o relatorio local existir.

### PR 9: Release readiness e merge

Escopo:
- Rodar suite completa:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --workspace`
  - `cargo audit`
  - `cargo deny check licenses`
  - `sentrux check .`
  - `sentrux gate .`
  - `cd crates/phai-cli/web && pnpm typecheck && pnpm build && pnpm test`
- Abrir PRs pequenos; mergear em `main` quando CI estiver verde.
- Aguardar PR do Release Please, revisar changelog/versionamento e mergear se CI
  estiver verde.

Testes:
- CI verde no PR de cada etapa.
- Release Please CI verde antes do merge final.

Paralelizavel:
- Nao paralelizar merge final.

## Ordem recomendada

1. PR 1 antes de qualquer refactor grande.
2. PR 2 antes de performance visual, para eliminar truncamento silencioso.
3. PR 3 e PR 4 podem correr em paralelo apos PR 1, mas PR 4 integra melhor
   depois da extracao de helpers.
4. PR 5 pode correr em paralelo com PR 4.
5. PR 6 depende de PR 4.
6. PR 7 pode correr em paralelo com PR 6.
7. PR 8 deve rodar depois de PR 2, PR 4, PR 5, PR 6 e PR 7.
8. PR 9 fecha o trabalho.

## Progresso

- [x] Branch criada.
- [x] Baseline Sentrux salvo.
- [x] Revisao inicial de ADRs e web app feita.
- [x] Plano inicial escrito.
- [x] PR 1 implementado por agente anterior e revisado.
- [x] PR 2 implementado por agente anterior e revisado.
- [x] PR 3 implementado por agente anterior e revisado.
- [x] PR 4 implementado por agente anterior e revisado.
- [x] PR 5 implementado por agente anterior e revisado.
- [x] PR 6 implementado por agente anterior e corrigido na revisao.
- [x] PR 7 implementado por agente anterior e corrigido na revisao.
- [ ] PR 8 implementado e auditoria real local executada.
- [ ] PR 9 merge/release.

## Revisao de integracao em 2026-05-31

Correcoes aplicadas antes do merge:

- Drop de transacao em categoria agora grava `null` para sem categoria, nao o
  sentinel visual `—`.
- Drop em `Entradas` deixou de criar categoria invalida.
- Drop em subcategoria plana deixou de gerar `categoria:—`.
- Drop targets agora desregistram no unmount para evitar targets stale.
- Atalho de categoria usa a transacao focada quando nao ha selecao explicita.
- Selecao com Shift+seta agora ancora corretamente mesmo sem clique previo.
- Popover de mover forecast por `Ctrl+M` foi movido para fora do formulario de
  nova previsao; agora aparece quando uma previsao existente esta selecionada.
- Mover forecast no cliente rejeita meses passados e ajusta o dia ao ultimo dia
  valido do mes alvo.
- Vite agora divide vendors em chunks menores e resolve corretamente
  `@livestore/wa-sqlite` no dev server.
- Helper de teste de enrichment agora limpa tambem `ANTHROPIC_AUTH_TOKEN`, que
  e aceito pelo seletor de provider.

Validado:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo audit` (somente avisos permitidos existentes)
- `cargo deny check licenses`
- `sentrux gate .`
- `sentrux check .`
- `cd crates/phai-cli/web && pnpm typecheck`
- `cd crates/phai-cli/web && pnpm test -- --run`
- `cd crates/phai-cli/web && pnpm build`
- `cd crates/phai-cli/web && pnpm test:e2e` contra Vite dev server
