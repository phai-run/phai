use anyhow::{bail, Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use finance_core::idempotency::{
    category_id, ensure_account_idempotency, ensure_forecast_idempotency, ensure_rule_idempotency,
    ensure_transaction_idempotency, manual_transaction_idempotency,
};
use finance_core::legacy::load_legacy_bundle;
use finance_core::migrations::run_migrations;
use finance_core::models::{
    decimal_from_str, AccountRecord, AuditEvent, CardClosedTransactionRow, CategoryRecord,
    ForecastRecord, RuleRecord, TransactionRecord,
};
use finance_core::pluggy::{sync_pluggy, SyncPluggyParams};
use finance_core::rules::{apply_rules_with_facts, compile_rules};
use finance_core::splits::{
    build_split_records, parse_split_payload, validate_split_payload, SplitPayload, SplitPreview,
};
use finance_core::storage::{open_store, FinanceStore};
use finance_core::{AppConfig, BackendKind, ConfigPaths};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

mod review;

const UPSERT_BATCH_SIZE: usize = 50;
const AUDIT_BATCH_SIZE: usize = 25;
const DEFAULT_SYNC_LOOKBACK_DAYS: i64 = 14;

#[derive(Parser)]
#[command(name = "finance", version, about = "Finance OS v1 CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
    Sync {
        #[command(subcommand)]
        command: SyncCommand,
    },
    Report {
        #[command(subcommand)]
        command: ReportCommand,
    },
    Tx {
        #[command(subcommand)]
        command: TxCommand,
    },
    Forecast {
        #[command(subcommand)]
        command: ForecastCommand,
    },
    Rule {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
}

#[derive(Subcommand)]
enum AuthCommand {
    Setup(AuthSetupArgs),
}

#[derive(Args)]
struct AuthSetupArgs {
    #[arg(long, value_enum)]
    backend: BackendArg,
    #[arg(long)]
    actor_id: String,
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    dataset_id: Option<String>,
    #[arg(long)]
    service_account_path: Option<PathBuf>,
    #[arg(long)]
    local_db_path: Option<PathBuf>,
    #[arg(long)]
    pluggy_start_date: Option<String>,
}

#[derive(Subcommand)]
enum AdminCommand {
    Migrate,
    ImportLegacy(ImportLegacyArgs),
    Reclassify(ReclassifyArgs),
}

#[derive(Args)]
struct ReclassifyArgs {
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct ImportLegacyArgs {
    #[arg(long, default_value = "finance")]
    finance_root: PathBuf,
}

#[derive(Subcommand)]
enum SyncCommand {
    Pluggy(SyncPluggyArgs),
}

#[derive(Args)]
struct SyncPluggyArgs {
    #[arg(long, default_value = "finance/pluggy-config.json")]
    pluggy_config: PathBuf,
    #[arg(long, default_value = "finance/data/contas.csv")]
    accounts_csv: PathBuf,
    #[arg(long)]
    fixture: Option<PathBuf>,
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    to: Option<String>,
    #[arg(long)]
    json_summary: bool,
    #[arg(long)]
    notify_summary: bool,
}

#[derive(Subcommand)]
enum ReportCommand {
    DailyPulse(DailyPulseArgs),
    MonthlySpend(MonthlySpendArgs),
    Cashflow(CashflowArgs),
    ForecastVsActual(ForecastVsActualArgs),
    CardSummary(CardSummaryArgs),
    CardClosedInsights(CardClosedInsightsArgs),
    Uncategorized(UncategorizedArgs),
    SplitCandidates(SplitCandidatesArgs),
    ItemPrices(ItemPricesArgs),
    DataHealth(DataHealthArgs),
    Scenario(ScenarioArgs),
    OfxConsistency(OfxConsistencyArgs),
    Review(ReviewArgs),
}

#[derive(Args)]
struct DailyPulseArgs {
    #[arg(long, default_value_t = 7)]
    days: i64,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct MonthlySpendArgs {
    #[arg(long)]
    month: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct CashflowArgs {
    #[arg(long, default_value_t = 6)]
    months: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ForecastVsActualArgs {
    #[arg(long)]
    month: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct CardSummaryArgs {
    #[arg(long)]
    month: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct CardClosedInsightsArgs {
    #[arg(long)]
    month: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct UncategorizedArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct SplitCandidatesArgs {
    #[arg(long, default_value_t = 30)]
    days: i64,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ItemPricesArgs {
    #[arg(long)]
    query: String,
    #[arg(long)]
    since: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct DataHealthArgs {
    #[arg(long, default_value_t = 180)]
    days: i64,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ScenarioArgs {
    #[arg(long)]
    month: Option<String>,
    #[arg(long, default_value_t = 3)]
    history_months: usize,
    #[arg(long, default_value = "0")]
    extra_expense: String,
    #[arg(long, default_value = "0")]
    extra_income: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct OfxConsistencyArgs {
    #[arg(long)]
    ofx: PathBuf,
    #[arg(long)]
    account_id: Option<String>,
    #[arg(long, default_value_t = 1)]
    date_tolerance_days: i64,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ReviewArgs {
    #[arg(long, default_value_t = 6)]
    months: usize,
    #[arg(long)]
    output: Option<String>,
    #[arg(long)]
    open: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncSummaryTransaction {
    transaction_id: String,
    transaction_date: String,
    day_of_week: String,
    description: String,
    amount: String,
    tx_type: String,
    category_id: Option<String>,
    category_source: String,
    context: Option<String>,
    account_id: Option<String>,
    account_label: Option<String>,
    payment_status: String,
    source: String,
    metadata_json: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncSummaryPending {
    transaction_id: String,
    transaction_date: String,
    day_of_week: String,
    description: String,
    amount: String,
    tx_type: String,
    account_id: Option<String>,
    account_label: Option<String>,
    category_source: String,
    payment_status: String,
    source: String,
    metadata_json: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncSummaryOutput {
    actor_id: String,
    backend: String,
    generated_at: String,
    summary_status: String,
    warnings: Vec<String>,
    new_transactions_count: usize,
    needs_context_count: i64,
    needs_context_returned_count: usize,
    needs_context_truncated: bool,
    new_transactions: Vec<SyncSummaryTransaction>,
    needs_context: Vec<SyncSummaryPending>,
}

struct SyncPendingSummaryResult {
    summary_status: String,
    warnings: Vec<String>,
    needs_context_count: i64,
    needs_context_returned_count: usize,
    needs_context_truncated: bool,
    needs_context: Vec<SyncSummaryPending>,
}

fn compact_error_message(err: &anyhow::Error) -> String {
    format!("{err:#}")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}

fn transaction_needs_context(row: &TransactionRecord) -> bool {
    row.category_id.is_none() || matches!(row.category_source.as_str(), "unclassified" | "fallback")
}

fn build_sync_pending_summary_fallback(
    transactions: &[TransactionRecord],
    existing_ids: &BTreeSet<String>,
    account_labels: &BTreeMap<String, String>,
    warnings: Vec<String>,
) -> SyncPendingSummaryResult {
    const NEEDS_CONTEXT_LIMIT: usize = 100;
    let total = transactions
        .iter()
        .filter(|row| !existing_ids.contains(&row.transaction_id))
        .filter(|row| transaction_needs_context(row))
        .count();
    let needs_context = transactions
        .iter()
        .filter(|row| !existing_ids.contains(&row.transaction_id))
        .filter(|row| transaction_needs_context(row))
        .take(NEEDS_CONTEXT_LIMIT)
        .map(|row| SyncSummaryPending {
            transaction_id: row.transaction_id.clone(),
            transaction_date: row.transaction_date.format("%Y-%m-%d").to_string(),
            day_of_week: day_of_week_text(row.transaction_date),
            description: row.description.clone(),
            amount: decimal_text(row.amount),
            tx_type: row.tx_type.clone(),
            account_id: row.account_id.clone(),
            account_label: row
                .account_id
                .as_ref()
                .and_then(|account_id| account_labels.get(account_id).cloned()),
            category_source: row.category_source.clone(),
            payment_status: row.payment_status.clone(),
            source: row.source.clone(),
            metadata_json: row.metadata_json.clone(),
        })
        .collect::<Vec<_>>();

    SyncPendingSummaryResult {
        summary_status: "partial".to_string(),
        warnings,
        needs_context_count: total as i64,
        needs_context_returned_count: needs_context.len(),
        needs_context_truncated: total > needs_context.len(),
        needs_context,
    }
}

async fn load_sync_pending_summary(
    store: &dyn FinanceStore,
    transactions: &[TransactionRecord],
    existing_ids: &BTreeSet<String>,
    account_labels: &BTreeMap<String, String>,
) -> SyncPendingSummaryResult {
    const NEEDS_CONTEXT_LIMIT: usize = 100;

    let needs_context_count = match store.count_uncategorized().await {
        Ok(total) => total,
        Err(err) => {
            return build_sync_pending_summary_fallback(
                transactions,
                existing_ids,
                account_labels,
                vec![format!(
                    "needs_context_fallback_sync_only: count_unavailable: {}",
                    compact_error_message(&err)
                )],
            );
        }
    };

    let needs_context = match store.uncategorized(NEEDS_CONTEXT_LIMIT).await {
        Ok(rows) => rows
            .into_iter()
            .map(|row| SyncSummaryPending {
                transaction_id: row.transaction_id,
                transaction_date: row.transaction_date.format("%Y-%m-%d").to_string(),
                day_of_week: day_of_week_text(row.transaction_date),
                description: row.description,
                amount: decimal_text(row.amount),
                tx_type: row.tx_type,
                account_id: row.account_id,
                account_label: row.account_label,
                category_source: row.category_source,
                payment_status: row.payment_status,
                source: row.source,
                metadata_json: row.metadata_json,
            })
            .collect::<Vec<_>>(),
        Err(err) => {
            return build_sync_pending_summary_fallback(
                transactions,
                existing_ids,
                account_labels,
                vec![format!(
                    "needs_context_fallback_sync_only: list_unavailable: {}",
                    compact_error_message(&err)
                )],
            );
        }
    };

    SyncPendingSummaryResult {
        summary_status: "complete".to_string(),
        warnings: Vec::new(),
        needs_context_count,
        needs_context_returned_count: needs_context.len(),
        needs_context_truncated: needs_context_count > needs_context.len() as i64,
        needs_context,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FlatCategoryCount {
    category_id: String,
    count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DataHealthOutput {
    actor_id: String,
    backend: String,
    generated_at: String,
    window_start: String,
    window_days: i64,
    latest_pluggy_transaction_date: Option<String>,
    pluggy_lag_days: Option<i64>,
    total_transactions: i64,
    transactions_with_context: i64,
    context_coverage_ratio: f64,
    uncategorized_count: i64,
    active_rules_count: usize,
    window_rows: usize,
    window_pluggy_rows: usize,
    window_legacy_rows: usize,
    window_other_rows: usize,
    categorized_rows: usize,
    flat_category_rows: usize,
    overlap_candidates_count: usize,
    overlap_candidates: Vec<OverlapCandidate>,
    flat_categories: Vec<FlatCategoryCount>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlapCandidate {
    account_id: Option<String>,
    amount: String,
    legacy_transaction_id: String,
    legacy_date: String,
    legacy_description: String,
    pluggy_transaction_id: String,
    pluggy_date: String,
    pluggy_description: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScenarioOutput {
    target_month: String,
    baseline_months: Vec<String>,
    avg_income: Decimal,
    avg_expenses: Decimal,
    avg_net: Decimal,
    planning_expenses: Decimal,
    known_forecast_expenses: Decimal,
    known_forecast_count: usize,
    carryover_open_card_month: String,
    carryover_open_card_amount: Decimal,
    extra_income: Decimal,
    extra_expense: Decimal,
    projected_net: Decimal,
    projected_cash_after_card_carry: Decimal,
    looks_ok_after_card_carry: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CardClosedAccountInsight {
    account_id: String,
    total_charges: Decimal,
    open_amount: Decimal,
    closed_amount: Decimal,
    closed_transactions: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CardClosedCategoryInsight {
    category_id: String,
    amount: Decimal,
    transactions: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CardClosedRecurringInsight {
    merchant_key: String,
    amount: Decimal,
    transactions: usize,
    months_detected: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CardClosedSubscriptionInsight {
    merchant_key: String,
    amount: Decimal,
    transactions: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CardClosedInstallmentInsight {
    merchant_key: String,
    marker: String,
    amount: Decimal,
    transactions: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CardClosedInsightsOutput {
    month_ref: String,
    accounts: Vec<CardClosedAccountInsight>,
    categories: Vec<CardClosedCategoryInsight>,
    recurring: Vec<CardClosedRecurringInsight>,
    subscriptions: Vec<CardClosedSubscriptionInsight>,
    closed_installments: Vec<CardClosedInstallmentInsight>,
    open_installments: Vec<CardClosedInstallmentInsight>,
}

#[derive(Debug, Clone)]
struct OfxTransaction {
    fit_id: Option<String>,
    transaction_date: NaiveDate,
    amount: Decimal,
    description: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OfxConsistencyMatch {
    status: String,
    ofx_fit_id: Option<String>,
    ofx_date: String,
    ofx_amount: Decimal,
    ofx_description: String,
    transaction_id: Option<String>,
    account_id: Option<String>,
    transaction_date: Option<String>,
    transaction_amount: Option<Decimal>,
    transaction_description: Option<String>,
    date_diff_days: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OfxConsistencyDbOnly {
    transaction_id: String,
    transaction_date: String,
    amount: Decimal,
    description: String,
    account_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OfxConsistencyOutput {
    ofx_path: String,
    account_filter: Option<String>,
    range_start: String,
    range_end: String,
    date_tolerance_days: i64,
    ofx_transactions: usize,
    db_candidates: usize,
    matched: usize,
    missing_in_finance: usize,
    extra_in_finance: usize,
    consistent: bool,
    matches: Vec<OfxConsistencyMatch>,
    extra_transactions: Vec<OfxConsistencyDbOnly>,
}

#[derive(Subcommand)]
enum TxCommand {
    UpsertManual(ManualTransactionArgs),
    Categorize(CategorizeTransactionArgs),
    SetContext(SetContextArgs),
    ListContext(ListContextArgs),
    Split {
        #[command(subcommand)]
        command: TxSplitCommand,
    },
}

#[derive(Subcommand)]
enum TxSplitCommand {
    Preview(TxSplitPreviewArgs),
    Apply(TxSplitApplyArgs),
    Show(TxSplitShowArgs),
    Clear(TxSplitClearArgs),
}

#[derive(Args)]
struct ManualTransactionArgs {
    #[arg(long)]
    transaction_id: Option<String>,
    #[arg(long)]
    account_id: Option<String>,
    #[arg(long)]
    date: String,
    #[arg(long)]
    description: String,
    #[arg(long)]
    amount: String,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    subcategory: Option<String>,
    #[arg(long)]
    context: Option<String>,
    #[arg(long, default_value = "confirmed")]
    payment_status: String,
}

#[derive(Args)]
struct CategorizeTransactionArgs {
    #[arg(long)]
    transaction_id: String,
    #[arg(long)]
    category: String,
    #[arg(long)]
    subcategory: Option<String>,
    #[arg(long)]
    context: Option<String>,
}

#[derive(Args)]
struct SetContextArgs {
    #[arg(long)]
    transaction_id: String,
    #[arg(long)]
    context: String,
}

#[derive(Args)]
struct ListContextArgs {
    #[arg(long, default_value_t = 50)]
    limit: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct TxSplitPreviewArgs {
    #[arg(long)]
    transaction_id: String,
    #[arg(long)]
    payload: PathBuf,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct TxSplitApplyArgs {
    #[arg(long)]
    transaction_id: String,
    #[arg(long)]
    payload: PathBuf,
}

#[derive(Args)]
struct TxSplitShowArgs {
    #[arg(long)]
    transaction_id: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct TxSplitClearArgs {
    #[arg(long)]
    transaction_id: String,
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Subcommand)]
enum ForecastCommand {
    Upsert(ForecastUpsertArgs),
}

#[derive(Args)]
struct ForecastUpsertArgs {
    #[arg(long)]
    forecast_id: Option<String>,
    #[arg(long)]
    date: Option<String>,
    #[arg(long)]
    description: String,
    #[arg(long)]
    amount: String,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    subcategory: Option<String>,
    #[arg(long)]
    account_id: Option<String>,
    #[arg(long, default_value = "active")]
    status: String,
    #[arg(long)]
    recurrence: Option<String>,
}

#[derive(Subcommand)]
enum RuleCommand {
    Upsert(RuleUpsertArgs),
    List(RuleListArgs),
    Inspect(RuleInspectArgs),
}

#[derive(Args)]
struct RuleUpsertArgs {
    #[arg(long)]
    rule_id: String,
    #[arg(long)]
    body: String,
    #[arg(long, default_value = "active")]
    status: String,
}

#[derive(Args)]
struct RuleListArgs {
    #[arg(long, value_enum, default_value_t = RuleStatusFilter::Active)]
    status: RuleStatusFilter,
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum RuleStatusFilter {
    Active,
    Disabled,
    All,
}

impl RuleStatusFilter {
    fn as_str(self) -> &'static str {
        match self {
            RuleStatusFilter::Active => "active",
            RuleStatusFilter::Disabled => "disabled",
            RuleStatusFilter::All => "all",
        }
    }
}

#[derive(Args)]
struct RuleInspectArgs {
    #[arg(long)]
    rule_id: String,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum AccountCommand {
    Upsert(AccountUpsertArgs),
}

#[derive(Args)]
struct AccountUpsertArgs {
    #[arg(long)]
    account_id: String,
    #[arg(long)]
    owner: String,
    #[arg(long)]
    account_type: String,
    #[arg(long)]
    bank: String,
    #[arg(long)]
    label: String,
    #[arg(long)]
    pluggy_account_id: Option<String>,
    #[arg(long)]
    pluggy_item_id: Option<String>,
    #[arg(long, default_value = "active")]
    status: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    Local,
    Bigquery,
}

impl From<BackendArg> for BackendKind {
    fn from(value: BackendArg) -> Self {
        match value {
            BackendArg::Local => BackendKind::Local,
            BackendArg::Bigquery => BackendKind::Bigquery,
        }
    }
}

fn brl(value: Decimal) -> String {
    let sign = if value.is_sign_negative() { "-" } else { "+" };
    let rounded = format!("{:.2}", value.abs().round_dp(2)).replace('.', ",");
    format!("{sign}R$ {rounded}")
}

fn decimal_text(value: Decimal) -> String {
    format!("{:.2}", value.round_dp(2))
}

fn day_of_week_text(date: NaiveDate) -> String {
    date.format("%A").to_string().to_ascii_lowercase()
}

fn normalize_inline_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn category_family(category_id: Option<&str>) -> Option<String> {
    let raw = category_id?.trim();
    if raw.is_empty() {
        return None;
    }
    let normalized = raw.replace([':', '-', '>'], " ");
    normalized
        .split_whitespace()
        .next()
        .map(|part| part.to_string())
}

fn category_emoji(category_id: Option<&str>, amount: Option<Decimal>) -> &'static str {
    let family = category_family(category_id);
    if family.as_deref() == Some("receitas")
        || family.as_deref() == Some("salario")
        || amount.is_some_and(|v| v > Decimal::ZERO)
    {
        "💰"
    } else if family.as_deref().is_some_and(|f| f.starts_with("transfer")) {
        "🔁"
    } else if family.as_deref() == Some("assinaturas") {
        "🔂"
    } else if matches!(family.as_deref(), Some("moradia" | "casa")) {
        "🏠"
    } else if family.as_deref() == Some("alimentacao") {
        "🍽️"
    } else if family.as_deref() == Some("saude") {
        "🩺"
    } else if matches!(family.as_deref(), Some("transporte" | "mobilidade")) {
        "🚗"
    } else if family.as_deref() == Some("educacao") {
        "📚"
    } else if family.as_deref() == Some("lazer") {
        "🎉"
    } else if family.as_deref() == Some("investimentos") {
        "📈"
    } else if family.as_deref() == Some("financeiro") {
        "🧾"
    } else if family.is_none() {
        "❓"
    } else {
        "💸"
    }
}

fn category_display(category_id: Option<&str>, amount: Option<Decimal>) -> String {
    let emoji = category_emoji(category_id, amount);
    let humanized = category_id
        .map(|category| category.replace(':', " > ").replace('-', " "))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "sem categoria".to_string());
    format!("{emoji} {humanized}")
}

fn display_label(
    description: &str,
    context: Option<&str>,
    category_id: Option<&str>,
    amount: Option<Decimal>,
) -> String {
    let label = context
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(description);
    format!(
        "{} {}",
        category_emoji(category_id, amount),
        normalize_inline_text(label)
    )
}

fn render_sync_notify_summary(summary: &SyncSummaryOutput) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Novas transações detectadas ({}):",
        summary.new_transactions_count
    ));
    for (idx, tx) in summary.new_transactions.iter().enumerate() {
        let amount = decimal_from_str(&tx.amount).ok();
        let category = category_display(tx.category_id.as_deref(), amount);
        let label = display_label(
            &tx.description,
            tx.context.as_deref(),
            tx.category_id.as_deref(),
            amount,
        );
        lines.push(format!(
            "{}. {} | {} | {} | {} ({}) | {}",
            idx + 1,
            tx.transaction_date,
            tx.amount,
            label,
            category,
            tx.category_source,
            tx.transaction_id
        ));
    }

    lines.push(String::new());
    lines.push(format!(
        "Pendências de contexto ({}){}:",
        summary.needs_context_count,
        if summary.needs_context_truncated {
            " (lista parcial)"
        } else {
            ""
        }
    ));
    for (idx, tx) in summary.needs_context.iter().enumerate() {
        let amount = decimal_from_str(&tx.amount).ok();
        let label = display_label(&tx.description, None, None, amount);
        lines.push(format!(
            "{}. {} | {} | {} | {}",
            idx + 1,
            tx.transaction_date,
            tx.amount,
            label,
            tx.transaction_id
        ));
    }

    if !summary.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Avisos:".to_string());
        for warning in &summary.warnings {
            lines.push(format!("- {}", normalize_inline_text(warning)));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "Fonte: {} | cli: finance {} | sync: {} | status: {}",
        summary.backend,
        env!("CARGO_PKG_VERSION"),
        summary.generated_at,
        summary.summary_status
    ));
    lines.join("\n")
}

fn parse_month_ref(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d")
        .with_context(|| format!("month inválido: {value} (esperado YYYY-MM)"))
}

fn month_ref_for(date: NaiveDate) -> String {
    date.format("%Y-%m").to_string()
}

fn shift_month(date: NaiveDate, delta: i32) -> Result<NaiveDate> {
    let mut year = date.year();
    let mut month = date.month() as i32 + delta;
    while month > 12 {
        year += 1;
        month -= 12;
    }
    while month < 1 {
        year -= 1;
        month += 12;
    }
    NaiveDate::from_ymd_opt(year, month as u32, 1)
        .with_context(|| format!("Falha ao calcular mês deslocado a partir de {}", date))
}

fn default_scenario_month(today: NaiveDate) -> Result<String> {
    Ok(month_ref_for(shift_month(
        NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
            .context("Falha ao calcular mês atual")?,
        1,
    )?))
}

fn default_closed_cards_month(today: NaiveDate) -> Result<String> {
    Ok(month_ref_for(shift_month(
        NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
            .context("Falha ao calcular mês atual")?,
        -1,
    )?))
}

fn is_open_card_payment_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "pending" | "em_aberto" | "parcial"
    )
}

fn is_flat_category(category_id: &str) -> bool {
    !category_id.contains(':')
}

fn normalize_description(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut prev_space = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        let mapped = if ch.is_ascii_alphanumeric() { ch } else { ' ' };
        if mapped == ' ' {
            if !prev_space {
                normalized.push(mapped);
            }
            prev_space = true;
        } else {
            normalized.push(mapped);
            prev_space = false;
        }
    }
    normalized.trim().to_string()
}

fn compact_token_ascii(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>()
}

fn extract_installment_marker_from_tokens(tokens: &[&str], idx: usize) -> Option<String> {
    let token = tokens.get(idx)?;
    let cleaned = token.trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '/');
    let mut parts = cleaned.split('/');
    if let (Some(left), Some(right), None) = (parts.next(), parts.next(), parts.next()) {
        if !left.is_empty()
            && !right.is_empty()
            && left.chars().all(|ch| ch.is_ascii_digit())
            && right.chars().all(|ch| ch.is_ascii_digit())
        {
            return Some(format!("{left}/{right}"));
        }
    }

    let compact = compact_token_ascii(token);
    if let Some(number) = compact
        .strip_prefix("parcela")
        .or_else(|| compact.strip_prefix("parc"))
    {
        if !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit()) {
            return Some(format!("parcela-{number}"));
        }
        if number.is_empty() {
            let next = tokens.get(idx + 1)?;
            let next_clean = next.trim_matches(|ch: char| !ch.is_ascii_digit());
            if !next_clean.is_empty() && next_clean.chars().all(|ch| ch.is_ascii_digit()) {
                return Some(format!("parcela-{next_clean}"));
            }
        }
    }
    None
}

fn extract_installment_marker(value: &str) -> Option<String> {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    for idx in 0..tokens.len() {
        if let Some(marker) = extract_installment_marker_from_tokens(&tokens, idx) {
            return Some(marker);
        }
    }
    None
}

fn strip_installment_marker(value: &str) -> String {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    let mut kept = Vec::new();
    let mut idx = 0usize;
    while idx < tokens.len() {
        if extract_installment_marker_from_tokens(&tokens, idx).is_some() {
            let compact = compact_token_ascii(tokens[idx]);
            if (compact == "parcela" || compact == "parc") && idx + 1 < tokens.len() {
                idx += 2;
            } else {
                idx += 1;
            }
            continue;
        }
        kept.push(tokens[idx]);
        idx += 1;
    }
    kept.join(" ")
}

fn metadata_contains_installment_signal(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, child)| {
            let key_lc = key.to_ascii_lowercase();
            key_lc.contains("installment")
                || key_lc.contains("parcela")
                || metadata_contains_installment_signal(child)
        }),
        Value::Array(items) => items.iter().any(metadata_contains_installment_signal),
        Value::String(text) => {
            let text_lc = text.to_ascii_lowercase();
            extract_installment_marker(text).is_some()
                || text_lc.contains("parc")
                || text_lc.contains("parcela")
        }
        _ => false,
    }
}

fn merchant_key_for_card_row(row: &CardClosedTransactionRow) -> String {
    let base = if row.label.trim().is_empty() {
        row.description.as_str()
    } else {
        row.label.as_str()
    };
    let normalized = normalize_description(&strip_installment_marker(base));
    if normalized.is_empty() {
        "sem-chave".to_string()
    } else {
        normalized
    }
}

fn detect_installment_marker(row: &CardClosedTransactionRow) -> Option<String> {
    extract_installment_marker(&row.label)
        .or_else(|| extract_installment_marker(&row.description))
        .or_else(|| {
            metadata_contains_installment_signal(&row.metadata_json).then(|| "metadata".to_string())
        })
}

fn is_subscription_row(row: &CardClosedTransactionRow) -> bool {
    if row
        .category_id
        .as_deref()
        .is_some_and(|category| category.starts_with("assinaturas"))
    {
        return true;
    }
    let label = format!("{} {}", row.label, row.description).to_ascii_lowercase();
    ["assinatura", "subscription", "mensalidade", "anuidade"]
        .iter()
        .any(|needle| label.contains(needle))
}

fn descriptions_look_related(left: &str, right: &str) -> bool {
    let left_norm = normalize_description(left);
    let right_norm = normalize_description(right);
    if left_norm.is_empty() || right_norm.is_empty() {
        return false;
    }
    if left_norm == right_norm || left_norm.contains(&right_norm) || right_norm.contains(&left_norm)
    {
        return true;
    }
    if extract_installment_marker(left) == extract_installment_marker(right)
        && extract_installment_marker(left).is_some()
    {
        return true;
    }
    let left_tokens = left_norm
        .split_whitespace()
        .filter(|token| token.len() >= 4)
        .collect::<BTreeSet<_>>();
    let right_tokens = right_norm
        .split_whitespace()
        .filter(|token| token.len() >= 4)
        .collect::<BTreeSet<_>>();
    left_tokens.intersection(&right_tokens).count() >= 2
}

fn extract_ofx_tag_value_from_line(line: &str, tag: &str) -> Option<String> {
    let trimmed = line.trim();
    let open_tag = format!("<{tag}>");
    let prefix = trimmed.get(..open_tag.len())?;
    if !prefix.eq_ignore_ascii_case(&open_tag) {
        return None;
    }
    let tail = trimmed.get(open_tag.len()..)?.trim();
    let value = tail.split('<').next().unwrap_or_default().trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_ofx_tag_values(raw: &str, tag: &str) -> Vec<String> {
    let mut values = Vec::new();
    for line in raw.lines() {
        if let Some(value) = extract_ofx_tag_value_from_line(line, tag) {
            values.push(value);
        }
    }
    values
}

fn parse_ofx_date(value: &str) -> Result<NaiveDate> {
    let digits = value
        .chars()
        .filter(char::is_ascii_digit)
        .take(8)
        .collect::<String>();
    if digits.len() != 8 {
        bail!("Data OFX inválida: {value}");
    }
    NaiveDate::parse_from_str(&digits, "%Y%m%d")
        .with_context(|| format!("Falha ao parsear data OFX {value}"))
}

fn parse_ofx_amount(value: &str) -> Result<Decimal> {
    let normalized = value.trim().replace(',', ".");
    decimal_from_str(&normalized).with_context(|| format!("Valor OFX inválido: {value}"))
}

fn parse_ofx_transactions(raw: &str) -> Result<Vec<OfxTransaction>> {
    let mut blocks = Vec::<String>::new();
    let mut current_block = Vec::<String>::new();
    let mut in_statement = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("<STMTTRN>") {
            if in_statement && !current_block.is_empty() {
                blocks.push(current_block.join("\n"));
                current_block.clear();
            }
            in_statement = true;
            continue;
        }
        if trimmed.eq_ignore_ascii_case("</STMTTRN>") {
            if in_statement {
                blocks.push(current_block.join("\n"));
                current_block.clear();
                in_statement = false;
            }
            continue;
        }
        if in_statement {
            current_block.push(trimmed.to_string());
        }
    }

    if in_statement && !current_block.is_empty() {
        blocks.push(current_block.join("\n"));
    }

    let mut rows = Vec::with_capacity(blocks.len());
    for block in blocks {
        let date_raw = parse_ofx_tag_values(&block, "DTPOSTED")
            .into_iter()
            .next()
            .context("Transação OFX sem DTPOSTED")?;
        let amount_raw = parse_ofx_tag_values(&block, "TRNAMT")
            .into_iter()
            .next()
            .context("Transação OFX sem TRNAMT")?;
        let fit_id = parse_ofx_tag_values(&block, "FITID").into_iter().next();
        let description = parse_ofx_tag_values(&block, "MEMO")
            .into_iter()
            .next()
            .or_else(|| parse_ofx_tag_values(&block, "NAME").into_iter().next())
            .or_else(|| parse_ofx_tag_values(&block, "PAYEE").into_iter().next())
            .unwrap_or_else(|| "sem-descricao".to_string());
        rows.push(OfxTransaction {
            fit_id,
            transaction_date: parse_ofx_date(&date_raw)?,
            amount: parse_ofx_amount(&amount_raw)?,
            description: normalize_inline_text(&description),
        });
    }
    Ok(rows)
}

fn description_distance(left: &str, right: &str) -> usize {
    let left_norm = normalize_description(left);
    let right_norm = normalize_description(right);
    if left_norm.is_empty() || right_norm.is_empty() {
        return usize::MAX / 2;
    }
    if left_norm == right_norm {
        return 0;
    }
    if left_norm.contains(&right_norm) || right_norm.contains(&left_norm) {
        return 1;
    }
    let left_tokens = left_norm
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .collect::<BTreeSet<_>>();
    let right_tokens = right_norm
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .collect::<BTreeSet<_>>();
    let overlap = left_tokens.intersection(&right_tokens).count();
    if overlap >= 3 {
        2
    } else if overlap == 2 {
        3
    } else if overlap == 1 {
        6
    } else {
        10
    }
}

fn metadata_pluggy_account_id(row: &TransactionRecord) -> Option<&str> {
    row.metadata_json
        .get("pluggy_account_id")
        .and_then(Value::as_str)
        .or_else(|| {
            row.metadata_json
                .get("raw")
                .and_then(|raw| raw.get("accountId"))
                .and_then(Value::as_str)
        })
}

fn infer_account_id_from_fit_ids(
    ofx_rows: &[OfxTransaction],
    db_rows: &[TransactionRecord],
) -> Option<String> {
    let fit_ids = ofx_rows
        .iter()
        .filter_map(|row| row.fit_id.as_deref())
        .collect::<BTreeSet<_>>();
    if fit_ids.is_empty() {
        return None;
    }

    let mut counts = BTreeMap::<String, usize>::new();
    for row in db_rows {
        if !fit_ids.contains(row.transaction_id.as_str()) {
            continue;
        }
        if let Some(account_id) = row.account_id.as_deref() {
            *counts.entry(account_id.to_string()).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|(left_id, left_count), (right_id, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_id.cmp(left_id))
        })
        .map(|(account_id, _)| account_id)
}

fn infer_account_id_from_overlap(
    ofx_rows: &[OfxTransaction],
    db_rows: &[TransactionRecord],
    date_tolerance_days: i64,
) -> Option<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for row in db_rows {
        let Some(account_id) = row.account_id.as_deref() else {
            continue;
        };
        let matches_any = ofx_rows.iter().any(|ofx_row| {
            let amount_diff = (row.amount - ofx_row.amount).abs();
            if amount_diff > Decimal::new(1, 2) {
                return false;
            }
            let date_diff = (row.transaction_date - ofx_row.transaction_date)
                .num_days()
                .abs();
            date_diff <= date_tolerance_days
        });
        if matches_any {
            *counts.entry(account_id.to_string()).or_insert(0) += 1;
        }
    }

    counts
        .into_iter()
        .max_by(|(left_id, left_count), (right_id, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_id.cmp(left_id))
        })
        .map(|(account_id, _)| account_id)
}

fn find_best_ofx_match(
    ofx_tx: &OfxTransaction,
    db_rows: &[TransactionRecord],
    used_rows: &[bool],
    date_tolerance_days: i64,
) -> Option<(usize, Decimal, i64, usize)> {
    let mut best: Option<(usize, Decimal, i64, usize)> = None;
    for (idx, candidate) in db_rows.iter().enumerate() {
        if used_rows.get(idx).copied().unwrap_or(false) {
            continue;
        }
        let amount_diff = (candidate.amount - ofx_tx.amount).abs();
        if amount_diff > Decimal::new(1, 2) {
            continue;
        }
        let date_diff = (candidate.transaction_date - ofx_tx.transaction_date)
            .num_days()
            .abs();
        if date_diff > date_tolerance_days {
            continue;
        }
        let desc_distance = description_distance(&ofx_tx.description, &candidate.description);
        let replace_best = match best {
            None => true,
            Some((_, best_amount, best_date, best_desc)) => {
                amount_diff < best_amount
                    || (amount_diff == best_amount
                        && (date_diff < best_date
                            || (date_diff == best_date && desc_distance < best_desc)))
            }
        };
        if replace_best {
            best = Some((idx, amount_diff, date_diff, desc_distance));
        }
    }
    best
}

fn build_category_records_from_transactions(
    actor_id: &str,
    rows: &[TransactionRecord],
) -> Vec<CategoryRecord> {
    let mut categories = BTreeMap::<String, CategoryRecord>::new();
    let now = Utc::now();
    for row in rows {
        if let Some(category_id) = &row.category_id {
            let mut parts = category_id.splitn(2, ':');
            if let Some(parent) = parts.next() {
                categories
                    .entry(parent.to_string())
                    .or_insert_with(|| CategoryRecord {
                        category_id: parent.to_string(),
                        name: parent.replace('-', " "),
                        parent_category_id: None,
                        metadata_json: json!({"source": "pluggy_sync"}),
                        actor_id: actor_id.to_string(),
                        updated_at: now,
                    });
                if let Some(child) = parts.next() {
                    categories
                        .entry(category_id.clone())
                        .or_insert_with(|| CategoryRecord {
                            category_id: category_id.clone(),
                            name: child.replace('-', " "),
                            parent_category_id: Some(parent.to_string()),
                            metadata_json: json!({"source": "pluggy_sync"}),
                            actor_id: actor_id.to_string(),
                            updated_at: now,
                        });
                }
            }
        }
    }
    categories.into_values().collect()
}

async fn upsert_accounts_chunked(
    store: &dyn FinanceStore,
    rows: &[AccountRecord],
) -> Result<usize> {
    let mut total = 0;
    for chunk in rows.chunks(UPSERT_BATCH_SIZE) {
        total += store.upsert_accounts(chunk).await?;
    }
    Ok(total)
}

async fn upsert_transactions_chunked(
    store: &dyn FinanceStore,
    rows: &[TransactionRecord],
) -> Result<usize> {
    let mut total = 0;
    for chunk in rows.chunks(UPSERT_BATCH_SIZE) {
        total += store.upsert_transactions(chunk).await?;
    }
    Ok(total)
}

async fn upsert_rules_chunked(store: &dyn FinanceStore, rows: &[RuleRecord]) -> Result<usize> {
    let mut total = 0;
    for chunk in rows.chunks(UPSERT_BATCH_SIZE) {
        total += store.upsert_rules(chunk).await?;
    }
    Ok(total)
}

async fn upsert_categories_chunked(
    store: &dyn FinanceStore,
    rows: &[CategoryRecord],
) -> Result<usize> {
    let mut total = 0;
    for chunk in rows.chunks(UPSERT_BATCH_SIZE) {
        total += store.upsert_categories(chunk).await?;
    }
    Ok(total)
}

async fn upsert_forecasts_chunked(
    store: &dyn FinanceStore,
    rows: &[ForecastRecord],
) -> Result<usize> {
    let mut total = 0;
    for chunk in rows.chunks(UPSERT_BATCH_SIZE) {
        total += store.upsert_forecasts(chunk).await?;
    }
    Ok(total)
}

async fn insert_audit_chunked(store: &dyn FinanceStore, rows: &[AuditEvent]) -> Result<usize> {
    let mut total = 0;
    for chunk in rows.chunks(AUDIT_BATCH_SIZE) {
        total += store.insert_audit_events(chunk).await?;
    }
    Ok(total)
}

async fn load_config() -> Result<(ConfigPaths, AppConfig)> {
    let paths = ConfigPaths::discover()?;
    paths.ensure()?;
    let config = AppConfig::load(&paths)?;
    Ok((paths, config))
}

fn ensure_bigquery_split_backend(config: &AppConfig) -> Result<()> {
    if !matches!(config.effective_backend(), BackendKind::Bigquery) {
        bail!("transaction split/detailing is supported only on the BigQuery backend");
    }
    Ok(())
}

async fn load_split_payload_preview(
    store: &dyn FinanceStore,
    transaction_id: &str,
    payload_path: &PathBuf,
) -> Result<(TransactionRecord, SplitPayload, SplitPreview)> {
    let parent = store
        .transaction_by_id(transaction_id)
        .await?
        .with_context(|| format!("Transação {transaction_id} não encontrada"))?;
    let raw = fs::read_to_string(payload_path)
        .with_context(|| format!("Falha ao ler payload {}", payload_path.display()))?;
    let payload = parse_split_payload(&raw)?;
    let preview = validate_split_payload(transaction_id, parent.amount, payload.clone())?;
    Ok((parent, payload, preview))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Auth { command } => match command {
            AuthCommand::Setup(args) => auth_setup(args).await,
        },
        Commands::Admin { command } => match command {
            AdminCommand::Migrate => admin_migrate().await,
            AdminCommand::ImportLegacy(args) => admin_import_legacy(args).await,
            AdminCommand::Reclassify(args) => admin_reclassify(args).await,
        },
        Commands::Sync { command } => match command {
            SyncCommand::Pluggy(args) => sync_pluggy_command(args).await,
        },
        Commands::Report { command } => match command {
            ReportCommand::DailyPulse(args) => report_daily_pulse(args).await,
            ReportCommand::MonthlySpend(args) => report_monthly_spend(args).await,
            ReportCommand::Cashflow(args) => report_cashflow(args).await,
            ReportCommand::ForecastVsActual(args) => report_forecast_vs_actual(args).await,
            ReportCommand::CardSummary(args) => report_card_summary(args).await,
            ReportCommand::CardClosedInsights(args) => report_card_closed_insights(args).await,
            ReportCommand::Uncategorized(args) => report_uncategorized(args).await,
            ReportCommand::SplitCandidates(args) => report_split_candidates(args).await,
            ReportCommand::ItemPrices(args) => report_item_prices(args).await,
            ReportCommand::DataHealth(args) => report_data_health(args).await,
            ReportCommand::Scenario(args) => report_scenario(args).await,
            ReportCommand::OfxConsistency(args) => report_ofx_consistency(args).await,
            ReportCommand::Review(args) => report_review(args).await,
        },
        Commands::Tx { command } => match command {
            TxCommand::UpsertManual(args) => tx_upsert_manual(args).await,
            TxCommand::Categorize(args) => tx_categorize(args).await,
            TxCommand::SetContext(args) => tx_set_context(args).await,
            TxCommand::ListContext(args) => tx_list_context(args).await,
            TxCommand::Split { command } => match command {
                TxSplitCommand::Preview(args) => tx_split_preview(args).await,
                TxSplitCommand::Apply(args) => tx_split_apply(args).await,
                TxSplitCommand::Show(args) => tx_split_show(args).await,
                TxSplitCommand::Clear(args) => tx_split_clear(args).await,
            },
        },
        Commands::Forecast { command } => match command {
            ForecastCommand::Upsert(args) => forecast_upsert(args).await,
        },
        Commands::Rule { command } => match command {
            RuleCommand::Upsert(args) => rule_upsert(args).await,
            RuleCommand::List(args) => rule_list(args).await,
            RuleCommand::Inspect(args) => rule_inspect(args).await,
        },
        Commands::Account { command } => match command {
            AccountCommand::Upsert(args) => account_upsert(args).await,
        },
    }
}

async fn auth_setup(args: AuthSetupArgs) -> Result<()> {
    let paths = ConfigPaths::discover()?;
    paths.ensure()?;
    let mut config = AppConfig::load(&paths)?;
    config.backend = args.backend.into();
    config.actor_id = args.actor_id;
    config.project_id = args.project_id;
    config.dataset_id = args.dataset_id;
    config.service_account_path = args.service_account_path;
    config.local_db_path = args.local_db_path.or(config.local_db_path.clone());
    config.pluggy_start_date = args.pluggy_start_date.or(config.pluggy_start_date.clone());
    config.save(&paths)?;

    println!("Configuração salva em {}", paths.config_file.display());
    println!("backend: {:?}", config.backend);
    println!("actor_id: {}", config.actor_id);
    if let Some(db_path) = &config.local_db_path {
        println!("local_db: {}", db_path.display());
    }
    Ok(())
}

async fn admin_migrate() -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let executed = run_migrations(store.as_ref(), &config).await?;
    if executed.is_empty() {
        println!("Nenhuma migration pendente.");
    } else {
        println!("Migrations aplicadas:");
        for version in executed {
            println!("- {version}");
        }
    }
    Ok(())
}

async fn admin_import_legacy(args: ImportLegacyArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let bundle = load_legacy_bundle(&args.finance_root, &config.actor_id)?;
    let mut audit = Vec::new();

    upsert_accounts_chunked(store.as_ref(), &bundle.accounts).await?;
    for row in &bundle.accounts {
        audit.push(AuditEvent::from_entity(
            "account",
            &row.account_id,
            "import_legacy",
            &config.actor_id,
            &row.idempotency_key,
            serde_json::to_value(row)?,
        ));
    }

    upsert_categories_chunked(store.as_ref(), &bundle.categories).await?;
    for row in &bundle.categories {
        audit.push(AuditEvent::from_entity(
            "category",
            &row.category_id,
            "import_legacy",
            &config.actor_id,
            &format!("category:{}", row.category_id),
            serde_json::to_value(row)?,
        ));
    }

    upsert_rules_chunked(store.as_ref(), &bundle.rules).await?;
    for row in &bundle.rules {
        audit.push(AuditEvent::from_entity(
            "rule",
            &row.rule_id,
            "import_legacy",
            &config.actor_id,
            &row.idempotency_key,
            serde_json::to_value(row)?,
        ));
    }

    upsert_forecasts_chunked(store.as_ref(), &bundle.forecasts).await?;
    for row in &bundle.forecasts {
        audit.push(AuditEvent::from_entity(
            "forecast",
            &row.forecast_id,
            "import_legacy",
            &config.actor_id,
            &row.idempotency_key,
            serde_json::to_value(row)?,
        ));
    }

    upsert_transactions_chunked(store.as_ref(), &bundle.transactions).await?;
    for row in &bundle.transactions {
        audit.push(AuditEvent::from_entity(
            "transaction",
            &row.transaction_id,
            "import_legacy",
            &config.actor_id,
            &row.idempotency_key,
            serde_json::to_value(row)?,
        ));
    }

    insert_audit_chunked(store.as_ref(), &audit).await?;

    println!("Import legado concluído:");
    println!("- accounts: {}", bundle.accounts.len());
    println!("- categories: {}", bundle.categories.len());
    println!("- rules: {}", bundle.rules.len());
    println!("- forecasts: {}", bundle.forecasts.len());
    println!("- transactions: {}", bundle.transactions.len());
    Ok(())
}

async fn admin_reclassify(args: ReclassifyArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let compiled_rules = compile_rules(&store.active_rules().await?)?;
    if compiled_rules.is_empty() {
        println!("Nenhuma regra ativa encontrada. Importe regras antes com `admin import-legacy`.");
        return Ok(());
    }

    let since = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    let items = store.daily_pulse(since).await?;
    println!("Transações encontradas: {}", items.len());
    println!("Regras compiladas: {}", compiled_rules.len());

    let mut changed = 0u64;
    let mut unchanged = 0u64;
    let mut audit = Vec::new();

    for item in &items {
        let rule_application = apply_rules_with_facts(
            &item.description,
            Some(item.amount),
            Some(item.transaction_id.as_str()),
            item.category_id.clone(),
            item.category_id.is_some(),
            &compiled_rules,
        );
        let new_category_id = rule_application.category_id;
        let new_source = rule_application.category_source;
        let new_context = rule_application.context;

        if new_source != "rule" {
            unchanged += 1;
            continue;
        }

        let cat_changed = new_category_id != item.category_id;
        if !cat_changed && new_context.is_none() {
            unchanged += 1;
            continue;
        }

        if args.dry_run {
            println!(
                "  [DRY-RUN] {} {:50} {:30} -> {}",
                item.transaction_date,
                &item.description[..item.description.len().min(50)],
                item.category_id.as_deref().unwrap_or("(nenhuma)"),
                new_category_id.as_deref().unwrap_or("(nenhuma)")
            );
        } else {
            let idem_key = format!("reclassify:{}", item.transaction_id);
            store
                .annotate_transaction(
                    &item.transaction_id,
                    new_category_id.as_deref(),
                    Some(&new_source),
                    new_context.as_deref(),
                    &config.actor_id,
                    &idem_key,
                )
                .await?;
            audit.push(AuditEvent::from_entity(
                "transaction",
                &item.transaction_id,
                "reclassify",
                &config.actor_id,
                &idem_key,
                serde_json::json!({
                    "old_category_id": item.category_id,
                    "new_category_id": new_category_id,
                    "source": new_source,
                    "new_context": new_context,
                }),
            ));
        }
        changed += 1;
    }

    if !audit.is_empty() {
        insert_audit_chunked(store.as_ref(), &audit).await?;
    }

    let mode = if args.dry_run { " (dry-run)" } else { "" };
    println!("Reclassificação concluída{mode}:");
    println!("- alteradas: {changed}");
    println!("- sem alteração: {unchanged}");
    Ok(())
}

async fn sync_pluggy_command(args: SyncPluggyArgs) -> Result<()> {
    if args.json_summary && args.notify_summary {
        bail!("Use apenas uma saída de resumo: --json-summary ou --notify-summary");
    }

    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let compiled_rules = compile_rules(&store.active_rules().await?)?;
    let internal_categories = store.internal_categories().await?;

    let effective_from = resolve_sync_from(
        args.from.as_deref(),
        config.pluggy_start_date.as_deref(),
        store.latest_pluggy_transaction_date().await?,
    )?;
    let to = args
        .to
        .unwrap_or_else(|| Utc::now().date_naive().format("%Y-%m-%d").to_string());
    let (accounts, transactions, rebinds) = sync_pluggy(SyncPluggyParams {
        actor_id: &config.actor_id,
        pluggy_config_path: &args.pluggy_config,
        accounts_csv_path: Some(&args.accounts_csv),
        fixture_path: args.fixture.as_deref(),
        from_override: Some(&effective_from),
        to_date: &to,
        rules: &compiled_rules,
        internal_categories: &internal_categories,
        api_base_url: None,
    })
    .await?;
    let existing_ids = store
        .existing_transaction_ids(
            &transactions
                .iter()
                .map(|row| row.transaction_id.clone())
                .collect::<Vec<_>>(),
        )
        .await?;
    let categories = build_category_records_from_transactions(&config.actor_id, &transactions);
    let mut audit = Vec::new();

    upsert_accounts_chunked(store.as_ref(), &accounts).await?;
    upsert_categories_chunked(store.as_ref(), &categories).await?;
    upsert_transactions_chunked(store.as_ref(), &transactions).await?;

    for rebind in &rebinds {
        audit.push(AuditEvent::from_entity(
            "account",
            &rebind.internal_account_id,
            "rebind",
            &config.actor_id,
            &format!(
                "rebind:{}:{}",
                rebind.binding_id, rebind.to_pluggy_account_id
            ),
            json!({
                "from": rebind.from_pluggy_account_id,
                "to": rebind.to_pluggy_account_id,
                "pluggy_item_id": rebind.pluggy_item_id,
            }),
        ));
    }
    for row in &accounts {
        audit.push(AuditEvent::from_entity(
            "account",
            &row.account_id,
            "sync_pluggy",
            &config.actor_id,
            &row.idempotency_key,
            serde_json::to_value(row)?,
        ));
    }
    for row in &transactions {
        audit.push(AuditEvent::from_entity(
            "transaction",
            &row.transaction_id,
            "sync_pluggy",
            &config.actor_id,
            &row.idempotency_key,
            serde_json::to_value(row)?,
        ));
    }
    insert_audit_chunked(store.as_ref(), &audit).await?;

    let account_labels = accounts
        .iter()
        .map(|row| (row.account_id.clone(), row.label.clone()))
        .collect::<BTreeMap<_, _>>();
    let new_transactions = transactions
        .iter()
        .filter(|row| !existing_ids.contains(&row.transaction_id))
        .map(|row| SyncSummaryTransaction {
            transaction_id: row.transaction_id.clone(),
            transaction_date: row.transaction_date.format("%Y-%m-%d").to_string(),
            day_of_week: day_of_week_text(row.transaction_date),
            description: row.description.clone(),
            amount: decimal_text(row.amount),
            tx_type: row.tx_type.clone(),
            category_id: row.category_id.clone(),
            category_source: row.category_source.clone(),
            context: row.context.clone(),
            account_id: row.account_id.clone(),
            account_label: row
                .account_id
                .as_ref()
                .and_then(|account_id| account_labels.get(account_id).cloned()),
            payment_status: row.payment_status.clone(),
            source: row.source.clone(),
            metadata_json: row.metadata_json.clone(),
        })
        .collect::<Vec<_>>();
    if args.json_summary || args.notify_summary {
        let pending_summary = load_sync_pending_summary(
            store.as_ref(),
            &transactions,
            &existing_ids,
            &account_labels,
        )
        .await;
        let summary = SyncSummaryOutput {
            actor_id: config.actor_id.clone(),
            backend: format!("{:?}", config.effective_backend()).to_lowercase(),
            generated_at: Utc::now().to_rfc3339(),
            summary_status: pending_summary.summary_status,
            warnings: pending_summary.warnings,
            new_transactions_count: new_transactions.len(),
            needs_context_count: pending_summary.needs_context_count,
            needs_context_returned_count: pending_summary.needs_context_returned_count,
            needs_context_truncated: pending_summary.needs_context_truncated,
            new_transactions,
            needs_context: pending_summary.needs_context,
        };
        if args.json_summary {
            println!("{}", serde_json::to_string_pretty(&summary)?);
        } else {
            println!("{}", render_sync_notify_summary(&summary));
        }
        return Ok(());
    }

    println!("Sync Pluggy concluído:");
    println!("- accounts: {}", accounts.len());
    println!("- transactions: {}", transactions.len());
    println!("- categories: {}", categories.len());
    println!("- actor: {}", config.actor_id);
    println!("- backend: {:?}", config.effective_backend());
    Ok(())
}

fn resolve_sync_from(
    explicit_from: Option<&str>,
    configured_start: Option<&str>,
    latest_seen: Option<NaiveDate>,
) -> Result<String> {
    if let Some(value) = explicit_from {
        return Ok(value.to_string());
    }

    let configured_start_date = configured_start
        .map(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .with_context(|| format!("Falha ao parsear pluggy_start_date {value}"))
        })
        .transpose()?;

    let computed = latest_seen
        .and_then(|latest| latest.checked_sub_signed(Duration::days(DEFAULT_SYNC_LOOKBACK_DAYS)))
        .or(configured_start_date)
        .unwrap_or_else(|| Utc::now().date_naive() - Duration::days(DEFAULT_SYNC_LOOKBACK_DAYS));

    Ok(configured_start_date
        .map(|start| start.max(computed))
        .unwrap_or(computed)
        .format("%Y-%m-%d")
        .to_string())
}

async fn report_daily_pulse(args: DailyPulseArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let since = Utc::now()
        .date_naive()
        .checked_sub_signed(Duration::days(args.days.saturating_sub(1)))
        .context("Falha ao calcular janela do daily pulse")?;
    let items = store.daily_pulse(since).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    let internal_categories = store.internal_categories().await?;
    let is_internal = |cat: &Option<String>| {
        cat.as_deref()
            .is_some_and(|c| internal_categories.contains(c))
    };
    let income = items
        .iter()
        .filter(|item| !item.amount.is_sign_negative() && !is_internal(&item.category_id))
        .fold(Decimal::ZERO, |acc, item| acc + item.amount);
    let expenses = items
        .iter()
        .filter(|item| item.amount.is_sign_negative() && !is_internal(&item.category_id))
        .fold(Decimal::ZERO, |acc, item| acc + item.amount);

    println!("📊 Daily pulse desde {}", since.format("%Y-%m-%d"));
    println!("- linhas: {}", items.len());
    println!("- entradas: {}", brl(income));
    println!("- saídas: {}", brl(expenses));
    println!();

    for item in items {
        let category = category_display(item.category_id.as_deref(), Some(item.amount));
        let account = item.account_id.unwrap_or_else(|| "sem-conta".to_string());
        println!(
            "{} | {} | {} | {} | 🏦 {} | {}",
            item.transaction_date.format("%Y-%m-%d"),
            brl(item.amount),
            normalize_inline_text(&item.description),
            category,
            account,
            item.payment_status
        );
    }
    Ok(())
}

async fn report_monthly_spend(args: MonthlySpendArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.monthly_spend(args.month.as_deref()).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "🧾 Monthly spend{}",
        args.month
            .as_deref()
            .map(|value| format!(" {value}"))
            .unwrap_or_default()
    );
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        let category = category_display(Some(&row.category_id), Some(-row.expenses));
        println!(
            "{} | {} | 🏦 {} | {} | {} transações",
            row.month_ref,
            category,
            row.account_id,
            brl(-row.expenses),
            row.expense_count
        );
    }
    Ok(())
}

async fn report_cashflow(args: CashflowArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.cashflow(args.months).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("💹 Cashflow últimos {} meses", args.months);
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        println!(
            "{} | entradas {} | saídas {} | líquido {}",
            row.month_ref,
            brl(row.income),
            brl(-row.expenses),
            brl(row.net)
        );
    }
    Ok(())
}

async fn report_forecast_vs_actual(args: ForecastVsActualArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.forecast_vs_actual(args.month.as_deref()).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "🧭 Forecast vs actual{}",
        args.month
            .as_deref()
            .map(|value| format!(" {value}"))
            .unwrap_or_default()
    );
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        let due_date = row
            .due_date
            .map(|value| value.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "sem-data".to_string());
        let account = row.account_id.unwrap_or_else(|| "sem-conta".to_string());
        let category = category_display(row.category_id.as_deref(), Some(-row.actual_amount));
        println!(
            "{} | {} | {} | 🏦 {} | previsto {} | realizado {} | variação {} | {}",
            row.month_ref,
            due_date,
            row.description,
            account,
            brl(-row.forecast_amount),
            brl(-row.actual_amount),
            brl(-row.variance),
            category
        );
    }
    Ok(())
}

async fn report_card_summary(args: CardSummaryArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.card_summary(args.month.as_deref()).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "💳 Card summary{}",
        args.month
            .as_deref()
            .map(|value| format!(" {value}"))
            .unwrap_or_default()
    );
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        println!(
            "{} | 💳 {} | total {} | em aberto {} | {} transações",
            row.month_ref,
            row.account_id,
            brl(-row.total_charges),
            brl(-row.open_amount),
            row.transaction_count
        );
    }
    Ok(())
}

async fn report_card_closed_insights(args: CardClosedInsightsArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let today = Utc::now().date_naive();
    let target_month = match args.month.as_deref() {
        Some(value) => {
            parse_month_ref(value)?;
            value.to_string()
        }
        None => default_closed_cards_month(today)?,
    };
    let target_month_date = parse_month_ref(&target_month)?;

    let card_rows = store.card_summary(Some(&target_month)).await?;
    let target_closed_rows = store.card_closed_transactions(Some(&target_month)).await?;
    let target_reportable_rows = store
        .card_reportable_transactions(Some(&target_month))
        .await?;

    let mut month_rows = BTreeMap::<String, Vec<CardClosedTransactionRow>>::new();
    month_rows.insert(target_month.clone(), target_closed_rows.clone());
    for delta in 1..=2 {
        let month = month_ref_for(shift_month(target_month_date, -delta)?);
        let rows = store.card_closed_transactions(Some(&month)).await?;
        month_rows.insert(month, rows);
    }

    let mut merchant_months = BTreeMap::<String, BTreeSet<String>>::new();
    for (month, rows) in &month_rows {
        for row in rows {
            merchant_months
                .entry(merchant_key_for_card_row(row))
                .or_default()
                .insert(month.clone());
        }
    }

    let mut closed_count_by_account = BTreeMap::<String, usize>::new();
    let mut categories = BTreeMap::<String, (Decimal, usize)>::new();
    let mut recurring = BTreeMap::<String, (Decimal, usize)>::new();
    let mut subscriptions = BTreeMap::<String, (Decimal, usize)>::new();
    let mut closed_installments = BTreeMap::<(String, String), (Decimal, usize)>::new();
    let mut open_installments = BTreeMap::<(String, String), (Decimal, usize)>::new();

    for row in &target_closed_rows {
        *closed_count_by_account
            .entry(row.account_id.clone())
            .or_insert(0) += 1;
        let category = row
            .category_id
            .clone()
            .unwrap_or_else(|| "sem-categoria".to_string());
        let category_entry = categories.entry(category).or_insert((Decimal::ZERO, 0));
        category_entry.0 += row.amount;
        category_entry.1 += 1;

        let merchant = merchant_key_for_card_row(row);
        if merchant_months
            .get(&merchant)
            .is_some_and(|months| months.len() >= 2)
        {
            let recurring_entry = recurring
                .entry(merchant.clone())
                .or_insert((Decimal::ZERO, 0));
            recurring_entry.0 += row.amount;
            recurring_entry.1 += 1;
        }

        if is_subscription_row(row) {
            let subscription_entry = subscriptions
                .entry(merchant.clone())
                .or_insert((Decimal::ZERO, 0));
            subscription_entry.0 += row.amount;
            subscription_entry.1 += 1;
        }

        if let Some(marker) = detect_installment_marker(row) {
            let installment_entry = closed_installments
                .entry((merchant, marker))
                .or_insert((Decimal::ZERO, 0));
            installment_entry.0 += row.amount;
            installment_entry.1 += 1;
        }
    }

    for row in &target_reportable_rows {
        if !is_open_card_payment_status(&row.payment_status) {
            continue;
        }
        if let Some(marker) = detect_installment_marker(row) {
            let merchant = merchant_key_for_card_row(row);
            let installment_entry = open_installments
                .entry((merchant, marker))
                .or_insert((Decimal::ZERO, 0));
            installment_entry.0 += row.amount;
            installment_entry.1 += 1;
        }
    }

    let mut accounts = card_rows
        .into_iter()
        .map(|row| CardClosedAccountInsight {
            account_id: row.account_id.clone(),
            total_charges: row.total_charges,
            open_amount: row.open_amount,
            closed_amount: row.total_charges - row.open_amount,
            closed_transactions: closed_count_by_account
                .get(&row.account_id)
                .copied()
                .unwrap_or(0),
        })
        .collect::<Vec<_>>();
    accounts.sort_by(|a, b| {
        b.closed_amount
            .cmp(&a.closed_amount)
            .then_with(|| a.account_id.cmp(&b.account_id))
    });

    let mut category_rows = categories
        .into_iter()
        .map(
            |(category_id, (amount, transactions))| CardClosedCategoryInsight {
                category_id,
                amount,
                transactions,
            },
        )
        .collect::<Vec<_>>();
    category_rows.sort_by(|a, b| {
        b.amount
            .cmp(&a.amount)
            .then_with(|| a.category_id.cmp(&b.category_id))
    });

    let mut recurring_rows = recurring
        .into_iter()
        .map(
            |(merchant_key, (amount, transactions))| CardClosedRecurringInsight {
                months_detected: merchant_months
                    .get(&merchant_key)
                    .map(|months| months.len())
                    .unwrap_or(1),
                merchant_key,
                amount,
                transactions,
            },
        )
        .collect::<Vec<_>>();
    recurring_rows.sort_by(|a, b| {
        b.amount
            .cmp(&a.amount)
            .then_with(|| a.merchant_key.cmp(&b.merchant_key))
    });

    let mut subscription_rows = subscriptions
        .into_iter()
        .map(
            |(merchant_key, (amount, transactions))| CardClosedSubscriptionInsight {
                merchant_key,
                amount,
                transactions,
            },
        )
        .collect::<Vec<_>>();
    subscription_rows.sort_by(|a, b| {
        b.amount
            .cmp(&a.amount)
            .then_with(|| a.merchant_key.cmp(&b.merchant_key))
    });

    let mut closed_installment_rows = closed_installments
        .into_iter()
        .map(
            |((merchant_key, marker), (amount, transactions))| CardClosedInstallmentInsight {
                merchant_key,
                marker,
                amount,
                transactions,
            },
        )
        .collect::<Vec<_>>();
    closed_installment_rows.sort_by(|a, b| {
        b.amount
            .cmp(&a.amount)
            .then_with(|| a.merchant_key.cmp(&b.merchant_key))
            .then_with(|| a.marker.cmp(&b.marker))
    });

    let mut open_installment_rows = open_installments
        .into_iter()
        .map(
            |((merchant_key, marker), (amount, transactions))| CardClosedInstallmentInsight {
                merchant_key,
                marker,
                amount,
                transactions,
            },
        )
        .collect::<Vec<_>>();
    open_installment_rows.sort_by(|a, b| {
        b.amount
            .cmp(&a.amount)
            .then_with(|| a.merchant_key.cmp(&b.merchant_key))
            .then_with(|| a.marker.cmp(&b.marker))
    });

    let output = CardClosedInsightsOutput {
        month_ref: target_month,
        accounts,
        categories: category_rows,
        recurring: recurring_rows,
        subscriptions: subscription_rows,
        closed_installments: closed_installment_rows,
        open_installments: open_installment_rows,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("💳 Card closed insights {}", output.month_ref);
    println!("- contas: {}", output.accounts.len());
    println!("- transações fechadas: {}", target_closed_rows.len());
    println!();

    if output.accounts.is_empty() {
        println!("Sem cartões com movimentos no mês.");
        return Ok(());
    }

    println!("Fechamento por cartão:");
    for row in &output.accounts {
        println!(
            "{} | fechado {} | total {} | em aberto {} | {} transações fechadas",
            row.account_id,
            brl(-row.closed_amount),
            brl(-row.total_charges),
            brl(-row.open_amount),
            row.closed_transactions
        );
    }

    println!();
    println!("Top categorias (fatura fechada):");
    if output.categories.is_empty() {
        println!("Sem categorias detectadas.");
    } else {
        for row in output.categories.iter().take(8) {
            println!(
                "{} | {} | {} transações",
                category_display(Some(&row.category_id), Some(-row.amount)),
                brl(-row.amount),
                row.transactions
            );
        }
    }

    println!();
    println!("Recorrentes:");
    if output.recurring.is_empty() {
        println!("Sem recorrentes detectados.");
    } else {
        for row in output.recurring.iter().take(8) {
            println!(
                "{} | {} | {} transações | {} meses",
                row.merchant_key,
                brl(-row.amount),
                row.transactions,
                row.months_detected
            );
        }
    }

    println!();
    println!("Assinaturas:");
    if output.subscriptions.is_empty() {
        println!("Sem assinaturas detectadas.");
    } else {
        for row in output.subscriptions.iter().take(8) {
            println!(
                "{} | {} | {} transações",
                row.merchant_key,
                brl(-row.amount),
                row.transactions
            );
        }
    }

    println!();
    println!("Parceladas fechadas:");
    if output.closed_installments.is_empty() {
        println!("Sem parceladas fechadas detectadas.");
    } else {
        for row in output.closed_installments.iter().take(8) {
            println!(
                "{} | {} | {} | {} transações",
                row.merchant_key,
                row.marker,
                brl(-row.amount),
                row.transactions
            );
        }
    }

    println!();
    println!("Parceladas em aberto:");
    if output.open_installments.is_empty() {
        println!("Sem parceladas em aberto detectadas.");
    } else {
        for row in output.open_installments.iter().take(8) {
            println!(
                "{} | {} | {} | {} transações",
                row.merchant_key,
                row.marker,
                brl(-row.amount),
                row.transactions
            );
        }
    }

    Ok(())
}

async fn report_ofx_consistency(args: OfxConsistencyArgs) -> Result<()> {
    if args.date_tolerance_days < 0 {
        bail!("date_tolerance_days deve ser >= 0");
    }

    let ofx_raw = fs::read_to_string(&args.ofx)
        .with_context(|| format!("Falha ao ler OFX {}", args.ofx.display()))?;
    let mut ofx_rows = parse_ofx_transactions(&ofx_raw)?;
    if ofx_rows.is_empty() {
        bail!("Nenhuma transação <STMTTRN> encontrada no OFX");
    }
    ofx_rows.sort_by(|a, b| {
        a.transaction_date
            .cmp(&b.transaction_date)
            .then_with(|| a.amount.cmp(&b.amount))
            .then_with(|| a.description.cmp(&b.description))
    });

    let range_start = ofx_rows
        .iter()
        .map(|row| row.transaction_date)
        .min()
        .context("OFX sem data mínima")?;
    let range_end = ofx_rows
        .iter()
        .map(|row| row.transaction_date)
        .max()
        .context("OFX sem data máxima")?;
    let query_since = range_start
        .checked_sub_signed(Duration::days(args.date_tolerance_days))
        .context("Falha ao calcular início da janela de busca")?;
    let query_until = range_end
        .checked_add_signed(Duration::days(args.date_tolerance_days))
        .context("Falha ao calcular fim da janela de busca")?;

    let ofx_account_ids = parse_ofx_tag_values(&ofx_raw, "ACCTID")
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<BTreeSet<_>>();

    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let mut db_rows = store
        .effective_transactions_window(query_since, query_until)
        .await?;

    let inferred_account_id = infer_account_id_from_fit_ids(&ofx_rows, &db_rows)
        .or_else(|| infer_account_id_from_overlap(&ofx_rows, &db_rows, args.date_tolerance_days));
    let account_filter = if let Some(account_id) = args.account_id.as_deref() {
        db_rows.retain(|row| row.account_id.as_deref() == Some(account_id));
        Some(format!("account_id:{account_id}"))
    } else if let Some(account_id) = inferred_account_id.as_deref() {
        db_rows.retain(|row| row.account_id.as_deref() == Some(account_id));
        Some(format!("inferred_account_id:{account_id}"))
    } else if !ofx_account_ids.is_empty() {
        let filtered = db_rows
            .iter()
            .filter(|row| {
                metadata_pluggy_account_id(row)
                    .is_some_and(|pluggy_id| ofx_account_ids.contains(pluggy_id))
            })
            .cloned()
            .collect::<Vec<_>>();
        if !filtered.is_empty() {
            db_rows = filtered;
            Some(format!(
                "pluggy_account_id:{}",
                ofx_account_ids
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(",")
            ))
        } else {
            None
        }
    } else {
        None
    };

    db_rows.sort_by(|a, b| {
        a.transaction_date
            .cmp(&b.transaction_date)
            .then_with(|| a.amount.cmp(&b.amount))
            .then_with(|| a.transaction_id.cmp(&b.transaction_id))
    });

    let mut used_rows = vec![false; db_rows.len()];
    let mut matches = Vec::with_capacity(ofx_rows.len());
    let mut matched = 0usize;
    let mut missing = 0usize;

    for ofx_row in &ofx_rows {
        if let Some((best_idx, amount_diff, date_diff, desc_distance)) =
            find_best_ofx_match(ofx_row, &db_rows, &used_rows, args.date_tolerance_days)
        {
            used_rows[best_idx] = true;
            matched += 1;
            let db_row = &db_rows[best_idx];
            let status = if amount_diff == Decimal::ZERO && date_diff == 0 && desc_distance <= 1 {
                "matched"
            } else {
                "matched-near"
            };
            matches.push(OfxConsistencyMatch {
                status: status.to_string(),
                ofx_fit_id: ofx_row.fit_id.clone(),
                ofx_date: ofx_row.transaction_date.format("%Y-%m-%d").to_string(),
                ofx_amount: ofx_row.amount,
                ofx_description: ofx_row.description.clone(),
                transaction_id: Some(db_row.transaction_id.clone()),
                account_id: db_row.account_id.clone(),
                transaction_date: Some(db_row.transaction_date.format("%Y-%m-%d").to_string()),
                transaction_amount: Some(db_row.amount),
                transaction_description: Some(normalize_inline_text(&db_row.description)),
                date_diff_days: Some(date_diff),
            });
        } else {
            missing += 1;
            matches.push(OfxConsistencyMatch {
                status: "missing-in-finance".to_string(),
                ofx_fit_id: ofx_row.fit_id.clone(),
                ofx_date: ofx_row.transaction_date.format("%Y-%m-%d").to_string(),
                ofx_amount: ofx_row.amount,
                ofx_description: ofx_row.description.clone(),
                transaction_id: None,
                account_id: None,
                transaction_date: None,
                transaction_amount: None,
                transaction_description: None,
                date_diff_days: None,
            });
        }
    }

    let extra_transactions = db_rows
        .iter()
        .zip(used_rows.iter())
        .filter_map(|(row, used)| {
            if *used {
                None
            } else {
                Some(OfxConsistencyDbOnly {
                    transaction_id: row.transaction_id.clone(),
                    transaction_date: row.transaction_date.format("%Y-%m-%d").to_string(),
                    amount: row.amount,
                    description: normalize_inline_text(&row.description),
                    account_id: row.account_id.clone(),
                })
            }
        })
        .collect::<Vec<_>>();

    let output = OfxConsistencyOutput {
        ofx_path: args.ofx.display().to_string(),
        account_filter,
        range_start: range_start.format("%Y-%m-%d").to_string(),
        range_end: range_end.format("%Y-%m-%d").to_string(),
        date_tolerance_days: args.date_tolerance_days,
        ofx_transactions: ofx_rows.len(),
        db_candidates: db_rows.len(),
        matched,
        missing_in_finance: missing,
        extra_in_finance: extra_transactions.len(),
        consistent: missing == 0 && extra_transactions.is_empty(),
        matches,
        extra_transactions,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("🧪 OFX consistency check");
    println!("- arquivo: {}", output.ofx_path);
    println!(
        "- período OFX: {} -> {}",
        output.range_start, output.range_end
    );
    if let Some(filter) = &output.account_filter {
        println!("- filtro de conta: {filter}");
    }
    println!("- tolerância de data: {} dias", output.date_tolerance_days);
    println!("- transações OFX: {}", output.ofx_transactions);
    println!("- transações base (janela): {}", output.db_candidates);
    println!("- casadas: {}", output.matched);
    println!("- faltando na base: {}", output.missing_in_finance);
    println!("- sobrando na base: {}", output.extra_in_finance);
    println!(
        "- consistente: {}",
        if output.consistent { "sim" } else { "não" }
    );

    println!();
    println!("Comparação transação a transação:");
    for row in &output.matches {
        match row.status.as_str() {
            "matched" => {
                println!(
                    "✅ {} | {} | {} | {}",
                    row.ofx_date,
                    brl(row.ofx_amount),
                    row.ofx_description,
                    row.transaction_id.as_deref().unwrap_or("sem-id")
                );
            }
            "matched-near" => {
                println!(
                    "⚠️ {} | {} | {} | match aproximado {} | delta-data {}d",
                    row.ofx_date,
                    brl(row.ofx_amount),
                    row.ofx_description,
                    row.transaction_id.as_deref().unwrap_or("sem-id"),
                    row.date_diff_days.unwrap_or(0)
                );
            }
            _ => {
                println!(
                    "❌ {} | {} | {} | sem match",
                    row.ofx_date,
                    brl(row.ofx_amount),
                    row.ofx_description
                );
            }
        }
    }

    println!();
    println!("Extras na base (sem par no OFX):");
    if output.extra_transactions.is_empty() {
        println!("Nenhum extra detectado.");
    } else {
        for row in &output.extra_transactions {
            println!(
                "{} | {} | {} | {}",
                row.transaction_date,
                brl(row.amount),
                row.description,
                row.transaction_id
            );
        }
    }

    Ok(())
}

async fn report_uncategorized(args: UncategorizedArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.uncategorized(args.limit).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("❗ Uncategorized");
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        let account = row.account_id.unwrap_or_else(|| "sem-conta".to_string());
        println!(
            "{} | {} | {} | 🏦 {} | {} | {}",
            row.transaction_date.format("%Y-%m-%d"),
            brl(row.amount),
            normalize_inline_text(&row.description),
            account,
            row.payment_status,
            row.category_source
        );
    }
    Ok(())
}

async fn report_split_candidates(args: SplitCandidatesArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    ensure_bigquery_split_backend(&config)?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let since = Utc::now()
        .date_naive()
        .checked_sub_signed(Duration::days(args.days.saturating_sub(1)))
        .context("Falha ao calcular janela de split candidates")?;
    let rows = store.split_candidates(since).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("Split candidates desde {}", since.format("%Y-%m-%d"));
    println!("- linhas: {}", rows.len());
    println!();
    for row in rows {
        println!(
            "{} | {} | {} | {} | policy {} ({})",
            row.transaction_date.format("%Y-%m-%d"),
            brl(row.amount),
            row.transaction_id,
            normalize_inline_text(&row.description),
            row.policy_id,
            row.match_type
        );
    }
    Ok(())
}

async fn report_item_prices(args: ItemPricesArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    ensure_bigquery_split_backend(&config)?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let since = args
        .since
        .as_deref()
        .map(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
        .transpose()
        .context("since inválido; use YYYY-MM-DD")?;
    let rows = store.item_prices(&args.query, since).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("Item prices {}", args.query);
    println!("- linhas: {}", rows.len());
    println!();
    for row in rows {
        println!(
            "{} | {} | qty {} {} | unit {} | total {} | {}",
            row.transaction_date.format("%Y-%m-%d"),
            normalize_inline_text(&row.description),
            row.quantity
                .map(decimal_text)
                .unwrap_or_else(|| "-".to_string()),
            row.unit.unwrap_or_default(),
            row.unit_price
                .map(decimal_text)
                .unwrap_or_else(|| "-".to_string()),
            row.total_price
                .map(decimal_text)
                .unwrap_or_else(|| "-".to_string()),
            row.store_name.unwrap_or_else(|| "sem-loja".to_string())
        );
    }
    Ok(())
}

async fn report_data_health(args: DataHealthArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let today = Utc::now().date_naive();
    let window_start = today
        .checked_sub_signed(Duration::days(args.days.saturating_sub(1)))
        .context("Falha ao calcular janela do data-health")?;
    let items = store.daily_pulse(window_start).await?;
    let latest_pluggy_date = store.latest_pluggy_transaction_date().await?;
    let total_transactions = store.count_rows("transactions").await?;
    let transactions_with_context = store.count_transactions_with_context().await?;
    let uncategorized_count = store.count_uncategorized().await?;
    let active_rules_count = store.active_rules().await?.len();

    let mut flat_counts = BTreeMap::<String, usize>::new();
    let mut overlap_groups = BTreeMap::<(Option<String>, String), Vec<_>>::new();
    let mut window_pluggy_rows = 0usize;
    let mut window_legacy_rows = 0usize;
    let mut categorized_rows = 0usize;

    for item in &items {
        match item.source.as_str() {
            "pluggy" => window_pluggy_rows += 1,
            "legacy" => window_legacy_rows += 1,
            _ => {}
        }
        if let Some(category_id) = item.category_id.as_deref() {
            categorized_rows += 1;
            if is_flat_category(category_id) {
                *flat_counts.entry(category_id.to_string()).or_insert(0) += 1;
            }
        }
        overlap_groups
            .entry((item.account_id.clone(), decimal_text(item.amount.abs())))
            .or_default()
            .push(item);
    }

    let flat_category_rows = flat_counts.values().copied().sum::<usize>();
    let mut flat_categories = flat_counts
        .into_iter()
        .map(|(category_id, count)| FlatCategoryCount { category_id, count })
        .collect::<Vec<_>>();
    flat_categories.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.category_id.cmp(&b.category_id))
    });
    let window_other_rows = items
        .len()
        .saturating_sub(window_pluggy_rows + window_legacy_rows);
    let context_coverage_ratio = if total_transactions > 0 {
        transactions_with_context as f64 / total_transactions as f64
    } else {
        0.0
    };
    let mut overlap_candidates = Vec::new();
    for ((account_id, amount), rows) in overlap_groups {
        let legacy_rows = rows
            .iter()
            .filter(|item| item.source == "legacy" && item.amount.is_sign_negative())
            .copied()
            .collect::<Vec<_>>();
        let pluggy_rows = rows
            .iter()
            .filter(|item| item.source == "pluggy" && item.amount.is_sign_negative())
            .copied()
            .collect::<Vec<_>>();
        for legacy in legacy_rows {
            if let Some(pluggy) = pluggy_rows.iter().copied().find(|pluggy| {
                (legacy.transaction_date - pluggy.transaction_date)
                    .num_days()
                    .abs()
                    <= 7
                    && descriptions_look_related(&legacy.description, &pluggy.description)
            }) {
                overlap_candidates.push(OverlapCandidate {
                    account_id: account_id.clone(),
                    amount: amount.clone(),
                    legacy_transaction_id: legacy.transaction_id.clone(),
                    legacy_date: legacy.transaction_date.format("%Y-%m-%d").to_string(),
                    legacy_description: legacy.description.clone(),
                    pluggy_transaction_id: pluggy.transaction_id.clone(),
                    pluggy_date: pluggy.transaction_date.format("%Y-%m-%d").to_string(),
                    pluggy_description: pluggy.description.clone(),
                });
            }
        }
    }
    overlap_candidates.sort_by(|a, b| {
        b.legacy_date
            .cmp(&a.legacy_date)
            .then_with(|| a.account_id.cmp(&b.account_id))
            .then_with(|| a.amount.cmp(&b.amount))
    });
    let overlap_candidates_count = overlap_candidates.len();
    overlap_candidates.truncate(10);

    let output = DataHealthOutput {
        actor_id: config.actor_id.clone(),
        backend: format!("{:?}", config.effective_backend()).to_lowercase(),
        generated_at: Utc::now().to_rfc3339(),
        window_start: window_start.format("%Y-%m-%d").to_string(),
        window_days: args.days,
        latest_pluggy_transaction_date: latest_pluggy_date
            .map(|value| value.format("%Y-%m-%d").to_string()),
        pluggy_lag_days: latest_pluggy_date.map(|value| (today - value).num_days()),
        total_transactions,
        transactions_with_context,
        context_coverage_ratio,
        uncategorized_count,
        active_rules_count,
        window_rows: items.len(),
        window_pluggy_rows,
        window_legacy_rows,
        window_other_rows,
        categorized_rows,
        flat_category_rows,
        overlap_candidates_count,
        overlap_candidates,
        flat_categories,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("Data health");
    println!("- backend: {}", output.backend);
    println!("- actor_id: {}", output.actor_id);
    println!(
        "- janela: {} até hoje ({} dias)",
        output.window_start, output.window_days
    );
    match (
        &output.latest_pluggy_transaction_date,
        output.pluggy_lag_days,
    ) {
        (Some(date), Some(days)) => {
            println!("- último lançamento Pluggy: {} (lag {} dias)", date, days)
        }
        _ => println!("- último lançamento Pluggy: sem dado"),
    }
    println!("- transações totais: {}", output.total_transactions);
    println!(
        "- com contexto: {} ({:.1}%)",
        output.transactions_with_context,
        output.context_coverage_ratio * 100.0
    );
    println!("- uncategorized: {}", output.uncategorized_count);
    println!("- regras ativas: {}", output.active_rules_count);
    println!(
        "- janela: {} linhas | pluggy {} | legacy {} | outros {}",
        output.window_rows,
        output.window_pluggy_rows,
        output.window_legacy_rows,
        output.window_other_rows
    );
    println!(
        "- categorias planas na janela: {} linhas",
        output.flat_category_rows
    );
    println!(
        "- possíveis sobreposições legacy x pluggy: {}",
        output.overlap_candidates_count
    );
    if output.flat_categories.is_empty() {
        println!();
        println!("Nenhuma categoria plana detectada na janela.");
    } else {
        println!();
        println!("Top categorias planas:");
        for row in output.flat_categories.iter().take(10) {
            println!("{} | {} linhas", row.category_id, row.count);
        }
    }
    if !output.overlap_candidates.is_empty() {
        println!();
        println!("Possíveis sobreposições:");
        for row in &output.overlap_candidates {
            println!(
                "{} | {} | legacy {} '{}' | pluggy {} '{}'",
                row.account_id.as_deref().unwrap_or("sem-conta"),
                row.amount,
                row.legacy_date,
                row.legacy_description,
                row.pluggy_date,
                row.pluggy_description
            );
        }
    }
    Ok(())
}

async fn report_scenario(args: ScenarioArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let today = Utc::now().date_naive();
    let current_month = month_ref_for(
        NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
            .context("Falha ao calcular o mês atual")?,
    );
    let target_month = match args.month.as_deref() {
        Some(value) => {
            parse_month_ref(value)?;
            value.to_string()
        }
        None => default_scenario_month(today)?,
    };
    let history_months = args.history_months.max(1);
    let extra_expense = decimal_from_str(&args.extra_expense)?;
    let extra_income = decimal_from_str(&args.extra_income)?;

    let mut cashflow_rows = store.cashflow(history_months.saturating_add(12)).await?;
    cashflow_rows.sort_by(|a, b| b.month_ref.cmp(&a.month_ref));
    let baseline_rows = cashflow_rows
        .into_iter()
        .filter(|row| row.month_ref < target_month)
        .filter(|row| !(target_month > current_month && row.month_ref == current_month))
        .take(history_months)
        .collect::<Vec<_>>();
    if baseline_rows.is_empty() {
        bail!(
            "Não há meses históricos suficientes antes de {} para calcular cenário",
            target_month
        );
    }

    let divisor = Decimal::from(baseline_rows.len() as i64);
    let avg_income = baseline_rows
        .iter()
        .fold(Decimal::ZERO, |acc, row| acc + row.income)
        / divisor;
    let avg_expenses = baseline_rows
        .iter()
        .fold(Decimal::ZERO, |acc, row| acc + row.expenses)
        / divisor;
    let avg_net = baseline_rows
        .iter()
        .fold(Decimal::ZERO, |acc, row| acc + row.net)
        / divisor;

    let forecast_rows = store.forecast_vs_actual(Some(&target_month)).await?;
    let known_forecast_expenses = forecast_rows
        .iter()
        .fold(Decimal::ZERO, |acc, row| acc + row.forecast_amount);
    let planning_expenses = if known_forecast_expenses > avg_expenses {
        known_forecast_expenses
    } else {
        avg_expenses
    };

    let carryover_open_card_month =
        month_ref_for(shift_month(parse_month_ref(&target_month)?, -1)?);
    let carryover_open_card_amount = store
        .card_summary(Some(&carryover_open_card_month))
        .await?
        .into_iter()
        .fold(Decimal::ZERO, |acc, row| acc + row.open_amount);

    let projected_net = avg_income - planning_expenses + extra_income - extra_expense;
    let projected_cash_after_card_carry = projected_net - carryover_open_card_amount;

    let output = ScenarioOutput {
        target_month,
        baseline_months: baseline_rows
            .iter()
            .map(|row| row.month_ref.clone())
            .collect(),
        avg_income,
        avg_expenses,
        avg_net,
        planning_expenses,
        known_forecast_expenses,
        known_forecast_count: forecast_rows.len(),
        carryover_open_card_month,
        carryover_open_card_amount,
        extra_income,
        extra_expense,
        projected_net,
        projected_cash_after_card_carry,
        looks_ok_after_card_carry: projected_cash_after_card_carry >= Decimal::ZERO,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("Scenario {}", output.target_month);
    println!("- baseline meses: {}", output.baseline_months.join(", "));
    println!("- média entradas: {}", brl(output.avg_income));
    println!("- média saídas: {}", brl(-output.avg_expenses));
    println!("- média líquida: {}", brl(output.avg_net));
    println!(
        "- despesas de planejamento: {}",
        brl(-output.planning_expenses)
    );
    println!(
        "- forecast conhecido: {} em {} itens",
        brl(-output.known_forecast_expenses),
        output.known_forecast_count
    );
    println!(
        "- faturas em aberto carregadas de {}: {}",
        output.carryover_open_card_month,
        brl(-output.carryover_open_card_amount)
    );
    println!("- ajuste manual de receita: {}", brl(output.extra_income));
    println!("- ajuste manual de despesa: {}", brl(-output.extra_expense));
    println!("- líquido projetado: {}", brl(output.projected_net));
    println!(
        "- caixa após carregar cartões em aberto: {}",
        brl(output.projected_cash_after_card_carry)
    );
    println!(
        "- parecer: {}",
        if output.looks_ok_after_card_carry {
            "fica positivo"
        } else {
            "fica pressionado / negativo"
        }
    );
    Ok(())
}

async fn report_review(args: ReviewArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;

    let cashflow = store.cashflow(args.months).await?;
    let monthly_spend = store.monthly_spend(None).await?;
    let card_summary = store.card_summary(None).await?;
    let forecast_vs_actual = store.forecast_vs_actual(None).await?;
    let uncategorized_count = store.count_uncategorized().await?;
    let uncategorized = store.uncategorized(20).await?;

    let payload = review::ReviewPayload {
        generated_at: Utc::now().to_rfc3339(),
        cashflow,
        monthly_spend,
        card_summary,
        forecast_vs_actual,
        uncategorized_count,
        uncategorized,
    };

    let html = review::generate_html(&payload)?;

    let output_path = args
        .output
        .unwrap_or_else(|| format!("review-{}.html", Utc::now().format("%Y-%m-%d")));
    std::fs::write(&output_path, &html)?;
    println!("Dashboard gerado: {output_path}");

    if args.open {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg(&output_path)
                .spawn()
                .ok();
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open")
                .arg(&output_path)
                .spawn()
                .ok();
        }
    }

    Ok(())
}

async fn tx_upsert_manual(args: ManualTransactionArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let tx_id = args
        .transaction_id
        .unwrap_or_else(|| format!("manual_{}", Uuid::now_v7()));
    let transaction_date =
        NaiveDate::parse_from_str(&args.date, "%Y-%m-%d").context("date inválida")?;
    let category_key = args
        .category
        .as_deref()
        .map(|value| category_id(value, args.subcategory.as_deref()));
    let now = Utc::now();
    let mut tx = TransactionRecord {
        transaction_id: tx_id.clone(),
        account_id: args.account_id.clone(),
        transaction_date,
        description: args.description,
        amount: decimal_from_str(&args.amount)?,
        tx_type: if args.amount.trim_start().starts_with('-') {
            "debit".to_string()
        } else {
            "credit".to_string()
        },
        category_id: category_key,
        category_source: if args.category.is_some() {
            "manual".to_string()
        } else {
            "uncategorized".to_string()
        },
        context: args.context,
        payment_status: args.payment_status,
        source: "manual".to_string(),
        actor_id: config.actor_id.clone(),
        idempotency_key: manual_transaction_idempotency(&config.actor_id),
        metadata_json: json!({"origin": "finance-cli"}),
        created_at: now,
        updated_at: now,
    };
    ensure_transaction_idempotency(&mut tx);
    store.upsert_transactions(&[tx.clone()]).await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "transaction",
            &tx.transaction_id,
            "upsert_manual",
            &config.actor_id,
            &tx.idempotency_key,
            serde_json::to_value(&tx)?,
        )])
        .await?;
    println!("Transação manual salva: {}", tx.transaction_id);
    Ok(())
}

async fn tx_categorize(args: CategorizeTransactionArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let category_key = category_id(&args.category, args.subcategory.as_deref());
    let idempotency_key = format!("annotate:{}:{}", args.transaction_id, Uuid::now_v7());
    store
        .annotate_transaction(
            &args.transaction_id,
            Some(&category_key),
            Some("manual"),
            args.context.as_deref(),
            &config.actor_id,
            &idempotency_key,
        )
        .await?;
    let audit = AuditEvent::from_entity(
        "transaction",
        &args.transaction_id,
        "categorize",
        &config.actor_id,
        &idempotency_key,
        json!({
            "category_id": category_key,
            "context": args.context,
        }),
    );
    store.insert_audit_events(&[audit]).await?;
    println!("Categoria atualizada para {}", args.transaction_id);
    Ok(())
}

async fn tx_set_context(args: SetContextArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let idempotency_key = format!("context:{}:{}", args.transaction_id, Uuid::now_v7());
    store
        .annotate_transaction(
            &args.transaction_id,
            None,
            Some("manual"),
            Some(&args.context),
            &config.actor_id,
            &idempotency_key,
        )
        .await?;
    let audit = AuditEvent::from_entity(
        "transaction",
        &args.transaction_id,
        "set_context",
        &config.actor_id,
        &idempotency_key,
        json!({ "context": args.context }),
    );
    store.insert_audit_events(&[audit]).await?;
    println!("Contexto atualizado para {}", args.transaction_id);
    Ok(())
}

async fn tx_list_context(args: ListContextArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let rows = store.transactions_with_context(args.limit).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("Transactions with context");
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        let account = row
            .account_label
            .or(row.account_id)
            .unwrap_or_else(|| "sem-conta".to_string());
        let category = row
            .category_id
            .unwrap_or_else(|| "sem-categoria".to_string());
        println!(
            "{} | {} | {} | {} | {} | {} | {}",
            row.transaction_id,
            row.transaction_date.format("%Y-%m-%d"),
            brl(row.amount),
            row.description,
            account,
            category,
            row.context
        );
    }
    Ok(())
}

async fn tx_split_preview(args: TxSplitPreviewArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    ensure_bigquery_split_backend(&config)?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let (parent, _payload, preview) =
        load_split_payload_preview(store.as_ref(), &args.transaction_id, &args.payload).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }

    println!("Split preview {}", preview.parent_transaction_id);
    println!("- pai: {} | {}", brl(parent.amount), parent.description);
    println!("- linhas: {}", preview.lines.len());
    println!("- itens de recibo: {}", preview.items.len());
    println!("- total split: {}", brl(preview.split_total));
    println!("- diferença: {}", brl(preview.difference));
    println!("- payload_hash: {}", preview.payload_hash);
    println!();
    for line in &preview.lines {
        println!(
            "{} | {} | {} | {}",
            line.line_index,
            brl(line.amount),
            line.category_id.as_deref().unwrap_or("sem-categoria"),
            line.description
        );
    }
    Ok(())
}

async fn tx_split_apply(args: TxSplitApplyArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    ensure_bigquery_split_backend(&config)?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let (_parent, payload, preview) =
        load_split_payload_preview(store.as_ref(), &args.transaction_id, &args.payload).await?;
    let now = Utc::now();
    let (split, lines, items) = build_split_records(
        &args.transaction_id,
        &config.actor_id,
        payload.source.as_deref(),
        payload.notes.as_deref(),
        payload.metadata.clone(),
        &preview,
        now,
    )?;
    store
        .apply_transaction_split(&split, &lines, &items)
        .await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "transaction_split",
            &split.split_id,
            "apply",
            &config.actor_id,
            &split.idempotency_key,
            json!({
                "parent_transaction_id": args.transaction_id,
                "payload_hash": split.payload_hash,
                "line_count": lines.len(),
                "receipt_item_count": items.len(),
            }),
        )])
        .await?;
    println!(
        "Split aplicado: {} ({} linhas, {} itens)",
        split.split_id,
        lines.len(),
        items.len()
    );
    Ok(())
}

