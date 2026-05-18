//! Daily / weekly pulse: the WhatsApp-shaped proactive summary.
//!
//! Produces a single message that answers four questions in this order:
//!
//! 1. How is the month closing? (MtD net + projected EoM net)
//! 2. What needs to brake? (categories above their T3M run-rate)
//! 3. What's coming up? (forecast vencimentos in the rest of the month)
//! 4. How am I exposed? (cards open + due dates)
//!
//! The final block surfaces actions (uncategorized count, budget alerts).
//!
//! All money is `rust_decimal::Decimal` end-to-end. Floats are not used
//! except in the unicode progress bar where the imprecision is cosmetic.

use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate};
use finance_core::models::{
    AccountRecord, BudgetStatusRow, CardSummaryRow, CashflowRow, ForecastRecord,
};
use finance_core::storage::FinanceStore;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

use crate::human_format::{
    bold, brl as hf_brl, brl_signed, category_emoji, category_family, family_label, month_pt_short,
    short_date, short_description, truncate_with_ellipsis,
};

/// Bundle of everything needed to render a pulse. Computed by
/// `gather_pulse_data` from the storage layer; rendered by `render_pulse`.
#[derive(Debug)]
pub struct PulseData {
    pub today: NaiveDate,
    /// Current-month cashflow (income, expenses, net) as observed MtD.
    pub mtd: CashflowRow,
    /// Mean income/expense over the last 3 closed months.
    pub baseline_t3m: CashflowRow,
    /// Per-category MtD expense subtotal (account-collapsed).
    pub current_by_category: BTreeMap<String, (Decimal, i64)>,
    /// Per-category T3M mean monthly expense.
    pub baseline_by_category: BTreeMap<String, Decimal>,
    /// Upcoming forecasts whose due_date falls in (today, end_of_month].
    pub upcoming: Vec<ForecastRecord>,
    /// Total upcoming forecast value for the rest of the month.
    pub upcoming_total: Decimal,
    /// Card open balances for the current month_ref.
    pub cards: Vec<CardSummaryRow>,
    pub accounts: Vec<AccountRecord>,
    /// Budget envelopes that crossed their alert threshold.
    pub budget_alerts: Vec<BudgetStatusRow>,
    pub uncategorized_count: i64,
}

/// Number of days in the calendar month of `date`.
pub fn days_in_month(date: NaiveDate) -> u32 {
    let (year, month) = (date.year(), date.month());
    let first_next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("next year jan 1 is valid")
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1).expect("next month 1 is valid")
    };
    let first_this = NaiveDate::from_ymd_opt(year, month, 1).expect("month 1 is valid");
    (first_next - first_this).num_days() as u32
}

/// "YYYY-MM" string for the calendar month of `date`.
pub fn month_ref(date: NaiveDate) -> String {
    format!("{:04}-{:02}", date.year(), date.month())
}

/// Subtract `months` from `(year, month)` returning a "YYYY-MM" string.
/// `months_back(2026, 5, 1)` -> "2026-04".
fn months_back(year: i32, month: u32, n: u32) -> String {
    let mut y = year;
    let mut m = month as i32 - n as i32;
    while m <= 0 {
        m += 12;
        y -= 1;
    }
    format!("{:04}-{:02}", y, m)
}

/// Returns the last 3 closed-month "YYYY-MM" strings (most recent first).
pub fn last_three_closed_months(today: NaiveDate) -> [String; 3] {
    let (y, m) = (today.year(), today.month());
    [
        months_back(y, m, 1),
        months_back(y, m, 2),
        months_back(y, m, 3),
    ]
}

