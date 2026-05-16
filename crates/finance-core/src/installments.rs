use crate::models::TransactionRecord;
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Identifies an installment marker in a transaction description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallmentMarker {
    /// Current installment number (1-based).
    pub current: u32,
    /// Total number of installments.
    pub total: u32,
    /// Description with the installment marker stripped and trimmed.
    pub base_description: String,
}

/// Parses an installment marker from a transaction description.
///
/// Recognized patterns (case-insensitive, checked in order):
/// 1. `"X/Y"` at end: e.g. `"ALGUMA COISA 3/12"`
/// 2. `"Parcela X de Y"` anywhere: e.g. `"Algo - Parcela 3 de 12"`
/// 3. `"X de Y"` at end: e.g. `"ALGUMA COISA 3 de 12"`
///
/// Returns `None` if no pattern matches or if the numbers are invalid
/// (X > Y, Y > 99, X == 0).
pub fn parse_installment_description(desc: &str) -> Option<InstallmentMarker> {
    parse_x_slash_y(desc)
        .or_else(|| parse_parcela_x_de_y(desc))
        .or_else(|| parse_x_de_y_at_end(desc))
}

fn validate_marker(current: u32, total: u32, base: &str) -> Option<InstallmentMarker> {
    if current == 0 || total == 0 || current > total || total > 99 {
        return None;
    }
    Some(InstallmentMarker {
        current,
        total,
        base_description: base.trim().to_string(),
    })
}

/// Parses `"X/Y"` at the end of the description.
fn parse_x_slash_y(desc: &str) -> Option<InstallmentMarker> {
    let trimmed = desc.trim();
    // Find last token that contains '/'
    let last_token = trimmed.split_whitespace().last()?;
    let slash_pos = last_token.find('/')?;
    let left = &last_token[..slash_pos];
    let right = &last_token[slash_pos + 1..];

    if left.is_empty() || right.is_empty() {
        return None;
    }
    if !left.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !right.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let current: u32 = left.parse().ok()?;
    let total: u32 = right.parse().ok()?;

    // Strip the last token from the description to get the base
    let base = trimmed[..trimmed.len() - last_token.len()].trim_end();
    validate_marker(current, total, base)
}

/// Parses `"Parcela X de Y"` anywhere in the description (case-insensitive).
fn parse_parcela_x_de_y(desc: &str) -> Option<InstallmentMarker> {
    let lower = desc.to_ascii_lowercase();
    // Find "parcela" in the lowercased string
    let parcela_pos = lower.find("parcela")?;
    let after_parcela = lower[parcela_pos + 7..].trim_start();

    // Expect a number
    let num_end = after_parcela
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, _)| i + 1)?;
    let current_str = &after_parcela[..num_end];
    if current_str.is_empty() {
        return None;
    }
    let current: u32 = current_str.parse().ok()?;

    // After number, expect " de " (case-insensitive, already lowercased)
    let after_num = after_parcela[num_end..].trim_start();
    if !after_num.starts_with("de ") {
        return None;
    }
    let after_de = after_num[3..].trim_start();

    // Read total number
    let total_end = after_de
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, _)| i + 1)?;
    let total_str = &after_de[..total_end];
    if total_str.is_empty() {
        return None;
    }
    let total: u32 = total_str.parse().ok()?;

    // Build base description by stripping the matched region from the original desc.
    // The matched region in `desc` starts at `parcela_pos`.
    // We strip from the character before "parcela" up to end of total.
    // Actually we need to find the boundaries in the original string.
    // The region to strip is from `parcela_pos` to the end of the total digits.
    let match_len_in_lower = {
        // parcela_pos + "parcela".len() + spaces + current_digits + " de " + spaces + total_digits
        let total_offset = (lower.len() - after_de.len()) + total_end;
        total_offset - parcela_pos
    };

    // Strip the "Parcela X de Y" segment (and surrounding separators)
    let before = &desc[..parcela_pos];
    let after_full = &desc[parcela_pos + match_len_in_lower..];

    // Clean up separators around the stripped portion
    let before_clean = before.trim_end_matches(|c: char| c == '-' || c == '–' || c.is_whitespace());
    let after_clean =
        after_full.trim_start_matches(|c: char| c == '-' || c == '–' || c.is_whitespace());

    let base = if before_clean.is_empty() {
        after_clean.to_string()
    } else if after_clean.is_empty() {
        before_clean.to_string()
    } else {
        format!("{before_clean} {after_clean}")
    };

    validate_marker(current, total, &base)
}