async fn tx_split_show(args: TxSplitShowArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    ensure_bigquery_split_backend(&config)?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let detail = store
        .transaction_split_detail(&args.transaction_id)
        .await?
        .with_context(|| format!("Transação {} não encontrada", args.transaction_id))?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&detail)?);
        return Ok(());
    }

    println!("Split {}", detail.parent.transaction_id);
    println!(
        "- pai: {} | {}",
        brl(detail.parent.amount),
        detail.parent.description
    );
    match &detail.split {
        Some(split) => {
            println!("- split_id: {}", split.split_id);
            println!("- status: {}", split.status);
            println!("- payload_hash: {}", split.payload_hash);
        }
        None => {
            println!("- sem split ativo");
            return Ok(());
        }
    }
    println!("- linhas: {}", detail.lines.len());
    for line in &detail.lines {
        println!(
            "{} | {} | {} | {}",
            line.line_index,
            brl(line.amount),
            line.category_id.as_deref().unwrap_or("sem-categoria"),
            line.description
        );
    }
    println!("- itens de recibo: {}", detail.items.len());
    for item in detail.items.iter().take(20) {
        println!(
            "{} | {} | {} | unit {} | total {}",
            item.item_index,
            item.description,
            item.quantity
                .map(decimal_text)
                .unwrap_or_else(|| "-".to_string()),
            item.unit_price
                .map(decimal_text)
                .unwrap_or_else(|| "-".to_string()),
            item.total_price
                .map(decimal_text)
                .unwrap_or_else(|| "-".to_string())
        );
    }
    Ok(())
}