pub async fn gather_pulse_data(
    store: &dyn FinanceStore,
    today: NaiveDate,
    days: i64,
) -> Result<PulseData> {
    // Kept for symmetry with the existing daily-pulse JSON view; not used
    // by the new headline-driven render but the storage call validates
    // that the window is internally consistent. Errors here would be
    // signal of a deeper backend issue.
    let _ = days; // silence unused warning when feature is disabled below
    let _since = today
        .checked_sub_signed(Duration::days(days.saturating_sub(1)))
        .context("janela invalida")?;
    let internal_categories = store.internal_categories().await?;

    let current_month = month_ref(today);

    // Cashflow over 4 months: current + previous 3.
    let cashflow = store.cashflow(4).await?;
    let mtd = cashflow
        .iter()
        .find(|r| r.month_ref == current_month)
        .cloned()
        .unwrap_or(CashflowRow {
            month_ref: current_month.clone(),
            income: Decimal::ZERO,
            expenses: Decimal::ZERO,
            net: Decimal::ZERO,
        });
    let closed: Vec<&CashflowRow> = cashflow
        .iter()
        .filter(|r| r.month_ref != current_month)
        .take(3)
        .collect();
    let three = Decimal::from(3);
    let baseline_t3m = if closed.is_empty() {
        CashflowRow {
            month_ref: "T3M".into(),
            income: Decimal::ZERO,
            expenses: Decimal::ZERO,
            net: Decimal::ZERO,
        }
    } else {
        let n = Decimal::from(closed.len() as i64);
        let income = closed.iter().map(|r| r.income).sum::<Decimal>() / n;
        let expenses = closed.iter().map(|r| r.expenses).sum::<Decimal>() / n;
        CashflowRow {
            month_ref: "T3M".into(),
            income,
            expenses,
            net: income - expenses,
        }
    };

    // Per-category MtD spend, collapsing accounts.
    let monthly_spend = store.monthly_spend(Some(&current_month)).await?;
    let mut current_by_category: BTreeMap<String, (Decimal, i64)> = BTreeMap::new();
    for row in monthly_spend {
        if internal_categories.contains(&row.category_id) {
            continue;
        }
        let entry = current_by_category
            .entry(row.category_id.clone())
            .or_insert((Decimal::ZERO, 0));
        entry.0 += row.expenses;
        entry.1 += row.expense_count;
    }

    // T3M baseline per category — mean of last 3 closed months.
    let last_three = last_three_closed_months(today);
    let mut baseline_by_category: BTreeMap<String, Decimal> = BTreeMap::new();
    for m in &last_three {
        let rows = store.monthly_spend(Some(m)).await?;
        for row in rows {
            if internal_categories.contains(&row.category_id) {
                continue;
            }
            *baseline_by_category
                .entry(row.category_id.clone())
                .or_insert(Decimal::ZERO) += row.expenses;
        }
    }
    for value in baseline_by_category.values_mut() {
        *value /= three;
    }

    // Upcoming forecasts: today+1 .. end_of_month
    let last_day = days_in_month(today);
    let end_of_month = NaiveDate::from_ymd_opt(today.year(), today.month(), last_day)
        .context("end of month invalid")?;
    let upcoming_start = today.succ_opt().unwrap_or(today);
    let upcoming = if upcoming_start > end_of_month {
        Vec::new()
    } else {
        store
            .upcoming_forecasts(upcoming_start, end_of_month)
            .await?
    };
    let upcoming_total = upcoming.iter().map(|f| f.amount.abs()).sum();

    // `cards_open_now` returns the bill that's actually open RIGHT NOW per
    // card (i.e. the cycle whose closing day is in the future). Using
    // `card_summary(current_calendar_month)` would silently return a
    // *closed* cycle for cards whose closing day already passed this month.
    let cards = store.cards_open_now().await.unwrap_or_default();
    let accounts = store.get_accounts().await.unwrap_or_default();

    let budget_alerts = store
        .budget_status_for_month(&current_month)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.alert || b.usage_pct >= Decimal::from(80))
        .collect();

    let uncategorized_count = store.count_uncategorized().await.unwrap_or(0);
    let _ = internal_categories; // already applied as filter above

    Ok(PulseData {
        today,
        mtd,
        baseline_t3m,
        current_by_category,
        baseline_by_category,
        upcoming,
        upcoming_total,
        cards,
        accounts,
        budget_alerts,
        uncategorized_count,
    })
}

/// What needs to brake and by how much per week to close the month with a
/// non-negative net.
#[derive(Debug, Clone)]
pub struct ClosingPlan {
    pub projected_eom_net: Decimal,
    pub days_left: u32,
    pub weeks_left: u32,
    pub max_weekly_variable_for_zero: Option<Decimal>,
    pub categories_to_brake: Vec<CategoryOverspend>,
    pub status: ClosingStatus,
}

#[derive(Debug, Clone, Copy)]
pub enum ClosingStatus {
    /// Projected net >= 0 even at T3M pace.
    OnTrack,
    /// Projected net < 0; brake variable to close at zero.
    Tight,
    /// Even cutting all variable to zero we close below zero.
    Stretched,
}

