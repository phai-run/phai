---
name: finance-os
description: >
  Runtime financeiro da OpenClaw baseado em Finance OS + BigQuery. Sincroniza Pluggy,
  grava descrição/estabelecimento/propósito/categoria direto no banco canônico e gera relatórios determinísticos
  via CLI única.
metadata:
  {
    "openclaw":
      {
        "emoji": "💸"
      }
  }
---

# Finance OS

Use esta skill para qualquer operação financeira da OpenClaw.

## Princípios

- Fonte de verdade dos dados: BigQuery (dataset configurado no runtime)
- Fonte de verdade do processamento: binário `finance-cli`
- Não usar `skills/finance-analyzer/*`
- Não rodar scripts Python de planilha; os dashboards leem BigQuery via Connected Sheets
- Não inventar formato de report quando a CLI já fornece saída padrão suficiente
- Priorizar os padrões de UX e classificação definidos em `FINANCE_OS.md`
- Fluxo de `tx split` é BigQuery-only no runtime Ford; backend `local` deve ser tratado como não suportado

## Wrapper oficial

Todos os comandos devem passar por:

```bash
bash skills/finance-os/finance.sh ...
```

## Comandos principais

Sincronizar Pluggy com resumo estruturado:

```bash
bash skills/finance-os/finance.sh sync pluggy --json-summary
```

Sincronizar Pluggy com mensagem pronta para notificação (UX centralizada no Finance OS):

```bash
bash skills/finance-os/finance.sh sync pluggy --notify-summary
```

Pulso diário:

```bash
bash skills/finance-os/finance.sh report daily-pulse --days 7
```

Resumo de gastos do mês:

```bash
bash skills/finance-os/finance.sh report monthly-spend --month YYYY-MM
```

Fluxo de caixa (competência de caixa, somente contas correntes — saldo inicial, entradas, saídas, saldo final do mês):

```bash
bash skills/finance-os/finance.sh report cashflow --month YYYY-MM
```

Gráfico de evolução de caixa (SVG; opcionalmente sobreposto com forecast):

```bash
# Default: últimos 6 meses, SVG em ./finance-cashflow.svg
bash skills/finance-os/finance.sh report cashflow-chart

# Janela e arquivo customizados, com sparkline ASCII no terminal:
bash skills/finance-os/finance.sh report cashflow-chart \
  --months 12 \
  --output ~/Downloads/cashflow.svg \
  --text

# Com overlay de forecast (linhas tracejadas de entradas/saídas previstas):
bash skills/finance-os/finance.sh report cashflow-chart --forecast
```

Auto-gerar forecasts de parcelamentos ativos (idempotente — pode ser rodado depois de cada sync):

```bash
bash skills/finance-os/finance.sh forecast refresh-installments
bash skills/finance-os/finance.sh forecast refresh-installments --raw   # JSON
```

Detectar e gerenciar templates recorrentes (subscriptions, contas fixas e envelopes por categoria):

```bash
# Detecta candidatos no histórico:
#  - subscriptions (≥3 meses, mensal, variação ≤10%)
#  - fixed bills (≤30%, com banda ±σ)
#  - envelopes por categoria (≥4 meses, variação ≤40%, exclui txs já
#    cobertas por subscriptions/fixed ativos para não duplicar)
# Persistidos como status='proposto'. Idempotente — re-rodar não
# re-sugere candidatos já em proposto/ativo/descartado.
bash skills/finance-os/finance.sh forecast suggest --raw

# Aceita um candidato → vira 'ativo' e materializa 6 forecasts futuros.
bash skills/finance-os/finance.sh forecast accept --template-id <id>

# Descarta um candidato → vira 'descartado' (não volta a ser sugerido).
bash skills/finance-os/finance.sh forecast dismiss --template-id <id>
```

Simulação what-if (read-only — não grava nada):

```bash
# "Posso comprometer R$ 250/mês com algo novo a partir de ago/2026?"
#  Mostra saldo projetado baseline vs com cenário, delta total no horizonte,
#  e (se passar --minimum-balance) o primeiro mês em que o saldo cairia
#  abaixo do limite.
bash skills/finance-os/finance.sh forecast scenario \
  --amount=-250 \
  --description "atividade extra" \
  --start 2026-08 \
  --months 12 \
  --minimum-balance 3000 \
  --raw
```

Trigger conversacional: quando o usuário perguntar coisas como "posso afford X?",
"e se eu colocar Y?", "quando acabam meus parcelamentos?", "quanto sobra por mês?",
o agente deve:

1. Para "posso afford?": rodar `forecast scenario` com o valor e período mencionados
   e responder com base no `delta_total` e `first_breach_month` retornados.
2. Para "quando acabam parcelamentos?": rodar `report installments` (já existente)
   e listar `projected_end` por cadeia. Alternativamente, listar templates ativos
   com `kind='installment'` e `end_date`.
