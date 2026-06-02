//! `finance report cashflow-chart` — renders an SVG (and optional ASCII
//! sparkline) of household cashflow and real checking-account balance.
//!
//! Layout: stacked bars per month — solid bottom = realized entradas/saídas,
//! hatched top = forecast remaining (what's still expected to come in or go
//! out this month). The saldo line is solid for the realized portion and
//! continues dashed through projected closings into the future. The window
//! always includes `--months` back from today and, when `--forecast` is on,
//! `--months-ahead` future months so the dashed projection has room to land.
//!
//! Pure data → string transforms live in [`render_svg`] and
//! [`render_sparkline`] so they're easy to unit test without a live store.

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use phai_core::migrations::run_migrations;
use phai_core::models::ForecastRecord;
use phai_core::storage::{open_store, FinanceStore};
use rust_decimal::Decimal;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::human_format;
use crate::{load_config, month_ref_for, parse_month_ref, shift_month, CashflowChartArgs};

/// One month's slice of data for the chart.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct MonthDatum {
    pub label: String, // display label, e.g. "mai/26"
    /// Stable month key in `YYYY-MM` (matches `TransactionRecord` month and the
    /// web client's selection key — never use the display `label` for matching).
    pub month: String,
    /// Realized inflows (what already hit the checking accounts). Zero for
    /// purely-future months.
    pub inflows: Decimal,
    /// Realized outflows (positive magnitude).
    pub outflows: Decimal,
    /// Realized closing balance for the month. `None` for future months and
    /// for past months without snapshot coverage.
    pub closing_balance: Option<Decimal>,
    /// Forecast still expected this month on top of what already came in.
    /// `None` when `--forecast` was off; `Some(0)` when forecast was fully
    /// realized or the realized inflows already exceeded the forecast.
    pub forecast_inflows_remaining: Option<Decimal>,
    /// Forecast outflow magnitude still expected on top of realized outflows.
    pub forecast_outflows_remaining: Option<Decimal>,
    /// Projected closing balance for this month: previous projection +
    /// realized net + forecast net remaining. `None` until forecast mode is
    /// on AND every preceding month had usable balance data.
    pub projected_closing_balance: Option<Decimal>,
    /// True when this month sits entirely in the future (start-of-month >
    /// today). Drives "future-only" rendering treatment.
    pub is_future: bool,
}

/// Bundle returned by the data collection pass — what the renderers consume.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChartData {
    pub months: Vec<MonthDatum>,
    /// Opening balance at the left edge of the window — the line anchor.
    pub initial_balance: Option<Decimal>,
    pub with_forecast: bool,
    /// How many months in `months` are in the past or current (the rest are
    /// purely future). Used by the renderer to split solid vs dashed
    /// segments of the saldo line.
    pub realized_count: usize,
    /// Optional what-if overlay: per-month projected saldo *with* a
    /// hypothetical recurring commitment added on top of the forecast
    /// baseline. `None` when `--scenario-amount` was not passed. When
    /// present, has exactly one entry per `months` and lines up by index.
    pub scenario: Option<ScenarioOverlay>,
}

/// What-if scenario overlay (see ADR-0016 Layer 5).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ScenarioOverlay {
    pub label: String,
    /// Signed monthly amount of the hypothetical commitment.
    pub amount: Decimal,
    pub start_month: NaiveDate,
    pub months: u32,
    /// Per-month projected balance with the scenario applied. Same length
    /// and ordering as `ChartData.months`.
    pub projected_balance: Vec<Option<Decimal>>,
}

pub(crate) async fn report_cashflow_chart(args: CashflowChartArgs) -> Result<()> {
    let months_back = args.months.clamp(2, 24);
    // When --forecast is on and the user didn't ask for a specific ahead
    // window, default to 6 future months so the projection has room to land.
    let months_ahead = match (args.months_ahead, args.forecast) {
        (Some(n), _) => n.min(24),
        (None, true) => 6,
        (None, false) => 0,
    };
    if args.no_svg && !args.text {
        anyhow::bail!("--no-svg sem --text não produz nada: passe --text ou remova --no-svg");
    }
    if months_ahead > 0 && !args.forecast {
        anyhow::bail!("--months-ahead requer --forecast (sem forecast não há nada para projetar)");
    }

    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let mut chart =
        build_chart_data(store.as_ref(), months_back, months_ahead, args.forecast).await?;
    let scenario = build_scenario_overlay(&args, &chart.months, chart.realized_count)?;
    if scenario.is_some() && !args.forecast {
        anyhow::bail!(
            "--scenario-amount requer --forecast (a sobreposição parte do saldo projetado)"
        );
    }
    chart.scenario = scenario;

    if !args.no_svg {
        let output_path = args
            .output
            .clone()
            .unwrap_or_else(|| PathBuf::from("finance-cashflow.svg"));
        let svg = render_svg(&chart);
        write_svg(&output_path, &svg)?;
        println!("📊 SVG gravado em {}", output_path.display());
    }

    if args.text {
        print!("{}", render_sparkline(&chart));
    }

    Ok(())
}

