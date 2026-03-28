use crate::config::{AppConfig, BackendKind};
use crate::models::{
    AccountRecord, AuditEvent, CardSummaryRow, CashflowRow, CategoryRecord, DailyPulseItem,
    ForecastRecord, ForecastVsActualRow, MonthlySpendRow, RuleRecord, TransactionRecord,
    UncategorizedRow,
};
use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use std::collections::BTreeSet;

pub mod bigquery;
pub mod local;

#[async_trait(?Send)]
pub trait FinanceStore {
    async fn applied_migrations(&self) -> Result<BTreeSet<String>>;
    async fn apply_sql(&self, sql: &str) -> Result<()>;
    async fn record_migration(&self, version: &str) -> Result<()>;

    async fn upsert_accounts(&self, rows: &[AccountRecord]) -> Result<usize>;
    async fn upsert_transactions(&self, rows: &[TransactionRecord]) -> Result<usize>;
    async fn upsert_rules(&self, rows: &[RuleRecord]) -> Result<usize>;
    async fn upsert_categories(&self, rows: &[CategoryRecord]) -> Result<usize>;
    async fn upsert_forecasts(&self, rows: &[ForecastRecord]) -> Result<usize>;
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

    async fn existing_transaction_ids(&self, ids: &[String]) -> Result<BTreeSet<String>>;
    async fn latest_pluggy_transaction_date(&self) -> Result<Option<NaiveDate>>;
    async fn daily_pulse(&self, since: NaiveDate) -> Result<Vec<DailyPulseItem>>;
    async fn monthly_spend(&self, month_ref: Option<&str>) -> Result<Vec<MonthlySpendRow>>;
    async fn cashflow(&self, months: usize) -> Result<Vec<CashflowRow>>;
    async fn forecast_vs_actual(&self, month_ref: Option<&str>)
        -> Result<Vec<ForecastVsActualRow>>;
    async fn card_summary(&self, month_ref: Option<&str>) -> Result<Vec<CardSummaryRow>>;
    async fn uncategorized(&self, limit: usize) -> Result<Vec<UncategorizedRow>>;
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
