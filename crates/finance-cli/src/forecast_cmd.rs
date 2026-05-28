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
    group_into_chains, AccountRecord, AppConfig, AuditEvent, ForecastRecord,
    ForecastTemplateRecord, InstallmentChain,
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
        let events = templates
            .iter()
            .map(|t| audit_event_for_template(t, "upsert", &config.actor_id))
            .collect::<Result<Vec<_>>>()?;
        store.insert_audit_events(&events).await?;
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

use finance_core::TransactionRecord;
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
        if finance_core::parse_installment_description(&raw).is_some() {
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
        if finance_core::parse_installment_description(&raw).is_some() {
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
    let mut existing_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for t in existing_proposto
        .iter()
        .chain(existing_ativo.iter())
        .chain(existing_descartado.iter())
    {
        existing_keys.insert(t.template_id.clone());
    }

    let now = Utc::now();
    let mut new_proposals = Vec::new();
    for cand in &candidates {
        let template_id = format!("{}-{}", cand.kind, cand.idempotency_hash());
        if existing_keys.contains(&template_id) {
            continue;
        }
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
fn amount_matches(forecast_amount: Decimal, tx_amount: Decimal) -> bool {
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
    let mut existing_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for t in existing_proposto
        .iter()
        .chain(active.iter())
        .chain(existing_descartado.iter())
    {
        existing_keys.insert(t.template_id.clone());
    }

    let now = Utc::now();
    let mut new_proposals = Vec::new();
    for cand in &candidates {
        let template_id = format!("{}-{}", cand.kind, cand.idempotency_hash());
        if existing_keys.contains(&template_id) {
            continue;
        }
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