/// Build [`ChartData`] for a window of `months_back` (past+current) +
/// `months_ahead` (future) months. When `with_forecast` is true, each
/// month includes the forecast-remaining portion (hatched bar tops) and
/// projected closing balances (dashed saldo line).
///
/// Realized bars come from [`FinanceStore::cashflow_reportable`] (the
/// `v_cashflow` household basis: every reportable transaction, card swipes
/// bucketed into their cash month, internal categories excluded) so the chart
/// agrees with the month-detail reports. The saldo line uses real
/// checking-account snapshots via [`FinanceStore::checking_balance_at`] for
/// past/current months when available, with the previous accumulated-net
/// behaviour as a fallback for stores without complete snapshot coverage.
pub(crate) async fn build_chart_data(
    store: &dyn FinanceStore,
    months_back: usize,
    months_ahead: usize,
    with_forecast: bool,
) -> Result<ChartData> {
    let today = chrono::Local::now().date_naive();
    let current_month = first_of_month(today)?;

    // Window: [current - (months_back - 1), current + months_ahead], oldest first.
    let total = months_back + months_ahead;
    let mut window: Vec<NaiveDate> = Vec::with_capacity(total);
    for i in 0..total {
        let delta = i as i32 - (months_back as i32 - 1);
        window.push(shift_month(current_month, delta)?);
    }

    // Accrual cashflow across all reportable accounts, keyed by `YYYY-MM`.
    let flows = store.cashflow_reportable().await?;
    let by_month: HashMap<&str, _> = flows.iter().map(|r| (r.month_ref.as_str(), r)).collect();

    // Fallback saldo anchor from the net of everything *before* the window.
    // Used only when snapshot coverage is incomplete.
    let first_ref = month_ref_for(window[0]);
    let fallback_initial_balance: Decimal = flows
        .iter()
        .filter(|r| r.month_ref < first_ref)
        .map(|r| r.net)
        .sum();
    let initial_anchor_date = window[0].pred_opt();
    let initial_balance = match initial_anchor_date {
        Some(anchor) => store
            .checking_balance_at(anchor)
            .await?
            .map(|b| b.balance)
            .unwrap_or(fallback_initial_balance),
        None => fallback_initial_balance,
    };

    let realized_count = months_back; // by construction
    let mut data: Vec<MonthDatum> = Vec::with_capacity(total);
    // Two running balances: `running` advances only on realized net (the
    // solid saldo line); `running_proj` also absorbs the forecast remainder
    // (the dashed projection). They coincide through the past and diverge
    // from the current month onward when forecasts exist.
    let mut running = fallback_initial_balance;
    let mut running_proj = initial_balance;

    for (i, month_start) in window.iter().enumerate() {
        let month_ref = month_ref_for(*month_start);
        parse_month_ref(&month_ref)?;
        let is_future = i >= realized_count;

        // Past + current: realized cashflow from v_cashflow. Future months
        // have no realized rows yet → zeros; their bars are purely forecast.
        let (inflows, outflows, net_realized) = if is_future {
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO)
        } else {
            match by_month.get(month_ref.as_str()) {
                Some(r) => (r.income, r.expenses, r.net),
                None => (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO),
            }
        };

        let (fc_in_remaining, fc_out_remaining) = if with_forecast {
            let last_day = last_day_of_month(*month_start)?;
            // "Remaining" = forecasts whose due_date hasn't passed yet.
            // For past months this returns nothing (all due_dates < today).
            // For current month it returns only the future-of-today portion.
            // For future months it returns the full month's forecasts.
            // This semantic avoids double-counting items that already
            // materialized (e.g. mid-month salary already received) while
            // still surfacing the end-of-month installment that hasn't.
            let lower = today.succ_opt().unwrap_or(today).max(*month_start);
            if lower > last_day {
                // Wholly-past month: every due date already elapsed.
                (Some(Decimal::ZERO), Some(Decimal::ZERO))
            } else {
                // Group remaining outflows by parent category so a budget
                // envelope ("Moradia", "Alimentação") can be netted against
                // spend already realized in that category; income is netted at
                // the month total (salary dominates, no per-category budget).
                let forecasts = store.upcoming_forecasts(lower, last_day).await?;
                let mut fi_total = Decimal::ZERO;
                let mut fo_by_cat: HashMap<String, Decimal> = HashMap::new();
                for f in &forecasts {
                    if !forecast_counts_in_chart_projection(f) {
                        continue;
                    }
                    if f.amount > Decimal::ZERO {
                        fi_total += f.amount;
                    } else {
                        *fo_by_cat
                            .entry(parent_category(f.category_id.as_deref()))
                            .or_default() += f.amount.abs();
                    }
                }
                if is_future {
                    // No realized rows yet — the full forecast projects through.
                    (Some(fi_total), Some(fo_by_cat.values().copied().sum()))
                } else {
                    // Current month: net the remaining budget against realized
                    // so realized spend isn't double-counted (ADR-0025).
                    let realized_by_cat = realized_outflow_by_parent(store, &month_ref).await?;
                    let fo_rem = envelope_remaining(&fo_by_cat, &realized_by_cat);
                    let fi_rem = (fi_total - inflows).max(Decimal::ZERO);
                    (Some(fi_rem), Some(fo_rem))
                }
            }
        } else {
            (None, None)
        };

        // Advance the derived fallback balance. The displayed realized saldo
        // prefers real checking snapshots; when they are unavailable, it falls
        // back to the accumulated flow-derived balance so card-only/demo stores
        // still render a continuous line.
        let net_remaining =
            fc_in_remaining.unwrap_or(Decimal::ZERO) - fc_out_remaining.unwrap_or(Decimal::ZERO);
        running += net_realized;
        let realized_anchor_date = if is_future {
            None
        } else if *month_start == current_month {
            Some(today)
        } else {
            Some(last_day_of_month(*month_start)?)
        };
        let realized_balance = match realized_anchor_date {
            Some(target) => store.checking_balance_at(target).await?.map(|b| b.balance),
            None => None,
        };

        let closing_balance = if is_future {
            None
        } else {
            Some(realized_balance.unwrap_or(running))
        };
        let projected = if with_forecast {
            if is_future {
                running_proj += net_remaining;
                Some(running_proj)
            } else {
                running_proj = closing_balance.unwrap_or(running);
                if *month_start == current_month {
                    running_proj += net_remaining;
                }
                Some(running_proj)
            }
        } else {
            closing_balance
        };

        data.push(MonthDatum {
            label: short_month_label(&month_ref),
            month: month_ref.clone(),
            inflows,
            outflows,
            closing_balance,
            forecast_inflows_remaining: fc_in_remaining,
            forecast_outflows_remaining: fc_out_remaining,
            projected_closing_balance: projected,
            is_future,
        });
    }

    Ok(ChartData {
        months: data,
        initial_balance: Some(initial_balance),
        with_forecast,
        realized_count,
        scenario: None,
    })
}

fn forecast_counts_in_chart_projection(forecast: &ForecastRecord) -> bool {
    if forecast.recurrence.as_deref() == Some("card-cycle") {
        return false;
    }
    forecast
        .metadata_json
        .get("source")
        .and_then(|v| v.as_str())
        != Some("card-open-bill")
}

/// Roll a (sub)category id up to its parent: `"moradia:servicos"` → `"moradia"`.
/// Empty/`None` maps to the `"sem-categoria"` bucket `v_monthly_spend` uses, so
/// uncategorised forecasts and realized rows net against each other.
fn parent_category(category_id: Option<&str>) -> String {
    match category_id {
        None | Some("") => "sem-categoria".to_string(),
        Some(c) => c.split(':').next().unwrap_or(c).to_string(),
    }
}

/// Realized outflow magnitude for `month_ref`, grouped by parent category. Reads
/// the cash-basis `v_monthly_spend` via [`FinanceStore::monthly_spend`] (card
/// swipes already bucketed into their bill's cash month, internal categories
/// excluded), so it lines up with the chart's realized bar.
async fn realized_outflow_by_parent(
    store: &dyn FinanceStore,
    month_ref: &str,
) -> Result<HashMap<String, Decimal>> {
    let rows = store.monthly_spend(Some(month_ref)).await?;
    let mut out: HashMap<String, Decimal> = HashMap::new();
    for r in rows {
        *out.entry(parent_category(Some(&r.category_id)))
            .or_default() += r.expenses;
    }
    Ok(out)
}

