# Runbook: validating financial consistency (web / reports vs. the bank)

How to confirm that the dashboard, CLI reports, and BigQuery agree with each
other and with the bank — without committing any private data. This is the
durable part of the (now-completed) `web-ux-performance-data` plan; the
architecture it validates lives in [ADR-0025](adr/0025-cashflow-basis-bill-explosion.md)
and [ADR-0026](adr/0026-single-view-chain-canonical-source.md).

## The invariant being checked

Every cash-flow/spend surface reads **one** deduped view chain
(`transactions → v_transactions_effective → v_transactions_reportable →
v_transactions_cashbasis → v_cashflow / v_monthly_spend`). If the chart, the
month-detail list, `report monthly-spend`, and a direct `v_cashflow` query ever
disagree for the same month, something is reading outside the chain — fix that,
don't patch one surface.

## 1. Duplicate audit (built-in, read-only)

```bash
phai report duplicates            # human summary
phai report duplicates --raw      # JSON
```

Reports groups of rows sharing the dedup fingerprint
(`transaction_date`, `account_id`, `amount_cents`, normalised `raw_description`)
— the Pluggy `transaction_id` drift and ofx/pluggy overlaps that would inflate
expenses. The view chain already neutralises ofx-shadowed rows in reports; this
surfaces the physical rows so they can be cleaned up deliberately.

## 2. Cross-surface agreement (synthetic, in CI)

The SQLite-backed tests assert the chain agrees with itself. When adding a
report, add a test that its month total equals the corresponding `v_cashflow`
month — never assert a hand-rolled aggregate.

## 3. Local OFX oracle (private data — never committed)

The operator's real OFX statements are used **only locally** as the final
oracle. Per the privacy rules ([AGENTS.md](../AGENTS.md)), never commit OFX,
account names, counterparties, or real totals; fixes go into the generic engine
or into private config outside the repo.

Method (what to compare, per account, per month):

1. Parse the OFX locally (throwaway script) → sum debits/credits, **excluding**
   internal movements: the card bill payment (`Pagamento de fatura`) and
   transfers between the household's own **tracked** accounts. Salary arriving
   from an **untracked** relay account is real income — keep it.
2. Point `phai` at the real backend (`PHAI_CONFIG_DIR=<runtime>`), then compare
   the OFX figure against the deduped, cash-basis view for that account/month:

   ```sql
   SELECT ROUND(SUM(IF(amount_cents<0,ABS(amount_cents),0))/100.0,2) AS expenses,
          ROUND(SUM(IF(amount_cents>0,amount_cents,0))/100.0,2)      AS income
   FROM `<project>.<dataset>.v_transactions_cashbasis`
   WHERE account_id='<acct>' AND cash_month='<YYYY-MM>'
     AND COALESCE(category_id,'') NOT IN (
       SELECT category_id FROM `<project>.<dataset>.internal_categories`);
   ```
3. A **credit card** only reconciles when `billing_closing_day` /
   `billing_due_day` are set (`phai account set-billing-cycle`); otherwise the
   bill can't be exploded into its payment month and the card falls back to the
   calendar posting month.

A clean run reconciles to the cent (small Pluggy-vs-OFX nuances on reversals are
expected); any larger gap is a generic bug to fix in the view chain or
ingestion, not a per-surface patch.
