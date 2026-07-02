//! `phai scenario …` — named what-if planning scenarios (ADR-0037).
//!
//! A scenario is a persisted set of typed deltas over the live forecast
//! baseline. Unlike `phai forecast scenario` (an ephemeral single-commitment
//! what-if), these scenarios are named, editable, comparable and can be
//! promoted into the real plan.

use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate, Utc};
use clap::{Args, Subcommand};
use phai_core::migrations::run_migrations;
use phai_core::scenario::{apply_scenario, diff_scenarios, parse_month, ScenarioProjection};
use phai_core::storage::{open_store, FinanceStore};
use phai_core::{AppConfig, AuditEvent, PlanChangeKind, PlanChangeRecord, PlanScenarioRecord};
use rust_decimal::Decimal;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::str::FromStr;

use crate::load_config;

const DEFAULT_HORIZON_MONTHS: u32 = 6;
const MAX_HORIZON_MONTHS: u32 = 36;

#[derive(Subcommand)]
pub(crate) enum ScenarioCommand {
    /// Create a new named scenario.
    Create(ScenarioCreateArgs),
    /// List scenarios (active by default).
    List(ScenarioListArgs),
    /// Show a scenario: its changes, orphans and monthly projection.
    Show(ScenarioShowArgs),
    /// Compare a scenario against the baseline or another scenario.
    Diff(ScenarioDiffArgs),
    /// Add a one-shot entry to a month (e.g. a planned trip).
    Add(ScenarioAddArgs),
    /// Override the amount of an existing forecast in the scenario.
    Adjust(ScenarioAdjustArgs),
    /// Skip an existing forecast in the scenario (one occurrence).
    Skip(ScenarioSkipArgs),
    /// End a recurring template from a month onwards (e.g. cancel a
    /// subscription in August).
    #[command(name = "end-template")]
    EndTemplate(ScenarioEndTemplateArgs),
    /// Add a hypothetical installment purchase (N monthly parcels).
    Installment(ScenarioInstallmentArgs),
    /// Remove a single change from a scenario.
    #[command(name = "delete-change")]
    DeleteChange(ScenarioDeleteChangeArgs),
    /// Archive a scenario (kept for reference, out of the active list).
    Archive(ScenarioIdArgs),
    /// Delete a scenario and all of its changes.
    Delete(ScenarioIdArgs),
    /// Mark orphaned changes (target realized/removed) so they stop
    /// cluttering the list. Never runs automatically.
    Prune(ScenarioIdArgs),
    /// Apply the scenario's changes to the real plan (forecasts and
    /// templates). Orphaned changes are skipped with a warning.
    Promote(ScenarioPromoteArgs),
}

#[derive(Args)]
pub(crate) struct ScenarioPromoteArgs {
    /// Scenario id (`scn-…`).
    scenario_id: String,
    /// Print the writes that would happen without applying them.
    #[arg(long)]
    dry_run: bool,
    /// Emit JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioCreateArgs {
    /// Scenario name (e.g. "com carro novo").
    #[arg(long)]
    name: String,
    /// Optional longer description.
    #[arg(long)]
    description: Option<String>,
    /// Emit the created scenario as JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioListArgs {
    /// Include archived and promoted scenarios.
    #[arg(long)]
    all: bool,
    /// Emit JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioShowArgs {
    /// Scenario id (`scn-…`).
    scenario_id: String,
    /// Projection horizon in months from today.
    #[arg(long, default_value_t = DEFAULT_HORIZON_MONTHS)]
    months: u32,
    /// Emit JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioDiffArgs {
    /// Scenario id (`scn-…`).
    scenario_id: String,
    /// Compare against this scenario instead of the baseline.
    #[arg(long)]
    against: Option<String>,
    /// Projection horizon in months from today.
    #[arg(long, default_value_t = DEFAULT_HORIZON_MONTHS)]
    months: u32,
    /// Emit JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioAddArgs {
    /// Scenario id (`scn-…`).
    scenario_id: String,
    /// Target month, `YYYY-MM`.
    #[arg(long)]
    month: String,
    /// Signed amount (negative = expense), e.g. `-2000.00`.
    #[arg(long)]
    amount: String,
    /// What this entry is (e.g. "viagem").
    #[arg(long)]
    description: String,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    account: Option<String>,
    /// Emit JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioAdjustArgs {
    /// Scenario id (`scn-…`).
    scenario_id: String,
    /// Target forecast id.
    #[arg(long)]
    forecast: String,
    /// New signed amount (absolute value, not a delta), e.g. `-800.00`.
    #[arg(long)]
    amount: String,
    /// Emit JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioSkipArgs {
    /// Scenario id (`scn-…`).
    scenario_id: String,
    /// Target forecast id.
    #[arg(long)]
    forecast: String,
    /// Emit JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioEndTemplateArgs {
    /// Scenario id (`scn-…`).
    scenario_id: String,
    /// Target forecast template id.
    #[arg(long)]
    template: String,
    /// First month without the template, `YYYY-MM` (inclusive).
    #[arg(long)]
    from: String,
    /// Emit JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioInstallmentArgs {
    /// Scenario id (`scn-…`).
    scenario_id: String,
    /// Month of the first parcel, `YYYY-MM`.
    #[arg(long)]
    start: String,
    /// Signed parcel amount (negative = expense), e.g. `-300.00`.
    #[arg(long)]
    amount: String,
    /// Number of parcels.
    #[arg(long)]
    months: u32,
    /// What this purchase is (e.g. "sofá 10x").
    #[arg(long)]
    description: String,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    account: Option<String>,
    /// Emit JSON.
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
pub(crate) struct ScenarioDeleteChangeArgs {
    /// Change id (`chg-…`).
    change_id: String,
    /// Scenario id the change belongs to (for the audit trail).
    #[arg(long)]
    scenario: String,
}

#[derive(Args)]
pub(crate) struct ScenarioIdArgs {
    /// Scenario id (`scn-…`).
    scenario_id: String,
}

pub(crate) async fn run(command: ScenarioCommand) -> Result<()> {
    match command {
        ScenarioCommand::Create(args) => run_create(args).await,
        ScenarioCommand::List(args) => run_list(args).await,
        ScenarioCommand::Show(args) => run_show(args).await,
        ScenarioCommand::Diff(args) => run_diff(args).await,
        ScenarioCommand::Add(args) => run_add(args).await,
        ScenarioCommand::Adjust(args) => run_adjust(args).await,
        ScenarioCommand::Skip(args) => run_skip(args).await,
        ScenarioCommand::EndTemplate(args) => run_end_template(args).await,
        ScenarioCommand::Installment(args) => run_installment(args).await,
        ScenarioCommand::DeleteChange(args) => run_delete_change(args).await,
        ScenarioCommand::Archive(args) => run_set_status(args, "arquivado").await,
        ScenarioCommand::Delete(args) => run_delete(args).await,
        ScenarioCommand::Prune(args) => run_prune(args).await,
        ScenarioCommand::Promote(args) => run_promote(args).await,
    }
}

async fn open() -> Result<(AppConfig, Box<dyn FinanceStore>)> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;
    Ok((config, store))
}