/// Envelope netting for the current month. A monthly budget forecast for a
/// category (e.g. "Alimentação" = R$7.500) only contributes the portion NOT yet
/// realized in that category, so realized spend and its budget envelope don't
/// double-count — the bar shows `max(realized, budget)` per category instead of
/// `realized + budget`. Over-spent categories contribute zero remaining (their
/// realized magnitude is already in the solid bar). Both maps are
/// parent-category → positive magnitude.
fn envelope_remaining(
    forecast_by_cat: &HashMap<String, Decimal>,
    realized_by_cat: &HashMap<String, Decimal>,
) -> Decimal {
    forecast_by_cat
        .iter()
        .map(|(cat, fc)| {
            let realized = realized_by_cat.get(cat).copied().unwrap_or(Decimal::ZERO);
            (*fc - realized).max(Decimal::ZERO)
        })
        .sum()
}

fn write_svg(path: &Path, body: &str) -> Result<()> {
    std::fs::write(path, body).with_context(|| format!("falha ao escrever {}", path.display()))
}

/// Build the optional scenario overlay from CLI args + the already-computed
/// per-month baseline projection. Returns `Ok(None)` when no scenario was
/// requested. The overlay walks the same window as `data`, starting from the
/// month's baseline projection and adding the scenario amount in every month
/// inside `[start_month, start_month + months - 1]`.
fn build_scenario_overlay(
    args: &CashflowChartArgs,
    data: &[MonthDatum],
    realized_count: usize,
) -> Result<Option<ScenarioOverlay>> {
    let Some(amount_str) = args.scenario_amount.as_deref() else {
        return Ok(None);
    };
    let amount = Decimal::from_str(amount_str.trim())
        .with_context(|| format!("--scenario-amount inválido: {amount_str}"))?;
    let label = args
        .scenario_description
        .clone()
        .unwrap_or_else(|| "cenário".to_string());

    // Default start = first future month in the chart (which is
    // `realized_count` — the first index that's `is_future`). When all months
    // are realised (no future), fall back to the first month of the window.
    let default_start_idx = realized_count.min(data.len().saturating_sub(1));
    let default_start = month_label_to_first_day(&data[default_start_idx].label)?;
    let start_month = match &args.scenario_start {
        Some(s) => parse_scenario_start(s)?,
        None => default_start,
    };
    // Default months: the remaining horizon from start_month to end of chart.
    let chart_end = month_label_to_first_day(&data[data.len() - 1].label)?;
    let default_months = months_between_inclusive(start_month, chart_end).max(1);
    let months = args.scenario_months.unwrap_or(default_months);

    // Per-month walk: each entry copies the baseline `projected_closing_balance`
    // and adds `amount` for every month that's within the scenario window.
    let mut projected_balance: Vec<Option<Decimal>> = Vec::with_capacity(data.len());
    let mut applied_so_far = Decimal::ZERO;
    for m in data {
        let month_first = month_label_to_first_day(&m.label)?;
        if month_within(month_first, start_month, months) {
            applied_so_far += amount;
        }
        projected_balance.push(m.projected_closing_balance.map(|b| b + applied_so_far));
    }

    Ok(Some(ScenarioOverlay {
        label,
        amount,
        start_month,
        months,
        projected_balance,
    }))
}

/// Convert the chart's "mai/26" short label back to a NaiveDate at the 1st
/// of the month. Returns an error for labels that don't parse — defensive
/// only; renderers produce these labels themselves so the inverse should
/// always succeed.
fn month_label_to_first_day(label: &str) -> Result<NaiveDate> {
    let mut parts = label.split('/');
    let month_str = parts.next().context("invalid month label")?;
    let year_str = parts.next().context("invalid month label")?;
    let month = match month_str {
        "jan" => 1,
        "fev" => 2,
        "mar" => 3,
        "abr" => 4,
        "mai" => 5,
        "jun" => 6,
        "jul" => 7,
        "ago" => 8,
        "set" => 9,
        "out" => 10,
        "nov" => 11,
        "dez" => 12,
        other => anyhow::bail!("unknown month abbrev: {other}"),
    };
    let yy: i32 = year_str.parse().context("invalid year in month label")?;
    let year = 2000 + yy;
    NaiveDate::from_ymd_opt(year, month, 1).context("invalid date for month label")
}

fn parse_scenario_start(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d")
        .with_context(|| format!("--scenario-start inválido: {value} (esperado YYYY-MM)"))
}

fn month_within(month: NaiveDate, start: NaiveDate, months: u32) -> bool {
    if month < start {
        return false;
    }
    let span = months as i32 - 1;
    let mut end_year = start.year();
    let mut end_month = start.month() as i32 + span;
    while end_month > 12 {
        end_year += 1;
        end_month -= 12;
    }
    let end = NaiveDate::from_ymd_opt(end_year, end_month as u32, 1).unwrap_or(start);
    month <= end
}

fn months_between_inclusive(start: NaiveDate, end: NaiveDate) -> u32 {
    if end < start {
        return 0;
    }
    let years = end.year() - start.year();
    let months = end.month() as i32 - start.month() as i32 + years * 12 + 1;
    months.max(0) as u32
}

fn first_of_month(date: NaiveDate) -> Result<NaiveDate> {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
        .context("data inválida ao calcular primeiro dia do mês")
}

fn last_day_of_month(first: NaiveDate) -> Result<NaiveDate> {
    let next = shift_month(first, 1)?;
    next.pred_opt()
        .context("falha ao calcular último dia do mês")
}

/// "2026-05" → "mai/26".
fn short_month_label(month_ref: &str) -> String {
    let parts: Vec<&str> = month_ref.split('-').collect();
    if parts.len() != 2 {
        return month_ref.to_string();
    }
    let year_short = parts[0]
        .get(parts[0].len().saturating_sub(2)..)
        .unwrap_or(parts[0]);
    let month_num: u32 = match parts[1].parse() {
        Ok(m) => m,
        Err(_) => return month_ref.to_string(),
    };
    let name = match month_num {
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
        _ => return month_ref.to_string(),
    };
    format!("{name}/{year_short}")
}

// ---------------------------------------------------------------------------
// SVG renderer
// ---------------------------------------------------------------------------

const W: f64 = 1100.0;
const H: f64 = 580.0;
const PAD_L: f64 = 80.0;
const PAD_R: f64 = 60.0;
const PAD_T: f64 = 80.0;
const PAD_B: f64 = 90.0;

const COL_BAL: &str = "#2563eb";
const COL_IN: &str = "#16a34a";
const COL_OUT: &str = "#dc2626";
const COL_GRID: &str = "#e5e7eb";
const COL_AXIS: &str = "#374151";
const COL_TXT: &str = "#111827";
const COL_MUTED: &str = "#6b7280";
const COL_SCENARIO: &str = "#f59e0b";

