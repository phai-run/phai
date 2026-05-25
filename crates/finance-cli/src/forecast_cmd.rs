//! `finance forecast …` — orchestrator for the forecast automation pipeline
//! described in ADR-0016.
//!
//! Layer 1 (installments) is the only layer implemented here. It detects
//! installment chains in the transaction history and materialises one
//! `forecast` row per remaining parcela, anchored on a single
//! `forecast_template` that lives for the life of the chain.

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate, Utc};
use finance_core::migrations::run_migrations;
use finance_core::storage::{open_store, FinanceStore};
use finance_core::{
    group_into_chains, AppConfig, ForecastRecord, ForecastTemplateRecord, InstallmentChain,
};
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::{enrich_description_from_metadata, load_config, ForecastRefreshInstallmentsArgs};

/// Summary returned by [`refresh_installments`] for CLI / agent display.
#[derive(Debug, Default)]
pub struct InstallmentsRefreshReport {
    pub chains_seen: usize,
    pub chains_active: usize,
    pub templates_upserted: usize,
    pub forecasts_upserted: usize,
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
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("🔁 Forecast · parcelamentos");
        println!("  Cadeias detectadas:  {}", report.chains_seen);
        println!("  Cadeias ativas:      {}", report.chains_active);
        println!("  Templates atualizados: {}", report.templates_upserted);
        println!("  Forecasts gravados:    {}", report.forecasts_upserted);
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

    let mut templates = Vec::new();
    let mut forecasts = Vec::new();

    for chain in &chains {
        if chain.remaining == 0 {
            continue;
        }
        report.chains_active += 1;

        let (template, instances) = build_template_and_instances(chain, &config.actor_id)?;
        templates.push(template);
        forecasts.extend(instances);
    }

    if !templates.is_empty() {
        report.templates_upserted = store.upsert_forecast_templates(&templates).await?;
    }
    if !forecasts.is_empty() {
        report.forecasts_upserted = store.upsert_forecasts(&forecasts).await?;
    }

    Ok(report)
}

/// Build the `forecast_template` row plus one forecast per remaining parcela
/// from a detected chain.
fn build_template_and_instances(
    chain: &InstallmentChain,
    actor_id: &str,
) -> Result<(ForecastTemplateRecord, Vec<ForecastRecord>)> {
    let now = Utc::now();
    let chain_key = chain_idempotency_key(chain);
    let template_id = format!("installment-{chain_key}");

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
fn chain_idempotency_key(chain: &InstallmentChain) -> String {
    let mut hasher = DefaultHasher::new();
    chain.account_id.hash(&mut hasher);
    chain.base_description.to_lowercase().hash(&mut hasher);
    chain.total.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
