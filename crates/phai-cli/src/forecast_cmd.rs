//! `finance forecast …` — orchestrator for the forecast automation pipeline
//! described in ADR-0016.
//!
//! Layer 1 (installments) is the only layer implemented here. It detects
//! installment chains in the transaction history and materialises one
//! `forecast` row per remaining parcela, anchored on a single
//! `forecast_template` that lives for the life of the chain.

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate, Utc};
use phai_core::migrations::run_migrations;
use phai_core::storage::{open_store, FinanceStore};
use phai_core::{
    group_into_chains, AccountRecord, AppConfig, AuditEvent, ForecastRecord,
    ForecastTemplateRecord, InstallmentChain, TransactionRecord,
};
use rust_decimal::Decimal;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    enrich_description_from_metadata, load_config, normalize_description, strip_installment_marker,
    ForecastAcceptArgs, ForecastDismissArgs, ForecastReconcileArgs, ForecastRefreshArgs,
    ForecastRefreshInstallmentsArgs, ForecastScenarioArgs, ForecastSuggestArgs,
};
use std::str::FromStr;

/// Summary returned by [`refresh_installments`] for CLI / agent display.
#[derive(Debug, Default, Clone)]
pub struct InstallmentsRefreshReport {
    pub chains_seen: usize,
    pub chains_active: usize,
    pub templates_upserted: usize,
    pub forecasts_upserted: usize,
    /// Duplicate templates collapsed to `descartado` before re-materialising
    /// (their orphan forecasts deactivated). Self-heal — see ADR-0022.
    pub templates_deduped: usize,
    /// Stale naming forks retired after re-derivation: active installment
    /// templates no longer derived from any chain but shadowed by a derived
    /// template for the same plan (same account/total/amount).
    pub templates_retired: usize,
}

