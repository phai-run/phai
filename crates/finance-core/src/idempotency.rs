use crate::models::{AccountRecord, ForecastRecord, RuleRecord, TransactionRecord};
use crate::split_payload::decimal_to_cents_exact;
use anyhow::{Context, Result};
use chrono::NaiveDate;
use deunicode::deunicode;
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn slugify(value: &str) -> String {
    let ascii = deunicode(value);
    let mut out = String::with_capacity(ascii.len());
    let mut last_dash = false;

    for ch in ascii.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }

    out.trim_matches('-').to_string()
}

pub fn hash_key(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn optional_str_part(value: Option<&str>) -> &str {
    value.unwrap_or("")
}

fn decimal_cents_part(value: Decimal, field_name: &str) -> Result<String> {
    Ok(decimal_to_cents_exact(value, field_name)?.to_string())
}

fn optional_decimal_part(value: Option<Decimal>) -> String {
    match value {
        Some(amount) => amount.normalize().to_string(),
        None => String::new(),
    }
}

pub fn split_line_hash(
    transaction_id: &str,
    line_index: i64,
    line_id: Option<&str>,
    description: Option<&str>,
    amount: Decimal,
) -> Result<String> {
    let amount_cents = decimal_cents_part(amount, "amount da linha de split")?;
    let parts = [
        transaction_id.to_string(),
        line_index.to_string(),
        optional_str_part(line_id).to_string(),
        slugify(optional_str_part(description)),
        amount_cents,
    ];
    let part_refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Ok(hash_key(&part_refs))
}

pub fn split_line_idempotency(transaction_id: &str, line_hash: &str) -> String {
    format!("split-line:{transaction_id}:{}", &line_hash[..16])
}

#[allow(clippy::too_many_arguments)]
pub fn split_item_hash(
    transaction_id: &str,
    line_index: i64,
    item_index: i64,
    item_code: Option<&str>,
    description: Option<&str>,
    quantity: Option<Decimal>,
    unit_price: Option<Decimal>,
    amount: Option<Decimal>,
) -> Result<String> {
    let quantity_raw = optional_decimal_part(quantity);
    let unit_price_raw = optional_decimal_part(unit_price);
    let item_amount_raw = optional_decimal_part(amount);
    let parts = [
        transaction_id.to_string(),
        line_index.to_string(),
        item_index.to_string(),
        optional_str_part(item_code).to_string(),
        slugify(optional_str_part(description)),
        quantity_raw,
        unit_price_raw,
        item_amount_raw,
    ];
    let part_refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Ok(hash_key(&part_refs))
}

pub fn split_item_idempotency(transaction_id: &str, item_hash: &str) -> String {
    format!("split-item:{transaction_id}:{}", &item_hash[..16])
}

pub fn pluggy_transaction_idempotency(transaction_id: &str) -> String {
    format!("pluggy:{transaction_id}")
}

pub fn manual_transaction_idempotency(actor_id: &str) -> String {
    format!("manual:{actor_id}:{}", Uuid::now_v7())
}

pub fn forecast_idempotency(actor_id: &str, description: &str, date: NaiveDate) -> String {
    let description_hash = hash_key(&[&slugify(description)]);
    format!(
        "forecast:{actor_id}:{}:{}",
        &description_hash[..16],
        date.format("%Y-%m-%d")
    )
}

pub fn rule_idempotency(slug: &str) -> String {
    format!("rule:{}", slugify(slug))
}

pub fn account_idempotency(account_id: &str) -> String {
    format!("account:{account_id}")
}

pub fn ensure_transaction_idempotency(row: &mut TransactionRecord) {
    if row.idempotency_key.trim().is_empty() {
        row.idempotency_key = if row.source == "pluggy" {
            pluggy_transaction_idempotency(&row.transaction_id)
        } else {
            manual_transaction_idempotency(&row.actor_id)
        };
    }
}

pub fn ensure_account_idempotency(row: &mut AccountRecord) {
    if row.idempotency_key.trim().is_empty() {
        row.idempotency_key = account_idempotency(&row.account_id);
    }
}

pub fn ensure_rule_idempotency(row: &mut RuleRecord) {
    if row.idempotency_key.trim().is_empty() {
        row.idempotency_key = rule_idempotency(&row.rule_id);
    }
}

pub fn ensure_forecast_idempotency(row: &mut ForecastRecord) -> Result<()> {
    if row.idempotency_key.trim().is_empty() {
        row.idempotency_key = forecast_idempotency(
            &row.actor_id,
            &row.description,
            row.due_date.context("due_date ausente para forecast")?,
        );
    }
    Ok(())
}

pub fn category_id(category: &str, subcategory: Option<&str>) -> String {
    let base = slugify(category);
    match subcategory {
        Some(value) if !value.trim().is_empty() => format!("{base}:{}", slugify(value)),
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn pluggy_transaction_key_is_stable() {
        assert_eq!(
            pluggy_transaction_idempotency("abc-123"),
            "pluggy:abc-123".to_string()
        );
    }

    #[test]
    fn forecast_key_is_deterministic() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 27).unwrap();
        let key_a = forecast_idempotency("test-actor", "Consulta recorrente", date);
        let key_b = forecast_idempotency("test-actor", "Consulta recorrente", date);
        assert_eq!(key_a, key_b);
    }

    #[test]
    fn category_id_slugifies_hierarchy() {
        assert_eq!(
            category_id("Saúde", Some("Consulta Médica")),
            "saude:consulta-medica".to_string()
        );
        assert_eq!(category_id("Moradia", None), "moradia".to_string());
    }

    #[test]
    fn category_id_collapses_runs_of_non_alnum() {
        // Regression: an older slugifier rendered " / " as three dashes and
        // produced `assinaturas:cloud---storage`. The current implementation
        // collapses any run of non-alphanumerics into a single dash, so any
        // visible input like "IA / Produtividade" or "Cloud  /  Storage"
        // converges on the same key.
        for variant in [
            "IA / Produtividade",
            "IA/Produtividade",
            "IA  /  Produtividade",
            "IA - Produtividade",
        ] {
            assert_eq!(
                category_id("Assinaturas", Some(variant)),
                "assinaturas:ia-produtividade".to_string(),
                "variant {variant:?} should slugify identically"
            );
        }
        assert!(
            !category_id("Foo", Some("Bar / Baz")).contains("---"),
            "slugifier must never produce ---"
        );
    }

    #[test]
    fn split_line_hash_is_deterministic() {
        let hash_a = split_line_hash(
            "tx-123",
            0,
            Some("line-a"),
            Some("Combustível"),
            Decimal::new(1300, 2),
        )
        .unwrap();
        let hash_b = split_line_hash(
            "tx-123",
            0,
            Some("line-a"),
            Some("Combustível"),
            Decimal::new(1300, 2),
        )
        .unwrap();
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn split_line_hash_changes_when_line_changes() {
        let hash_a = split_line_hash(
            "tx-123",
            0,
            Some("line-a"),
            Some("Combustível"),
            Decimal::new(1300, 2),
        )
        .unwrap();
        let hash_b = split_line_hash(
            "tx-123",
            1,
            Some("line-b"),
            Some("Lavagem"),
            Decimal::new(1300, 2),
        )
        .unwrap();
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn split_line_hash_rejects_fraction_of_cent() {
        let err = split_line_hash("tx-123", 0, None, None, Decimal::new(1001, 3)).unwrap_err();
        assert!(err.to_string().contains("2 casas decimais"));
    }

    #[test]
    fn split_item_hash_is_deterministic() {
        let hash_a = split_item_hash(
            "tx-123",
            0,
            0,
            Some("sku-1"),
            Some("Gasolina"),
            Some(Decimal::new(401, 1)),
            Some(Decimal::new(300, 2)),
            Some(Decimal::new(12030, 2)),
        )
        .unwrap();
        let hash_b = split_item_hash(
            "tx-123",
            0,
            0,
            Some("sku-1"),
            Some("Gasolina"),
            Some(Decimal::new(401, 1)),
            Some(Decimal::new(300, 2)),
            Some(Decimal::new(12030, 2)),
        )
        .unwrap();
        assert_eq!(hash_a, hash_b);
    }
}