3. Para visão geral: rodar `report cashflow-chart --forecast --text` e usar o
   sparkline + Hoje/Projetado pra contextualizar.

Forecast vs realizado:

```bash
bash skills/finance-os/finance.sh report forecast-vs-actual --month YYYY-MM
```

Resumo de cartões:

```bash
bash skills/finance-os/finance.sh report card-summary --month YYYY-MM
```

Pendências sem categoria:

```bash
bash skills/finance-os/finance.sh report uncategorized --limit 20
```

Pendências de campos humanos:

```bash
bash skills/finance-os/finance.sh tx pending-human --kind description --limit 20
bash skills/finance-os/finance.sh tx pending-human --kind merchant --limit 20
bash skills/finance-os/finance.sh tx pending-human --kind purpose --min-abs-amount 30 --limit 20
```

Persistir descrição, estabelecimento ou propósito:

```bash
bash skills/finance-os/finance.sh tx set-anatomy --transaction-id ID --description "texto curto"
bash skills/finance-os/finance.sh tx set-anatomy --transaction-id ID --merchant-name "Nome limpo"
bash skills/finance-os/finance.sh tx set-anatomy --transaction-id ID --purpose "finalidade humana"
```

Persistir categoria manual:

```bash
bash skills/finance-os/finance.sh tx categorize --transaction-id ID --category Categoria --subcategory "Subcategoria opcional" --context "texto opcional"
```

Revisão humana interativa/local:

```bash
bash skills/finance-os/finance.sh review
bash skills/finance-os/finance.sh tx review-human --kind all --limit 20 --tui --sound
```

Fluxo recomendado via WhatsApp/OpenClaw:

```bash
# Sempre passar --owner <nome> para escopar pela pessoa que está conversando
# (Ford = "felipe", OpenClaw da Aline = "aline"). Isso filtra a fila para
# as contas daquele owner — ver accounts.owner no BigQuery.
bash skills/finance-os/finance.sh tx review-human --summary --owner felipe --json
bash skills/finance-os/finance.sh tx review-human --kind all --limit 5 --owner felipe --json
bash skills/finance-os/finance.sh tx review-human --transaction-id ID \
  --description "texto curto" \
  --merchant-name "Nome limpo" \
  --purpose "finalidade opcional" \
  --category categoria:subcategoria \
  --bulk identical \
  --json
```

Pré-visualizar split de transação (BigQuery-only):

```bash
bash skills/finance-os/finance.sh tx split preview --transaction-id ID --payload split.json
```

Aplicar split de transação (BigQuery-only):

```bash
bash skills/finance-os/finance.sh tx split apply --transaction-id ID --payload split.json
```

Consultar split aplicado (BigQuery-only):

```bash
bash skills/finance-os/finance.sh tx split show --transaction-id ID
```

Limpar split aplicado (BigQuery-only):

```bash
bash skills/finance-os/finance.sh tx split clear --transaction-id ID
```

Listar candidatos a split (BigQuery-only):

```bash
bash skills/finance-os/finance.sh report split-candidates
```

Listar preços por item (BigQuery-only):

```bash
bash skills/finance-os/finance.sh report item-prices --query "item"
```

## Regras operacionais

