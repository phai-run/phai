use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use finance_core::idempotency::{
    category_id, ensure_account_idempotency, ensure_forecast_idempotency, ensure_rule_idempotency,
    ensure_transaction_idempotency, manual_transaction_idempotency,
};
use finance_core::legacy::load_legacy_bundle;
use finance_core::migrations::run_migrations;
use finance_core::models::{
    decimal_from_str, AccountRecord, AuditEvent, CategoryRecord, ForecastRecord, RuleRecord,
    TransactionRecord,
};
use finance_core::pluggy::sync_pluggy;
use finance_core::storage::{open_store, FinanceStore};
use finance_core::{AppConfig, BackendKind, ConfigPaths};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

const UPSERT_BATCH_SIZE: usize = 50;
const AUDIT_BATCH_SIZE: usize = 25;

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
}

#[derive(Subcommand)]
enum ReportCommand {
    DailyPulse(DailyPulseArgs),
    MonthlySpend(MonthlySpendArgs),
    Cashflow(CashflowArgs),
    ForecastVsActual(ForecastVsActualArgs),
    CardSummary(CardSummaryArgs),
    Uncategorized(UncategorizedArgs),
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
struct UncategorizedArgs {
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncSummaryTransaction {
    transaction_id: String,
    transaction_date: String,
    description: String,
    amount: String,
    category_id: Option<String>,
    account_id: Option<String>,
    payment_status: String,
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncSummaryPending {
    transaction_id: String,
    transaction_date: String,
    description: String,
    amount: String,
    account_id: Option<String>,
    category_source: String,
    payment_status: String,
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncSummaryOutput {
    actor_id: String,
    backend: String,
    generated_at: String,
    new_transactions_count: usize,
    needs_context_count: usize,
    new_transactions: Vec<SyncSummaryTransaction>,
    needs_context: Vec<SyncSummaryPending>,
}

#[derive(Subcommand)]
enum TxCommand {
    UpsertManual(ManualTransactionArgs),
    Categorize(CategorizeTransactionArgs),
    SetContext(SetContextArgs),
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
            ReportCommand::Uncategorized(args) => report_uncategorized(args).await,
        },
        Commands::Tx { command } => match command {
            TxCommand::UpsertManual(args) => tx_upsert_manual(args).await,
            TxCommand::Categorize(args) => tx_categorize(args).await,
            TxCommand::SetContext(args) => tx_set_context(args).await,
        },
        Commands::Forecast { command } => match command {
            ForecastCommand::Upsert(args) => forecast_upsert(args).await,
        },
        Commands::Rule { command } => match command {
            RuleCommand::Upsert(args) => rule_upsert(args).await,
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
    println!("local_db: {}", config.local_db_path.unwrap().display());
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

async fn sync_pluggy_command(args: SyncPluggyArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let to = args
        .to
        .unwrap_or_else(|| Utc::now().date_naive().format("%Y-%m-%d").to_string());
    let (accounts, transactions) = sync_pluggy(
        &config.actor_id,
        &args.pluggy_config,
        Some(&args.accounts_csv),
        args.fixture.as_deref(),
        args.from.as_deref().or(config.pluggy_start_date.as_deref()),
        &to,
    )
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

    let new_transactions = transactions
        .iter()
        .filter(|row| !existing_ids.contains(&row.transaction_id))
        .map(|row| SyncSummaryTransaction {
            transaction_id: row.transaction_id.clone(),
            transaction_date: row.transaction_date.format("%Y-%m-%d").to_string(),
            description: row.description.clone(),
            amount: decimal_text(row.amount),
            category_id: row.category_id.clone(),
            account_id: row.account_id.clone(),
            payment_status: row.payment_status.clone(),
            source: row.source.clone(),
        })
        .collect::<Vec<_>>();
    let needs_context = store
        .uncategorized(100)
        .await?
        .into_iter()
        .map(|row| SyncSummaryPending {
            transaction_id: row.transaction_id,
            transaction_date: row.transaction_date.format("%Y-%m-%d").to_string(),
            description: row.description,
            amount: decimal_text(row.amount),
            account_id: row.account_id,
            category_source: row.category_source,
            payment_status: row.payment_status,
            source: row.source,
        })
        .collect::<Vec<_>>();

    if args.json_summary {
        let summary = SyncSummaryOutput {
            actor_id: config.actor_id.clone(),
            backend: format!("{:?}", config.effective_backend()).to_lowercase(),
            generated_at: Utc::now().to_rfc3339(),
            new_transactions_count: new_transactions.len(),
            needs_context_count: needs_context.len(),
            new_transactions,
            needs_context,
        };
        println!("{}", serde_json::to_string_pretty(&summary)?);
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

async fn report_daily_pulse(args: DailyPulseArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let since = Utc::now()
        .date_naive()
        .checked_sub_signed(Duration::days(args.days.saturating_sub(1)))
        .context("Falha ao calcular janela do daily pulse")?;
    let items = store.daily_pulse(since).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    let income = items
        .iter()
        .filter(|item| !item.amount.is_sign_negative())
        .fold(Decimal::ZERO, |acc, item| acc + item.amount);
    let expenses = items
        .iter()
        .filter(|item| item.amount.is_sign_negative())
        .fold(Decimal::ZERO, |acc, item| acc + item.amount);

    println!("Daily pulse desde {}", since.format("%Y-%m-%d"));
    println!("- linhas: {}", items.len());
    println!("- entradas: {}", brl(income));
    println!("- saídas: {}", brl(expenses));
    println!();

    for item in items {
        let category = item
            .category_id
            .unwrap_or_else(|| "sem-categoria".to_string());
        let account = item.account_id.unwrap_or_else(|| "sem-conta".to_string());
        println!(
            "{} | {} | {} | {} | {} | {}",
            item.transaction_date.format("%Y-%m-%d"),
            brl(item.amount),
            item.description,
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
    let rows = store.monthly_spend(args.month.as_deref()).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "Monthly spend{}",
        args.month
            .as_deref()
            .map(|value| format!(" {value}"))
            .unwrap_or_default()
    );
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        println!(
            "{} | {} | {} | {} | {} transações",
            row.month_ref,
            row.category_id,
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
    let rows = store.cashflow(args.months).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("Cashflow últimos {} meses", args.months);
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
    let rows = store.forecast_vs_actual(args.month.as_deref()).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "Forecast vs actual{}",
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
        let category = row
            .category_id
            .unwrap_or_else(|| "sem-categoria".to_string());
        println!(
            "{} | {} | {} | {} | previsto {} | realizado {} | variação {} | {}",
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
    let rows = store.card_summary(args.month.as_deref()).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "Card summary{}",
        args.month
            .as_deref()
            .map(|value| format!(" {value}"))
            .unwrap_or_default()
    );
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        println!(
            "{} | {} | total {} | em aberto {} | {} transações",
            row.month_ref,
            row.account_id,
            brl(-row.total_charges),
            brl(-row.open_amount),
            row.transaction_count
        );
    }
    Ok(())
}

async fn report_uncategorized(args: UncategorizedArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    let rows = store.uncategorized(args.limit).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("Uncategorized");
    println!("- linhas: {}", rows.len());
    println!();

    for row in rows {
        let account = row.account_id.unwrap_or_else(|| "sem-conta".to_string());
        println!(
            "{} | {} | {} | {} | {} | {}",
            row.transaction_date.format("%Y-%m-%d"),
            brl(row.amount),
            row.description,
            account,
            row.payment_status,
            row.category_source
        );
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