pub(crate) async fn run_refresh_installments(args: ForecastRefreshInstallmentsArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let report = refresh_installments(store.as_ref(), &config, args.lookback_months).await?;

    if args.raw {
        let payload = json!({
            "chains_seen": report.chains_seen,
            "chains_active": report.chains_active,
            "templates_upserted": report.templates_upserted,
            "forecasts_upserted": report.forecasts_upserted,
            "templates_deduped": report.templates_deduped,
            "templates_retired": report.templates_retired,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("🔁 Forecast · parcelamentos");
        println!("  Cadeias detectadas:  {}", report.chains_seen);
        println!("  Cadeias ativas:      {}", report.chains_active);
        println!("  Templates atualizados: {}", report.templates_upserted);
        println!("  Forecasts gravados:    {}", report.forecasts_upserted);
        if report.templates_deduped > 0 {
            println!("  Duplicados colapsados: {}", report.templates_deduped);
        }
        if report.templates_retired > 0 {
            println!("  Renomeados aposentados: {}", report.templates_retired);
        }
    }
    Ok(())
}

/// Core algorithm — pure store interaction so it's reusable from the sync
/// pipeline (a future PR can wire it in there directly).
pub async fn refresh_installments(
    store: &dyn FinanceStore,
    config: &AppConfig,
    lookback_months: u32,
) -> Result<InstallmentsRefreshReport> {
    let today = Utc::now().date_naive();
    let from = shift_months_back(today, lookback_months as i32)?;
    let raw = store
        .transactions_in_date_range(None, from, today)
        .await
        .context("falha ao carregar transações para detectar parcelamentos")?;
    // Inject installment markers that live only in the Pluggy metadata so
    // older synced transactions still match the X/N regex.
    let txs: Vec<_> = raw
        .into_iter()
        .map(|mut tx| {
            tx.raw_description =
                enrich_description_from_metadata(&tx.raw_description, &tx.metadata_json);
            tx
        })
        .collect();

    let chains = group_into_chains(&txs);
    let mut report = InstallmentsRefreshReport {
        chains_seen: chains.len(),
        ..Default::default()
    };

    // Self-heal first: collapse any duplicate templates (same identity, drifted
    // id) and deactivate their orphan forecasts so this pass can't leave stale
    // duplicates active. The standalone `refresh-installments` command (and any
    // cron wired to it) previously skipped this — only `refresh_all` collapsed —
    // so forked installment forecasts accumulated and inflated future months
    // (ADR-0022). Idempotent: a no-op once the store is clean.
    report.templates_deduped = collapse_duplicate_templates(store, &config.actor_id).await?;

    // Map every known identity to its canonical template_id so re-derivation
    // reuses the existing row instead of forking a duplicate (ADR-0022).
    let existing_templates = store.list_forecast_templates(None, None).await?;
    let dedup_plan = plan_template_dedup(&existing_templates);

    let mut templates = Vec::new();
    let mut forecasts = Vec::new();

    for chain in &chains {
        if chain.remaining == 0 {
            continue;
        }
        report.chains_active += 1;

        let (template, instances) =
            build_template_and_instances(chain, &config.actor_id, &dedup_plan)?;
        templates.push(template);
        forecasts.extend(instances);
    }

    if !templates.is_empty() {
        report.templates_upserted = store.upsert_forecast_templates(&templates).await?;
        let events = templates
            .iter()
            .map(|t| audit_event_for_template(t, "upsert", &config.actor_id))
            .collect::<Result<Vec<_>>>()?;
        store.insert_audit_events(&events).await?;
        // A renamed plan keeps its old-name template behind: no chain derives
        // it anymore, so neither the natural-key collapse above nor this
        // upsert touches it — retire it or its parcelas project twice.
        report.templates_retired =
            retire_shadowed_templates(store, &templates, &config.actor_id).await?;
    }
    if !forecasts.is_empty() {
        report.forecasts_upserted = store.upsert_forecasts(&forecasts).await?;
        let events = forecasts
            .iter()
            .map(|f| audit_event_for_forecast(f, "upsert", &config.actor_id))
            .collect::<Result<Vec<_>>>()?;
        store.insert_audit_events(&events).await?;
    }

    Ok(report)
}

fn audit_event_for_template(
    row: &ForecastTemplateRecord,
    action: &str,
    actor_id: &str,
) -> Result<AuditEvent> {
    Ok(AuditEvent::from_entity(
        "forecast_template",
        &row.template_id,
        action,
        actor_id,
        &row.idempotency_key,
        serde_json::to_value(row)?,
    ))
}

fn audit_event_for_forecast(
    row: &ForecastRecord,
    action: &str,
    actor_id: &str,
) -> Result<AuditEvent> {
    Ok(AuditEvent::from_entity(
        "forecast",
        &row.forecast_id,
        action,
        actor_id,
        &row.idempotency_key,
        serde_json::to_value(row)?,
    ))
}

/// Build the `forecast_template` row plus one forecast per remaining parcela
/// from a detected chain.
fn build_template_and_instances(
    chain: &InstallmentChain,
    actor_id: &str,
    dedup_plan: &TemplateDedupPlan,
) -> Result<(ForecastTemplateRecord, Vec<ForecastRecord>)> {
    let now = Utc::now();
    // Reuse the existing id for this identity when one is known, so a drifted
    // hash updates the canonical row in place instead of forking a duplicate.
    let derived_id = format!("installment-{}", chain_idempotency_key(chain));
    let natural_key = installment_natural_key(chain);
    let template_id = dedup_plan.resolve(&natural_key, &derived_id).to_string();

    // Per-installment amount: prefer the most recent parcela's amount (it
    // tends to reflect any rate adjustments). The `amount` is stored signed
    // negative because installments are outflows (ADR-0016 convention).
    let per_installment = chain
        .installments
        .last()
        .map(|tx| tx.amount.abs())
        .unwrap_or_else(|| chain.total_amount.abs() / Decimal::from(chain.current.max(1)));
    let per_installment_signed = -per_installment;

    // Materialise dates: parcela N falls (N - current) months after the
    // most recent known parcela. Card cycle nuances are intentionally not
    // modelled here yet — the date is good enough for the chart bucket.
    let last_known_date = chain
        .installments
        .last()
        .map(|tx| tx.transaction_date)
        .unwrap_or(chain.first_date);

    let template = ForecastTemplateRecord {
        template_id: template_id.clone(),
        kind: "installment".to_string(),
        description: chain.base_description.clone(),
        merchant_pattern: Some(chain.base_description.clone()),
        category_id: None,
        account_id: Some(chain.account_id.clone()),
        amount: per_installment_signed,
        amount_lower: None,
        amount_upper: None,
        cadence: "monthly".to_string(),
        next_due_day: Some(last_known_date.day() as i32),
        start_date: chain.first_date,
        end_date: Some(chain.projected_end),
        remaining_count: Some(chain.remaining as i32),
        source: "detected".to_string(),
        confidence: Some(1.0),
        status: "ativo".to_string(),
        metadata_json: json!({
            "installments_total": chain.total,
            "installments_current": chain.current,
            "detector_version": 1,
        }),
        actor_id: actor_id.to_string(),
        idempotency_key: format!("forecast-template-{template_id}"),
        created_at: now,
        updated_at: now,
    };

    let mut instances = Vec::with_capacity(chain.remaining as usize);
    for offset in 1..=chain.remaining {
        let n = chain.current + offset;
        let due_date = match shift_months(last_known_date, offset as i32) {
            Some(d) => d,
            None => continue,
        };
        let forecast_id = format!("{template_id}-{n:03}");
        instances.push(ForecastRecord {
            forecast_id: forecast_id.clone(),
            due_date: Some(due_date),
            description: format!("{} ({n}/{})", chain.base_description, chain.total),
            amount: per_installment_signed,
            category_id: None,
            account_id: Some(chain.account_id.clone()),
            status: "ativo".to_string(),
            recurrence: Some("mensal".to_string()),
            actor_id: actor_id.to_string(),
            idempotency_key: format!("forecast-{forecast_id}"),
            metadata_json: json!({
                "source_template": template_id,
                "installment_number": n,
                "installments_total": chain.total,
            }),
            created_at: now,
            updated_at: now,
            template_id: Some(template_id.clone()),
            realized_transaction_id: None,
            realized_at: None,
        });
    }

    Ok((template, instances))
}

/// Stable hash of the chain's identity (account + base description + total)
/// so the template_id is deterministic across runs.
///
/// SHA-256 (truncated) — `DefaultHasher` is not stable across Rust
/// versions/platforms, so upgrading the toolchain would change all
/// template_ids and break the idempotency guarantee callers rely on
/// to avoid duplicate forecast templates.
fn chain_idempotency_key(chain: &InstallmentChain) -> String {
    let mut hasher = Sha256::new();
    hasher.update(chain.account_id.as_bytes());
    hasher.update(b"\x1f");
    hasher.update(chain.base_description.to_lowercase().as_bytes());
    hasher.update(b"\x1f");
    hasher.update(chain.total.to_string().as_bytes());
    let digest = hasher.finalize();
    format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    )
}

// ---------------------------------------------------------------------------
// Idempotency hardening — natural keys + self-healing dedup (ADR-0022)
// ---------------------------------------------------------------------------
//
// `template_id` is a content hash, so it is only stable while the hashing
// algorithm AND its inputs stay byte-identical. A release that tweaks the
// hashing, or a detector revision, re-derives a *different* id for the *same*
// real commitment; because the upsert dedups solely on `template_id`, the new
// id is INSERTed instead of updating the old row, forking the template (and,
// via `tpl-{template_id}-{yyyymm}` / `{template_id}-{n}`, every materialised
// forecast) into a duplicate that `refresh` then re-materialises forever.
//
// The natural key is a canonical, hash-independent identity. Two templates
// that share one describe the same commitment and must never coexist as live
// rows. We use it to (a) reuse an existing id when re-deriving, so drift
// updates in place, and (b) collapse any duplicates that already slipped in.

/// ASCII unit separator — keeps the joined key fields unambiguous.
const NATURAL_KEY_SEP: char = '\u{1f}';

/// Canonical identity of a forecast template, independent of the `template_id`
/// hash. Installments include their total so two distinct plans at the same
/// merchant stay separate; recurring kinds key on account + merchant + category.
fn template_natural_key(t: &ForecastTemplateRecord) -> String {
    let account = t.account_id.as_deref().unwrap_or("");
    let merchant = t.merchant_pattern.as_deref().unwrap_or("").to_lowercase();
    let category = t.category_id.as_deref().unwrap_or("");
    let total = if t.kind == "installment" {
        match t.metadata_json.get("installments_total") {
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(serde_json::Value::String(s)) => s.clone(),
            _ => String::new(),
        }
    } else {
        String::new()
    };
    join_natural_key([
        t.kind.as_str(),
        account,
        merchant.as_str(),
        category,
        &total,
    ])
}

/// Natural key of an installment chain. Must equal `template_natural_key` of
/// the template built from the same chain (asserted in tests).
fn installment_natural_key(chain: &InstallmentChain) -> String {
    join_natural_key([
        "installment",
        chain.account_id.as_str(),
        chain.base_description.to_lowercase().as_str(),
        "",
        chain.total.to_string().as_str(),
    ])
}

fn join_natural_key<const N: usize>(parts: [&str; N]) -> String {
    parts.join(&NATURAL_KEY_SEP.to_string())
}

/// Outcome of analysing existing templates for duplicate identities.
#[derive(Debug, Default)]
struct TemplateDedupPlan {
    /// natural key -> the `template_id` that should own that identity.
    canonical_id: std::collections::HashMap<String, String>,
    /// `template_id`s that lost the election and must be demoted.
    demote_ids: std::collections::HashSet<String>,
}

impl TemplateDedupPlan {
    /// The canonical id to use for a freshly-derived template, falling back to
    /// the derived id when this identity is new.
    fn resolve<'a>(&'a self, natural_key: &str, derived_id: &'a str) -> &'a str {
        self.canonical_id
            .get(natural_key)
            .map(String::as_str)
            .unwrap_or(derived_id)
    }
}

/// Group existing templates by natural key and elect one canonical row per
/// identity. The oldest row wins (ties broken by `template_id` for
/// determinism); already-`descartado` rows never win and are never re-demoted.
fn plan_template_dedup(existing: &[ForecastTemplateRecord]) -> TemplateDedupPlan {
    use std::collections::HashMap;
    let mut groups: HashMap<String, Vec<&ForecastTemplateRecord>> = HashMap::new();
    for t in existing {
        groups.entry(template_natural_key(t)).or_default().push(t);
    }
    let mut plan = TemplateDedupPlan::default();
    for (natural_key, mut rows) in groups {
        rows.sort_by(|a, b| {
            let a_dismissed = u8::from(a.status == "descartado");
            let b_dismissed = u8::from(b.status == "descartado");
            a_dismissed
                .cmp(&b_dismissed)
                .then(a.created_at.cmp(&b.created_at))
                .then_with(|| a.template_id.cmp(&b.template_id))
        });
        let canonical = rows[0];
        plan.canonical_id
            .insert(natural_key, canonical.template_id.clone());
        for dup in &rows[1..] {
            if dup.status != "descartado" {
                plan.demote_ids.insert(dup.template_id.clone());
            }
        }
    }
    plan
}

/// Forecast statuses that record a realised commitment — never auto-deactivated
/// by the dedup pass, since they tie a prediction to a real transaction.
fn is_realized_status(status: &str) -> bool {
    matches!(status, "realizado" | "realized" | "effected")
}

/// Collapse duplicate templates that share a natural key: demote the losers to
/// `descartado` and deactivate their non-realised forecasts to `inativo` so
/// they stop being projected and re-materialised. Idempotent — a second run
/// finds nothing to do. Returns the number of templates demoted.
async fn collapse_duplicate_templates(store: &dyn FinanceStore, actor_id: &str) -> Result<usize> {
    let existing = store.list_forecast_templates(None, None).await?;
    let plan = plan_template_dedup(&existing);
    if plan.demote_ids.is_empty() {
        return Ok(0);
    }
    let now = Utc::now();

    let mut demoted_templates = Vec::new();
    for t in &existing {
        if !plan.demote_ids.contains(&t.template_id) {
            continue;
        }
        let mut row = t.clone();
        row.status = "descartado".to_string();
        row.updated_at = now;
        if let serde_json::Value::Object(map) = &mut row.metadata_json {
            map.insert("dedup_demoted".to_string(), json!(true));
            if let Some(canonical) = plan.canonical_id.get(&template_natural_key(t)) {
                map.insert("dedup_canonical".to_string(), json!(canonical));
            }
        }
        demoted_templates.push(row);
    }
    store.upsert_forecast_templates(&demoted_templates).await?;
    let template_events = demoted_templates
        .iter()
        .map(|t| audit_event_for_template(t, "dedup-demote", actor_id))
        .collect::<Result<Vec<_>>>()?;
    store.insert_audit_events(&template_events).await?;

    // Deactivate the demoted templates' non-realised forecasts.
    deactivate_template_forecasts(store, &plan.demote_ids, "dedup-deactivate", actor_id).await?;

    Ok(demoted_templates.len())
}

/// Set every non-realised, non-inactive forecast belonging to `template_ids`
/// to `inativo`, with one audit event per row. Returns how many were touched.
async fn deactivate_template_forecasts(
    store: &dyn FinanceStore,
    template_ids: &std::collections::HashSet<String>,
    audit_action: &str,
    actor_id: &str,
) -> Result<usize> {
    let now = Utc::now();
    let forecasts = store.list_forecasts(None, None, None).await?;
    let mut deactivated = Vec::new();
    for f in &forecasts {
        let belongs = f
            .template_id
            .as_deref()
            .is_some_and(|tid| template_ids.contains(tid));
        if belongs && !is_realized_status(&f.status) && f.status != "inativo" {
            let mut row = f.clone();
            row.status = "inativo".to_string();
            row.updated_at = now;
            deactivated.push(row);
        }
    }
    if !deactivated.is_empty() {
        store.upsert_forecasts(&deactivated).await?;
        let forecast_events = deactivated
            .iter()
            .map(|f| audit_event_for_forecast(f, audit_action, actor_id))
            .collect::<Result<Vec<_>>>()?;
        store.insert_audit_events(&forecast_events).await?;
    }
    Ok(deactivated.len())
}

/// Retire naming forks left behind by [`merge_renamed_chains`]: an `ativo`
/// installment template that was NOT re-derived in this refresh, but whose
/// plan (account + installments_total + per-parcela amount) IS covered by a
/// template that was derived, is a stale alias of that plan — its description
/// no longer matches what the processor emits. Demote it and deactivate its
/// pending forecasts so the plan projects exactly once.
async fn retire_shadowed_templates(
    store: &dyn FinanceStore,
    derived: &[ForecastTemplateRecord],
    actor_id: &str,
) -> Result<usize> {
    use std::collections::HashSet;
    let derived_ids: HashSet<&str> = derived.iter().map(|t| t.template_id.as_str()).collect();
    let plan_of = |t: &ForecastTemplateRecord| -> Option<(String, String, Decimal)> {
        let total = match t.metadata_json.get("installments_total") {
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(serde_json::Value::String(s)) => s.clone(),
            _ => return None,
        };
        Some((
            t.account_id.clone().unwrap_or_default(),
            total,
            t.amount.abs().round_dp(2),
        ))
    };
    let derived_plans: HashSet<(String, String, Decimal)> =
        derived.iter().filter_map(plan_of).collect();

    let existing = store.list_forecast_templates(None, None).await?;
    let now = Utc::now();
    let mut retired = Vec::new();
    for t in &existing {
        let alive = matches!(t.status.as_str(), "ativo" | "active");
        if !alive || t.kind != "installment" || derived_ids.contains(t.template_id.as_str()) {
            continue;
        }
        let Some(plan) = plan_of(t) else { continue };
        if !derived_plans.contains(&plan) {
            continue;
        }
        let mut row = t.clone();
        row.status = "descartado".to_string();
        row.updated_at = now;
        if let serde_json::Value::Object(map) = &mut row.metadata_json {
            map.insert("retired_shadowed".to_string(), json!(true));
        }
        retired.push(row);
    }
    if retired.is_empty() {
        return Ok(0);
    }
    store.upsert_forecast_templates(&retired).await?;
    let events = retired
        .iter()
        .map(|t| audit_event_for_template(t, "shadow-retire", actor_id))
        .collect::<Result<Vec<_>>>()?;
    store.insert_audit_events(&events).await?;

    let ids: std::collections::HashSet<String> =
        retired.iter().map(|t| t.template_id.clone()).collect();
    deactivate_template_forecasts(store, &ids, "shadow-deactivate", actor_id).await?;
    Ok(retired.len())
}

fn shift_months_back(date: NaiveDate, n: i32) -> Result<NaiveDate> {
    shift_months(date, -n).context("falha ao deslocar data")
}

/// Add `delta` months to `date`, clamping the day if the target month is
/// shorter (e.g. 2026-01-31 + 1 → 2026-02-28).
fn shift_months(date: NaiveDate, delta: i32) -> Option<NaiveDate> {
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
    let last_day = days_in_month(year, month as u32);
    let day = date.day().min(last_day);
    NaiveDate::from_ymd_opt(year, month as u32, day)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = NaiveDate::from_ymd_opt(next_year, next_month, 1).expect("valid date");
    let last = first_next - chrono::Duration::days(1);
    last.day()
}

// ---------------------------------------------------------------------------
// Layer 2/3 — subscriptions + fixed bills
// ---------------------------------------------------------------------------

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::MathematicalOps;

#[derive(Debug, Clone)]
pub(crate) struct RecurringCandidate {
    /// `subscription` (variance ≤ 10%) or `fixed` (≤ 30%, with band).
    pub kind: String,
    pub account_id: String,
    pub merchant_key: String,
    pub label: String,
    pub category_id: Option<String>,
    /// Median absolute amount (signed negative since these are outflows).
    pub median_amount: Decimal,
    pub amount_lower: Decimal,
    pub amount_upper: Decimal,
    pub months_seen: usize,
    pub last_seen: NaiveDate,
    pub typical_day_of_month: u32,
    pub coefficient_of_variation: f64,
    pub confidence: f64,
}

impl RecurringCandidate {
    fn idempotency_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.account_id.as_bytes());
        hasher.update(b"\x1f");
        hasher.update(self.merchant_key.as_bytes());
        hasher.update(b"\x1f");
        hasher.update(self.kind.as_bytes());
        let digest = hasher.finalize();
        format!(
            "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
        )
    }

    /// Natural key of this candidate. Must equal `template_natural_key` of the
    /// template built from it (asserted in tests), so a candidate is recognised
    /// as already-known even if its derived id has drifted across releases.
    fn natural_key(&self) -> String {
        let account = if self.account_id == "any" {
            ""
        } else {
            self.account_id.as_str()
        };
        let merchant = if self.kind == "envelope" {
            String::new()
        } else {
            self.merchant_key.to_lowercase()
        };
        let category = self.category_id.as_deref().unwrap_or("");
        join_natural_key([self.kind.as_str(), account, merchant.as_str(), category, ""])
    }
}

