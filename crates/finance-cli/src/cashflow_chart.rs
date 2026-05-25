//! `finance report cashflow-chart` — renders an SVG (and optional ASCII
//! sparkline) of cash-basis cashflow on checking accounts.
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
use finance_core::migrations::run_migrations;
use finance_core::storage::open_store;
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};

use crate::human_format;
use crate::{load_config, month_ref_for, parse_month_ref, shift_month, CashflowChartArgs};

/// One month's slice of data for the chart.
#[derive(Debug, Clone)]
pub(crate) struct MonthDatum {
    pub label: String, // "mai/26"
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
#[derive(Debug, Clone)]
pub(crate) struct ChartData {
    pub months: Vec<MonthDatum>,
    /// Opening balance at the left edge of the window — the line anchor.
    pub initial_balance: Option<Decimal>,
    pub with_forecast: bool,
    /// How many months in `months` are in the past or current (the rest are
    /// purely future). Used by the renderer to split solid vs dashed
    /// segments of the saldo line.
    pub realized_count: usize,
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
        // months-ahead without forecast = pure-future months with no data.
        // The bars would just be empty. Better to fail loud than draw nothing.
        anyhow::bail!("--months-ahead requer --forecast (sem forecast não há nada para projetar)");
    }

    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let today = chrono::Local::now().date_naive();
    let current_month = first_of_month(today)?;

    // Window: [current - (months_back - 1), current + months_ahead], oldest first.
    let total = months_back + months_ahead;
    let mut window: Vec<NaiveDate> = Vec::with_capacity(total);
    for i in 0..total {
        let delta = i as i32 - (months_back as i32 - 1);
        window.push(shift_month(current_month, delta)?);
    }

    // Anchor for the leftmost month: balance at the last day of the *previous* month.
    let first_month_start = window[0];
    let anchor_date = first_month_start
        .pred_opt()
        .context("falha ao computar último dia do mês anterior à janela")?;
    let initial_balance = store
        .checking_balance_at(anchor_date)
        .await?
        .map(|b| b.balance);

    let realized_count = months_back; // by construction
    let mut data: Vec<MonthDatum> = Vec::with_capacity(total);
    // For projected_closing rollover: start from initial_balance.
    let mut running_projection: Option<Decimal> = initial_balance;

    for (i, month_start) in window.iter().enumerate() {
        let month_ref = month_ref_for(*month_start);
        parse_month_ref(&month_ref)?;
        let is_future = i >= realized_count;

        // Past + current: realized cashflow comes from the store. Future:
        // no transactions yet, so zeros — bars are purely the forecast.
        let (inflows, outflows, closing_balance) = if !is_future {
            let row = store.cashflow_month(&month_ref).await?;
            (row.income, row.expenses, row.closing_balance)
        } else {
            (Decimal::ZERO, Decimal::ZERO, None)
        };

        let (fc_in_remaining, fc_out_remaining) = if args.forecast {
            let last_day = last_day_of_month(*month_start)?;
            // "Remaining" = forecasts whose due_date hasn't passed yet.
            // For past months this returns nothing (all due_dates < today).
            // For current month it returns only the future-of-today portion.
            // For future months it returns the full month's forecasts.
            // This semantic avoids double-counting items that already
            // materialized (e.g. mid-month salary already received) while
            // still surfacing the end-of-month installment that hasn't.
            let lower = today.succ_opt().unwrap_or(today).max(*month_start);
            let (mut fi_rem, mut fo_rem) = (Decimal::ZERO, Decimal::ZERO);
            if lower <= last_day {
                let forecasts = store.upcoming_forecasts(lower, last_day).await?;
                for f in &forecasts {
                    if f.amount > Decimal::ZERO {
                        fi_rem += f.amount;
                    } else {
                        fo_rem += f.amount.abs();
                    }
                }
            }
            (Some(fi_rem), Some(fo_rem))
        } else {
            (None, None)
        };

        // Projected closing: realized for past, realized + forecast remaining
        // for current, prev_projection + (fc_in - fc_out) for future. We do
        // it in one expression by relying on the fact that for future months
        // realized is zero, so the formula collapses naturally.
        let projected = if args.forecast {
            let net_realized = inflows - outflows;
            let net_remaining = fc_in_remaining.unwrap_or(Decimal::ZERO)
                - fc_out_remaining.unwrap_or(Decimal::ZERO);
            running_projection.map(|prev| prev + net_realized + net_remaining)
        } else {
            // No forecast → projection is just the realized closing.
            closing_balance
        };

        // Roll the projection forward. Prefer the snapshot-anchored closing
        // when available (snapshots are ground truth and may differ from
        // running_projection ± rounding); otherwise carry the projection.
        running_projection = if !is_future && closing_balance.is_some() {
            // Use projected (which incorporates forecast remaining) if forecast
            // is on, else use the realized closing.
            if args.forecast {
                projected
            } else {
                closing_balance
            }
        } else {
            projected
        };

        data.push(MonthDatum {
            label: short_month_label(&month_ref),
            inflows,
            outflows,
            closing_balance,
            forecast_inflows_remaining: fc_in_remaining,
            forecast_outflows_remaining: fc_out_remaining,
            projected_closing_balance: projected,
            is_future,
        });
    }

    let chart = ChartData {
        months: data,
        initial_balance,
        with_forecast: args.forecast,
        realized_count,
    };

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

fn write_svg(path: &Path, body: &str) -> Result<()> {
    std::fs::write(path, body).with_context(|| format!("falha ao escrever {}", path.display()))
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

pub(crate) fn render_svg(chart: &ChartData) -> String {
    let plot_w = W - PAD_L - PAD_R;
    let plot_h = H - PAD_T - PAD_B;
    let n = chart.months.len();
    let slots = (n + 1) as f64; // +1 for the leading "início" slot
    let slot_w = plot_w / slots;
    let slot_center = |i: usize| PAD_L + slot_w * (i as f64 + 0.5);

    // Y scale: max of all stacked series.
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
        .max(chart.initial_balance.map(decimal_to_f64).unwrap_or(0.0));
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

        // "Hoje" + "Projetado" callouts in the top-right corner
        let hoje = realized_pts.last().map(|(_, _, v)| *v);
        let projetado = chart
            .months
            .last()
            .and_then(|m| m.projected_closing_balance)
            .map(decimal_to_f64);
        let mut callouts: Vec<(String, String, bool)> = Vec::new();
        if let Some(v) = hoje {
            callouts.push(("Hoje".into(), brl_no_sign(v), false));
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
                true,
            ));
        }
        let cx0 = W - PAD_R - 200.0;
        let cy0 = PAD_T - 12.0;
        for (i, (lbl, val, dashed)) in callouts.iter().enumerate() {
            let y = cy0 - (callouts.len() as f64 - 1.0 - i as f64) * 18.0;
            let stroke_attr = if *dashed {
                r#" stroke-dasharray="3 2""#
            } else {
                ""
            };
            svg.push_str(&format!(
                r#"<line x1="{cx0:.1}" y1="{y2:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="{COL_BAL}" stroke-width="2"{stroke_attr}/>"#,
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
    use rust_decimal_macros::dec;

    fn realized_only() -> ChartData {
        ChartData {
            months: vec![
                MonthDatum {
                    label: "mar/26".into(),
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
        }
    }

    fn realized_plus_future() -> ChartData {
        // 1 realized + 1 current (partial) + 1 future.
        ChartData {
            months: vec![
                MonthDatum {
                    label: "mar/26".into(),
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
        }
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
