---
type: ADR
id: "0009"
title: "Pulse as a proactive closing-plan, not a retrospective transaction list"
status: accepted
date: 2026-05-18
---

## Context

The pre-existing `report daily-pulse` rendered the last *N* days of transactions
grouped by category, with one bullet per transaction and a `Saldo do período`
footer. It answered "what happened?" — useful as an audit log but useless as a
behavioural prompt: the user could not look at the message and decide *what to
do today* to land the month positive.

Field-tested constraints:

- The author runs this report on WhatsApp from cron. The phone screen is small,
  reading windows are seconds long, and the user is rarely at a keyboard when
  the message arrives.
- A transaction list rewards the user for sins already committed. The
  behavioural lever ("brake category X by R$Y this week") only fires *before*
  spend, not after.
- Forecast, card open balances, and category baselines are already in the
  store. Nothing in the report used them.

## Decision

**`report daily-pulse` is reframed as a proactive monthly closing-plan
message.** The body is five short blocks, each answering one question:

1. **Month-to-date headline.** Income / expenses / net so far, with the
   "previsão T3M" projection.
2. **Closing plan.** Three states — *on track* / *tight* / *stretched* —
   each with a single actionable number. In *tight*, the message states a
   maximum weekly variable budget that lands the month at zero.
3. **Frear neste mês.** Up to three categories whose projected EoM total
   already exceeds the T3M average by ≥10% AND ≥R$200. Categories with
   <3 MtD hits are treated as lumpy (one-shot fixed bills) and compared to
   the *full* baseline, not pro-rated — this suppresses early-month timing
   noise from fixed costs (aluguel, escolas, condomínio).
4. **A vencer.** Active forecasts in `(today, end_of_month]`, sorted by
   due date, plus a total.
5. **Cartões em aberto.** Open balance per credit card with the next
   `billing_due_day` resolved from `accounts.metadata_json`.

A final **Ação** footer lists uncategorized count and any budget envelope at
≥80% consumption.

The legacy per-transaction view is preserved behind `--raw` (JSON).

## Options considered

- **Refactor `daily-pulse` in place** (chosen): zero new commands, existing
  cron / skill / shell aliases keep working. The user has to update only the
  WhatsApp gateway that consumes stdout. Old behaviour stays available via
  `--raw`.
- **Add a new `report pulse` subcommand**: rejected — splits the surface,
  forces cron / skill / wrapper updates, and the legacy daily-pulse loses
  oxygen anyway.
- **Render two messages (daily + monthly)**: rejected — two messages per
  cron tick is noise. One opinionated message that combines both signals
  beats two passive ones.

## Consequences

- The pulse depends on `cashflow`, `monthly_spend`, `card_summary`,
  `budget_status_for_month`, `count_uncategorized`, `get_accounts` and the
  new `upcoming_forecasts`. All of these are already in the `FinanceStore`
  trait surface, so both backends keep parity.
- A new trait method `upcoming_forecasts(from, until) -> Vec<ForecastRecord>`
  was added with SQLite and BigQuery implementations.
- The pulse no longer prints individual transactions. Consumers that
  expected per-transaction lines must move to `--raw` (e.g. the openclaw
  skill's `report daily-pulse` invocation).
- The "Frear" heuristic uses two regimes (lumpy vs frequent) to decide
  whether to project. Without explicit fixed/variable category tagging,
  this is the cleanest signal we have. Adding a `kind: "fixed" | "variable"`
  tag on `categories.metadata_json` would make this exact and is a
  reasonable next step.
- `notify whatsapp` was introduced as a thin subcommand that posts the
  rendered body to a user-supplied webhook (`FINANCE_OS_WHATSAPP_WEBHOOK_URL`,
  optional bearer token), so cron entries can call the binary directly
  instead of piping stdout through a script.
- The e2e harness now sets `FINANCE_OS_NO_AUTO_UPDATE=1` and the `sync
  pluggy` force-check respects it. This unblocks reproducible local test
  runs (the auto-updater was overwriting `target/debug/finance-cli` between
  subprocess calls).