/// Group transactions by (account, normalized merchant) and rank by
/// "looks recurring": ≥3 months seen, monthly cadence, variance bound.
/// Returns only the candidates that pass the heuristic.
pub(crate) fn detect_recurring_candidates(
    txs: &[TransactionRecord],
    today: NaiveDate,
    lookback_months: u32,
) -> Vec<RecurringCandidate> {
    use std::collections::BTreeMap;

    let cutoff = shift_months(today, -(lookback_months as i32)).unwrap_or(today);
    let mut groups: BTreeMap<(String, String), Vec<&TransactionRecord>> = BTreeMap::new();

    for tx in txs {
        // Layer 2/3 looks at outflows only. Installments are Layer 1's job.
        if tx.amount >= Decimal::ZERO {
            continue;
        }
        if tx.transaction_date < cutoff {
            continue;
        }
        let raw = enrich_description_from_metadata(&tx.raw_description, &tx.metadata_json);
        if phai_core::parse_installment_description(&raw).is_some() {
            continue;
        }
        let label = tx
            .description
            .clone()
            .or(tx.merchant_name.clone())
            .unwrap_or_else(|| raw.clone());
        let merchant_key = merchant_key_from_label(&label);
        if merchant_key.is_empty() {
            continue;
        }
        let account_id = tx.account_id.clone().unwrap_or_default();
        if account_id.is_empty() {
            continue;
        }
        groups
            .entry((account_id, merchant_key))
            .or_default()
            .push(tx);
    }

    let mut candidates = Vec::new();
    for ((account_id, merchant_key), group) in groups {
        let mut group = group;
        group.sort_by_key(|tx| tx.transaction_date);

        // Count distinct (year, month) the merchant appeared in.
        let mut months_set = std::collections::BTreeSet::new();
        for tx in &group {
            months_set.insert((tx.transaction_date.year(), tx.transaction_date.month()));
        }
        let months_seen = months_set.len();
        if months_seen < 3 {
            continue;
        }

        // Drop accounts where the cadence is clearly not monthly: median gap
        // between consecutive transactions should be 25..=35 days.
        if group.len() >= 2 {
            let mut gaps: Vec<i64> = Vec::new();
            for pair in group.windows(2) {
                gaps.push((pair[1].transaction_date - pair[0].transaction_date).num_days());
            }
            let median_gap = median_i64(&gaps);
            if !(20..=45).contains(&median_gap) {
                continue;
            }
        }

        let amounts: Vec<Decimal> = group.iter().map(|tx| tx.amount.abs()).collect();
        let median = median_decimal(&amounts);
        if median <= Decimal::ZERO {
            continue;
        }
        let stddev = stddev_decimal(&amounts);
        // Coefficient of variation isn't monetary — it's a dimensionless
        // ratio used only for the thresholds below, so an f64 cast at the
        // boundary is fine. The amounts themselves stay in Decimal.
        let cv = (stddev / median).to_f64().unwrap_or(f64::INFINITY);

        let (kind, confidence) = match cv {
            c if c.is_finite() && c <= 0.10 => ("subscription", (1.0 - c).max(0.6)),
            c if c.is_finite() && c <= 0.30 => ("fixed", (0.9 - c).max(0.4)),
            _ => continue,
        };

        // Pick the most frequent day-of-month from the seen records — that
        // becomes `next_due_day` in the template.
        let typical_day_of_month = mode_u32(
            &group
                .iter()
                .map(|tx| tx.transaction_date.day())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|| {
            group
                .last()
                .map(|tx| tx.transaction_date.day())
                .unwrap_or(1)
        });

        // Pick the most recent label so it reads natural to the user.
        let label = group
            .last()
            .and_then(|tx| tx.description.clone().or(tx.merchant_name.clone()))
            .unwrap_or_else(|| merchant_key.clone());

        // Pick the most frequent non-empty category seen on the chain.
        let category_id = group
            .iter()
            .filter_map(|tx| tx.category_id.as_ref().filter(|c| !c.is_empty()).cloned())
            .fold(BTreeMap::<String, usize>::new(), |mut acc, c| {
                *acc.entry(c).or_default() += 1;
                acc
            })
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(c, _)| c);

        let last_seen = group.last().expect("non-empty group").transaction_date;
        let median_amount = -median.round_dp(2);
        let band_half = stddev.round_dp(2);
        candidates.push(RecurringCandidate {
            kind: kind.to_string(),
            account_id,
            merchant_key,
            label,
            category_id,
            median_amount,
            amount_lower: median_amount - band_half,
            amount_upper: median_amount + band_half,
            months_seen,
            last_seen,
            typical_day_of_month,
            coefficient_of_variation: cv,
            confidence,
        });
    }

    // Highest confidence first — what the user is most likely to confirm.
    candidates.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
}

fn merchant_key_from_label(label: &str) -> String {
    normalize_description(&strip_installment_marker(label))
}

fn median_decimal(values: &[Decimal]) -> Decimal {
    if values.is_empty() {
        return Decimal::ZERO;
    }
    let mut sorted = values.to_vec();
    sorted.sort();
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / Decimal::TWO
    } else {
        sorted[mid]
    }
}

fn median_i64(values: &[i64]) -> i64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

/// Population standard deviation of `values`, computed entirely in
/// `Decimal`. Returns `Decimal::ZERO` when the input has fewer than two
/// samples or when `sqrt` fails (e.g. on a negative variance produced
/// by accumulated rounding — shouldn't happen here but stays defensive).
fn stddev_decimal(values: &[Decimal]) -> Decimal {
    if values.len() < 2 {
        return Decimal::ZERO;
    }
    let count = Decimal::from(values.len());
    let mean = values.iter().sum::<Decimal>() / count;
    let var = values
        .iter()
        .map(|v| {
            let diff = *v - mean;
            diff * diff
        })
        .sum::<Decimal>()
        / count;
    var.sqrt().unwrap_or(Decimal::ZERO)
}

fn mode_u32(values: &[u32]) -> Option<u32> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for v in values {
        *counts.entry(*v).or_default() += 1;
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(v, _)| v)
}

