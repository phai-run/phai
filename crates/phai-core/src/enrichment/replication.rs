//! Anatomy replication: copy human-curated `description` and `purpose`
//! from a prior transaction with the same merchant or raw description.
//!
//! When a new transaction arrives from Pluggy, `description` and `purpose`
//! are always NULL. LLM enrichment sets `merchant_name` but not those
//! fields. For recurring merchants, this module propagates the
//! human-curated anatomy from the best-matching prior transaction.
//! When historical rows have not been enriched with `merchant_name`, the
//! raw Pluggy description is used as a conservative fallback key.
//!
//! Selection criteria (in priority order):
//!   1. Same `category_id` as the target transaction.
//!   2. Amount within ±20% of the target.
//!   3. Most recent (donors are pre-ordered by `transaction_date DESC`).
//!
//! Only NULL fields in the target are filled — already-set values are
//! never overwritten. The [`AnatomyReplication`] value carries exactly
//! which fields will be written, so callers can apply them with a
//! single `update_transaction_anatomy` patch and emit one audit event.

use crate::models::TransactionRecord;
use crate::storage::FinanceStore;
use anyhow::Result;
use rust_decimal::Decimal;

/// What the replication engine decided to copy from a donor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnatomyReplication {
    /// The transaction whose anatomy was chosen as the source.
    pub donor_id: String,
    /// Description to write to the target (only when target had `None`).
    pub description: Option<String>,
    /// Purpose to write to the target (only when target had `None`).
    pub purpose: Option<String>,
}

/// Outcome of a replication attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicationOutcome {
    /// At least one field was replicated from a donor.
    Replicated(AnatomyReplication),
    /// The transaction has no `merchant_name` — nothing to match on.
    NoMerchant,
    /// No suitable donor was found (no prior transaction with that merchant
    /// carries a human-curated description or purpose).
    NoDonor,
    /// Both `description` and `purpose` are already set on the target —
    /// nothing to replicate.
    AlreadyComplete,
}

/// Select the best donor from `candidates` for a target with the given
/// `category_id` and `amount`.
///
/// Scoring (additive):
///   +2  same `category_id` as target
///   +1  amount within ±20% of target
///
/// Candidates are assumed to be ordered by `transaction_date DESC` so
/// the most-recent donor wins ties. `max_by_key` picks the last element
/// among equals, so we reverse to ensure recency wins ties on equal score.
pub fn select_donor<'a>(
    candidates: &'a [TransactionRecord],
    target_category_id: Option<&str>,
    target_amount: Decimal,
) -> Option<&'a TransactionRecord> {
    candidates
        .iter()
        .rev() // most-recent last so max_by_key returns the most-recent among ties
        .filter_map(|d| {
            let same_category = d.category_id.as_deref() == target_category_id;
            let close_amount = amount_within_tolerance(d.amount, target_amount);
            let score = (same_category as u8) * 2 + (close_amount as u8);
            (score > 0).then_some((d, score))
        })
        .max_by_key(|(_, score)| *score)
        .map(|(d, _)| d)
}

fn amount_within_tolerance(donor: Decimal, target: Decimal) -> bool {
    if target.is_zero() {
        return donor.is_zero();
    }
    let diff = (donor - target).abs();
    let threshold = target.abs() * Decimal::new(20, 2); // 20%
    diff <= threshold
}