#[derive(Debug, Clone)]
pub struct CategoryOverspend {
    pub category_id: String,
    pub mtd: Decimal,
    pub over: Decimal,
    pub over_pct: i64,
}

pub fn compute_closing_plan(data: &PulseData) -> ClosingPlan {
    let today = data.today;
    let last_day = days_in_month(today);
    let days_left = last_day.saturating_sub(today.day());
    // Ceil division: any remaining days form at least one "week".
    let weeks_left = days_left.div_ceil(7).max(1);
    let day_fraction = Decimal::from(today.day()) / Decimal::from(last_day.max(1));

    let expected_income_left = (data.baseline_t3m.income - data.mtd.income).max(Decimal::ZERO);

    // What we know is going to happen this month: forecasts in upcoming + MtD already-posted.
    let known_outflow = data.mtd.expenses + data.upcoming_total;
    let projected_known = data.mtd.income + expected_income_left - known_outflow;

    // What's likely to happen on the variable side if we keep T3M pace.
    // We approximate variable T3M as (baseline_expense_T3M - typical_fixed_T3M).
    // Without a fixed/variable tag, we proxy "fixed already accounted for" by
    // subtracting forecasts (they cover most fixed costs). Daily variable run-rate
    // is then the unaccounted T3M expense divided by month length.
    let unaccounted_t3m = (data.baseline_t3m.expenses - data.upcoming_total).max(Decimal::ZERO);
    let variable_daily_t3m = unaccounted_t3m / Decimal::from(last_day.max(1));
    let projected_variable_remaining = variable_daily_t3m * Decimal::from(days_left);
    let projected_eom_net = projected_known - projected_variable_remaining;

    let max_weekly_variable_for_zero = if projected_known > Decimal::ZERO && weeks_left > 0 {
        Some((projected_known / Decimal::from(weeks_left)).max(Decimal::ZERO))
    } else {
        None
    };

    let status = if projected_eom_net >= Decimal::ZERO {
        ClosingStatus::OnTrack
    } else if projected_known >= Decimal::ZERO {
        ClosingStatus::Tight
    } else {
        ClosingStatus::Stretched
    };

    // Overspend by category. We only flag categories where the projected
    // end-of-month total exceeds the T3M monthly mean by >=10% AND the
    // overshoot is at least R$50. This deliberately suppresses fixed
    // categories that just happen to land early in the month (their MtD
    // would be above pro-rata but their EoM projection lands on baseline).
    //
    // Projection: assume the rest of the month keeps the same daily pace,
    // i.e. eom_proj = mtd * (last_day / day_so_far).
    let mut categories_to_brake: Vec<CategoryOverspend> = data
        .current_by_category
        .iter()
        .filter_map(|(cat, (mtd, count))| {
            let baseline = data
                .baseline_by_category
                .get(cat)
                .copied()
                .unwrap_or(Decimal::ZERO);
            if baseline.is_zero() {
                return None;
            }
            let _expected_pro_rata = baseline * day_fraction;
            // Categories with <3 hits MtD are treated as "lumpy" (likely a
            // one-shot fixed payment, school fee, insurance, etc.). For
            // those we compare MtD directly to the FULL baseline, never
            // pacing. Pacing is only meaningful for high-frequency items
            // (groceries, fuel, restaurants).
            let reference = if *count >= 3 && today.day() > 0 {
                *mtd * Decimal::from(last_day) / Decimal::from(today.day())
            } else {
                *mtd
            };
            let threshold = baseline * Decimal::new(11, 1); // 1.1 × baseline
            if reference > threshold && (reference - baseline) > Decimal::from(200) {
                let over = reference - baseline;
                let over_pct: i64 = if baseline > Decimal::ZERO {
                    (over / baseline * Decimal::from(100))
                        .round()
                        .try_into()
                        .unwrap_or(0)
                } else {
                    0
                };
                Some(CategoryOverspend {
                    category_id: cat.clone(),
                    mtd: *mtd,
                    over,
                    over_pct,
                })
            } else {
                None
            }
        })
        .collect();
    categories_to_brake.sort_by(|a, b| b.over.cmp(&a.over));
    categories_to_brake.truncate(3);

    ClosingPlan {
        projected_eom_net,
        days_left,
        weeks_left,
        max_weekly_variable_for_zero,
        categories_to_brake,
        status,
    }
}

