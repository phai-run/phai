use anyhow::{bail, Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Timelike, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use finance_core::idempotency::{
    category_id, ensure_account_idempotency, ensure_forecast_idempotency, ensure_rule_idempotency,
    ensure_transaction_idempotency, manual_transaction_idempotency,
};
use finance_core::installments::{group_into_chains, InstallmentChain};
use finance_core::legacy::load_legacy_bundle;
use finance_core::migrations::run_migrations;
use finance_core::models::{
    decimal_from_str, AccountRecord, AccountSnapshotRecord, AuditEvent, BudgetStatusRow,
    CardClosedTransactionRow, CategoryBudgetRecord, CategoryRecord, ForecastRecord, RuleRecord,
    TransactionRecord,
};
use finance_core::pluggy::{sync_pluggy, SyncPluggyParams};
use finance_core::rules::{apply_rules_with_facts, compile_rules};
use finance_core::splits::{
    build_split_records, parse_split_payload, validate_split_payload, SplitPayload, SplitPreview,
};
use finance_core::storage::{open_store, FinanceStore};
use finance_core::{AppConfig, BackendKind, ConfigPaths};
use rust_decimal::Decimal;
use self_cmd::SelfCommand;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

mod enrich;
mod human_format;
mod pulse;
mod review;
mod self_cmd;
mod update;
mod update_state;

use human_format::{
    bold, brl as hf_brl, category_emoji, month_label, progress_bar, subsection_header,
};

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
    Budget {
        #[command(subcommand)]
        command: BudgetCommand,
    },
    /// Push the daily pulse to an external channel (WhatsApp via webhook).
    Notify {
        #[command(subcommand)]
        command: NotifyCommand,
    },
    #[command(name = "self")]
    SelfCmd {
        #[command(subcommand)]
        command: SelfCommand,
    },
}

#[derive(Subcommand)]
enum NotifyCommand {
    /// POST the rendered daily-pulse to a webhook (WhatsApp gateway).
    ///
    /// Reads `FINANCE_OS_WHATSAPP_WEBHOOK_URL` (required) and
    /// `FINANCE_OS_WHATSAPP_WEBHOOK_TOKEN` (optional, sent as
    /// `Authorization: Bearer <token>`). Posts JSON `{ "text": "..." }`.
    /// Designed for cron / scheduled tasks.
    Whatsapp(NotifyWhatsappArgs),
}

#[derive(Args)]
struct NotifyWhatsappArgs {
    /// Window of the rendered pulse, same semantics as `report daily-pulse --days`.
    #[arg(long, default_value_t = 1)]
    days: i64,
    /// Print the body to stdout in addition to posting. Useful for cron logs.
    #[arg(long)]
    echo: bool,
    /// Don't post — just render and print. Used to preview without sending.
    #[arg(long)]
    dry_run: bool,
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
    /// Skip the automatic post-sync enrichment hook (Phase 5).
    /// Useful in CI / batch jobs where the LLM is intentionally
    /// unavailable. Defaults to false — enrichment runs by default.
    #[arg(long)]
    no_enrich: bool,
}

#[derive(Subcommand)]
enum ReportCommand {
    #[command(
        about = "recent activity grouped by category, last N days",
        long_about = "Shows transactions from the last N days, grouped by spending category \
                      with totals and per-category breakdowns. Useful for a quick health-check \
                      on recent spending without waiting for a full monthly close. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    DailyPulse(DailyPulseArgs),
    #[command(
        about = "monthly expenses broken down by category and subcategory",
        long_about = "Summarises all expenses for a given calendar month, organised by top-level \
                      category and subcategory, with subtotals and a grand total. Defaults to the \
                      current month when --month is omitted. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    MonthlySpend(MonthlySpendArgs),
    #[command(
        about = "monthly income vs expenses vs net, last N months",
        long_about = "Displays a side-by-side comparison of total income, total expenses, and net \
                      cash flow for each of the last N calendar months. Good for spotting \
                      seasonal patterns or confirming that income reliably covers expenses. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    Cashflow(CashflowArgs),
    #[command(
        about = "planned amounts vs what actually happened, sorted by variance",
        long_about = "Compares each budget envelope's forecasted amount against actual spend for \
                      the selected month, ranked by the size of the variance so the biggest \
                      over/under-runs surface first. Useful for post-month budget reviews. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    ForecastVsActual(ForecastVsActualArgs),
    #[command(
        about = "(deprecated alias for `report cards`) credit card cycle totals",
        long_about = "Summarises credit card spending per card for the selected billing cycle. \
                      This subcommand is a deprecated alias — prefer `finance report cards` once \
                      that subcommand is available. Kept for backwards-compatibility with existing \
                      scripts and agent integrations. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    CardSummary(CardSummaryArgs),
    #[command(
        about = "(deprecated alias for `report cards --closed`) most recent closed bill insights",
        long_about = "Shows analysis of the most recently closed credit card bill: top merchants, \
                      category breakdown, and any anomalies worth reviewing before payment. \
                      This subcommand is a deprecated alias — prefer `finance report cards --closed` \
                      once available. WhatsApp-friendly by default; pass --raw for JSON."
    )]
    CardClosedInsights(CardClosedInsightsArgs),
    #[command(
        about = "active installment chains (parcelas), with projected end dates and amounts",
        long_about = "Lists all active installment purchase chains (parcelas), showing how many \
                      installments remain, the monthly amount, and the projected final payment date. \
                      Optionally filtered to a single account via --account-id. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    Installments(InstallmentsArgs),
    #[command(
        about = "transactions still needing a category, grouped by description similarity",
        long_about = "Finds transactions that have no category or are tagged as unclassified, \
                      clusters them by description similarity so similar merchants appear together, \
                      and shows the most impactful ones first (by total amount). Use this to \
                      prioritise categorisation work. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    Uncategorized(UncategorizedArgs),
    #[command(
        about = "transactions whose total matches the sum of multiple smaller items in the same \
                 window (potential splits)",
        long_about = "Detects transactions that look like they should be split into sub-items — \
                      for example, a single supermarket charge whose amount equals the sum of \
                      several receipt line items imported from a different source. Scans the last \
                      N days. WhatsApp-friendly by default; pass --raw for JSON."
    )]
    SplitCandidates(SplitCandidatesArgs),
    #[command(
        about = "historical prices for a specific item across receipts (BigQuery only)",
        long_about = "Searches receipt line-item data (BigQuery backend only) for a specific \
                      product name and returns the price history across all matching purchases. \
                      Useful for tracking price inflation on frequently bought items. \
                      Pass --raw for JSON output instead of a formatted table."
    )]
    ItemPrices(ItemPricesArgs),
    #[command(
        about = "consistency checks across accounts, categories, rules, and idempotency keys",
        long_about = "Runs a battery of data-quality checks: orphaned transactions, missing \
                      account references, duplicate idempotency keys, rule conflicts, and more. \
                      Intended for periodic audits or after bulk imports to confirm data integrity. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    DataHealth(DataHealthArgs),
    #[command(
        about = "cash projection under a hypothetical extra income/expense for the month",
        long_about = "Projects the month-end balance assuming an extra one-off income or expense \
                      on top of the current actuals and the historical average for the remaining \
                      days. Useful for quick what-if questions before a large purchase or bonus. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    Scenario(ScenarioArgs),
    #[command(
        about = "compare an OFX file against the database transaction-by-transaction",
        long_about = "Loads an OFX export from your bank and matches each transaction against \
                      what is already in the database, flagging mismatches in amount, date, or \
                      description and listing items present in only one source. Helpful for \
                      reconciliation after a manual import. Pass --raw for JSON output."
    )]
    OfxConsistency(OfxConsistencyArgs),
    #[command(
        about = "monthly review summary with key metrics and trends, optionally written to a \
                 markdown file",
        long_about = "Generates a narrative monthly review covering income, expenses, net, top \
                      categories, and multi-month trends. Can write the result to a markdown file \
                      (--output) and open it automatically (--open). \
                      Output is always rendered as markdown; there is no --raw flag for this report."
    )]
    Review(ReviewArgs),
    #[command(
        about = "category budgets vs actual spend for the month, with progress bars and alerts",
        long_about = "Compares each category's defined budget against actual spend for the given \
                      month, renders a progress bar for each envelope, and highlights categories \
                      that are over budget or approaching their limit. Requires --month (YYYY-MM). \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    BudgetStatus(BudgetStatusArgs),
    #[command(
        about = "credit-card insights: charges, payment status, installments, and by-category breakdown",
        long_about = "Shows everything that hit your credit cards in a given month: per-card \
                      totals with payment status (paid/open/overdue, inferred by matching \
                      credit-card-payment debits in checking accounts), the active installment \
                      commitment, and every charge grouped by category. Use --month YYYY-MM for \
                      a specific cycle or --closed for each card's most recent closed bill. \
                      Replaces the deprecated `card-summary` and `card-closed-insights` reports. \
                      WhatsApp-friendly by default; pass --raw for JSON."
    )]
    Cards(CardsArgs),
    #[command(
        about = "saldo em conta por dono e total, do snapshot mais recente do Pluggy",
        long_about = "Mostra o saldo em conta mais recente sincronizado do Pluggy para cada \
                      conta corrente, agrupado por dono (owner) com subtotais e o total geral. \
                      Cartões de crédito não aparecem aqui — para ver fatura em aberto, use \
                      `report cards` ou o pulse. WhatsApp-friendly by default; pass --raw for JSON."
    )]
    Balances(BalancesArgs),
}

#[derive(Args)]
struct BalancesArgs {
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
}

impl BalancesArgs {
    fn structured_output(&self) -> bool {
        self.raw
    }
}

#[derive(Args)]
struct DailyPulseArgs {
    /// Number of days to look back (e.g. 7 for the past week, 30 for a month).
    #[arg(long, default_value_t = 7)]
    days: i64,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`. Kept for backwards-compatibility with
    /// agents and scripts that still pass `--json`.
    #[arg(long, hide = true)]
    json: bool,
}

impl DailyPulseArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct MonthlySpendArgs {
    /// Target month in YYYY-MM format (e.g. 2024-11). Defaults to the current month.
    #[arg(long)]
    month: Option<String>,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`. Kept for backwards-compat with scripts/skills.
    #[arg(long, hide = true)]
    json: bool,
}

impl MonthlySpendArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct CashflowArgs {
    /// Number of trailing calendar months to include (e.g. 6 for the past half-year).
    #[arg(long, default_value_t = 6)]
    months: usize,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`. Kept for backwards-compat with scripts/skills.
    #[arg(long, hide = true)]
    json: bool,
}

impl CashflowArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct ForecastVsActualArgs {
    /// Target month in YYYY-MM format. Defaults to the current month.
    #[arg(long)]
    month: Option<String>,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`. Kept for backwards-compat with scripts/skills.
    #[arg(long, hide = true)]
    json: bool,
}

