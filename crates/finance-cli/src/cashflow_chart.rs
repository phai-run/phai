//! `finance report cashflow-chart` — renders an SVG (and optional ASCII
//! sparkline) of the last N months of cash-basis cashflow on checking
//! accounts. Optionally overlays forecast totals (dashed) when `--forecast`
//! is passed.
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
    pub label: String,     // "mai/26"
    pub inflows: Decimal,  // actual entradas
    pub outflows: Decimal, // actual saídas (positive)
    pub closing_balance: Option<Decimal>,
    pub forecast_inflows: Option<Decimal>, // None if --forecast was off
    pub forecast_outflows: Option<Decimal>, // None if --forecast was off
}

/// Bundle returned by the data collection pass — what the renderers consume.
#[derive(Debug, Clone)]
pub(crate) struct ChartData {
    pub months: Vec<MonthDatum>,
    /// Opening balance for the leftmost month — the line anchor.
    pub initial_balance: Option<Decimal>,
    pub with_forecast: bool,
}

pub(crate) async fn report_cashflow_chart(args: CashflowChartArgs) -> Result<()> {
    let months = args.months.clamp(2, 24);
    if args.no_svg && !args.text {
        anyhow::bail!("--no-svg sem --text não produz nada: passe --text ou remova --no-svg");
    }

    let (_, config) = load_config().await?;
    let store = open_store(&config).await?;
    run_migrations(store.as_ref(), &config).await?;

    let today = chrono::Local::now().date_naive();
    let current_month = first_of_month(today)?;

    // Build window: months going back from current_month, inclusive.
    let mut window: Vec<NaiveDate> = Vec::with_capacity(months);
    for i in (0..months as i32).rev() {
        window.push(shift_month(current_month, -i)?);
    }

    // Anchor for the leftmost month: balance at the last day of the
    // *previous* month (= opening of the first month in window).
    let first_month_start = window[0];
    let anchor_date = first_month_start
        .pred_opt()
        .context("falha ao computar último dia do mês anterior à janela")?;
    let initial_balance = store
        .checking_balance_at(anchor_date)
        .await?
        .map(|b| b.balance);

    // Collect per-month data.
    let mut data = Vec::with_capacity(months);
    for month_start in &window {
        let month_ref = month_ref_for(*month_start);
        parse_month_ref(&month_ref)?;
        let row = store.cashflow_month(&month_ref).await?;

        let (fc_in, fc_out) = if args.forecast {
            let last_day_date = last_day_of_month(*month_start)?;
            let forecasts = store
                .upcoming_forecasts(*month_start, last_day_date)
                .await?;
            let mut fi = Decimal::ZERO;
            let mut fo = Decimal::ZERO;
            for f in &forecasts {
                if f.amount > Decimal::ZERO {
                    fi += f.amount;
                } else {
                    fo += f.amount.abs();
                }
            }
            (Some(fi), Some(fo))
        } else {
            (None, None)
        };

        data.push(MonthDatum {
            label: short_month_label(&month_ref),
            inflows: row.income,
            outflows: row.expenses,
            closing_balance: row.closing_balance,
            forecast_inflows: fc_in,
            forecast_outflows: fc_out,
        });
    }

    let chart = ChartData {
        months: data,
        initial_balance,
        with_forecast: args.forecast,
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

const W: f64 = 960.0;
const H: f64 = 560.0;
const PAD_L: f64 = 80.0;
const PAD_R: f64 = 40.0;
const PAD_T: f64 = 70.0;
const PAD_B: f64 = 80.0;

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

    // Y scale: max of all series, rounded up to nearest 10k for nice grid.
    let mut max_val = chart
        .months
        .iter()
        .flat_map(|m| {
            [
                decimal_to_f64(m.inflows),
                decimal_to_f64(m.outflows),
                m.closing_balance.map(decimal_to_f64).unwrap_or(0.0),
                m.forecast_inflows.map(decimal_to_f64).unwrap_or(0.0),
                m.forecast_outflows.map(decimal_to_f64).unwrap_or(0.0),
            ]
        })
        .fold(0.0_f64, f64::max);
    max_val = max_val.max(chart.initial_balance.map(decimal_to_f64).unwrap_or(0.0));
    let y_max = if max_val <= 0.0 {
        10_000.0
    } else {
        // round up to next 10k
        ((max_val / 10_000.0).ceil()) * 10_000.0
    };
    let y_for = |v: f64| PAD_T + plot_h - (v / y_max) * plot_h;

    let bar_w = slot_w * 0.32;
    let mut svg = String::with_capacity(8 * 1024);
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" font-family="-apple-system, BlinkMacSystemFont, sans-serif" font-size="12">"#
    ));
    svg.push_str(&format!(
        r##"<rect width="{W}" height="{H}" fill="#ffffff"/>"##
    ));

    // Title
    let title = "Evolução de caixa · contas correntes";
    let subtitle = build_subtitle(chart);
    svg.push_str(&format!(
        r#"<text x="{x}" y="32" text-anchor="middle" font-size="18" font-weight="600" fill="{COL_TXT}">{title}</text>"#,
        x = W / 2.0
    ));
    svg.push_str(&format!(
        r#"<text x="{x}" y="50" text-anchor="middle" font-size="11" fill="{COL_MUTED}">{subtitle}</text>"#,
        x = W / 2.0
    ));

    // Y gridlines + labels at multiples of 10k
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

    // Bars per month
    for (i, m) in chart.months.iter().enumerate() {
        let cx = slot_center(i + 1);
        let ent = decimal_to_f64(m.inflows);
        let sai = decimal_to_f64(m.outflows);

        let x_in = cx - bar_w - 2.0;
        let y_in = y_for(ent);
        let h_in = PAD_T + plot_h - y_in;
        svg.push_str(&format!(
            r#"<rect x="{x_in:.1}" y="{y_in:.1}" width="{bar_w:.1}" height="{h_in:.1}" fill="{COL_IN}" opacity="0.85"/>"#
        ));
        svg.push_str(&format!(
            r#"<text x="{xm:.1}" y="{ylbl:.1}" text-anchor="middle" font-size="9" fill="{COL_IN}">{lbl}</text>"#,
            xm = x_in + bar_w / 2.0,
            ylbl = y_in - 4.0,
            lbl = brl_k(ent)
        ));

        let x_out = cx + 2.0;
        let y_out = y_for(sai);
        let h_out = PAD_T + plot_h - y_out;
        svg.push_str(&format!(
            r#"<rect x="{x_out:.1}" y="{y_out:.1}" width="{bar_w:.1}" height="{h_out:.1}" fill="{COL_OUT}" opacity="0.85"/>"#
        ));
        svg.push_str(&format!(
            r#"<text x="{xm:.1}" y="{ylbl:.1}" text-anchor="middle" font-size="9" fill="{COL_OUT}">{lbl}</text>"#,
            xm = x_out + bar_w / 2.0,
            ylbl = y_out - 4.0,
            lbl = brl_k(sai)
        ));
    }

    // X labels
    svg.push_str(&format!(
        r#"<text x="{x:.1}" y="{y:.1}" text-anchor="middle" fill="{COL_MUTED}" font-style="italic">início</text>"#,
        x = slot_center(0),
        y = PAD_T + plot_h + 18.0,
    ));
    for (i, m) in chart.months.iter().enumerate() {
        svg.push_str(&format!(
            r#"<text x="{x:.1}" y="{y:.1}" text-anchor="middle" fill="{COL_TXT}">{lbl}</text>"#,
            x = slot_center(i + 1),
            y = PAD_T + plot_h + 18.0,
            lbl = m.label
        ));
    }

    // Saldo final line
    let initial = chart.initial_balance.map(decimal_to_f64);
    let mut points: Vec<(f64, f64, f64)> = Vec::new();
    if let Some(init) = initial {
        points.push((slot_center(0), y_for(init), init));
    }
    for (i, m) in chart.months.iter().enumerate() {
        if let Some(bal) = m.closing_balance {
            let bal_f = decimal_to_f64(bal);
            points.push((slot_center(i + 1), y_for(bal_f), bal_f));
        }
    }
    if points.len() >= 2 {
        let path = points
            .iter()
            .enumerate()
            .map(|(i, (x, y, _))| format!("{} {:.1},{:.1}", if i == 0 { "M" } else { "L" }, x, y))
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!(
            r#"<path d="{path}" fill="none" stroke="{COL_BAL}" stroke-width="2.5"/>"#
        ));
        for (x, y, v) in &points {
            svg.push_str(&format!(
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="4" fill="{COL_BAL}" stroke="#ffffff" stroke-width="1.5"/>"##
            ));
            svg.push_str(&format!(
                r#"<text x="{x:.1}" y="{y2:.1}" text-anchor="middle" font-size="10" font-weight="600" fill="{COL_BAL}">{lbl}</text>"#,
                y2 = y - 10.0,
                lbl = brl_no_sign(*v)
            ));
        }
    }

    // Forecast overlay (dashed lines connecting per-month forecast values)
    if chart.with_forecast {
        let fc_in_points: Vec<(f64, f64)> = chart
            .months
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                m.forecast_inflows.map(|f| {
                    let v = decimal_to_f64(f);
                    (slot_center(i + 1), y_for(v))
                })
            })
            .collect();
        let fc_out_points: Vec<(f64, f64)> = chart
            .months
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                m.forecast_outflows.map(|f| {
                    let v = decimal_to_f64(f);
                    (slot_center(i + 1), y_for(v))
                })
            })
            .collect();
        for (pts, color) in [(&fc_in_points, COL_IN), (&fc_out_points, COL_OUT)] {
            if pts.len() < 2 {
                continue;
            }
            let path = pts
                .iter()
                .enumerate()
                .map(|(i, (x, y))| format!("{} {:.1},{:.1}", if i == 0 { "M" } else { "L" }, x, y))
                .collect::<Vec<_>>()
                .join(" ");
            svg.push_str(&format!(
                r#"<path d="{path}" fill="none" stroke="{color}" stroke-width="1.5" stroke-dasharray="5 4" opacity="0.85"/>"#
            ));
            for (x, y) in pts {
                svg.push_str(&format!(
                    r##"<circle cx="{x:.1}" cy="{y:.1}" r="3" fill="#ffffff" stroke="{color}" stroke-width="1.5"/>"##
                ));
            }
        }
    }

    // Legend
    let ly = H - 30.0;
    let mut lx = PAD_L;
    let mut legend_items: Vec<(&str, &str, bool)> = vec![
        (COL_BAL, "Saldo final do mês", false),
        (COL_IN, "Entradas", false),
        (COL_OUT, "Saídas", false),
    ];
    if chart.with_forecast {
        legend_items.push((COL_IN, "Forecast entradas", true));
        legend_items.push((COL_OUT, "Forecast saídas", true));
    }
    for (color, label, dashed) in legend_items {
        if dashed {
            svg.push_str(&format!(
                r#"<line x1="{lx}" y1="{y}" x2="{x2}" y2="{y}" stroke="{color}" stroke-width="2" stroke-dasharray="5 4"/>"#,
                y = ly - 3.0,
                x2 = lx + 14.0
            ));
        } else {
            svg.push_str(&format!(
                r#"<rect x="{lx}" y="{y}" width="14" height="14" fill="{color}" opacity="0.85"/>"#,
                y = ly - 10.0
            ));
        }
        svg.push_str(&format!(
            r#"<text x="{x}" y="{y}" fill="{COL_TXT}">{label}</text>"#,
            x = lx + 20.0,
            y = ly + 1.0,
        ));
        lx += 160.0;
    }

    svg.push_str("</svg>");
    svg
}