async fn tx_split_clear(args: TxSplitClearArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    ensure_bigquery_split_backend(&config)?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let parent = store
        .transaction_by_id(&args.transaction_id)
        .await?
        .with_context(|| format!("Transação {} não encontrada", args.transaction_id))?;
    let idempotency_key = format!("split-clear:{}:{}", args.transaction_id, Uuid::now_v7());
    store
        .clear_transaction_split(
            &args.transaction_id,
            &config.actor_id,
            &idempotency_key,
            args.reason.as_deref(),
        )
        .await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "transaction_split",
            &args.transaction_id,
            "clear",
            &config.actor_id,
            &idempotency_key,
            json!({
                "parent_transaction_id": parent.transaction_id,
                "reason": args.reason,
            }),
        )])
        .await?;
    println!("Split limpo para {}", args.transaction_id);
    Ok(())
}

async fn forecast_upsert(args: ForecastUpsertArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let due_date = args
        .date
        .as_deref()
        .map(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
        .transpose()
        .context("date inválida para forecast")?;
    let mut row = ForecastRecord {
        forecast_id: args
            .forecast_id
            .unwrap_or_else(|| format!("forecast_{}", Uuid::now_v7())),
        due_date,
        description: args.description,
        amount: decimal_from_str(&args.amount)?,
        category_id: args
            .category
            .as_deref()
            .map(|value| category_id(value, args.subcategory.as_deref())),
        account_id: args.account_id,
        status: args.status,
        recurrence: args.recurrence,
        actor_id: config.actor_id.clone(),
        idempotency_key: String::new(),
        metadata_json: json!({"origin": "finance-cli"}),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    ensure_forecast_idempotency(&mut row)?;
    store.upsert_forecasts(&[row.clone()]).await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "forecast",
            &row.forecast_id,
            "upsert",
            &config.actor_id,
            &row.idempotency_key,
            serde_json::to_value(&row)?,
        )])
        .await?;
    println!("Forecast salvo: {}", row.forecast_id);
    Ok(())
}