/// Parses `"X de Y"` at the end of the description (case-insensitive).
fn parse_x_de_y_at_end(desc: &str) -> Option<InstallmentMarker> {
    let trimmed = desc.trim();
    let lower = trimmed.to_ascii_lowercase();

    // Work backwards: find trailing digits (total), then " de ", then more digits (current)
    // We scan tokens from the end.
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let n = tokens.len();
    if n < 3 {
        return None;
    }

    // Last token should be all digits (total)
    let total_token = tokens[n - 1];
    if !total_token.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    // Second-to-last token should be "de" (case-insensitive)
    let de_token = tokens[n - 2].to_ascii_lowercase();
    if de_token != "de" {
        return None;
    }

    // Third-to-last token should be all digits (current)
    let current_token = tokens[n - 3];
    if !current_token.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let total: u32 = total_token.parse().ok()?;
    let current: u32 = current_token.parse().ok()?;

    // Build the base by stripping the last 3 tokens
    let base_tokens = &tokens[..n - 3];
    let base = base_tokens.join(" ");
    let _ = lower; // suppress unused warning

    validate_marker(current, total, &base)
}

/// A group of transaction records that form a single installment chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallmentChain {
    /// Account identifier shared by all installments.
    pub account_id: String,
    /// Normalized base description (with installment marker stripped).
    pub base_description: String,
    /// Total number of installments declared in descriptions.
    pub total: u32,
    /// Maximum `current` seen across all known installments.
    pub current: u32,
    /// All transaction records belonging to this chain (sorted by date).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub installments: Vec<TransactionRecord>,
    /// Date of the earliest known installment.
    pub first_date: NaiveDate,
    /// Projected end date: `first_date` + (`total` - 1) months.
    pub projected_end: NaiveDate,
    /// `total - current` installments still pending.
    pub remaining: u32,
    /// `true` when only one installment remains (next month completes the chain).
    pub released_next_month: bool,
    /// Sum of amounts across all known installments.
    pub total_amount: Decimal,
}

/// Groups a flat list of transactions into installment chains.
///
/// Transactions that do not match any installment pattern are silently skipped.
///
/// Grouping key: `(account_id, base_description, total)`.
pub fn group_into_chains(transactions: &[TransactionRecord]) -> Vec<InstallmentChain> {
    // Key: (account_id, base_description, total)
    type Key = (String, String, u32);

    let mut groups: BTreeMap<Key, Vec<TransactionRecord>> = BTreeMap::new();

    for tx in transactions {
        let marker = match parse_installment_description(&tx.description) {
            Some(m) => m,
            None => continue,
        };

        let account_id = tx.account_id.clone().unwrap_or_default();
        let key: Key = (account_id, marker.base_description.clone(), marker.total);
        groups.entry(key).or_default().push(tx.clone());
    }

    let mut chains: Vec<InstallmentChain> = groups
        .into_iter()
        .filter_map(
            |((account_id, base_description, total), mut installments)| {
                installments.sort_by_key(|tx| tx.transaction_date);

                let first_date = installments.first()?.transaction_date;

                // Find the maximum `current` among all records in this group.
                let current = installments
                    .iter()
                    .filter_map(|tx| {
                        parse_installment_description(&tx.description).map(|m| m.current)
                    })
                    .max()
                    .unwrap_or(0);

                if current == 0 {
                    return None;
                }

                let remaining = total.saturating_sub(current);
                let released_next_month = remaining == 1;

                // Projected end: first_date + (total - 1) months
                let projected_end = add_months(first_date, total.saturating_sub(1))?;

                let total_amount = installments
                    .iter()
                    .map(|tx| tx.amount)
                    .fold(Decimal::ZERO, |acc, v| acc + v);

                Some(InstallmentChain {
                    account_id,
                    base_description,
                    total,
                    current,
                    installments,
                    first_date,
                    projected_end,
                    remaining,
                    released_next_month,
                    total_amount,
                })
            },
        )
        .collect();

    // Sort: released_next_month descending, then projected_end ascending, then base_description
    chains.sort_by(|a, b| {
        b.released_next_month
            .cmp(&a.released_next_month)
            .then_with(|| a.projected_end.cmp(&b.projected_end))
            .then_with(|| a.base_description.cmp(&b.base_description))
    });

    chains
}