impl ForecastVsActualArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct CardSummaryArgs {
    /// Billing month in YYYY-MM format. Defaults to the current month.
    #[arg(long)]
    month: Option<String>,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

impl CardSummaryArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct CardClosedInsightsArgs {
    /// Billing month of the closed bill in YYYY-MM format. Defaults to the most recently closed cycle.
    #[arg(long)]
    month: Option<String>,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

impl CardClosedInsightsArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct InstallmentsArgs {
    /// Filter results to a single account (e.g. `nubank-credit`).
    #[arg(long)]
    account_id: Option<String>,
    /// How many months back to search for installment chains (default 12).
    #[arg(long, default_value_t = 12)]
    lookback_months: u32,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
    /// Show individual installment transactions in addition to chain summaries.
    #[arg(long)]
    verbose: bool,
}

impl InstallmentsArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct UncategorizedArgs {
    /// Maximum number of uncategorized transaction groups to show (default 20).
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

impl UncategorizedArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct SplitCandidatesArgs {
    /// Window (in days) to scan for matching sub-item sums (default 30).
    #[arg(long, default_value_t = 30)]
    days: i64,
    /// Emit machine-readable JSON.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

impl SplitCandidatesArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct ItemPricesArgs {
    /// Product name or keyword to search for across receipt line items (e.g. "leite integral").
    #[arg(long)]
    query: String,
    /// Earliest date to include, in YYYY-MM-DD format. Defaults to all available history.
    #[arg(long)]
    since: Option<String>,
    /// Emit machine-readable JSON.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

impl ItemPricesArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct DataHealthArgs {
    /// How many days of recent data to include in recency-sensitive checks (default 180).
    #[arg(long, default_value_t = 180)]
    days: i64,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

impl DataHealthArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct ScenarioArgs {
    /// Month to project, in YYYY-MM format. Defaults to the current month.
    #[arg(long)]
    month: Option<String>,
    /// Number of past months used to estimate the daily spending rate (default 3).
    #[arg(long, default_value_t = 3)]
    history_months: usize,
    /// Hypothetical one-off extra expense to add to the projection (e.g. "500.00").
    #[arg(long, default_value = "0")]
    extra_expense: String,
    /// Hypothetical one-off extra income to add to the projection (e.g. "1000.00").
    #[arg(long, default_value = "0")]
    extra_income: String,
    /// Emit machine-readable JSON.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

impl ScenarioArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct OfxConsistencyArgs {
    /// Path to the OFX file exported from the bank (e.g. ~/Downloads/nubank.ofx).
    #[arg(long)]
    ofx: PathBuf,
    /// Account ID to scope the database lookup (e.g. `nubank-checking`). Auto-detected if omitted.
    #[arg(long)]
    account_id: Option<String>,
    /// How many days of date difference to still consider a match (default 1).
    #[arg(long, default_value_t = 1)]
    date_tolerance_days: i64,
    /// Emit machine-readable JSON.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

impl OfxConsistencyArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct ReviewArgs {
    /// Number of trailing months to include in trend analysis (default 6).
    #[arg(long, default_value_t = 6)]
    months: usize,
    /// Write the markdown report to this file path instead of printing to stdout.
    #[arg(long)]
    output: Option<String>,
    /// Open the output file in the system's default markdown viewer after writing.
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallmentChainView {
    account_id: String,
    base_description: String,
    total: u32,
    current: u32,
    first_date: String,
    projected_end: String,
    remaining: u32,
    released_next_month: bool,
    total_amount: Decimal,
}

impl InstallmentChainView {
    fn from_chain(chain: &InstallmentChain) -> Self {
        Self {
            account_id: chain.account_id.clone(),
            base_description: chain.base_description.clone(),
            total: chain.total,
            current: chain.current,
            first_date: chain.first_date.format("%Y-%m-%d").to_string(),
            projected_end: chain.projected_end.format("%Y-%m-%d").to_string(),
            remaining: chain.remaining,
            released_next_month: chain.released_next_month,
            total_amount: chain.total_amount,
        }
    }
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
    Find(TxFindArgs),
    Pending(TxPendingArgs),
    SetContextByDesc(SetContextByDescArgs),
    Split {
        #[command(subcommand)]
        command: TxSplitCommand,
    },
    /// Run the LLM-driven enrichment pipeline over uncategorized
    /// transactions. Supports human + machine (NDJSON) modes.
    Enrich(enrich::EnrichArgs),
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
struct TxFindArgs {
    #[arg(long)]
    query: String,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct TxPendingArgs {
    #[arg(long, default_value_t = 10)]
    limit: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct SetContextByDescArgs {
    #[arg(long)]
    query: String,
    #[arg(long)]
    context: String,
    #[arg(long)]
    dry_run: bool,
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

#[derive(Subcommand)]
enum BudgetCommand {
    Upsert(BudgetUpsertArgs),
    List(BudgetListArgs),
}

#[derive(Args)]
struct BudgetUpsertArgs {
    #[arg(long)]
    category_id: String,
    #[arg(long)]
    subcategory_id: Option<String>,
    #[arg(long)]
    month: Option<String>,
    #[arg(long)]
    amount: String,
    #[arg(long, default_value_t = 80)]
    alert_threshold_pct: i64,
}

#[derive(Args)]
struct BudgetListArgs {
    #[arg(long)]
    month: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct BudgetStatusArgs {
    /// Month to report on, in YYYY-MM format (required, e.g. 2024-11).
    #[arg(long)]
    month: String,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

impl BudgetStatusArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
}

#[derive(Args)]
struct CardsArgs {
    /// Target month in YYYY-MM. Defaults to the most recent closed bill
    /// (same as `--closed`).
    #[arg(long, conflicts_with_all = ["closed", "next"])]
    month: Option<String>,
    /// Show each card's most recent closed bill. This is the default when no
    /// month is given. Mutually exclusive with --month and --next.
    #[arg(long, conflicts_with = "next")]
    closed: bool,
    /// Show the partial state of the currently open cycle (what's been
    /// charged so far this month — payment status is not inferred since
    /// the bill hasn't closed). Mutually exclusive with --closed and --month.
    #[arg(long)]
    next: bool,
    /// Limit the report to a single credit-card account.
    #[arg(long)]
    account_id: Option<String>,
    /// List every transaction in each category instead of truncating to the
    /// top 5 by absolute amount.
    #[arg(long)]
    all: bool,
    /// Show only installment (parcelada) transactions — hides one-off and
    /// subscription charges so you can focus on multi-month commitments.
    #[arg(long)]
    installments_only: bool,
    /// Emit machine-readable JSON instead of the WhatsApp-friendly summary.
    #[arg(long)]
    raw: bool,
    /// Deprecated alias for `--raw`.
    #[arg(long, hide = true)]
    json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CardsMode {
    Closed,
    Specific,
    Next,
}

impl CardsArgs {
    fn structured_output(&self) -> bool {
        self.raw || self.json
    }
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

/// Formats a Decimal as BRL with sign and thousands separator: `-R$ 1.500,00`.
fn brl_signed(value: Decimal) -> String {
    let sign = if value.is_sign_negative() { "-" } else { "+" };
    let abs_val = value.abs().round_dp(2);
    let formatted = format!("{:.2}", abs_val).replace('.', ",");
    // Insert thousands separators (period) before the comma
    let (integer_part, decimal_part) = formatted.split_once(',').unwrap_or((&formatted, "00"));
    let with_thousands = insert_thousands(integer_part);
    format!("{sign}R$ {with_thousands},{decimal_part}")
}

/// Formats a Decimal as unsigned BRL with thousands separator: `R$ 1.500,00`.
fn brl_abs(value: Decimal) -> String {
    let abs_val = value.abs().round_dp(2);
    let formatted = format!("{:.2}", abs_val).replace('.', ",");
    let (integer_part, decimal_part) = formatted.split_once(',').unwrap_or((&formatted, "00"));
    let with_thousands = insert_thousands(integer_part);
    format!("R$ {with_thousands},{decimal_part}")
}

fn insert_thousands(s: &str) -> String {
    let digits: Vec<char> = s.chars().collect();
    let n = digits.len();
    let mut result = String::with_capacity(n + n / 3);
    for (i, ch) in digits.iter().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            result.push('.');
        }
        result.push(*ch);
    }
    result
}

/// Formats a NaiveDate as short Portuguese date: `14/mai/2026`.
fn short_date_pt(date: NaiveDate) -> String {
    let month_abbr = match date.month() {
        1 => "jan",
        2 => "fev",
        3 => "mar",
        4 => "abr",
        5 => "mai",
        6 => "jun",
        7 => "jul",
        8 => "ago",
        9 => "set",
        10 => "out",
        11 => "nov",
        12 => "dez",
        _ => "???",
    };
    format!("{:02}/{}/{}", date.day(), month_abbr, date.year())
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

/// Render the sync notify summary as a single phone-readable WhatsApp
/// message. The legacy pipe-separated CLI-log format was unreadable on a
/// phone — this replaces it with a five-block layout:
///
///   🔄 *Sync · seg 18/mai 21:34*
///
///   *N novas transações* · -R$ X,YZ
///     <emoji> <descrição curta> · <valor> · <data>
///     …
///
///   *Saldo em conta*   ← only when there were new transactions
///     💰 <conta> · <saldo>
///     *Total*: <total>
///
///   *N sem categoria* (responda 1..N para classificar)
///     1. <descrição> · <valor> (id <transaction_id>)
///
///   ⚠️ <aviso>
///
///   _finance X.Y.Z · <backend> · <hora>_
fn render_sync_notify_summary(
    summary: &SyncSummaryOutput,
    accounts: &[AccountRecord],
    snapshots: &[finance_core::models::AccountSnapshotRecord],
) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    // Header: weekday, day, month, HH:MM.
    let now = Utc::now();
    let local = now.naive_local();
    let header_date = local.date();
    let header_time = local.time();
    let weekday = match header_date.weekday().num_days_from_monday() {
        0 => "seg",
        1 => "ter",
        2 => "qua",
        3 => "qui",
        4 => "sex",
        5 => "sáb",
        _ => "dom",
    };
    let month = match header_date.month() {
        1 => "jan",
        2 => "fev",
        3 => "mar",
        4 => "abr",
        5 => "mai",
        6 => "jun",
        7 => "jul",
        8 => "ago",
        9 => "set",
        10 => "out",
        11 => "nov",
        _ => "dez",
    };
    let _ = writeln!(
        out,
        "🔄 *Sync · {weekday} {:02}/{month} {:02}:{:02}*",
        header_date.day(),
        header_time.hour(),
        header_time.minute(),
    );

    let has_new = summary.new_transactions_count > 0;

    // -------- Block 1: novas transações --------
    let _ = writeln!(out);
    if !has_new {
        let _ = writeln!(out, "_sem novidades_");
    } else {
        let mut total = Decimal::ZERO;
        for tx in &summary.new_transactions {
            if let Ok(amount) = decimal_from_str(&tx.amount) {
                total += amount;
            }
        }
        let _ = writeln!(
            out,
            "*{} nova{} transaç{}* · {}",
            summary.new_transactions_count,
            if summary.new_transactions_count == 1 {
                ""
            } else {
                "s"
            },
            if summary.new_transactions_count == 1 {
                "ão"
            } else {
                "ões"
            },
            brl_signed(total),
        );
        const MAX_TX: usize = 8;
        let show = summary.new_transactions.iter().take(MAX_TX);
        for tx in show {
            let amount = decimal_from_str(&tx.amount).ok();
            let emoji = human_format::category_emoji(tx.category_id.as_deref(), amount);
            let date = NaiveDate::parse_from_str(&tx.transaction_date, "%Y-%m-%d")
                .map(human_format::short_date)
                .unwrap_or_else(|_| tx.transaction_date.clone());
            let label = human_format::truncate_with_ellipsis(
                &human_format::short_description(tx.context.as_deref().unwrap_or(&tx.description)),
                34,
            );
            let amt_str = amount.map(brl_signed).unwrap_or_else(|| tx.amount.clone());
            let _ = writeln!(out, "  {emoji} {label} · {amt_str} · {date}");
        }
        if summary.new_transactions.len() > MAX_TX {
            let _ = writeln!(
                out,
                "  _… mais {} lançamento{}_",
                summary.new_transactions.len() - MAX_TX,
                if summary.new_transactions.len() - MAX_TX == 1 {
                    ""
                } else {
                    "s"
                }
            );
        }

        // -------- Block 2: saldo em conta (only when there are new tx) --------
        let checking_ids: BTreeSet<String> = accounts
            .iter()
            .filter(|a| a.account_type == "checking" && !a.account_id.is_empty())
            .map(|a| a.account_id.clone())
            .collect();
        let balances: Vec<(&finance_core::models::AccountSnapshotRecord, &AccountRecord)> =
            snapshots
                .iter()
                .filter(|s| checking_ids.contains(&s.account_id) && s.balance.is_some())
                .filter_map(|s| {
                    accounts
                        .iter()
                        .find(|a| a.account_id == s.account_id)
                        .map(|a| (s, a))
                })
                .collect();
        if !balances.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "*Saldo em conta*");
            let mut total_bal = Decimal::ZERO;
            for (snap, acc) in &balances {
                let label = if acc.label.is_empty() {
                    acc.account_id.clone()
                } else {
                    acc.label.clone()
                };
                let balance = snap.balance.unwrap_or(Decimal::ZERO);
                total_bal += balance;
                let _ = writeln!(out, "  💰 {} · {}", label, brl_signed(balance));
            }
            if balances.len() > 1 {
                let _ = writeln!(out, "  *Total*: {}", brl_signed(total_bal));
            }
        }
    }

    // -------- Block 3: pendências (sem categoria) --------
    if summary.needs_context_count > 0 {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "*{} sem categoria*{} — responda 1..{} para classificar",
            summary.needs_context_count,
            if summary.needs_context_truncated {
                " (parcial)"
            } else {
                ""
            },
            summary
                .needs_context
                .len()
                .min(summary.needs_context_count as usize),
        );
        for (idx, tx) in summary.needs_context.iter().enumerate() {
            let amount = decimal_from_str(&tx.amount).ok();
            let label = human_format::truncate_with_ellipsis(
                &human_format::short_description(&tx.description),
                34,
            );
            let amt_str = amount.map(brl_signed).unwrap_or_else(|| tx.amount.clone());
            // Show a short id suffix for reference.
            let short_id = tx
                .transaction_id
                .split('-')
                .next_back()
                .unwrap_or(&tx.transaction_id);
            let _ = writeln!(out, "  {}. {label} · {amt_str} (id …{short_id})", idx + 1);
        }
    }

    // -------- Block 4: avisos --------
    if !summary.warnings.is_empty() {
        let _ = writeln!(out);
        for warning in &summary.warnings {
            let _ = writeln!(out, "⚠️ {}", normalize_inline_text(warning));
        }
    }

    // -------- Footer --------
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "_finance {} · {} · status {}_",
        env!("CARGO_PKG_VERSION"),
        summary.backend,
        summary.summary_status
    );