async fn rule_upsert(args: RuleUpsertArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let now = Utc::now();
    let mut row = RuleRecord {
        rule_id: args.rule_id,
        body: args.body,
        status: args.status,
        actor_id: config.actor_id.clone(),
        idempotency_key: String::new(),
        created_at: now,
        updated_at: now,
    };
    ensure_rule_idempotency(&mut row);
    store.upsert_rules(&[row.clone()]).await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "rule",
            &row.rule_id,
            "upsert",
            &config.actor_id,
            &row.idempotency_key,
            serde_json::to_value(&row)?,
        )])
        .await?;
    println!("Rule salva: {}", row.rule_id);
    Ok(())
}

fn rule_matches_status(rule: &RuleRecord, status: RuleStatusFilter) -> bool {
    match status {
        RuleStatusFilter::All => true,
        RuleStatusFilter::Active => rule.status.eq_ignore_ascii_case("active"),
        RuleStatusFilter::Disabled => rule.status.eq_ignore_ascii_case("disabled"),
    }
}

async fn rule_list(args: RuleListArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let rows = store
        .all_rules()
        .await?
        .into_iter()
        .filter(|row| rule_matches_status(row, args.status))
        .collect::<Vec<_>>();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("Rules status={}", args.status.as_str());
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        println!(
            "{} | {} | {} | {}",
            row.rule_id,
            row.status,
            row.updated_at.format("%Y-%m-%d"),
            row.body
        );
    }
    Ok(())
}