fn add_months(date: NaiveDate, months: u32) -> Option<NaiveDate> {
    let mut year = date.year();
    let mut month = date.month() as i32 + months as i32;
    while month > 12 {
        year += 1;
        month -= 12;
    }
    // Clamp the day to the last valid day of the target month
    let max_day = days_in_month(year, month as u32)?;
    let day = date.day().min(max_day);
    NaiveDate::from_ymd_opt(year, month as u32, day)
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    // First day of the next month minus one day
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_of_next = NaiveDate::from_ymd_opt(next_year, next_month, 1)?;
    let last = first_of_next.pred_opt()?;
    Some(last.day())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal::prelude::FromStr;
    use serde_json::json;

    // ── parse_installment_description tests ────────────────────────────────

    #[test]
    fn parse_x_slash_y_basic() {
        let m = parse_installment_description("ALGUMA COISA 3/12").unwrap();
        assert_eq!(m.current, 3);
        assert_eq!(m.total, 12);
        assert_eq!(m.base_description, "ALGUMA COISA");
    }

    #[test]
    fn parse_x_slash_y_at_end_single_word() {
        let m = parse_installment_description("PURCHASE 1/6").unwrap();
        assert_eq!(m.current, 1);
        assert_eq!(m.total, 6);
        assert_eq!(m.base_description, "PURCHASE");
    }

    #[test]
    fn parse_x_slash_y_first_installment() {
        let m = parse_installment_description("NOTEBOOK XYZ 1/12").unwrap();
        assert_eq!(m.current, 1);
        assert_eq!(m.total, 12);
    }

    #[test]
    fn parse_parcela_x_de_y_basic() {
        let m = parse_installment_description("Algo - Parcela 3 de 12").unwrap();
        assert_eq!(m.current, 3);
        assert_eq!(m.total, 12);
        assert_eq!(m.base_description, "Algo");
    }

    #[test]
    fn parse_parcela_x_de_y_case_insensitive() {
        let m = parse_installment_description("Something PARCELA 2 DE 6 extra").unwrap();
        assert_eq!(m.current, 2);
        assert_eq!(m.total, 6);
    }

    #[test]
    fn parse_x_de_y_at_end() {
        let m = parse_installment_description("ALGUMA COISA 3 de 12").unwrap();
        assert_eq!(m.current, 3);
        assert_eq!(m.total, 12);
        assert_eq!(m.base_description, "ALGUMA COISA");
    }

    #[test]
    fn parse_x_de_y_case_insensitive() {
        let m = parse_installment_description("ITEM 1 DE 6").unwrap();
        assert_eq!(m.current, 1);
        assert_eq!(m.total, 6);
        assert_eq!(m.base_description, "ITEM");
    }

    #[test]
    fn parse_no_match_plain_description() {
        assert!(parse_installment_description("Supermercado Angeloni").is_none());
    }

    #[test]
    fn parse_no_match_empty() {
        assert!(parse_installment_description("").is_none());
    }

    #[test]
    fn parse_invalid_x_greater_than_y() {
        assert!(parse_installment_description("THING 7/6").is_none());
    }

    #[test]
    fn parse_invalid_x_equals_zero() {
        assert!(parse_installment_description("THING 0/6").is_none());
    }

    #[test]
    fn parse_invalid_y_greater_than_99() {
        assert!(parse_installment_description("THING 1/100").is_none());
    }

    #[test]
    fn parse_invalid_y_equals_zero() {
        assert!(parse_installment_description("THING 1/0").is_none());
    }

    #[test]
    fn parse_x_slash_y_not_at_end_returns_none() {
        // "3/12" is in the middle — X/Y must be the last token to match pattern 1.
        // "parcela" is absent, "de" not followed by number at end.
        assert!(parse_installment_description("THING 3/12 extra words").is_none());
    }

    #[test]
    fn parse_parcela_in_middle() {
        let m = parse_installment_description("Loja XYZ Parcela 2 de 5 pagamento").unwrap();
        assert_eq!(m.current, 2);
        assert_eq!(m.total, 5);
    }

    // ── group_into_chains tests ────────────────────────────────────────────

    fn make_tx(id: &str, account_id: &str, date: &str, description: &str) -> TransactionRecord {
        TransactionRecord {
            transaction_id: id.to_string(),
            account_id: Some(account_id.to_string()),
            transaction_date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            description: description.to_string(),
            amount: Decimal::from_str("-100.00").unwrap(),
            tx_type: "DEBIT".to_string(),
            category_id: None,
            category_source: "manual".to_string(),
            context: None,
            payment_status: "posted".to_string(),
            source: "manual".to_string(),
            actor_id: "test".to_string(),
            idempotency_key: id.to_string(),
            metadata_json: json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            enrichment_attempted_at: None,
        }
    }

    #[test]
    fn group_empty_input() {
        let chains = group_into_chains(&[]);
        assert!(chains.is_empty());
    }

    #[test]
    fn group_single_chain_multiple_installments() {
        let txs = vec![
            make_tx("t3", "acc1", "2026-03-01", "PURCHASE XYZ 3/6"),
            make_tx("t1", "acc1", "2026-01-01", "PURCHASE XYZ 1/6"),
            make_tx("t2", "acc1", "2026-02-01", "PURCHASE XYZ 2/6"),
        ];
        let chains = group_into_chains(&txs);
        assert_eq!(chains.len(), 1);
        let chain = &chains[0];
        assert_eq!(chain.base_description, "PURCHASE XYZ");
        assert_eq!(chain.total, 6);
        assert_eq!(chain.current, 3);
        assert_eq!(chain.remaining, 3);
        assert!(!chain.released_next_month);
        // installments sorted by date
        assert_eq!(chain.installments[0].transaction_id, "t1");
        assert_eq!(chain.installments[1].transaction_id, "t2");
        assert_eq!(chain.installments[2].transaction_id, "t3");
        // projected_end: first_date(2026-01-01) + 5 months = 2026-06-01
        assert_eq!(
            chain.projected_end,
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
    }

    #[test]
    fn group_multiple_chains() {
        let txs = vec![
            make_tx("a1", "acc1", "2026-01-01", "ITEM A 1/3"),
            make_tx("b1", "acc1", "2026-01-01", "ITEM B 1/6"),
            make_tx("a2", "acc1", "2026-02-01", "ITEM A 2/3"),
        ];
        let chains = group_into_chains(&txs);
        assert_eq!(chains.len(), 2);
        let descriptions: Vec<&str> = chains.iter().map(|c| c.base_description.as_str()).collect();
        assert!(descriptions.contains(&"ITEM A"));
        assert!(descriptions.contains(&"ITEM B"));
    }

    #[test]
    fn group_chains_across_different_accounts() {
        let txs = vec![
            make_tx("x1", "acc1", "2026-01-01", "ITEM 1/3"),
            make_tx("x2", "acc2", "2026-01-01", "ITEM 1/3"),
        ];
        let chains = group_into_chains(&txs);
        assert_eq!(chains.len(), 2);
        let accounts: Vec<&str> = chains.iter().map(|c| c.account_id.as_str()).collect();
        assert!(accounts.contains(&"acc1"));
        assert!(accounts.contains(&"acc2"));
    }

    #[test]
    fn group_partial_chain_missing_some_installments() {
        // Only installment 5/6 present — still creates a chain
        let txs = vec![make_tx("p5", "acc1", "2026-05-01", "PHONE 5/6")];
        let chains = group_into_chains(&txs);
        assert_eq!(chains.len(), 1);
        let chain = &chains[0];
        assert_eq!(chain.current, 5);
        assert_eq!(chain.remaining, 1);
        assert!(chain.released_next_month);
    }

    #[test]
    fn group_no_installment_transactions_skipped() {
        let txs = vec![
            make_tx("plain", "acc1", "2026-01-01", "Supermercado"),
            make_tx("t1", "acc1", "2026-01-01", "ITEM 1/3"),
        ];
        let chains = group_into_chains(&txs);
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].installments.len(), 1);
    }

    #[test]
    fn group_complete_chain_has_remaining_zero() {
        let txs = vec![
            make_tx("q1", "acc1", "2026-01-01", "GADGET 1/3"),
            make_tx("q2", "acc1", "2026-02-01", "GADGET 2/3"),
            make_tx("q3", "acc1", "2026-03-01", "GADGET 3/3"),
        ];
        let chains = group_into_chains(&txs);
        assert_eq!(chains.len(), 1);
        let chain = &chains[0];
        assert_eq!(chain.remaining, 0);
        assert!(!chain.released_next_month);
    }

    #[test]
    fn group_sort_released_next_month_first() {
        let txs = vec![
            make_tx("z1", "acc1", "2026-01-01", "LONG ITEM 1/12"),
            make_tx("z2", "acc1", "2026-01-01", "SHORT ITEM 5/6"),
        ];
        let chains = group_into_chains(&txs);
        assert_eq!(chains.len(), 2);
        // SHORT ITEM 5/6 has remaining=1 => released_next_month=true, should be first
        assert_eq!(chains[0].base_description, "SHORT ITEM");
        assert!(chains[0].released_next_month);
    }
}
