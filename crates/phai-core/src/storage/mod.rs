use crate::config::{AppConfig, BackendKind};
use crate::models::{
    AccountRecord, AccountSnapshotRecord, AuditEvent, BudgetStatusRow, CardClosedTransactionRow,
    CardSummaryRow, CashflowRow, CategoryBudgetRecord, CategoryRecord, CheckingBalance,
    DailyPulseItem, ForecastRecord, ForecastTemplateRecord, ForecastVsActualRow, MonthlySpendRow,
    RuleRecord, TransactionContextRow, TransactionRecord, UncategorizedRow,
};
use crate::splits::{
    ItemPriceRow, ReceiptItemRecord, SplitCandidateRow, TransactionSplitDetail,
    TransactionSplitLineRecord, TransactionSplitRecord,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::BTreeSet;

pub mod bigquery;
pub mod local;

#[derive(Debug, Clone, Copy, Default)]
pub struct TransactionAnatomyPatch<'a> {
    pub description: Option<&'a str>,
    pub merchant_name: Option<&'a str>,
    pub purpose: Option<&'a str>,
    pub classifier_trace: Option<&'a str>,
    /// Raw Pluggy context label (e.g. `"mercado-mes"`). `None` means keep
    /// the existing value; `Some(v)` overwrites it.
    pub context: Option<&'a str>,
}

const ALLOWED_TABLES: &[&str] = &[
    "schema_versions",
    "accounts",
    "account_snapshots",
    "categories",
    "category_budgets",
    "internal_categories",
    "rules",
    "transactions",
    "transaction_splits",
    "transaction_split_lines",
    "receipt_items",
    "split_review_policies",
    "audit_log",
    "forecast",
];

pub fn validate_table_name(table: &str) -> Result<()> {
    if ALLOWED_TABLES.contains(&table) {
        Ok(())
    } else {
        Err(anyhow!("Nome de tabela inválido: {table}"))
    }
}

#[async_trait(?Send)]
pub trait FinanceStore {
    async fn applied_migrations(&self) -> Result<BTreeSet<String>>;
    async fn apply_sql(&self, sql: &str) -> Result<()>;
    async fn record_migration(&self, version: &str) -> Result<()>;