/// Compute what should be replicated given a set of pre-fetched donor
/// candidates. This is the pure core of the replication logic: it takes
/// no I/O and is easy to unit-test.
///
/// `current_description` / `current_purpose` represent the target's
/// current (possibly `None`) values. Only `None` fields are filled.
pub fn compute_replication(
    match_key: Option<&str>,
    current_description: Option<&str>,
    current_purpose: Option<&str>,
    target_category_id: Option<&str>,
    target_amount: Decimal,
    candidates: &[TransactionRecord],
) -> ReplicationOutcome {
    if match_key.map(str::trim).unwrap_or("").is_empty() {
        return ReplicationOutcome::NoMerchant;
    }
    let current_description = non_blank(current_description);
    let current_purpose = non_blank(current_purpose);
    if current_description.is_some() && current_purpose.is_some() {
        return ReplicationOutcome::AlreadyComplete;
    }
    let donor = match select_donor(candidates, target_category_id, target_amount) {
        Some(d) => d,
        None => return ReplicationOutcome::NoDonor,
    };
    let description = current_description
        .is_none()
        .then(|| {
            donor
                .description
                .as_deref()
                .and_then(|value| non_blank(Some(value)))
                .map(str::to_string)
        })
        .flatten();
    let purpose = current_purpose
        .is_none()
        .then(|| {
            donor
                .purpose
                .as_deref()
                .and_then(|value| non_blank(Some(value)))
                .map(str::to_string)
        })
        .flatten();
    if description.is_none() && purpose.is_none() {
        // Donor exists but has no anatomy we can use.
        return ReplicationOutcome::NoDonor;
    }
    ReplicationOutcome::Replicated(AnatomyReplication {
        donor_id: donor.transaction_id.clone(),
        description,
        purpose,
    })
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn anatomy_match_key(tx: &TransactionRecord) -> Option<&str> {
    non_blank(tx.merchant_name.as_deref()).or_else(|| non_blank(Some(&tx.raw_description)))
}

/// Async entry point: fetch donor candidates from the store, then run
/// [`compute_replication`]. Use this for the batch command and any other
/// callers that already have a full [`TransactionRecord`].
pub async fn find_and_replicate(
    store: &dyn FinanceStore,
    tx: &TransactionRecord,
) -> Result<ReplicationOutcome> {
    let match_key = match anatomy_match_key(tx) {
        Some(value) => value,
        None => {
            return Ok(ReplicationOutcome::NoMerchant);
        }
    };
    if match_key.is_empty() {
        return Ok(ReplicationOutcome::NoMerchant);
    }
    let candidates = store
        .find_anatomy_donors(match_key, &tx.transaction_id)
        .await?;
    Ok(compute_replication(
        Some(match_key),
        tx.description.as_deref(),
        tx.purpose.as_deref(),
        tx.category_id.as_deref(),
        tx.amount,
        &candidates,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    fn record(
        id: &str,
        category: &str,
        cents: i64,
        desc: Option<&str>,
        purpose: Option<&str>,
    ) -> TransactionRecord {
        TransactionRecord {
            transaction_id: id.to_string(),
            account_id: None,
            transaction_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            raw_description: "RAW".to_string(),
            description: desc.map(str::to_string),
            merchant_name: Some("Sapiens".to_string()),
            purpose: purpose.map(str::to_string),
            amount: Decimal::new(cents, 2),
            tx_type: "debit".to_string(),
            category_id: Some(category.to_string()),
            category_source: "manual".to_string(),
            context: None,
            classifier_trace: None,
            payment_status: "posted".to_string(),
            source: "pluggy".to_string(),
            actor_id: "u".to_string(),
            idempotency_key: "k".to_string(),
            metadata_json: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            enrichment_attempted_at: None,
            amount_cents: None,
        }
    }

    #[test]
    fn test_compute_replication_treats_blank_fields_as_missing() {
        let candidates = vec![record(
            "donor-1",
            "alimentacao:restaurantes",
            -5000,
            Some("  Almoço  "),
            Some("  lazer  "),
        )];
        let out = compute_replication(
            Some("Sapiens"),
            Some("  "),
            Some(""),
            Some("alimentacao:restaurantes"),
            Decimal::new(-5000, 2),
            &candidates,
        );
        match out {
            ReplicationOutcome::Replicated(rep) => {
                assert_eq!(rep.description.as_deref(), Some("Almoço"));
                assert_eq!(rep.purpose.as_deref(), Some("lazer"));
            }
            other => panic!("expected Replicated, got {:?}", other),
        }
    }

    #[test]
    fn test_anatomy_match_key_falls_back_to_raw_description() {
        let mut tx = record("target", "alimentacao:restaurantes", -5000, None, None);
        tx.merchant_name = None;
        tx.raw_description = "  Loja Exemplo  ".to_string();

        assert_eq!(anatomy_match_key(&tx), Some("Loja Exemplo"));
    }

    #[test]
    fn test_select_donor_prefers_same_category() {
        let candidates = vec![
            record(
                "a",
                "alimentacao:restaurantes",
                -5000,
                Some("Almoço"),
                Some("lazer"),
            ),
            record(
                "b",
                "saude:medicina",
                -5000,
                Some("Consulta"),
                Some("saude"),
            ),
        ];
        let donor = select_donor(
            &candidates,
            Some("alimentacao:restaurantes"),
            Decimal::new(-5000, 2),
        );
        assert_eq!(donor.unwrap().transaction_id, "a");
    }

    #[test]
    fn test_select_donor_uses_amount_as_tiebreaker() {
        let candidates = vec![
            record("a", "alimentacao:restaurantes", -5000, Some("Almoço"), None),
            record("b", "alimentacao:restaurantes", -9000, Some("Jantar"), None),
        ];
        // Target is -50.00 → "a" at -50.00 is within 20%; "b" at -90.00 is not
        let donor = select_donor(
            &candidates,
            Some("alimentacao:restaurantes"),
            Decimal::new(-5000, 2),
        );
        assert_eq!(donor.unwrap().transaction_id, "a");
    }

    #[test]
    fn test_select_donor_returns_none_for_empty_candidates() {
        let result = select_donor(&[], Some("alimentacao:restaurantes"), Decimal::new(-100, 2));
        assert!(result.is_none());
    }

    #[test]
    fn test_select_donor_rejects_zero_score_candidates() {
        let candidates = vec![record(
            "unrelated",
            "compras:eletronicos",
            -700_000,
            Some("Monitor"),
            Some("trabalho"),
        )];
        let donor = select_donor(&candidates, Some("educacao:livros"), Decimal::new(-3500, 2));
        assert!(donor.is_none());
    }

    #[test]
    fn test_compute_replication_no_merchant() {
        let candidates = vec![record("a", "x", -100, Some("Almoço"), None)];
        let out = compute_replication(None, None, None, None, Decimal::ZERO, &candidates);
        assert_eq!(out, ReplicationOutcome::NoMerchant);

        let out2 = compute_replication(Some("  "), None, None, None, Decimal::ZERO, &candidates);
        assert_eq!(out2, ReplicationOutcome::NoMerchant);
    }

    #[test]
    fn test_compute_replication_already_complete() {
        let candidates = vec![record("a", "x", -100, Some("Almoço"), Some("lazer"))];
        let out = compute_replication(
            Some("Sapiens"),
            Some("já tem desc"),
            Some("já tem purpose"),
            None,
            Decimal::new(-100, 2),
            &candidates,
        );
        assert_eq!(out, ReplicationOutcome::AlreadyComplete);
    }

    #[test]
    fn test_compute_replication_no_donor() {
        let out = compute_replication(
            Some("Sapiens"),
            None,
            None,
            Some("alimentacao:restaurantes"),
            Decimal::new(-100, 2),
            &[],
        );
        assert_eq!(out, ReplicationOutcome::NoDonor);
    }

    #[test]
    fn test_compute_replication_copies_both_fields() {
        let candidates = vec![record(
            "donor-1",
            "alimentacao:restaurantes",
            -5000,
            Some("Almoço"),
            Some("lazer"),
        )];
        let out = compute_replication(
            Some("Sapiens"),
            None,
            None,
            Some("alimentacao:restaurantes"),
            Decimal::new(-5000, 2),
            &candidates,
        );
        match out {
            ReplicationOutcome::Replicated(rep) => {
                assert_eq!(rep.donor_id, "donor-1");
                assert_eq!(rep.description.as_deref(), Some("Almoço"));
                assert_eq!(rep.purpose.as_deref(), Some("lazer"));
            }
            other => panic!("expected Replicated, got {:?}", other),
        }
    }

    #[test]
    fn test_compute_replication_copies_only_missing_fields() {
        let candidates = vec![record(
            "donor-1",
            "alimentacao:restaurantes",
            -5000,
            Some("Almoço"),
            Some("lazer"),
        )];
        // Target already has purpose
        let out = compute_replication(
            Some("Sapiens"),
            None,
            Some("já tem purpose"),
            Some("alimentacao:restaurantes"),
            Decimal::new(-5000, 2),
            &candidates,
        );
        match out {
            ReplicationOutcome::Replicated(rep) => {
                assert_eq!(rep.description.as_deref(), Some("Almoço"));
                assert!(
                    rep.purpose.is_none(),
                    "should not overwrite existing purpose"
                );
            }
            other => panic!("expected Replicated, got {:?}", other),
        }
    }

    #[test]
    fn test_compute_replication_no_donor_when_donor_has_no_anatomy() {
        // Donor has no description or purpose to give
        let candidates = vec![record(
            "donor-1",
            "alimentacao:restaurantes",
            -5000,
            None,
            None,
        )];
        let out = compute_replication(
            Some("Sapiens"),
            None,
            None,
            Some("alimentacao:restaurantes"),
            Decimal::new(-5000, 2),
            &candidates,
        );
        assert_eq!(out, ReplicationOutcome::NoDonor);
    }

    #[test]
    fn test_amount_within_tolerance() {
        // 50.00 ± 20% → [40.00, 60.00]
        assert!(amount_within_tolerance(
            Decimal::new(5000, 2),
            Decimal::new(5000, 2)
        ));
        assert!(amount_within_tolerance(
            Decimal::new(4001, 2),
            Decimal::new(5000, 2)
        ));
        assert!(amount_within_tolerance(
            Decimal::new(5999, 2),
            Decimal::new(5000, 2)
        ));
        assert!(!amount_within_tolerance(
            Decimal::new(3999, 2),
            Decimal::new(5000, 2)
        ));
        assert!(!amount_within_tolerance(
            Decimal::new(6001, 2),
            Decimal::new(5000, 2)
        ));
        // Zero target: only matches zero donor
        assert!(amount_within_tolerance(Decimal::ZERO, Decimal::ZERO));
        assert!(!amount_within_tolerance(Decimal::new(1, 2), Decimal::ZERO));
    }
}
