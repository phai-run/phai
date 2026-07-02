//! Scenario projection engine (ADR-0037).
//!
//! A planning scenario is a set of typed deltas (`PlanChangeRecord`) applied
//! over the live forecast baseline at read time. This module is pure — no
//! I/O — so the CLI, the serve bridge, and tests all share one canonical
//! implementation.
//!
//! Orphan semantics: a change whose target is missing from the baseline
//! (realized, discarded, or removed by an earlier change in the same
//! scenario) becomes a no-op and is reported in `orphaned_change_ids`.
//! Reality wins over the plan; nothing is deleted automatically.

use crate::models::{ForecastRecord, ForecastTemplateRecord, PlanChangeKind, PlanChangeRecord};
use chrono::{Datelike, Months, NaiveDate};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// Day-of-month used for synthetic forecasts (`add_one_shot`,
/// `hypothetical_installment`). Mid-month keeps them clear of billing-cycle
/// boundaries on either edge.
const SYNTHETIC_DUE_DAY: u32 = 15;

#[derive(Debug, Clone)]
pub struct ScenarioProjection {
    /// The baseline transformed by the scenario: adjusted amounts, skipped
    /// rows removed, synthetic rows (ids `scn-...`) appended. Sorted by
    /// due date.
    pub virtual_forecasts: Vec<ForecastRecord>,
    /// Net signed delta vs the baseline per `YYYY-MM` month. Only months
    /// with a non-zero delta appear.
    pub monthly_delta: BTreeMap<String, Decimal>,
    /// Changes that could not be applied (target missing/inactive). Order
    /// follows the input changes.
    pub orphaned_change_ids: Vec<String>,
}

/// Parse `YYYY-MM` into the first day of that month.
pub fn parse_month(month: &str) -> Option<NaiveDate> {
    let (year, month) = month.split_once('-')?;
    NaiveDate::from_ymd_opt(year.parse().ok()?, month.parse().ok()?, 1)
}

fn month_ref(date: NaiveDate) -> String {
    format!("{:04}-{:02}", date.year(), date.month())
}

fn synthetic_due_date(month_start: NaiveDate) -> NaiveDate {
    month_start
        .with_day(SYNTHETIC_DUE_DAY)
        .unwrap_or(month_start)
}

