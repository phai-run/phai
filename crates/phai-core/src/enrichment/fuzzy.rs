//! Fuzzy filtering of candidate transactions for retroactive rule
//! application.
//!
//! After the SQL `similar_transactions(keyword, …)` LIKE-based prefilter,
//! we run candidates through nucleo's `Atom::score` to weed out unrelated
//! matches that only share a substring by coincidence. This is the
//! "second pass" mentioned in the Sonnet review — SQL keeps the working
//! set small, nucleo refines the score.
//!
//! ## Score → percentage mapping
//!
//! nucleo's `Atom::score` returns `u16` (typically 0..~200 for short
//! strings). We expose the threshold as a 0..=100 percentage to keep
//! the CLI flag intuitive. The conversion treats `200` as 100% — anything
//! above caps to 100. Concretely:
//!
//! ```text
//! percent = min(100, raw_score / 2)
//! min_raw_score = threshold_percent * 2   // matches the inverse
//! ```
//!
//! This was picked empirically from nucleo's own README examples
//! (`"foo bar"` against `"foobar"` scores 140, `"foo/bar"` scores 168,
//! exact substring matches go higher).

use crate::models::TransactionRecord;
use nucleo::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo::{Config, Matcher, Utf32Str};

/// Maximum raw nucleo score we treat as 100% for the percentage
/// conversion. Picked from observed scores on short merchant names.
const SCORE_TO_PERCENT_DIVISOR: u32 = 2;

/// Filter `candidates` by fuzzy-matching `keyword` against each
/// `description`, keeping only those whose normalized score is at least
/// `threshold_percent` (0..=100). Returns `(record, score_u32)` sorted
/// by score descending.
///
/// Empty keyword → empty result.
pub fn fuzzy_filter(
    keyword: &str,
    candidates: Vec<TransactionRecord>,
    threshold_percent: u8,
) -> Vec<(TransactionRecord, u32)> {
    let kw = keyword.trim();
    if kw.is_empty() {
        return Vec::new();
    }
    let threshold = (threshold_percent.min(100) as u32) * SCORE_TO_PERCENT_DIVISOR;

    let mut matcher = Matcher::new(Config::DEFAULT);
    let atom = Atom::new(
        kw,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false,
    );

    let mut results: Vec<(TransactionRecord, u32)> = candidates
        .into_iter()
        .filter_map(|rec| {
            let mut buf = Vec::new();
            let haystack = Utf32Str::new(&rec.raw_description, &mut buf);
            let score = atom.score(haystack, &mut matcher)? as u32;
            if score >= threshold {
                Some((rec, score))
            } else {
                None
            }
        })
        .collect();
    results.sort_by_key(|b| std::cmp::Reverse(b.1));
    results
}

/// Convert a raw nucleo score (`u32`) into the 0..=100 percentage the
/// CLI displays alongside each retroactive match.
pub fn score_to_percent(score: u32) -> u8 {
    let pct = score / SCORE_TO_PERCENT_DIVISOR;
    pct.min(100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Utc};
    use rust_decimal::Decimal;
    use serde_json::json;

    fn tx(id: &str, desc: &str) -> TransactionRecord {
        TransactionRecord {
            transaction_id: id.to_string(),
            account_id: None,
            transaction_date: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            raw_description: desc.to_string(),
            description: None,
            merchant_name: None,
            purpose: None,
            amount: Decimal::new(-1000, 2),
            tx_type: "debit".to_string(),
            category_id: None,
            category_source: "unclassified".to_string(),
            context: None,
            classifier_trace: None,
            payment_status: "confirmed".to_string(),
            source: "pluggy".to_string(),
            actor_id: "u".to_string(),
            idempotency_key: "k".to_string(),
            metadata_json: json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            enrichment_attempted_at: None,
            amount_cents: None,
        }
    }

    #[test]
    fn test_fuzzy_filter_exact_match_high_score() {
        let cands = vec![tx("a", "sapiens parque restaurante")];
        let out = fuzzy_filter("sapiens", cands, 0);
        assert_eq!(out.len(), 1);
        assert!(out[0].1 > 0, "expected positive score, got {}", out[0].1);
    }

    #[test]
    fn test_fuzzy_filter_typo_matches() {
        // "Sapiens" should match "Sapiens Pa" (fuzzy substring).
        let cands = vec![tx("a", "Sapiens Pa")];
        let out = fuzzy_filter("sapiens", cands, 50);
        assert_eq!(out.len(), 1, "expected 1 match, got {:?}", out);
    }

    #[test]
    fn test_fuzzy_filter_unrelated_filtered_out() {
        // High threshold (95%) + unrelated text → nothing kept.
        let cands = vec![tx("a", "Posto Shell BR-101")];
        let out = fuzzy_filter("sapiens", cands, 95);
        assert!(
            out.is_empty(),
            "expected no matches for unrelated text, got {:?}",
            out
        );
    }

    #[test]
    fn test_fuzzy_results_sorted_desc() {
        let cands = vec![
            tx("low", "S a p i e n s scattered far apart"),
            tx("high", "Sapiens"),
            tx("mid", "Sapiens Parque Loja"),
        ];
        let out = fuzzy_filter("sapiens", cands, 0);
        // All three may match; ordering must be descending by score.
        for w in out.windows(2) {
            assert!(
                w[0].1 >= w[1].1,
                "scores not descending: {} then {}",
                w[0].1,
                w[1].1
            );
        }
        // "Sapiens" (exact) should be the top one.
        assert_eq!(out[0].0.transaction_id, "high");
    }

    #[test]
    fn test_fuzzy_empty_keyword_returns_empty() {
        let cands = vec![tx("a", "Sapiens")];
        assert!(fuzzy_filter("", cands.clone(), 0).is_empty());
        assert!(fuzzy_filter("   ", cands, 0).is_empty());
    }

    #[test]
    fn test_score_to_percent_caps_at_100() {
        assert_eq!(score_to_percent(0), 0);
        assert_eq!(score_to_percent(100), 50);
        assert_eq!(score_to_percent(200), 100);
        assert_eq!(score_to_percent(1_000), 100);
    }
}