    async fn upsert_accounts(&self, rows: &[AccountRecord]) -> Result<usize>;
    async fn get_accounts(&self) -> Result<Vec<AccountRecord>>;
    async fn insert_account_snapshots(&self, rows: &[AccountSnapshotRecord]) -> Result<usize>;
    /// Latest snapshot per account, ordered by `account_id`. One row per
    /// account that has ever been snapshotted; never duplicates. Used to
    /// answer "what's the saldo em conta right now?".
    async fn latest_account_snapshots(&self) -> Result<Vec<AccountSnapshotRecord>>;
    async fn upsert_transactions(&self, rows: &[TransactionRecord]) -> Result<usize>;
    async fn upsert_rules(&self, rows: &[RuleRecord]) -> Result<usize>;
    async fn upsert_categories(&self, rows: &[CategoryRecord]) -> Result<usize>;
    async fn upsert_forecasts(&self, rows: &[ForecastRecord]) -> Result<usize>;
    /// Insert or update forecast templates (ADR-0016). Uses
    /// `idempotency_key` as the merge key, like the other upserts.
    async fn upsert_forecast_templates(&self, rows: &[ForecastTemplateRecord]) -> Result<usize>;
    /// List forecast templates, optionally filtered by kind and/or status.
    /// `None` filters disable the corresponding `WHERE` clause.
    async fn list_forecast_templates(
        &self,
        kind: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<ForecastTemplateRecord>>;
    /// Look up a single template by its primary key. `Ok(None)` when missing.
    async fn get_forecast_template(
        &self,
        template_id: &str,
    ) -> Result<Option<ForecastTemplateRecord>>;
    /// Active forecasts whose `due_date` falls in `[from, until]` (inclusive).
    /// Only `status = 'ativo'` rows are returned. Ordered by due_date ascending,
    /// then by amount descending so the biggest commitments lead within a day.
    async fn upcoming_forecasts(
        &self,
        from: NaiveDate,
        until: NaiveDate,
    ) -> Result<Vec<ForecastRecord>>;
    /// List forecast records with optional status and/or date-range filters.
    /// When `status` is `None`, all statuses are included. When `from`/`until`
    /// are `None`, no date filtering is applied. Ordered by due_date ASC.
    async fn list_forecasts(
        &self,
        status: Option<&str>,
        from: Option<NaiveDate>,
        until: Option<NaiveDate>,
    ) -> Result<Vec<ForecastRecord>>;
    /// Look up a single forecast by its primary key. `Ok(None)` when missing.
    async fn get_forecast(&self, forecast_id: &str) -> Result<Option<ForecastRecord>>;
    /// Return every category record from the `categories` table with its name
    /// and parent. Unlike `list_all_category_ids` (which returns a flat set),
    /// this returns the full records suitable for dropdown rendering.
    async fn get_categories(&self) -> Result<Vec<CategoryRecord>>;
    async fn apply_transaction_split(
        &self,
        split: &TransactionSplitRecord,
        lines: &[TransactionSplitLineRecord],
        items: &[ReceiptItemRecord],
    ) -> Result<()>;
    async fn insert_audit_events(&self, rows: &[AuditEvent]) -> Result<usize>;

    async fn annotate_transaction(
        &self,
        transaction_id: &str,
        category_id: Option<&str>,
        category_source: Option<&str>,
        classifier_trace: Option<&str>,
        actor_id: &str,
        idempotency_key: &str,
    ) -> Result<()>;

    async fn update_transaction_anatomy(
        &self,
        transaction_id: &str,
        patch: TransactionAnatomyPatch<'_>,
        actor_id: &str,
        idempotency_key: &str,
    ) -> Result<()>;

    async fn find_transactions_by_description(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<TransactionRecord>>;
    async fn latest_uncategorized_transactions(
        &self,
        limit: usize,
    ) -> Result<Vec<TransactionRecord>>;
    async fn pending_human_descriptions(&self, limit: usize) -> Result<Vec<TransactionRecord>>;
    async fn pending_merchants(&self, limit: usize) -> Result<Vec<TransactionRecord>>;
    async fn pending_purposes(
        &self,
        min_abs_amount: Decimal,
        limit: usize,
    ) -> Result<Vec<TransactionRecord>>;
    async fn count_pending_human_descriptions(&self) -> Result<i64>;
    async fn count_pending_merchants(&self) -> Result<i64>;
    async fn count_pending_purposes(&self, min_abs_amount: Decimal) -> Result<i64>;

    async fn existing_transaction_ids(&self, ids: &[String]) -> Result<BTreeSet<String>>;
    async fn transaction_by_id(&self, transaction_id: &str) -> Result<Option<TransactionRecord>>;
    async fn transaction_split_detail(
        &self,
        transaction_id: &str,
    ) -> Result<Option<TransactionSplitDetail>>;
    async fn clear_transaction_split(
        &self,
        transaction_id: &str,
        actor_id: &str,
        idempotency_key: &str,
        reason: Option<&str>,
    ) -> Result<()>;
    async fn split_candidates(&self, since: NaiveDate) -> Result<Vec<SplitCandidateRow>>;
    async fn item_prices(&self, query: &str, since: Option<NaiveDate>)
        -> Result<Vec<ItemPriceRow>>;
    async fn all_rules(&self) -> Result<Vec<RuleRecord>>;
    async fn active_rules(&self) -> Result<Vec<RuleRecord>>;
    async fn internal_categories(&self) -> Result<BTreeSet<String>>;
    /// Returns every category id known to the system — the union of the
    /// `categories` reference table and DISTINCT `transactions.category_id`.
    /// Used by interactive tools (e.g. the review TUI picker) to surface the
    /// full set of available categories, not just the ones in the current
    /// in-memory queue.
    async fn list_all_category_ids(&self) -> Result<BTreeSet<String>>;
    async fn transactions_with_context(&self, limit: usize) -> Result<Vec<TransactionContextRow>>;
    async fn count_transactions_with_context(&self) -> Result<i64>;
    async fn latest_pluggy_transaction_date(&self) -> Result<Option<NaiveDate>>;
    async fn daily_pulse(&self, since: NaiveDate) -> Result<Vec<DailyPulseItem>>;
    /// Like [`Self::transactions_in_date_range`] but reads from the effective
    /// view, so split lines surface in place of their (hidden) parent. Use
    /// this for any user-facing listing or report. Reserve the raw
    /// `transactions_in_date_range` for dedup/reconciliation paths that must
    /// see exactly what was upserted.
    async fn effective_transactions_window(
        &self,
        account_id: Option<&str>,
        since: NaiveDate,
        until: NaiveDate,
    ) -> Result<Vec<TransactionRecord>>;
    /// Returns all transactions in `[from, to]` optionally filtered by account_id.
    /// If `account_id` is `None`, all accounts are included.
    async fn transactions_in_date_range(
        &self,
        account_id: Option<&str>,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<TransactionRecord>>;
    async fn monthly_spend(&self, month_ref: Option<&str>) -> Result<Vec<MonthlySpendRow>>;
    /// Cash-basis monthly cashflow restricted to `account_type='checking'`
    /// accounts. Excludes `transfer-internal` (transfers between own
    /// accounts) but **includes** `credit-card-payment` (the actual cash
    /// event when a card bill is paid). Returned rows carry
    /// `opening_balance` / `closing_balance` as `None` — use
    /// [`Self::cashflow_month`] when you need anchored balances for a
    /// single month.
    async fn cashflow(&self, months: usize) -> Result<Vec<CashflowRow>>;
    /// Single-month variant of [`Self::cashflow`] with snapshot-anchored
    /// `opening_balance` and `closing_balance` populated when every
    /// checking account has a usable snapshot anchor.
    async fn cashflow_month(&self, month_ref: &str) -> Result<CashflowRow>;
    /// Aggregate balance across all `account_type='checking'` accounts at
    /// `target`. Anchored on the latest snapshot ≤ `target` per account
    /// plus the delta of transactions between snapshot date and `target`.
    /// Returns `Ok(None)` when at least one checking account lacks a
    /// snapshot ≤ `target` (callers should surface "incomplete" rather
    /// than guessing).
    async fn checking_balance_at(&self, target: NaiveDate) -> Result<Option<CheckingBalance>>;
    async fn forecast_vs_actual(&self, month_ref: Option<&str>)
        -> Result<Vec<ForecastVsActualRow>>;
    async fn card_summary(&self, month_ref: Option<&str>) -> Result<Vec<CardSummaryRow>>;
    /// Cards' open bills as of "now". For each active credit account this
    /// returns at most one row — the cycle whose closing day is in the
    /// future (or today), determined per-account from
    /// `accounts.metadata_json.billing_closing_day`. Cards without a
    /// closing-day field fall back to the next calendar month.
    async fn cards_open_now(&self) -> Result<Vec<CardSummaryRow>>;
    async fn card_closed_transactions(
        &self,
        month_ref: Option<&str>,
    ) -> Result<Vec<CardClosedTransactionRow>>;
    async fn card_reportable_transactions(
        &self,
        month_ref: Option<&str>,
    ) -> Result<Vec<CardClosedTransactionRow>>;
    async fn uncategorized(&self, limit: usize) -> Result<Vec<UncategorizedRow>>;
    async fn count_uncategorized(&self) -> Result<i64>;
    async fn count_rows(&self, table: &str) -> Result<i64>;

    async fn upsert_category_budget(&self, record: &CategoryBudgetRecord) -> Result<()>;
    async fn list_category_budgets(&self, month: Option<&str>)
        -> Result<Vec<CategoryBudgetRecord>>;
    async fn budget_status_for_month(&self, month: &str) -> Result<Vec<BudgetStatusRow>>;

    /// Sibling transactions on the same day/account, ordered by Pluggy's
    /// `raw.order` (NULLS LAST). Excludes `exclude_id` so callers don't
    /// see the current transaction in the temporal context window.
    async fn transactions_on_date(
        &self,
        date: NaiveDate,
        account_id: &str,
        exclude_id: &str,
    ) -> Result<Vec<crate::enrichment::types::ContextTx>>;

    /// Transactions whose `description` contains `keyword`
    /// (case-insensitive), excluding `exclude_id`. When
    /// `only_uncategorized` is true, also filters to transactions whose
    /// category is missing or came from a weak source ('unclassified',
    /// 'fallback', 'pluggy').
    async fn similar_transactions(
        &self,
        keyword: &str,
        exclude_id: &str,
        only_uncategorized: bool,
    ) -> Result<Vec<TransactionRecord>>;

    /// Mark a transaction's `enrichment_attempted_at` to the current
    /// timestamp so subsequent enrichment runs skip it (unless
    /// `--retry` is passed). Inserts an `enrich_attempted` audit event.
    async fn mark_enrichment_attempted(
        &self,
        transaction_id: &str,
        actor_id: &str,
        idempotency_key: &str,
    ) -> Result<()>;

    /// Prior transactions from the same merchant, or the same raw description
    /// when merchant enrichment is not available, that carry a human-curated
    /// `description` or `purpose`. Used by the replication engine to
    /// propagate anatomy from recurring history.
    ///
    /// Any transaction with `description IS NOT NULL` or `purpose IS NOT NULL`
    /// qualifies — both fields are exclusively set by humans (via `set-anatomy`
    /// or `review-human`), so no `category_source` filter is needed.
    ///
    /// Results are ordered by `transaction_date DESC` (most recent first)
    /// and capped at 5 so callers can pick the best match without fetching
    /// unbounded history. `exclude_id` prevents a transaction from donating
    /// anatomy to itself.
    async fn find_anatomy_donors(
        &self,
        merchant_name: &str,
        exclude_id: &str,
    ) -> Result<Vec<TransactionRecord>>;

    /// Transactions that have a merchant/raw-description match key but are
    /// missing at least one of `description` or `purpose`. Used by the batch
    /// replication command to find candidates that may benefit from
    /// [`find_anatomy_donors`].
    ///
    /// Only categorized transactions (`category_id IS NOT NULL`) are
    /// included — uncategorized ones are still in-flight and not ready
    /// for anatomy propagation.
    async fn replicable_anatomy_candidates(&self, limit: usize) -> Result<Vec<TransactionRecord>>;
}

pub async fn open_store(config: &AppConfig) -> Result<Box<dyn FinanceStore>> {
    match config.effective_backend() {
        BackendKind::Bigquery => Ok(Box::new(
            bigquery::BigQueryStore::new(config.clone()).await?,
        )),
        BackendKind::Local => Ok(Box::new(local::LocalStore::new(config.clone())?)),
    }
}