/// Apply a scenario's changes to the forecast baseline within `horizon`
/// (inclusive on both ends).
///
/// `baseline` should be the active forecast instances in the horizon (the
/// caller typically passes `list_forecasts(Some("ativo"), from, until)`).
/// `templates` is consulted only to validate `end_template` targets.
/// Changes with status `aplicado` are ignored (already promoted); `ativo`
/// and `orfao` are both evaluated — orphanhood is recomputed on every read.
pub fn apply_scenario(
    baseline: &[ForecastRecord],
    templates: &[ForecastTemplateRecord],
    changes: &[PlanChangeRecord],
    horizon: (NaiveDate, NaiveDate),
) -> ScenarioProjection {
    let (from, until) = horizon;
    let mut virtual_forecasts: Vec<ForecastRecord> = baseline
        .iter()
        .filter(|f| {
            f.status == "ativo" && f.due_date.map(|d| d >= from && d <= until).unwrap_or(false)
        })
        .cloned()
        .collect();
    let mut monthly_delta: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut orphaned_change_ids = Vec::new();

    let mut bump = |deltas: &mut BTreeMap<String, Decimal>, month: String, amount: Decimal| {
        if amount.is_zero() {
            return;
        }
        *deltas.entry(month).or_insert_with(Decimal::default) += amount;
    };

    for change in changes {
        if change.status == "aplicado" {
            continue;
        }
        let Some(kind) = PlanChangeKind::parse(&change.kind) else {
            orphaned_change_ids.push(change.change_id.clone());
            continue;
        };
        match kind {
            PlanChangeKind::AddOneShot => {
                let (Some(month), Some(amount)) = (change.month.as_deref(), change.amount) else {
                    orphaned_change_ids.push(change.change_id.clone());
                    continue;
                };
                let Some(month_start) = parse_month(month) else {
                    orphaned_change_ids.push(change.change_id.clone());
                    continue;
                };
                let due_date = synthetic_due_date(month_start);
                if due_date < from || due_date > until {
                    // Outside the horizon: valid change, no effect here.
                    continue;
                }
                virtual_forecasts.push(synthetic_forecast(
                    format!("scn-{}", change.change_id),
                    due_date,
                    amount,
                    change,
                ));
                bump(&mut monthly_delta, month.to_string(), amount);
            }
            PlanChangeKind::AdjustAmount => {
                let (Some(target), Some(new_amount)) =
                    (change.target_forecast_id.as_deref(), change.amount)
                else {
                    orphaned_change_ids.push(change.change_id.clone());
                    continue;
                };
                match virtual_forecasts
                    .iter_mut()
                    .find(|f| f.forecast_id == target)
                {
                    Some(forecast) => {
                        let month = forecast.due_date.map(month_ref).unwrap_or_default();
                        let delta = new_amount - forecast.amount;
                        forecast.amount = new_amount;
                        bump(&mut monthly_delta, month, delta);
                    }
                    None => orphaned_change_ids.push(change.change_id.clone()),
                }
            }
            PlanChangeKind::SkipForecast => {
                let Some(target) = change.target_forecast_id.as_deref() else {
                    orphaned_change_ids.push(change.change_id.clone());
                    continue;
                };
                match virtual_forecasts
                    .iter()
                    .position(|f| f.forecast_id == target)
                {
                    Some(index) => {
                        let removed = virtual_forecasts.remove(index);
                        let month = removed.due_date.map(month_ref).unwrap_or_default();
                        bump(&mut monthly_delta, month, -removed.amount);
                    }
                    None => orphaned_change_ids.push(change.change_id.clone()),
                }
            }
            PlanChangeKind::EndTemplate => {
                let (Some(target), Some(effective_from)) = (
                    change.target_template_id.as_deref(),
                    change.effective_from.as_deref(),
                ) else {
                    orphaned_change_ids.push(change.change_id.clone());
                    continue;
                };
                let Some(cutoff) = parse_month(effective_from) else {
                    orphaned_change_ids.push(change.change_id.clone());
                    continue;
                };
                let template_alive = templates
                    .iter()
                    .find(|t| t.template_id == target)
                    .map(|t| {
                        t.status != "descartado"
                            && t.end_date.map(|end| end >= cutoff).unwrap_or(true)
                    })
                    .unwrap_or(false);
                if !template_alive {
                    orphaned_change_ids.push(change.change_id.clone());
                    continue;
                }
                let mut kept = Vec::with_capacity(virtual_forecasts.len());
                for forecast in virtual_forecasts.drain(..) {
                    let ends = forecast.template_id.as_deref() == Some(target)
                        && forecast.due_date.map(|d| d >= cutoff).unwrap_or(false);
                    if ends {
                        let month = forecast.due_date.map(month_ref).unwrap_or_default();
                        bump(&mut monthly_delta, month, -forecast.amount);
                    } else {
                        kept.push(forecast);
                    }
                }
                virtual_forecasts = kept;
            }
            PlanChangeKind::HypotheticalInstallment => {
                let (Some(effective_from), Some(amount), Some(count)) = (
                    change.effective_from.as_deref(),
                    change.amount,
                    change.months_count,
                ) else {
                    orphaned_change_ids.push(change.change_id.clone());
                    continue;
                };
                let Some(first_month) = parse_month(effective_from) else {
                    orphaned_change_ids.push(change.change_id.clone());
                    continue;
                };
                for n in 0..count.max(0) as u32 {
                    let Some(month_start) = first_month.checked_add_months(Months::new(n)) else {
                        break;
                    };
                    let due_date = synthetic_due_date(month_start);
                    if due_date < from || due_date > until {
                        continue;
                    }
                    virtual_forecasts.push(synthetic_forecast(
                        format!("scn-{}-{:03}", change.change_id, n + 1),
                        due_date,
                        amount,
                        change,
                    ));
                    bump(&mut monthly_delta, month_ref(due_date), amount);
                }
            }
        }
    }

    monthly_delta.retain(|_, v| !v.is_zero());
    virtual_forecasts.sort_by_key(|f| f.due_date);
    ScenarioProjection {
        virtual_forecasts,
        monthly_delta,
        orphaned_change_ids,
    }
}