/// Day-of-month label like `"qua 18/mai"`. Uses BR-pt weekday short names.
fn br_date_header(date: NaiveDate) -> String {
    let wd = match date.weekday().num_days_from_monday() {
        0 => "seg",
        1 => "ter",
        2 => "qua",
        3 => "qui",
        4 => "sex",
        5 => "sáb",
        _ => "dom",
    };
    format!("{wd} {:02}/{}", date.day(), month_pt_short(date.month()))
}

/// Render. The output is a sequence of `println!`-style lines (newline-
/// separated, no trailing newline) so callers can route to stdout or a
/// webhook unchanged.
pub fn render_pulse(data: &PulseData, plan: &ClosingPlan, days: i64) -> String {
    let mut out = String::new();
    use std::fmt::Write;

    let title = if days <= 1 {
        format!("Pulso · {}", br_date_header(data.today))
    } else {
        format!("Pulso · últimos {days}d · {}", br_date_header(data.today))
    };
    let _ = writeln!(out, "💸 {}", bold(&title));
    let _ = writeln!(out);

    // -------- Block 1: Mês até hoje + plano de fechamento --------
    let net_mtd = data.mtd.income - data.mtd.expenses;
    let _ = writeln!(
        out,
        "{} {}",
        bold(&format!(
            "Mês até dia {:02} · {} dias restantes",
            data.today.day(),
            plan.days_left
        )),
        if net_mtd.is_sign_negative() {
            "🔻"
        } else {
            "✅"
        }
    );
    let _ = writeln!(
        out,
        "  entradas {} · saídas {} · saldo {}",
        hf_brl(data.mtd.income),
        hf_brl(data.mtd.expenses),
        brl_signed(net_mtd),
    );
    // Headline: closing plan.
    let _ = writeln!(out);
    match plan.status {
        ClosingStatus::OnTrack => {
            let _ = writeln!(
                out,
                "🎯 {} (proj. {})",
                bold("Fecha positivo no ritmo atual"),
                brl_signed(plan.projected_eom_net),
            );
        }
        ClosingStatus::Tight => {
            if let Some(weekly) = plan.max_weekly_variable_for_zero {
                let _ = writeln!(
                    out,
                    "⚠️ {} no ritmo T3M (proj. {})",
                    bold("Fecha negativo"),
                    brl_signed(plan.projected_eom_net),
                );
                let _ = writeln!(
                    out,
                    "   Para fechar zerado: até {} por semana ({} sem) em variáveis",
                    bold(&hf_brl(weekly)),
                    plan.weeks_left,
                );
            }
        }
        ClosingStatus::Stretched => {
            let _ = writeln!(
                out,
                "🚨 {} mesmo cortando variáveis (proj. {})",
                bold("Mês fecha negativo"),
                brl_signed(plan.projected_eom_net),
            );
            let _ = writeln!(
                out,
                "   Compromissos fixos restantes ({}) excedem a receita esperada do mês",
                hf_brl(data.upcoming_total),
            );
        }
    }

    // -------- Block 2: Categorias a frear --------
    if !plan.categories_to_brake.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", bold("Frear neste mês"));
        for c in &plan.categories_to_brake {
            let label = pretty_category_label(&c.category_id);
            let _ = writeln!(
                out,
                "  {} {} · gasto MtD {} (proj. +{}% vs média)",
                category_emoji(Some(&c.category_id), None),
                label,
                hf_brl(c.mtd),
                c.over_pct
            );
        }
    }

    // -------- Block 3: Próximos vencimentos --------
    if !data.upcoming.is_empty() {
        let _ = writeln!(out);
        let next_n: Vec<&ForecastRecord> = data.upcoming.iter().take(6).collect();
        let _ = writeln!(
            out,
            "{} ({} no total até fim do mês)",
            bold("A vencer"),
            hf_brl(data.upcoming_total),
        );
        for f in &next_n {
            let due = f.due_date.map(short_date).unwrap_or_else(|| "—".into());
            let desc = truncate_with_ellipsis(&short_description(&f.description), 28);
            let _ = writeln!(out, "  • {} · {} · {}", due, desc, hf_brl(f.amount.abs()));
        }
        if data.upcoming.len() > next_n.len() {
            let _ = writeln!(
                out,
                "  _… mais {} compromissos_",
                data.upcoming.len() - next_n.len()
            );
        }
    }

    // -------- Block 4: Cartões em aberto --------
    let cards_open: Vec<&CardSummaryRow> = data
        .cards
        .iter()
        .filter(|c| c.open_amount > Decimal::ZERO)
        .collect();
    if !cards_open.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", bold("Cartões em aberto"));
        for c in cards_open {
            let due_label = card_due_label(&data.accounts, &c.account_id, &c.month_ref, data.today);
            let acc_label = card_account_label(&data.accounts, &c.account_id);
            let _ = writeln!(
                out,
                "  💳 {} · {}{}",
                acc_label,
                hf_brl(c.open_amount),
                due_label.map(|s| format!(" ({s})")).unwrap_or_default(),
            );
        }
    }

    // -------- Block 5: Ações --------
    let mut actions: Vec<String> = Vec::new();
    if data.uncategorized_count > 0 {
        actions.push(format!(
            "{} lançamento{} sem categoria",
            data.uncategorized_count,
            if data.uncategorized_count == 1 {
                ""
            } else {
                "s"
            },
        ));
    }
    for b in &data.budget_alerts {
        let label = pretty_category_label(&b.category_id);
        actions.push(format!(
            "{} {}% do orçamento de {}",
            if b.alert { "🚨" } else { "⚠️" },
            b.usage_pct.round(),
            label,
        ));
    }
    if !actions.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", bold("Ação"));
        for a in actions {
            let _ = writeln!(out, "  • {}", a);
        }
    }

    out.trim_end().to_string()
}

