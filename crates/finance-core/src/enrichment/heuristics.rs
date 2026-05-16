//! Cheap heuristic features computed before any LLM call.
//!
//! These features feed the prompt as labelled signals:
//!   - "round" amounts (R$50, R$100) often mean splits / freelancers
//!   - hour buckets correlate with category (e.g. madrugada → app delivery)
//!   - weekday matters for weekly recurring services
//!   - "is_recurring" is the most predictive signal we have for a
//!     personal-spending model.

use super::types::{Heuristics, HourBucket};
use crate::storage::FinanceStore;
use chrono::Weekday;
use rust_decimal::Decimal;

/// `true` iff `amount.abs()` is integer-valued OR a multiple of 10. In
/// practice both conditions reduce to "round enough to be a manual
/// split", so the second collapses into the first for integers ≥ 10.
pub fn round_number_flag(amount: Decimal) -> bool {
    let abs = amount.abs();
    if abs.is_zero() {
        // Zero is not a useful signal.
        return false;
    }
    if abs.fract().is_zero() {
        return true;
    }
    // Check multiples of 10 with decimals (e.g. 10.00 already caught
    // above; nothing else qualifies because fract is non-zero).
    false
}

/// Bucket a 0..=23 hour-of-day into morning/afternoon/etc. Values out
/// of range fall into `Madrugada` (mod 24 would lie about input
/// validity).
pub fn hour_bucket(hour: u32) -> HourBucket {
    match hour {
        0..=5 => HourBucket::Madrugada,
        6..=11 => HourBucket::Manha,
        12..=17 => HourBucket::Tarde,
        18..=23 => HourBucket::Noite,
        _ => HourBucket::Madrugada,
    }
}

/// Compute the static portion of the heuristics struct. `is_recurring`
/// must be filled in separately by the pipeline (it requires an async
/// store lookup).
pub fn base_heuristics(amount: Decimal, hour: Option<u32>, weekday: Weekday) -> Heuristics {
    Heuristics {
        is_round_number: round_number_flag(amount),
        hour_bucket: hour.map(hour_bucket),
        weekday,
        is_recurring: false,
    }
}

/// Recurrence detector. Searches `similar_transactions` for entries
/// with the same merchant token (`doc`, typically a CNPJ or short
/// merchant slug) and an amount within ±20%, returning `true` if at
/// least two matches exist outside the current transaction.
pub async fn detect_recurring(
    store: &dyn FinanceStore,
    doc: Option<&str>,
    amount: Decimal,
    exclude_id: &str,
) -> bool {
    let Some(keyword) = doc else {
        return false;
    };
    if keyword.trim().is_empty() {
        return false;
    }
    let matches = match store
        .similar_transactions(keyword, exclude_id, /* only_uncategorized = */ false)
        .await
    {
        Ok(rows) => rows,
        Err(err) => {
            eprintln!("aviso: similar_transactions falhou ao detectar recorrência: {err:#}");
            return false;
        }
    };
    let target = amount.abs();
    if target.is_zero() {
        return false;
    }
    let low = target * Decimal::new(80, 2); // 0.80
    let high = target * Decimal::new(120, 2); // 1.20
    let count = matches
        .iter()
        .filter(|tx| tx.transaction_id != exclude_id)
        .filter(|tx| {
            let abs = tx.amount.abs();
            abs >= low && abs <= high
        })
        .count();
    count >= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn test_round_number_integer() {
        assert!(round_number_flag(d("100")));
        assert!(round_number_flag(d("50.00")));
        assert!(round_number_flag(d("-25")));
        assert!(round_number_flag(d("7")));
    }

    #[test]
    fn test_round_number_decimal() {
        assert!(!round_number_flag(d("31.90")));
        assert!(!round_number_flag(d("-25.99")));
        assert!(!round_number_flag(d("0.50")));
    }

    #[test]
    fn test_round_number_zero_is_not_round() {
        assert!(!round_number_flag(d("0")));
    }

    #[test]
    fn test_hour_bucket_madrugada() {
        assert_eq!(hour_bucket(0), HourBucket::Madrugada);
        assert_eq!(hour_bucket(5), HourBucket::Madrugada);
    }

    #[test]
    fn test_hour_bucket_manha() {
        assert_eq!(hour_bucket(6), HourBucket::Manha);
        assert_eq!(hour_bucket(11), HourBucket::Manha);
    }

    #[test]
    fn test_hour_bucket_tarde() {
        assert_eq!(hour_bucket(12), HourBucket::Tarde);
        assert_eq!(hour_bucket(17), HourBucket::Tarde);
    }

    #[test]
    fn test_hour_bucket_noite() {
        assert_eq!(hour_bucket(18), HourBucket::Noite);
        assert_eq!(hour_bucket(23), HourBucket::Noite);
    }

    #[test]
    fn test_hour_bucket_out_of_range() {
        // Out-of-range gets stuck in madrugada (defensive, no panic).
        assert_eq!(hour_bucket(24), HourBucket::Madrugada);
        assert_eq!(hour_bucket(999), HourBucket::Madrugada);
    }

    #[test]
    fn test_base_heuristics_fills_round_and_weekday() {
        let h = base_heuristics(d("50"), Some(13), Weekday::Sat);
        assert!(h.is_round_number);
        assert_eq!(h.hour_bucket, Some(HourBucket::Tarde));
        assert_eq!(h.weekday, Weekday::Sat);
        assert!(!h.is_recurring);
    }
}