async fn rule_inspect(args: RuleInspectArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let row = store
        .all_rules()
        .await?
        .into_iter()
        .find(|row| row.rule_id == args.rule_id)
        .with_context(|| format!("Rule {} não encontrada", args.rule_id))?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&row)?);
        return Ok(());
    }

    println!("Rule {}", row.rule_id);
    println!("- status: {}", row.status);
    println!("- actor_id: {}", row.actor_id);
    println!("- created_at: {}", row.created_at.to_rfc3339());
    println!("- updated_at: {}", row.updated_at.to_rfc3339());
    println!("- idempotency_key: {}", row.idempotency_key);
    println!();
    println!("{}", row.body);
    Ok(())
}

async fn account_upsert(args: AccountUpsertArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let now = Utc::now();
    let mut row = AccountRecord {
        account_id: args.account_id,
        owner: args.owner,
        account_type: args.account_type,
        bank: args.bank,
        label: args.label,
        pluggy_account_id: args.pluggy_account_id,
        pluggy_item_id: args.pluggy_item_id,
        status: args.status,
        actor_id: config.actor_id.clone(),
        idempotency_key: String::new(),
        metadata_json: json!({"origin": "finance-cli"}),
        created_at: now,
        updated_at: now,
    };
    ensure_account_idempotency(&mut row);
    store.upsert_accounts(&[row.clone()]).await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "account",
            &row.account_id,
            "upsert",
            &config.actor_id,
            &row.idempotency_key,
            serde_json::to_value(&row)?,
        )])
        .await?;
    println!("Conta salva: {}", row.account_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_sync_from;
    use chrono::NaiveDate;

    #[test]
    fn resolve_sync_from_prefers_explicit_date() {
        let latest = NaiveDate::from_ymd_opt(2026, 3, 20);
        assert_eq!(
            resolve_sync_from(Some("2026-03-01"), Some("2025-12-01"), latest).unwrap(),
            "2026-03-01"
        );
    }

    #[test]
    fn resolve_sync_from_uses_latest_seen_with_lookback() {
        let latest = NaiveDate::from_ymd_opt(2026, 3, 27);
        assert_eq!(
            resolve_sync_from(None, Some("2025-12-01"), latest).unwrap(),
            "2026-03-13"
        );
    }

    #[test]
    fn resolve_sync_from_never_goes_before_configured_start() {
        let latest = NaiveDate::from_ymd_opt(2025, 12, 10);
        assert_eq!(
            resolve_sync_from(None, Some("2025-12-01"), latest).unwrap(),
            "2025-12-01"
        );
    }
}