    out.trim_end().to_string()
}

/// Copy billing metadata keys from an existing account record into a freshly
/// built Pluggy account so manual overrides survive re-syncs.
fn merge_billing_metadata(new_meta: &mut serde_json::Value, existing_meta: &serde_json::Value) {
    for key in ["billing_closing_day", "billing_due_day"] {
        if new_meta.get(key).is_none() {
            if let Some(v) = existing_meta.get(key) {
                new_meta[key] = v.clone();
            }
        }
    }
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
            // Also try descriptionRaw stored by Pluggy sync in metadata.
            row.metadata_json
                .get("raw")
                .and_then(|r| r.get("descriptionRaw"))
                .and_then(|v| v.as_str())
                .and_then(extract_installment_marker)
        })
        .or_else(|| {
            metadata_contains_installment_signal(&row.metadata_json).then(|| "metadata".to_string())
        })
}

/// Returns the description enriched with an installment marker sourced from
/// Pluggy metadata when the stored description text lacks one. Used to make
/// `report installments` work with data synced before the Pluggy fix landed.
fn enrich_description_from_metadata(description: &str, metadata: &Value) -> String {
    if finance_core::installments::parse_installment_description(description).is_some() {
        return description.to_string();
    }
    // Try descriptionRaw first.
    if let Some(raw) = metadata
        .get("raw")
        .and_then(|r| r.get("descriptionRaw"))
        .and_then(|v| v.as_str())
        .filter(|s| finance_core::installments::parse_installment_description(s).is_some())
    {
        return raw.to_string();
    }
    // Fall back to creditCardMetadata structured fields.
    if let (Some(current), Some(total)) = (
        metadata
            .get("raw")
            .and_then(|r| r.get("creditCardMetadata"))
            .and_then(|m| m.get("installmentNumber"))
            .and_then(|v| v.as_u64()),
        metadata
            .get("raw")
            .and_then(|r| r.get("creditCardMetadata"))
            .and_then(|m| m.get("totalInstallments"))
            .and_then(|v| v.as_u64()),
    ) {
        if current > 0 && total > 0 && current <= total && total <= 99 {
            return format!("{} {}/{}", description.trim(), current, total);
        }
    }
    description.to_string()
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

async fn insert_snapshots_chunked(
    store: &dyn FinanceStore,
    rows: &[AccountSnapshotRecord],
) -> Result<usize> {
    let mut total = 0;
    for chunk in rows.chunks(UPSERT_BATCH_SIZE) {
        total += store.insert_account_snapshots(chunk).await?;
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

fn should_run_auto_check(cli: &Cli) -> bool {
    if std::env::var_os("FINANCE_OS_NO_AUTO_UPDATE").is_some() {
        return false;
    }
    if let Some(updated_ver) = std::env::var_os("FINANCE_OS_UPDATED") {
        let updated_ver = updated_ver.to_string_lossy();
        if updated_ver != env!("CARGO_PKG_VERSION") {
            eprintln!(
                "warning: re-exec sentinel mismatch — old binary still running? \
                 (sentinel={updated_ver}, binary={})",
                env!("CARGO_PKG_VERSION")
            );
        }
        return false;
    }
    if matches!(cli.command, Commands::SelfCmd { .. }) {
        return false;
    }
    true
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // If we were just re-execed by a self-update, show the release notes
    // for the new version once, then continue with the user's command.
    // The sentinel only lives in the immediate post-exec env, so this
    // fires exactly once per upgrade.
    if let Some(updated_to) = std::env::var_os("FINANCE_OS_UPDATED") {
        let version = updated_to.to_string_lossy().to_string();
        if version == env!("CARGO_PKG_VERSION") {
            update::print_release_notes(&version).await;
        }
    }

    // Auto-update check (never blocks command execution)
    if should_run_auto_check(&cli) {
        let paths = ConfigPaths::discover().ok();
        if let Some(ref p) = paths {
            update::auto_check(&p.data_dir).await;
        }
    }

    match cli.command {
        Commands::SelfCmd { command } => self_cmd::run(command).await,
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
            ReportCommand::Installments(args) => report_installments(args).await,
            ReportCommand::Uncategorized(args) => report_uncategorized(args).await,
            ReportCommand::SplitCandidates(args) => report_split_candidates(args).await,
            ReportCommand::ItemPrices(args) => report_item_prices(args).await,
            ReportCommand::DataHealth(args) => report_data_health(args).await,
            ReportCommand::Scenario(args) => report_scenario(args).await,
            ReportCommand::OfxConsistency(args) => report_ofx_consistency(args).await,
            ReportCommand::Review(args) => report_review(args).await,
            ReportCommand::BudgetStatus(args) => report_budget_status(args).await,
            ReportCommand::Cards(args) => report_cards(args).await,
            ReportCommand::Balances(args) => report_balances(args).await,
        },
        Commands::Tx { command } => match command {
            TxCommand::UpsertManual(args) => tx_upsert_manual(args).await,
            TxCommand::Categorize(args) => tx_categorize(args).await,
            TxCommand::SetContext(args) => tx_set_context(args).await,
            TxCommand::ListContext(args) => tx_list_context(args).await,
            TxCommand::Find(args) => tx_find(args).await,
            TxCommand::Pending(args) => tx_pending(args).await,
            TxCommand::SetContextByDesc(args) => tx_set_context_by_desc(args).await,
            TxCommand::Split { command } => match command {
                TxSplitCommand::Preview(args) => tx_split_preview(args).await,
                TxSplitCommand::Apply(args) => tx_split_apply(args).await,
                TxSplitCommand::Show(args) => tx_split_show(args).await,
                TxSplitCommand::Clear(args) => tx_split_clear(args).await,
            },
            TxCommand::Enrich(args) => tx_enrich(args).await,
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
        Commands::Budget { command } => match command {
            BudgetCommand::Upsert(args) => budget_upsert(args).await,
            BudgetCommand::List(args) => budget_list(args).await,
        },
        Commands::Notify { command } => match command {
            NotifyCommand::Whatsapp(args) => notify_whatsapp(args).await,
        },
    }
}

const WHATSAPP_WEBHOOK_URL_ENV: &str = "FINANCE_OS_WHATSAPP_WEBHOOK_URL";
const WHATSAPP_WEBHOOK_TOKEN_ENV: &str = "FINANCE_OS_WHATSAPP_WEBHOOK_TOKEN";

async fn notify_whatsapp(args: NotifyWhatsappArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let today = Utc::now().date_naive();
    let data = pulse::gather_pulse_data(store.as_ref(), today, args.days).await?;
    let plan = pulse::compute_closing_plan(&data);
    let body = pulse::render_pulse(&data, &plan, args.days);

    if args.echo || args.dry_run {
        println!("{body}");
    }

    if args.dry_run {
        return Ok(());
    }

    let url = std::env::var(WHATSAPP_WEBHOOK_URL_ENV).map_err(|_| {
        anyhow::anyhow!(
            "{WHATSAPP_WEBHOOK_URL_ENV} not set. Export the webhook URL or use --dry-run."
        )
    })?;
    let token = std::env::var(WHATSAPP_WEBHOOK_TOKEN_ENV).ok();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .context("failed to build HTTP client")?;
    let payload = json!({ "text": body });
    let mut req = client.post(&url).json(&payload);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let response = req
        .send()
        .await
        .with_context(|| format!("POST {url} failed"))?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("webhook returned {status}: {text}");
    }
    if args.echo {
        eprintln!("notify whatsapp: {status}");
    }
    Ok(())
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

    // Always run on the latest binary before touching external data.
    // Skip when we just re-execed after an upgrade (FINANCE_OS_UPDATED) or
    // when the user opted out of auto-update (FINANCE_OS_NO_AUTO_UPDATE) —
    // CI and e2e tests rely on the latter to keep `target/debug/finance-cli`
    // stable across subprocess invocations.
    if std::env::var_os("FINANCE_OS_UPDATED").is_none()
        && std::env::var_os("FINANCE_OS_NO_AUTO_UPDATE").is_none()
    {
        if let Ok(paths) = ConfigPaths::discover() {
            update::force_check(&paths.data_dir).await;
        }
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
    let (mut accounts, transactions, rebinds) = sync_pluggy(SyncPluggyParams {
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
    // Merge billing metadata from existing accounts so that manually set
    // billing_closing_day / billing_due_day survive Pluggy re-syncs (which
    // always rebuild metadata_json from the API payload).
    if let Ok(existing_accounts) = store.get_accounts().await {
        let existing_map: BTreeMap<&str, &serde_json::Value> = existing_accounts
            .iter()
            .map(|a| (a.account_id.as_str(), &a.metadata_json))
            .collect();
        for acct in &mut accounts {
            if let Some(existing_meta) = existing_map.get(acct.account_id.as_str()) {
                merge_billing_metadata(&mut acct.metadata_json, existing_meta);
            }
        }
    }

    let categories = build_category_records_from_transactions(&config.actor_id, &transactions);
    let mut audit = Vec::new();

    upsert_accounts_chunked(store.as_ref(), &accounts).await?;

    // Insert one snapshot per account for today's date — idempotent on repeated syncs.
    let today = Utc::now().date_naive();
    let snapshots: Vec<AccountSnapshotRecord> = accounts
        .iter()
        .map(|row| {
            let idempotency_key = format!(
                "snapshot:{}:{}:pluggy",
                row.account_id,
                today.format("%Y-%m-%d")
            );
            AccountSnapshotRecord {
                snapshot_id: Uuid::now_v7().to_string(),
                account_id: row.account_id.clone(),
                snapshot_date: today,
                balance: row.metadata_json.get("balance").and_then(|v| {
                    v.as_str()
                        .and_then(|s| s.parse::<Decimal>().ok())
                        .or_else(|| {
                            v.as_f64()
                                .and_then(|f| format!("{f:.2}").parse::<Decimal>().ok())
                        })
                }),
                credit_limit: None,
                currency_code: row
                    .metadata_json
                    .get("currency_code")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                source: "pluggy".to_string(),
                actor_id: config.actor_id.clone(),
                idempotency_key,
                metadata_json: serde_json::json!({}),
                created_at: Utc::now(),
            }
        })
        .collect();
    insert_snapshots_chunked(store.as_ref(), &snapshots).await?;

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
            // The human-friendly notify-summary message folds in the
            // current saldo per checking account so a phone-side reader
            // immediately sees the balance impact of the new transactions.
            let snapshots = store.latest_account_snapshots().await.unwrap_or_default();
            let stored_accounts = store.get_accounts().await.unwrap_or_default();
            println!(
                "{}",
                render_sync_notify_summary(&summary, &stored_accounts, &snapshots)
            );
        }
        return Ok(());
    }

    println!("Sync Pluggy concluído:");
    println!("- accounts: {}", accounts.len());
    println!("- transactions: {}", transactions.len());
    println!("- categories: {}", categories.len());
    println!("- actor: {}", config.actor_id);
    println!("- backend: {:?}", config.effective_backend());

    // Phase 5 — post-sync enrichment hook. Non-fatal: any failure
    // inside `enrich_after_sync` is logged via eprintln and the sync
    // result is preserved. The user can re-run `finance tx enrich`
    // manually whenever the LLM is back up.
    let new_tx_ids: Vec<String> = transactions
        .iter()
        .filter(|row| !existing_ids.contains(&row.transaction_id))
        .map(|row| row.transaction_id.clone())
        .collect();
    println!(
        "Sincronização concluída: {} transações novas",
        new_tx_ids.len()
    );

    if args.no_enrich {
        println!("Enrichment automático: pulado (--no-enrich).");
    } else if new_tx_ids.is_empty() {
        println!("Enrichment automático: sem transações novas para processar.");
    } else {
        // Non-TTY (CI, pipes, batch) → force auto_only so we never
        // print Suggest/AskUser prompts to a non-interactive stdout.
        let auto_only = !std::io::IsTerminal::is_terminal(&std::io::stdin());
        let summary =
            enrich::enrich_after_sync(&config, store.as_ref(), &new_tx_ids, auto_only).await;
        println!("{}", summary.format_summary());
        if summary.deferred > 0 {
            println!("Para revisar as adiadas: finance tx enrich --days 7");
        }
    }
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

    if args.structured_output() {
        // Backwards-compat: --raw still emits the flat list of items.
        let since = Utc::now()
            .date_naive()
            .checked_sub_signed(Duration::days(args.days.saturating_sub(1)))
            .context("Falha ao calcular janela do daily pulse")?;
        let items = store.daily_pulse(since).await?;
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    let today = Utc::now().date_naive();
    let data = pulse::gather_pulse_data(store.as_ref(), today, args.days).await?;
    let plan = pulse::compute_closing_plan(&data);
    println!("{}", pulse::render_pulse(&data, &plan, args.days));
    Ok(())
}

async fn report_balances(args: BalancesArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let accounts = store.get_accounts().await?;
    let snapshots = store.latest_account_snapshots().await?;

    // Index account → owner + label.
    let account_by_id: BTreeMap<&str, &AccountRecord> = accounts
        .iter()
        .filter(|a| !a.account_id.is_empty())
        .map(|a| (a.account_id.as_str(), a))
        .collect();

    // Only checking accounts have a meaningful "saldo em conta". Credit
    // cards expose debt via card_summary; mixing them here would mislead.
    let rows: Vec<(&AccountRecord, &finance_core::models::AccountSnapshotRecord)> = snapshots
        .iter()
        .filter_map(|s| {
            account_by_id
                .get(s.account_id.as_str())
                .filter(|a| a.account_type == "checking")
                .map(|a| (*a, s))
        })
        .collect();

    if args.structured_output() {
        let payload: Vec<_> = rows
            .iter()
            .map(|(a, s)| {
                serde_json::json!({
                    "account_id": a.account_id,
                    "owner": a.owner,
                    "label": a.label,
                    "balance": s.balance,
                    "currency_code": s.currency_code,
                    "snapshot_date": s.snapshot_date,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("{}", human_format::bold("Saldo em conta"));
        println!("  _(nenhum snapshot disponível — rode `finance sync pluggy` para atualizar)_");
        return Ok(());
    }

    // Group by owner so "Aline · Felipe · Total" reads naturally.
    let mut by_owner: BTreeMap<
        String,
        Vec<(&AccountRecord, &finance_core::models::AccountSnapshotRecord)>,
    > = BTreeMap::new();
    for (acc, snap) in &rows {
        by_owner
            .entry(acc.owner.clone())
            .or_default()
            .push((acc, snap));
    }

    println!("{}", human_format::bold("Saldo em conta"));
    let mut grand_total = Decimal::ZERO;
    for (owner, items) in &by_owner {
        let mut owner_total = Decimal::ZERO;
        for (acc, snap) in items {
            let balance = snap.balance.unwrap_or(Decimal::ZERO);
            owner_total += balance;
            grand_total += balance;
            let label = if acc.label.is_empty() {
                acc.account_id.clone()
            } else {
                acc.label.clone()
            };
            println!(
                "  💰 {} · {} ({})",
                label,
                human_format::brl_signed(balance),
                human_format::short_date(snap.snapshot_date),
            );
        }
        if items.len() > 1 {
            println!(
                "  └ {}: {}",
                human_format::bold(&capitalize(owner)),
                human_format::brl_signed(owner_total),
            );
        }
    }
    println!(
        "  {}: {}",
        human_format::bold("Total"),
        human_format::brl_signed(grand_total),
    );
    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

async fn report_monthly_spend(args: MonthlySpendArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.monthly_spend(args.month.as_deref()).await?;

    if args.structured_output() {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    print_monthly_spend_human(&rows, args.month.as_deref());
    Ok(())
}

fn print_monthly_spend_human(rows: &[finance_core::models::MonthlySpendRow], month: Option<&str>) {
    use human_format::{
        bold, brl, category_emoji, category_family, family_label, month_label, pct,
    };
    use std::cmp::Reverse;
    use std::collections::HashMap;

    // Determine the month_ref label
    let month_display = month
        .map(month_label)
        .or_else(|| rows.first().map(|r| month_label(&r.month_ref)))
        .unwrap_or_else(|| "—".to_string());

    println!("📊 {}", bold(&format!("Gastos · {month_display}")));

    if rows.is_empty() {
        println!("- linhas: 0");
        return;
    }

    let internal_prefix = "financeiro-pagamento-recebido";

    // Filter out internal categories
    let visible: Vec<_> = rows
        .iter()
        .filter(|r| !r.category_id.starts_with(internal_prefix))
        .collect();

    // Group by family, accumulate spend per family
    let mut family_totals: HashMap<String, Decimal> = HashMap::new();
    // family → list of (category_id, spend)
    let mut family_rows: HashMap<String, Vec<(&str, Decimal)>> = HashMap::new();

    for row in &visible {
        let spend = -row.expenses; // expenses are stored negative
        let family =
            category_family(Some(&row.category_id)).unwrap_or_else(|| "outros".to_string());
        *family_totals.entry(family.clone()).or_insert(Decimal::ZERO) += spend;
        family_rows
            .entry(family)
            .or_default()
            .push((&row.category_id, spend));
    }

    // Sort families by spend descending
    let mut families: Vec<String> = family_totals.keys().cloned().collect();
    families.sort_by_key(|f| Reverse(family_totals[f]));

    let grand_total: Decimal = family_totals.values().copied().sum();

    println!();
    for family in &families {
        let family_spend = family_totals[family];
        let emoji = category_emoji(Some(family), None);
        let label = family_label(family);
        println!("{emoji} *{}*   {}", label, brl(family_spend));

        let sub_rows = &family_rows[family];
        // Only show sub-breakdown when there are multiple sub-categories
        if sub_rows.len() > 1 {
            let mut sorted_sub = sub_rows.clone();
            sorted_sub.sort_by_key(|&(_, v)| Reverse(v));
            for (cat_id, spend) in &sorted_sub {
                // Strip family prefix to show only the sub-category part
                let sub_label = cat_id
                    .strip_prefix(family)
                    .and_then(|s| s.strip_prefix(':').or_else(|| s.strip_prefix('-')))
                    .unwrap_or(cat_id)
                    .replace(':', " > ")
                    .replace('-', " ");
                println!("  _{}   {}_", sub_label, brl(*spend));
            }
        }
    }

    println!();
    println!("*Total*: {}", brl(grand_total));

    // Top-3 category share breakdown
    if grand_total > Decimal::ZERO && families.len() > 1 {
        println!();
        let top3 = families.iter().take(3);
        for family in top3 {
            let share = family_totals[family] / grand_total * Decimal::from(100u32);
            println!("  {} {}%", family_label(family), pct(share));
        }
    }
}

async fn report_cashflow(args: CashflowArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.cashflow(args.months).await?;

    if args.structured_output() {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    print_cashflow_human(&rows, args.months);
    Ok(())
}

fn print_cashflow_human(rows: &[finance_core::models::CashflowRow], months: usize) {
    use human_format::{bold, brl, brl_signed, month_label};

    println!("📊 {}", bold(&format!("Cashflow · últimos {months} meses")));

    if rows.is_empty() {
        return;
    }

    println!();
    for row in rows {
        let net_emoji = if row.net > Decimal::ZERO {
            "✅"
        } else if row.net < Decimal::ZERO {
            "🔻"
        } else {
            "⚖️"
        };
        println!(
            "• *{}*  entradas {}  saídas {}  líquido {} {net_emoji}",
            month_label(&row.month_ref),
            brl(row.income),
            brl(-row.expenses),
            brl_signed(row.net),
        );
    }

    // Footer: average net + best/worst month
    let count = rows.len() as i64;
    if count > 0 {
        let total_net: Decimal = rows.iter().map(|r| r.net).sum();
        let avg_net = total_net / Decimal::from(count);

        let best = rows.iter().max_by_key(|r| r.net);
        let worst = rows.iter().min_by_key(|r| r.net);

        println!();
        println!("*Média mensal*: {}", brl_signed(avg_net));
        if let Some(b) = best {
            println!(
                "_Melhor mês_: {} ({})",
                month_label(&b.month_ref),
                brl_signed(b.net)
            );
        }
        if let Some(w) = worst {
            println!(
                "_Pior mês_: {} ({})",
                month_label(&w.month_ref),
                brl_signed(w.net)
            );
        }
    }
}

async fn report_forecast_vs_actual(args: ForecastVsActualArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.forecast_vs_actual(args.month.as_deref()).await?;

    if args.structured_output() {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    print_forecast_vs_actual_human(&rows, args.month.as_deref());
    Ok(())
}

fn print_forecast_vs_actual_human(
    rows: &[finance_core::models::ForecastVsActualRow],
    month: Option<&str>,
) {
    use human_format::{
        bold, brl, brl_signed, category_emoji, category_family, family_label, month_label,
        short_description,
    };
    use std::cmp::Reverse;
    use std::collections::HashMap;

    let month_display = month
        .map(month_label)
        .or_else(|| rows.first().map(|r| month_label(&r.month_ref)))
        .unwrap_or_else(|| "—".to_string());

    println!(
        "📊 {}",
        bold(&format!("Previsto vs Realizado · {month_display}"))
    );

    if rows.is_empty() {
        return;
    }

    // Group by category family
    let mut family_rows: HashMap<String, Vec<&finance_core::models::ForecastVsActualRow>> =
        HashMap::new();
    for row in rows {
        let family =
            category_family(row.category_id.as_deref()).unwrap_or_else(|| "outros".to_string());
        family_rows.entry(family).or_default().push(row);
    }

    // Sort families by absolute variance descending
    let mut families: Vec<String> = family_rows.keys().cloned().collect();
    families.sort_by_key(|f| {
        Reverse(
            family_rows[f]
                .iter()
                .map(|r| r.variance.abs())
                .fold(Decimal::ZERO, |acc, v| acc + v),
        )
    });

    println!();
    for family in &families {
        let fam_rows = &family_rows[family];
        let emoji = category_emoji(Some(family), None);
        let label = family_label(family);
        println!("{emoji} *{}*", label);

        // Sort items within family by absolute variance descending
        let mut sorted: Vec<&&finance_core::models::ForecastVsActualRow> =
            fam_rows.iter().collect();
        sorted.sort_by_key(|r| Reverse(r.variance.abs()));

        for row in sorted {
            let forecast = -row.forecast_amount;
            let actual = -row.actual_amount;
            let variance = -row.variance;

            // variance > 0 means over budget (spent more than forecast)
            let indicator = if variance > Decimal::ZERO {
                "🔻"
            } else if actual.abs() > forecast.abs() * Decimal::from(80u32) / Decimal::from(100u32) {
                "⚠️"
            } else {
                "✅"
            };

            let due_label = match row.due_date {
                Some(d) => format!("({})", d.format("%d/%m")),
                None => String::new(),
            };
            println!(
                "  {} {due_label} {indicator}  previsto {}  realizado {}  variação {}",
                short_description(&row.description),
                brl(forecast),
                brl(actual),
                brl_signed(variance),
            );
        }
    }

    // Footer totals
    let total_forecast: Decimal = rows.iter().map(|r| -r.forecast_amount).sum();
    let total_actual: Decimal = rows.iter().map(|r| -r.actual_amount).sum();
    let total_variance: Decimal = rows.iter().map(|r| -r.variance).sum();

    println!();
    println!(
        "*Total*  previsto {}  realizado {}  variação {}",
        brl(total_forecast),
        brl(total_actual),
        brl_signed(total_variance)
    );
}

async fn report_card_summary(args: CardSummaryArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.card_summary(args.month.as_deref()).await?;

    if args.structured_output() {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    let month_label_str = args
        .month
        .as_deref()
        .map(month_label)
        .unwrap_or_else(|| "—".to_string());
    println!("💳 {}", bold(&format!("Cartões · {month_label_str}")));
    println!();

    if rows.is_empty() {
        println!("Sem movimentos de cartão no período.");
        return Ok(());
    }

    let today = Utc::now().date_naive();
    let mut total_charges = Decimal::ZERO;

    for row in &rows {
        // Parse cycle window from month_ref (YYYY-MM → 13/prev → 12/curr)
        // We use the account_id as label (no technical UUID shown)
        let account_label = human_format::truncate_with_ellipsis(
            &human_format::short_description(&row.account_id),
            24,
        );
        let charged = -row.total_charges; // total_charges is stored negative
        let open = -row.open_amount;
        total_charges += charged;

        // Payment status emoji
        let status_emoji = if open <= Decimal::ZERO {
            "🟢"
        } else if open < charged {
            "🟡"
        } else {
            // fully open — check if month is past
            let month_ref_str = &row.month_ref;
            let is_past = NaiveDate::parse_from_str(&format!("{month_ref_str}-28"), "%Y-%m-%d")
                .map(|d| d < today)
                .unwrap_or(false);
            if is_past {
                "🔴"
            } else {
                "🟡"
            }
        };

        let paid_label = if open <= Decimal::ZERO {
            "pago".to_string()
        } else if open < charged {
            format!("aberto {}", hf_brl(open))
        } else {
            format!("em aberto {}", hf_brl(open))
        };

        println!(
            "{status_emoji} {} · {} · {}",
            bold(&account_label),
            hf_brl(charged),
            paid_label
        );
    }

    println!();
    println!("Total cobrado: {}", bold(&hf_brl(total_charges)));

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

    if args.structured_output() {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!(
        "🧾 {}",
        bold(&format!(
            "Fatura fechada · {}",
            month_label(&output.month_ref)
        ))
    );
    println!();

    if output.accounts.is_empty() {
        println!("Sem cartões com movimentos no mês.");
        return Ok(());
    }

    // ── Fechamento por cartão ──
    println!("{}", subsection_header("Fechamento por cartão"));
    for row in &output.accounts {
        let account_label = human_format::truncate_with_ellipsis(
            &human_format::short_description(&row.account_id),
            24,
        );
        let charged = -row.closed_amount;
        let open = -row.open_amount;
        let status = if open <= Decimal::ZERO {
            "🟢 pago".to_string()
        } else {
            format!("🟡 aberto {}", hf_brl(open))
        };
        println!(
            "• {} — {} · {}",
            bold(&account_label),
            hf_brl(charged),
            status
        );
    }

    // ── Top categorias ──
    println!();
    println!("{}", subsection_header("Top categorias"));
    if output.categories.is_empty() {
        println!("Sem categorias detectadas.");
    } else {
        for row in output.categories.iter().take(3) {
            let emoji = category_emoji(Some(&row.category_id), None);
            let label = row.category_id.replace(':', " › ").replace('-', " ");
            println!(
                "• {} {} — {} ({} txn)",
                emoji,
                label,
                hf_brl(-row.amount),
                row.transactions
            );
        }
    }

    // ── Assinaturas ──
    println!();
    println!("{}", subsection_header("Assinaturas"));
    if output.subscriptions.is_empty() {
        println!("Nenhuma assinatura detectada.");
    } else {
        for row in output.subscriptions.iter().take(8) {
            println!(
                "• {} — {}/mês",
                human_format::truncate_with_ellipsis(
                    &human_format::short_description(&row.merchant_key),
                    28
                ),
                hf_brl(-row.amount)
            );
        }
    }

    // ── Parcelamentos ──
    println!();
    println!("{}", subsection_header("Parcelamentos"));
    let total_open_installments = output
        .open_installments
        .iter()
        .fold(Decimal::ZERO, |acc, r| acc + (-r.amount));
    let n_open = output.open_installments.len();
    if n_open == 0 {
        println!("Sem parcelas em aberto.");
    } else {
        println!(
            "{} parcela{} ativa{}, {} restantes",
            n_open,
            if n_open == 1 { "" } else { "s" },
            if n_open == 1 { "" } else { "s" },
            hf_brl(total_open_installments)
        );
        for row in output.open_installments.iter().take(5) {
            println!(
                "• {} ({}) — {}",
                human_format::truncate_with_ellipsis(
                    &human_format::short_description(&row.merchant_key),
                    24
                ),
                row.marker,
                hf_brl(-row.amount)
            );
        }
    }
    if !output.closed_installments.is_empty() {
        println!();
        println!("{}", bold("Fechadas nesta fatura:"));
        for row in output.closed_installments.iter().take(5) {
            println!(
                "• {} ({}) — {}",
                human_format::truncate_with_ellipsis(
                    &human_format::short_description(&row.merchant_key),
                    24
                ),
                row.marker,
                hf_brl(-row.amount)
            );
        }
    }

    // ── Recorrentes ──
    println!();
    println!("{}", subsection_header("Recorrentes"));
    if output.recurring.is_empty() {
        println!("Nenhum recorrente detectado.");
    } else {
        for row in output.recurring.iter().take(6) {
            println!(
                "• {} — {} ({} meses)",
                human_format::truncate_with_ellipsis(
                    &human_format::short_description(&row.merchant_key),
                    28
                ),
                hf_brl(-row.amount),
                row.months_detected
            );
        }
    }

    Ok(())
}

async fn report_installments(args: InstallmentsArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let today = Utc::now().date_naive();
    let from = shift_month(
        NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
            .context("Falha ao calcular mês atual")?,
        -(args.lookback_months as i32),
    )?;
    let to = today;

    let transactions: Vec<_> = store
        .transactions_in_date_range(args.account_id.as_deref(), from, to)
        .await?
        .into_iter()
        .map(|mut tx| {
            tx.description = enrich_description_from_metadata(&tx.description, &tx.metadata_json);
            tx
        })
        .collect();

    let all_chains = group_into_chains(&transactions);
    // Report only active chains (remaining > 0)
    let active_chains: Vec<&InstallmentChain> = all_chains
        .iter()
        .filter(|chain| chain.remaining > 0)
        .collect();

    if args.structured_output() {
        if args.verbose {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &all_chains
                        .iter()
                        .filter(|c| c.remaining > 0)
                        .collect::<Vec<_>>()
                )?
            );
        } else {
            let views: Vec<InstallmentChainView> = active_chains
                .iter()
                .map(|chain| InstallmentChainView::from_chain(chain))
                .collect();
            println!("{}", serde_json::to_string_pretty(&views)?);
        }
        return Ok(());
    }

    println!("📦 *Parcelas ativas*");
    println!();

    if active_chains.is_empty() {
        println!("Nenhuma parcela ativa encontrada.");
        return Ok(());
    }

    let releasing: Vec<&&InstallmentChain> = active_chains
        .iter()
        .filter(|c| c.released_next_month)
        .collect();
    let ongoing: Vec<&&InstallmentChain> = active_chains
        .iter()
        .filter(|c| !c.released_next_month && c.remaining > 1)
        .collect();

    if !releasing.is_empty() {
        println!("🔔 *Liberam no próximo mês*");
        for chain in &releasing {
            println!(
                "{} · {}/{} · {} · termina {}",
                normalize_inline_text(&chain.base_description),
                chain.current,
                chain.total,
                brl_signed(chain.total_amount),
                short_date_pt(chain.projected_end),
            );
        }
        println!();
    }

    if !ongoing.is_empty() {
        let mut sorted_ongoing = ongoing.clone();
        sorted_ongoing.sort_by_key(|c| c.projected_end);
        println!("📅 *Em andamento*");
        for chain in &sorted_ongoing {
            println!(
                "{} · {}/{} · {} · termina {}",
                normalize_inline_text(&chain.base_description),
                chain.current,
                chain.total,
                brl_signed(chain.total_amount),
                short_date_pt(chain.projected_end),
            );
        }
        println!();
    }

    // Footer: total monthly commitment (next installment per chain = total_amount / total)
    let monthly_commitment: rust_decimal::Decimal = active_chains
        .iter()
        .map(|c| {
            if c.total > 0 {
                c.total_amount / rust_decimal::Decimal::from(c.total)
            } else {
                rust_decimal::Decimal::ZERO
            }
        })
        .fold(rust_decimal::Decimal::ZERO, |acc, v| acc + v);
    println!(
        "Compromisso mensal estimado: {}",
        brl_abs(monthly_commitment.abs())
    );

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

    if args.structured_output() {
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

    if args.structured_output() {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    let total_count = rows.len();
    let total_amount: rust_decimal::Decimal = rows
        .iter()
        .fold(rust_decimal::Decimal::ZERO, |acc, r| acc + r.amount);

    // Group by normalized description prefix (first 3 tokens, case-insensitive key)
    // Value: (display_label, total_amount, count, latest_date)
    let mut groups: std::collections::BTreeMap<
        String,
        (String, rust_decimal::Decimal, usize, chrono::NaiveDate),
    > = std::collections::BTreeMap::new();
    for row in &rows {
        let prefix: Vec<&str> = row.description.split_whitespace().take(3).collect();
        let key = prefix.join(" ").to_ascii_uppercase();
        let display = normalize_inline_text(&row.description);
        let entry = groups.entry(key).or_insert((
            display,
            rust_decimal::Decimal::ZERO,
            0,
            row.transaction_date,
        ));
        entry.1 += row.amount;
        entry.2 += 1;
        if row.transaction_date > entry.3 {
            entry.3 = row.transaction_date;
        }
    }

    // Sort by absolute total descending
    let mut sorted_groups: Vec<(String, rust_decimal::Decimal, usize, chrono::NaiveDate)> = groups
        .into_iter()
        .map(|(_, (label, amount, count, latest))| (label, amount, count, latest))
        .collect();
    sorted_groups.sort_by_key(|(_, amount, _, _)| std::cmp::Reverse(amount.abs()));

    println!("❓ *Sem categoria · top {}*", args.limit);
    println!();

    for (desc, amount, count, latest) in &sorted_groups {
        println!(
            "{} ({}×) · {} · mais recente {}",
            desc,
            count,
            brl_abs(*amount),
            short_date_pt(*latest),
        );
    }

    println!();
    println!(
        "Total: {} transações · {}",
        total_count,
        brl_abs(total_amount.abs())
    );

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

    if args.structured_output() {
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

    if args.structured_output() {
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

    if args.structured_output() {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Build check items: (severity 0=fail 1=warn 2=ok, emoji, description)
    let mut checks: Vec<(u8, &str, String)> = Vec::new();

    // Pluggy lag
    match output.pluggy_lag_days {
        Some(days) if days > 3 => {
            checks.push((0, "🔻", format!("Pluggy desatualizado · lag {} dias", days)))
        }
        Some(days) if days > 1 => checks.push((
            1,
            "🟡",
            format!("Pluggy levemente atrasado · lag {} dias", days),
        )),
        Some(_) => checks.push((2, "✅", "Pluggy atualizado".to_string())),
        None => checks.push((1, "🟡", "Sem data Pluggy registrada".to_string())),
    }

    // Uncategorized
    if output.uncategorized_count > 10 {
        checks.push((
            0,
            "🔻",
            format!("Sem categoria · {} transações", output.uncategorized_count),
        ));
    } else if output.uncategorized_count > 0 {
        checks.push((
            1,
            "🟡",
            format!("Sem categoria · {} transações", output.uncategorized_count),
        ));
    } else {
        checks.push((2, "✅", "Todas as transações categorizadas".to_string()));
    }

    // Flat categories
    if output.flat_category_rows > 0 {
        checks.push((
            1,
            "🟡",
            format!(
                "Categorias planas (sem hierarquia) · {} linhas",
                output.flat_category_rows
            ),
        ));
    } else {
        checks.push((2, "✅", "Sem categorias planas".to_string()));
    }

    // Overlaps
    if output.overlap_candidates_count > 0 {
        checks.push((
            0,
            "🔻",
            format!(
                "Sobreposições legacy×pluggy · {} candidatos",
                output.overlap_candidates_count
            ),
        ));
    } else {
        checks.push((2, "✅", "Sem sobreposições detectadas".to_string()));
    }

    // Context coverage
    let coverage_pct = output.context_coverage_ratio * 100.0;
    if coverage_pct < 50.0 {
        checks.push((
            1,
            "🟡",
            format!("Cobertura de contexto baixa · {:.0}%", coverage_pct),
        ));
    } else {
        checks.push((
            2,
            "✅",
            format!("Cobertura de contexto · {:.0}%", coverage_pct),
        ));
    }

    // Sort: failures first, then warnings, then ok
    checks.sort_by_key(|(severity, _, _)| *severity);

    let total_checks = checks.len();
    let problems = checks.iter().filter(|(s, _, _)| *s < 2).count();

    println!("🩹 *Saúde dos dados*");
    println!();

    if problems == 0 {
        println!("✅ Tudo certo · {} verificações.", total_checks);
    } else {
        for (_, emoji, desc) in &checks {
            println!("{} {}", emoji, desc);
        }

        if !output.overlap_candidates.is_empty() {
            println!();
            println!("*Sobreposições detectadas:*");
            for row in &output.overlap_candidates {
                println!(
                    "  {} · legacy {} · pluggy {}",
                    row.account_id.as_deref().unwrap_or("sem-conta"),
                    row.legacy_date,
                    row.pluggy_date,
                );
            }
        }

        println!();
        println!("{} problema(s) em {} verificações.", problems, total_checks);
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

    if args.structured_output() {
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
        enrichment_attempted_at: None,
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

async fn tx_enrich(args: enrich::EnrichArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    enrich::run(args, &config, store.as_ref()).await
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

fn print_transaction_row(row: &TransactionRecord) {
    let account = row.account_id.as_deref().unwrap_or("sem-conta");
    println!(
        "{} | {} | {} | {} | {} | {:?}",
        row.transaction_id,
        row.transaction_date.format("%Y-%m-%d"),
        decimal_text(row.amount),
        row.description,
        account,
        row.context,
    );
}

async fn tx_find(args: TxFindArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let rows = store
        .find_transactions_by_description(&args.query, args.limit)
        .await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("Transactions matching {:?}", args.query);
    println!("- linhas: {}", rows.len());
    println!();
    for row in &rows {
        print_transaction_row(row);
    }
    Ok(())
}

async fn tx_pending(args: TxPendingArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let rows = store.latest_uncategorized_transactions(args.limit).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("Pending (no context) transactions");
    println!("- linhas: {}", rows.len());
    println!();
    for row in &rows {
        print_transaction_row(row);
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SetContextByDescResult {
    transaction_id: String,
    description: String,
    old_context: Option<String>,
    new_context: String,
}

async fn tx_set_context_by_desc(args: SetContextByDescArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let rows = store
        .find_transactions_by_description(&args.query, 100)
        .await?;

    let results: Vec<SetContextByDescResult> = rows
        .iter()
        .map(|row| SetContextByDescResult {
            transaction_id: row.transaction_id.clone(),
            description: row.description.clone(),
            old_context: row.context.clone(),
            new_context: args.context.clone(),
        })
        .collect();

    if args.dry_run {
        if args.json {
            println!("{}", serde_json::to_string_pretty(&results)?);
        } else {
            println!("Dry-run: {} transaction(s) would be updated", results.len());
            for r in &results {
                println!(
                    "{} | {} | {:?} -> {:?}",
                    r.transaction_id, r.description, r.old_context, r.new_context
                );
            }
        }
        return Ok(());
    }

    // Real run: apply context to each matching transaction
    let mut audit = Vec::new();
    for row in &rows {
        let idempotency_key = format!(
            "context-by-desc:{}:{}:{}",
            row.transaction_id,
            args.context,
            Uuid::now_v7()
        );
        store
            .annotate_transaction(
                &row.transaction_id,
                None,
                Some("manual"),
                Some(&args.context),
                &config.actor_id,
                &idempotency_key,
            )
            .await?;
        audit.push(AuditEvent::from_entity(
            "transaction",
            &row.transaction_id,
            "set_context_by_desc",
            &config.actor_id,
            &idempotency_key,
            json!({
                "query": args.query,
                "context": args.context,
                "old_context": row.context,
            }),
        ));
    }
    insert_audit_chunked(store.as_ref(), &audit).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        println!("Contexto atualizado para {} transação(ões)", results.len());
        for r in &results {
            println!(
                "{} | {} | {:?} -> {:?}",
                r.transaction_id, r.description, r.old_context, r.new_context
            );
        }
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

async fn budget_upsert(args: BudgetUpsertArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    if let Some(ref month) = args.month {
        parse_month_ref(month)?;
    }

    let amount = decimal_from_str(&args.amount)?;
    let now = Utc::now();
    let budget_id = format!("budget_{}", uuid::Uuid::now_v7());
    let idempotency_key = format!(
        "budget:{}:{}:{}",
        args.category_id,
        args.subcategory_id.as_deref().unwrap_or(""),
        args.month.as_deref().unwrap_or("_default"),
    );
    let record = CategoryBudgetRecord {
        budget_id: budget_id.clone(),
        category_id: args.category_id.clone(),
        subcategory_id: args.subcategory_id.clone(),
        month_ref: args.month.clone(),
        amount,
        alert_threshold_pct: args.alert_threshold_pct,
        actor_id: config.actor_id.clone(),
        idempotency_key: idempotency_key.clone(),
        created_at: now,
        updated_at: now,
    };
    store.upsert_category_budget(&record).await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "category_budget",
            &budget_id,
            "upsert",
            &config.actor_id,
            &idempotency_key,
            serde_json::to_value(&record)?,
        )])
        .await?;
    println!(
        "Budget salvo: {} (category: {}, month: {})",
        budget_id,
        args.category_id,
        args.month.as_deref().unwrap_or("_default"),
    );
    Ok(())
}

async fn budget_list(args: BudgetListArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    let rows = store.list_category_budgets(args.month.as_deref()).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "Budgets{}",
        args.month
            .as_deref()
            .map(|m| format!(" {m}"))
            .unwrap_or_default()
    );
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        let subcat = row.subcategory_id.as_deref().unwrap_or("-");
        let month = row.month_ref.as_deref().unwrap_or("_default");
        println!(
            "{} | sub: {} | month: {} | budget: {} | threshold: {}%",
            row.category_id,
            subcat,
            month,
            brl(row.amount),
            row.alert_threshold_pct,
        );
    }
    Ok(())
}

async fn report_budget_status(args: BudgetStatusArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    parse_month_ref(&args.month)?;
    let rows: Vec<BudgetStatusRow> = store.budget_status_for_month(&args.month).await?;

    if args.structured_output() {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "🎯 {}",
        bold(&format!("Orçamentos · {}", month_label(&args.month)))
    );
    println!();

    if rows.is_empty() {
        println!("Nenhum orçamento configurado para o período.");
        return Ok(());
    }

    // Sort: over budget first, then alert, then by usage_pct descending
    let mut sorted = rows.clone();
    sorted.sort_by_key(|r| {
        let tier: i32 = if r.actual_amount > r.budget_amount {
            0 // over
        } else if r.alert {
            1 // near threshold
        } else {
            2 // ok
        };
        let usage_inv = -(r.usage_pct.to_string().parse::<f64>().unwrap_or(0.0) as i64);
        (tier, usage_inv)
    });

    let mut total_budget = Decimal::ZERO;
    let mut total_actual = Decimal::ZERO;

    for row in &sorted {
        let usage_i64 = row
            .usage_pct
            .to_string()
            .parse::<f64>()
            .map(|v| v as i64)
            .unwrap_or(0);
        let bar = progress_bar(usage_i64);
        let status_emoji = if row.actual_amount > row.budget_amount {
            "🔻"
        } else if row.alert {
            "🟡"
        } else {
            "✅"
        };
        let label = row.category_id.replace(':', " › ").replace('-', " ");
        println!(
            "{} {} — {} / {} ({}%)",
            status_emoji,
            bold(&label),
            hf_brl(row.actual_amount),
            hf_brl(row.budget_amount),
            usage_i64
        );
        println!("   {bar}");
        total_budget += row.budget_amount;
        total_actual += row.actual_amount;
    }

    println!();
    println!(
        "Total: {} de {} orçados",
        bold(&hf_brl(total_actual)),
        hf_brl(total_budget)
    );

    Ok(())
}

// ─── report cards ──────────────────────────────────────────────────────────
//
// Unified credit-card report. Supersedes the deprecated `card-summary` and
// `card-closed-insights` reports.
//
// Sections (default human output):
//   1. Visão geral — per-card totals + payment status (paid / open / overdue)
//   2. Parceladas — active installment chains: total commitment, what's new
//      this month, what frees up next month
//   3. Gastos por categoria — every charge in the month grouped by family,
//      then by category, sorted by spend descending
//
// Payment status is inferred by searching checking accounts for an outgoing
// debit categorized as `credit-card-payment` (or whatever internal category
// the user's classifier rules assign to bill payments) within a ±15-day
// window of the bill close date, matching the bill total within ±5%.

#[derive(Debug, Clone, serde::Serialize)]
struct CardsBillStatus {
    /// `paid`, `open`, `overdue`, or `unknown`
    state: &'static str,
    /// Date the bill closes (start of the next cycle, ~billing_closing_day).
    /// Inferred as the last day of the report month when we don't know better.
    close_date: NaiveDate,
    /// Date the bill is due. Inferred as close_date + 7 days when we don't
    /// have a `billing_due_day` for the account.
    due_date: NaiveDate,
    /// When `state = paid`, the date we believe the user paid the bill.
    paid_on: Option<NaiveDate>,
    /// Total of the bill in the report month.
    total: Decimal,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CardsAccountReport {
    account_id: String,
    transaction_count: usize,
    status: CardsBillStatus,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CardsReport {
    month_ref: String,
    accounts: Vec<CardsAccountReport>,
    grand_total: Decimal,
}

/// Extract Pluggy's `creditCardMetadata.billId` from a transaction metadata blob.
fn extract_bill_id(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("raw")
        .and_then(|v| v.get("creditCardMetadata"))
        .and_then(|v| v.get("billId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Compute a synthetic bill identifier from a transaction date and the
/// account's billing closing day. Brazilian credit cards conventionally
/// attribute transactions posted ON the closing day to the *next* cycle
/// (Nubank's OFX statements confirm: DTSTART = closing_day_prev_month means
/// transactions on that date are the first in the new cycle). So:
/// transactions on day < closing_day belong to the bill closing this month;
/// transactions on day >= closing_day belong to next month's bill.
fn compute_bill_id(date: NaiveDate, closing_day: u32) -> String {
    let (year, month, day) = (date.year(), date.month(), date.day());
    let (bill_year, bill_month) = if day < closing_day {
        (year, month)
    } else if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    format!("cycle-{}-{:02}-{:02}", bill_year, bill_month, closing_day)
}

/// Parse `billing_closing_day` from account metadata JSON (stored as a string
/// or integer for forward compat).  Only values in 1..=28 are accepted.
fn parse_closing_day(metadata: &serde_json::Value) -> Option<u32> {
    let v = metadata.get("billing_closing_day")?;
    match v {
        serde_json::Value::String(s) => s.parse::<u32>().ok(),
        serde_json::Value::Number(n) => n.as_u64().map(|d| d as u32),
        _ => None,
    }
    .filter(|d| (1..=28).contains(d))
}

/// Infer the account's billing closing day by inspecting Pluggy-supplied
/// billId clusters in the transaction data. Pluggy tags some (~20%) of
/// credit-card transactions with `creditCardMetadata.billId`. Each unique
/// billId is a real bill; its `max(transaction_date)` is the last day of
/// the cycle, and the bill itself closes 1–2 days later (Pluggy stops
/// posting once the cycle ends).
///
/// Returns the *mode* of (max_date.day + 2), clamped to 1..=28. Returns
/// None when fewer than 2 Pluggy-tagged bills are observed (not enough
/// signal to commit to a value).
fn infer_closing_day_from_pluggy(rows: &[CardClosedTransactionRow]) -> Option<u32> {
    use std::collections::HashMap;
    let mut bill_max: HashMap<String, NaiveDate> = HashMap::new();
    for row in rows {
        if let Some(bill_id) = extract_bill_id(&row.metadata_json) {
            let entry = bill_max.entry(bill_id).or_insert(row.transaction_date);
            if row.transaction_date > *entry {
                *entry = row.transaction_date;
            }
        }
    }
    if bill_max.len() < 2 {
        return None;
    }
    let mut day_counts: HashMap<u32, u32> = HashMap::new();
    for max_date in bill_max.values() {
        let close_day = (max_date.day() + 2).min(28);
        *day_counts.entry(close_day).or_insert(0) += 1;
    }
    day_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(day, _)| day)
}

async fn report_cards(args: CardsArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let accounts = store.get_accounts().await?;
    let mut closing_days: BTreeMap<String, u32> = accounts
        .iter()
        .filter_map(|a| parse_closing_day(&a.metadata_json).map(|d| (a.account_id.clone(), d)))
        .collect();

    let today = Utc::now().date_naive();
    let (mode, target_month) = match (&args.month, args.closed, args.next) {
        (Some(value), _, _) => {
            parse_month_ref(value)?;
            (CardsMode::Specific, value.to_string())
        }
        (None, _, true) => (CardsMode::Next, format!("{}", today.format("%Y-%m"))),
        _ => (
            CardsMode::Closed,
            format!("{}", today.format("%Y-%m")), // placeholder; we'll resolve later from billId clusters
        ),
    };

    // Fetch a wide window so we capture full bill cycles regardless of where
    // they straddle calendar months. 90 days covers the most-recent-closed
    // bill (~30d) plus the cycle before it as fallback.
    let window_end = today;
    let window_start = today
        .checked_sub_signed(Duration::days(90))
        .unwrap_or(today);
    let mut all_rows: Vec<CardClosedTransactionRow> = Vec::new();
    // Existing storage API is month-scoped; iterate over months in the window.
    let mut cursor = window_start;
    while cursor <= window_end {
        let m = format!("{}", cursor.format("%Y-%m"));
        let txs = store.card_closed_transactions(Some(&m)).await?;
        all_rows.extend(txs);
        cursor = shift_month(cursor, 1).unwrap_or(window_end + Duration::days(1));
    }

    // Optional --account-id filter
    if let Some(id) = args.account_id.as_deref() {
        all_rows.retain(|r| r.account_id == id);
    }

    // Optional --installments-only filter: keep only rows that carry an
    // installment marker in their label, description, or Pluggy metadata.
    if args.installments_only {
        all_rows.retain(|r| detect_installment_marker(r).is_some());
    }

    // For each account that doesn't have a configured `billing_closing_day`
    // in metadata, infer one from the Pluggy billId clusters in the data.
    // This makes the report work out of the box on accounts that were never
    // explicitly configured, falling back to the explicit value when it
    // exists.
    let mut rows_by_account: BTreeMap<String, Vec<&CardClosedTransactionRow>> = BTreeMap::new();
    for row in &all_rows {
        rows_by_account
            .entry(row.account_id.clone())
            .or_default()
            .push(row);
    }
    for (acct, rows_ref) in &rows_by_account {
        if closing_days.contains_key(acct) {
            continue;
        }
        let owned: Vec<CardClosedTransactionRow> = rows_ref.iter().map(|r| (*r).clone()).collect();
        if let Some(d) = infer_closing_day_from_pluggy(&owned) {
            closing_days.insert(acct.clone(), d);
        }
    }

    // Group by (account_id, bill_id).  Resolution order:
    //   1. Account-level billing_closing_day (explicit or inferred) →
    //      synthetic cycle bill_id (best, because it consolidates the
    //      ~80% of Pluggy txs without a billId into the same bill as
    //      the ~20% that do have one).
    //   2. Pluggy's creditCardMetadata.billId (kept as a fallback for
    //      accounts where we couldn't infer a closing day).
    //   3. Calendar-month fallback (no-bill-YYYY-MM).
    type BillKey = (String, String);
    type BillBucket = Vec<CardClosedTransactionRow>;
    let mut bills: BTreeMap<BillKey, BillBucket> = BTreeMap::new();
    for row in &all_rows {
        let bill_id = closing_days
            .get(&row.account_id)
            .map(|d| compute_bill_id(row.transaction_date, *d))
            .or_else(|| extract_bill_id(&row.metadata_json))
            .unwrap_or_else(|| format!("no-bill-{}", row.transaction_date.format("%Y-%m")));
        bills
            .entry((row.account_id.clone(), bill_id))
            .or_default()
            .push(row.clone());
    }

    // Per account, list of bill candidates with the date we'll use to order
    // and classify them.
    struct BillCandidate {
        account_id: String,
        /// Last transaction date seen in this bill. Used for display ("closed
        /// on...") and as a fallback ordering key.
        max_date: NaiveDate,
        /// Authoritative cycle close date, derived from the synthetic
        /// bill_id format `cycle-YYYY-MM-DD` when available. When None we
        /// fall back to `max_date` for ordering.
        derived_close_date: Option<NaiveDate>,
        txs: Vec<CardClosedTransactionRow>,
    }
    /// Parse a synthetic bill_id of the form `cycle-YYYY-MM-DD` into the
    /// underlying close date. Returns None for Pluggy-real billIds or the
    /// no-bill fallback.
    fn parse_synthetic_close_date(bill_id: &str) -> Option<NaiveDate> {
        let rest = bill_id.strip_prefix("cycle-")?;
        NaiveDate::parse_from_str(rest, "%Y-%m-%d").ok()
    }
    let candidates: Vec<BillCandidate> = bills
        .into_iter()
        .map(|((account_id, bill_id), txs)| {
            let max_date = txs
                .iter()
                .map(|t| t.transaction_date)
                .max()
                .unwrap_or(today);
            let derived_close_date = parse_synthetic_close_date(&bill_id);
            BillCandidate {
                account_id,
                max_date,
                derived_close_date,
                txs,
            }
        })
        .collect();

    // Pick the right bill per card based on mode.
    let mut per_account_unique: BTreeMap<String, Vec<&BillCandidate>> = BTreeMap::new();
    for c in &candidates {
        per_account_unique
            .entry(c.account_id.clone())
            .or_default()
            .push(c);
    }
    let mut selected: Vec<&BillCandidate> = Vec::new();
    for bills_for_account in per_account_unique.values() {
        // Bills with a synthetic close_date split cleanly by today's date:
        // close_date <= today → closed; close_date > today → open. This is
        // the authoritative signal when `billing_closing_day` is set on the
        // account. For accounts/bills without a synthetic close_date (e.g.
        // Pluggy real billIds or the no-bill fallback) we fall back to
        // max_date ordering as before.
        let pick: Option<&BillCandidate> = match mode {
            CardsMode::Closed => {
                // Most recent bill whose close_date <= today.
                let with_close: Vec<&&BillCandidate> = bills_for_account
                    .iter()
                    .filter(|b| b.derived_close_date.is_some_and(|d| d <= today))
                    .collect();
                if !with_close.is_empty() {
                    with_close
                        .into_iter()
                        .max_by_key(|b| b.derived_close_date.unwrap_or(b.max_date))
                        .copied()
                } else {
                    // Fallback: sort by max_date desc and skip the latest
                    // (assumed to be the currently open cycle). If only one
                    // bucket exists (e.g. a new card), use it rather than
                    // silently omitting the account from the report.
                    let mut sorted: Vec<&BillCandidate> = bills_for_account.to_vec();
                    sorted.sort_by_key(|b| std::cmp::Reverse(b.max_date));
                    sorted.get(1).or_else(|| sorted.first()).copied()
                }
            }
            CardsMode::Next => {
                // Most recent bill whose close_date > today (the cycle still
                // accruing charges). Fall back to "bill with the latest
                // max_date" if no synthetic close dates are available.
                let with_close: Vec<&&BillCandidate> = bills_for_account
                    .iter()
                    .filter(|b| b.derived_close_date.is_some_and(|d| d > today))
                    .collect();
                if !with_close.is_empty() {
                    with_close
                        .into_iter()
                        .min_by_key(|b| b.derived_close_date.unwrap_or(b.max_date))
                        .copied()
                } else {
                    bills_for_account.iter().copied().max_by_key(|b| b.max_date)
                }
            }
            CardsMode::Specific => {
                // Pick the bill whose close_date (or max_date fallback) falls
                // in the target month.
                bills_for_account.iter().copied().find(|b| {
                    let key = b.derived_close_date.unwrap_or(b.max_date);
                    key.format("%Y-%m").to_string() == target_month
                })
            }
        };
        if let Some(b) = pick {
            selected.push(b);
        }
    }

    // Re-derive `target_month` from selected bills for display: use the
    // most recent max_date among the selected.
    let display_month = selected
        .iter()
        .map(|b| b.max_date)
        .max()
        .map(|d| d.format("%Y-%m").to_string())
        .unwrap_or_else(|| target_month.clone());

    // Collect rows from the selected bills for the per-category breakdown.
    let rows: Vec<CardClosedTransactionRow> = selected
        .iter()
        .flat_map(|b| b.txs.iter().cloned())
        .collect();

    // Group rows by account_id for the visão geral section.
    let mut by_account: BTreeMap<String, Vec<CardClosedTransactionRow>> = BTreeMap::new();
    for row in &rows {
        by_account
            .entry(row.account_id.clone())
            .or_default()
            .push(row.clone());
    }

    // Map account_id → derived_close_date of the bill we selected for that
    // account, so the status section can use the *real* cycle close date
    // rather than approximating from the last-seen transaction.
    let selected_close_dates: BTreeMap<String, Option<NaiveDate>> = selected
        .iter()
        .map(|b| (b.account_id.clone(), b.derived_close_date))
        .collect();

    // For payment inference: gather all checking-account debits classified
    // as bill payments in a window covering the cycle close dates onward.
    let pay_window_start = window_start;
    let pay_window_end = today + Duration::days(20);
    // Identify bill payments by either:
    //  (a) category_id resembles `credit-card-payment` / `pagamento-fatura`, or
    //  (b) description starts with "Pagamento de fatura" (Pluggy's canonical
    //      text for an outgoing CC payment from checking). Many setups don't
    //      classify these (category_id stays NULL), so description match is
    //      the most robust signal.
    let payment_candidates: Vec<TransactionRecord> = store
        .transactions_in_date_range(None, pay_window_start, pay_window_end)
        .await?
        .into_iter()
        .filter(|tx| {
            if tx.amount >= Decimal::ZERO {
                return false;
            }
            if tx.category_id.as_deref().is_some_and(|c| {
                c.contains("credit-card-payment") || c.contains("pagamento-fatura")
            }) {
                return true;
            }
            let desc_lower = tx.description.to_lowercase();
            desc_lower.contains("pagamento de fatura")
                || desc_lower.contains("pagamento cart")
                || desc_lower.contains("pagamento de cart")
                || desc_lower.contains("nubank pagamento")
        })
        .collect();

    // Build per-account report
    let mut accounts_report = Vec::with_capacity(by_account.len());
    let mut grand_total = Decimal::ZERO;
    for (account_id, txs) in &by_account {
        // Signed sum: debits are negative, statement credits (IOF reversals,
        // merchant refunds) are positive. They net automatically, which is
        // what shows up on the actual bill the user receives.
        let net_spend: Decimal = txs.iter().map(|t| t.amount).sum();
        // For display we keep using the absolute value of `net_spend` —
        // the human output explicitly prefixes "-R$ ..." so showing the
        // unsigned amount and letting the formatter add the sign is what
        // matches the rest of the report styling.
        let total: Decimal = net_spend.abs();
        grand_total += total;

        // Prefer the synthetic cycle close date when we have it (derived
        // from the account's billing_closing_day). Otherwise fall back to
        // max(transaction_date) — Pluggy stops adding new charges once the
        // cycle ends, so the last-seen date is a tight upper bound.
        let close_date = selected_close_dates
            .get(account_id)
            .copied()
            .flatten()
            .unwrap_or_else(|| {
                txs.iter()
                    .map(|t| t.transaction_date)
                    .max()
                    .unwrap_or(today)
            });
        // Due date approximated as close + 7 days (typical for Brazilian
        // cards). Would refine further with `billing_due_day` metadata.
        let due_date = close_date
            .checked_add_signed(Duration::days(7))
            .unwrap_or(close_date);

        let status = if mode == CardsMode::Next {
            // Open cycle (--next): the bill hasn't closed yet. We don't try
            // to infer payment status — we just show how the cycle is
            // forming up.
            CardsBillStatus {
                state: "partial",
                close_date,
                due_date,
                paid_on: None,
                total,
            }
        } else if today < close_date {
            CardsBillStatus {
                state: "open",
                close_date,
                due_date,
                paid_on: None,
                total,
            }
        } else {
            // Bill closed — try to match a payment. We allow a generous 10%
            // tolerance to absorb the gap between calendar-month totals and
            // actual statement cycle totals (which include the previous
            // month's tail and exclude the current month's tail). Description
            // already filters to canonical "Pagamento de fatura" lines, so
            // false positives are unlikely.
            let tolerance = total * Decimal::new(10, 2);
            let lower = total - tolerance;
            let upper = total + tolerance;
            let paid = payment_candidates
                .iter()
                .filter(|t| t.transaction_date >= close_date.saturating_sub_signed_unsafe())
                .filter(|t| {
                    let abs = t.amount.abs();
                    abs >= lower && abs <= upper
                })
                .min_by_key(|t| (t.transaction_date - close_date).num_days().unsigned_abs());
            match paid {
                Some(tx) => CardsBillStatus {
                    state: "paid",
                    close_date,
                    due_date,
                    paid_on: Some(tx.transaction_date),
                    total,
                },
                None if today > due_date => CardsBillStatus {
                    state: "overdue",
                    close_date,
                    due_date,
                    paid_on: None,
                    total,
                },
                None => CardsBillStatus {
                    state: "open",
                    close_date,
                    due_date,
                    paid_on: None,
                    total,
                },
            }
        };

        accounts_report.push(CardsAccountReport {
            account_id: account_id.clone(),
            transaction_count: txs.len(),
            status,
        });
    }

    let report = CardsReport {
        month_ref: display_month.clone(),
        accounts: accounts_report,
        grand_total,
    };

    if args.structured_output() {
        // Raw output: include both summary and the full transaction list so
        // agents have everything they need.
        let payload = serde_json::json!({
            "summary": report,
            "transactions": rows,
            // Debug: expose every candidate bill so a tooling caller can
            // verify the per-card grouping logic, not just the one we
            // selected. Will be removed once the report is stable.
            "all_candidates": candidates.iter().map(|b| {
                serde_json::json!({
                    "account_id": b.account_id,
                    "min_date": b.txs.iter().map(|t| t.transaction_date).min().map(|d| d.to_string()),
                    "max_date": b.max_date.to_string(),
                    "derived_close_date": b.derived_close_date.map(|d| d.to_string()),
                    "transaction_count": b.txs.len(),
                    "total": b.txs.iter().map(|t| t.amount.abs()).sum::<Decimal>().to_string(),
                    "bill_id_sample": b.txs.first().and_then(|t| extract_bill_id(&t.metadata_json)),
                })
            }).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    // ─── Human output ──────────────────────────────────────────────────────
    let mode_suffix = match mode {
        CardsMode::Closed => " (fechado)",
        CardsMode::Next => " (em curso)",
        CardsMode::Specific => "",
    };
    let installments_suffix = if args.installments_only {
        " · só parceladas"
    } else {
        ""
    };
    println!(
        "💳 {}",
        bold(&format!(
            "Cartões · {}{}{}",
            month_label(&display_month),
            mode_suffix,
            installments_suffix
        ))
    );
    println!();

    if rows.is_empty() {
        println!("_(sem lançamentos de cartão no período)_");
        return Ok(());
    }

    // Visão geral
    println!("{}", subsection_header("Visão geral"));
    for acct in &report.accounts {
        let s = &acct.status;
        let (emoji, status_text) = match s.state {
            "paid" => (
                "🟢",
                format!(
                    "pago {}",
                    s.paid_on
                        .map(human_format::short_date)
                        .unwrap_or_else(|| "—".to_string())
                ),
            ),
            "open" => (
                "🟡",
                format!("em aberto · vence {}", human_format::short_date(s.due_date)),
            ),
            "overdue" => (
                "🔴",
                format!("ATRASADO · vencia {}", human_format::short_date(s.due_date)),
            ),
            "partial" => {
                let days_to_close = (s.close_date - today).num_days();
                let when = if days_to_close > 0 {
                    format!("fecha em ~{days_to_close}d")
                } else {
                    "fechando agora".to_string()
                };
                ("🟡", format!("em curso · {when}"))
            }
            _ => ("⚪", "status desconhecido".to_string()),
        };
        println!(
            "{} {} · {} ({} lanç) · {}",
            emoji,
            bold(&acct.account_id),
            human_format::brl_signed(-s.total),
            acct.transaction_count,
            status_text,
        );
    }
    println!(
        "{}: {}",
        bold("Total"),
        human_format::brl_signed(-report.grand_total)
    );
    println!();

    // Gastos por categoria — family → subcategory → transactions
    println!("{}", subsection_header("Gastos por categoria"));
    let mut by_family: BTreeMap<String, Vec<&CardClosedTransactionRow>> = BTreeMap::new();
    for row in &rows {
        let family = human_format::category_family(row.category_id.as_deref())
            .unwrap_or_else(|| "sem-categoria".to_string());
        by_family.entry(family).or_default().push(row);
    }
    let mut family_totals: Vec<(String, Decimal, Vec<&CardClosedTransactionRow>)> = by_family
        .into_iter()
        .map(|(f, list)| {
            // Signed sum: refunds (positive) net against charges (negative)
            // so the category total reflects the actual cost the user paid
            // in that category for the cycle.
            let net: Decimal = list.iter().map(|r| r.amount).sum();
            (f, net, list)
        })
        .collect();
    // Sort by absolute net descending — categories with the biggest
    // movement (positive or negative) come first.
    family_totals.sort_by_key(|(_, net, _)| std::cmp::Reverse(net.abs()));

    const TX_LIMIT: usize = 5;

    for (family, family_net, list) in &family_totals {
        let repr_cat = list.first().and_then(|r| r.category_id.as_deref());
        let emoji = human_format::category_emoji(repr_cat, Some(*family_net));
        let label = human_format::family_label(family);
        println!(
            "{} {} · {} ({} lanç)",
            emoji,
            bold(&label),
            human_format::brl_signed(*family_net),
            list.len(),
        );

        // Group this family's rows by subcategory. Subcategory comes from
        // the part after the first `:` in category_id. Pluggy-default ids
        // (no `:`) bucket under "—".
        let mut by_sub: BTreeMap<String, Vec<&&CardClosedTransactionRow>> = BTreeMap::new();
        for row in list {
            let sub = row
                .category_id
                .as_deref()
                .and_then(|c| c.split_once(':').map(|(_, s)| s.to_string()))
                .unwrap_or_else(|| "—".to_string());
            by_sub.entry(sub).or_default().push(row);
        }

        // If there's only one subcategory bucket (or only "—"), skip the
        // intermediate header and list transactions directly under the
        // family. Otherwise render each subcategory as a nested section.
        let render_subs = by_sub.len() > 1 || !by_sub.contains_key("—");

        if render_subs {
            let mut sub_buckets: Vec<(String, Decimal, Vec<&&CardClosedTransactionRow>)> = by_sub
                .into_iter()
                .map(|(name, items)| {
                    let net: Decimal = items.iter().map(|r| r.amount).sum();
                    (name, net, items)
                })
                .collect();
            sub_buckets.sort_by_key(|(_, net, _)| std::cmp::Reverse(net.abs()));

            for (sub_name, sub_net, sub_items) in sub_buckets {
                let sub_label = if sub_name == "—" {
                    "sem subcategoria".to_string()
                } else {
                    sub_name.replace('-', " ")
                };
                println!(
                    "  ↳ {} · {} ({} lanç)",
                    bold(&sub_label),
                    human_format::brl_signed(sub_net),
                    sub_items.len(),
                );
                let mut sorted_sub = sub_items.clone();
                sorted_sub.sort_by_key(|r| std::cmp::Reverse(r.amount.abs()));
                let take_n = if args.all { sorted_sub.len() } else { TX_LIMIT };
                for tx in sorted_sub.iter().take(take_n) {
                    println!(
                        "      • {} · {} ({})",
                        human_format::short_description(&tx.description),
                        human_format::brl_signed(tx.amount),
                        human_format::short_date(tx.transaction_date),
                    );
                }
                if !args.all && sorted_sub.len() > TX_LIMIT {
                    println!(
                        "      _… mais {} {}_",
                        sorted_sub.len() - TX_LIMIT,
                        if sorted_sub.len() - TX_LIMIT == 1 {
                            "lançamento"
                        } else {
                            "lançamentos"
                        }
                    );
                }
            }
        } else {
            // Single bucket (only "—") — list flat under family.
            let mut sorted_list: Vec<&&CardClosedTransactionRow> = list.iter().collect();
            sorted_list.sort_by_key(|r| std::cmp::Reverse(r.amount.abs()));
            let take_n = if args.all {
                sorted_list.len()
            } else {
                TX_LIMIT
            };
            for tx in sorted_list.iter().take(take_n) {
                println!(
                    "  • {} · {} ({})",
                    human_format::short_description(&tx.description),
                    human_format::brl_signed(tx.amount),
                    human_format::short_date(tx.transaction_date),
                );
            }
            if !args.all && sorted_list.len() > TX_LIMIT {
                println!(
                    "  _… mais {} {}_",
                    sorted_list.len() - TX_LIMIT,
                    if sorted_list.len() - TX_LIMIT == 1 {
                        "lançamento"
                    } else {
                        "lançamentos"
                    }
                );
            }
        }
        println!();
    }

    // Surface Pluggy-default classified transactions so the user can build
    // rules for them. Heuristic: any category_id that doesn't contain `:`
    // is a bare word that came from Pluggy's default classifier (the user's
    // own rules consistently produce `family:subcategory` ids). They're
    // already remapped onto PT families for display by `category_family`,
    // but their volume is worth highlighting so the user can close the loop.
    let pluggy_default: Vec<&CardClosedTransactionRow> = rows
        .iter()
        .filter(|r| {
            r.category_id
                .as_deref()
                .is_some_and(|c| !c.is_empty() && !c.contains(':'))
        })
        .collect();
    if !pluggy_default.is_empty() {
        let pluggy_net: Decimal = pluggy_default.iter().map(|r| r.amount).sum();
        println!(
            "💡 {} lanç auto-classificados pela Pluggy ({}) — criar regra pra consolidar",
            pluggy_default.len(),
            human_format::brl_signed(pluggy_net)
        );
        println!();
    }

    Ok(())
}

// Small helpers used only by report_cards.

trait NaiveDateSat {
    fn saturating_sub_signed_unsafe(self) -> NaiveDate;
}
impl NaiveDateSat for NaiveDate {
    fn saturating_sub_signed_unsafe(self) -> NaiveDate {
        // Subtract 7 days as a heuristic "start of payment-search window".
        self.checked_sub_signed(Duration::days(7)).unwrap_or(self)
    }
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
