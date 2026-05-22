# finance-cli

Command-line interface for **Finance OS** — the personal finance pipeline
that keeps your Pluggy / OFX / manual transactions categorized, audited,
and ready to query.

## Enrichment

Automatic categorization for transactions that the rules engine doesn't
match. The pipeline combines four signals — CNPJ lookup (BrasilAPI),
Pluggy's coarse category, temporal context, and an LLM call — to either
auto-apply a category, suggest one to the user, or ask for clarification.

See `crates/finance-core/src/enrichment/` for the full architecture and
`/.claude/plans/inherited-roaming-milner.md` for the plan that landed it.

### Manual: `finance tx enrich`

```sh
finance tx enrich [--days 30] [--limit 20] [--dry-run] [--auto]
                  [--retry] [--no-rule] [--retroactive-threshold 80]
                  [--provider <name>] [--model <id>]
                  [--machine] [--machine-timeout 60]
                  [--transaction-id <id>]
```

Notable flags:

- `--days N` — look back N days for uncategorized transactions (default
  `30`).
- `--limit N` — max transactions to process this run (default `20`).
- `--dry-run` — show the analysis without writing anything.
- `--auto` — apply every `confidence >= AUTO_THRESHOLD` decision without
  prompting. Decisions below the threshold are skipped (still rendered).
- `--retry` — re-process transactions that were already attempted
  (`enrichment_attempted_at IS NOT NULL`).
- `--no-rule` — skip automatic rule creation + retroactive application
  after a successful categorization.
- `--machine` — emit NDJSON decisions to stdout and read JSON actions
  from stdin (one round-trip per transaction). Designed for agent
  integrations like OpenClaw.
- `--transaction-id <id>` — process exactly one transaction by id,
  bypassing the `--days` / `--limit` filter.

### Automatic: hook on `finance sync`

After a successful `finance sync pluggy`, the CLI automatically runs
enrichment on the newly upserted transactions. The hook is **non-fatal**:
if the LLM is unreachable, BrasilAPI is throttled, or anything else
fails, sync still returns success — the failure is logged and the
affected transactions are re-tried on the next `finance tx enrich`.

```sh
# Default — enrichment runs automatically:
finance sync pluggy

# Disable the hook (useful in CI / cron / batch jobs):
finance sync pluggy --no-enrich
```

Sample output:

```
Sync Pluggy concluído:
- accounts: 4
- transactions: 142
- categories: 18
- actor: felipe
- backend: Local
Sincronização concluída: 42 transações novas
Enrichment automático: 18 categorizadas | 22 adiadas para revisão | 2 falhas
Para revisar as adiadas: finance tx enrich --days 7
```

For quick human cleanup of transaction anatomy, use the interactive
review loop locally. `finance review` opens a dense terminal UI with
editable human fields, read-only raw bank data, category autocomplete,
filters, a searchable details modal, and Ctrl+S to save:

```bash
finance review
```

The lower-level command is still available when you need explicit queue
selection. The shortcut loads a long local TUI queue by default; keep small
limits for OpenClaw/WhatsApp JSON flows.

```bash
finance tx review-human --kind all --limit 500 --tui --sound
```

Filter the queue before opening the TUI or emitting JSON:

```bash
finance review --month 2026-03 --account-id shared_credit --category gas-stations --merchant posto
finance tx review-human --kind all --json --month 2026-03 --filter-category gas-stations
```

Inside the TUI, press `Ctrl+F` to open the filter menu, then choose `m`, `a`,
`c`, or `e` to filter by the current transaction's month, account, category, or
merchant. Press `0` in that menu to clear TUI filters.

For OpenClaw/WhatsApp, list machine-readable pending items and then
apply one response by transaction id:

```bash
finance tx review-human --summary --json
finance tx review-human --kind all --limit 5 --json
finance tx review-human --transaction-id TX_ID \
  --description "Compra de mercado" \
  --merchant-name "Mercado Exemplo" \
  --category alimentacao:mercado \
  --json
```

Behavior summary:

- `AutoApply` decisions (high confidence) are persisted as
  `category_source = "enriched:llm"`, audited, and a DSL rule is
  generated when the keyword is suitable.
- `Suggest` / `AskUser` decisions are **deferred**: the transaction is
  marked `enrichment_attempted_at = now` (so it's not retried by
  default), but its category is untouched. The user can pick it up
  later with `finance tx enrich --days 7`.
- Any error inside the hook increments the `failures` counter and the
  loop continues with the next transaction.

When the sync runs in a non-interactive context (no TTY on stdin —
typical for cron, CI, piped output), the hook automatically operates in
"auto-only" mode: only `AutoApply` decisions are persisted and nothing
is printed beyond the summary line.

### Environment variables

API keys are **never** stored in `config.toml` — read from env only.

| Variable | Purpose |
|---|---|
| `FINANCE_LLM_PROVIDER` | Provider override: `anthropic`, `openai`, `deepseek`, `ollama`. Falls back to the first env var present below. |
| `FINANCE_LLM_MODEL` | Model id override (defaults per provider). |
| `ANTHROPIC_API_KEY` | Required if the Anthropic provider is selected. |
| `OPENAI_API_KEY` | Required if the OpenAI provider is selected. |
| `DEEPSEEK_API_KEY` | Required if the Deepseek provider is selected. |
| `OLLAMA_BASE_URL` | Base URL for a local Ollama daemon (default `http://localhost:11434/v1`). |

If none of the above are set when the hook runs, the failure is logged
("aviso: enrichment indisponível…") and sync exits cleanly — the user
can configure a provider later and re-run `finance tx enrich` by hand.