fn short_hash(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    )
}

fn audit_event_for_scenario(
    row: &PlanScenarioRecord,
    action: &str,
    actor_id: &str,
) -> Result<AuditEvent> {
    Ok(AuditEvent::from_entity(
        "plan_scenario",
        &row.scenario_id,
        action,
        actor_id,
        &row.idempotency_key,
        serde_json::to_value(row)?,
    ))
}

fn audit_event_for_plan_change(
    row: &PlanChangeRecord,
    action: &str,
    actor_id: &str,
) -> Result<AuditEvent> {
    Ok(AuditEvent::from_entity(
        "plan_change",
        &row.change_id,
        action,
        actor_id,
        &row.idempotency_key,
        serde_json::to_value(row)?,
    ))
}

fn parse_amount(raw: &str) -> Result<Decimal> {
    Decimal::from_str(raw).with_context(|| format!("--amount inválido: {raw} (esperado decimal)"))
}

fn validate_month(raw: &str) -> Result<()> {
    parse_month(raw)
        .map(|_| ())
        .with_context(|| format!("mês inválido: {raw} (esperado YYYY-MM)"))
}

async fn require_active_scenario(
    store: &dyn FinanceStore,
    scenario_id: &str,
) -> Result<PlanScenarioRecord> {
    let scenario = store
        .get_plan_scenario(scenario_id)
        .await?
        .with_context(|| format!("Cenário não encontrado: {scenario_id}"))?;
    if scenario.status != "ativo" {
        bail!(
            "Cenário {scenario_id} está '{status}' — apenas cenários ativos aceitam mudanças.",
            status = scenario.status
        );
    }
    Ok(scenario)
}

/// Empty template used when a change needs no extra fields filled in.
fn new_change(
    scenario_id: &str,
    kind: PlanChangeKind,
    actor_id: &str,
    discriminator: &str,
) -> PlanChangeRecord {
    let now = Utc::now();
    let change_id = format!(
        "chg-{}",
        short_hash(&format!(
            "{scenario_id}|{}|{discriminator}|{}",
            kind.as_str(),
            now.to_rfc3339()
        ))
    );
    PlanChangeRecord {
        change_id: change_id.clone(),
        scenario_id: scenario_id.to_string(),
        kind: kind.as_str().to_string(),
        target_forecast_id: None,
        target_template_id: None,
        month: None,
        effective_from: None,
        amount: None,
        months_count: None,
        description: None,
        category_id: None,
        account_id: None,
        status: "ativo".to_string(),
        payload_json: json!({}),
        actor_id: actor_id.to_string(),
        idempotency_key: change_id,
        created_at: now,
        updated_at: now,
    }
}

async fn persist_change(
    store: &dyn FinanceStore,
    config: &AppConfig,
    change: PlanChangeRecord,
    raw: bool,
    human_summary: String,
) -> Result<()> {
    store
        .upsert_plan_changes(std::slice::from_ref(&change))
        .await?;
    store
        .insert_audit_events(&[audit_event_for_plan_change(
            &change,
            "insert",
            &config.actor_id,
        )?])
        .await?;
    if raw {
        println!("{}", serde_json::to_string_pretty(&json!(change))?);
    } else {
        println!("{human_summary}");
        println!("  change_id: {}", change.change_id);
    }
    Ok(())
}