fn build_subtitle(chart: &ChartData) -> String {
    let n = chart.months.len();
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
    let mut s = format!("{n} meses · {first} – {last} · base cash-basis · valores em BRL");
    if chart.with_forecast {
        s.push_str(" · forecast sobreposto");
    }
    s
}

fn decimal_to_f64(d: Decimal) -> f64 {
    d.to_string().parse::<f64>().unwrap_or(0.0)
}

fn brl_short(v: f64) -> String {
    // "R$ 30.000"
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
    let header = labels.join(" ");
    let pad = if header.is_empty() { 0 } else { header.len() };

    let saldo: Vec<f64> = chart
        .months
        .iter()
        .map(|m| m.closing_balance.map(decimal_to_f64).unwrap_or(0.0))
        .collect();
    let inflows: Vec<f64> = chart
        .months
        .iter()
        .map(|m| decimal_to_f64(m.inflows))
        .collect();
    let outflows: Vec<f64> = chart
        .months
        .iter()
        .map(|m| decimal_to_f64(m.outflows))
        .collect();

    out.push_str(&format!(
        "💵 Evolução de caixa · {} meses\n",
        chart.months.len()
    ));
    out.push_str(&format!("  Saldo     {}\n", sparkline_line(&saldo)));
    out.push_str(&format!("  Entradas  {}\n", sparkline_line(&inflows)));
    out.push_str(&format!("  Saídas    {}\n", sparkline_line(&outflows)));

    if chart.with_forecast {
        let fc_in: Vec<f64> = chart
            .months
            .iter()
            .map(|m| m.forecast_inflows.map(decimal_to_f64).unwrap_or(0.0))
            .collect();
        let fc_out: Vec<f64> = chart
            .months
            .iter()
            .map(|m| m.forecast_outflows.map(decimal_to_f64).unwrap_or(0.0))
            .collect();
        out.push_str(&format!("  ⇢ Ent.   {}\n", sparkline_line(&fc_in)));
        out.push_str(&format!("  ⇢ Saí.   {}\n", sparkline_line(&fc_out)));
    }

    // Labels footer — render below the sparkline, one per month
    if pad > 0 {
        // Sparkline lines use one block char per month; align labels to columns.
        // Months may have multi-char labels so we space them out evenly.
        let n = chart.months.len();
        let mut line = String::from("            ");
        for (i, label) in labels.iter().enumerate() {
            if i > 0 {
                line.push(' ');
            }
            line.push_str(label);
            // make sure spacing matches (truncated to label length)
            let _ = n;
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
        // padding so blocks align with month labels (~6 cols: "mai/26")
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

    fn sample_data(with_forecast: bool) -> ChartData {
        let months = vec![
            MonthDatum {
                label: "mar/26".into(),
                inflows: dec!(10000),
                outflows: dec!(8000),
                closing_balance: Some(dec!(12000)),
                forecast_inflows: if with_forecast {
                    Some(dec!(9500))
                } else {
                    None
                },
                forecast_outflows: if with_forecast {
                    Some(dec!(8500))
                } else {
                    None
                },
            },
            MonthDatum {
                label: "abr/26".into(),
                inflows: dec!(11000),
                outflows: dec!(9500),
                closing_balance: Some(dec!(13500)),
                forecast_inflows: if with_forecast {
                    Some(dec!(10500))
                } else {
                    None
                },
                forecast_outflows: if with_forecast {
                    Some(dec!(9000))
                } else {
                    None
                },
            },
        ];
        ChartData {
            months,
            initial_balance: Some(dec!(10000)),
            with_forecast,
        }
    }

    #[test]
    fn svg_contains_expected_markers_without_forecast() {
        let svg = render_svg(&sample_data(false));
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("Evolução de caixa"));
        assert!(svg.contains("mar/26"));
        assert!(svg.contains("abr/26"));
        // No forecast → no dashed stroke
        assert!(!svg.contains("stroke-dasharray"));
    }

    #[test]
    fn svg_includes_dashed_forecast_overlay_when_enabled() {
        let svg = render_svg(&sample_data(true));
        assert!(svg.contains("stroke-dasharray"));
        assert!(svg.contains("Forecast entradas"));
        assert!(svg.contains("Forecast saídas"));
    }

    #[test]
    fn sparkline_renders_one_block_per_month() {
        let txt = render_sparkline(&sample_data(false));
        assert!(txt.contains("Saldo"));
        assert!(txt.contains("Entradas"));
        assert!(txt.contains("Saídas"));
        // No forecast lines when disabled
        assert!(!txt.contains("⇢"));
    }

    #[test]
    fn sparkline_includes_forecast_lines_when_enabled() {
        let txt = render_sparkline(&sample_data(true));
        assert!(txt.contains("⇢ Ent"));
        assert!(txt.contains("⇢ Saí"));
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