/// User-friendly category label: prefer "Família · Sub" using known families.
/// Falls back to capitalized last segment.
fn pretty_category_label(category_id: &str) -> String {
    let fam = category_family(Some(category_id));
    let fam_label = fam.as_deref().map(family_label).unwrap_or_default();

    let sub = category_id
        .split(':')
        .nth(1)
        .map(|s| s.replace('-', " "))
        .map(|s| {
            // Capitalize words.
            s.split_whitespace()
                .map(|w| {
                    let mut chars = w.chars();
                    chars
                        .next()
                        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ")
        });

    match (fam_label.is_empty(), sub) {
        (false, Some(s)) if !s.is_empty() => format!("{fam_label} · {s}"),
        (false, _) => fam_label,
        (true, Some(s)) if !s.is_empty() => s,
        _ => category_id.to_string(),
    }
}

/// Card due-date label derived from `accounts.metadata_json.billing_due_day`
/// and the cycle's `month_ref`. Returns `"vence DD/mmm"` for upcoming due
/// dates and `"venceu DD/mmm"` for past-due bills so the message surfaces
/// overdue cycles instead of silently rolling them forward.
fn card_due_label(
    accounts: &[AccountRecord],
    account_id: &str,
    cycle_ref: &str,
    today: NaiveDate,
) -> Option<String> {
    let acc = accounts.iter().find(|a| a.account_id == account_id)?;
    let day_str = acc.metadata_json.get("billing_due_day")?.as_str()?;
    let day: u32 = day_str.parse().ok()?;
    if !(1..=31).contains(&day) {
        return None;
    }
    let parts: Vec<&str> = cycle_ref.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
    let y: i32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let last = days_in_month(NaiveDate::from_ymd_opt(y, m, 1)?);
    let due = NaiveDate::from_ymd_opt(y, m, day.min(last))?;
    let verb = if due < today { "venceu" } else { "vence" };
    Some(format!("{verb} {}", short_date(due)))
}

fn card_account_label(accounts: &[AccountRecord], account_id: &str) -> String {
    accounts
        .iter()
        .find(|a| a.account_id == account_id)
        .map(|a| {
            if a.label.is_empty() {
                a.account_id.clone()
            } else {
                a.label.clone()
            }
        })
        .unwrap_or_else(|| account_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn cf(month: &str, income: Decimal, expenses: Decimal) -> CashflowRow {
        CashflowRow {
            month_ref: month.into(),
            income,
            expenses,
            net: income - expenses,
        }
    }

    #[test]
    fn days_in_month_handles_feb_leap_and_dec() {
        assert_eq!(
            days_in_month(NaiveDate::from_ymd_opt(2024, 2, 15).unwrap()),
            29
        );
        assert_eq!(
            days_in_month(NaiveDate::from_ymd_opt(2025, 2, 15).unwrap()),
            28
        );
        assert_eq!(
            days_in_month(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()),
            31
        );
    }

    #[test]
    fn last_three_closed_months_wraps_year() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 10).unwrap();
        assert_eq!(
            last_three_closed_months(today),
            [
                "2026-01".to_string(),
                "2025-12".to_string(),
                "2025-11".to_string()
            ]
        );
    }

    #[test]
    fn closing_plan_on_track_when_projected_positive() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 5).unwrap();
        let data = PulseData {
            today,
            mtd: cf("2026-05", dec!(5000), dec!(1000)),
            baseline_t3m: cf("T3M", dec!(20000), dec!(15000)),
            current_by_category: BTreeMap::new(),
            baseline_by_category: BTreeMap::new(),
            upcoming: vec![],
            upcoming_total: dec!(2000),
            cards: vec![],
            accounts: vec![],
            budget_alerts: vec![],
            uncategorized_count: 0,
        };
        let plan = compute_closing_plan(&data);
        assert!(matches!(plan.status, ClosingStatus::OnTrack));
        assert!(plan.projected_eom_net > Decimal::ZERO);
    }

    #[test]
    fn closing_plan_stretched_when_fixed_exceeds_income() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 5).unwrap();
        let data = PulseData {
            today,
            mtd: cf("2026-05", dec!(2000), dec!(1000)),
            baseline_t3m: cf("T3M", dec!(10000), dec!(15000)),
            current_by_category: BTreeMap::new(),
            baseline_by_category: BTreeMap::new(),
            upcoming: vec![],
            upcoming_total: dec!(20000),
            cards: vec![],
            accounts: vec![],
            budget_alerts: vec![],
            uncategorized_count: 0,
        };
        let plan = compute_closing_plan(&data);
        assert!(matches!(plan.status, ClosingStatus::Stretched));
    }

    #[test]
    fn closing_plan_flags_category_overspend() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 15).unwrap(); // half month
        let mut current = BTreeMap::new();
        // Spent R$1000 on alimentacao MtD with baseline R$1000 → pro-rata 500.
        current.insert("alimentacao:mercado".to_string(), (dec!(1000), 8));
        let mut baseline = BTreeMap::new();
        baseline.insert("alimentacao:mercado".to_string(), dec!(1000));
        let data = PulseData {
            today,
            mtd: cf("2026-05", dec!(5000), dec!(1000)),
            baseline_t3m: cf("T3M", dec!(20000), dec!(15000)),
            current_by_category: current,
            baseline_by_category: baseline,
            upcoming: vec![],
            upcoming_total: Decimal::ZERO,
            cards: vec![],
            accounts: vec![],
            budget_alerts: vec![],
            uncategorized_count: 0,
        };
        let plan = compute_closing_plan(&data);
        assert_eq!(plan.categories_to_brake.len(), 1);
        let over = &plan.categories_to_brake[0];
        assert_eq!(over.category_id, "alimentacao:mercado");
        assert!(over.over > Decimal::ZERO);
    }

    #[test]
    fn pretty_category_label_handles_hierarchy() {
        assert_eq!(
            pretty_category_label("alimentacao:mercado"),
            "Alimentação · Mercado"
        );
        assert_eq!(pretty_category_label("moradia"), "Moradia");
    }

    #[test]
    fn render_pulse_contains_headline_blocks() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();
        let data = PulseData {
            today,
            mtd: cf("2026-05", dec!(10000), dec!(7000)),
            baseline_t3m: cf("T3M", dec!(20000), dec!(18000)),
            current_by_category: BTreeMap::new(),
            baseline_by_category: BTreeMap::new(),
            upcoming: vec![],
            upcoming_total: Decimal::ZERO,
            cards: vec![],
            accounts: vec![],
            budget_alerts: vec![],
            uncategorized_count: 3,
        };
        let plan = compute_closing_plan(&data);
        let rendered = render_pulse(&data, &plan, 1);
        assert!(rendered.contains("Pulso"));
        assert!(rendered.contains("Mês até dia"));
        assert!(rendered.contains("sem categoria"));
    }
}