async fn run_create(args: ScenarioCreateArgs) -> Result<()> {
    let (config, store) = open().await?;
    let now = Utc::now();
    let scenario_id = format!(
        "scn-{}",
        short_hash(&format!("{}|{}", args.name, now.to_rfc3339()))
    );
    let scenario = PlanScenarioRecord {
        scenario_id: scenario_id.clone(),
        name: args.name,
        description: args.description,
        status: "ativo".to_string(),
        promoted_at: None,
        metadata_json: json!({}),
        actor_id: config.actor_id.clone(),
        idempotency_key: scenario_id.clone(),
        created_at: now,
        updated_at: now,
    };
    store
        .upsert_plan_scenarios(std::slice::from_ref(&scenario))
        .await?;
    store
        .insert_audit_events(&[audit_event_for_scenario(
            &scenario,
            "insert",
            &config.actor_id,
        )?])
        .await?;
    if args.raw {
        println!("{}", serde_json::to_string_pretty(&json!(scenario))?);
    } else {
        println!("🧪 Cenário criado: {}", scenario.name);
        println!("  scenario_id: {scenario_id}");
        println!("  Adicione mudanças com `phai scenario add|adjust|skip|end-template|installment {scenario_id} …`");
    }
    Ok(())
}

async fn run_list(args: ScenarioListArgs) -> Result<()> {
    let (_, store) = open().await?;
    let scenarios = if args.all {
        store.list_plan_scenarios(None).await?
    } else {
        store.list_plan_scenarios(Some("ativo")).await?
    };
    if args.raw {
        println!("{}", serde_json::to_string_pretty(&json!(scenarios))?);
        return Ok(());
    }
    if scenarios.is_empty() {
        println!("Nenhum cenário. Crie um com `phai scenario create --name \"…\"`.");
        return Ok(());
    }
    println!("🧪 Cenários");
    for scenario in &scenarios {
        let changes = store
            .list_plan_changes(&scenario.scenario_id, None)
            .await?
            .len();
        println!(
            "  {}  {}  [{}]  {} mudança(s)",
            scenario.scenario_id, scenario.name, scenario.status, changes
        );
    }
    Ok(())
}

/// Everything `show`/`diff` need in one load: the scenario, its changes and
/// the projection over the current baseline.
async fn load_projection(
    store: &dyn FinanceStore,
    scenario_id: &str,
    months: u32,
) -> Result<(
    PlanScenarioRecord,
    Vec<PlanChangeRecord>,
    ScenarioProjection,
    (NaiveDate, NaiveDate),
)> {
    let scenario = store
        .get_plan_scenario(scenario_id)
        .await?
        .with_context(|| format!("Cenário não encontrado: {scenario_id}"))?;
    let changes = store.list_plan_changes(scenario_id, None).await?;
    let months = months.clamp(1, MAX_HORIZON_MONTHS);
    let today = Utc::now().date_naive();
    let until = today
        .checked_add_months(chrono::Months::new(months))
        .context("falha ao calcular horizonte")?;
    let horizon = (today, until);
    let baseline = store
        .list_forecasts(Some("ativo"), Some(today), Some(until))
        .await?;
    let templates = store.list_forecast_templates(None, None).await?;
    let projection = apply_scenario(&baseline, &templates, &changes, horizon);
    Ok((scenario, changes, projection, horizon))
}

fn describe_change(change: &PlanChangeRecord) -> String {
    let amount = change
        .amount
        .map(crate::human_format::brl_signed)
        .unwrap_or_default();
    match PlanChangeKind::parse(&change.kind) {
        Some(PlanChangeKind::AddOneShot) => format!(
            "+ {} em {} ({amount})",
            change.description.as_deref().unwrap_or("entrada pontual"),
            change.month.as_deref().unwrap_or("?"),
        ),
        Some(PlanChangeKind::AdjustAmount) => format!(
            "~ ajustar {} para {amount}",
            change.target_forecast_id.as_deref().unwrap_or("?"),
        ),
        Some(PlanChangeKind::SkipForecast) => format!(
            "- pular {}",
            change.target_forecast_id.as_deref().unwrap_or("?"),
        ),
        Some(PlanChangeKind::EndTemplate) => format!(
            "✂ encerrar {} a partir de {}",
            change.target_template_id.as_deref().unwrap_or("?"),
            change.effective_from.as_deref().unwrap_or("?"),
        ),
        Some(PlanChangeKind::HypotheticalInstallment) => format!(
            "≡ {} — {}x de {amount} desde {}",
            change.description.as_deref().unwrap_or("parcelamento"),
            change.months_count.unwrap_or(0),
            change.effective_from.as_deref().unwrap_or("?"),
        ),
        None => format!("? mudança desconhecida ({})", change.kind),
    }
}