- Sempre preferir o `transaction_id` explícito quando o usuário responder descrição, estabelecimento ou propósito.
- Trigger conversacional: quando o usuário disser algo como "Ford, como estamos com as categorizações?", "tem coisa pra categorizar?", "vamos categorizar?", ou "quero brincar de categorizar", executar `tx review-human --summary --json`. Responder com as contagens de sem categoria, sem descrição, sem estabelecimento e sem propósito; se houver pendências, perguntar se ele quer revisar algumas agora.
- Se o usuário aceitar revisar, executar `tx review-human --kind all --limit 5 --json`, mostrar uma transação por vez em formato curto, e aguardar resposta.
- **Trigger proativo no sync horário (anatomia das transações novas).** Após anunciar transações recém-chegadas, NÃO ficar em silêncio nem esperar o usuário pedir. Para cada transação nova retornada pelo `--json-summary`/`--notify-summary`, perguntar imediatamente: "**descrição humana**, **estabelecimento** (se diferente do raw) e **propósito** (se aplicável) da transação X?" — uma transação por vez. Quando o usuário responder em texto livre, mapear o conteúdo em `--description`, `--merchant-name`, `--purpose` e/ou `--category` e gravar via `tx review-human --transaction-id ID --... --json`. Use a anatomia para enriquecer o histórico — isso alimenta a replicação automática para próximas transações do mesmo estabelecimento. Se o usuário disser "pula", "depois" ou ignorar, registrar o estado e seguir adiante; voltar a perguntar somente se ele iniciar a conversa de novo.
- **Escopo por owner (multi-usuário).** Quando a skill rodar para uma pessoa específica (Ford = Felipe, OpenClaw da Aline = Aline), sempre passar `--owner <nome>` em todas as queries de revisão e pendências: `tx review-human --summary --owner aline --json`, `tx review-human --kind all --limit 5 --owner aline --json`, etc. Isso garante que cada assistente só veja transações da sua pessoa, mesmo num dataset BigQuery compartilhado. Combine com `--account-id` quando o usuário pedir uma conta específica.
- Para revisão pelo WhatsApp, usar `tx review-human --kind all --limit N --json` para obter a fila e `tx review-human --transaction-id ... --json` para salvar em uma única chamada. O modo sem `--transaction-id` e sem `--json` é interativo e deve ser usado apenas em terminal local.
- Ao receber resposta natural do usuário para uma pendência, preencher somente os campos informados ou inferíveis com segurança. Exemplos: "é Mercado Exemplo, compra de mercado" salva `merchant_name = Mercado Exemplo` e `description = Compra de mercado`; "muda pra educação material escolar" salva `category = educacao:material-escolar`; "pula" não salva nada e passa para a próxima.
- Quando houver áudio, transcrever primeiro, estruturar em `description`, `merchant_name`, `purpose` e `category`, apresentar um resumo com botões/ações `[Correto] [Ajustar]`, e só persistir depois do `Correto`. Se o usuário escolher `Ajustar`, aceitar texto ou áudio adicional, recompor a proposta e confirmar de novo.
- Se a resposta do usuário sugerir que a mesma edição vale para transações idênticas ("todas essas", "aplica nas iguais", "é sempre isso"), chamar `tx review-human --transaction-id ... --bulk identical --json`. Caso contrário, salvar apenas a transação atual.
- No fluxo Ford de split, seguir a ordem:
  1. `report split-candidates` para shortlist inicial
  2. `tx split preview` para validar estrutura e totais
  3. confirmação explícita do usuário antes de persistir
  4. `tx split apply` para gravar
  5. `tx split show` para auditoria pós-gravação
  6. `report item-prices` quando o usuário pedir comparação de preço unitário
- Se backend não for BigQuery (ou runtime sem suporte de split), responder limitação e não inventar saída.
- Se a resposta estiver ambígua entre múltiplas pendências, pedir qual ID deve ser atualizado.
- Nunca inventar categoria. Se a categoria não estiver clara, gravar só os campos humanos que o usuário informou.
- Nunca codificar heurística pessoal de item/produto em código compartilhado; usar somente dados/rules privados no runtime.
- Ao relatar sync horário:
  - se `newTransactionsCount = 0` e `needsContextCount = 0`, ficar em silêncio
  - se `summaryStatus != "complete"`, tratar como resumo parcial: confiar apenas em `newTransactions*`, expor `warnings` e não inferir pendências a partir de `needsContextCount = -1`
  - se houver novidade, usar apenas o JSON do `--json-summary` como base factual
  - para repasse 1:1 em texto (sem remontar mensagem na Ford), usar `--notify-summary`
  - **depois de listar as transações novas, perguntar a anatomia (descrição/estabelecimento/propósito) de cada uma, uma por vez** — não esperar o usuário iniciar (ver "Trigger proativo no sync horário" acima). Use `tx review-human --transaction-id ID --description … --merchant-name … --purpose … --owner <pessoa> --json` para gravar a resposta.
- Para interações com usuário:
  - sempre priorizar labels efetivos da CLI (description/merchant_name/raw_description e categoria já aplicados no Finance OS)
  - não remontar manualmente listagens de transação, exceto se o usuário pedir uma visão específica
  - em caso de solicitação de visão customizada, deixar explícito que é um formato adicional sobre dados da CLI
- Para perguntas sobre cartão/fatura, desambiguar antes de responder:
  - "em aberto", "fatura atual", "em andamento" => foco em saldo aberto (`open_amount`)
  - "fecharam", "fatura fechada", "última fatura" => foco em fatura fechada; usar último mês fechado por padrão quando não houver mês explícito
  - se o texto incluir "esse mês" + "fechada/fecharam", converter para mês fechado inferido e explicitar o `YYYY-MM` na resposta
- Quando o usuário pedir "como fecharam os cartões":
  - executar `bash skills/finance-os/finance.sh report card-summary --month YYYY-MM`
  - calcular e destacar `total fechado = total_charges - open_amount` por cartão
  - trazer `em aberto` apenas como contexto secundário
- Quando o usuário pedir visão customizada de fatura fechada (categorias, recorrentes, assinaturas, parcelados):
  - combinar relatórios CLI para montar a visão adicional
  - se algum bloco não estiver disponível no runtime atual, informar explicitamente a limitação em vez de inferir
- Ao citar versão do runtime, usar:

```bash
bash skills/finance-os/finance.sh --version
```
