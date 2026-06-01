//! Canonical cash-flow bucketing — the single Rust source of truth for which
//! month a transaction's cash actually moves in (`cash_month`).
//!
//! This mirrors the SQL view `v_transactions_cashbasis` (migration 037) exactly
//! so Rust-side surfaces (the web bridge, reports) agree with the database. See
//! [ADR-0025](../../docs/adr/0025-cashflow-basis-bill-explosion.md):
//!
//! * non-card accounts → the transaction's own month (cash moves immediately);
//! * credit cards → the month the bill containing the purchase is due/paid,
//!   from `billing_closing_day` (which cycle it closes in: day ≤ closing stays
//!   in the current cycle) plus `billing_due_day` (a one-month roll when the due
//!   day precedes the closing day).

use chrono::{Datelike, NaiveDate};

/// Compute the canonical `cash_month` (`"%Y-%m"`) for a transaction.
///
/// `is_credit` is the account's `account_type == "credit"`. `closing_day` and
/// `due_day` come from the account's `billing_closing_day` / `billing_due_day`
/// metadata. A card without a closing day falls back to the posting month, as
/// do all non-card accounts.
pub fn cash_month_for(
    date: NaiveDate,
    is_credit: bool,
    closing_day: Option<u32>,
    due_day: Option<u32>,
) -> String {
    let Some(closing) = closing_day.filter(|_| is_credit) else {
        return format!("{:04}-{:02}", date.year(), date.month());
    };
    // Day after the closing day rolls into the cycle that closes next month;
    // a due day that precedes the closing day rolls payment one more month.
    let mut offset: i32 = if date.day() > closing { 1 } else { 0 };
    if let Some(due) = due_day {
        if due < closing {
            offset += 1;
        }
    }
    let zero_based = date.year() * 12 + date.month0() as i32 + offset;
    let year = zero_based.div_euclid(12);
    let month = zero_based.rem_euclid(12) as u32 + 1;
    format!("{year:04}-{month:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn non_card_keeps_posting_month() {
        assert_eq!(
            cash_month_for(d(2026, 5, 10), false, Some(3), Some(10)),
            "2026-05"
        );
        assert_eq!(cash_month_for(d(2026, 5, 10), false, None, None), "2026-05");
    }

    #[test]
    fn card_without_closing_day_keeps_posting_month() {
        assert_eq!(cash_month_for(d(2026, 5, 10), true, None, None), "2026-05");
    }

    #[test]
    fn card_purchase_after_closing_rolls_to_due_month() {
        // closing 3, due 10: a 2026-04-28 swipe closes in the May cycle, due May.
        assert_eq!(
            cash_month_for(d(2026, 4, 28), true, Some(3), Some(10)),
            "2026-05"
        );
    }

    #[test]
    fn card_purchase_on_or_before_closing_stays_in_cycle() {
        // closing 10, due 17: a 2026-04-05 swipe (<= 10) closes/pays in April.
        assert_eq!(
            cash_month_for(d(2026, 4, 5), true, Some(10), Some(17)),
            "2026-04"
        );
    }

    #[test]
    fn due_before_closing_rolls_extra_month() {
        // closing 25, due 5: a 2026-04-10 swipe closes April, due 2026-05-05.
        assert_eq!(
            cash_month_for(d(2026, 4, 10), true, Some(25), Some(5)),
            "2026-05"
        );
    }

    #[test]
    fn year_boundary_rolls_correctly() {
        // closing 3, due 10: a 2026-12-28 swipe closes in the Jan-2027 cycle.
        assert_eq!(
            cash_month_for(d(2026, 12, 28), true, Some(3), Some(10)),
            "2027-01"
        );
    }
}
