# Reporting UX

Repository-level guidance for any agent (OpenClaw, Claude, ChatGPT, Codex, etc.) interacting with phai outputs.

## Core Rules

- Use `phai` as the single source of truth for operational outputs.
- Prefer standard CLI reports and summaries over custom, agent-generated report formats.
- For sync notifications, prefer:
  - `phai sync pluggy --notify-summary` for human-readable text
  - `phai sync pluggy --json-summary` for structured automation
- Only create ad-hoc report formatting when the user explicitly asks for a custom view.

## Classification and Naming

- Never invent categories.
- Category assignment must come from phai rules and effective overrides in the runtime database/view layer.
- User-facing transaction naming must prioritize the human anatomy fields:
  - `display_label = description when present; otherwise merchant_name; otherwise raw_description`
- `classifier_trace` is technical/debug output and must not appear in normal family-facing reports.
- Transaction names should appear with an emoji prefix from phai display rules.

## Interaction Consistency

When answering users about recent transactions or daily finance activity:

1. Use CLI output as primary evidence.
2. Preserve phai display labels and category display strings.
3. Do not replace phai labels with raw institution text unless asked.
4. If data is ambiguous, ask for transaction ID and then persist description/merchant/purpose/category through CLI commands.

For card-bill requests, disambiguate intent before answering:

- `v_card_summary.month_ref` is the **billing cycle** the bill closes in (driven by `accounts.metadata_json.billing_closing_day`), **not** the calendar month a transaction was posted. A purchase on Mar 28 with closing-day 3 lives in the cycle that closes Apr 3 (`month_ref = 2026-04`).
- If user asks about "fatura em aberto", "em andamento", or "fatura atual", prioritize the most-recent open balance per card from `v_card_open_now` (or `cards_open_now()` on the trait). That view returns at most one row per card — the cycle that is closed and still has open balance.
- If user asks "como fecharam os cartões", "fatura fechada", or "última fatura", prioritize closed bills and default to the last fully closed cycle when month is omitted.
- If user writes "esse mês" together with "fecharam/fatura fechada", answer with the inferred closed cycle and state that month explicitly in `YYYY-MM`.
- In closed-bill answers, report `total fechado = total_charges - open_amount` first; mention `open_amount` only as secondary context.
- If user asks a custom closed-bill view (categories, recorrentes, assinaturas, parcelados), provide the custom view over CLI-backed data and clearly label any unavailable slice.

## Message Delivery (WhatsApp / mobile)

All user-facing messages are delivered via WhatsApp or mobile apps that do not render markdown.

- **No markdown tables.** Use `Label: *value*` lines instead of columnar data.
- **No code blocks** for non-code data. No headers (`#`).
- Allowed formatting: `*bold*`, `_italic_`, `•` bullet lists, numbered lists, line breaks.
- For sync notifications: pass through `--notify-summary` output verbatim. Do not reformat it.
- For ad-hoc reports: keep under 15 lines. Offer detail expansion on request.
- Structured data → one line per field: `emoji Label: *value*`.

## Privacy and Portability

- Do not hardcode personal counterparties or user-specific heuristics in shared code.
- Keep personal mappings in runtime rules/private configuration.
- Shared repository changes must stay generic and reusable.
