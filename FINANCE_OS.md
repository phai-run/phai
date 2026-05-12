# FINANCE_OS

Repository-level guidance for any agent (OpenClaw, Claude, ChatGPT, Codex, etc.) interacting with Finance OS outputs.

## Core Rules

- Use `finance-cli` as the single source of truth for operational outputs.
- Prefer standard CLI reports and summaries over custom, agent-generated report formats.
- For sync notifications, prefer:
  - `finance sync pluggy --notify-summary` for human-readable text
  - `finance sync pluggy --json-summary` for structured automation
- Only create ad-hoc report formatting when the user explicitly asks for a custom view.

## Classification and Naming

- Never invent categories.
- Category assignment must come from Finance OS rules and effective overrides in the runtime database/view layer.
- User-facing transaction naming must prioritize effective context:
  - `display_label = context when present; otherwise original description`
- Transaction names should appear with an emoji prefix from Finance OS display rules.

## Interaction Consistency

When answering users about recent transactions or daily finance activity:

1. Use CLI output as primary evidence.
2. Preserve Finance OS display labels and category display strings.
3. Do not replace Finance OS labels with raw institution text unless asked.
4. If data is ambiguous, ask for transaction ID and then persist context/category through CLI commands.

For card-bill requests, disambiguate intent before answering:

- If user asks about "fatura em aberto", "em andamento", or "fatura atual", prioritize open balance (`open_amount`) for the requested month.
- If user asks "como fecharam os cartões", "fatura fechada", or "última fatura", prioritize closed bills and default to the last fully closed month when month is omitted.
- If user writes "esse mês" together with "fecharam/fatura fechada", answer with the inferred closed month and state that month explicitly in `YYYY-MM`.
- In closed-bill answers, report `total fechado = total_charges - open_amount` first; mention `open_amount` only as secondary context.
- If user asks a custom closed-bill view (categories, recorrentes, assinaturas, parcelados), provide the custom view over CLI-backed data and clearly label any unavailable slice.

## Privacy and Portability

- Do not hardcode personal counterparties or user-specific heuristics in shared code.
- Keep personal mappings in runtime rules/private configuration.
- Shared repository changes must stay generic and reusable.
