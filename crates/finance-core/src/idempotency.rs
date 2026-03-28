use crate::models::{AccountRecord, ForecastRecord, RuleRecord, TransactionRecord};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use deunicode::deunicode;
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
        hasher.update(part.as_bytes());
        hasher.update(b"|");
    }
    hex::encode(hasher.finalize())
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
        let key_a = forecast_idempotency("ford", "Consulta recorrente", date);
        let key_b = forecast_idempotency("ford", "Consulta recorrente", date);
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
}
