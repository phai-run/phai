# Daily pulse via cron + WhatsApp

This document shows how to wire `finance-cli` to push the daily pulse to a
WhatsApp number on a schedule.

> See also: [ADR-0009](adr/0009-proactive-pulse-and-closing-plan.md) for the
> design rationale behind the new pulse format.

## What the message looks like

```
💸 *Pulso · seg 18/mai*

*Mês até dia 18 · 13 dias restantes* 🔻
  entradas R$ 17.335,19 · saídas R$ 30.856,79 · saldo -R$ 13.521,60

⚠️ *Fecha negativo* no ritmo T3M (proj. -R$ 6.717,06)
   Para fechar zerado: até *R$ 4.176,06* por semana (2 sem) em variáveis

*Frear neste mês*
  📚 Educação · Logosofia · gasto MtD R$ 4.467,50 (proj. +253% vs média)
  🍽️ Alimentação · Restaurantes · gasto MtD R$ 3.155,40 (proj. +148% vs média)
  🩺 Saúde · Terapia · gasto MtD R$ 3.278,21 (proj. +25% vs média)

*A vencer* (R$ 6.812,77 no total até fim do mês)
  • 20/mai · Aluguel · R$ 4.253,00
  • 22/mai · Wellhub · R$ 89,90
  • 24/mai · Netflix · R$ 59,90

*Cartões em aberto*
  💳 Felipe Nubank Cartão · R$ 5.643,86 (vence 27/mai)
  💳 Aline Nubank Cartão · R$ 4.463,36 (vence 10/jun)

*Ação*
  • 4 lançamentos sem categoria
  • ⚠️ 82% do orçamento de Alimentação · Restaurantes
```

The five blocks are designed for a single phone-screen glance:
1. Month-to-date headline with delta vs T3M.
2. Closing plan — on-track / tight / stretched, with the maximum weekly
   variable budget when tight.
3. *Frear* — up to three categories projected to overshoot baseline.
4. *A vencer* — forecasts due in the rest of the month.
5. *Cartões* + *Ação* — open card balances and outstanding hygiene items.

## Direct webhook (recommended)

`finance-cli notify whatsapp` posts the rendered body to a webhook of your
choice. Configure two env vars:

```bash
export FINANCE_OS_WHATSAPP_WEBHOOK_URL="https://your-gateway.example.com/messages"
export FINANCE_OS_WHATSAPP_WEBHOOK_TOKEN="optional-bearer-token"
```

The payload is JSON: `{ "text": "<rendered body>" }`. If a token is set, it's
sent as `Authorization: Bearer <token>`.

```bash
# Preview without sending
finance-cli notify whatsapp --dry-run

# Send and log to stdout
finance-cli notify whatsapp --echo
```

### Crontab example

Send the pulse every day at 21:00 local time:

```cron
# Daily pulse to WhatsApp at 21:00
0 21 * * * /usr/bin/env -i \
  PATH=/usr/local/bin:/usr/bin \
  FINANCE_OS_WHATSAPP_WEBHOOK_URL="https://your-gateway.example.com/messages" \
  FINANCE_OS_WHATSAPP_WEBHOOK_TOKEN="..." \
  HOME="$HOME" \
  /Users/you/.local/bin/finance-cli notify whatsapp >> ~/.finance-os/pulse.log 2>&1
```

The `env -i` keeps the cron environment minimal so secrets aren't leaked from
the user's shell rc files. `HOME` is required so the CLI finds its config
under `~/Library/Application Support/finance-os` (macOS) or
`~/.config/finance-os` (Linux).

### macOS launchd

For macOS, `launchd` is more reliable than cron under sleep/wake. Drop this in
`~/Library/LaunchAgents/io.finance-os.pulse.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>io.finance-os.pulse</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/you/.local/bin/finance-cli</string>
    <string>notify</string>
    <string>whatsapp</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>FINANCE_OS_WHATSAPP_WEBHOOK_URL</key>
    <string>https://your-gateway.example.com/messages</string>
    <key>FINANCE_OS_WHATSAPP_WEBHOOK_TOKEN</key>
    <string>...</string>
    <key>HOME</key><string>/Users/you</string>
  </dict>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>21</integer><key>Minute</key><integer>0</integer></dict>
  <key>StandardOutPath</key><string>/tmp/finance-os-pulse.log</string>
  <key>StandardErrorPath</key><string>/tmp/finance-os-pulse.log</string>
</dict></plist>
```

Then: `launchctl load ~/Library/LaunchAgents/io.finance-os.pulse.plist`.

## Piping stdout (alternative)

If your gateway only accepts text on stdin, render the body with
`report daily-pulse` and pipe it:

```bash
finance-cli report daily-pulse --days 1 | your-whatsapp-sender
```

## Tuning

- `--days N` shifts the headline window (default 1 = today). The rest of the
  message is always month-bound.
- The "Frear" block only fires when a category is projected to overshoot the
  T3M baseline by ≥10% and ≥R$200. Categories with fewer than 3 MtD hits are
  compared to the full-month baseline directly (no pacing), which suppresses
  noise from one-shot fixed bills.
- The "A vencer" block reads from the `forecast` table. Keep it current with
  `finance-cli forecast upsert ...`. Stale forecasts → empty block; this is
  intentional and surfaces the data hygiene gap explicitly.
- Card due dates come from `accounts.metadata_json.billing_due_day`. When the
  field is empty (e.g. corporate meal-voucher cards), the due-date hint is
  omitted.

## Failure modes

- **`FINANCE_OS_WHATSAPP_WEBHOOK_URL not set`** — export the env var or pass
  `--dry-run`.
- **Webhook returns non-2xx** — `notify whatsapp` exits non-zero and prints
  the status + body to stderr. Cron will log it.
- **No new data** — the message still renders. Empty MtD shows
  "saldo R$ 0,00" and an "OnTrack" closing plan. This is intentional: silence
  hides problems.