/// Per-month net difference `a - b` between two projections (union of
/// months). Diffing against an empty projection yields `a`'s deltas.
pub fn diff_scenarios(a: &ScenarioProjection, b: &ScenarioProjection) -> BTreeMap<String, Decimal> {
    let mut out = a.monthly_delta.clone();
    for (month, amount) in &b.monthly_delta {
        *out.entry(month.clone()).or_insert_with(Decimal::default) -= *amount;
    }
    out.retain(|_, v| !v.is_zero());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn horizon() -> (NaiveDate, NaiveDate) {
        (
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        )
    }

    fn forecast(
        id: &str,
        due: (i32, u32, u32),
        amount: i64,
        template: Option<&str>,
    ) -> ForecastRecord {
        let now = Utc::now();
        ForecastRecord {
            forecast_id: id.to_string(),
            due_date: NaiveDate::from_ymd_opt(due.0, due.1, due.2),
            description: "assinatura streaming".to_string(),
            amount: Decimal::new(amount, 2),
            category_id: Some("assinaturas".to_string()),
            account_id: Some("acc-1".to_string()),
            status: "ativo".to_string(),
            recurrence: None,
            actor_id: "test".to_string(),
            idempotency_key: id.to_string(),
            metadata_json: json!({}),
            created_at: now,
            updated_at: now,
            template_id: template.map(str::to_string),
            realized_transaction_id: None,
            realized_at: None,
        }
    }

    fn template(id: &str, status: &str) -> ForecastTemplateRecord {
        let now = Utc::now();
        ForecastTemplateRecord {
            template_id: id.to_string(),
            kind: "subscription".to_string(),
            description: "assinatura streaming".to_string(),
            merchant_pattern: Some("streaming".to_string()),
            category_id: Some("assinaturas".to_string()),
            account_id: Some("acc-1".to_string()),
            amount: Decimal::new(-5000, 2),
            amount_lower: None,
            amount_upper: None,
            cadence: "monthly".to_string(),
            next_due_day: Some(10),
            start_date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            end_date: None,
            remaining_count: None,
            source: "detected".to_string(),
            confidence: Some(0.9),
            status: status.to_string(),
            metadata_json: json!({}),
            actor_id: "test".to_string(),
            idempotency_key: format!("tpl-{id}"),
            created_at: now,
            updated_at: now,
        }
    }

    fn change(id: &str, kind: PlanChangeKind) -> PlanChangeRecord {
        let now = Utc::now();
        PlanChangeRecord {
            change_id: id.to_string(),
            scenario_id: "scn-1".to_string(),
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
            actor_id: "test".to_string(),
            idempotency_key: id.to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn add_one_shot_appends_synthetic_forecast_and_delta() {
        let mut c = change("c1", PlanChangeKind::AddOneShot);
        c.month = Some("2026-09".to_string());
        c.amount = Some(Decimal::new(-200000, 2));
        c.description = Some("viagem".to_string());

        let projection = apply_scenario(&[], &[], &[c], horizon());
        assert_eq!(projection.virtual_forecasts.len(), 1);
        assert_eq!(
            projection.virtual_forecasts[0].due_date,
            NaiveDate::from_ymd_opt(2026, 9, 15)
        );
        assert_eq!(
            projection.monthly_delta.get("2026-09"),
            Some(&Decimal::new(-200000, 2))
        );
        assert!(projection.orphaned_change_ids.is_empty());
    }

    #[test]
    fn adjust_amount_overrides_value_and_tracks_delta() {
        let baseline = vec![forecast("f1", (2026, 8, 10), -120000, None)];
        let mut c = change("c1", PlanChangeKind::AdjustAmount);
        c.target_forecast_id = Some("f1".to_string());
        c.amount = Some(Decimal::new(-80000, 2));

        let projection = apply_scenario(&baseline, &[], &[c], horizon());
        assert_eq!(
            projection.virtual_forecasts[0].amount,
            Decimal::new(-80000, 2)
        );
        // -800 - (-1200) = +400 freed in August.
        assert_eq!(
            projection.monthly_delta.get("2026-08"),
            Some(&Decimal::new(40000, 2))
        );
    }

    #[test]
    fn skip_forecast_removes_row_and_reverses_amount() {
        let baseline = vec![forecast("f1", (2026, 9, 5), -30000, None)];
        let mut c = change("c1", PlanChangeKind::SkipForecast);
        c.target_forecast_id = Some("f1".to_string());

        let projection = apply_scenario(&baseline, &[], &[c], horizon());
        assert!(projection.virtual_forecasts.is_empty());
        assert_eq!(
            projection.monthly_delta.get("2026-09"),
            Some(&Decimal::new(30000, 2))
        );
    }

    #[test]
    fn end_template_drops_materialized_forecasts_from_cutoff() {
        let baseline = vec![
            forecast("tpl-x-202607", (2026, 7, 10), -5000, Some("tpl-x")),
            forecast("tpl-x-202608", (2026, 8, 10), -5000, Some("tpl-x")),
            forecast("tpl-x-202609", (2026, 9, 10), -5000, Some("tpl-x")),
        ];
        let templates = vec![template("tpl-x", "ativo")];
        let mut c = change("c1", PlanChangeKind::EndTemplate);
        c.target_template_id = Some("tpl-x".to_string());
        c.effective_from = Some("2026-08".to_string());

        let projection = apply_scenario(&baseline, &templates, &[c], horizon());
        assert_eq!(projection.virtual_forecasts.len(), 1);
        assert_eq!(projection.virtual_forecasts[0].forecast_id, "tpl-x-202607");
        assert_eq!(
            projection.monthly_delta.get("2026-08"),
            Some(&Decimal::new(5000, 2))
        );
        assert_eq!(
            projection.monthly_delta.get("2026-09"),
            Some(&Decimal::new(5000, 2))
        );
    }

    #[test]
    fn hypothetical_installment_materializes_parcels_within_horizon() {
        let mut c = change("c1", PlanChangeKind::HypotheticalInstallment);
        c.effective_from = Some("2026-10".to_string());
        c.amount = Some(Decimal::new(-30000, 2));
        c.months_count = Some(10);
        c.description = Some("parcela sofá".to_string());

        let projection = apply_scenario(&[], &[], &[c], horizon());
        // Horizon ends 2026-12 → only 3 of the 10 parcels are visible.
        assert_eq!(projection.virtual_forecasts.len(), 3);
        assert_eq!(
            projection.monthly_delta.get("2026-10"),
            Some(&Decimal::new(-30000, 2))
        );
        assert_eq!(
            projection.monthly_delta.get("2026-12"),
            Some(&Decimal::new(-30000, 2))
        );
        assert!(projection.orphaned_change_ids.is_empty());
    }

    #[test]
    fn changes_with_missing_targets_are_orphaned_no_ops() {
        let mut adjust = change("c-adjust", PlanChangeKind::AdjustAmount);
        adjust.target_forecast_id = Some("missing".to_string());
        adjust.amount = Some(Decimal::new(-100, 2));
        let mut skip = change("c-skip", PlanChangeKind::SkipForecast);
        skip.target_forecast_id = Some("missing".to_string());
        let mut end = change("c-end", PlanChangeKind::EndTemplate);
        end.target_template_id = Some("missing".to_string());
        end.effective_from = Some("2026-08".to_string());

        let projection = apply_scenario(&[], &[], &[adjust, skip, end], horizon());
        assert!(projection.virtual_forecasts.is_empty());
        assert!(projection.monthly_delta.is_empty());
        assert_eq!(
            projection.orphaned_change_ids,
            vec!["c-adjust", "c-skip", "c-end"]
        );
    }

    #[test]
    fn end_template_on_discarded_template_is_orphaned() {
        let baseline = vec![forecast(
            "tpl-x-202608",
            (2026, 8, 10),
            -5000,
            Some("tpl-x"),
        )];
        let templates = vec![template("tpl-x", "descartado")];
        let mut c = change("c1", PlanChangeKind::EndTemplate);
        c.target_template_id = Some("tpl-x".to_string());
        c.effective_from = Some("2026-08".to_string());

        let projection = apply_scenario(&baseline, &templates, &[c], horizon());
        assert_eq!(projection.orphaned_change_ids, vec!["c1"]);
        assert_eq!(projection.virtual_forecasts.len(), 1);
    }

    #[test]
    fn adjust_after_end_template_on_same_row_is_orphaned() {
        let baseline = vec![forecast(
            "tpl-x-202609",
            (2026, 9, 10),
            -5000,
            Some("tpl-x"),
        )];
        let templates = vec![template("tpl-x", "ativo")];
        let mut end = change("c-end", PlanChangeKind::EndTemplate);
        end.target_template_id = Some("tpl-x".to_string());
        end.effective_from = Some("2026-09".to_string());
        let mut adjust = change("c-adjust", PlanChangeKind::AdjustAmount);
        adjust.target_forecast_id = Some("tpl-x-202609".to_string());
        adjust.amount = Some(Decimal::new(-1000, 2));

        let projection = apply_scenario(&baseline, &templates, &[end, adjust], horizon());
        assert_eq!(projection.orphaned_change_ids, vec!["c-adjust"]);
        // Delta reflects only the end_template.
        assert_eq!(
            projection.monthly_delta.get("2026-09"),
            Some(&Decimal::new(5000, 2))
        );
    }

    #[test]
    fn realized_baseline_rows_are_not_projectable_targets() {
        let mut realized = forecast("f1", (2026, 8, 10), -5000, None);
        realized.status = "realizado".to_string();
        let mut c = change("c1", PlanChangeKind::SkipForecast);
        c.target_forecast_id = Some("f1".to_string());

        let projection = apply_scenario(&[realized], &[], &[c], horizon());
        assert_eq!(projection.orphaned_change_ids, vec!["c1"]);
        assert!(projection.monthly_delta.is_empty());
    }

    #[test]
    fn applied_changes_are_ignored() {
        let mut c = change("c1", PlanChangeKind::AddOneShot);
        c.month = Some("2026-09".to_string());
        c.amount = Some(Decimal::new(-100, 2));
        c.status = "aplicado".to_string();

        let projection = apply_scenario(&[], &[], &[c], horizon());
        assert!(projection.virtual_forecasts.is_empty());
        assert!(projection.monthly_delta.is_empty());
        assert!(projection.orphaned_change_ids.is_empty());
    }

    #[test]
    fn diff_scenarios_subtracts_per_month() {
        let mut a = change("a1", PlanChangeKind::AddOneShot);
        a.month = Some("2026-09".to_string());
        a.amount = Some(Decimal::new(-100000, 2));
        let mut b = change("b1", PlanChangeKind::AddOneShot);
        b.month = Some("2026-09".to_string());
        b.amount = Some(Decimal::new(-40000, 2));

        let pa = apply_scenario(&[], &[], &[a], horizon());
        let pb = apply_scenario(&[], &[], &[b], horizon());
        let diff = diff_scenarios(&pa, &pb);
        assert_eq!(diff.get("2026-09"), Some(&Decimal::new(-60000, 2)));
    }
}

fn synthetic_forecast(
    forecast_id: String,
    due_date: NaiveDate,
    amount: Decimal,
    change: &PlanChangeRecord,
) -> ForecastRecord {
    ForecastRecord {
        forecast_id,
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
        actor_id: change.actor_id.clone(),
        idempotency_key: format!("scenario-{}-{}", change.scenario_id, change.change_id),
        metadata_json: serde_json::json!({ "scenario_id": change.scenario_id }),
        created_at: change.created_at,
        updated_at: change.updated_at,
        template_id: None,
        realized_transaction_id: None,
        realized_at: None,
    }
}