/// Per-category envelope detector (Layer 4 of ADR-0016). Groups outflows
/// by `category_id` over the lookback window, requires ≥4 months of data,
/// and a coefficient of variation ≤ 0.40 on the monthly totals — envelopes
/// are inherently noisier than subscriptions, so the threshold is wider.
///
/// To avoid double-counting with already-active subscription / fixed
/// templates, transactions whose normalized merchant matches one of those
/// templates (within the same account) are excluded before aggregation.
pub(crate) fn detect_envelope_candidates(
    txs: &[TransactionRecord],
    today: NaiveDate,
    lookback_months: u32,
    excluded_merchants_per_account: &std::collections::BTreeMap<
        String,
        std::collections::BTreeSet<String>,
    >,
) -> Vec<RecurringCandidate> {
    use std::collections::BTreeMap;

    let cutoff = shift_months(today, -(lookback_months as i32)).unwrap_or(today);

    // Per (category_id) → per (year,month) → sum of outflow magnitudes.
    let mut buckets: BTreeMap<String, BTreeMap<(i32, u32), Decimal>> = BTreeMap::new();
    // Track the most recent label per category for the description field.
    let mut latest_label: BTreeMap<String, String> = BTreeMap::new();

    for tx in txs {
        if tx.amount >= Decimal::ZERO {
            continue;
        }
        if tx.transaction_date < cutoff {
            continue;
        }
        let raw = enrich_description_from_metadata(&tx.raw_description, &tx.metadata_json);
        if phai_core::parse_installment_description(&raw).is_some() {
            continue;
        }
        let category_id = match tx.category_id.as_ref().filter(|c| !c.is_empty()) {
            Some(c) => c.clone(),
            None => continue,
        };
        // Skip pseudo-categories that aren't real spend.
        if matches!(
            category_id.as_str(),
            "transfer-internal" | "credit-card-payment" | "cashback"
        ) {
            continue;
        }
        // Skip transactions already covered by an accepted subscription/fixed
        // template — those are accounted for separately and would otherwise
        // inflate the envelope estimate.
        if let Some(account_id) = tx.account_id.as_ref() {
            let label = tx
                .description
                .clone()
                .or(tx.merchant_name.clone())
                .unwrap_or_else(|| raw.clone());
            let key = merchant_key_from_label(&label);
            if let Some(excluded) = excluded_merchants_per_account.get(account_id) {
                if excluded.contains(&key) {
                    continue;
                }
            }
        }

        let month = (tx.transaction_date.year(), tx.transaction_date.month());
        let amount = tx.amount.abs();
        *buckets
            .entry(category_id.clone())
            .or_default()
            .entry(month)
            .or_default() += amount;
        latest_label.insert(category_id, format_category_label(&tx.category_id));
    }

    let mut out = Vec::new();
    for (category_id, months) in buckets {
        if months.len() < 4 {
            continue;
        }
        let totals: Vec<Decimal> = months.values().copied().collect();
        let median = median_decimal(&totals);
        if median < Decimal::from(50) {
            // Skip noise: categories that barely register a small amount
            // per month aren't worth materialising into the chart.
            continue;
        }
        let stddev = stddev_decimal(&totals);
        let cv = (stddev / median).to_f64().unwrap_or(f64::INFINITY);
        if !cv.is_finite() || cv > 0.40 {
            continue;
        }
        let confidence = (0.7 - cv).clamp(0.3, 0.7);
        let last_month_key = months
            .keys()
            .max()
            .copied()
            .unwrap_or((today.year(), today.month()));
        let last_seen =
            NaiveDate::from_ymd_opt(last_month_key.0, last_month_key.1, 15).unwrap_or(today);
        let median_amount = -median.round_dp(2);
        let band_half = stddev.round_dp(2);
        let label = latest_label
            .get(&category_id)
            .cloned()
            .unwrap_or_else(|| category_id.clone());
        out.push(RecurringCandidate {
            kind: "envelope".to_string(),
            // Envelopes are not account-scoped (they're a category total
            // across the whole household). We still use a marker so the
            // idempotency hash is unique per category.
            account_id: "any".to_string(),
            merchant_key: format!("envelope:{category_id}"),
            label,
            category_id: Some(category_id),
            median_amount,
            amount_lower: median_amount - band_half,
            amount_upper: median_amount + band_half,
            months_seen: months.len(),
            last_seen,
            typical_day_of_month: 15,
            coefficient_of_variation: cv,
            confidence,
        });
    }

    out.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn format_category_label(category_id: &Option<String>) -> String {
    category_id
        .clone()
        .unwrap_or_else(|| "(sem categoria)".to_string())
}

/// CLI entry: `fin forecast suggest`. Detects new candidates, persists them
/// as `status='proposto'` templates (so the user can later accept / dismiss
/// without us re-suggesting), and lists pending proposals.
pub(crate) async fn run_suggest(args: ForecastSuggestArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let today = Utc::now().date_naive();
    let lookback = args.lookback_months.max(3);
    let from = shift_months_back(today, lookback as i32)?;
    let txs = store
        .transactions_in_date_range(None, from, today)
        .await
        .context("falha ao carregar transações")?;

    // Layers 2 + 3 first — these are merchant-scoped.
    let mut candidates = detect_recurring_candidates(&txs, today, lookback);

    // Layer 4 (envelopes) — fed with the set of merchants already covered
    // by active subscription/fixed templates so we don't double-count.
    let active = store.list_forecast_templates(None, Some("ativo")).await?;
    let mut excluded_merchants: std::collections::BTreeMap<
        String,
        std::collections::BTreeSet<String>,
    > = std::collections::BTreeMap::new();
    for tpl in &active {
        if tpl.kind != "subscription" && tpl.kind != "fixed" {
            continue;
        }
        if let (Some(account), Some(merchant)) = (&tpl.account_id, &tpl.merchant_pattern) {
            excluded_merchants
                .entry(account.clone())
                .or_default()
                .insert(merchant.clone());
        }
    }
    let envelope_candidates =
        detect_envelope_candidates(&txs, today, lookback, &excluded_merchants);
    candidates.extend(envelope_candidates);

    // Skip any candidate whose template_id already exists (in any status —
    // proposto/ativo/descartado). That's the "remember-the-rejection"
    // semantics from ADR-0016.
    let existing_proposto = store
        .list_forecast_templates(None, Some("proposto"))
        .await?;
    let existing_ativo = active;
    let existing_descartado = store
        .list_forecast_templates(None, Some("descartado"))
        .await?;
    let mut existing_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for t in existing_proposto
        .iter()
        .chain(existing_ativo.iter())
        .chain(existing_descartado.iter())
    {
        existing_keys.insert(template_natural_key(t));
    }

    let now = Utc::now();
    let mut new_proposals = Vec::new();
    for cand in &candidates {
        // Skip by natural key: a template for this identity already exists
        // (even one whose derived id drifted across releases), so re-proposing
        // would fork a duplicate.
        if existing_keys.contains(&cand.natural_key()) {
            continue;
        }
        let template_id = format!("{}-{}", cand.kind, cand.idempotency_hash());
        new_proposals.push(template_from_candidate(
            cand,
            template_id,
            &config.actor_id,
            now,
        ));
    }

    if !new_proposals.is_empty() {
        store.upsert_forecast_templates(&new_proposals).await?;
        let events = new_proposals
            .iter()
            .map(|t| audit_event_for_template(t, "upsert", &config.actor_id))
            .collect::<Result<Vec<_>>>()?;
        store.insert_audit_events(&events).await?;
    }

    // Re-read the full proposto list so we can print it.
    let proposto_now = store
        .list_forecast_templates(None, Some("proposto"))
        .await?;
    if args.raw {
        println!("{}", serde_json::to_string_pretty(&proposto_now)?);
    } else {
        print_suggest_human(&proposto_now, new_proposals.len());
    }
    Ok(())
}

fn template_from_candidate(
    cand: &RecurringCandidate,
    template_id: String,
    actor_id: &str,
    now: chrono::DateTime<Utc>,
) -> ForecastTemplateRecord {
    // Subscriptions are tight (single value, no band). Fixed bills and
    // envelopes carry the ±σ band so the chart can surface variance.
    let amount_lower = if cand.kind == "subscription" {
        None
    } else {
        Some(cand.amount_lower)
    };
    let amount_upper = if cand.kind == "subscription" {
        None
    } else {
        Some(cand.amount_upper)
    };
    // Envelopes span the whole household (no specific account) — the
    // detector marks them with the "any" sentinel.
    let account_id = if cand.account_id == "any" {
        None
    } else {
        Some(cand.account_id.clone())
    };
    // Envelopes pattern by category, not merchant.
    let merchant_pattern = if cand.kind == "envelope" {
        None
    } else {
        Some(cand.merchant_key.clone())
    };
    ForecastTemplateRecord {
        template_id: template_id.clone(),
        kind: cand.kind.clone(),
        description: cand.label.clone(),
        merchant_pattern,
        category_id: cand.category_id.clone(),
        account_id,
        amount: cand.median_amount,
        amount_lower,
        amount_upper,
        cadence: "monthly".to_string(),
        next_due_day: Some(cand.typical_day_of_month as i32),
        start_date: cand.last_seen,
        end_date: None,
        remaining_count: None,
        source: "detected".to_string(),
        confidence: Some(cand.confidence),
        status: "proposto".to_string(),
        metadata_json: json!({
            "months_seen": cand.months_seen,
            "coefficient_of_variation": cand.coefficient_of_variation,
            "detector_version": 1,
        }),
        actor_id: actor_id.to_string(),
        idempotency_key: format!("forecast-template-{template_id}"),
        created_at: now,
        updated_at: now,
    }
}

fn print_suggest_human(templates: &[ForecastTemplateRecord], new_count: usize) {
    if templates.is_empty() {
        println!("🔎 Nenhum candidato recorrente em revisão.");
        return;
    }
    println!(
        "🔎 {} candidato(s) recorrente(s) em revisão ({new_count} novos)",
        templates.len()
    );
    println!();
    for t in templates {
        let amount = crate::human_format::brl(t.amount.abs());
        let cadence_label = format!("mensal · dia {}", t.next_due_day.unwrap_or(1));
        let conf = t
            .confidence
            .map(|c| format!("{:.0}%", c * 100.0))
            .unwrap_or_default();
        let band = match (t.amount_lower, t.amount_upper) {
            (Some(lo), Some(hi)) => format!(
                " (±{})",
                crate::human_format::brl((hi - lo) / Decimal::from(2))
            ),
            _ => String::new(),
        };
        println!("• [{}] {}", t.kind, t.description);
        println!("   {amount}{band} · {cadence_label} · confiança {conf}");
        println!("   id={}", t.template_id);
    }
    println!();
    println!("Para aceitar: fin forecast accept --template-id <id>");
    println!("Para descartar: fin forecast dismiss --template-id <id>");
}

pub(crate) async fn run_accept(args: ForecastAcceptArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let template = store
        .get_forecast_template(&args.template_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("template não encontrado: {}", args.template_id))?;
    if template.status != "proposto" {
        anyhow::bail!(
            "template {} não está em status 'proposto' (atual: {})",
            template.template_id,
            template.status
        );
    }
    let now = Utc::now();
    let mut updated = template.clone();
    updated.status = "ativo".to_string();
    updated.updated_at = now;
    store
        .upsert_forecast_templates(std::slice::from_ref(&updated))
        .await?;
    store
        .insert_audit_events(&[audit_event_for_template(
            &updated,
            "accept",
            &config.actor_id,
        )?])
        .await?;

    // Materialise next N months of instances right away.
    let materialised = materialise_template_forecasts(
        store.as_ref(),
        &updated,
        args.materialize_months,
        &config.actor_id,
        now,
    )
    .await?;

    println!(
        "✅ Aceito: {} · {} forecast(s) gravado(s) para os próximos {} meses.",
        updated.description, materialised, args.materialize_months
    );
    Ok(())
}

pub(crate) async fn run_dismiss(args: ForecastDismissArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let template = store
        .get_forecast_template(&args.template_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("template não encontrado: {}", args.template_id))?;
    if template.status == "descartado" {
        println!("ℹ️  Template já descartado: {}", template.template_id);
        return Ok(());
    }
    let mut updated = template.clone();
    updated.status = "descartado".to_string();
    updated.updated_at = Utc::now();
    store
        .upsert_forecast_templates(std::slice::from_ref(&updated))
        .await?;
    store
        .insert_audit_events(&[audit_event_for_template(
            &updated,
            "dismiss",
            &config.actor_id,
        )?])
        .await?;
    println!("🗑  Descartado: {}", updated.description);
    Ok(())
}

/// Materialise N monthly forecast instances ahead of `today` for a given
/// template. Idempotent on `forecast_id` (`tpl-{template_id}-YYYYMM`).
pub(crate) async fn materialise_template_forecasts(
    store: &dyn FinanceStore,
    template: &ForecastTemplateRecord,
    months_ahead: u32,
    actor_id: &str,
    now: chrono::DateTime<Utc>,
) -> Result<usize> {
    let today = now.date_naive();
    let day_of_month = template.next_due_day.unwrap_or(1).max(1) as u32;
    let mut instances = Vec::with_capacity(months_ahead as usize);
    for offset in 1..=months_ahead {
        let base = shift_months(
            NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
                .context("invalid current month")?,
            offset as i32,
        )
        .context("month shift failed")?;
        let last_day = days_in_month(base.year(), base.month());
        let due_date =
            NaiveDate::from_ymd_opt(base.year(), base.month(), day_of_month.min(last_day))
                .context("invalid due_date")?;
        let yyyymm = format!("{}{:02}", base.year(), base.month());
        let forecast_id = format!("tpl-{}-{yyyymm}", template.template_id);
        instances.push(ForecastRecord {
            forecast_id: forecast_id.clone(),
            due_date: Some(due_date),
            description: template.description.clone(),
            amount: template.amount,
            category_id: template.category_id.clone(),
            account_id: template.account_id.clone(),
            status: "ativo".to_string(),
            recurrence: Some("mensal".to_string()),
            actor_id: actor_id.to_string(),
            idempotency_key: format!("forecast-{forecast_id}"),
            metadata_json: json!({
                "source_template": template.template_id,
                "template_kind": template.kind,
            }),
            created_at: now,
            updated_at: now,
            template_id: Some(template.template_id.clone()),
            realized_transaction_id: None,
            realized_at: None,
        });
    }
    if instances.is_empty() {
        return Ok(0);
    }
    let upserted = store.upsert_forecasts(&instances).await?;
    let events = instances
        .iter()
        .map(|f| audit_event_for_forecast(f, "upsert", actor_id))
        .collect::<Result<Vec<_>>>()?;
    store.insert_audit_events(&events).await?;
    Ok(upserted)
}

// ---------------------------------------------------------------------------
// Scenario evaluation (Layer 5 — read-only what-if)
// ---------------------------------------------------------------------------

/// CLI entry: `fin forecast scenario`. Pure compute, no DB writes. Projects
/// the saldo trajectory for the next N months with and without a hypothetical
/// recurring commitment, returns the deltas and (optionally) the first
/// month the projected saldo would fall below `--minimum-balance`.
pub(crate) async fn run_scenario(args: ForecastScenarioArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let today = Utc::now().date_naive();
    let amount = Decimal::from_str(&args.amount)
        .with_context(|| format!("--amount inválido: {}", args.amount))?;
    let minimum_balance = match &args.minimum_balance {
        Some(s) => {
            Some(Decimal::from_str(s).with_context(|| format!("--minimum-balance inválido: {s}"))?)
        }
        None => None,
    };
    let start_month = match &args.start {
        Some(s) => parse_month_start(s)?,
        None => shift_months(first_of_month(today)?, 1).context("falha ao calcular próximo mês")?,
    };
    let project_months = args.project_months.clamp(1, 36);
    let scenario_months = args.months.max(1);

    // Anchor today's balance.
    let initial = store
        .checking_balance_at(today)
        .await?
        .map(|b| b.balance)
        .context(
            "Saldo atual não pôde ser ancorado — rode `finance sync pluggy` ou \
             cheque se há snapshot recente para todas as contas correntes.",
        )?;

    // Walk projection: each month we add forecast_net_remaining for that month.
    let mut baseline = initial;
    let mut with_scenario = initial;
    let mut current_month = first_of_month(today)?;
    let mut first_breach: Option<NaiveDate> = None;
    let mut by_month_baseline: Vec<(NaiveDate, Decimal)> = Vec::new();
    let mut by_month_scenario: Vec<(NaiveDate, Decimal)> = Vec::new();

    for _ in 0..project_months {
        let last_day = last_day_of_month(current_month)?;
        let lower = today.succ_opt().unwrap_or(today).max(current_month);
        let mut forecast_net = Decimal::ZERO;
        if lower <= last_day {
            let fcs = store.upcoming_forecasts(lower, last_day).await?;
            for f in &fcs {
                forecast_net += f.amount;
            }
        }
        baseline += forecast_net;
        with_scenario += forecast_net;
        if month_within_scenario(current_month, start_month, scenario_months)? {
            with_scenario += amount;
        }
        by_month_baseline.push((current_month, baseline));
        by_month_scenario.push((current_month, with_scenario));
        if let Some(min) = minimum_balance {
            if first_breach.is_none() && with_scenario < min {
                first_breach = Some(current_month);
            }
        }
        current_month = shift_months(current_month, 1).context("month shift")?;
    }

    let final_baseline = baseline;
    let final_scenario = with_scenario;
    let delta = final_scenario - final_baseline;
    let last_month = current_month.pred_opt().unwrap_or(current_month);

    if args.raw {
        let payload = json!({
            "scenario_description": args.description,
            "scenario_amount": amount.to_string(),
            "scenario_start": start_month.format("%Y-%m").to_string(),
            "scenario_months": scenario_months,
            "project_months": project_months,
            "initial_balance": initial.to_string(),
            "baseline_final_balance": final_baseline.to_string(),
            "scenario_final_balance": final_scenario.to_string(),
            "delta_total": delta.to_string(),
            "first_breach_month": first_breach.map(|d| d.format("%Y-%m").to_string()),
            "minimum_balance": minimum_balance.map(|d| d.to_string()),
            "monthly_baseline": by_month_baseline
                .iter()
                .map(|(d, v)| json!({"month": d.format("%Y-%m").to_string(), "balance": v.to_string()}))
                .collect::<Vec<_>>(),
            "monthly_scenario": by_month_scenario
                .iter()
                .map(|(d, v)| json!({"month": d.format("%Y-%m").to_string(), "balance": v.to_string()}))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!(
            "🔮 Cenário: {} ({}/mês por {} meses, início {})",
            args.description,
            crate::human_format::brl_signed(amount),
            scenario_months,
            start_month.format("%Y-%m"),
        );
        println!();
        println!(
            "  Saldo hoje                {}",
            crate::human_format::brl(initial)
        );
        println!(
            "  Saldo projetado em {}   {} (baseline)",
            last_month.format("%Y-%m"),
            crate::human_format::brl(final_baseline)
        );
        println!(
            "  Saldo projetado em {}   {} (com cenário)",
            last_month.format("%Y-%m"),
            crate::human_format::brl(final_scenario)
        );
        println!(
            "  Δ total no horizonte      {}",
            crate::human_format::brl_signed(delta)
        );
        if let Some(min) = minimum_balance {
            match first_breach {
                Some(month) => println!(
                    "  ⚠️  Saldo cairia abaixo de {} em {}",
                    crate::human_format::brl(min),
                    month.format("%Y-%m")
                ),
                None => println!(
                    "  ✅ Saldo permanece ≥ {} durante todo o horizonte.",
                    crate::human_format::brl(min)
                ),
            }
        }
    }
    Ok(())
}

fn first_of_month(date: NaiveDate) -> Result<NaiveDate> {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
        .context("data inválida ao calcular primeiro dia do mês")
}

fn last_day_of_month(first: NaiveDate) -> Result<NaiveDate> {
    let next = shift_months(first, 1).context("month shift")?;
    next.pred_opt().context("last day calc")
}

fn parse_month_start(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d")
        .with_context(|| format!("--start inválido: {value} (esperado YYYY-MM)"))
}

fn month_within_scenario(month: NaiveDate, start: NaiveDate, months: u32) -> Result<bool> {
    if month < start {
        return Ok(false);
    }
    let end = shift_months(start, months as i32 - 1).context("falha ao calcular fim do cenário")?;
    Ok(month <= end)
}

// ---------------------------------------------------------------------------
// Reconciliation (forecast → realizado)
// ---------------------------------------------------------------------------

/// Default lookback window for the reconciler: how far back to scan for
/// `status='ativo'` forecasts whose due_date is already in the past. Past
/// this window, an active forecast is treated as "missed" and left alone —
/// the user can still flip it manually.
pub const RECONCILE_DEFAULT_LOOKBACK_DAYS: i64 = 45;

/// Day window around the forecast's due_date in which a candidate
/// transaction must fall to be considered a match.
const RECONCILE_DATE_TOLERANCE_DAYS: i64 = 3;

/// Relative tolerance on the amount magnitude (5%).
const RECONCILE_AMOUNT_TOLERANCE: f64 = 0.05;

#[derive(Debug, Default, Clone)]
pub struct ReconcileReport {
    pub forecasts_scanned: usize,
    pub matched: usize,
    pub ambiguous: usize,
    pub no_match: usize,
}

pub(crate) async fn run_reconcile(args: ForecastReconcileArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let report = reconcile_forecasts(store.as_ref(), &config, args.lookback_days).await?;

    if args.raw {
        let payload = json!({
            "forecasts_scanned": report.forecasts_scanned,
            "matched": report.matched,
            "ambiguous": report.ambiguous,
            "no_match": report.no_match,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("🔗 Forecast · reconciliação");
        println!("  Forecasts varridos:   {}", report.forecasts_scanned);
        println!("  Realizados:           {}", report.matched);
        println!("  Ambíguos (pulados):   {}", report.ambiguous);
        println!("  Sem match:            {}", report.no_match);
    }
    Ok(())
}

/// Core reconciler: for each ativo forecast with due_date in the lookback
/// window, find a candidate transaction on the same account within
/// ±[`RECONCILE_DATE_TOLERANCE_DAYS`] whose amount is within ±5% of the
/// forecast amount (and same sign). On a unique match, flip the forecast
/// to `realizado` and record the FK + timestamp. Emits a `reconcile` audit
/// event per realized forecast.
pub async fn reconcile_forecasts(
    store: &dyn FinanceStore,
    config: &AppConfig,
    lookback_days: i64,
) -> Result<ReconcileReport> {
    let today = Utc::now().date_naive();
    let lookback = lookback_days.max(1);
    let from = today
        .checked_sub_signed(chrono::Duration::days(lookback))
        .unwrap_or(today);

    let candidates = store.upcoming_forecasts(from, today).await?;
    let mut report = ReconcileReport {
        forecasts_scanned: candidates.len(),
        ..Default::default()
    };

    let mut to_update: Vec<ForecastRecord> = Vec::new();
    let now = Utc::now();

    for forecast in candidates {
        // Already realised (defensive — upcoming_forecasts filters by ativo).
        if forecast.realized_transaction_id.is_some() {
            continue;
        }
        let Some(due) = forecast.due_date else {
            report.no_match += 1;
            continue;
        };
        let Some(account_id) = forecast.account_id.clone() else {
            report.no_match += 1;
            continue;
        };
        let from_date = due
            .checked_sub_signed(chrono::Duration::days(RECONCILE_DATE_TOLERANCE_DAYS))
            .unwrap_or(due);
        let to_date = due
            .checked_add_signed(chrono::Duration::days(RECONCILE_DATE_TOLERANCE_DAYS))
            .unwrap_or(due);
        let txs = store
            .effective_transactions_window(Some(&account_id), from_date, to_date)
            .await
            .context("falha ao carregar transações para reconciliar forecast")?;

        let matches: Vec<&TransactionRecord> = txs
            .iter()
            .filter(|tx| amount_matches(forecast.amount, tx.amount))
            .collect();

        match matches.len() {
            0 => report.no_match += 1,
            1 => {
                let tx = matches[0];
                let mut updated = forecast.clone();
                preserve_predicted_amount(&mut updated);
                stamp_realized_amount_metadata(&mut updated, tx, "auto");
                updated.amount = tx.amount;
                updated.status = "realizado".to_string();
                updated.realized_transaction_id = Some(tx.transaction_id.clone());
                updated.realized_at = Some(now);
                updated.updated_at = now;
                to_update.push(updated);
                report.matched += 1;
            }
            _ => {
                // Multiple candidate transactions match — bail rather than
                // guess which one fulfilled the forecast. The user can
                // disambiguate manually if needed.
                report.ambiguous += 1;
            }
        }
    }

    if !to_update.is_empty() {
        store.upsert_forecasts(&to_update).await?;
        let events = to_update
            .iter()
            .map(|f| audit_event_for_forecast(f, "reconcile", &config.actor_id))
            .collect::<Result<Vec<_>>>()?;
        store.insert_audit_events(&events).await?;
    }

    Ok(report)
}

/// True when `tx_amount` matches `forecast_amount` in sign and magnitude
/// (within [`RECONCILE_AMOUNT_TOLERANCE`]). Returns false when the forecast
/// amount is zero — we never auto-realise a zero forecast.
pub(crate) fn amount_matches(forecast_amount: Decimal, tx_amount: Decimal) -> bool {
    if forecast_amount.is_zero() {
        return false;
    }
    // Same sign required (an outflow forecast doesn't match an inflow tx).
    if (forecast_amount > Decimal::ZERO) != (tx_amount > Decimal::ZERO) {
        return false;
    }
    let expected = forecast_amount.abs().to_f64().unwrap_or(0.0);
    let actual = tx_amount.abs().to_f64().unwrap_or(0.0);
    if expected <= 0.0 {
        return false;
    }
    let rel = (actual - expected).abs() / expected;
    rel <= RECONCILE_AMOUNT_TOLERANCE
}

fn preserve_predicted_amount(record: &mut ForecastRecord) {
    if !record.metadata_json.is_object() {
        record.metadata_json = json!({});
    }
    if let Some(obj) = record.metadata_json.as_object_mut() {
        obj.entry("predicted_amount".to_string())
            .or_insert_with(|| json!(record.amount.to_string()));
    }
}

fn stamp_realized_amount_metadata(
    record: &mut ForecastRecord,
    tx: &TransactionRecord,
    source: &str,
) {
    preserve_predicted_amount(record);
    if let Some(obj) = record.metadata_json.as_object_mut() {
        let predicted = obj
            .get("predicted_amount")
            .and_then(|value| value.as_str())
            .and_then(|value| Decimal::from_str(value).ok())
            .unwrap_or(record.amount);
        obj.insert("ui_role".to_string(), json!("planned_transaction"));
        obj.insert("realized_amount".to_string(), json!(tx.amount.to_string()));
        obj.insert(
            "realized_transaction_date".to_string(),
            json!(tx.transaction_date.to_string()),
        );
        obj.insert(
            "realized_transaction_description".to_string(),
            json!(tx.display_description()),
        );
        obj.insert("realization_source".to_string(), json!(source));
        obj.insert(
            "amount_variance".to_string(),
            json!((tx.amount - predicted).to_string()),
        );
    }
}

// ---------------------------------------------------------------------------
// Refresh orchestrator (full pipeline)
// ---------------------------------------------------------------------------

/// Summary of one full `refresh` pass — what the orchestrator did across the
/// 4 layers + reconciliation. Returned both to the CLI and to the
/// post-`sync pluggy` hook so the user sees a single line of feedback.
#[derive(Debug, Default, Clone)]
pub struct RefreshReport {
    pub installments: InstallmentsRefreshReport,
    pub reconcile: ReconcileReport,
    pub templates_materialised: usize,
    pub forecasts_materialised: usize,
    pub new_suggestions: usize,
    /// Number of open-bill forecast rows upserted (one per card with pending charges).
    pub open_bill_forecasts: usize,
    /// Number of duplicate templates demoted by the self-healing dedup pass.
    pub templates_deduped: usize,
}

pub(crate) async fn run_refresh(args: ForecastRefreshArgs) -> Result<()> {
    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let report = refresh_all(
        store.as_ref(),
        &config,
        args.lookback_months,
        args.materialize_months,
        args.skip_suggest,
    )
    .await?;

    if args.raw {
        let payload = json!({
            "installments": {
                "chains_seen": report.installments.chains_seen,
                "chains_active": report.installments.chains_active,
                "templates_upserted": report.installments.templates_upserted,
                "forecasts_upserted": report.installments.forecasts_upserted,
            },
            "reconcile": {
                "forecasts_scanned": report.reconcile.forecasts_scanned,
                "matched": report.reconcile.matched,
                "ambiguous": report.reconcile.ambiguous,
                "no_match": report.reconcile.no_match,
            },
            "templates_materialised": report.templates_materialised,
            "forecasts_materialised": report.forecasts_materialised,
            "open_bill_forecasts": report.open_bill_forecasts,
            "new_suggestions": report.new_suggestions,
            "templates_deduped": report.templates_deduped,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_refresh_report(&report);
    }
    Ok(())
}

/// Full orchestrator. Wraps Layer 1 (installments), reconciliation,
/// materialisation of N months ahead for every ativo template, and
/// (optionally) a fresh `suggest` pass that surfaces new Layer 2/3/4
/// candidates as `proposto` templates.
pub async fn refresh_all(
    store: &dyn FinanceStore,
    config: &AppConfig,
    lookback_months: u32,
    materialize_months: u32,
    skip_suggest: bool,
) -> Result<RefreshReport> {
    // Self-heal first: collapse any duplicate templates (same identity, drifted
    // id) so the materialisation pass below can't re-emit their duplicate
    // forecasts. Idempotent — a no-op once the store is clean (ADR-0022).
    let templates_deduped = collapse_duplicate_templates(store, &config.actor_id).await?;

    let installments = refresh_installments(store, config, lookback_months).await?;
    let reconcile = reconcile_forecasts(store, config, RECONCILE_DEFAULT_LOOKBACK_DAYS).await?;

    // For every accepted template (subscription / fixed / envelope) ensure
    // the next N months of instances exist. Idempotent on forecast_id.
    let active = store.list_forecast_templates(None, Some("ativo")).await?;
    let now = Utc::now();
    let mut templates_materialised = 0usize;
    let mut forecasts_materialised = 0usize;
    for tpl in &active {
        // Installment templates already materialise their own remaining
        // parcelas in `refresh_installments` — skip them here to avoid
        // double-emitting audit events for the same forecast rows.
        if tpl.kind == "installment" {
            continue;
        }
        let count =
            materialise_template_forecasts(store, tpl, materialize_months, &config.actor_id, now)
                .await?;
        if count > 0 {
            templates_materialised += 1;
            forecasts_materialised += count;
        }
    }

    let open_bill_forecasts = refresh_open_card_bills(store, &config.actor_id, now).await?;

    let new_suggestions = if skip_suggest {
        0
    } else {
        run_suggest_silent(store, config, lookback_months).await?
    };

    Ok(RefreshReport {
        installments,
        reconcile,
        templates_materialised,
        forecasts_materialised,
        new_suggestions,
        open_bill_forecasts,
        templates_deduped,
    })
}

/// Same detection logic as `run_suggest`, but returns the count of new
/// `proposto` templates persisted (no stdout). Used by the orchestrator and
/// the post-sync hook so the noisy listing doesn't fire on every sync.
async fn run_suggest_silent(
    store: &dyn FinanceStore,
    config: &AppConfig,
    lookback_months: u32,
) -> Result<usize> {
    let today = Utc::now().date_naive();
    let lookback = lookback_months.max(3);
    let from = shift_months_back(today, lookback as i32)?;
    let txs = store
        .transactions_in_date_range(None, from, today)
        .await
        .context("falha ao carregar transações")?;

    let mut candidates = detect_recurring_candidates(&txs, today, lookback);

    let active = store.list_forecast_templates(None, Some("ativo")).await?;
    let mut excluded_merchants: std::collections::BTreeMap<
        String,
        std::collections::BTreeSet<String>,
    > = std::collections::BTreeMap::new();
    for tpl in &active {
        if tpl.kind != "subscription" && tpl.kind != "fixed" {
            continue;
        }
        if let (Some(account), Some(merchant)) = (&tpl.account_id, &tpl.merchant_pattern) {
            excluded_merchants
                .entry(account.clone())
                .or_default()
                .insert(merchant.clone());
        }
    }
    candidates.extend(detect_envelope_candidates(
        &txs,
        today,
        lookback,
        &excluded_merchants,
    ));

    let existing_proposto = store
        .list_forecast_templates(None, Some("proposto"))
        .await?;
    let existing_descartado = store
        .list_forecast_templates(None, Some("descartado"))
        .await?;
    let mut existing_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for t in existing_proposto
        .iter()
        .chain(active.iter())
        .chain(existing_descartado.iter())
    {
        existing_keys.insert(template_natural_key(t));
    }

    let now = Utc::now();
    let mut new_proposals = Vec::new();
    for cand in &candidates {
        // Skip by natural key (see `run_suggest`): avoids re-proposing an
        // identity whose derived id has drifted into a duplicate.
        if existing_keys.contains(&cand.natural_key()) {
            continue;
        }
        let template_id = format!("{}-{}", cand.kind, cand.idempotency_hash());
        new_proposals.push(template_from_candidate(
            cand,
            template_id,
            &config.actor_id,
            now,
        ));
    }

    if !new_proposals.is_empty() {
        store.upsert_forecast_templates(&new_proposals).await?;
        let events = new_proposals
            .iter()
            .map(|t| audit_event_for_template(t, "upsert", &config.actor_id))
            .collect::<Result<Vec<_>>>()?;
        store.insert_audit_events(&events).await?;
    }
    Ok(new_proposals.len())
}

// ---------------------------------------------------------------------------
// Open card-bill forecasts
// ---------------------------------------------------------------------------

/// Materialize one `forecast` row per credit card that has a non-zero open
/// bill (`payment_status = 'pending'` charges in the current cycle). The row
/// is keyed on `(account_id, cycle_month)` so it is updated in-place on every
/// refresh as new purchases accumulate. Skips cards without `billing_due_day`
/// configured in `accounts.metadata_json`.
pub async fn refresh_open_card_bills(
    store: &dyn FinanceStore,
    actor_id: &str,
    now: chrono::DateTime<Utc>,
) -> Result<usize> {
    let open_cards = store
        .cards_open_now()
        .await
        .context("falha ao carregar faturas abertas")?;
    if open_cards.is_empty() {
        return Ok(0);
    }

    let accounts = store
        .get_accounts()
        .await
        .context("falha ao carregar contas")?;

    let mut forecasts: Vec<ForecastRecord> = Vec::new();
    for card in &open_cards {
        if card.open_amount.is_zero() {
            continue;
        }
        let Some(acc) = accounts.iter().find(|a| a.account_id == card.account_id) else {
            continue;
        };
        let Some(due_day) = billing_due_day_from_account(acc) else {
            continue;
        };
        let Some(due_date) = card_open_bill_due_date(&card.month_ref, due_day) else {
            continue;
        };
        let forecast_id = card_open_bill_forecast_id(&card.account_id, &card.month_ref);
        let label = if acc.label.is_empty() {
            acc.account_id.as_str()
        } else {
            acc.label.as_str()
        };
        forecasts.push(ForecastRecord {
            forecast_id: forecast_id.clone(),
            due_date: Some(due_date),
            description: format!("Fatura {label}"),
            amount: -card.open_amount,
            category_id: None,
            account_id: Some(card.account_id.clone()),
            status: "ativo".to_string(),
            recurrence: Some("card-cycle".to_string()),
            actor_id: actor_id.to_string(),
            idempotency_key: forecast_id,
            metadata_json: json!({
                "source": "card-open-bill",
                "cycle_month": card.month_ref,
            }),
            created_at: now,
            updated_at: now,
            template_id: None,
            realized_transaction_id: None,
            realized_at: None,
        });
    }

    let count = forecasts.len();
    if !forecasts.is_empty() {
        store
            .upsert_forecasts(&forecasts)
            .await
            .context("falha ao gravar forecasts de fatura aberta")?;
        let events = forecasts
            .iter()
            .map(|f| audit_event_for_forecast(f, "upsert", actor_id))
            .collect::<Result<Vec<_>>>()?;
        store.insert_audit_events(&events).await?;
    }
    Ok(count)
}

fn billing_due_day_from_account(acc: &AccountRecord) -> Option<u32> {
    acc.metadata_json
        .get("billing_due_day")?
        .as_str()?
        .parse::<u32>()
        .ok()
        .filter(|&d| (1..=31).contains(&d))
}

/// Due date for an open bill: `billing_due_day` of the `month_ref` cycle
/// month (clamped to the last day of that month). The cycle month is
/// already the month in which the bill closes, so the due date falls within
/// the same month.
fn card_open_bill_due_date(month_ref: &str, due_day: u32) -> Option<NaiveDate> {
    let (year_str, month_str) = month_ref.split_once('-')?;
    let y: i32 = year_str.parse().ok()?;
    let m: u32 = month_str.parse().ok()?;
    let last = days_in_month(y, m);
    NaiveDate::from_ymd_opt(y, m, due_day.min(last))
}

/// Deterministic forecast ID for a card's open-bill forecast, keyed on
/// `(account_id, cycle_month)`.
fn card_open_bill_forecast_id(account_id: &str, month_ref: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"card-open-bill\x1f");
    hasher.update(account_id.as_bytes());
    hasher.update(b"\x1f");
    hasher.update(month_ref.as_bytes());
    let digest = hasher.finalize();
    format!(
        "cob-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    )
}

fn print_refresh_report(report: &RefreshReport) {
    println!("🔁 Forecast · refresh completo");
    println!(
        "  Parcelamentos  · cadeias ativas {} · templates {} · forecasts {}",
        report.installments.chains_active,
        report.installments.templates_upserted,
        report.installments.forecasts_upserted,
    );
    println!(
        "  Reconciliação  · varridos {} · realizados {} · ambíguos {}",
        report.reconcile.forecasts_scanned, report.reconcile.matched, report.reconcile.ambiguous,
    );
    println!(
        "  Recorrentes    · templates materializados {} · forecasts {}",
        report.templates_materialised, report.forecasts_materialised,
    );
    if report.templates_deduped > 0 {
        println!(
            "  Deduplicação   · {} template(s) duplicado(s) colapsado(s)",
            report.templates_deduped,
        );
    }
    if report.open_bill_forecasts > 0 {
        println!(
            "  Faturas abertas · {} forecast(s) atualizado(s)",
            report.open_bill_forecasts,
        );
    }
    if report.new_suggestions > 0 {
        println!(
            "  Sugestões      · {} novo(s) candidato(s) — rode `fin forecast suggest`",
            report.new_suggestions,
        );
    } else {
        println!("  Sugestões      · sem novos candidatos");
    }
}

/// One-line summary of a refresh run, used by the post-`sync pluggy` hook
/// where verbose multi-line output would drown out the sync's own report.
pub fn refresh_one_line(report: &RefreshReport) -> String {
    format!(
        "Forecast: {} realizado(s), {} forecast(s) novo(s), {} sugestão(ões) pendente(s).",
        report.reconcile.matched,
        report.installments.forecasts_upserted + report.forecasts_materialised,
        report.new_suggestions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_store() -> (tempfile::TempDir, AppConfig, Box<dyn FinanceStore>) {
        let dir = tempfile::tempdir().unwrap();
        let config = AppConfig {
            local_db_path: Some(dir.path().join("test.db")),
            actor_id: "test-actor".into(),
            ..AppConfig::default()
        };
        let store = open_store(&config).await.unwrap();
        run_migrations(store.as_ref(), &config).await.unwrap();
        (dir, config, store)
    }

    fn installment_tx(
        id: &str,
        date: NaiveDate,
        description: &str,
        amount: &str,
    ) -> TransactionRecord {
        TransactionRecord {
            transaction_id: id.to_string(),
            account_id: Some("card-1".to_string()),
            transaction_date: date,
            raw_description: description.to_string(),
            description: None,
            merchant_name: None,
            purpose: None,
            amount: Decimal::from_str_exact(amount).unwrap(),
            tx_type: "DEBIT".to_string(),
            category_id: None,
            category_source: "manual".to_string(),
            context: None,
            classifier_trace: None,
            payment_status: "posted".to_string(),
            source: "manual".to_string(),
            actor_id: "test-actor".to_string(),
            idempotency_key: id.to_string(),
            metadata_json: json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            enrichment_attempted_at: None,
            amount_cents: None,
        }
    }

    /// Regression: a plan whose description flips between the POS capture
    /// ("Pdv*Beagle 4/5") and the statement line ("Beagle - Parcela 3/5") must
    /// end the refresh with exactly ONE live template and ONE pending forecast
    /// per remaining parcela — the stale naming fork (and its forecasts) is
    /// retired, not left projecting the same parcela twice.
    #[tokio::test(flavor = "current_thread")]
    async fn refresh_retires_stale_naming_fork_of_renamed_plan() {
        let (_dir, config, store) = temp_store().await;
        let today = Utc::now().date_naive();
        let two_months_ago = shift_months(today, -2).unwrap();
        let last_month = shift_months(today, -1).unwrap();

        store
            .upsert_transactions(&[
                installment_tx("t-old", two_months_ago, "Beagle - Parcela 3/5", "-102.86"),
                installment_tx("t-new", last_month, "Pdv*Beagle 4/5", "-102.86"),
            ])
            .await
            .unwrap();

        // Stale fork left over from when the statement naming had its own
        // chain: ativo template + a pending materialised forecast.
        let stale_template = ForecastTemplateRecord {
            template_id: "installment-stale".into(),
            kind: "installment".into(),
            description: "Beagle - Parcela".into(),
            merchant_pattern: Some("Beagle - Parcela".into()),
            category_id: None,
            account_id: Some("card-1".into()),
            amount: Decimal::from_str_exact("-102.86").unwrap(),
            amount_lower: None,
            amount_upper: None,
            cadence: "monthly".into(),
            next_due_day: None,
            start_date: two_months_ago,
            end_date: None,
            remaining_count: Some(2),
            source: "detected".into(),
            confidence: None,
            status: "ativo".into(),
            metadata_json: json!({"installments_total": 5, "installments_current": 3}),
            actor_id: "test-actor".into(),
            idempotency_key: "tpl:installment-stale".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store
            .upsert_forecast_templates(&[stale_template])
            .await
            .unwrap();
        let stale_forecast = ForecastRecord {
            forecast_id: "installment-stale-5".into(),
            due_date: shift_months(today, 1),
            description: "Beagle - Parcela (5/5)".into(),
            amount: Decimal::from_str_exact("-102.86").unwrap(),
            category_id: None,
            account_id: Some("card-1".into()),
            status: "ativo".into(),
            recurrence: None,
            actor_id: "test-actor".into(),
            idempotency_key: "fc:installment-stale-5".into(),
            metadata_json: json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            template_id: Some("installment-stale".into()),
            realized_transaction_id: None,
            realized_at: None,
        };
        store.upsert_forecasts(&[stale_forecast]).await.unwrap();

        let report = refresh_installments(store.as_ref(), &config, 12)
            .await
            .unwrap();
        assert_eq!(report.templates_retired, 1, "{report:#?}");

        // Exactly one live installment template for the plan.
        let templates = store.list_forecast_templates(None, None).await.unwrap();
        let live: Vec<_> = templates
            .iter()
            .filter(|t| t.kind == "installment" && t.status == "ativo")
            .collect();
        assert_eq!(live.len(), 1, "{live:#?}");
        assert_eq!(live[0].description, "Pdv*Beagle");
        let stale = templates
            .iter()
            .find(|t| t.template_id == "installment-stale")
            .unwrap();
        assert_eq!(stale.status, "descartado");

        // The plan projects exactly once: the stale pending forecast is gone
        // and only the canonical chain's remaining parcela stays ativo.
        let forecasts = store.list_forecasts(None, None, None).await.unwrap();
        let stale_fc = forecasts
            .iter()
            .find(|f| f.forecast_id == "installment-stale-5")
            .unwrap();
        assert_eq!(stale_fc.status, "inativo");
        let pending: Vec<_> = forecasts
            .iter()
            .filter(|f| f.status == "ativo" && f.amount < Decimal::ZERO)
            .collect();
        assert_eq!(pending.len(), 1, "{pending:#?}");
    }

    #[test]
    fn shift_months_handles_year_boundary() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        assert_eq!(
            shift_months(d, 2),
            Some(NaiveDate::from_ymd_opt(2027, 1, 30).unwrap())
        );
        assert_eq!(
            shift_months(d, -12),
            Some(NaiveDate::from_ymd_opt(2025, 11, 30).unwrap())
        );
    }

    #[test]
    fn shift_months_clamps_day_in_shorter_target_month() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        assert_eq!(
            shift_months(d, 1),
            Some(NaiveDate::from_ymd_opt(2026, 2, 28).unwrap())
        );
    }

    #[test]
    fn chain_idempotency_key_is_stable_per_chain_identity() {
        let a = InstallmentChain {
            account_id: "card_a".into(),
            base_description: "Magazine Luiza".into(),
            total: 12,
            current: 3,
            installments: Vec::new(),
            first_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            projected_end: NaiveDate::from_ymd_opt(2026, 12, 15).unwrap(),
            remaining: 9,
            released_next_month: false,
            total_amount: Decimal::from(900),
        };
        let b = a.clone();
        assert_eq!(chain_idempotency_key(&a), chain_idempotency_key(&b));

        let mut c = a.clone();
        c.total = 24;
        assert_ne!(chain_idempotency_key(&a), chain_idempotency_key(&c));
    }

    #[test]
    fn chain_idempotency_key_is_stable_across_builds() {
        // Frozen value — if this assertion ever fails it means upgrading
        // some dependency changed the persisted template_id format, which
        // would orphan every forecast_template row already in production.
        let chain = InstallmentChain {
            account_id: "acc-frozen".into(),
            base_description: "Compra Parcelada".into(),
            total: 6,
            current: 1,
            installments: Vec::new(),
            first_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            projected_end: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            remaining: 5,
            released_next_month: false,
            total_amount: Decimal::from(600),
        };
        assert_eq!(chain_idempotency_key(&chain), "30e6c2ca1c2b5901");
    }

    #[test]
    fn reconciliation_match_amount_within_tolerance() {
        let forecast = Decimal::from(-100);
        // 95 → 5% off, accepted; 94 → 6% off, rejected.
        assert!(amount_matches(forecast, Decimal::from(-95)));
        assert!(!amount_matches(forecast, Decimal::from(-94)));
        // Sign mismatch always rejects.
        assert!(!amount_matches(forecast, Decimal::from(100)));
        // Zero forecast never matches (defensive).
        assert!(!amount_matches(Decimal::ZERO, Decimal::ZERO));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconcile_forecasts_preserves_predicted_amount_and_updates_actual() {
        let (_dir, config, store) = temp_store().await;
        let today = Utc::now().date_naive();
        let tx = installment_tx("tx-real", today, "Academia", "-95.00");
        let forecast = ForecastRecord {
            forecast_id: "f-manual".into(),
            due_date: Some(today),
            description: "Academia".into(),
            amount: Decimal::from_str("-100.00").unwrap(),
            category_id: Some("saude:academia".into()),
            account_id: Some("card-1".into()),
            status: "ativo".into(),
            recurrence: None,
            actor_id: "test-actor".into(),
            idempotency_key: "forecast-test".into(),
            metadata_json: json!({ "ui_role": "planned_transaction" }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            template_id: None,
            realized_transaction_id: None,
            realized_at: None,
        };
        store.upsert_transactions(&[tx]).await.unwrap();
        store.upsert_forecasts(&[forecast]).await.unwrap();

        let report = reconcile_forecasts(store.as_ref(), &config, 7)
            .await
            .unwrap();
        assert_eq!(report.matched, 1);

        let stored = store.get_forecast("f-manual").await.unwrap().unwrap();
        assert_eq!(stored.status, "realizado");
        assert_eq!(stored.amount, Decimal::from_str("-95.00").unwrap());
        assert_eq!(stored.realized_transaction_id.as_deref(), Some("tx-real"));
        assert_eq!(
            stored
                .metadata_json
                .get("predicted_amount")
                .and_then(|value| value.as_str()),
            Some("-100.00")
        );
        assert_eq!(
            stored
                .metadata_json
                .get("realized_amount")
                .and_then(|value| value.as_str()),
            Some("-95.00")
        );
    }

    #[test]
    fn recurring_candidate_idempotency_hash_is_stable_across_builds() {
        let c = RecurringCandidate {
            kind: "subscription".into(),
            account_id: "acc-frozen".into(),
            merchant_key: "netflix".into(),
            label: "Netflix".into(),
            category_id: None,
            median_amount: Decimal::ZERO,
            amount_lower: Decimal::ZERO,
            amount_upper: Decimal::ZERO,
            months_seen: 0,
            last_seen: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            typical_day_of_month: 1,
            coefficient_of_variation: 0.0,
            confidence: 0.0,
        };
        assert_eq!(c.idempotency_hash(), "f25d52649954089b");
    }

    fn sample_chain(account: &str, desc: &str, total: u32) -> InstallmentChain {
        InstallmentChain {
            account_id: account.into(),
            base_description: desc.into(),
            total,
            current: 1,
            installments: Vec::new(),
            first_date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            projected_end: NaiveDate::from_ymd_opt(2026, 6, 10).unwrap(),
            remaining: total.saturating_sub(1),
            released_next_month: false,
            total_amount: Decimal::from(600),
        }
    }

    fn sample_candidate(kind: &str, account: &str, merchant: &str) -> RecurringCandidate {
        RecurringCandidate {
            kind: kind.into(),
            account_id: account.into(),
            merchant_key: merchant.into(),
            label: merchant.into(),
            category_id: None,
            median_amount: Decimal::from(-50),
            amount_lower: Decimal::from(-55),
            amount_upper: Decimal::from(-45),
            months_seen: 3,
            last_seen: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            typical_day_of_month: 1,
            coefficient_of_variation: 0.0,
            confidence: 1.0,
        }
    }

    #[test]
    fn installment_natural_key_matches_built_template() {
        // The chain-side and template-side natural keys must agree, otherwise
        // id reuse and dedup would silently miss each other.
        let chain = sample_chain("card_a", "Magazine Luiza", 12);
        let (template, _) =
            build_template_and_instances(&chain, "actor", &TemplateDedupPlan::default()).unwrap();
        assert_eq!(
            installment_natural_key(&chain),
            template_natural_key(&template)
        );
    }

    #[test]
    fn candidate_natural_key_matches_built_template() {
        for kind in ["subscription", "fixed", "envelope"] {
            let cand = sample_candidate(kind, "chk_1", "Netflix");
            let template = template_from_candidate(
                &cand,
                format!("{kind}-x"),
                "actor",
                NaiveDate::from_ymd_opt(2026, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc(),
            );
            assert_eq!(
                cand.natural_key(),
                template_natural_key(&template),
                "kind {kind} natural keys diverge"
            );
        }
    }

    #[test]
    fn installment_natural_key_separates_distinct_totals() {
        // Two plans at the same merchant with different totals are distinct.
        assert_ne!(
            installment_natural_key(&sample_chain("card_a", "Loja X", 6)),
            installment_natural_key(&sample_chain("card_a", "Loja X", 12)),
        );
    }

    fn template_with(id: &str, status: &str, created_min: u32) -> ForecastTemplateRecord {
        let now = NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, created_min, 0)
            .unwrap()
            .and_utc();
        ForecastTemplateRecord {
            template_id: id.into(),
            kind: "subscription".into(),
            description: "Netflix".into(),
            merchant_pattern: Some("netflix".into()),
            category_id: None,
            account_id: Some("chk_1".into()),
            amount: Decimal::from(-50),
            amount_lower: None,
            amount_upper: None,
            cadence: "monthly".into(),
            next_due_day: Some(1),
            start_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            end_date: None,
            remaining_count: None,
            source: "detected".into(),
            confidence: Some(1.0),
            status: status.into(),
            metadata_json: json!({}),
            actor_id: "actor".into(),
            idempotency_key: format!("forecast-template-{id}"),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn plan_dedup_elects_oldest_and_demotes_younger() {
        // Same identity (merchant/account/kind), two ids → oldest is canonical.
        let existing = vec![
            template_with("sub-new", "ativo", 30),
            template_with("sub-old", "ativo", 10),
        ];
        let plan = plan_template_dedup(&existing);
        let nk = template_natural_key(&existing[0]);
        assert_eq!(
            plan.canonical_id.get(&nk).map(String::as_str),
            Some("sub-old")
        );
        assert!(plan.demote_ids.contains("sub-new"));
        assert!(!plan.demote_ids.contains("sub-old"));
        // A fresh derivation reuses the canonical id.
        assert_eq!(plan.resolve(&nk, "sub-fresh"), "sub-old");
    }

    #[test]
    fn plan_dedup_never_elects_or_redemotes_descartado() {
        let existing = vec![
            template_with("sub-dismissed", "descartado", 5),
            template_with("sub-live", "ativo", 20),
        ];
        let plan = plan_template_dedup(&existing);
        let nk = template_natural_key(&existing[1]);
        // The live row wins even though the dismissed one is older.
        assert_eq!(
            plan.canonical_id.get(&nk).map(String::as_str),
            Some("sub-live")
        );
        // The dismissed row is left alone (already terminal).
        assert!(!plan.demote_ids.contains("sub-dismissed"));
    }

    #[test]
    fn plan_dedup_noop_when_all_identities_unique() {
        let existing = vec![template_with("a", "ativo", 1), {
            let mut t = template_with("b", "ativo", 2);
            t.merchant_pattern = Some("spotify".into());
            t
        }];
        let plan = plan_template_dedup(&existing);
        assert!(plan.demote_ids.is_empty());
    }

    #[test]
    fn card_open_bill_due_date_returns_correct_day() {
        // Cycle closes in June, due on day 10 of June.
        assert_eq!(
            card_open_bill_due_date("2026-06", 10),
            NaiveDate::from_ymd_opt(2026, 6, 10),
        );
    }

    #[test]
    fn card_open_bill_due_date_clamps_to_month_end() {
        // February 2026 has 28 days; due_day=31 must clamp to 28.
        assert_eq!(
            card_open_bill_due_date("2026-02", 31),
            NaiveDate::from_ymd_opt(2026, 2, 28),
        );
    }

    #[test]
    fn card_open_bill_forecast_id_is_stable_across_builds() {
        // Frozen value — changing this breaks existing forecast rows in prod.
        assert_eq!(
            card_open_bill_forecast_id("shared_credit", "2026-06"),
            "cob-83b9b495c65a7647",
        );
    }

    #[test]
    fn card_open_bill_forecast_id_differs_by_cycle_month() {
        let a = card_open_bill_forecast_id("card_a", "2026-05");
        let b = card_open_bill_forecast_id("card_a", "2026-06");
        assert_ne!(a, b);
    }
}
