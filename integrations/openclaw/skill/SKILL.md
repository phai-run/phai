---
name: finance-os
description: >
  Runtime financeiro da OpenClaw baseado em Finance OS + BigQuery. Sincroniza Pluggy,
  grava contexto/categoria direto no banco canônico e gera relatórios determinísticos
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

Fluxo de caixa:

```bash
bash skills/finance-os/finance.sh report cashflow --months 3
```

Forecast vs realizado:

```bash
bash skills/finance-os/finance.sh report forecast-vs-actual --month YYYY-MM
```

Resumo de cartões:

```bash
bash skills/finance-os/finance.sh report card-summary --month YYYY-MM
```

Pendências sem categoria/contexto:

```bash
bash skills/finance-os/finance.sh report uncategorized --limit 20
```

Persistir contexto manual:

```bash
bash skills/finance-os/finance.sh tx set-context --transaction-id ID --context "texto"
```

Persistir categoria manual:

```bash
bash skills/finance-os/finance.sh tx categorize --transaction-id ID --category Categoria --subcategory "Subcategoria opcional" --context "texto opcional"
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

- Sempre preferir o `transaction_id` explícito quando o usuário responder contexto.
- No fluxo Ford de split, seguir a ordem:
  1. `report split-candidates` para shortlist inicial
  2. `tx split preview` para validar estrutura e totais
  3. confirmação explícita do usuário antes de persistir
  4. `tx split apply` para gravar
  5. `tx split show` para auditoria pós-gravação
  6. `report item-prices` quando o usuário pedir comparação de preço unitário
- Se backend não for BigQuery (ou runtime sem suporte de split), responder limitação e não inventar saída.
- Se a resposta estiver ambígua entre múltiplas pendências, pedir qual ID deve ser atualizado.
- Nunca inventar categoria. Se a categoria não estiver clara, gravar só o contexto.
- Nunca codificar heurística pessoal de item/produto em código compartilhado; usar somente dados/rules privados no runtime.
- Ao relatar sync horário:
  - se `newTransactionsCount = 0` e `needsContextCount = 0`, ficar em silêncio
  - se `summaryStatus != "complete"`, tratar como resumo parcial: confiar apenas em `newTransactions*`, expor `warnings` e não inferir pendências a partir de `needsContextCount = -1`
  - se houver novidade, usar apenas o JSON do `--json-summary` como base factual
  - para repasse 1:1 em texto (sem remontar mensagem na Ford), usar `--notify-summary`
- Para interações com usuário:
  - sempre priorizar labels efetivos da CLI (contexto/categoria já aplicados no Finance OS)
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