async fn run_show(args: ScenarioShowArgs) -> Result<()> {
    let (_, store) = open().await?;
    let (scenario, changes, projection, horizon) =
        load_projection(store.as_ref(), &args.scenario_id, args.months).await?;
    let today = Utc::now().date_naive();
    let anchor = store
        .checking_balance_at(today)
        .await
        .ok()
        .flatten()
        .map(|b| b.balance);

    if args.raw {
        let payload = json!({
            "scenario": scenario,
            "changes": changes,
            "orphaned_change_ids": projection.orphaned_change_ids,
            "monthly_delta": projection.monthly_delta.iter()
                .map(|(m, v)| json!({"month": m, "delta": v.to_string()}))
                .collect::<Vec<_>>(),
            "horizon_from": horizon.0.format("%Y-%m-%d").to_string(),
            "horizon_until": horizon.1.format("%Y-%m-%d").to_string(),
            "current_balance": anchor.map(|b| b.to_string()),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("🧪 {} [{}]", scenario.name, scenario.status);
    println!("  scenario_id: {}", scenario.scenario_id);
    if changes.is_empty() {
        println!("  (sem mudanças ainda)");
        return Ok(());
    }
    println!();
    println!("  Mudanças:");
    for change in &changes {
        let orphan = if projection.orphaned_change_ids.contains(&change.change_id) {
            " ⚠️ órfã"
        } else {
            ""
        };
        println!(
            "    {}  {}{orphan}",
            change.change_id,
            describe_change(change)
        );
    }
    if !projection.monthly_delta.is_empty() {
        println!();
        println!("  Δ mensal vs baseline:");
        for (month, delta) in &projection.monthly_delta {
            println!("    {month}  {}", crate::human_format::brl_signed(*delta));
        }
        let total: Decimal = projection.monthly_delta.values().copied().sum();
        println!("    total  {}", crate::human_format::brl_signed(total));
    }
    if let Some(balance) = anchor {
        let total: Decimal = projection.monthly_delta.values().copied().sum();
        println!();
        println!(
            "  Saldo hoje {} → efeito do cenário no horizonte: {}",
            crate::human_format::brl(balance),
            crate::human_format::brl_signed(total)
        );
    }
    Ok(())
}

async fn run_diff(args: ScenarioDiffArgs) -> Result<()> {
    let (_, store) = open().await?;
    let (scenario_a, _, projection_a, _) =
        load_projection(store.as_ref(), &args.scenario_id, args.months).await?;
    let (label_b, diff): (String, BTreeMap<String, Decimal>) = match &args.against {
        Some(other) => {
            let (scenario_b, _, projection_b, _) =
                load_projection(store.as_ref(), other, args.months).await?;
            (
                scenario_b.name,
                diff_scenarios(&projection_a, &projection_b),
            )
        }
        None => ("baseline".to_string(), projection_a.monthly_delta.clone()),
    };

    if args.raw {
        let payload = json!({
            "scenario": scenario_a.scenario_id,
            "against": args.against,
            "monthly_diff": diff.iter()
                .map(|(m, v)| json!({"month": m, "delta": v.to_string()}))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("🧪 {} vs {}", scenario_a.name, label_b);
    if diff.is_empty() {
        println!("  Sem diferença no horizonte.");
        return Ok(());
    }
    for (month, delta) in &diff {
        println!("  {month}  {}", crate::human_format::brl_signed(*delta));
    }
    let total: Decimal = diff.values().copied().sum();
    println!("  total  {}", crate::human_format::brl_signed(total));
    Ok(())
}

async fn run_add(args: ScenarioAddArgs) -> Result<()> {
    let (config, store) = open().await?;
    require_active_scenario(store.as_ref(), &args.scenario_id).await?;
    validate_month(&args.month)?;
    let amount = parse_amount(&args.amount)?;
    let mut change = new_change(
        &args.scenario_id,
        PlanChangeKind::AddOneShot,
        &config.actor_id,
        &format!("{}|{}", args.month, args.description),
    );
    change.month = Some(args.month.clone());
    change.amount = Some(amount);
    change.description = Some(args.description.clone());
    change.category_id = args.category.clone();
    change.account_id = args.account.clone();
    let summary = format!(
        "🧪 {} em {} adicionado ao cenário.",
        args.description, args.month
    );
    persist_change(store.as_ref(), &config, change, args.raw, summary).await
}

async fn run_adjust(args: ScenarioAdjustArgs) -> Result<()> {
    let (config, store) = open().await?;
    require_active_scenario(store.as_ref(), &args.scenario_id).await?;
    let amount = parse_amount(&args.amount)?;
    let forecast = store
        .get_forecast(&args.forecast)
        .await?
        .with_context(|| format!("Forecast não encontrado: {}", args.forecast))?;
    let mut change = new_change(
        &args.scenario_id,
        PlanChangeKind::AdjustAmount,
        &config.actor_id,
        &args.forecast,
    );
    change.target_forecast_id = Some(args.forecast.clone());
    change.amount = Some(amount);
    change.description = Some(forecast.description.clone());
    let summary = format!(
        "🧪 {} ajustado para {} no cenário.",
        forecast.description,
        crate::human_format::brl_signed(amount)
    );
    persist_change(store.as_ref(), &config, change, args.raw, summary).await
}

async fn run_skip(args: ScenarioSkipArgs) -> Result<()> {
    let (config, store) = open().await?;
    require_active_scenario(store.as_ref(), &args.scenario_id).await?;
    let forecast = store
        .get_forecast(&args.forecast)
        .await?
        .with_context(|| format!("Forecast não encontrado: {}", args.forecast))?;
    let mut change = new_change(
        &args.scenario_id,
        PlanChangeKind::SkipForecast,
        &config.actor_id,
        &args.forecast,
    );
    change.target_forecast_id = Some(args.forecast.clone());
    change.description = Some(forecast.description.clone());
    let summary = format!("🧪 {} pulado no cenário.", forecast.description);
    persist_change(store.as_ref(), &config, change, args.raw, summary).await
}

async fn run_end_template(args: ScenarioEndTemplateArgs) -> Result<()> {
    let (config, store) = open().await?;
    require_active_scenario(store.as_ref(), &args.scenario_id).await?;
    validate_month(&args.from)?;
    let template = store
        .get_forecast_template(&args.template)
        .await?
        .with_context(|| format!("Template não encontrado: {}", args.template))?;
    let mut change = new_change(
        &args.scenario_id,
        PlanChangeKind::EndTemplate,
        &config.actor_id,
        &format!("{}|{}", args.template, args.from),
    );
    change.target_template_id = Some(args.template.clone());
    change.effective_from = Some(args.from.clone());
    change.description = Some(template.description.clone());
    let summary = format!(
        "🧪 {} encerrado a partir de {} no cenário.",
        template.description, args.from
    );
    persist_change(store.as_ref(), &config, change, args.raw, summary).await
}

async fn run_installment(args: ScenarioInstallmentArgs) -> Result<()> {
    let (config, store) = open().await?;
    require_active_scenario(store.as_ref(), &args.scenario_id).await?;
    validate_month(&args.start)?;
    if args.months == 0 {
        bail!("--months deve ser ≥ 1");
    }
    let amount = parse_amount(&args.amount)?;
    let mut change = new_change(
        &args.scenario_id,
        PlanChangeKind::HypotheticalInstallment,
        &config.actor_id,
        &format!("{}|{}|{}", args.start, args.months, args.description),
    );
    change.effective_from = Some(args.start.clone());
    change.amount = Some(amount);
    change.months_count = Some(args.months as i32);
    change.description = Some(args.description.clone());
    change.category_id = args.category.clone();
    change.account_id = args.account.clone();
    let summary = format!(
        "🧪 {} — {}x de {} desde {} adicionado ao cenário.",
        args.description,
        args.months,
        crate::human_format::brl_signed(amount),
        args.start
    );
    persist_change(store.as_ref(), &config, change, args.raw, summary).await
}

async fn run_delete_change(args: ScenarioDeleteChangeArgs) -> Result<()> {
    let (config, store) = open().await?;
    let changes = store.list_plan_changes(&args.scenario, None).await?;
    let change = changes
        .into_iter()
        .find(|c| c.change_id == args.change_id)
        .with_context(|| {
            format!(
                "Mudança {} não encontrada no cenário {}",
                args.change_id, args.scenario
            )
        })?;
    store.delete_plan_change(&args.change_id).await?;
    store
        .insert_audit_events(&[audit_event_for_plan_change(
            &change,
            "delete",
            &config.actor_id,
        )?])
        .await?;
    println!("🧪 Mudança {} removida.", args.change_id);
    Ok(())
}

async fn run_set_status(args: ScenarioIdArgs, status: &str) -> Result<()> {
    let (config, store) = open().await?;
    store
        .set_plan_scenario_status(&args.scenario_id, status, &config.actor_id)
        .await?;
    let scenario = store
        .get_plan_scenario(&args.scenario_id)
        .await?
        .with_context(|| format!("Cenário não encontrado: {}", args.scenario_id))?;
    store
        .insert_audit_events(&[audit_event_for_scenario(
            &scenario,
            "update",
            &config.actor_id,
        )?])
        .await?;
    println!("🧪 Cenário {} → {status}.", args.scenario_id);
    Ok(())
}

async fn run_delete(args: ScenarioIdArgs) -> Result<()> {
    let (config, store) = open().await?;
    let scenario = store
        .get_plan_scenario(&args.scenario_id)
        .await?
        .with_context(|| format!("Cenário não encontrado: {}", args.scenario_id))?;
    store.delete_plan_scenario(&args.scenario_id).await?;
    store
        .insert_audit_events(&[audit_event_for_scenario(
            &scenario,
            "delete",
            &config.actor_id,
        )?])
        .await?;
    println!(
        "🧪 Cenário {} removido (com suas mudanças).",
        args.scenario_id
    );
    Ok(())
}

async fn run_prune(args: ScenarioIdArgs) -> Result<()> {
    let (config, store) = open().await?;
    let (_, changes, projection, _) =
        load_projection(store.as_ref(), &args.scenario_id, DEFAULT_HORIZON_MONTHS).await?;
    let mut pruned = Vec::new();
    for change in changes {
        if projection.orphaned_change_ids.contains(&change.change_id) && change.status == "ativo" {
            let mut orphan = change;
            orphan.status = "orfao".to_string();
            orphan.updated_at = Utc::now();
            pruned.push(orphan);
        }
    }
    if pruned.is_empty() {
        println!("🧪 Nenhuma mudança órfã.");
        return Ok(());
    }
    store.upsert_plan_changes(&pruned).await?;
    let events = pruned
        .iter()
        .map(|c| audit_event_for_plan_change(c, "update", &config.actor_id))
        .collect::<Result<Vec<_>>>()?;
    store.insert_audit_events(&events).await?;
    println!("🧪 {} mudança(s) marcada(s) como órfã(s).", pruned.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// Promotion — apply scenario deltas to the real plan (ADR-0037)
// ---------------------------------------------------------------------------

/// One concrete write the promotion will perform (or performed).
#[derive(Debug, serde::Serialize)]
pub(crate) struct PromotionAction {
    pub change_id: String,
    pub kind: String,
    pub description: String,
}

#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct PromotionSummary {
    pub applied: Vec<PromotionAction>,
    pub skipped_orphans: Vec<String>,
    pub forecasts_written: usize,
    pub templates_written: usize,
}

/// Deterministic promotion order: template-level cuts first, then per-row
/// edits, then additions. Every write is idempotent via natural keys
/// (`scenario-{scenario_id}-{change_id}` — ADR-0022), so a retry after a
/// partial BigQuery failure re-executes as a no-op.
fn promotion_rank(kind: &str) -> u8 {
    match PlanChangeKind::parse(kind) {
        Some(PlanChangeKind::EndTemplate) => 0,
        Some(PlanChangeKind::SkipForecast) => 1,
        Some(PlanChangeKind::AdjustAmount) => 2,
        Some(PlanChangeKind::AddOneShot) => 3,
        Some(PlanChangeKind::HypotheticalInstallment) => 4,
        None => u8::MAX,
    }
}

pub(crate) async fn promote_scenario(
    store: &dyn FinanceStore,
    config: &AppConfig,
    scenario_id: &str,
    dry_run: bool,
) -> Result<PromotionSummary> {
    let scenario = store
        .get_plan_scenario(scenario_id)
        .await?
        .with_context(|| format!("Cenário não encontrado: {scenario_id}"))?;
    if scenario.status == "arquivado" {
        bail!(
            "Cenário {scenario_id} está arquivado — desarquive criando um novo ou promova outro."
        );
    }
    let mut changes = store.list_plan_changes(scenario_id, None).await?;
    changes.sort_by_key(|c| (promotion_rank(&c.kind), c.created_at));

    let mut summary = PromotionSummary::default();
    let mut applied_changes: Vec<PlanChangeRecord> = Vec::new();

    for change in changes {
        if change.status == "aplicado" {
            continue;
        }
        let outcome = match PlanChangeKind::parse(&change.kind) {
            Some(PlanChangeKind::EndTemplate) => {
                promote_end_template(store, config, &change, dry_run).await?
            }
            Some(PlanChangeKind::SkipForecast) => {
                promote_skip(store, config, &change, dry_run).await?
            }
            Some(PlanChangeKind::AdjustAmount) => {
                promote_adjust(store, config, &change, dry_run).await?
            }
            Some(PlanChangeKind::AddOneShot) => {
                promote_add_one_shot(store, config, &change, dry_run).await?
            }
            Some(PlanChangeKind::HypotheticalInstallment) => {
                promote_installment(store, config, &change, dry_run).await?
            }
            None => None,
        };
        match outcome {
            Some(action) => {
                summary.forecasts_written += action.forecasts_written;
                summary.templates_written += action.templates_written;
                summary.applied.push(PromotionAction {
                    change_id: change.change_id.clone(),
                    kind: change.kind.clone(),
                    description: action.description,
                });
                applied_changes.push(change);
            }
            None => summary.skipped_orphans.push(change.change_id.clone()),
        }
    }

    if !dry_run {
        for change in &mut applied_changes {
            change.status = "aplicado".to_string();
            change.updated_at = Utc::now();
        }
        if !applied_changes.is_empty() {
            store.upsert_plan_changes(&applied_changes).await?;
            let events = applied_changes
                .iter()
                .map(|c| audit_event_for_plan_change(c, "scenario_promote", &config.actor_id))
                .collect::<Result<Vec<_>>>()?;
            store.insert_audit_events(&events).await?;
        }
        store
            .set_plan_scenario_status(scenario_id, "promovido", &config.actor_id)
            .await?;
        let promoted = store
            .get_plan_scenario(scenario_id)
            .await?
            .with_context(|| format!("Cenário não encontrado: {scenario_id}"))?;
        store
            .insert_audit_events(&[audit_event_for_scenario(
                &promoted,
                "scenario_promote",
                &config.actor_id,
            )?])
            .await?;
    }

    Ok(summary)
}

/// Result of promoting a single change.
struct ChangeOutcome {
    description: String,
    forecasts_written: usize,
    templates_written: usize,
}

async fn promote_end_template(
    store: &dyn FinanceStore,
    config: &AppConfig,
    change: &PlanChangeRecord,
    dry_run: bool,
) -> Result<Option<ChangeOutcome>> {
    let (Some(target), Some(effective_from)) = (
        change.target_template_id.as_deref(),
        change.effective_from.as_deref(),
    ) else {
        return Ok(None);
    };
    let Some(cutoff) = parse_month(effective_from) else {
        return Ok(None);
    };
    let Some(mut template) = store.get_forecast_template(target).await? else {
        return Ok(None);
    };
    if template.status == "descartado" {
        return Ok(None);
    }
    // Last day of the month before `effective_from`.
    let end_date = cutoff.pred_opt().context("data inválida no cutoff")?;
    let doomed: Vec<_> = store
        .list_forecasts(Some("ativo"), Some(cutoff), None)
        .await?
        .into_iter()
        .filter(|f| f.template_id.as_deref() == Some(target))
        .collect();
    let description = format!(
        "encerrar {} a partir de {effective_from} ({} forecast(s) descartado(s))",
        template.description,
        doomed.len()
    );
    if dry_run {
        return Ok(Some(ChangeOutcome {
            description,
            forecasts_written: doomed.len(),
            templates_written: 1,
        }));
    }
    template.end_date = Some(end_date);
    template.updated_at = Utc::now();
    store
        .upsert_forecast_templates(std::slice::from_ref(&template))
        .await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "forecast_template",
            &template.template_id,
            "scenario_promote",
            &config.actor_id,
            &format!("scenario-{}-{}", change.scenario_id, change.change_id),
            serde_json::to_value(&template)?,
        )])
        .await?;
    let mut discarded = doomed;
    for forecast in &mut discarded {
        forecast.status = "descartado".to_string();
        forecast.updated_at = Utc::now();
    }
    let forecasts_written = discarded.len();
    if !discarded.is_empty() {
        store.upsert_forecasts(&discarded).await?;
        let events = discarded
            .iter()
            .map(|f| {
                AuditEvent::from_entity(
                    "forecast",
                    &f.forecast_id,
                    "scenario_promote",
                    &config.actor_id,
                    &format!("scenario-{}-{}", change.scenario_id, change.change_id),
                    serde_json::to_value(f).unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>();
        store.insert_audit_events(&events).await?;
    }
    Ok(Some(ChangeOutcome {
        description,
        forecasts_written,
        templates_written: 1,
    }))
}

async fn promote_forecast_edit(
    store: &dyn FinanceStore,
    config: &AppConfig,
    change: &PlanChangeRecord,
    dry_run: bool,
    edit: impl FnOnce(&mut phai_core::ForecastRecord),
    describe: impl FnOnce(&phai_core::ForecastRecord) -> String,
) -> Result<Option<ChangeOutcome>> {
    let Some(target) = change.target_forecast_id.as_deref() else {
        return Ok(None);
    };
    let Some(mut forecast) = store.get_forecast(target).await? else {
        return Ok(None);
    };
    if forecast.status != "ativo" {
        return Ok(None);
    }
    let description = describe(&forecast);
    if dry_run {
        return Ok(Some(ChangeOutcome {
            description,
            forecasts_written: 1,
            templates_written: 0,
        }));
    }
    edit(&mut forecast);
    forecast.updated_at = Utc::now();
    store
        .upsert_forecasts(std::slice::from_ref(&forecast))
        .await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "forecast",
            &forecast.forecast_id,
            "scenario_promote",
            &config.actor_id,
            &format!("scenario-{}-{}", change.scenario_id, change.change_id),
            serde_json::to_value(&forecast)?,
        )])
        .await?;
    Ok(Some(ChangeOutcome {
        description,
        forecasts_written: 1,
        templates_written: 0,
    }))
}

async fn promote_skip(
    store: &dyn FinanceStore,
    config: &AppConfig,
    change: &PlanChangeRecord,
    dry_run: bool,
) -> Result<Option<ChangeOutcome>> {
    promote_forecast_edit(
        store,
        config,
        change,
        dry_run,
        |f| f.status = "descartado".to_string(),
        |f| format!("descartar {} ({})", f.forecast_id, f.description),
    )
    .await
}

async fn promote_adjust(
    store: &dyn FinanceStore,
    config: &AppConfig,
    change: &PlanChangeRecord,
    dry_run: bool,
) -> Result<Option<ChangeOutcome>> {
    let Some(new_amount) = change.amount else {
        return Ok(None);
    };
    promote_forecast_edit(
        store,
        config,
        change,
        dry_run,
        |f| f.amount = new_amount,
        |f| {
            format!(
                "ajustar {} ({}) para {}",
                f.forecast_id,
                f.description,
                crate::human_format::brl_signed(new_amount)
            )
        },
    )
    .await
}

async fn promote_add_one_shot(
    store: &dyn FinanceStore,
    config: &AppConfig,
    change: &PlanChangeRecord,
    dry_run: bool,
) -> Result<Option<ChangeOutcome>> {
    let (Some(month), Some(amount)) = (change.month.as_deref(), change.amount) else {
        return Ok(None);
    };
    let Some(month_start) = parse_month(month) else {
        return Ok(None);
    };
    let due_date = month_start.with_day(15).unwrap_or(month_start);
    let now = Utc::now();
    let idempotency_key = format!("scenario-{}-{}", change.scenario_id, change.change_id);
    let forecast = phai_core::ForecastRecord {
        forecast_id: idempotency_key.clone(),
        due_date: Some(due_date),
        description: change
            .description
            .clone()
            .unwrap_or_else(|| "planejado".to_string()),
        amount,
        category_id: change.category_id.clone(),
        account_id: change.account_id.clone(),
        status: "ativo".to_string(),
        recurrence: None,
        actor_id: config.actor_id.clone(),
        idempotency_key: idempotency_key.clone(),
        metadata_json: serde_json::json!({ "scenario_id": change.scenario_id }),
        created_at: now,
        updated_at: now,
        template_id: None,
        realized_transaction_id: None,
        realized_at: None,
    };
    let description = format!(
        "criar {} em {} ({})",
        forecast.description,
        month,
        crate::human_format::brl_signed(amount)
    );
    if dry_run {
        return Ok(Some(ChangeOutcome {
            description,
            forecasts_written: 1,
            templates_written: 0,
        }));
    }
    store
        .upsert_forecasts(std::slice::from_ref(&forecast))
        .await?;
    store
        .insert_audit_events(&[AuditEvent::from_entity(
            "forecast",
            &forecast.forecast_id,
            "scenario_promote",
            &config.actor_id,
            &idempotency_key,
            serde_json::to_value(&forecast)?,
        )])
        .await?;
    Ok(Some(ChangeOutcome {
        description,
        forecasts_written: 1,
        templates_written: 0,
    }))
}

async fn promote_installment(
    store: &dyn FinanceStore,
    config: &AppConfig,
    change: &PlanChangeRecord,
    dry_run: bool,
) -> Result<Option<ChangeOutcome>> {
    let (Some(effective_from), Some(amount), Some(count)) = (
        change.effective_from.as_deref(),
        change.amount,
        change.months_count,
    ) else {
        return Ok(None);
    };
    let Some(first_month) = parse_month(effective_from) else {
        return Ok(None);
    };
    if count <= 0 {
        return Ok(None);
    }
    let now = Utc::now();
    let template_id = format!("scn-installment-{}", change.change_id);
    let description_text = change
        .description
        .clone()
        .unwrap_or_else(|| "parcelamento planejado".to_string());
    let last_month = first_month
        .checked_add_months(chrono::Months::new(count as u32 - 1))
        .context("falha ao calcular última parcela")?;
    let template = phai_core::ForecastTemplateRecord {
        template_id: template_id.clone(),
        kind: "installment".to_string(),
        description: description_text.clone(),
        merchant_pattern: None,
        category_id: change.category_id.clone(),
        account_id: change.account_id.clone(),
        amount,
        amount_lower: None,
        amount_upper: None,
        cadence: "monthly".to_string(),
        next_due_day: Some(15),
        start_date: first_month,
        end_date: Some(last_month),
        remaining_count: Some(count),
        source: "manual".to_string(),
        confidence: None,
        status: "ativo".to_string(),
        metadata_json: serde_json::json!({ "scenario_id": change.scenario_id }),
        actor_id: config.actor_id.clone(),
        idempotency_key: format!("scenario-{}-{}", change.scenario_id, change.change_id),
        created_at: now,
        updated_at: now,
    };
    let mut forecasts = Vec::with_capacity(count as usize);
    for n in 0..count as u32 {
        let Some(month_start) = first_month.checked_add_months(chrono::Months::new(n)) else {
            break;
        };
        let due_date = month_start.with_day(15).unwrap_or(month_start);
        let forecast_id = format!("{template_id}-{:03}", n + 1);
        forecasts.push(phai_core::ForecastRecord {
            forecast_id: forecast_id.clone(),
            due_date: Some(due_date),
            description: format!("{} {}/{}", description_text, n + 1, count),
            amount,
            category_id: change.category_id.clone(),
            account_id: change.account_id.clone(),
            status: "ativo".to_string(),
            recurrence: None,
            actor_id: config.actor_id.clone(),
            idempotency_key: forecast_id,
            metadata_json: serde_json::json!({ "scenario_id": change.scenario_id }),
            created_at: now,
            updated_at: now,
            template_id: Some(template_id.clone()),
            realized_transaction_id: None,
            realized_at: None,
        });
    }
    let description = format!(
        "criar {} — {}x de {} desde {}",
        description_text,
        count,
        crate::human_format::brl_signed(amount),
        effective_from
    );
    if dry_run {
        return Ok(Some(ChangeOutcome {
            description,
            forecasts_written: forecasts.len(),
            templates_written: 1,
        }));
    }
    store
        .upsert_forecast_templates(std::slice::from_ref(&template))
        .await?;
    store.upsert_forecasts(&forecasts).await?;
    let mut events = vec![AuditEvent::from_entity(
        "forecast_template",
        &template.template_id,
        "scenario_promote",
        &config.actor_id,
        &template.idempotency_key,
        serde_json::to_value(&template)?,
    )];
    for forecast in &forecasts {
        events.push(AuditEvent::from_entity(
            "forecast",
            &forecast.forecast_id,
            "scenario_promote",
            &config.actor_id,
            &forecast.idempotency_key,
            serde_json::to_value(forecast)?,
        ));
    }
    store.insert_audit_events(&events).await?;
    Ok(Some(ChangeOutcome {
        description,
        forecasts_written: forecasts.len(),
        templates_written: 1,
    }))
}

async fn run_promote(args: ScenarioPromoteArgs) -> Result<()> {
    let (config, store) = open().await?;
    let summary =
        promote_scenario(store.as_ref(), &config, &args.scenario_id, args.dry_run).await?;

    if args.raw {
        println!("{}", serde_json::to_string_pretty(&json!(summary))?);
        return Ok(());
    }
    let header = if args.dry_run {
        "🧪 Promoção (dry-run) — nada foi gravado:"
    } else {
        "🧪 Cenário promovido ao plano real:"
    };
    println!("{header}");
    if summary.applied.is_empty() {
        println!("  Nenhuma mudança pendente.");
    }
    for action in &summary.applied {
        println!("  ✓ {}", action.description);
    }
    for orphan in &summary.skipped_orphans {
        println!("  ⚠️ {orphan} pulada (alvo realizado/removido)");
    }
    println!(
        "  {} forecast(s), {} template(s) afetado(s).",
        summary.forecasts_written, summary.templates_written
    );
    Ok(())
}
