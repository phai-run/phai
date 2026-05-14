use crate::idempotency::hash_key;
use crate::models::{decimal_from_str, TransactionRecord};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SplitPayload {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
    pub lines: Vec<SplitPayloadLine>,
    #[serde(default)]
    pub items: Vec<ReceiptItemPayload>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SplitPayloadLine {
    pub description: String,
    pub amount: Value,
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptItemPayload {
    pub description: String,
    #[serde(default)]
    pub quantity: Option<Value>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub unit_price: Option<Value>,
    #[serde(default)]
    pub total_price: Option<Value>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub store_name: Option<String>,
    #[serde(default)]
    pub split_line_index: Option<usize>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitPreview {
    pub parent_transaction_id: String,
    pub parent_amount: Decimal,
    pub split_total: Decimal,
    pub difference: Decimal,
    pub payload_hash: String,
    pub lines: Vec<ValidatedSplitLine>,
    pub items: Vec<ValidatedReceiptItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedSplitLine {
    pub line_index: i64,
    pub description: String,
    pub amount: Decimal,
    pub category_id: Option<String>,
    pub context: Option<String>,
    pub metadata_json: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedReceiptItem {
    pub item_index: i64,
    pub description: String,
    pub quantity: Option<Decimal>,
    pub unit: Option<String>,
    pub unit_price: Option<Decimal>,
    pub total_price: Option<Decimal>,
    pub code: Option<String>,
    pub store_name: Option<String>,
    pub split_line_index: Option<i64>,
    pub metadata_json: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionSplitRecord {
    pub split_id: String,
    pub parent_transaction_id: String,
    pub payload_hash: String,
    pub status: String,
    pub source: String,
    pub notes: Option<String>,
    pub actor_id: String,
    pub idempotency_key: String,
    pub metadata_json: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionSplitLineRecord {
    pub split_line_id: String,
    pub split_id: String,
    pub parent_transaction_id: String,
    pub line_index: i64,
    pub description: String,
    pub amount: Decimal,
    pub category_id: Option<String>,
    pub category_source: String,
    pub context: Option<String>,
    pub status: String,
    pub actor_id: String,
    pub idempotency_key: String,
    pub metadata_json: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptItemRecord {
    pub receipt_item_id: String,
    pub parent_transaction_id: String,
    pub split_id: Option<String>,
    pub split_line_id: Option<String>,
    pub item_index: i64,
    pub description: String,
    pub quantity: Option<Decimal>,
    pub unit: Option<String>,
    pub unit_price: Option<Decimal>,
    pub total_price: Option<Decimal>,
    pub code: Option<String>,
    pub store_name: Option<String>,
    pub status: String,
    pub actor_id: String,
    pub idempotency_key: String,
    pub metadata_json: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitReviewPolicyRecord {
    pub policy_id: String,
    pub name: String,
    pub match_type: String,
    pub match_value: String,
    pub min_abs_amount: Option<Decimal>,
    pub status: String,
    pub actor_id: String,
    pub idempotency_key: String,
    pub metadata_json: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionSplitDetail {
    pub parent: TransactionRecord,
    pub split: Option<TransactionSplitRecord>,
    pub lines: Vec<TransactionSplitLineRecord>,
    pub items: Vec<ReceiptItemRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitCandidateRow {
    pub transaction_id: String,
    pub transaction_date: chrono::NaiveDate,
    pub description: String,
    pub amount: Decimal,
    pub account_id: Option<String>,
    pub category_id: Option<String>,
    pub context: Option<String>,
    pub policy_id: String,
    pub policy_name: String,
    pub match_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPriceRow {
    pub transaction_id: String,
    pub transaction_date: chrono::NaiveDate,
    pub description: String,
    pub quantity: Option<Decimal>,
    pub unit: Option<String>,
    pub unit_price: Option<Decimal>,
    pub total_price: Option<Decimal>,
    pub code: Option<String>,
    pub store_name: Option<String>,
    pub parent_description: String,
}

fn metadata_or_empty(value: Option<Value>) -> Value {
    value.unwrap_or_else(|| json!({}))
}

fn decimal_value_to_raw(value: &Value) -> Result<String> {
    match value {
        Value::Number(number) => Ok(number.to_string()),
        Value::String(text) => Ok(text.trim().to_string()),
        _ => bail!("Valor decimal deve ser string ou número JSON"),
    }
}

fn parse_optional_decimal(value: Option<&Value>, field: &str) -> Result<Option<Decimal>> {
    value
        .map(|raw| {
            decimal_from_str(&decimal_value_to_raw(raw)?)
                .with_context(|| format!("Campo {field} inválido"))
                .map(|decimal| decimal.round_dp(2))
        })
        .transpose()
}

fn parse_line_amount(raw: &Value, parent_amount: Decimal, index: usize) -> Result<Decimal> {
    let raw_text = decimal_value_to_raw(raw)?;
    let explicit_sign =
        raw_text.trim_start().starts_with('-') || raw_text.trim_start().starts_with('+');
    let parsed = decimal_from_str(&raw_text)
        .with_context(|| format!("amount inválido na linha {}", index + 1))?
        .round_dp(2);
    if explicit_sign {
        Ok(parsed)
    } else if parent_amount.is_sign_negative() {
        Ok(-parsed.abs())
    } else {
        Ok(parsed.abs())
    }
}

fn clean_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn validate_description(value: &str, field: &str, index: usize) -> Result<String> {
    let cleaned = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        bail!("{field} vazio no item {}", index + 1);
    }
    Ok(cleaned)
}

pub fn validate_split_payload(
    parent_transaction_id: &str,
    parent_amount: Decimal,
    payload: SplitPayload,
) -> Result<SplitPreview> {
    if payload.lines.is_empty() {
        bail!("Payload de split precisa ter pelo menos uma linha financeira");
    }

    let mut lines = Vec::with_capacity(payload.lines.len());
    for (index, line) in payload.lines.into_iter().enumerate() {
        let description = validate_description(&line.description, "description", index)?;
        let amount = parse_line_amount(&line.amount, parent_amount, index)?;
        lines.push(ValidatedSplitLine {
            line_index: index as i64,
            description,
            amount,
            category_id: clean_optional_text(line.category_id),
            context: clean_optional_text(line.context),
            metadata_json: metadata_or_empty(line.metadata),
        });
    }

    let split_total = lines
        .iter()
        .fold(Decimal::ZERO, |acc, line| acc + line.amount)
        .round_dp(2);
    let parent_amount = parent_amount.round_dp(2);
    let difference = (parent_amount - split_total).round_dp(2);
    if difference != Decimal::ZERO {
        bail!(
            "Soma do split ({}) não bate com a transação pai ({})",
            split_total,
            parent_amount
        );
    }

    let mut items = Vec::with_capacity(payload.items.len());
    for (index, item) in payload.items.into_iter().enumerate() {
        let split_line_index = item.split_line_index.map(|line_index| line_index as i64);
        if let Some(line_index) = split_line_index {
            if line_index < 0 || line_index as usize >= lines.len() {
                bail!(
                    "splitLineIndex {} do item {} não aponta para uma linha válida",
                    line_index,
                    index + 1
                );
            }
        }
        items.push(ValidatedReceiptItem {
            item_index: index as i64,
            description: validate_description(&item.description, "description", index)?,
            quantity: parse_optional_decimal(item.quantity.as_ref(), "quantity")?,
            unit: clean_optional_text(item.unit),
            unit_price: parse_optional_decimal(item.unit_price.as_ref(), "unitPrice")?,
            total_price: parse_optional_decimal(item.total_price.as_ref(), "totalPrice")?,
            code: clean_optional_text(item.code),
            store_name: clean_optional_text(item.store_name),
            split_line_index,
            metadata_json: metadata_or_empty(item.metadata),
        });
    }

    let canonical = json!({
        "source": clean_optional_text(payload.source),
        "notes": clean_optional_text(payload.notes),
        "metadata": metadata_or_empty(payload.metadata),
        "lines": lines,
        "items": items,
    });
    let payload_hash = hash_key(&[parent_transaction_id, &canonical.to_string()]);

    Ok(SplitPreview {
        parent_transaction_id: parent_transaction_id.to_string(),
        parent_amount,
        split_total,
        difference,
        payload_hash,
        lines,
        items,
    })
}

pub fn split_id(parent_transaction_id: &str, payload_hash: &str) -> String {
    format!("split:{parent_transaction_id}:{}", &payload_hash[..16])
}

pub fn split_line_id(split_id: &str, line_index: i64) -> String {
    format!("{split_id}:line:{line_index:03}")
}

pub fn receipt_item_id(split_id: &str, item_index: i64) -> String {
    format!("{split_id}:item:{item_index:03}")
}

pub fn split_idempotency(parent_transaction_id: &str, payload_hash: &str) -> String {
    format!("split:{parent_transaction_id}:{payload_hash}")
}

pub fn build_split_records(
    parent_transaction_id: &str,
    actor_id: &str,
    payload_source: Option<&str>,
    payload_notes: Option<&str>,
    payload_metadata: Option<Value>,
    preview: &SplitPreview,
    now: DateTime<Utc>,
) -> Result<(
    TransactionSplitRecord,
    Vec<TransactionSplitLineRecord>,
    Vec<ReceiptItemRecord>,
)> {
    let split_id = split_id(parent_transaction_id, &preview.payload_hash);
    let idempotency_key = split_idempotency(parent_transaction_id, &preview.payload_hash);
    let split = TransactionSplitRecord {
        split_id: split_id.clone(),
        parent_transaction_id: parent_transaction_id.to_string(),
        payload_hash: preview.payload_hash.clone(),
        status: "active".to_string(),
        source: payload_source
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("cli")
            .to_string(),
        notes: payload_notes
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
        actor_id: actor_id.to_string(),
        idempotency_key: idempotency_key.clone(),
        metadata_json: payload_metadata.unwrap_or_else(|| json!({})),
        created_at: now,
        updated_at: now,
    };

    let lines = preview
        .lines
        .iter()
        .map(|line| {
            let line_id = split_line_id(&split_id, line.line_index);
            TransactionSplitLineRecord {
                split_line_id: line_id.clone(),
                split_id: split_id.clone(),
                parent_transaction_id: parent_transaction_id.to_string(),
                line_index: line.line_index,
                description: line.description.clone(),
                amount: line.amount,
                category_id: line.category_id.clone(),
                category_source: "split".to_string(),
                context: line.context.clone(),
                status: "active".to_string(),
                actor_id: actor_id.to_string(),
                idempotency_key: format!("{idempotency_key}:line:{}", line.line_index),
                metadata_json: line.metadata_json.clone(),
                created_at: now,
                updated_at: now,
            }
        })
        .collect::<Vec<_>>();

    let items = preview
        .items
        .iter()
        .map(|item| {
            let linked_line_id = item
                .split_line_index
                .and_then(|line_index| lines.get(line_index as usize))
                .map(|line| line.split_line_id.clone());
            ReceiptItemRecord {
                receipt_item_id: receipt_item_id(&split_id, item.item_index),
                parent_transaction_id: parent_transaction_id.to_string(),
                split_id: Some(split_id.clone()),
                split_line_id: linked_line_id,
                item_index: item.item_index,
                description: item.description.clone(),
                quantity: item.quantity,
                unit: item.unit.clone(),
                unit_price: item.unit_price,
                total_price: item.total_price,
                code: item.code.clone(),
                store_name: item.store_name.clone(),
                status: "active".to_string(),
                actor_id: actor_id.to_string(),
                idempotency_key: format!("{idempotency_key}:item:{}", item.item_index),
                metadata_json: item.metadata_json.clone(),
                created_at: now,
                updated_at: now,
            }
        })
        .collect::<Vec<_>>();

    Ok((split, lines, items))
}

pub fn parse_split_payload(raw: &str) -> Result<SplitPayload> {
    serde_json::from_str(raw).map_err(|err| anyhow!("Payload de split inválido: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unsigned_line_amounts_inherit_parent_sign() {
        let payload = serde_json::from_value(json!({
            "lines": [
                {"description": "Mercado", "amount": "60.00", "categoryId": "alimentacao:mercado"},
                {"description": "Limpeza", "amount": "40.00", "categoryId": "casa:limpeza"}
            ]
        }))
        .unwrap();
        let preview =
            validate_split_payload("tx-1", decimal_from_str("-100.00").unwrap(), payload).unwrap();
        assert_eq!(preview.lines[0].amount, decimal_from_str("-60.00").unwrap());
        assert_eq!(preview.split_total, decimal_from_str("-100.00").unwrap());
    }

    #[test]
    fn signed_line_amounts_are_respected() {
        let payload = serde_json::from_value(json!({
            "lines": [
                {"description": "Principal", "amount": "-90.00"},
                {"description": "Estorno parcial", "amount": "+10.00"}
            ]
        }))
        .unwrap();
        let preview =
            validate_split_payload("tx-1", decimal_from_str("-80.00").unwrap(), payload).unwrap();
        assert_eq!(preview.split_total, decimal_from_str("-80.00").unwrap());
    }

    #[test]
    fn rejects_mismatched_totals() {
        let payload = serde_json::from_value(json!({
            "lines": [
                {"description": "Mercado", "amount": "60.00"}
            ]
        }))
        .unwrap();
        let err = validate_split_payload("tx-1", decimal_from_str("-100.00").unwrap(), payload)
            .unwrap_err();
        assert!(err.to_string().contains("não bate"));
    }

    #[test]
    fn validates_item_line_reference() {
        let payload = serde_json::from_value(json!({
            "lines": [
                {"description": "Mercado", "amount": "100.00"}
            ],
            "items": [
                {"description": "Leite", "totalPrice": "10.00", "splitLineIndex": 2}
            ]
        }))
        .unwrap();
        let err = validate_split_payload("tx-1", decimal_from_str("-100.00").unwrap(), payload)
            .unwrap_err();
        assert!(err.to_string().contains("splitLineIndex"));
    }

    #[test]
    fn split_ids_are_stable() {
        let payload = serde_json::from_value(json!({
            "lines": [
                {"description": "Mercado", "amount": "100.00"}
            ]
        }))
        .unwrap();
        let a =
            validate_split_payload("tx-1", decimal_from_str("-100.00").unwrap(), payload).unwrap();
        let payload = serde_json::from_value(json!({
            "lines": [
                {"description": "Mercado", "amount": "100.00"}
            ]
        }))
        .unwrap();
        let b =
            validate_split_payload("tx-1", decimal_from_str("-100.00").unwrap(), payload).unwrap();
        assert_eq!(a.payload_hash, b.payload_hash);
        assert_eq!(
            split_id("tx-1", &a.payload_hash),
            split_id("tx-1", &b.payload_hash)
        );
    }
}
