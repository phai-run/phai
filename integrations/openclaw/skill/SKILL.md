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

## Regras operacionais

- Sempre preferir o `transaction_id` explícito quando o usuário responder contexto.
- Se a resposta estiver ambígua entre múltiplas pendências, pedir qual ID deve ser atualizado.
- Nunca inventar categoria. Se a categoria não estiver clara, gravar só o contexto.
- Ao relatar sync horário:
  - se `newTransactionsCount = 0` e `needsContextCount = 0`, ficar em silêncio
  - se houver novidade, usar apenas o JSON do `--json-summary` como base factual
- Ao citar versão do runtime, usar:

```bash
bash skills/finance-os/finance.sh --version
```
