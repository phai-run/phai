use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::str::FromStr;
use uuid::Uuid;

pub fn parse_datetime_or_now(value: Option<&str>) -> DateTime<Utc> {
    value
        .and_then(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                DateTime::parse_from_rfc3339(trimmed).ok()
            }
        })
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMetadata {
    pub actor_id: String,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    pub account_id: String,
    pub owner: String,
    pub account_type: String,
    pub bank: String,
    pub label: String,
    pub pluggy_account_id: Option<String>,
    pub pluggy_item_id: Option<String>,
    pub status: String,
    pub actor_id: String,
    pub idempotency_key: String,
    pub metadata_json: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub transaction_id: String,
    pub account_id: Option<String>,
    pub transaction_date: NaiveDate,
    pub description: String,
    pub amount: Decimal,
    pub tx_type: String,
    pub category_id: Option<String>,
    pub category_source: String,
    pub context: Option<String>,
    pub payment_status: String,
    pub source: String,
    pub actor_id: String,
    pub idempotency_key: String,
    pub metadata_json: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleRecord {
    pub rule_id: String,
    pub body: String,
    pub status: String,
    pub actor_id: String,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastRecord {
    pub forecast_id: String,
    pub due_date: Option<NaiveDate>,
    pub description: String,
    pub amount: Decimal,
    pub category_id: Option<String>,
    pub account_id: Option<String>,
    pub status: String,
    pub recurrence: Option<String>,
    pub actor_id: String,
    pub idempotency_key: String,
    pub metadata_json: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryRecord {
    pub category_id: String,
    pub name: String,
    pub parent_category_id: Option<String>,
    pub metadata_json: Value,
    pub actor_id: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyPulseItem {
    pub transaction_id: String,
    pub transaction_date: NaiveDate,
    pub description: String,
    pub amount: Decimal,
    pub category_id: Option<String>,
    pub source: String,
    pub payment_status: String,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonthlySpendRow {
    pub month_ref: String,
    pub category_id: String,
    pub account_id: String,
    pub expenses: Decimal,
    pub expense_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashflowRow {
    pub month_ref: String,
    pub income: Decimal,
    pub expenses: Decimal,
    pub net: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastVsActualRow {
    pub forecast_id: String,
    pub month_ref: String,
    pub due_date: Option<NaiveDate>,
    pub description: String,
    pub account_id: Option<String>,
    pub category_id: Option<String>,
    pub forecast_amount: Decimal,
    pub actual_amount: Decimal,
    pub variance: Decimal,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardSummaryRow {
    pub month_ref: String,
    pub account_id: String,
    pub total_charges: Decimal,
    pub open_amount: Decimal,
    pub transaction_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncategorizedRow {
    pub transaction_id: String,
    pub transaction_date: NaiveDate,
    pub description: String,
    pub amount: Decimal,
    pub account_id: Option<String>,
    pub account_label: Option<String>,
    pub tx_type: String,
    pub category_source: String,
    pub payment_status: String,
    pub source: String,
    pub metadata_json: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionContextRow {
    pub transaction_id: String,
    pub transaction_date: NaiveDate,
    pub description: String,
    pub amount: Decimal,
    pub account_id: Option<String>,
    pub account_label: Option<String>,
    pub category_id: Option<String>,
    pub context: String,
    pub payment_status: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub action: String,
    pub actor_id: String,
    pub event_timestamp: DateTime<Utc>,
    pub idempotency_key: String,
    pub diff_json: Value,
}

impl AuditEvent {
    pub fn from_entity(
        entity_type: &str,
        entity_id: &str,
        action: &str,
        actor_id: &str,
        idempotency_key: &str,
        diff_json: Value,
    ) -> Self {
        Self {
            event_id: Uuid::now_v7().to_string(),
            entity_type: entity_type.to_string(),
            entity_id: entity_id.to_string(),
            action: action.to_string(),
            actor_id: actor_id.to_string(),
            event_timestamp: Utc::now(),
            idempotency_key: idempotency_key.to_string(),
            diff_json,
        }
    }
}

impl DailyPulseItem {
    pub fn format_brl(&self) -> String {
        let sign = if self.amount.is_sign_negative() {
            "-"
        } else {
            "+"
        };
        let value = self.amount.abs().round_dp(2).to_string().replace('.', ",");
        format!("{sign}R$ {value}")
    }
}

pub fn decimal_from_str(value: &str) -> Result<Decimal> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("String vazia não pode ser parseada como decimal"));
    }
    // US/international format: digits with optional dot decimal separator, no commas
    if trimmed.contains('.') && !trimmed.contains(',') {
        return Decimal::from_str(trimmed)
            .with_context(|| format!("Falha ao parsear decimal '{value}'"));
    }
    // Brazilian format: dots as thousand separators, comma as decimal separator
    let cleaned = trimmed.replace('.', "").replace(',', ".");
    Decimal::from_str(&cleaned).with_context(|| format!("Falha ao parsear decimal '{value}'"))
}

pub fn json_object_or_empty(value: Option<Value>) -> Value {
    value.unwrap_or_else(|| json!({}))
}
