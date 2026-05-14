use crate::config::{AppConfig, BackendKind};
use crate::models::{
    AccountRecord, AccountSnapshotRecord, AuditEvent, CardClosedTransactionRow, CardSummaryRow,
    CashflowRow, CategoryRecord, DailyPulseItem, ForecastRecord, ForecastVsActualRow,
    MonthlySpendRow, RuleRecord, TransactionContextRow, TransactionRecord, UncategorizedRow,
};
use crate::splits::{
    ItemPriceRow, ReceiptItemRecord, SplitCandidateRow, TransactionSplitDetail,
    TransactionSplitLineRecord, TransactionSplitRecord,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use std::collections::BTreeSet;

pub mod bigquery;
pub mod local;

const ALLOWED_TABLES: &[&str] = &[
    "schema_versions",
    "accounts",
    "account_snapshots",
    "categories",
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
    async fn insert_account_snapshots(&self, rows: &[AccountSnapshotRecord]) -> Result<usize>;
    async fn upsert_transactions(&self, rows: &[TransactionRecord]) -> Result<usize>;
    async fn upsert_rules(&self, rows: &[RuleRecord]) -> Result<usize>;
    async fn upsert_categories(&self, rows: &[CategoryRecord]) -> Result<usize>;
    async fn upsert_forecasts(&self, rows: &[ForecastRecord]) -> Result<usize>;
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
        context: Option<&str>,
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
    async fn transactions_with_context(&self, limit: usize) -> Result<Vec<TransactionContextRow>>;
    async fn count_transactions_with_context(&self) -> Result<i64>;
    async fn latest_pluggy_transaction_date(&self) -> Result<Option<NaiveDate>>;
    async fn daily_pulse(&self, since: NaiveDate) -> Result<Vec<DailyPulseItem>>;
    async fn effective_transactions_window(
        &self,
        since: NaiveDate,
        until: NaiveDate,
    ) -> Result<Vec<TransactionRecord>>;
    async fn monthly_spend(&self, month_ref: Option<&str>) -> Result<Vec<MonthlySpendRow>>;
    async fn cashflow(&self, months: usize) -> Result<Vec<CashflowRow>>;
    async fn forecast_vs_actual(&self, month_ref: Option<&str>)
        -> Result<Vec<ForecastVsActualRow>>;
    async fn card_summary(&self, month_ref: Option<&str>) -> Result<Vec<CardSummaryRow>>;
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
}

pub async fn open_store(config: &AppConfig) -> Result<Box<dyn FinanceStore>> {
    match config.effective_backend() {
        BackendKind::Bigquery => Ok(Box::new(
            bigquery::BigQueryStore::new(config.clone()).await?,
        )),
        BackendKind::Local => Ok(Box::new(local::LocalStore::new(config.clone())?)),
    }
}