pub(crate) fn render_svg(chart: &ChartData) -> String {
    let plot_w = W - PAD_L - PAD_R;
    let plot_h = H - PAD_T - PAD_B;
    let n = chart.months.len();
    let slots = (n + 1) as f64; // +1 for the leading "início" slot
    let slot_w = plot_w / slots;
    let slot_center = |i: usize| PAD_L + slot_w * (i as f64 + 0.5);

    // Y scale: max of all stacked series.
    let scenario_max = chart
        .scenario
        .as_ref()
        .map(|s| {
            s.projected_balance
                .iter()
                .filter_map(|v| v.map(decimal_to_f64))
                .fold(0.0_f64, f64::max)
        })
        .unwrap_or(0.0);
    let max_val = chart
        .months
        .iter()
        .flat_map(|m| {
            let in_total = decimal_to_f64(m.inflows)
                + m.forecast_inflows_remaining
                    .map(decimal_to_f64)
                    .unwrap_or(0.0);
            let out_total = decimal_to_f64(m.outflows)
                + m.forecast_outflows_remaining
                    .map(decimal_to_f64)
                    .unwrap_or(0.0);
            [
                in_total,
                out_total,
                m.closing_balance.map(decimal_to_f64).unwrap_or(0.0),
                m.projected_closing_balance
                    .map(decimal_to_f64)
                    .unwrap_or(0.0),
            ]
        })
        .fold(0.0_f64, f64::max)
        .max(chart.initial_balance.map(decimal_to_f64).unwrap_or(0.0))
        .max(scenario_max);
    let y_max = if max_val <= 0.0 {
        10_000.0
    } else {
        ((max_val / 10_000.0).ceil()) * 10_000.0
    };
    let y_for = |v: f64| PAD_T + plot_h - (v / y_max) * plot_h;

    let bar_w = slot_w * 0.32;
    let mut svg = String::with_capacity(16 * 1024);
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" font-family="-apple-system, BlinkMacSystemFont, sans-serif" font-size="12">"#
    ));

    // Hatch patterns for forecast extension on top of bars.
    svg.push_str(&format!(
        r##"<defs>
<pattern id="hatch-in" patternUnits="userSpaceOnUse" width="6" height="6" patternTransform="rotate(45)">
<rect width="6" height="6" fill="{COL_IN}" opacity="0.18"/>
<line x1="0" y1="0" x2="0" y2="6" stroke="{COL_IN}" stroke-width="2"/>
</pattern>
<pattern id="hatch-out" patternUnits="userSpaceOnUse" width="6" height="6" patternTransform="rotate(45)">
<rect width="6" height="6" fill="{COL_OUT}" opacity="0.18"/>
<line x1="0" y1="0" x2="0" y2="6" stroke="{COL_OUT}" stroke-width="2"/>
</pattern>
</defs>"##
    ));

    svg.push_str(&format!(
        r##"<rect width="{W}" height="{H}" fill="#ffffff"/>"##
    ));

    // Title + subtitle
    let title = "Evolução de caixa · contas correntes";
    let subtitle = build_subtitle(chart);
    svg.push_str(&format!(
        r#"<text x="{x}" y="34" text-anchor="middle" font-size="18" font-weight="600" fill="{COL_TXT}">{title}</text>"#,
        x = W / 2.0
    ));
    svg.push_str(&format!(
        r#"<text x="{x}" y="54" text-anchor="middle" font-size="11" fill="{COL_MUTED}">{subtitle}</text>"#,
        x = W / 2.0
    ));

    // Y gridlines + labels
    let step = ((y_max / 6.0 / 10_000.0).ceil().max(1.0)) * 10_000.0;
    let mut v = 0.0_f64;
    while v <= y_max + 1.0 {
        let y = y_for(v);
        svg.push_str(&format!(
            r#"<line x1="{PAD_L}" y1="{y:.1}" x2="{x2}" y2="{y:.1}" stroke="{COL_GRID}" stroke-width="1"/>"#,
            x2 = W - PAD_R
        ));
        svg.push_str(&format!(
            r#"<text x="{x}" y="{y2:.1}" text-anchor="end" fill="{COL_MUTED}">{lbl}</text>"#,
            x = PAD_L - 8.0,
            y2 = y + 4.0,
            lbl = brl_short(v)
        ));
        v += step;
    }

    // X axis baseline
    svg.push_str(&format!(
        r#"<line x1="{PAD_L}" y1="{y}" x2="{x2}" y2="{y}" stroke="{COL_AXIS}" stroke-width="1.5"/>"#,
        y = PAD_T + plot_h,
        x2 = W - PAD_R
    ));

    // Highlight band over the future-only portion of the plot.
    if chart.realized_count < n {
        let band_x = slot_center(chart.realized_count) - slot_w / 2.0;
        let band_w = slot_center(n) + slot_w / 2.0 - band_x;
        svg.push_str(&format!(
            r##"<rect x="{band_x:.1}" y="{PAD_T}" width="{band_w:.1}" height="{plot_h}" fill="#f9fafb"/>"##
        ));
        // Label
        svg.push_str(&format!(
            r#"<text x="{x:.1}" y="{y:.1}" text-anchor="middle" fill="{COL_MUTED}" font-size="10" font-style="italic">projetado</text>"#,
            x = band_x + band_w / 2.0,
            y = PAD_T + 14.0,
        ));
    }

    // Stacked bars per month: solid (realized) + hatched (forecast remaining)
    for (i, m) in chart.months.iter().enumerate() {
        let cx = slot_center(i + 1);

        // ── entradas (left) ──
        let ent_realized = decimal_to_f64(m.inflows);
        let ent_fc = m
            .forecast_inflows_remaining
            .map(decimal_to_f64)
            .unwrap_or(0.0);
        let x_in = cx - bar_w - 2.0;
        let y_top = y_for(ent_realized + ent_fc);
        let y_realized_top = y_for(ent_realized);
        // realized solid bottom
        if ent_realized > 0.0 {
            let h = PAD_T + plot_h - y_realized_top;
            svg.push_str(&format!(
                r#"<rect x="{x_in:.1}" y="{y_realized_top:.1}" width="{bar_w:.1}" height="{h:.1}" fill="{COL_IN}" opacity="0.85"/>"#
            ));
        }
        // forecast remaining hatched on top
        if ent_fc > 0.0 {
            let h = y_realized_top - y_top;
            svg.push_str(&format!(
                r#"<rect x="{x_in:.1}" y="{y_top:.1}" width="{bar_w:.1}" height="{h:.1}" fill="url(#hatch-in)" stroke="{COL_IN}" stroke-width="0.8" stroke-dasharray="2 2"/>"#
            ));
        }
        // label on top of stacked bar
        let total_in = ent_realized + ent_fc;
        if total_in > 0.0 {
            svg.push_str(&format!(
                r#"<text x="{xm:.1}" y="{yl:.1}" text-anchor="middle" font-size="9" fill="{COL_IN}">{lbl}</text>"#,
                xm = x_in + bar_w / 2.0,
                yl = y_top - 4.0,
                lbl = brl_k(total_in)
            ));
        }

        // ── saídas (right) ──
        let sai_realized = decimal_to_f64(m.outflows);
        let sai_fc = m
            .forecast_outflows_remaining
            .map(decimal_to_f64)
            .unwrap_or(0.0);
        let x_out = cx + 2.0;
        let y_top_out = y_for(sai_realized + sai_fc);
        let y_realized_top_out = y_for(sai_realized);
        if sai_realized > 0.0 {
            let h = PAD_T + plot_h - y_realized_top_out;
            svg.push_str(&format!(
                r#"<rect x="{x_out:.1}" y="{y_realized_top_out:.1}" width="{bar_w:.1}" height="{h:.1}" fill="{COL_OUT}" opacity="0.85"/>"#
            ));
        }
        if sai_fc > 0.0 {
            let h = y_realized_top_out - y_top_out;
            svg.push_str(&format!(
                r#"<rect x="{x_out:.1}" y="{y_top_out:.1}" width="{bar_w:.1}" height="{h:.1}" fill="url(#hatch-out)" stroke="{COL_OUT}" stroke-width="0.8" stroke-dasharray="2 2"/>"#
            ));
        }
        let total_out = sai_realized + sai_fc;
        if total_out > 0.0 {
            svg.push_str(&format!(
                r#"<text x="{xm:.1}" y="{yl:.1}" text-anchor="middle" font-size="9" fill="{COL_OUT}">{lbl}</text>"#,
                xm = x_out + bar_w / 2.0,
                yl = y_top_out - 4.0,
                lbl = brl_k(total_out)
            ));
        }
    }

    // X labels
    svg.push_str(&format!(
        r#"<text x="{x:.1}" y="{y:.1}" text-anchor="middle" fill="{COL_MUTED}" font-style="italic">início</text>"#,
        x = slot_center(0),
        y = PAD_T + plot_h + 18.0,
    ));
    for (i, m) in chart.months.iter().enumerate() {
        let color = if m.is_future { COL_MUTED } else { COL_TXT };
        svg.push_str(&format!(
            r#"<text x="{x:.1}" y="{y:.1}" text-anchor="middle" fill="{color}">{lbl}</text>"#,
            x = slot_center(i + 1),
            y = PAD_T + plot_h + 18.0,
            lbl = m.label
        ));
    }

    // ── Saldo line: solid through realized, dashed through projected ──
    let initial = chart.initial_balance.map(decimal_to_f64);

    // Realized segment: initial + per-month closing_balance up to realized_count.
    let mut realized_pts: Vec<(f64, f64, f64)> = Vec::new();
    if let Some(init) = initial {
        realized_pts.push((slot_center(0), y_for(init), init));
    }
    for (i, m) in chart.months.iter().enumerate() {
        if m.is_future {
            break;
        }
        if let Some(bal) = m.closing_balance {
            let v = decimal_to_f64(bal);
            realized_pts.push((slot_center(i + 1), y_for(v), v));
        }
    }
    if realized_pts.len() >= 2 {
        let path = realized_pts
            .iter()
            .enumerate()
            .map(|(i, (x, y, _))| format!("{} {:.1},{:.1}", if i == 0 { "M" } else { "L" }, x, y))
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!(
            r#"<path d="{path}" fill="none" stroke="{COL_BAL}" stroke-width="2.5"/>"#
        ));
    }
    for (x, y, v) in &realized_pts {
        svg.push_str(&format!(
            r##"<circle cx="{x:.1}" cy="{y:.1}" r="4" fill="{COL_BAL}" stroke="#ffffff" stroke-width="1.5"/>"##
        ));
        svg.push_str(&format!(
            r#"<text x="{x:.1}" y="{y2:.1}" text-anchor="middle" font-size="10" font-weight="600" fill="{COL_BAL}">{lbl}</text>"#,
            y2 = y - 10.0,
            lbl = brl_no_sign(*v)
        ));
    }

    // Projected segment: continues from the last realized point through the
    // projected_closing of every subsequent month (current + future).
    if chart.with_forecast {
        // Start the projected path at the last realized point so the dashed
        // line connects seamlessly.
        let mut proj_pts: Vec<(f64, f64, f64, bool)> = Vec::new();
        if let Some(&(x, y, v)) = realized_pts.last() {
            proj_pts.push((x, y, v, false)); // anchor (don't redraw circle)
        }
        for (i, m) in chart.months.iter().enumerate() {
            // Skip months that already contributed a realized point with
            // identical projected value (their realized circle is enough).
            if !m.is_future
                && m.closing_balance.is_some()
                && m.projected_closing_balance == m.closing_balance
            {
                continue;
            }
            if let Some(p) = m.projected_closing_balance {
                let v = decimal_to_f64(p);
                proj_pts.push((slot_center(i + 1), y_for(v), v, true));
            }
        }
        if proj_pts.len() >= 2 {
            let path = proj_pts
                .iter()
                .enumerate()
                .map(|(i, (x, y, _, _))| {
                    format!("{} {:.1},{:.1}", if i == 0 { "M" } else { "L" }, x, y)
                })
                .collect::<Vec<_>>()
                .join(" ");
            svg.push_str(&format!(
                r#"<path d="{path}" fill="none" stroke="{COL_BAL}" stroke-width="2" stroke-dasharray="6 4" opacity="0.75"/>"#
            ));
            // Open-circle markers only on the projected-only points (skip the anchor).
            for (x, y, v, draw) in proj_pts.iter().skip(1) {
                if !draw {
                    continue;
                }
                svg.push_str(&format!(
                    r##"<circle cx="{x:.1}" cy="{y:.1}" r="4" fill="#ffffff" stroke="{COL_BAL}" stroke-width="1.5"/>"##
                ));
                svg.push_str(&format!(
                    r#"<text x="{x:.1}" y="{y2:.1}" text-anchor="middle" font-size="10" font-weight="600" fill="{COL_BAL}" opacity="0.85">{lbl}</text>"#,
                    y2 = y - 10.0,
                    lbl = brl_no_sign(*v)
                ));
            }
        }

        // Scenario overlay: a second dashed line in COL_SCENARIO sharing
        // the same anchor as the baseline projection (so the eye reads the
        // delta as "what the line *would* do if I added X/mo").
        if let Some(scenario) = chart.scenario.as_ref() {
            let mut scen_pts: Vec<(f64, f64, f64, bool)> = Vec::new();
            if let Some(&(x, y, v)) = realized_pts.last() {
                scen_pts.push((x, y, v, false)); // anchor
            }
            for (i, balance) in scenario.projected_balance.iter().enumerate() {
                let Some(b) = balance else {
                    continue;
                };
                let v = decimal_to_f64(*b);
                let m = &chart.months[i];
                // Skip points where the scenario equals the baseline (no
                // delta accumulated yet) to keep the line clean.
                if let Some(p) = m.projected_closing_balance {
                    if (decimal_to_f64(p) - v).abs() < 0.005 {
                        continue;
                    }
                }
                scen_pts.push((slot_center(i + 1), y_for(v), v, true));
            }
            if scen_pts.len() >= 2 {
                let path = scen_pts
                    .iter()
                    .enumerate()
                    .map(|(i, (x, y, _, _))| {
                        format!("{} {:.1},{:.1}", if i == 0 { "M" } else { "L" }, x, y)
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                svg.push_str(&format!(
                    r#"<path d="{path}" fill="none" stroke="{COL_SCENARIO}" stroke-width="2" stroke-dasharray="3 3" opacity="0.9"/>"#
                ));
                for (x, y, _v, draw) in scen_pts.iter().skip(1) {
                    if !draw {
                        continue;
                    }
                    svg.push_str(&format!(
                        r##"<circle cx="{x:.1}" cy="{y:.1}" r="3.5" fill="#ffffff" stroke="{COL_SCENARIO}" stroke-width="1.5"/>"##
                    ));
                }
                // Label the last scenario point so the delta vs baseline is
                // readable at a glance.
                if let Some((x, y, v, _)) = scen_pts.last() {
                    svg.push_str(&format!(
                        r#"<text x="{x:.1}" y="{y2:.1}" text-anchor="middle" font-size="10" font-weight="700" fill="{COL_SCENARIO}">{lbl}</text>"#,
                        y2 = y + 14.0,
                        lbl = brl_no_sign(*v)
                    ));
                }
            }
        }

        // "Hoje" + "Projetado" callouts in the top-right corner
        let hoje = realized_pts.last().map(|(_, _, v)| *v);
        let projetado = chart
            .months
            .last()
            .and_then(|m| m.projected_closing_balance)
            .map(decimal_to_f64);
        let projetado_scenario = chart
            .scenario
            .as_ref()
            .and_then(|s| s.projected_balance.last().copied().flatten())
            .map(decimal_to_f64);
        let mut callouts: Vec<(String, String, CalloutStyle)> = Vec::new();
        if let Some(v) = hoje {
            callouts.push(("Hoje".into(), brl_no_sign(v), CalloutStyle::Solid));
        }
        if let Some(v) = projetado {
            callouts.push((
                format!(
                    "Projetado ({})",
                    chart
                        .months
                        .last()
                        .map(|m| m.label.clone())
                        .unwrap_or_default()
                ),
                brl_no_sign(v),
                CalloutStyle::DashedBlue,
            ));
        }
        if let (Some(v), Some(scenario)) = (projetado_scenario, chart.scenario.as_ref()) {
            let delta = projetado.map(|p| v - p).unwrap_or(0.0);
            callouts.push((
                format!(
                    "{} (Δ {sign}{deltav})",
                    scenario.label,
                    sign = if delta >= 0.0 { "+" } else { "" },
                    deltav = brl_no_sign(delta)
                ),
                brl_no_sign(v),
                CalloutStyle::DashedScenario,
            ));
        }
        let cx0 = W - PAD_R - 240.0;
        let cy0 = PAD_T - 12.0;
        for (i, (lbl, val, style)) in callouts.iter().enumerate() {
            let y = cy0 - (callouts.len() as f64 - 1.0 - i as f64) * 18.0;
            let (stroke_color, stroke_attr) = match style {
                CalloutStyle::Solid => (COL_BAL, ""),
                CalloutStyle::DashedBlue => (COL_BAL, r#" stroke-dasharray="3 2""#),
                CalloutStyle::DashedScenario => (COL_SCENARIO, r#" stroke-dasharray="3 3""#),
            };
            svg.push_str(&format!(
                r#"<line x1="{cx0:.1}" y1="{y2:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="{stroke_color}" stroke-width="2"{stroke_attr}/>"#,
                x2 = cx0 + 18.0,
                y2 = y - 4.0,
            ));
            svg.push_str(&format!(
                r#"<text x="{x:.1}" y="{y:.1}" fill="{COL_TXT}" font-size="11">{lbl}: <tspan font-weight="700">{val}</tspan></text>"#,
                x = cx0 + 24.0,
            ));
        }
    }

    // Legend (bottom)
    let ly = H - 28.0;
    let mut lx = PAD_L;
    let mut legend_items: Vec<(&str, &str, LegendStyle)> = vec![
        (COL_IN, "Entradas realizadas", LegendStyle::Solid),
        (COL_OUT, "Saídas realizadas", LegendStyle::Solid),
        (COL_BAL, "Saldo realizado", LegendStyle::Line),
    ];
    if chart.with_forecast {
        legend_items.push((COL_IN, "Entradas previstas", LegendStyle::Hatched));
        legend_items.push((COL_OUT, "Saídas previstas", LegendStyle::Hatched));
        legend_items.push((COL_BAL, "Saldo projetado", LegendStyle::Dashed));
        if chart.scenario.is_some() {
            legend_items.push((COL_SCENARIO, "Saldo c/ cenário", LegendStyle::ScenarioLine));
        }
    }
    for (color, label, style) in legend_items {
        match style {
            LegendStyle::Solid => svg.push_str(&format!(
                r#"<rect x="{lx}" y="{y}" width="14" height="14" fill="{color}" opacity="0.85"/>"#,
                y = ly - 10.0
            )),
            LegendStyle::Hatched => {
                let id = if color == COL_IN { "hatch-in" } else { "hatch-out" };
                svg.push_str(&format!(
                    r##"<rect x="{lx}" y="{y}" width="14" height="14" fill="url(#{id})" stroke="{color}" stroke-width="0.8" stroke-dasharray="2 2"/>"##,
                    y = ly - 10.0
                ));
            }
            LegendStyle::Line => svg.push_str(&format!(
                r#"<line x1="{lx}" y1="{y}" x2="{x2}" y2="{y}" stroke="{color}" stroke-width="2.5"/>"#,
                y = ly - 3.0,
                x2 = lx + 14.0
            )),
            LegendStyle::Dashed => svg.push_str(&format!(
                r#"<line x1="{lx}" y1="{y}" x2="{x2}" y2="{y}" stroke="{color}" stroke-width="2" stroke-dasharray="5 4" opacity="0.85"/>"#,
                y = ly - 3.0,
                x2 = lx + 14.0
            )),
            LegendStyle::ScenarioLine => svg.push_str(&format!(
                r#"<line x1="{lx}" y1="{y}" x2="{x2}" y2="{y}" stroke="{color}" stroke-width="2" stroke-dasharray="3 3"/>"#,
                y = ly - 3.0,
                x2 = lx + 14.0
            )),
        }
        svg.push_str(&format!(
            r#"<text x="{x}" y="{y}" fill="{COL_TXT}">{label}</text>"#,
            x = lx + 20.0,
            y = ly + 1.0,
        ));
        lx += 165.0;
    }

    svg.push_str("</svg>");
    svg
}

#[derive(Clone, Copy)]
enum LegendStyle {
    Solid,
    Hatched,
    Line,
    Dashed,
    ScenarioLine,
}

#[derive(Clone, Copy)]
enum CalloutStyle {
    Solid,
    DashedBlue,
    DashedScenario,
}

fn build_subtitle(chart: &ChartData) -> String {
    let n = chart.months.len();
    let realized = chart.realized_count;
    let future = n.saturating_sub(realized);
    let first = chart
        .months
        .first()
        .map(|m| m.label.clone())
        .unwrap_or_default();
    let last = chart
        .months
        .last()
        .map(|m| m.label.clone())
        .unwrap_or_default();
    let mut s = if future == 0 {
        format!("{realized} meses · {first} – {last} · cash-basis · BRL")
    } else {
        format!("{realized} realizados + {future} projetados · {first} – {last} · cash-basis · BRL")
    };
    if chart.with_forecast {
        s.push_str(" · forecast empilhado");
    }
    if let Some(scenario) = chart.scenario.as_ref() {
        s.push_str(&format!(
            " · cenário “{}” {}/mês por {}m a partir de {}",
            scenario.label,
            human_format::brl_signed(scenario.amount),
            scenario.months,
            short_month_label(&format!(
                "{}-{:02}",
                scenario.start_month.year(),
                scenario.start_month.month()
            ))
        ));
    }
    s
}

fn decimal_to_f64(d: Decimal) -> f64 {
    d.to_string().parse::<f64>().unwrap_or(0.0)
}

fn brl_short(v: f64) -> String {
    let int = v.round() as i64;
    let mut s = String::new();
    let abs = int.abs().to_string();
    let bytes = abs.as_bytes();
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            s.push('.');
        }
        s.push(*b as char);
    }
    if int < 0 {
        format!("R$ -{s}")
    } else {
        format!("R$ {s}")
    }
}

fn brl_no_sign(v: f64) -> String {
    let d = Decimal::from_f64_retain(v).unwrap_or(Decimal::ZERO);
    human_format::brl(d)
}

fn brl_k(v: f64) -> String {
    format!("{:.1}k", v / 1000.0)
}

// ---------------------------------------------------------------------------
// ASCII sparkline renderer
// ---------------------------------------------------------------------------

const SPARK_BLOCKS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub(crate) fn render_sparkline(chart: &ChartData) -> String {
    let mut out = String::new();
    let labels: Vec<&str> = chart.months.iter().map(|m| m.label.as_str()).collect();

    // Stacked totals (realized + forecast remaining) — this is what the
    // SVG bars show, so the sparkline echoes the same shape.
    let in_total: Vec<f64> = chart
        .months
        .iter()
        .map(|m| {
            decimal_to_f64(m.inflows)
                + m.forecast_inflows_remaining
                    .map(decimal_to_f64)
                    .unwrap_or(0.0)
        })
        .collect();
    let out_total: Vec<f64> = chart
        .months
        .iter()
        .map(|m| {
            decimal_to_f64(m.outflows)
                + m.forecast_outflows_remaining
                    .map(decimal_to_f64)
                    .unwrap_or(0.0)
        })
        .collect();

    // Saldo: prefer realized closing, fall back to projection.
    let saldo: Vec<f64> = chart
        .months
        .iter()
        .map(|m| {
            m.closing_balance
                .or(m.projected_closing_balance)
                .map(decimal_to_f64)
                .unwrap_or(0.0)
        })
        .collect();

    let realized = chart.realized_count;
    let future = chart.months.len().saturating_sub(realized);
    let header = if future == 0 {
        format!("💵 Evolução de caixa · {realized} meses\n")
    } else {
        format!("💵 Evolução de caixa · {realized} realizados + {future} projetados\n")
    };
    out.push_str(&header);
    out.push_str(&format!("  Saldo     {}\n", sparkline_line(&saldo)));
    out.push_str(&format!("  Entradas  {}\n", sparkline_line(&in_total)));
    out.push_str(&format!("  Saídas    {}\n", sparkline_line(&out_total)));

    // Callouts: hoje vs projetado
    if chart.with_forecast {
        let hoje = chart
            .months
            .iter()
            .filter(|m| !m.is_future)
            .filter_map(|m| m.closing_balance)
            .next_back();
        let projetado = chart
            .months
            .last()
            .and_then(|m| m.projected_closing_balance);
        if let (Some(h), Some(p)) = (hoje, projetado) {
            out.push_str(&format!("  Hoje      {}\n", human_format::brl(h)));
            out.push_str(&format!("  Projetado {}\n", human_format::brl(p)));
        }
    }

    if !labels.is_empty() {
        let mut line = String::from("            ");
        for (i, label) in labels.iter().enumerate() {
            if i > 0 {
                line.push(' ');
            }
            line.push_str(label);
        }
        out.push_str(&line);
        out.push('\n');
    }

    out
}

fn sparkline_line(values: &[f64]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let max = values
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max)
        .max(0.0);
    let mut s = String::with_capacity(values.len() * 6);
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let idx = if max <= 0.0 {
            0
        } else {
            (((v / max) * (SPARK_BLOCKS.len() - 1) as f64).round() as usize)
                .min(SPARK_BLOCKS.len() - 1)
        };
        s.push(SPARK_BLOCKS[idx]);
        for _ in 1..5 {
            s.push(' ');
        }
    }
    s.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rust_decimal_macros::dec;
    use serde_json::json;

    #[test]
    fn parent_category_rolls_up_subcategories() {
        assert_eq!(parent_category(Some("moradia:servicos")), "moradia");
        assert_eq!(parent_category(Some("moradia")), "moradia");
        assert_eq!(parent_category(None), "sem-categoria");
        assert_eq!(parent_category(Some("")), "sem-categoria");
    }

    #[test]
    fn envelope_remaining_nets_realized_against_budget() {
        // Moradia barely spent → almost the whole budget still remains.
        // Alimentação over-spent → nothing remains (realized is in the bar).
        let fc = HashMap::from([
            ("moradia".to_string(), dec!(9300)),
            ("alimentacao".to_string(), dec!(7500)),
        ]);
        let realized = HashMap::from([
            ("moradia".to_string(), dec!(100)),
            ("alimentacao".to_string(), dec!(8000)),
        ]);
        assert_eq!(envelope_remaining(&fc, &realized), dec!(9200));
    }

    #[test]
    fn envelope_remaining_passes_full_budget_when_nothing_realized() {
        let fc = HashMap::from([("educacao".to_string(), dec!(4900))]);
        assert_eq!(envelope_remaining(&fc, &HashMap::new()), dec!(4900));
    }

    fn realized_only() -> ChartData {
        ChartData {
            months: vec![
                MonthDatum {
                    label: "mar/26".into(),
                    month: "2026-03".into(),
                    inflows: dec!(10000),
                    outflows: dec!(8000),
                    closing_balance: Some(dec!(12000)),
                    forecast_inflows_remaining: None,
                    forecast_outflows_remaining: None,
                    projected_closing_balance: Some(dec!(12000)),
                    is_future: false,
                },
                MonthDatum {
                    label: "abr/26".into(),
                    month: "2026-04".into(),
                    inflows: dec!(11000),
                    outflows: dec!(9500),
                    closing_balance: Some(dec!(13500)),
                    forecast_inflows_remaining: None,
                    forecast_outflows_remaining: None,
                    projected_closing_balance: Some(dec!(13500)),
                    is_future: false,
                },
            ],
            initial_balance: Some(dec!(10000)),
            with_forecast: false,
            realized_count: 2,
            scenario: None,
        }
    }

    fn sample_forecast(recurrence: Option<&str>, source: Option<&str>) -> ForecastRecord {
        ForecastRecord {
            forecast_id: "forecast-1".into(),
            due_date: Some(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()),
            description: "Synthetic forecast".into(),
            amount: dec!(-100),
            category_id: None,
            account_id: Some("account-1".into()),
            status: "ativo".into(),
            recurrence: recurrence.map(str::to_string),
            actor_id: "test".into(),
            idempotency_key: "forecast-1".into(),
            metadata_json: source
                .map(|s| json!({ "source": s }))
                .unwrap_or_else(|| json!({})),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            template_id: None,
            realized_transaction_id: None,
            realized_at: None,
        }
    }

    #[test]
    fn chart_projection_ignores_open_card_bill_forecasts() {
        assert!(!forecast_counts_in_chart_projection(&sample_forecast(
            Some("card-cycle"),
            None
        )));
        assert!(!forecast_counts_in_chart_projection(&sample_forecast(
            None,
            Some("card-open-bill")
        )));
        assert!(forecast_counts_in_chart_projection(&sample_forecast(
            Some("monthly"),
            Some("manual")
        )));
    }

    fn realized_plus_future() -> ChartData {
        // 1 realized + 1 current (partial) + 1 future.
        ChartData {
            months: vec![
                MonthDatum {
                    label: "mar/26".into(),
                    month: "2026-03".into(),
                    inflows: dec!(10000),
                    outflows: dec!(8000),
                    closing_balance: Some(dec!(12000)),
                    forecast_inflows_remaining: Some(dec!(0)),
                    forecast_outflows_remaining: Some(dec!(0)),
                    projected_closing_balance: Some(dec!(12000)),
                    is_future: false,
                },
                MonthDatum {
                    label: "abr/26".into(),
                    month: "2026-04".into(),
                    inflows: dec!(5000), // partial month
                    outflows: dec!(2000),
                    closing_balance: Some(dec!(15000)),
                    forecast_inflows_remaining: Some(dec!(6000)),
                    forecast_outflows_remaining: Some(dec!(7000)),
                    projected_closing_balance: Some(dec!(14000)),
                    is_future: false,
                },
                MonthDatum {
                    label: "mai/26".into(),
                    month: "2026-05".into(),
                    inflows: dec!(0),
                    outflows: dec!(0),
                    closing_balance: None,
                    forecast_inflows_remaining: Some(dec!(11000)),
                    forecast_outflows_remaining: Some(dec!(9000)),
                    projected_closing_balance: Some(dec!(16000)),
                    is_future: true,
                },
            ],
            initial_balance: Some(dec!(10000)),
            with_forecast: true,
            realized_count: 2,
            scenario: None,
        }
    }

    fn realized_plus_future_with_scenario() -> ChartData {
        let mut base = realized_plus_future();
        let mut balances: Vec<Option<Decimal>> = base
            .months
            .iter()
            .map(|m| m.projected_closing_balance)
            .collect();
        // Scenario: -R$ 500/month starting at index 1 (current month).
        let mut applied = Decimal::ZERO;
        for (i, b) in balances.iter_mut().enumerate() {
            if i >= 1 {
                applied += dec!(-500);
            }
            *b = b.map(|v| v + applied);
        }
        base.scenario = Some(ScenarioOverlay {
            label: "academia extra".to_string(),
            amount: dec!(-500),
            start_month: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            months: 12,
            projected_balance: balances,
        });
        base
    }

    #[test]
    fn svg_realized_only_has_no_hatch_or_projection() {
        let svg = render_svg(&realized_only());
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("Evolução de caixa"));
        assert!(svg.contains("mar/26") && svg.contains("abr/26"));
        // No --forecast → no hatched extension, no projected line
        assert!(!svg.contains("url(#hatch-in)"));
        assert!(!svg.contains("url(#hatch-out)"));
        assert!(!svg.contains("Saldo projetado"));
        assert!(!svg.contains("Projetado ("));
    }

    #[test]
    fn svg_forecast_mode_stacks_bars_and_draws_projection() {
        let svg = render_svg(&realized_plus_future());
        // hatched extensions on inflow + outflow bars
        assert!(svg.contains("url(#hatch-in)"));
        assert!(svg.contains("url(#hatch-out)"));
        // projected dashed saldo
        assert!(svg.contains("stroke-dasharray=\"6 4\""));
        // legend gained the forecast entries
        assert!(svg.contains("Entradas previstas"));
        assert!(svg.contains("Saídas previstas"));
        assert!(svg.contains("Saldo projetado"));
        // future-month band label
        assert!(svg.contains(">projetado<"));
        // Hoje + Projetado callouts
        assert!(svg.contains("Hoje:"));
        assert!(svg.contains("Projetado ("));
    }

    #[test]
    fn svg_scenario_overlay_draws_extra_line_and_callout() {
        let svg = render_svg(&realized_plus_future_with_scenario());
        // scenario dashed line (3 3 pattern) + amber color
        assert!(
            svg.contains("stroke-dasharray=\"3 3\"") && svg.contains(COL_SCENARIO),
            "expected dashed scenario line in amber"
        );
        // legend entry + callout with delta
        assert!(svg.contains("Saldo c/ cenário"));
        assert!(svg.contains("academia extra"));
        assert!(svg.contains("(Δ "));
        // subtitle should mention the scenario
        assert!(svg.contains("cenário"));
    }

    #[test]
    fn sparkline_realized_only() {
        let txt = render_sparkline(&realized_only());
        assert!(txt.contains("Saldo"));
        assert!(txt.contains("Entradas"));
        assert!(txt.contains("Saídas"));
        assert!(!txt.contains("Hoje"));
        assert!(!txt.contains("Projetado"));
    }

    #[test]
    fn sparkline_forecast_includes_hoje_projetado() {
        let txt = render_sparkline(&realized_plus_future());
        assert!(txt.contains("Hoje"));
        assert!(txt.contains("Projetado"));
        assert!(txt.contains("realizados + 1 projetados"));
    }

    #[test]
    fn short_month_label_formats_known_month() {
        assert_eq!(short_month_label("2026-05"), "mai/26");
        assert_eq!(short_month_label("2025-12"), "dez/25");
    }

    #[test]
    fn short_month_label_falls_back_on_garbage() {
        assert_eq!(short_month_label("nope"), "nope");
        assert_eq!(short_month_label("2026-13"), "2026-13");
    }
}
