use super::{FinanceStore, TransactionAnatomyPatch};
use crate::config::AppConfig;
use crate::models::{
    parse_datetime_or_now, AccountRecord, AccountSnapshotRecord, AuditEvent, BudgetStatusRow,
    CardClosedTransactionRow, CardSummaryRow, CashflowRow, CategoryBudgetRecord, CategoryRecord,
    CheckingBalance, DailyPulseItem, ForecastRecord, ForecastTemplateRecord, ForecastVsActualRow,
    MonthlySpendRow, RuleRecord, TransactionContextRow, TransactionRecord, UncategorizedRow,
};
use crate::splits::{
    ItemPriceRow, ReceiptItemRecord, SplitCandidateRow, TransactionSplitDetail,
    TransactionSplitLineRecord, TransactionSplitRecord,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::{Datelike, Days, NaiveDate, Utc};
use reqwest::Client;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};
use yup_oauth2::{read_service_account_key, ServiceAccountAuthenticator};

const BIGQUERY_SCOPE: &str = "https://www.googleapis.com/auth/bigquery";
const BIGQUERY_SCOPES: &[&str] = &[BIGQUERY_SCOPE];

pub struct BigQueryStore {
    config: AppConfig,
    client: Client,
    service_account_path: PathBuf,
    cached_token: RefCell<Option<(String, Instant)>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryResponse {
    #[serde(default)]
    job_complete: bool,
    #[serde(default)]
    job_reference: Option<JobReference>,
    #[serde(default)]
    rows: Vec<QueryRow>,
    /// Populated for DML statements (UPDATE/INSERT/DELETE).
    #[serde(default)]
    num_dml_affected_rows: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobReference {
    job_id: String,
    location: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QueryRow {
    f: Vec<QueryCell>,
}

#[derive(Debug, Deserialize)]
struct QueryCell {
    v: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryRequest<'a> {
    query: &'a str,
    use_legacy_sql: bool,
    timeout_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameter_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    query_parameters: Vec<Value>,
}

/// BigQuery query parameter type declaration, mirroring the REST schema at
/// https://cloud.google.com/bigquery/docs/reference/rest/v2/QueryParameterType.
/// Only the subset used by phai write paths is modelled; expand on
/// demand. `Bool` is intentionally part of the surface so callers can opt
/// in without touching this module.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum BqType {
    String,
    Numeric,
    Date,
    Timestamp,
    Json,
    Int64,
    Float64,
    Bool,
    Struct(Vec<(String, BqType)>),
    Array(Box<BqType>),
}

impl BqType {
    fn to_json(&self) -> Value {
        match self {
            BqType::String => json!({"type": "STRING"}),
            BqType::Numeric => json!({"type": "NUMERIC"}),
            BqType::Date => json!({"type": "DATE"}),
            BqType::Timestamp => json!({"type": "TIMESTAMP"}),
            BqType::Json => json!({"type": "JSON"}),
            BqType::Int64 => json!({"type": "INT64"}),
            BqType::Float64 => json!({"type": "FLOAT64"}),
            BqType::Bool => json!({"type": "BOOL"}),
            BqType::Struct(fields) => {
                let struct_types: Vec<Value> = fields
                    .iter()
                    .map(|(name, ty)| json!({"name": name, "type": ty.to_json()}))
                    .collect();
                json!({"type": "STRUCT", "structTypes": struct_types})
            }
            BqType::Array(inner) => {
                json!({"type": "ARRAY", "arrayType": inner.to_json()})
            }
        }
    }
}

/// BigQuery query parameter value, typed to match `BqType`. `Null` is paired
/// with the spec's declared type at the `Param` level.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum BqValue {
    String(String),
    Numeric(String),
    Date(NaiveDate),
    Timestamp(chrono::DateTime<Utc>),
    Json(String),
    Int64(i64),
    Float64(f64),
    Bool(bool),
    Null,
    Struct(Vec<(String, BqValue)>),
    Array(Vec<BqValue>),
}

impl BqValue {
    fn to_json(&self) -> Value {
        match self {
            // BigQuery's REST API uses an empty parameterValue object to
            // signal NULL; sending JSON `null` here returns
            // `"Missing query parameter value"`.
            BqValue::Null => json!({}),
            BqValue::String(s) => json!({"value": s}),
            BqValue::Numeric(s) => json!({"value": s}),
            BqValue::Date(d) => json!({"value": d.format("%Y-%m-%d").to_string()}),
            BqValue::Timestamp(t) => json!({"value": t.to_rfc3339()}),
            BqValue::Json(s) => json!({"value": s}),
            BqValue::Int64(i) => json!({"value": i.to_string()}),
            BqValue::Float64(f) => json!({"value": f.to_string()}),
            BqValue::Bool(b) => json!({"value": if *b { "true" } else { "false" }}),
            BqValue::Struct(fields) => {
                let mut obj = serde_json::Map::new();
                for (name, value) in fields {
                    obj.insert(name.clone(), value.to_json());
                }
                json!({"structValues": Value::Object(obj)})
            }
            BqValue::Array(values) => {
                let array_values: Vec<Value> = values.iter().map(|v| v.to_json()).collect();
                json!({"arrayValues": array_values})
            }
        }
    }
}

/// A single named query parameter passed to BigQuery via `queryParameters`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Param {
    name: String,
    ty: BqType,
    value: BqValue,
}

#[allow(dead_code)]
impl Param {
    fn new(name: &str, ty: BqType, value: BqValue) -> Self {
        Self {
            name: name.to_string(),
            ty,
            value,
        }
    }

    fn string(name: &str, value: impl Into<String>) -> Self {
        Self::new(name, BqType::String, BqValue::String(value.into()))
    }

    fn optional_string<S: Into<String>>(name: &str, value: Option<S>) -> Self {
        match value {
            Some(v) => Self::new(name, BqType::String, BqValue::String(v.into())),
            None => Self::new(name, BqType::String, BqValue::Null),
        }
    }

    fn decimal(name: &str, value: Decimal) -> Self {
        Self::new(
            name,
            BqType::Numeric,
            BqValue::Numeric(value.round_dp(2).to_string()),
        )
    }

    fn optional_decimal(name: &str, value: Option<Decimal>) -> Self {
        match value {
            Some(v) => Self::decimal(name, v),
            None => Self::new(name, BqType::Numeric, BqValue::Null),
        }
    }

    fn date(name: &str, value: NaiveDate) -> Self {
        Self::new(name, BqType::Date, BqValue::Date(value))
    }

    fn optional_date(name: &str, value: Option<NaiveDate>) -> Self {
        match value {
            Some(v) => Self::date(name, v),
            None => Self::new(name, BqType::Date, BqValue::Null),
        }
    }

    fn timestamp(name: &str, value: chrono::DateTime<Utc>) -> Self {
        Self::new(name, BqType::Timestamp, BqValue::Timestamp(value))
    }

    fn json(name: &str, value: &Value) -> Self {
        Self::new(
            name,
            BqType::Json,
            BqValue::Json(serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())),
        )
    }

    fn int64(name: &str, value: i64) -> Self {
        Self::new(name, BqType::Int64, BqValue::Int64(value))
    }

    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "parameterType": self.ty.to_json(),
            "parameterValue": self.value.to_json(),
        })
    }
}

// Compact constructors used inside batch struct rows.
fn bv_str<S: Into<String>>(value: S) -> BqValue {
    BqValue::String(value.into())
}

fn bv_opt_str(value: Option<&str>) -> BqValue {
    match value {
        Some(s) => BqValue::String(s.to_string()),
        None => BqValue::Null,
    }
}

fn bv_dec(value: Decimal) -> BqValue {
    BqValue::Numeric(value.round_dp(2).to_string())
}

fn bv_opt_dec(value: Option<Decimal>) -> BqValue {
    match value {
        Some(v) => bv_dec(v),
        None => BqValue::Null,
    }
}

fn bv_date(value: NaiveDate) -> BqValue {
    BqValue::Date(value)
}

fn bv_opt_date(value: Option<NaiveDate>) -> BqValue {
    match value {
        Some(v) => bv_date(v),
        None => BqValue::Null,
    }
}

fn bv_ts(value: chrono::DateTime<Utc>) -> BqValue {
    BqValue::Timestamp(value)
}

fn bv_json(value: &Value) -> BqValue {
    BqValue::Json(serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()))
}

fn bv_int(value: i64) -> BqValue {
    BqValue::Int64(value)
}

fn bv_opt_int(value: Option<i64>) -> BqValue {
    match value {
        Some(v) => BqValue::Int64(v),
        None => BqValue::Null,
    }
}

fn bv_opt_float(value: Option<f64>) -> BqValue {
    match value {
        Some(v) => BqValue::Float64(v),
        None => BqValue::Null,
    }
}

fn field<S: Into<String>>(name: S, value: BqValue) -> (String, BqValue) {
    (name.into(), value)
}

fn batch_array_param(
    name: &str,
    fields: Vec<(&str, BqType)>,
    rows: Vec<Vec<(String, BqValue)>>,
) -> Param {
    let struct_fields: Vec<(String, BqType)> = fields
        .into_iter()
        .map(|(n, t)| (n.to_string(), t))
        .collect();
    let array = BqValue::Array(rows.into_iter().map(BqValue::Struct).collect());
    Param::new(
        name,
        BqType::Array(Box::new(BqType::Struct(struct_fields))),
        array,
    )
}

impl BigQueryStore {
    pub async fn new(config: AppConfig) -> Result<Self> {
        let service_account_path = config.service_account_path()?.to_path_buf();
        Ok(Self {
            config,
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .context("Falha ao construir cliente HTTP")?,
            service_account_path,
            cached_token: RefCell::new(None),
        })
    }

    async fn bearer_token(&self) -> Result<String> {
        if let Some((token, created_at)) = self.cached_token.borrow().as_ref() {
            if created_at.elapsed() < Duration::from_secs(3000) {
                return Ok(token.clone());
            }
        }
        let key = read_service_account_key(&self.service_account_path)
            .await
            .context("Falha ao ler service account do BigQuery")?;
        let auth = ServiceAccountAuthenticator::builder(key)
            .build()
            .await
            .context("Falha ao construir autenticador BigQuery")?;
        let token = auth.token(BIGQUERY_SCOPES).await?;
        let token_str = token
            .token()
            .map(|value| value.to_string())
            .context("Token BigQuery ausente")?;
        *self.cached_token.borrow_mut() = Some((token_str.clone(), Instant::now()));
        Ok(token_str)
    }

    fn query_endpoint(&self) -> Result<String> {
        Ok(format!(
            "https://bigquery.googleapis.com/bigquery/v2/projects/{}/queries",
            self.config.project_id()?
        ))
    }

    fn query_job_endpoint(&self, job_id: &str, location: Option<&str>) -> Result<String> {
        let mut url = format!(
            "https://bigquery.googleapis.com/bigquery/v2/projects/{}/queries/{}",
            self.config.project_id()?,
            job_id
        );
        if let Some(loc) = location {
            let encoded: String = loc
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c.to_string()
                    } else {
                        format!("%{:02X}", c as u8)
                    }
                })
                .collect();
            url.push_str(&format!("?location={encoded}"));
        }
        Ok(url)
    }

    async fn run_query(&self, sql: &str) -> Result<QueryResponse> {
        self.run_query_with_params(sql, &[]).await
    }

    async fn run_query_with_params(&self, sql: &str, params: &[Param]) -> Result<QueryResponse> {
        let token = self.bearer_token().await?;
        let (parameter_mode, query_parameters) = if params.is_empty() {
            (None, Vec::new())
        } else {
            (
                Some("NAMED"),
                params.iter().map(|p| p.to_json()).collect::<Vec<_>>(),
            )
        };
        let response = self
            .client
            .post(self.query_endpoint()?)
            .bearer_auth(&token)
            .json(&QueryRequest {
                query: sql,
                use_legacy_sql: false,
                timeout_ms: 30_000,
                parameter_mode,
                query_parameters,
            })
            .send()
            .await
            .context("Falha ao chamar BigQuery")?;
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("BigQuery query falhou: {body}"));
        }
        let mut parsed: QueryResponse =
            response.json().await.context("JSON inválido do BigQuery")?;
        let mut poll_attempts = 0u32;
        const MAX_POLL_ATTEMPTS: u32 = 60;
        while !parsed.job_complete {
            poll_attempts += 1;
            if poll_attempts > MAX_POLL_ATTEMPTS {
                return Err(anyhow!(
                    "BigQuery job não completou após {MAX_POLL_ATTEMPTS} tentativas"
                ));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            let job = parsed
                .job_reference
                .as_ref()
                .context("BigQuery retornou job incompleto sem referência")?;
            let poll = self
                .client
                .get(self.query_job_endpoint(&job.job_id, job.location.as_deref())?)
                .bearer_auth(&token)
                .send()
                .await
                .context("Falha ao consultar job BigQuery")?;
            if !poll.status().is_success() {
                let body = poll.text().await.unwrap_or_default();
                return Err(anyhow!("Polling do BigQuery falhou: {body}"));
            }
            parsed = poll
                .json()
                .await
                .context("JSON inválido no polling BigQuery")?;
        }
        Ok(parsed)
    }

    fn qualified_table(&self, table: &str) -> Result<String> {
        Ok(format!(
            "`{}.{}.{}`",
            self.config.project_id()?,
            self.config.dataset_id()?,
            table
        ))
    }

    /// If `transaction_id` matches a row in `transaction_split_lines` with an
    /// active or confirmed split, returns the parent transaction id so writes
    /// can be routed appropriately. Returns `None` for ordinary transactions.
    ///
    /// Why: after splitting, the parent row is hidden by `v_transactions_effective`
    /// and the user interacts with the synthetic child rows (whose id is the
    /// `split_line_id`). Anatomy/category edits against those ids must hit
    /// `transaction_split_lines` — otherwise the UPDATE silently affects zero
    /// rows on `transactions`.
    async fn resolve_split_line_target(&self, transaction_id: &str) -> Result<Option<String>> {
        let sql = format!(
            "
            SELECT parent_transaction_id
            FROM {}
            WHERE split_line_id = @tid
              AND status IN ('active', 'confirmed')
            LIMIT 1
            ",
            self.qualified_table("transaction_split_lines")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::string("tid", transaction_id)])
            .await?;
        let Some(row) = response.rows.first() else {
            return Ok(None);
        };
        let values = row_values(row);
        Ok(Some(required_string(&values, 0, "parent_transaction_id")?))
    }
}

/// Mirror of `local::last_day_of_target_month`: last calendar day of the
/// month at `month_start`, capped at `today` so the current month's
/// closing anchor is "now".
fn bq_last_day_of_target_month(month_start: NaiveDate, today: NaiveDate) -> Result<NaiveDate> {
    let (year, month) = (month_start.year(), month_start.month());
    let next_month_first = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .context("Falha ao calcular início do mês seguinte")?;
    let last_day = next_month_first
        .checked_sub_days(Days::new(1))
        .context("Falha ao calcular último dia do mês")?;
    Ok(if last_day > today { today } else { last_day })
}

fn parse_scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => Some(value.to_string()),
    }
}

fn row_values(row: &QueryRow) -> Vec<Option<String>> {
    row.f
        .iter()
        .map(|cell| parse_scalar_string(&cell.v))
        .collect::<Vec<_>>()
}

fn required_string(values: &[Option<String>], index: usize, field: &str) -> Result<String> {
    values
        .get(index)
        .and_then(|value| value.clone())
        .with_context(|| format!("{field} ausente na linha do BigQuery"))
}

fn optional_string(values: &[Option<String>], index: usize) -> Option<String> {
    values.get(index).and_then(|value| value.clone())
}

fn required_decimal(values: &[Option<String>], index: usize, field: &str) -> Result<Decimal> {
    Decimal::from_str(&required_string(values, index, field)?)
        .with_context(|| format!("Falha ao parsear {field} do BigQuery"))
}

fn required_i64(values: &[Option<String>], index: usize, field: &str) -> Result<i64> {
    required_string(values, index, field)?
        .parse::<i64>()
        .with_context(|| format!("Falha ao parsear {field} do BigQuery"))
}

fn required_date(values: &[Option<String>], index: usize, field: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(&required_string(values, index, field)?, "%Y-%m-%d")
        .with_context(|| format!("Falha ao parsear {field} do BigQuery"))
}

fn optional_date(
    values: &[Option<String>],
    index: usize,
    field: &str,
) -> Result<Option<NaiveDate>> {
    optional_string(values, index)
        .map(|value| {
            NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                .with_context(|| format!("Falha ao parsear {field} do BigQuery"))
        })
        .transpose()
}

fn optional_json(values: &[Option<String>], index: usize, field: &str) -> Result<Option<Value>> {
    optional_string(values, index)
        .map(|value| {
            serde_json::from_str(&value)
                .with_context(|| format!("Falha ao parsear {field} do BigQuery"))
        })
        .transpose()
}

fn forecast_template_from_bq(values: &[Option<String>]) -> Result<ForecastTemplateRecord> {
    let amount = required_decimal(values, 6, "amount")?;
    let amount_lower = optional_string(values, 7)
        .map(|s| Decimal::from_str(&s).with_context(|| "amount_lower"))
        .transpose()?;
    let amount_upper = optional_string(values, 8)
        .map(|s| Decimal::from_str(&s).with_context(|| "amount_upper"))
        .transpose()?;
    let next_due_day = optional_string(values, 10)
        .map(|s| s.parse::<i32>().with_context(|| "next_due_day"))
        .transpose()?;
    let start_date = required_date(values, 11, "start_date")?;
    let end_date = optional_date(values, 12, "end_date")?;
    let remaining_count = optional_string(values, 13)
        .map(|s| s.parse::<i32>().with_context(|| "remaining_count"))
        .transpose()?;
    let confidence = optional_string(values, 15)
        .map(|s| s.parse::<f64>().with_context(|| "confidence"))
        .transpose()?;
    let metadata_json = optional_json(values, 17, "metadata_json")?
        .unwrap_or_else(|| Value::Object(Default::default()));
    let created_str = required_string(values, 20, "created_at")?;
    let updated_str = required_string(values, 21, "updated_at")?;
    Ok(ForecastTemplateRecord {
        template_id: required_string(values, 0, "template_id")?,
        kind: required_string(values, 1, "kind")?,
        description: required_string(values, 2, "description")?,
        merchant_pattern: optional_string(values, 3),
        category_id: optional_string(values, 4),
        account_id: optional_string(values, 5),
        amount,
        amount_lower,
        amount_upper,
        cadence: required_string(values, 9, "cadence")?,
        next_due_day,
        start_date,
        end_date,
        remaining_count,
        source: required_string(values, 14, "source")?,
        confidence,
        status: required_string(values, 16, "status")?,
        metadata_json,
        actor_id: required_string(values, 18, "actor_id")?,
        idempotency_key: required_string(values, 19, "idempotency_key")?,
        created_at: parse_datetime_or_now(Some(&created_str)),
        updated_at: parse_datetime_or_now(Some(&updated_str)),
    })
}

fn transaction_record_from_values(values: &[Option<String>]) -> Result<TransactionRecord> {
    let created_at = required_string(values, 18, "created_at")?;
    let updated_at = required_string(values, 19, "updated_at")?;
    let enrichment_attempted_at =
        optional_string(values, 20).map(|raw| parse_datetime_or_now(Some(&raw)));
    let description = optional_string(values, 4);
    let raw_description = optional_string(values, 3)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| description.clone().unwrap_or_default());
    Ok(TransactionRecord {
        transaction_id: required_string(values, 0, "transaction_id")?,
        account_id: optional_string(values, 1),
        transaction_date: required_date(values, 2, "transaction_date")?,
        raw_description,
        description,
        merchant_name: optional_string(values, 5),
        purpose: optional_string(values, 6),
        amount: required_decimal(values, 7, "amount")?,
        tx_type: required_string(values, 8, "tx_type")?,
        category_id: optional_string(values, 9),
        category_source: required_string(values, 10, "category_source")?,
        context: optional_string(values, 11),
        classifier_trace: optional_string(values, 12),
        payment_status: required_string(values, 13, "payment_status")?,
        source: required_string(values, 14, "source")?,
        actor_id: required_string(values, 15, "actor_id")?,
        idempotency_key: required_string(values, 16, "idempotency_key")?,
        metadata_json: optional_json(values, 17, "metadata_json")?.unwrap_or_else(|| json!({})),
        created_at: parse_datetime_or_now(Some(&created_at)),
        updated_at: parse_datetime_or_now(Some(&updated_at)),
        enrichment_attempted_at,
        amount_cents: None,
    })
}

fn account_record_from_values(values: &[Option<String>]) -> Result<AccountRecord> {
    let created_at = required_string(values, 11, "created_at")?;
    let updated_at = required_string(values, 12, "updated_at")?;
    Ok(AccountRecord {
        account_id: required_string(values, 0, "account_id")?,
        owner: required_string(values, 1, "owner")?,
        account_type: required_string(values, 2, "account_type")?,
        bank: required_string(values, 3, "bank")?,
        label: required_string(values, 4, "label")?,
        pluggy_account_id: optional_string(values, 5),
        pluggy_item_id: optional_string(values, 6),
        status: required_string(values, 7, "status")?,
        actor_id: required_string(values, 8, "actor_id")?,
        idempotency_key: required_string(values, 9, "idempotency_key")?,
        metadata_json: optional_json(values, 10, "metadata_json")?.unwrap_or_else(|| json!({})),
        created_at: parse_datetime_or_now(Some(&created_at)),
        updated_at: parse_datetime_or_now(Some(&updated_at)),
    })
}

fn split_record_from_values(values: &[Option<String>]) -> Result<TransactionSplitRecord> {
    let created_at = required_string(values, 8, "created_at")?;
    let updated_at = required_string(values, 9, "updated_at")?;
    Ok(TransactionSplitRecord {
        split_id: required_string(values, 0, "split_id")?,
        parent_transaction_id: required_string(values, 1, "parent_transaction_id")?,
        payload_hash: required_string(values, 2, "payload_hash")?,
        status: required_string(values, 3, "status")?,
        source: required_string(values, 4, "source")?,
        notes: optional_string(values, 5),
        actor_id: required_string(values, 6, "actor_id")?,
        idempotency_key: required_string(values, 7, "idempotency_key")?,
        metadata_json: optional_json(values, 10, "metadata_json")?.unwrap_or_else(|| json!({})),
        created_at: parse_datetime_or_now(Some(&created_at)),
        updated_at: parse_datetime_or_now(Some(&updated_at)),
    })
}

fn split_line_record_from_values(values: &[Option<String>]) -> Result<TransactionSplitLineRecord> {
    let created_at = required_string(values, 13, "created_at")?;
    let updated_at = required_string(values, 14, "updated_at")?;
    Ok(TransactionSplitLineRecord {
        split_line_id: required_string(values, 0, "split_line_id")?,
        split_id: required_string(values, 1, "split_id")?,
        parent_transaction_id: required_string(values, 2, "parent_transaction_id")?,
        line_index: required_i64(values, 3, "line_index")?,
        description: required_string(values, 4, "description")?,
        amount: required_decimal(values, 5, "amount")?,
        category_id: optional_string(values, 6),
        category_source: required_string(values, 7, "category_source")?,
        context: optional_string(values, 8),
        status: required_string(values, 9, "status")?,
        actor_id: required_string(values, 10, "actor_id")?,
        idempotency_key: required_string(values, 11, "idempotency_key")?,
        metadata_json: optional_json(values, 12, "metadata_json")?.unwrap_or_else(|| json!({})),
        created_at: parse_datetime_or_now(Some(&created_at)),
        updated_at: parse_datetime_or_now(Some(&updated_at)),
    })
}

fn receipt_item_record_from_values(values: &[Option<String>]) -> Result<ReceiptItemRecord> {
    let created_at = required_string(values, 15, "created_at")?;
    let updated_at = required_string(values, 16, "updated_at")?;
    Ok(ReceiptItemRecord {
        receipt_item_id: required_string(values, 0, "receipt_item_id")?,
        parent_transaction_id: required_string(values, 1, "parent_transaction_id")?,
        split_id: optional_string(values, 2),
        split_line_id: optional_string(values, 3),
        item_index: required_i64(values, 4, "item_index")?,
        description: required_string(values, 5, "description")?,
        quantity: optional_string(values, 6)
            .map(|value| {
                Decimal::from_str(&value)
                    .with_context(|| "Falha ao parsear quantity do BigQuery".to_string())
            })
            .transpose()?,
        unit: optional_string(values, 7),
        unit_price: optional_string(values, 8)
            .map(|value| {
                Decimal::from_str(&value)
                    .with_context(|| "Falha ao parsear unit_price do BigQuery".to_string())
            })
            .transpose()?,
        total_price: optional_string(values, 9)
            .map(|value| {
                Decimal::from_str(&value)
                    .with_context(|| "Falha ao parsear total_price do BigQuery".to_string())
            })
            .transpose()?,
        code: optional_string(values, 10),
        store_name: optional_string(values, 11),
        status: required_string(values, 12, "status")?,
        actor_id: required_string(values, 13, "actor_id")?,
        idempotency_key: required_string(values, 14, "idempotency_key")?,
        metadata_json: optional_json(values, 17, "metadata_json")?.unwrap_or_else(|| json!({})),
        created_at: parse_datetime_or_now(Some(&created_at)),
        updated_at: parse_datetime_or_now(Some(&updated_at)),
    })
}

#[async_trait(?Send)]
impl FinanceStore for BigQueryStore {
    async fn applied_migrations(&self) -> Result<BTreeSet<String>> {
        let sql = format!(
            "SELECT version FROM {} ORDER BY version",
            self.qualified_table("schema_versions")?
        );
        match self.run_query(&sql).await {
            Ok(response) => Ok(response
                .rows
                .into_iter()
                .filter_map(|row| row.f.first().and_then(|cell| parse_scalar_string(&cell.v)))
                .collect()),
            Err(error) if error.to_string().contains("Not found: Table") => Ok(BTreeSet::new()),
            Err(error) => Err(error),
        }
    }

    async fn apply_sql(&self, sql: &str) -> Result<()> {
        self.run_query(sql).await?;
        Ok(())
    }

    async fn record_migration(&self, version: &str) -> Result<()> {
        let sql = format!(
            "
            MERGE {} target
            USING (SELECT @version AS version, CURRENT_TIMESTAMP() AS applied_at) source
            ON target.version = source.version
            WHEN MATCHED THEN UPDATE SET applied_at = source.applied_at
            WHEN NOT MATCHED THEN INSERT (version, applied_at) VALUES (source.version, source.applied_at)
            ",
            self.qualified_table("schema_versions")?,
        );
        self.run_query_with_params(&sql, &[Param::string("version", version)])
            .await?;
        Ok(())
    }

    async fn upsert_accounts(&self, rows: &[AccountRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let param = batch_array_param(
            "batch",
            vec![
                ("account_id", BqType::String),
                ("owner", BqType::String),
                ("account_type", BqType::String),
                ("bank", BqType::String),
                ("label", BqType::String),
                ("pluggy_account_id", BqType::String),
                ("pluggy_item_id", BqType::String),
                ("status", BqType::String),
                ("actor_id", BqType::String),
                ("idempotency_key", BqType::String),
                ("metadata_json", BqType::Json),
                ("created_at", BqType::Timestamp),
                ("updated_at", BqType::Timestamp),
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        field("account_id", bv_str(&r.account_id)),
                        field("owner", bv_str(&r.owner)),
                        field("account_type", bv_str(&r.account_type)),
                        field("bank", bv_str(&r.bank)),
                        field("label", bv_str(&r.label)),
                        field(
                            "pluggy_account_id",
                            bv_opt_str(r.pluggy_account_id.as_deref()),
                        ),
                        field("pluggy_item_id", bv_opt_str(r.pluggy_item_id.as_deref())),
                        field("status", bv_str(&r.status)),
                        field("actor_id", bv_str(&r.actor_id)),
                        field("idempotency_key", bv_str(&r.idempotency_key)),
                        field("metadata_json", bv_json(&r.metadata_json)),
                        field("created_at", bv_ts(r.created_at)),
                        field("updated_at", bv_ts(r.updated_at)),
                    ]
                })
                .collect(),
        );

        let sql = format!(
            "
            MERGE {} target
            USING (SELECT * FROM UNNEST(@batch)) source
            ON target.account_id = source.account_id
            WHEN MATCHED THEN UPDATE SET
              owner = source.owner,
              account_type = source.account_type,
              bank = source.bank,
              label = source.label,
              pluggy_account_id = source.pluggy_account_id,
              pluggy_item_id = source.pluggy_item_id,
              status = source.status,
              actor_id = source.actor_id,
              idempotency_key = source.idempotency_key,
              metadata_json = source.metadata_json,
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              account_id, owner, account_type, bank, label, pluggy_account_id, pluggy_item_id,
              status, actor_id, idempotency_key, metadata_json, created_at, updated_at
            ) VALUES (
              source.account_id, source.owner, source.account_type, source.bank, source.label, source.pluggy_account_id, source.pluggy_item_id,
              source.status, source.actor_id, source.idempotency_key, source.metadata_json, source.created_at, source.updated_at
            )
            ",
            self.qualified_table("accounts")?,
        );
        self.run_query_with_params(&sql, &[param]).await?;
        Ok(rows.len())
    }

    async fn get_accounts(&self) -> Result<Vec<AccountRecord>> {
        let sql = format!(
            "
            SELECT
              account_id, owner, account_type, bank, label,
              pluggy_account_id, pluggy_item_id, status, actor_id, idempotency_key,
              TO_JSON_STRING(metadata_json),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at)
            FROM {}
            ORDER BY account_id
            ",
            self.qualified_table("accounts")?,
        );
        let response = self.run_query(&sql).await?;
        response
            .rows
            .iter()
            .map(|row| account_record_from_values(&row_values(row)))
            .collect()
    }

    async fn insert_account_snapshots(&self, rows: &[AccountSnapshotRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let param = batch_array_param(
            "batch",
            vec![
                ("snapshot_id", BqType::String),
                ("account_id", BqType::String),
                ("snapshot_date", BqType::Date),
                ("balance", BqType::Numeric),
                ("credit_limit", BqType::Numeric),
                ("currency_code", BqType::String),
                ("source", BqType::String),
                ("actor_id", BqType::String),
                ("idempotency_key", BqType::String),
                ("metadata_json", BqType::Json),
                ("created_at", BqType::Timestamp),
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        field("snapshot_id", bv_str(&r.snapshot_id)),
                        field("account_id", bv_str(&r.account_id)),
                        field("snapshot_date", bv_date(r.snapshot_date)),
                        field("balance", bv_opt_dec(r.balance)),
                        field("credit_limit", bv_opt_dec(r.credit_limit)),
                        field("currency_code", bv_opt_str(r.currency_code.as_deref())),
                        field("source", bv_str(&r.source)),
                        field("actor_id", bv_str(&r.actor_id)),
                        field("idempotency_key", bv_str(&r.idempotency_key)),
                        field("metadata_json", bv_json(&r.metadata_json)),
                        field("created_at", bv_ts(r.created_at)),
                    ]
                })
                .collect(),
        );

        let sql = format!(
            "
            MERGE {} target
            USING (SELECT * FROM UNNEST(@batch)) source
            ON target.idempotency_key = source.idempotency_key
            WHEN NOT MATCHED THEN INSERT (
              snapshot_id, account_id, snapshot_date, balance, credit_limit, currency_code,
              source, actor_id, idempotency_key, metadata_json, created_at
            ) VALUES (
              source.snapshot_id, source.account_id, source.snapshot_date, source.balance, source.credit_limit, source.currency_code,
              source.source, source.actor_id, source.idempotency_key, source.metadata_json, source.created_at
            )
            ",
            self.qualified_table("account_snapshots")?,
        );
        self.run_query_with_params(&sql, &[param]).await?;
        Ok(rows.len())
    }

    async fn latest_account_snapshots(&self) -> Result<Vec<AccountSnapshotRecord>> {
        let sql = format!(
            "
            WITH ranked AS (
              SELECT
                snapshot_id, account_id, snapshot_date,
                CAST(balance AS STRING) AS balance_str,
                CAST(credit_limit AS STRING) AS credit_limit_str,
                currency_code, source, actor_id, idempotency_key,
                TO_JSON_STRING(metadata_json) AS metadata_json,
                FORMAT_TIMESTAMP('%FT%T%Ez', created_at) AS created_at_str,
                ROW_NUMBER() OVER (
                  PARTITION BY account_id
                  ORDER BY snapshot_date DESC, created_at DESC
                ) AS rn
              FROM {}
            )
            SELECT snapshot_id, account_id, CAST(snapshot_date AS STRING),
                   balance_str, credit_limit_str, currency_code, source,
                   actor_id, idempotency_key, metadata_json, created_at_str
            FROM ranked
            WHERE rn = 1
            ORDER BY account_id
            ",
            self.qualified_table("account_snapshots")?,
        );
        let response = self.run_query(&sql).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            let metadata_str = optional_string(&values, 9).unwrap_or_else(|| "{}".to_string());
            let metadata_json: Value =
                serde_json::from_str(&metadata_str).unwrap_or(Value::Object(Default::default()));
            let created_str = required_string(&values, 10, "created_at")?;
            items.push(AccountSnapshotRecord {
                snapshot_id: required_string(&values, 0, "snapshot_id")?,
                account_id: required_string(&values, 1, "account_id")?,
                snapshot_date: optional_date(&values, 2, "snapshot_date")?
                    .ok_or_else(|| anyhow!("snapshot_date is null"))?,
                balance: optional_string(&values, 3).and_then(|s| Decimal::from_str(&s).ok()),
                credit_limit: optional_string(&values, 4).and_then(|s| Decimal::from_str(&s).ok()),
                currency_code: optional_string(&values, 5),
                source: required_string(&values, 6, "source")?,
                actor_id: required_string(&values, 7, "actor_id")?,
                idempotency_key: required_string(&values, 8, "idempotency_key")?,
                metadata_json,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
            });
        }
        Ok(items)
    }

    async fn find_transactions_by_description(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<TransactionRecord>> {
        let pattern = format!("%{}%", query.to_ascii_lowercase());
        let sql = format!(
            "
            SELECT
              transaction_id, account_id, CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''), description, merchant_name, purpose,
              CAST(amount AS STRING), tx_type, category_id, category_source, context,
              classifier_trace, payment_status, source, actor_id, idempotency_key,
              TO_JSON_STRING(metadata_json),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', enrichment_attempted_at)
            FROM {}
            WHERE LOWER(COALESCE(raw_description, '')) LIKE @pattern
               OR LOWER(COALESCE(description, '')) LIKE @pattern
               OR LOWER(COALESCE(merchant_name, '')) LIKE @pattern
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT @lim
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self
            .run_query_with_params(
                &sql,
                &[
                    Param::string("pattern", pattern),
                    Param::int64("lim", limit as i64),
                ],
            )
            .await?;
        response
            .rows
            .iter()
            .map(|row| transaction_record_from_values(&row_values(row)))
            .collect()
    }

    async fn latest_uncategorized_transactions(
        &self,
        limit: usize,
    ) -> Result<Vec<TransactionRecord>> {
        let sql = format!(
            "
            SELECT
              transaction_id, account_id, CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''), description, merchant_name, purpose,
              CAST(amount AS STRING), tx_type, category_id, category_source, context,
              classifier_trace, payment_status, source, actor_id, idempotency_key,
              TO_JSON_STRING(metadata_json),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', enrichment_attempted_at)
            FROM {}
            WHERE context IS NULL
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT @lim
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::int64("lim", limit as i64)])
            .await?;
        response
            .rows
            .iter()
            .map(|row| transaction_record_from_values(&row_values(row)))
            .collect()
    }

    async fn pending_human_descriptions(&self, limit: usize) -> Result<Vec<TransactionRecord>> {
        // Read from the effective view so transactions whose anatomy lives in
        // `transaction_split_lines` are surfaced instead of their (now hidden)
        // parents. Without this, splitting a transaction leaves the parent
        // stuck in the review queue with stale anatomy.
        let sql = format!(
            "
            SELECT
              transaction_id, account_id, CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''), description, merchant_name, purpose,
              CAST(amount AS STRING), tx_type, category_id, category_source, context,
              classifier_trace, payment_status, source, actor_id, idempotency_key,
              TO_JSON_STRING(metadata_json),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', enrichment_attempted_at)
            FROM {}
            WHERE (description IS NULL OR TRIM(description) = '')
              AND ABS(amount) > 0
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT @lim
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::int64("lim", limit as i64)])
            .await?;
        response
            .rows
            .iter()
            .map(|row| transaction_record_from_values(&row_values(row)))
            .collect()
    }

    async fn pending_merchants(&self, limit: usize) -> Result<Vec<TransactionRecord>> {
        let sql = format!(
            "
            SELECT
              transaction_id, account_id, CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''), description, merchant_name, purpose,
              CAST(amount AS STRING), tx_type, category_id, category_source, context,
              classifier_trace, payment_status, source, actor_id, idempotency_key,
              TO_JSON_STRING(metadata_json),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', enrichment_attempted_at)
            FROM {}
            WHERE (merchant_name IS NULL OR TRIM(merchant_name) = '')
              AND category_source != 'unclassified'
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT @lim
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::int64("lim", limit as i64)])
            .await?;
        response
            .rows
            .iter()
            .map(|row| transaction_record_from_values(&row_values(row)))
            .collect()
    }

    async fn pending_purposes(
        &self,
        min_abs_amount: Decimal,
        limit: usize,
    ) -> Result<Vec<TransactionRecord>> {
        let sql = format!(
            "
            SELECT
              transaction_id, account_id, CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''), description, merchant_name, purpose,
              CAST(amount AS STRING), tx_type, category_id, category_source, context,
              classifier_trace, payment_status, source, actor_id, idempotency_key,
              TO_JSON_STRING(metadata_json),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', enrichment_attempted_at)
            FROM {}
            WHERE (purpose IS NULL OR TRIM(purpose) = '')
              AND ABS(amount) >= @min_abs
              AND category_id IS NOT NULL
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT @lim
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self
            .run_query_with_params(
                &sql,
                &[
                    Param::decimal("min_abs", min_abs_amount.abs()),
                    Param::int64("lim", limit as i64),
                ],
            )
            .await?;
        response
            .rows
            .iter()
            .map(|row| transaction_record_from_values(&row_values(row)))
            .collect()
    }

    async fn count_pending_human_descriptions(&self) -> Result<i64> {
        let sql = format!(
            "
            SELECT CAST(COUNT(*) AS STRING)
            FROM {}
            WHERE (description IS NULL OR TRIM(description) = '')
              AND ABS(amount) > 0
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self.run_query(&sql).await?;
        let count = response
            .rows
            .first()
            .and_then(|row| row.f.first())
            .and_then(|cell| parse_scalar_string(&cell.v))
            .unwrap_or_else(|| "0".to_string());
        Ok(count.parse().unwrap_or(0))
    }

    async fn count_pending_merchants(&self) -> Result<i64> {
        let sql = format!(
            "
            SELECT CAST(COUNT(*) AS STRING)
            FROM {}
            WHERE (merchant_name IS NULL OR TRIM(merchant_name) = '')
              AND category_source != 'unclassified'
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self.run_query(&sql).await?;
        let count = response
            .rows
            .first()
            .and_then(|row| row.f.first())
            .and_then(|cell| parse_scalar_string(&cell.v))
            .unwrap_or_else(|| "0".to_string());
        Ok(count.parse().unwrap_or(0))
    }

    async fn count_pending_purposes(&self, min_abs_amount: Decimal) -> Result<i64> {
        let sql = format!(
            "
            SELECT CAST(COUNT(*) AS STRING)
            FROM {}
            WHERE (purpose IS NULL OR TRIM(purpose) = '')
              AND ABS(amount) >= @min_abs
              AND category_id IS NOT NULL
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::decimal("min_abs", min_abs_amount.abs())])
            .await?;
        let count = response
            .rows
            .first()
            .and_then(|row| row.f.first())
            .and_then(|cell| parse_scalar_string(&cell.v))
            .unwrap_or_else(|| "0".to_string());
        Ok(count.parse().unwrap_or(0))
    }

    async fn upsert_transactions(&self, rows: &[TransactionRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let param = batch_array_param(
            "batch",
            vec![
                ("transaction_id", BqType::String),
                ("account_id", BqType::String),
                ("transaction_date", BqType::Date),
                ("raw_description", BqType::String),
                ("description", BqType::String),
                ("merchant_name", BqType::String),
                ("purpose", BqType::String),
                ("amount", BqType::Numeric),
                ("amount_cents", BqType::Int64),
                ("tx_type", BqType::String),
                ("category_id", BqType::String),
                ("category_source", BqType::String),
                ("context", BqType::String),
                ("classifier_trace", BqType::String),
                ("payment_status", BqType::String),
                ("source", BqType::String),
                ("actor_id", BqType::String),
                ("idempotency_key", BqType::String),
                ("metadata_json", BqType::Json),
                ("created_at", BqType::Timestamp),
                ("updated_at", BqType::Timestamp),
                ("enrichment_attempted_at", BqType::Timestamp),
            ],
            rows.iter()
                .map(|r| {
                    let cents = (r.amount * Decimal::from(100_i64))
                        .round()
                        .to_i64()
                        .unwrap_or(0);
                    vec![
                        field("transaction_id", bv_str(&r.transaction_id)),
                        field("account_id", bv_opt_str(r.account_id.as_deref())),
                        field("transaction_date", bv_date(r.transaction_date)),
                        field("raw_description", bv_str(&r.raw_description)),
                        field("description", bv_opt_str(r.description.as_deref())),
                        field("merchant_name", bv_opt_str(r.merchant_name.as_deref())),
                        field("purpose", bv_opt_str(r.purpose.as_deref())),
                        field("amount", bv_dec(r.amount)),
                        field("amount_cents", bv_int(cents)),
                        field("tx_type", bv_str(&r.tx_type)),
                        field("category_id", bv_opt_str(r.category_id.as_deref())),
                        field("category_source", bv_str(&r.category_source)),
                        field("context", bv_opt_str(r.context.as_deref())),
                        field(
                            "classifier_trace",
                            bv_opt_str(r.classifier_trace.as_deref()),
                        ),
                        field("payment_status", bv_str(&r.payment_status)),
                        field("source", bv_str(&r.source)),
                        field("actor_id", bv_str(&r.actor_id)),
                        field("idempotency_key", bv_str(&r.idempotency_key)),
                        field("metadata_json", bv_json(&r.metadata_json)),
                        field("created_at", bv_ts(r.created_at)),
                        field("updated_at", bv_ts(r.updated_at)),
                        field(
                            "enrichment_attempted_at",
                            match r.enrichment_attempted_at {
                                Some(ts) => bv_ts(ts),
                                None => BqValue::Null,
                            },
                        ),
                    ]
                })
                .collect(),
        );

        let sql = format!(
            "
            MERGE {} target
            USING (SELECT * FROM UNNEST(@batch)) source
            ON target.transaction_id = source.transaction_id
            WHEN MATCHED THEN UPDATE SET
              account_id = source.account_id,
              transaction_date = source.transaction_date,
              raw_description = COALESCE(NULLIF(target.raw_description, ''), source.raw_description),
              description = IF(source.source = 'pluggy', COALESCE(target.description, source.description), source.description),
              merchant_name = COALESCE(target.merchant_name, source.merchant_name),
              purpose = COALESCE(target.purpose, source.purpose),
              amount = source.amount,
              amount_cents = source.amount_cents,
              tx_type = source.tx_type,
              category_id = IF(target.category_source = 'manual', target.category_id, source.category_id),
              category_source = IF(target.category_source = 'manual', target.category_source, source.category_source),
              classifier_trace = IF(target.category_source = 'manual', target.classifier_trace, source.classifier_trace),
              payment_status = source.payment_status,
              source = source.source,
              actor_id = source.actor_id,
              idempotency_key = source.idempotency_key,
              metadata_json = source.metadata_json,
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              transaction_id, account_id, transaction_date, raw_description, description,
              merchant_name, purpose, amount, amount_cents, tx_type, category_id,
              category_source, context, classifier_trace, payment_status, source, actor_id,
              idempotency_key, metadata_json, created_at, updated_at, enrichment_attempted_at
            ) VALUES (
              source.transaction_id, source.account_id, source.transaction_date, source.raw_description,
              source.description, source.merchant_name, source.purpose, source.amount,
              source.amount_cents, source.tx_type, source.category_id, source.category_source,
              source.context, source.classifier_trace, source.payment_status, source.source,
              source.actor_id, source.idempotency_key, source.metadata_json, source.created_at,
              source.updated_at, source.enrichment_attempted_at
            )
            ",
            self.qualified_table("transactions")?,
        );
        self.run_query_with_params(&sql, &[param]).await?;
        Ok(rows.len())
    }

    async fn upsert_rules(&self, rows: &[RuleRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let param = batch_array_param(
            "batch",
            vec![
                ("rule_id", BqType::String),
                ("body", BqType::String),
                ("status", BqType::String),
                ("actor_id", BqType::String),
                ("idempotency_key", BqType::String),
                ("created_at", BqType::Timestamp),
                ("updated_at", BqType::Timestamp),
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        field("rule_id", bv_str(&r.rule_id)),
                        field("body", bv_str(&r.body)),
                        field("status", bv_str(&r.status)),
                        field("actor_id", bv_str(&r.actor_id)),
                        field("idempotency_key", bv_str(&r.idempotency_key)),
                        field("created_at", bv_ts(r.created_at)),
                        field("updated_at", bv_ts(r.updated_at)),
                    ]
                })
                .collect(),
        );
        let sql = format!(
            "
            MERGE {} target
            USING (SELECT * FROM UNNEST(@batch)) source
            ON target.rule_id = source.rule_id
            WHEN MATCHED THEN UPDATE SET
              body = source.body,
              status = source.status,
              actor_id = source.actor_id,
              idempotency_key = source.idempotency_key,
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              rule_id, body, status, actor_id, idempotency_key, created_at, updated_at
            ) VALUES (
              source.rule_id, source.body, source.status, source.actor_id, source.idempotency_key, source.created_at, source.updated_at
            )
            ",
            self.qualified_table("rules")?,
        );
        self.run_query_with_params(&sql, &[param]).await?;
        Ok(rows.len())
    }

    async fn upsert_categories(&self, rows: &[CategoryRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let param = batch_array_param(
            "batch",
            vec![
                ("category_id", BqType::String),
                ("name", BqType::String),
                ("parent_category_id", BqType::String),
                ("metadata_json", BqType::Json),
                ("actor_id", BqType::String),
                ("updated_at", BqType::Timestamp),
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        field("category_id", bv_str(&r.category_id)),
                        field("name", bv_str(&r.name)),
                        field(
                            "parent_category_id",
                            bv_opt_str(r.parent_category_id.as_deref()),
                        ),
                        field("metadata_json", bv_json(&r.metadata_json)),
                        field("actor_id", bv_str(&r.actor_id)),
                        field("updated_at", bv_ts(r.updated_at)),
                    ]
                })
                .collect(),
        );
        let sql = format!(
            "
            MERGE {} target
            USING (SELECT * FROM UNNEST(@batch)) source
            ON target.category_id = source.category_id
            WHEN MATCHED THEN UPDATE SET
              name = source.name,
              parent_category_id = source.parent_category_id,
              metadata_json = source.metadata_json,
              actor_id = source.actor_id,
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              category_id, name, parent_category_id, metadata_json, actor_id, updated_at
            ) VALUES (
              source.category_id, source.name, source.parent_category_id, source.metadata_json, source.actor_id, source.updated_at
            )
            ",
            self.qualified_table("categories")?,
        );
        self.run_query_with_params(&sql, &[param]).await?;
        Ok(rows.len())
    }

    async fn upsert_forecasts(&self, rows: &[ForecastRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let param = batch_array_param(
            "batch",
            vec![
                ("forecast_id", BqType::String),
                ("due_date", BqType::Date),
                ("description", BqType::String),
                ("amount", BqType::Numeric),
                ("category_id", BqType::String),
                ("account_id", BqType::String),
                ("status", BqType::String),
                ("recurrence", BqType::String),
                ("actor_id", BqType::String),
                ("idempotency_key", BqType::String),
                ("metadata_json", BqType::Json),
                ("created_at", BqType::Timestamp),
                ("updated_at", BqType::Timestamp),
                ("template_id", BqType::String),
                ("realized_transaction_id", BqType::String),
                ("realized_at", BqType::Timestamp),
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        field("forecast_id", bv_str(&r.forecast_id)),
                        field("due_date", bv_opt_date(r.due_date)),
                        field("description", bv_str(&r.description)),
                        field("amount", bv_dec(r.amount)),
                        field("category_id", bv_opt_str(r.category_id.as_deref())),
                        field("account_id", bv_opt_str(r.account_id.as_deref())),
                        field("status", bv_str(&r.status)),
                        field("recurrence", bv_opt_str(r.recurrence.as_deref())),
                        field("actor_id", bv_str(&r.actor_id)),
                        field("idempotency_key", bv_str(&r.idempotency_key)),
                        field("metadata_json", bv_json(&r.metadata_json)),
                        field("created_at", bv_ts(r.created_at)),
                        field("updated_at", bv_ts(r.updated_at)),
                        field("template_id", bv_opt_str(r.template_id.as_deref())),
                        field(
                            "realized_transaction_id",
                            bv_opt_str(r.realized_transaction_id.as_deref()),
                        ),
                        field(
                            "realized_at",
                            match r.realized_at {
                                Some(ts) => bv_ts(ts),
                                None => BqValue::Null,
                            },
                        ),
                    ]
                })
                .collect(),
        );
        let sql = format!(
            "
            MERGE {} target
            USING (SELECT * FROM UNNEST(@batch)) source
            ON target.forecast_id = source.forecast_id
            WHEN MATCHED THEN UPDATE SET
              due_date = source.due_date,
              description = source.description,
              amount = source.amount,
              category_id = source.category_id,
              account_id = source.account_id,
              status = source.status,
              recurrence = source.recurrence,
              actor_id = source.actor_id,
              idempotency_key = source.idempotency_key,
              metadata_json = source.metadata_json,
              updated_at = source.updated_at,
              template_id = source.template_id,
              realized_transaction_id = source.realized_transaction_id,
              realized_at = source.realized_at
            WHEN NOT MATCHED THEN INSERT (
              forecast_id, due_date, description, amount, category_id, account_id, status,
              recurrence, actor_id, idempotency_key, metadata_json, created_at, updated_at,
              template_id, realized_transaction_id, realized_at
            ) VALUES (
              source.forecast_id, source.due_date, source.description, source.amount, source.category_id, source.account_id, source.status,
              source.recurrence, source.actor_id, source.idempotency_key, source.metadata_json, source.created_at, source.updated_at,
              source.template_id, source.realized_transaction_id, source.realized_at
            )
            ",
            self.qualified_table("forecast")?,
        );
        self.run_query_with_params(&sql, &[param]).await?;
        Ok(rows.len())
    }

    async fn upsert_forecast_templates(&self, rows: &[ForecastTemplateRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let param = batch_array_param(
            "batch",
            vec![
                ("template_id", BqType::String),
                ("kind", BqType::String),
                ("description", BqType::String),
                ("merchant_pattern", BqType::String),
                ("category_id", BqType::String),
                ("account_id", BqType::String),
                ("amount", BqType::Numeric),
                ("amount_lower", BqType::Numeric),
                ("amount_upper", BqType::Numeric),
                ("cadence", BqType::String),
                ("next_due_day", BqType::Int64),
                ("start_date", BqType::Date),
                ("end_date", BqType::Date),
                ("remaining_count", BqType::Int64),
                ("source", BqType::String),
                ("confidence", BqType::Float64),
                ("status", BqType::String),
                ("metadata_json", BqType::Json),
                ("actor_id", BqType::String),
                ("idempotency_key", BqType::String),
                ("created_at", BqType::Timestamp),
                ("updated_at", BqType::Timestamp),
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        field("template_id", bv_str(&r.template_id)),
                        field("kind", bv_str(&r.kind)),
                        field("description", bv_str(&r.description)),
                        field(
                            "merchant_pattern",
                            bv_opt_str(r.merchant_pattern.as_deref()),
                        ),
                        field("category_id", bv_opt_str(r.category_id.as_deref())),
                        field("account_id", bv_opt_str(r.account_id.as_deref())),
                        field("amount", bv_dec(r.amount)),
                        field("amount_lower", bv_opt_dec(r.amount_lower)),
                        field("amount_upper", bv_opt_dec(r.amount_upper)),
                        field("cadence", bv_str(&r.cadence)),
                        field("next_due_day", bv_opt_int(r.next_due_day.map(|d| d as i64))),
                        field("start_date", bv_date(r.start_date)),
                        field("end_date", bv_opt_date(r.end_date)),
                        field(
                            "remaining_count",
                            bv_opt_int(r.remaining_count.map(|c| c as i64)),
                        ),
                        field("source", bv_str(&r.source)),
                        field("confidence", bv_opt_float(r.confidence)),
                        field("status", bv_str(&r.status)),
                        field("metadata_json", bv_json(&r.metadata_json)),
                        field("actor_id", bv_str(&r.actor_id)),
                        field("idempotency_key", bv_str(&r.idempotency_key)),
                        field("created_at", bv_ts(r.created_at)),
                        field("updated_at", bv_ts(r.updated_at)),
                    ]
                })
                .collect(),
        );
        let sql = format!(
            "
            MERGE {} target
            USING (SELECT * FROM UNNEST(@batch)) source
            ON target.template_id = source.template_id
            WHEN MATCHED THEN UPDATE SET
              kind = source.kind,
              description = source.description,
              merchant_pattern = source.merchant_pattern,
              category_id = source.category_id,
              account_id = source.account_id,
              amount = source.amount,
              amount_lower = source.amount_lower,
              amount_upper = source.amount_upper,
              cadence = source.cadence,
              next_due_day = source.next_due_day,
              start_date = source.start_date,
              end_date = source.end_date,
              remaining_count = source.remaining_count,
              source = source.source,
              confidence = source.confidence,
              status = source.status,
              metadata_json = source.metadata_json,
              actor_id = source.actor_id,
              idempotency_key = source.idempotency_key,
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              template_id, kind, description, merchant_pattern, category_id, account_id,
              amount, amount_lower, amount_upper, cadence, next_due_day,
              start_date, end_date, remaining_count, source, confidence,
              status, metadata_json, actor_id, idempotency_key, created_at, updated_at
            ) VALUES (
              source.template_id, source.kind, source.description, source.merchant_pattern, source.category_id, source.account_id,
              source.amount, source.amount_lower, source.amount_upper, source.cadence, source.next_due_day,
              source.start_date, source.end_date, source.remaining_count, source.source, source.confidence,
              source.status, source.metadata_json, source.actor_id, source.idempotency_key, source.created_at, source.updated_at
            )
            ",
            self.qualified_table("forecast_template")?,
        );
        self.run_query_with_params(&sql, &[param]).await?;
        Ok(rows.len())
    }

    async fn list_forecast_templates(
        &self,
        kind: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<ForecastTemplateRecord>> {
        let mut filters = Vec::new();
        let mut params = Vec::new();
        if let Some(k) = kind {
            filters.push("kind = @kind");
            params.push(Param::string("kind", k));
        }
        if let Some(s) = status {
            filters.push("status = @status");
            params.push(Param::string("status", s));
        }
        let where_sql = if filters.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", filters.join(" AND "))
        };
        let sql = format!(
            "
            SELECT
              template_id, kind, description, merchant_pattern, category_id, account_id,
              CAST(amount AS STRING), CAST(amount_lower AS STRING), CAST(amount_upper AS STRING),
              cadence, next_due_day,
              CAST(start_date AS STRING), CAST(end_date AS STRING), remaining_count,
              source, confidence, status, TO_JSON_STRING(metadata_json),
              actor_id, idempotency_key,
              FORMAT_TIMESTAMP('%FT%T%Ez', created_at),
              FORMAT_TIMESTAMP('%FT%T%Ez', updated_at)
            FROM {}
            {where_sql}
            ORDER BY created_at DESC
            ",
            self.qualified_table("forecast_template")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(forecast_template_from_bq(&values)?);
        }
        Ok(items)
    }

    async fn get_forecast_template(
        &self,
        template_id: &str,
    ) -> Result<Option<ForecastTemplateRecord>> {
        let sql = format!(
            "
            SELECT
              template_id, kind, description, merchant_pattern, category_id, account_id,
              CAST(amount AS STRING), CAST(amount_lower AS STRING), CAST(amount_upper AS STRING),
              cadence, next_due_day,
              CAST(start_date AS STRING), CAST(end_date AS STRING), remaining_count,
              source, confidence, status, TO_JSON_STRING(metadata_json),
              actor_id, idempotency_key,
              FORMAT_TIMESTAMP('%FT%T%Ez', created_at),
              FORMAT_TIMESTAMP('%FT%T%Ez', updated_at)
            FROM {}
            WHERE template_id = @tid
            LIMIT 1
            ",
            self.qualified_table("forecast_template")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::string("tid", template_id)])
            .await?;
        let Some(row) = response.rows.into_iter().next() else {
            return Ok(None);
        };
        let values = row_values(&row);
        Ok(Some(forecast_template_from_bq(&values)?))
    }

    async fn upcoming_forecasts(
        &self,
        from: NaiveDate,
        until: NaiveDate,
    ) -> Result<Vec<ForecastRecord>> {
        let sql = format!(
            "
            SELECT
              forecast_id,
              CAST(due_date AS STRING),
              description,
              CAST(amount AS STRING),
              category_id,
              account_id,
              status,
              recurrence,
              actor_id,
              idempotency_key,
              TO_JSON_STRING(metadata_json),
              FORMAT_TIMESTAMP('%FT%T%Ez', created_at),
              FORMAT_TIMESTAMP('%FT%T%Ez', updated_at),
              template_id,
              realized_transaction_id,
              FORMAT_TIMESTAMP('%FT%T%Ez', realized_at)
            FROM {}
            WHERE LOWER(status) IN ('ativo', 'active')
              AND due_date IS NOT NULL
              AND due_date BETWEEN @from AND @until
            ORDER BY due_date ASC, amount DESC
            ",
            self.qualified_table("forecast")?,
        );
        let response = self
            .run_query_with_params(
                &sql,
                &[Param::date("from", from), Param::date("until", until)],
            )
            .await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            let metadata_str = optional_string(&values, 10).unwrap_or_else(|| "{}".to_string());
            let metadata_json: Value =
                serde_json::from_str(&metadata_str).unwrap_or(Value::Object(Default::default()));
            let created_str = required_string(&values, 11, "created_at")?;
            let updated_str = required_string(&values, 12, "updated_at")?;
            items.push(ForecastRecord {
                forecast_id: required_string(&values, 0, "forecast_id")?,
                due_date: optional_date(&values, 1, "due_date")?,
                description: required_string(&values, 2, "description")?,
                amount: required_decimal(&values, 3, "amount")?,
                category_id: optional_string(&values, 4),
                account_id: optional_string(&values, 5),
                status: required_string(&values, 6, "status")?,
                recurrence: optional_string(&values, 7),
                actor_id: required_string(&values, 8, "actor_id")?,
                idempotency_key: required_string(&values, 9, "idempotency_key")?,
                metadata_json,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                template_id: optional_string(&values, 13),
                realized_transaction_id: optional_string(&values, 14),
                realized_at: optional_string(&values, 15).and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .ok()
                }),
            });
        }
        Ok(items)
    }

    async fn list_forecasts(
        &self,
        status: Option<&str>,
        from: Option<NaiveDate>,
        until: Option<NaiveDate>,
    ) -> Result<Vec<ForecastRecord>> {
        let mut filters = Vec::new();
        let mut params = Vec::new();
        if let Some(s) = status {
            filters.push("status = @status");
            params.push(Param::string("status", s));
        }
        if let Some(d) = from {
            filters.push("due_date >= @from");
            params.push(Param::date("from", d));
        }
        if let Some(d) = until {
            filters.push("due_date <= @until");
            params.push(Param::date("until", d));
        }
        let where_sql = if filters.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", filters.join(" AND "))
        };
        let sql = format!(
            "
            SELECT
              forecast_id,
              CAST(due_date AS STRING),
              description,
              CAST(amount AS STRING),
              category_id,
              account_id,
              status,
              recurrence,
              actor_id,
              idempotency_key,
              TO_JSON_STRING(metadata_json),
              FORMAT_TIMESTAMP('%FT%T%Ez', created_at),
              FORMAT_TIMESTAMP('%FT%T%Ez', updated_at),
              template_id,
              realized_transaction_id,
              FORMAT_TIMESTAMP('%FT%T%Ez', realized_at)
            FROM {}
            {where_sql}
            ORDER BY due_date ASC, amount DESC
            ",
            self.qualified_table("forecast")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            let metadata_str = optional_string(&values, 10).unwrap_or_else(|| "{}".to_string());
            let metadata_json: Value =
                serde_json::from_str(&metadata_str).unwrap_or(Value::Object(Default::default()));
            let created_str = required_string(&values, 11, "created_at")?;
            let updated_str = required_string(&values, 12, "updated_at")?;
            items.push(ForecastRecord {
                forecast_id: required_string(&values, 0, "forecast_id")?,
                due_date: optional_date(&values, 1, "due_date")?,
                description: required_string(&values, 2, "description")?,
                amount: required_decimal(&values, 3, "amount")?,
                category_id: optional_string(&values, 4),
                account_id: optional_string(&values, 5),
                status: required_string(&values, 6, "status")?,
                recurrence: optional_string(&values, 7),
                actor_id: required_string(&values, 8, "actor_id")?,
                idempotency_key: required_string(&values, 9, "idempotency_key")?,
                metadata_json,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                template_id: optional_string(&values, 13),
                realized_transaction_id: optional_string(&values, 14),
                realized_at: optional_string(&values, 15).and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .ok()
                }),
            });
        }
        Ok(items)
    }

    async fn get_forecast(&self, forecast_id: &str) -> Result<Option<ForecastRecord>> {
        let sql = format!(
            "
            SELECT
              forecast_id,
              CAST(due_date AS STRING),
              description,
              CAST(amount AS STRING),
              category_id,
              account_id,
              status,
              recurrence,
              actor_id,
              idempotency_key,
              TO_JSON_STRING(metadata_json),
              FORMAT_TIMESTAMP('%FT%T%Ez', created_at),
              FORMAT_TIMESTAMP('%FT%T%Ez', updated_at),
              template_id,
              realized_transaction_id,
              FORMAT_TIMESTAMP('%FT%T%Ez', realized_at)
            FROM {}
            WHERE forecast_id = @forecast_id
            ",
            self.qualified_table("forecast")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::string("forecast_id", forecast_id)])
            .await?;
        if response.rows.is_empty() {
            return Ok(None);
        }
        let values = row_values(&response.rows[0]);
        let metadata_str = optional_string(&values, 10).unwrap_or_else(|| "{}".to_string());
        let metadata_json: Value =
            serde_json::from_str(&metadata_str).unwrap_or(Value::Object(Default::default()));
        let created_str = required_string(&values, 11, "created_at")?;
        let updated_str = required_string(&values, 12, "updated_at")?;
        Ok(Some(ForecastRecord {
            forecast_id: required_string(&values, 0, "forecast_id")?,
            due_date: optional_date(&values, 1, "due_date")?,
            description: required_string(&values, 2, "description")?,
            amount: required_decimal(&values, 3, "amount")?,
            category_id: optional_string(&values, 4),
            account_id: optional_string(&values, 5),
            status: required_string(&values, 6, "status")?,
            recurrence: optional_string(&values, 7),
            actor_id: required_string(&values, 8, "actor_id")?,
            idempotency_key: required_string(&values, 9, "idempotency_key")?,
            metadata_json,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
            template_id: optional_string(&values, 13),
            realized_transaction_id: optional_string(&values, 14),
            realized_at: optional_string(&values, 15).and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .ok()
            }),
        }))
    }

    async fn get_categories(&self) -> Result<Vec<CategoryRecord>> {
        let sql = format!(
            "
            SELECT
              category_id,
              name,
              parent_category_id,
              TO_JSON_STRING(metadata_json),
              actor_id,
              FORMAT_TIMESTAMP('%FT%T%Ez', updated_at)
            FROM {}
            ORDER BY name ASC
            ",
            self.qualified_table("categories")?,
        );
        let response = self.run_query(&sql).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            let metadata_str = optional_string(&values, 3).unwrap_or_else(|| "{}".to_string());
            let metadata_json: Value =
                serde_json::from_str(&metadata_str).unwrap_or(Value::Object(Default::default()));
            let updated_str = required_string(&values, 5, "updated_at")?;
            items.push(CategoryRecord {
                category_id: required_string(&values, 0, "category_id")?,
                name: required_string(&values, 1, "name")?,
                parent_category_id: optional_string(&values, 2),
                metadata_json,
                actor_id: required_string(&values, 4, "actor_id")?,
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
            });
        }
        Ok(items)
    }

    async fn apply_transaction_split(
        &self,
        split: &TransactionSplitRecord,
        lines: &[TransactionSplitLineRecord],
        items: &[ReceiptItemRecord],
    ) -> Result<()> {
        if lines.is_empty() {
            return Err(anyhow!("Split precisa ter pelo menos uma linha"));
        }

        let split_param = batch_array_param(
            "splits",
            vec![
                ("split_id", BqType::String),
                ("parent_transaction_id", BqType::String),
                ("payload_hash", BqType::String),
                ("status", BqType::String),
                ("source", BqType::String),
                ("notes", BqType::String),
                ("actor_id", BqType::String),
                ("idempotency_key", BqType::String),
                ("metadata_json", BqType::Json),
                ("created_at", BqType::Timestamp),
                ("updated_at", BqType::Timestamp),
            ],
            vec![vec![
                field("split_id", bv_str(&split.split_id)),
                field(
                    "parent_transaction_id",
                    bv_str(&split.parent_transaction_id),
                ),
                field("payload_hash", bv_str(&split.payload_hash)),
                field("status", bv_str(&split.status)),
                field("source", bv_str(&split.source)),
                field("notes", bv_opt_str(split.notes.as_deref())),
                field("actor_id", bv_str(&split.actor_id)),
                field("idempotency_key", bv_str(&split.idempotency_key)),
                field("metadata_json", bv_json(&split.metadata_json)),
                field("created_at", bv_ts(split.created_at)),
                field("updated_at", bv_ts(split.updated_at)),
            ]],
        );

        let lines_param = batch_array_param(
            "lines",
            vec![
                ("split_line_id", BqType::String),
                ("split_id", BqType::String),
                ("parent_transaction_id", BqType::String),
                ("line_index", BqType::Int64),
                ("description", BqType::String),
                ("amount", BqType::Numeric),
                ("category_id", BqType::String),
                ("category_source", BqType::String),
                ("context", BqType::String),
                ("status", BqType::String),
                ("actor_id", BqType::String),
                ("idempotency_key", BqType::String),
                ("metadata_json", BqType::Json),
                ("created_at", BqType::Timestamp),
                ("updated_at", BqType::Timestamp),
            ],
            lines
                .iter()
                .map(|r| {
                    vec![
                        field("split_line_id", bv_str(&r.split_line_id)),
                        field("split_id", bv_str(&r.split_id)),
                        field("parent_transaction_id", bv_str(&r.parent_transaction_id)),
                        field("line_index", bv_int(r.line_index)),
                        field("description", bv_str(&r.description)),
                        field("amount", bv_dec(r.amount)),
                        field("category_id", bv_opt_str(r.category_id.as_deref())),
                        field("category_source", bv_str(&r.category_source)),
                        field("context", bv_opt_str(r.context.as_deref())),
                        field("status", bv_str(&r.status)),
                        field("actor_id", bv_str(&r.actor_id)),
                        field("idempotency_key", bv_str(&r.idempotency_key)),
                        field("metadata_json", bv_json(&r.metadata_json)),
                        field("created_at", bv_ts(r.created_at)),
                        field("updated_at", bv_ts(r.updated_at)),
                    ]
                })
                .collect(),
        );

        let mut params = vec![
            Param::string("parent_id", &split.parent_transaction_id),
            Param::string("split_id", &split.split_id),
            split_param,
            lines_param,
        ];

        let item_statement = if items.is_empty() {
            String::new()
        } else {
            let items_param = batch_array_param(
                "items",
                vec![
                    ("receipt_item_id", BqType::String),
                    ("parent_transaction_id", BqType::String),
                    ("split_id", BqType::String),
                    ("split_line_id", BqType::String),
                    ("item_index", BqType::Int64),
                    ("description", BqType::String),
                    ("quantity", BqType::Numeric),
                    ("unit", BqType::String),
                    ("unit_price", BqType::Numeric),
                    ("total_price", BqType::Numeric),
                    ("code", BqType::String),
                    ("store_name", BqType::String),
                    ("status", BqType::String),
                    ("actor_id", BqType::String),
                    ("idempotency_key", BqType::String),
                    ("metadata_json", BqType::Json),
                    ("created_at", BqType::Timestamp),
                    ("updated_at", BqType::Timestamp),
                ],
                items
                    .iter()
                    .map(|r| {
                        vec![
                            field("receipt_item_id", bv_str(&r.receipt_item_id)),
                            field("parent_transaction_id", bv_str(&r.parent_transaction_id)),
                            field("split_id", bv_opt_str(r.split_id.as_deref())),
                            field("split_line_id", bv_opt_str(r.split_line_id.as_deref())),
                            field("item_index", bv_int(r.item_index)),
                            field("description", bv_str(&r.description)),
                            field("quantity", bv_opt_dec(r.quantity)),
                            field("unit", bv_opt_str(r.unit.as_deref())),
                            field("unit_price", bv_opt_dec(r.unit_price)),
                            field("total_price", bv_opt_dec(r.total_price)),
                            field("code", bv_opt_str(r.code.as_deref())),
                            field("store_name", bv_opt_str(r.store_name.as_deref())),
                            field("status", bv_str(&r.status)),
                            field("actor_id", bv_str(&r.actor_id)),
                            field("idempotency_key", bv_str(&r.idempotency_key)),
                            field("metadata_json", bv_json(&r.metadata_json)),
                            field("created_at", bv_ts(r.created_at)),
                            field("updated_at", bv_ts(r.updated_at)),
                        ]
                    })
                    .collect(),
            );
            params.push(items_param);
            format!(
                "
                MERGE {} target
                USING (SELECT * FROM UNNEST(@items)) source
                ON target.receipt_item_id = source.receipt_item_id
                WHEN MATCHED THEN UPDATE SET
                  parent_transaction_id = source.parent_transaction_id,
                  split_id = source.split_id,
                  split_line_id = source.split_line_id,
                  item_index = source.item_index,
                  description = source.description,
                  quantity = source.quantity,
                  unit = source.unit,
                  unit_price = source.unit_price,
                  total_price = source.total_price,
                  code = source.code,
                  store_name = source.store_name,
                  status = source.status,
                  actor_id = source.actor_id,
                  idempotency_key = source.idempotency_key,
                  metadata_json = source.metadata_json,
                  updated_at = source.updated_at
                WHEN NOT MATCHED THEN INSERT (
                  receipt_item_id, parent_transaction_id, split_id, split_line_id, item_index,
                  description, quantity, unit, unit_price, total_price, code, store_name, status,
                  actor_id, idempotency_key, metadata_json, created_at, updated_at
                ) VALUES (
                  source.receipt_item_id, source.parent_transaction_id, source.split_id, source.split_line_id, source.item_index,
                  source.description, source.quantity, source.unit, source.unit_price, source.total_price, source.code, source.store_name, source.status,
                  source.actor_id, source.idempotency_key, source.metadata_json, source.created_at, source.updated_at
                );
                ",
                self.qualified_table("receipt_items")?,
            )
        };

        let sql = format!(
            "
            UPDATE {}
            SET status = 'inactive', updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = @parent_id
              AND status = 'active'
              AND split_id != @split_id;

            UPDATE {}
            SET status = 'inactive', updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = @parent_id
              AND status = 'active'
              AND split_id != @split_id;

            UPDATE {}
            SET status = 'inactive', updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = @parent_id
              AND status = 'active'
              AND COALESCE(split_id, '') != @split_id;

            MERGE {} target
            USING (SELECT * FROM UNNEST(@splits)) source
            ON target.split_id = source.split_id
            WHEN MATCHED THEN UPDATE SET
              parent_transaction_id = source.parent_transaction_id,
              payload_hash = source.payload_hash,
              status = source.status,
              source = source.source,
              notes = source.notes,
              actor_id = source.actor_id,
              idempotency_key = source.idempotency_key,
              metadata_json = source.metadata_json,
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              split_id, parent_transaction_id, payload_hash, status, source, notes, actor_id,
              idempotency_key, metadata_json, created_at, updated_at
            ) VALUES (
              source.split_id, source.parent_transaction_id, source.payload_hash, source.status, source.source, source.notes, source.actor_id,
              source.idempotency_key, source.metadata_json, source.created_at, source.updated_at
            );

            MERGE {} target
            USING (SELECT * FROM UNNEST(@lines)) source
            ON target.split_line_id = source.split_line_id
            WHEN MATCHED THEN UPDATE SET
              split_id = source.split_id,
              parent_transaction_id = source.parent_transaction_id,
              line_index = source.line_index,
              description = source.description,
              amount = source.amount,
              category_id = source.category_id,
              category_source = source.category_source,
              context = source.context,
              status = source.status,
              actor_id = source.actor_id,
              idempotency_key = source.idempotency_key,
              metadata_json = source.metadata_json,
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              split_line_id, split_id, parent_transaction_id, line_index, description, amount,
              category_id, category_source, context, status, actor_id, idempotency_key,
              metadata_json, created_at, updated_at
            ) VALUES (
              source.split_line_id, source.split_id, source.parent_transaction_id, source.line_index, source.description, source.amount,
              source.category_id, source.category_source, source.context, source.status, source.actor_id, source.idempotency_key,
              source.metadata_json, source.created_at, source.updated_at
            );

            {item_statement}
            ",
            self.qualified_table("transaction_splits")?,
            self.qualified_table("transaction_split_lines")?,
            self.qualified_table("receipt_items")?,
            self.qualified_table("transaction_splits")?,
            self.qualified_table("transaction_split_lines")?,
        );
        self.run_query_with_params(&sql, &params).await?;
        Ok(())
    }

    async fn insert_audit_events(&self, rows: &[AuditEvent]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let param = batch_array_param(
            "batch",
            vec![
                ("event_id", BqType::String),
                ("entity_type", BqType::String),
                ("entity_id", BqType::String),
                ("action", BqType::String),
                ("actor_id", BqType::String),
                ("event_timestamp", BqType::Timestamp),
                ("idempotency_key", BqType::String),
                ("diff_json", BqType::Json),
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        field("event_id", bv_str(&r.event_id)),
                        field("entity_type", bv_str(&r.entity_type)),
                        field("entity_id", bv_str(&r.entity_id)),
                        field("action", bv_str(&r.action)),
                        field("actor_id", bv_str(&r.actor_id)),
                        field("event_timestamp", bv_ts(r.event_timestamp)),
                        field("idempotency_key", bv_str(&r.idempotency_key)),
                        field("diff_json", bv_json(&r.diff_json)),
                    ]
                })
                .collect(),
        );
        let sql = format!(
            "
            INSERT INTO {} (event_id, entity_type, entity_id, action, actor_id, event_timestamp, idempotency_key, diff_json)
            SELECT event_id, entity_type, entity_id, action, actor_id, event_timestamp, idempotency_key, diff_json
            FROM UNNEST(@batch)
            ",
            self.qualified_table("audit_log")?,
        );
        self.run_query_with_params(&sql, &[param]).await?;
        Ok(rows.len())
    }

    async fn annotate_transaction(
        &self,
        transaction_id: &str,
        category_id: Option<&str>,
        category_source: Option<&str>,
        classifier_trace: Option<&str>,
        actor_id: &str,
        idempotency_key: &str,
    ) -> Result<()> {
        // If this id points at a split line, route the category update to
        // `transaction_split_lines` so the change is visible via the effective
        // view. `transaction_split_lines` has no `classifier_trace` column —
        // the view derives it from `context` for child rows — so we fold the
        // trace into `context` when no explicit context was provided.
        if self
            .resolve_split_line_target(transaction_id)
            .await?
            .is_some()
        {
            let sql = format!(
                "
                UPDATE {}
                SET category_id = COALESCE(@category_id, category_id),
                    category_source = COALESCE(@category_source, category_source),
                    context = COALESCE(@classifier_trace, context),
                    actor_id = @actor_id,
                    idempotency_key = @idempotency_key,
                    updated_at = CURRENT_TIMESTAMP()
                WHERE split_line_id = @transaction_id
                  AND status IN ('active', 'confirmed')
                ",
                self.qualified_table("transaction_split_lines")?,
            );
            let resp = self
                .run_query_with_params(
                    &sql,
                    &[
                        Param::optional_string("category_id", category_id),
                        Param::optional_string("category_source", category_source),
                        Param::optional_string("classifier_trace", classifier_trace),
                        Param::string("actor_id", actor_id),
                        Param::string("idempotency_key", idempotency_key),
                        Param::string("transaction_id", transaction_id),
                    ],
                )
                .await?;
            let affected: i64 = resp
                .num_dml_affected_rows
                .as_deref()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
            if affected == 0 {
                anyhow::bail!("Linha de split {transaction_id} não encontrada");
            }
            return Ok(());
        }

        let sql = format!(
            "
            UPDATE {}
            SET category_id = COALESCE(@category_id, category_id),
                category_source = COALESCE(@category_source, category_source),
                classifier_trace = COALESCE(@classifier_trace, classifier_trace),
                actor_id = @actor_id,
                idempotency_key = @idempotency_key,
                updated_at = CURRENT_TIMESTAMP()
            WHERE transaction_id = @transaction_id
            ",
            self.qualified_table("transactions")?,
        );
        let resp = self
            .run_query_with_params(
                &sql,
                &[
                    Param::optional_string("category_id", category_id),
                    Param::optional_string("category_source", category_source),
                    Param::optional_string("classifier_trace", classifier_trace),
                    Param::string("actor_id", actor_id),
                    Param::string("idempotency_key", idempotency_key),
                    Param::string("transaction_id", transaction_id),
                ],
            )
            .await?;
        let affected: i64 = resp
            .num_dml_affected_rows
            .as_deref()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);
        if affected == 0 {
            anyhow::bail!("Transação {transaction_id} não encontrada");
        }
        Ok(())
    }

    async fn update_transaction_anatomy(
        &self,
        transaction_id: &str,
        patch: TransactionAnatomyPatch<'_>,
        actor_id: &str,
        idempotency_key: &str,
    ) -> Result<()> {
        // Split-line routing: `description` and `context` live on the split
        // line itself; `merchant_name` and `purpose` are receipt-level so they
        // remain on the parent (shared across siblings). `classifier_trace`
        // has no dedicated column on `transaction_split_lines`, so when no
        // explicit context is provided we fold it into the line's `context`
        // (mirroring how the view derives `classifier_trace` from `sl.context`
        // for child rows).
        if let Some(parent_id) = self.resolve_split_line_target(transaction_id).await? {
            let line_context = patch.context.or(patch.classifier_trace);
            let line_sql = format!(
                "
                UPDATE {}
                SET description = COALESCE(@description, description),
                    context = COALESCE(@context, context),
                    actor_id = @actor_id,
                    idempotency_key = @idempotency_key,
                    updated_at = CURRENT_TIMESTAMP()
                WHERE split_line_id = @transaction_id
                  AND status IN ('active', 'confirmed')
                ",
                self.qualified_table("transaction_split_lines")?,
            );
            let resp = self
                .run_query_with_params(
                    &line_sql,
                    &[
                        Param::optional_string("description", patch.description),
                        Param::optional_string("context", line_context),
                        Param::string("actor_id", actor_id),
                        Param::string("idempotency_key", idempotency_key),
                        Param::string("transaction_id", transaction_id),
                    ],
                )
                .await?;
            let affected: i64 = resp
                .num_dml_affected_rows
                .as_deref()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
            if affected == 0 {
                anyhow::bail!("Linha de split {transaction_id} não encontrada");
            }

            if patch.merchant_name.is_some() || patch.purpose.is_some() {
                let parent_sql = format!(
                    "
                    UPDATE {}
                    SET merchant_name = COALESCE(@merchant_name, merchant_name),
                        purpose = COALESCE(@purpose, purpose),
                        actor_id = @actor_id,
                        idempotency_key = @idempotency_key,
                        updated_at = CURRENT_TIMESTAMP()
                    WHERE transaction_id = @parent_id
                    ",
                    self.qualified_table("transactions")?,
                );
                self.run_query_with_params(
                    &parent_sql,
                    &[
                        Param::optional_string("merchant_name", patch.merchant_name),
                        Param::optional_string("purpose", patch.purpose),
                        Param::string("actor_id", actor_id),
                        Param::string("idempotency_key", idempotency_key),
                        Param::string("parent_id", &parent_id),
                    ],
                )
                .await?;
            }
            return Ok(());
        }

        let sql = format!(
            "
            UPDATE {}
            SET description = COALESCE(@description, description),
                merchant_name = COALESCE(@merchant_name, merchant_name),
                purpose = COALESCE(@purpose, purpose),
                classifier_trace = COALESCE(@classifier_trace, classifier_trace),
                context = COALESCE(@context, context),
                actor_id = @actor_id,
                idempotency_key = @idempotency_key,
                updated_at = CURRENT_TIMESTAMP()
            WHERE transaction_id = @transaction_id
            ",
            self.qualified_table("transactions")?,
        );
        let resp = self
            .run_query_with_params(
                &sql,
                &[
                    Param::optional_string("description", patch.description),
                    Param::optional_string("merchant_name", patch.merchant_name),
                    Param::optional_string("purpose", patch.purpose),
                    Param::optional_string("classifier_trace", patch.classifier_trace),
                    Param::optional_string("context", patch.context),
                    Param::string("actor_id", actor_id),
                    Param::string("idempotency_key", idempotency_key),
                    Param::string("transaction_id", transaction_id),
                ],
            )
            .await?;
        let affected: i64 = resp
            .num_dml_affected_rows
            .as_deref()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);
        if affected == 0 {
            anyhow::bail!("Transação {transaction_id} não encontrada");
        }
        Ok(())
    }

    async fn existing_transaction_ids(&self, ids: &[String]) -> Result<BTreeSet<String>> {
        if ids.is_empty() {
            return Ok(BTreeSet::new());
        }
        let array_param = Param::new(
            "ids",
            BqType::Array(Box::new(BqType::String)),
            BqValue::Array(ids.iter().map(bv_str).collect()),
        );
        let sql = format!(
            "
            SELECT transaction_id
            FROM {}
            WHERE transaction_id IN UNNEST(@ids)
            ",
            self.qualified_table("transactions")?,
        );
        let response = self.run_query_with_params(&sql, &[array_param]).await?;
        let mut existing = BTreeSet::new();
        for row in response.rows {
            let values = row_values(&row);
            existing.insert(required_string(&values, 0, "transaction_id")?);
        }
        Ok(existing)
    }

    async fn transaction_by_id(&self, transaction_id: &str) -> Result<Option<TransactionRecord>> {
        let sql = format!(
            "
            SELECT
              transaction_id,
              account_id,
              CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''),
              description,
              merchant_name,
              purpose,
              CAST(amount AS STRING),
              tx_type,
              category_id,
              category_source,
              context,
              classifier_trace,
              payment_status,
              source,
              actor_id,
              idempotency_key,
              COALESCE(TO_JSON_STRING(metadata_json), '{{}}'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', enrichment_attempted_at, 'UTC')
            FROM {}
            WHERE transaction_id = @tid
            LIMIT 1
            ",
            self.qualified_table("transactions")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::string("tid", transaction_id)])
            .await?;
        let Some(row) = response.rows.first() else {
            return Ok(None);
        };
        let values = row_values(row);
        transaction_record_from_values(&values).map(Some)
    }

    async fn transaction_split_detail(
        &self,
        transaction_id: &str,
    ) -> Result<Option<TransactionSplitDetail>> {
        let Some(parent) = self.transaction_by_id(transaction_id).await? else {
            return Ok(None);
        };

        let split_sql = format!(
            "
            SELECT
              split_id,
              parent_transaction_id,
              payload_hash,
              status,
              source,
              notes,
              actor_id,
              idempotency_key,
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at, 'UTC'),
              COALESCE(TO_JSON_STRING(metadata_json), '{{}}')
            FROM {}
            WHERE parent_transaction_id = @parent_id
              AND status = 'active'
            ORDER BY updated_at DESC, split_id DESC
            LIMIT 1
            ",
            self.qualified_table("transaction_splits")?,
        );
        let split_response = self
            .run_query_with_params(&split_sql, &[Param::string("parent_id", transaction_id)])
            .await?;
        let split = split_response
            .rows
            .first()
            .map(|row| split_record_from_values(&row_values(row)))
            .transpose()?;
        let Some(active_split) = split.clone() else {
            return Ok(Some(TransactionSplitDetail {
                parent,
                split: None,
                lines: Vec::new(),
                items: Vec::new(),
            }));
        };

        let lines_sql = format!(
            "
            SELECT
              split_line_id,
              split_id,
              parent_transaction_id,
              CAST(line_index AS STRING),
              description,
              CAST(amount AS STRING),
              category_id,
              category_source,
              context,
              status,
              actor_id,
              idempotency_key,
              COALESCE(TO_JSON_STRING(metadata_json), '{{}}'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at, 'UTC')
            FROM {}
            WHERE split_id = @split_id
              AND status = 'active'
            ORDER BY line_index ASC
            ",
            self.qualified_table("transaction_split_lines")?,
        );
        let line_response = self
            .run_query_with_params(
                &lines_sql,
                &[Param::string("split_id", &active_split.split_id)],
            )
            .await?;
        let mut lines = Vec::with_capacity(line_response.rows.len());
        for row in line_response.rows {
            lines.push(split_line_record_from_values(&row_values(&row))?);
        }

        let items_sql = format!(
            "
            SELECT
              receipt_item_id,
              parent_transaction_id,
              split_id,
              split_line_id,
              CAST(item_index AS STRING),
              description,
              CAST(quantity AS STRING),
              unit,
              CAST(unit_price AS STRING),
              CAST(total_price AS STRING),
              code,
              store_name,
              status,
              actor_id,
              idempotency_key,
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at, 'UTC'),
              COALESCE(TO_JSON_STRING(metadata_json), '{{}}')
            FROM {}
            WHERE split_id = @split_id
              AND status = 'active'
            ORDER BY item_index ASC
            ",
            self.qualified_table("receipt_items")?,
        );
        let item_response = self
            .run_query_with_params(
                &items_sql,
                &[Param::string("split_id", &active_split.split_id)],
            )
            .await?;
        let mut items = Vec::with_capacity(item_response.rows.len());
        for row in item_response.rows {
            items.push(receipt_item_record_from_values(&row_values(&row))?);
        }

        Ok(Some(TransactionSplitDetail {
            parent,
            split,
            lines,
            items,
        }))
    }

    async fn clear_transaction_split(
        &self,
        transaction_id: &str,
        actor_id: &str,
        idempotency_key: &str,
        _reason: Option<&str>,
    ) -> Result<()> {
        let sql = format!(
            "
            UPDATE {}
            SET status = 'inactive',
                actor_id = @actor_id,
                idempotency_key = @idempotency_key,
                updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = @transaction_id
              AND status = 'active';

            UPDATE {}
            SET status = 'inactive',
                actor_id = @actor_id,
                idempotency_key = @idempotency_key,
                updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = @transaction_id
              AND status = 'active';

            UPDATE {}
            SET status = 'inactive',
                actor_id = @actor_id,
                idempotency_key = @idempotency_key,
                updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = @transaction_id
              AND status = 'active';
            ",
            self.qualified_table("transaction_splits")?,
            self.qualified_table("transaction_split_lines")?,
            self.qualified_table("receipt_items")?,
        );
        self.run_query_with_params(
            &sql,
            &[
                Param::string("actor_id", actor_id),
                Param::string("idempotency_key", idempotency_key),
                Param::string("transaction_id", transaction_id),
            ],
        )
        .await?;
        Ok(())
    }

    async fn split_candidates(&self, since: NaiveDate) -> Result<Vec<SplitCandidateRow>> {
        let sql = format!(
            "
            WITH unsplit_transactions AS (
              SELECT t.*
              FROM {} t
              WHERE t.transaction_date >= @since
                AND NOT EXISTS (
                  SELECT 1
                  FROM {} s
                  WHERE s.parent_transaction_id = t.transaction_id
                    AND s.status = 'active'
                )
            )
            SELECT
              t.transaction_id,
              CAST(t.transaction_date AS STRING),
              COALESCE(t.description, t.merchant_name, t.raw_description) AS description,
              CAST(t.amount AS STRING),
              t.account_id,
              t.category_id,
              t.classifier_trace,
              p.policy_id,
              p.name,
              p.match_type
            FROM unsplit_transactions t
            JOIN {} p
              ON p.status = 'active'
             AND (
               (p.match_type = 'description_contains' AND STRPOS(LOWER(t.raw_description), LOWER(p.match_value)) > 0)
               OR (p.match_type = 'category_prefix' AND STARTS_WITH(COALESCE(t.category_id, ''), p.match_value))
               OR (p.match_type = 'account_id' AND COALESCE(t.account_id, '') = p.match_value)
             )
            WHERE p.min_abs_amount IS NULL OR ABS(t.amount) >= p.min_abs_amount
            ORDER BY t.transaction_date DESC, ABS(t.amount) DESC, t.transaction_id ASC, p.policy_id ASC
            LIMIT 100
            ",
            self.qualified_table("transactions")?,
            self.qualified_table("transaction_splits")?,
            self.qualified_table("split_review_policies")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::date("since", since)])
            .await?;
        let mut rows = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            rows.push(SplitCandidateRow {
                transaction_id: required_string(&values, 0, "transaction_id")?,
                transaction_date: required_date(&values, 1, "transaction_date")?,
                description: required_string(&values, 2, "description")?,
                amount: required_decimal(&values, 3, "amount")?,
                account_id: optional_string(&values, 4),
                category_id: optional_string(&values, 5),
                context: optional_string(&values, 6),
                policy_id: required_string(&values, 7, "policy_id")?,
                policy_name: required_string(&values, 8, "policy_name")?,
                match_type: required_string(&values, 9, "match_type")?,
            });
        }
        Ok(rows)
    }

    async fn item_prices(
        &self,
        query: &str,
        since: Option<NaiveDate>,
    ) -> Result<Vec<ItemPriceRow>> {
        let mut params = Vec::new();
        let query_filter = if query.trim().is_empty() {
            String::new()
        } else {
            params.push(Param::string("q", query.trim()));
            "AND STRPOS(LOWER(i.description), LOWER(@q)) > 0".to_string()
        };
        let since_filter = if let Some(value) = since {
            params.push(Param::date("since", value));
            "AND t.transaction_date >= @since".to_string()
        } else {
            String::new()
        };
        let sql = format!(
            "
            SELECT
              t.transaction_id,
              CAST(t.transaction_date AS STRING),
              i.description,
              CAST(i.quantity AS STRING),
              i.unit,
              CAST(i.unit_price AS STRING),
              CAST(i.total_price AS STRING),
              i.code,
              i.store_name,
              t.description
            FROM {} i
            JOIN {} t
              ON t.transaction_id = i.parent_transaction_id
            WHERE i.status = 'active'
              {query_filter}
              {since_filter}
            ORDER BY t.transaction_date DESC, i.description ASC, i.receipt_item_id ASC
            LIMIT 100
            ",
            self.qualified_table("receipt_items")?,
            self.qualified_table("transactions")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut rows = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            rows.push(ItemPriceRow {
                transaction_id: required_string(&values, 0, "transaction_id")?,
                transaction_date: required_date(&values, 1, "transaction_date")?,
                description: required_string(&values, 2, "description")?,
                quantity: optional_string(&values, 3)
                    .map(|value| Decimal::from_str(&value).context("quantity inválido"))
                    .transpose()?,
                unit: optional_string(&values, 4),
                unit_price: optional_string(&values, 5)
                    .map(|value| Decimal::from_str(&value).context("unit_price inválido"))
                    .transpose()?,
                total_price: optional_string(&values, 6)
                    .map(|value| Decimal::from_str(&value).context("total_price inválido"))
                    .transpose()?,
                code: optional_string(&values, 7),
                store_name: optional_string(&values, 8),
                parent_description: required_string(&values, 9, "parent_description")?,
            });
        }
        Ok(rows)
    }

    async fn all_rules(&self) -> Result<Vec<RuleRecord>> {
        let sql = format!(
            "
            SELECT rule_id, body, status, actor_id, idempotency_key,
                   FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at, 'UTC'),
                   FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at, 'UTC')
            FROM {}
            ORDER BY rule_id ASC
            ",
            self.qualified_table("rules")?,
        );
        let response = self.run_query(&sql).await?;
        let mut rules = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            let created_at = required_string(&values, 5, "created_at")?;
            let updated_at = required_string(&values, 6, "updated_at")?;
            rules.push(RuleRecord {
                rule_id: required_string(&values, 0, "rule_id")?,
                body: required_string(&values, 1, "body")?,
                status: required_string(&values, 2, "status")?,
                actor_id: required_string(&values, 3, "actor_id")?,
                idempotency_key: required_string(&values, 4, "idempotency_key")?,
                created_at: parse_datetime_or_now(Some(&created_at)),
                updated_at: parse_datetime_or_now(Some(&updated_at)),
            });
        }
        Ok(rules)
    }

    async fn latest_pluggy_transaction_date(&self) -> Result<Option<NaiveDate>> {
        let sql = format!(
            "SELECT MAX(transaction_date) FROM {} WHERE source = 'pluggy'",
            self.qualified_table("transactions")?
        );
        let response = self.run_query(&sql).await?;
        let Some(row) = response.rows.first() else {
            return Ok(None);
        };
        let values = row_values(row);
        optional_date(&values, 0, "max_transaction_date")
    }

    async fn active_rules(&self) -> Result<Vec<RuleRecord>> {
        let sql = format!(
            "
            SELECT rule_id, body, status, actor_id, idempotency_key,
                   FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at, 'UTC'),
                   FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at, 'UTC')
            FROM {}
            WHERE LOWER(status) = 'active'
            ORDER BY rule_id ASC
            ",
            self.qualified_table("rules")?,
        );
        let response = self.run_query(&sql).await?;
        let mut rules = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            let created_at = required_string(&values, 5, "created_at")?;
            let updated_at = required_string(&values, 6, "updated_at")?;
            rules.push(RuleRecord {
                rule_id: required_string(&values, 0, "rule_id")?,
                body: required_string(&values, 1, "body")?,
                status: required_string(&values, 2, "status")?,
                actor_id: required_string(&values, 3, "actor_id")?,
                idempotency_key: required_string(&values, 4, "idempotency_key")?,
                created_at: parse_datetime_or_now(Some(&created_at)),
                updated_at: parse_datetime_or_now(Some(&updated_at)),
            });
        }
        Ok(rules)
    }

    async fn internal_categories(&self) -> Result<BTreeSet<String>> {
        let sql = format!(
            "
            SELECT category_id
            FROM {}
            ORDER BY category_id ASC
            ",
            self.qualified_table("internal_categories")?,
        );
        let response = self.run_query(&sql).await?;
        let mut categories = BTreeSet::new();
        for row in response.rows {
            let values = row_values(&row);
            categories.insert(required_string(&values, 0, "category_id")?);
        }
        Ok(categories)
    }

    async fn list_all_category_ids(&self) -> Result<BTreeSet<String>> {
        let sql = format!(
            "
            SELECT category_id FROM {categories}
            UNION DISTINCT
            SELECT DISTINCT category_id FROM {transactions}
              WHERE category_id IS NOT NULL AND TRIM(category_id) <> ''
            ",
            categories = self.qualified_table("categories")?,
            transactions = self.qualified_table("transactions")?,
        );
        let response = self.run_query(&sql).await?;
        let mut categories = BTreeSet::new();
        for row in response.rows {
            let values = row_values(&row);
            if let Some(id) = optional_string(&values, 0) {
                if !id.trim().is_empty() {
                    categories.insert(id);
                }
            }
        }
        Ok(categories)
    }

    async fn transactions_with_context(&self, limit: usize) -> Result<Vec<TransactionContextRow>> {
        let sql = format!(
            "
            SELECT
              t.transaction_id,
              CAST(t.transaction_date AS STRING),
              t.raw_description,
              CAST(t.amount AS STRING),
              t.account_id,
              a.label,
              t.category_id,
              t.context,
              t.payment_status,
              t.source
            FROM {} t
            LEFT JOIN {} a ON a.account_id = t.account_id
            WHERE t.context IS NOT NULL
              AND TRIM(t.context) <> ''
            ORDER BY t.transaction_date DESC, ABS(t.amount) DESC, t.transaction_id ASC
            LIMIT {}
            ",
            self.qualified_table("v_transactions_reportable")?,
            self.qualified_table("accounts")?,
            limit,
        );
        let response = self.run_query(&sql).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(TransactionContextRow {
                transaction_id: required_string(&values, 0, "transaction_id")?,
                transaction_date: required_date(&values, 1, "transaction_date")?,
                description: required_string(&values, 2, "description")?,
                amount: required_decimal(&values, 3, "amount")?,
                account_id: optional_string(&values, 4),
                account_label: optional_string(&values, 5),
                category_id: optional_string(&values, 6),
                context: required_string(&values, 7, "context")?,
                payment_status: required_string(&values, 8, "payment_status")?,
                source: required_string(&values, 9, "source")?,
            });
        }
        Ok(items)
    }

    async fn count_transactions_with_context(&self) -> Result<i64> {
        let sql = format!(
            "
            SELECT CAST(COUNT(*) AS STRING) FROM {}
            WHERE context IS NOT NULL
              AND TRIM(context) <> ''
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self.run_query(&sql).await?;
        let count = response
            .rows
            .first()
            .and_then(|row| row.f.first())
            .and_then(|cell| parse_scalar_string(&cell.v))
            .unwrap_or_else(|| "0".to_string());
        Ok(count.parse().unwrap_or(0))
    }

    async fn daily_pulse(&self, since: NaiveDate) -> Result<Vec<DailyPulseItem>> {
        let sql = format!(
            "
            SELECT transaction_id, CAST(transaction_date AS STRING), description, CAST(amount AS STRING),
                   category_id, source, payment_status, account_id
            FROM {}
            WHERE transaction_date >= @since
            ORDER BY transaction_date DESC, amount ASC, transaction_id ASC
            ",
            self.qualified_table("v_daily_pulse")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::date("since", since)])
            .await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(DailyPulseItem {
                transaction_id: required_string(&values, 0, "transaction_id")?,
                transaction_date: required_date(&values, 1, "transaction_date")?,
                description: required_string(&values, 2, "description")?,
                amount: required_decimal(&values, 3, "amount")?,
                category_id: optional_string(&values, 4),
                source: optional_string(&values, 5).unwrap_or_else(|| "unknown".to_string()),
                payment_status: optional_string(&values, 6)
                    .unwrap_or_else(|| "unknown".to_string()),
                account_id: optional_string(&values, 7),
            });
        }
        Ok(items)
    }

    async fn effective_transactions_window(
        &self,
        account_id: Option<&str>,
        since: NaiveDate,
        until: NaiveDate,
    ) -> Result<Vec<TransactionRecord>> {
        let mut params = vec![Param::date("since", since), Param::date("until", until)];
        let account_clause = if let Some(id) = account_id {
            params.push(Param::string("acc", id));
            "AND account_id = @acc"
        } else {
            ""
        };
        let sql = format!(
            "
            SELECT
              transaction_id,
              account_id,
              CAST(transaction_date AS STRING),
              raw_description,
              description,
              merchant_name,
              purpose,
              CAST(ROUND(COALESCE(amount_cents, 0) / 100.0, 2) AS STRING),
              tx_type,
              category_id,
              category_source,
              context,
              classifier_trace,
              payment_status,
              source,
              actor_id,
              idempotency_key,
              COALESCE(TO_JSON_STRING(metadata_json), '{{}}'),
              CAST(created_at AS STRING),
              CAST(updated_at AS STRING),
              CAST(enrichment_attempted_at AS STRING)
            FROM {}
            WHERE transaction_date >= @since
              AND transaction_date <= @until
              {account_clause}
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(transaction_record_from_values(&values)?);
        }
        Ok(items)
    }

    async fn transactions_in_date_range(
        &self,
        account_id: Option<&str>,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<TransactionRecord>> {
        let mut params = vec![Param::date("from", from), Param::date("to", to)];
        let account_clause = if let Some(id) = account_id {
            params.push(Param::string("acc", id));
            "AND account_id = @acc"
        } else {
            ""
        };
        let sql = format!(
            "
            SELECT
              transaction_id,
              account_id,
              CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''),
              description,
              merchant_name,
              purpose,
              CAST(amount AS STRING),
              tx_type,
              category_id,
              category_source,
              context,
              classifier_trace,
              payment_status,
              source,
              actor_id,
              idempotency_key,
              COALESCE(TO_JSON_STRING(metadata_json), '{{}}'),
              CAST(created_at AS STRING),
              CAST(updated_at AS STRING),
              CAST(enrichment_attempted_at AS STRING)
            FROM {}
            WHERE transaction_date >= @from
              AND transaction_date <= @to
              {account_clause}
            ORDER BY transaction_date ASC, transaction_id ASC
            ",
            self.qualified_table("transactions")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(transaction_record_from_values(&values)?);
        }
        Ok(items)
    }

    async fn monthly_spend(&self, month_ref: Option<&str>) -> Result<Vec<MonthlySpendRow>> {
        let mut params = Vec::new();
        let where_clause = if let Some(value) = month_ref {
            params.push(Param::string("month_ref", value));
            "WHERE month_ref = @month_ref"
        } else {
            ""
        };
        let sql = format!(
            "
            SELECT month_ref, category_id, account_id, CAST(expenses AS STRING), CAST(expense_count AS STRING)
            FROM {}
            {where_clause}
            ORDER BY month_ref DESC, expenses DESC, category_id ASC, account_id ASC
            ",
            self.qualified_table("v_monthly_spend")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(MonthlySpendRow {
                month_ref: required_string(&values, 0, "month_ref")?,
                category_id: required_string(&values, 1, "category_id")?,
                account_id: required_string(&values, 2, "account_id")?,
                expenses: required_decimal(&values, 3, "expenses")?,
                expense_count: required_i64(&values, 4, "expense_count")?,
            });
        }
        Ok(items)
    }

    async fn cashflow(&self, months: usize) -> Result<Vec<CashflowRow>> {
        // Cash-basis: only checking accounts contribute. See the SQLite
        // implementation for the rationale on the inclusion of
        // `credit-card-payment` and exclusion of `transfer-internal`.
        let sql = format!(
            "
            WITH base AS (
              SELECT
                FORMAT_DATE('%Y-%m', t.transaction_date) AS month_ref,
                t.amount,
                COALESCE(t.category_id, '') AS category_id
              FROM {tx} t
              JOIN {accounts} a ON a.account_id = t.account_id
              WHERE a.account_type = 'checking'
                AND COALESCE(t.category_id, '') != 'transfer-internal'
            )
            SELECT month_ref,
                   CAST(SUM(CASE WHEN amount > 0 AND category_id != 'cashback' THEN amount ELSE 0 END) AS STRING) AS income,
                   CAST(SUM(CASE WHEN amount < 0 THEN -amount ELSE 0 END) AS STRING) AS expenses,
                   CAST(SUM(CASE WHEN amount > 0 AND category_id = 'cashback' THEN amount ELSE 0 END) AS STRING) AS expense_reduction,
                   CAST(SUM(amount) AS STRING) AS net
            FROM base
            GROUP BY month_ref
            ORDER BY month_ref DESC
            LIMIT {limit}
            ",
            tx = self.qualified_table("v_transactions_reportable")?,
            accounts = self.qualified_table("accounts")?,
            limit = months,
        );
        let response = self.run_query(&sql).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(CashflowRow {
                month_ref: required_string(&values, 0, "month_ref")?,
                income: required_decimal(&values, 1, "income")?,
                expenses: required_decimal(&values, 2, "expenses")?,
                expense_reduction: required_decimal(&values, 3, "expense_reduction")?,
                net: required_decimal(&values, 4, "net")?,
                opening_balance: None,
                closing_balance: None,
            });
        }
        Ok(items)
    }

    async fn cashflow_month(&self, month_ref: &str) -> Result<CashflowRow> {
        let sql = format!(
            "
            WITH base AS (
              SELECT
                t.amount,
                COALESCE(t.category_id, '') AS category_id
              FROM {tx} t
              JOIN {accounts} a ON a.account_id = t.account_id
              WHERE a.account_type = 'checking'
                AND COALESCE(t.category_id, '') != 'transfer-internal'
                AND FORMAT_DATE('%Y-%m', t.transaction_date) = @month_ref
            )
            SELECT
              CAST(COALESCE(SUM(CASE WHEN amount > 0 AND category_id != 'cashback' THEN amount ELSE 0 END), 0) AS STRING) AS income,
              CAST(COALESCE(SUM(CASE WHEN amount < 0 THEN -amount ELSE 0 END), 0) AS STRING) AS expenses,
              CAST(COALESCE(SUM(CASE WHEN amount > 0 AND category_id = 'cashback' THEN amount ELSE 0 END), 0) AS STRING) AS expense_reduction,
              CAST(COALESCE(SUM(amount), 0) AS STRING) AS net
            FROM base
            ",
            tx = self.qualified_table("v_transactions_reportable")?,
            accounts = self.qualified_table("accounts")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::string("month_ref", month_ref)])
            .await?;
        let (income, expenses, expense_reduction, net) = response
            .rows
            .first()
            .map(|row| {
                let values = row_values(row);
                Ok::<_, anyhow::Error>((
                    required_decimal(&values, 0, "income")?,
                    required_decimal(&values, 1, "expenses")?,
                    required_decimal(&values, 2, "expense_reduction")?,
                    required_decimal(&values, 3, "net")?,
                ))
            })
            .transpose()?
            .unwrap_or((Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO));

        let target_month_start = NaiveDate::parse_from_str(&format!("{month_ref}-01"), "%Y-%m-%d")
            .with_context(|| format!("month_ref inválido: {month_ref} (esperado YYYY-MM)"))?;
        let opening_anchor = target_month_start
            .checked_sub_days(Days::new(1))
            .context("Falha ao calcular dia anterior ao início do mês")?;
        let closing_anchor =
            bq_last_day_of_target_month(target_month_start, Utc::now().date_naive())?;

        let opening = self.checking_balance_at(opening_anchor).await?;
        let closing = self.checking_balance_at(closing_anchor).await?;

        Ok(CashflowRow {
            month_ref: month_ref.to_string(),
            income,
            expenses,
            expense_reduction,
            net,
            opening_balance: opening.map(|b| b.balance),
            closing_balance: closing.map(|b| b.balance),
        })
    }

    async fn cashflow_reportable(&self) -> Result<Vec<CashflowRow>> {
        // Cash-flow basis over all reportable accounts. This mirrors v_cashflow
        // (buckets by the canonical `cash_month` from v_transactions_cashbasis,
        // so a card purchase lands in the month its bill is paid) and drops OFX
        // rows when Pluggy produced the same transaction key. See ADR-0025.
        let sql = format!(
            "
            WITH reportable AS (
              SELECT
                *,
                COUNTIF(source = 'pluggy') OVER (
                  PARTITION BY
                    transaction_date,
                    COALESCE(account_id, ''),
                    COALESCE(amount_cents, CAST(ROUND(amount * 100) AS INT64)),
                    LOWER(TRIM(raw_description))
                ) > 0 AS has_pluggy
              FROM {tx}
              WHERE COALESCE(category_id, '') NOT IN (
                SELECT category_id FROM {internal}
              )
            )
            SELECT month_ref, income, expenses, expense_reduction, net
            FROM (
              SELECT
                cash_month AS month_ref,
                ROUND(SUM(IF(amount_cents > 0 AND COALESCE(category_id, '') != 'cashback',
                  amount_cents, 0)) / 100.0, 2) AS income,
                ROUND(SUM(IF(amount_cents < 0, ABS(amount_cents), 0)) / 100.0, 2) AS expenses,
                ROUND(SUM(IF(amount_cents > 0 AND COALESCE(category_id, '') = 'cashback',
                  amount_cents, 0)) / 100.0, 2) AS expense_reduction,
                ROUND(SUM(amount_cents) / 100.0, 2) AS net
              FROM reportable
              WHERE NOT (source = 'ofx' AND has_pluggy)
              GROUP BY 1
            )
            ORDER BY month_ref ASC
            ",
            tx = self.qualified_table("v_transactions_cashbasis")?,
            internal = self.qualified_table("internal_categories")?,
        );
        let response = self.run_query(&sql).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(CashflowRow {
                month_ref: required_string(&values, 0, "month_ref")?,
                income: required_decimal(&values, 1, "income")?,
                expenses: required_decimal(&values, 2, "expenses")?,
                expense_reduction: required_decimal(&values, 3, "expense_reduction")?,
                net: required_decimal(&values, 4, "net")?,
                opening_balance: None,
                closing_balance: None,
            });
        }
        Ok(items)
    }

    async fn checking_balance_at(&self, target: NaiveDate) -> Result<Option<CheckingBalance>> {
        // Snapshot-anchored aggregate: per checking account, take the latest
        // snapshot ≤ target and add the sum of `amount` for transactions
        // strictly after snapshot_date and ≤ target. If any checking account
        // has no snapshot in range, return None.
        let sql = format!(
            "
            WITH checking AS (
              SELECT account_id
              FROM {accounts}
              WHERE account_type = 'checking'
            ),
            anchors AS (
              SELECT
                c.account_id,
                ARRAY_AGG(
                  STRUCT(s.snapshot_date AS snapshot_date, s.balance AS balance)
                  ORDER BY s.snapshot_date DESC, s.created_at DESC
                  LIMIT 1
                )[OFFSET(0)] AS anchor
              FROM checking c
              LEFT JOIN {snapshots} s
                ON s.account_id = c.account_id
               AND s.snapshot_date <= @target
              GROUP BY c.account_id
            ),
            deltas AS (
              SELECT
                a.account_id,
                a.anchor.snapshot_date AS anchor_date,
                a.anchor.balance AS anchor_balance,
                COALESCE((
                  SELECT SUM(t.amount)
                  FROM {tx} t
                  WHERE t.account_id = a.account_id
                    AND t.transaction_date > a.anchor.snapshot_date
                    AND t.transaction_date <= @target
                ), 0) AS delta
              FROM anchors a
            )
            SELECT
              CAST(account_id AS STRING) AS account_id,
              CAST(anchor_date AS STRING) AS anchor_date,
              CAST(anchor_balance AS STRING) AS anchor_balance,
              CAST(delta AS STRING) AS delta
            FROM deltas
            ORDER BY account_id
            ",
            accounts = self.qualified_table("accounts")?,
            snapshots = self.qualified_table("account_snapshots")?,
            tx = self.qualified_table("v_transactions_reportable")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::date("target", target)])
            .await?;

        let mut total = Decimal::ZERO;
        let mut accounts_considered = 0usize;
        let mut latest_anchor: Option<NaiveDate> = None;

        for row in response.rows {
            let values = row_values(&row);
            accounts_considered += 1;
            let anchor_date_str = optional_string(&values, 1);
            let anchor_balance_str = optional_string(&values, 2);
            if anchor_date_str.is_none() {
                return Ok(None);
            }
            let anchor_date = optional_date(&values, 1, "anchor_date")?
                .ok_or_else(|| anyhow!("anchor_date inesperadamente nulo"))?;
            let anchor_balance = anchor_balance_str
                .as_deref()
                .map(Decimal::from_str)
                .transpose()
                .with_context(|| "balance inválido em snapshot")?
                .unwrap_or(Decimal::ZERO);
            let delta = required_decimal(&values, 3, "delta")?;
            total += anchor_balance + delta;
            latest_anchor = Some(match latest_anchor {
                Some(prev) if prev > anchor_date => prev,
                _ => anchor_date,
            });
        }

        Ok(Some(CheckingBalance {
            balance: total,
            accounts_considered,
            snapshot_anchor_date: latest_anchor,
        }))
    }

    async fn forecast_vs_actual(
        &self,
        month_ref: Option<&str>,
    ) -> Result<Vec<ForecastVsActualRow>> {
        let mut params = Vec::new();
        let where_clause = if let Some(value) = month_ref {
            params.push(Param::string("month_ref", value));
            "WHERE month_ref = @month_ref"
        } else {
            ""
        };
        let sql = format!(
            "
            SELECT
              forecast_id,
              month_ref,
              CAST(due_date AS STRING),
              description,
              account_id,
              category_id,
              CAST(forecast_amount AS STRING),
              CAST(actual_amount AS STRING),
              CAST(variance AS STRING),
              status
            FROM {}
            {where_clause}
            ORDER BY month_ref DESC, due_date ASC, description ASC
            ",
            self.qualified_table("v_forecast_vs_actual")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(ForecastVsActualRow {
                forecast_id: required_string(&values, 0, "forecast_id")?,
                month_ref: required_string(&values, 1, "month_ref")?,
                due_date: optional_date(&values, 2, "due_date")?,
                description: required_string(&values, 3, "description")?,
                account_id: optional_string(&values, 4),
                category_id: optional_string(&values, 5),
                forecast_amount: required_decimal(&values, 6, "forecast_amount")?,
                actual_amount: required_decimal(&values, 7, "actual_amount")?,
                variance: required_decimal(&values, 8, "variance")?,
                status: required_string(&values, 9, "status")?,
            });
        }
        Ok(items)
    }

    async fn card_summary(&self, month_ref: Option<&str>) -> Result<Vec<CardSummaryRow>> {
        let mut params = Vec::new();
        let where_clause = if let Some(value) = month_ref {
            params.push(Param::string("month_ref", value));
            "WHERE month_ref = @month_ref"
        } else {
            ""
        };
        let sql = format!(
            "
            SELECT
              month_ref,
              account_id,
              CAST(total_charges AS STRING),
              CAST(open_amount AS STRING),
              CAST(installments_future AS STRING),
              CAST(transaction_count AS STRING)
            FROM {}
            {where_clause}
            ORDER BY month_ref DESC, total_charges DESC, account_id ASC
            ",
            self.qualified_table("v_card_summary")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(CardSummaryRow {
                month_ref: required_string(&values, 0, "month_ref")?,
                account_id: required_string(&values, 1, "account_id")?,
                total_charges: required_decimal(&values, 2, "total_charges")?,
                open_amount: required_decimal(&values, 3, "open_amount")?,
                installments_future: required_decimal(&values, 4, "installments_future")?,
                transaction_count: required_i64(&values, 5, "transaction_count")?,
            });
        }
        Ok(items)
    }

    async fn cards_open_now(&self) -> Result<Vec<CardSummaryRow>> {
        let sql = format!(
            "
            SELECT
              month_ref,
              account_id,
              CAST(total_charges AS STRING),
              CAST(open_amount AS STRING),
              CAST(installments_future AS STRING),
              CAST(transaction_count AS STRING)
            FROM {}
            ORDER BY account_id ASC
            ",
            self.qualified_table("v_card_open_now")?,
        );
        let response = self.run_query(&sql).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(CardSummaryRow {
                month_ref: required_string(&values, 0, "month_ref")?,
                account_id: required_string(&values, 1, "account_id")?,
                total_charges: required_decimal(&values, 2, "total_charges")?,
                open_amount: required_decimal(&values, 3, "open_amount")?,
                installments_future: required_decimal(&values, 4, "installments_future")?,
                transaction_count: required_i64(&values, 5, "transaction_count")?,
            });
        }
        Ok(items)
    }

    async fn card_closed_transactions(
        &self,
        month_ref: Option<&str>,
    ) -> Result<Vec<CardClosedTransactionRow>> {
        let mut params = Vec::new();
        let where_month = if let Some(value) = month_ref {
            params.push(Param::string("month_ref", value));
            "AND FORMAT_DATE('%Y-%m', t.transaction_date) = @month_ref"
        } else {
            ""
        };
        // Returns every charge AND statement credit (IOF reversals,
        // merchant refunds, etc.) — credits net naturally against debits in
        // the bill total. The one credit type we exclude is the bill
        // payment itself ("Pagamento recebido"), which appears on the card
        // side but mirrors a debit on checking and would double-count.
        // Amounts are returned signed (debits negative, credits positive).
        let sql = format!(
            "
            SELECT
              FORMAT_DATE('%Y-%m', t.transaction_date) AS month_ref,
              t.account_id,
              t.transaction_id,
              CAST(t.transaction_date AS STRING),
              t.raw_description,
              COALESCE(t.description, t.merchant_name, t.raw_description),
              CAST(t.amount AS STRING),
              t.category_id,
              t.payment_status,
              COALESCE(TO_JSON_STRING(t.metadata_json), '{{}}')
            FROM {} t
            JOIN {} a
              ON a.account_id = t.account_id
            WHERE a.account_type = 'credit'
              AND NOT (t.amount > 0 AND LOWER(t.raw_description) LIKE '%pagamento recebido%')
              AND COALESCE(t.category_id, '') NOT IN (SELECT category_id FROM {})
              {where_month}
            ORDER BY t.transaction_date DESC, ABS(t.amount) DESC, t.transaction_id ASC
            ",
            self.qualified_table("v_transactions_reportable")?,
            self.qualified_table("accounts")?,
            self.qualified_table("internal_categories")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(CardClosedTransactionRow {
                month_ref: required_string(&values, 0, "month_ref")?,
                account_id: required_string(&values, 1, "account_id")?,
                transaction_id: required_string(&values, 2, "transaction_id")?,
                transaction_date: required_date(&values, 3, "transaction_date")?,
                label: required_string(&values, 4, "display_label")?,
                description: required_string(&values, 5, "description")?,
                amount: required_decimal(&values, 6, "closed_amount")?,
                category_id: optional_string(&values, 7),
                payment_status: required_string(&values, 8, "payment_status")?,
                metadata_json: optional_json(&values, 9, "metadata_json")?
                    .unwrap_or_else(|| serde_json::json!({})),
            });
        }
        Ok(items)
    }

    async fn card_reportable_transactions(
        &self,
        month_ref: Option<&str>,
    ) -> Result<Vec<CardClosedTransactionRow>> {
        let mut params = Vec::new();
        let where_month = if let Some(value) = month_ref {
            params.push(Param::string("month_ref", value));
            "AND FORMAT_DATE('%Y-%m', t.transaction_date) = @month_ref"
        } else {
            ""
        };
        let sql = format!(
            "
            SELECT
              FORMAT_DATE('%Y-%m', t.transaction_date) AS month_ref,
              t.account_id,
              t.transaction_id,
              CAST(t.transaction_date AS STRING),
              t.display_label,
              COALESCE(t.description, t.merchant_name, t.raw_description),
              CAST(ABS(t.amount) AS STRING),
              t.category_id,
              t.payment_status,
              COALESCE(TO_JSON_STRING(t.metadata_json), '{{}}')
            FROM {} t
            JOIN {} a
              ON a.account_id = t.account_id
            WHERE a.account_type = 'credit'
              AND t.amount < 0
              AND COALESCE(t.category_id, '') NOT IN (SELECT category_id FROM {})
              {where_month}
            ORDER BY t.transaction_date DESC, ABS(t.amount) DESC, t.transaction_id ASC
            ",
            self.qualified_table("v_transactions_reportable")?,
            self.qualified_table("accounts")?,
            self.qualified_table("internal_categories")?,
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(CardClosedTransactionRow {
                month_ref: required_string(&values, 0, "month_ref")?,
                account_id: required_string(&values, 1, "account_id")?,
                transaction_id: required_string(&values, 2, "transaction_id")?,
                transaction_date: required_date(&values, 3, "transaction_date")?,
                label: required_string(&values, 4, "display_label")?,
                description: required_string(&values, 5, "description")?,
                amount: required_decimal(&values, 6, "amount")?,
                category_id: optional_string(&values, 7),
                payment_status: required_string(&values, 8, "payment_status")?,
                metadata_json: optional_json(&values, 9, "metadata_json")?
                    .unwrap_or_else(|| serde_json::json!({})),
            });
        }
        Ok(items)
    }

    async fn uncategorized(&self, limit: usize) -> Result<Vec<UncategorizedRow>> {
        let sql = format!(
            "
            SELECT
              t.transaction_id,
              CAST(t.transaction_date AS STRING),
              t.display_label,
              CAST(t.amount AS STRING),
              t.account_id,
              a.label,
              t.tx_type,
              t.category_source,
              t.payment_status,
              t.source,
              COALESCE(TO_JSON_STRING(t.metadata_json), '{{}}')
            FROM {} t
            LEFT JOIN {} a ON a.account_id = t.account_id
            WHERE t.category_id IS NULL
               OR t.category_source IN ('unclassified', 'fallback')
            ORDER BY t.transaction_date DESC, ABS(t.amount) DESC, t.transaction_id ASC
            LIMIT {}
            ",
            self.qualified_table("v_transactions_reportable")?,
            self.qualified_table("accounts")?,
            limit,
        );
        let response = self.run_query(&sql).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(UncategorizedRow {
                transaction_id: required_string(&values, 0, "transaction_id")?,
                transaction_date: required_date(&values, 1, "transaction_date")?,
                description: required_string(&values, 2, "description")?,
                amount: required_decimal(&values, 3, "amount")?,
                account_id: optional_string(&values, 4),
                account_label: optional_string(&values, 5),
                tx_type: required_string(&values, 6, "tx_type")?,
                category_source: required_string(&values, 7, "category_source")?,
                payment_status: required_string(&values, 8, "payment_status")?,
                source: required_string(&values, 9, "source")?,
                metadata_json: optional_json(&values, 10, "metadata_json")?
                    .unwrap_or_else(|| json!({})),
            });
        }
        Ok(items)
    }

    async fn count_uncategorized(&self) -> Result<i64> {
        let sql = format!(
            "
            SELECT CAST(COUNT(*) AS STRING) FROM {}
            WHERE category_id IS NULL
               OR category_source IN ('unclassified', 'fallback')
            ",
            self.qualified_table("v_transactions_reportable")?,
        );
        let response = self.run_query(&sql).await?;
        let count = response
            .rows
            .first()
            .and_then(|row| row.f.first())
            .and_then(|cell| parse_scalar_string(&cell.v))
            .unwrap_or_else(|| "0".to_string());
        Ok(count.parse().unwrap_or(0))
    }

    async fn count_rows(&self, table: &str) -> Result<i64> {
        super::validate_table_name(table)?;
        let sql = format!(
            "SELECT CAST(COUNT(*) AS STRING) FROM {}",
            self.qualified_table(table)?
        );
        let response = self.run_query(&sql).await?;
        let count = response
            .rows
            .first()
            .and_then(|row| row.f.first())
            .and_then(|cell| parse_scalar_string(&cell.v))
            .unwrap_or_else(|| "0".to_string());
        Ok(count.parse().unwrap_or(0))
    }

    async fn upsert_category_budget(&self, record: &CategoryBudgetRecord) -> Result<()> {
        let table = self.qualified_table("category_budgets")?;
        let sql = format!(
            "
            MERGE {table} target
            USING (SELECT
              @budget_id AS budget_id,
              @category_id AS category_id,
              @subcategory_id AS subcategory_id,
              @month_ref AS month_ref,
              @amount AS amount,
              @alert_threshold_pct AS alert_threshold_pct,
              @actor_id AS actor_id,
              @idempotency_key AS idempotency_key,
              @created_at AS created_at,
              @updated_at AS updated_at
            ) source
            ON (
              target.category_id = source.category_id
              AND COALESCE(target.subcategory_id, '') = COALESCE(source.subcategory_id, '')
              AND COALESCE(target.month_ref, '_default') = COALESCE(source.month_ref, '_default')
            )
            WHEN MATCHED THEN UPDATE SET
              budget_id = source.budget_id,
              amount = source.amount,
              alert_threshold_pct = source.alert_threshold_pct,
              actor_id = source.actor_id,
              idempotency_key = source.idempotency_key,
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              budget_id, category_id, subcategory_id, month_ref, amount,
              alert_threshold_pct, actor_id, idempotency_key, created_at, updated_at
            ) VALUES (
              source.budget_id, source.category_id, source.subcategory_id, source.month_ref,
              source.amount, source.alert_threshold_pct, source.actor_id, source.idempotency_key,
              source.created_at, source.updated_at
            )
            ",
            table = table,
        );
        self.run_query_with_params(
            &sql,
            &[
                Param::string("budget_id", &record.budget_id),
                Param::string("category_id", &record.category_id),
                Param::optional_string("subcategory_id", record.subcategory_id.as_deref()),
                Param::optional_string("month_ref", record.month_ref.as_deref()),
                Param::decimal("amount", record.amount),
                Param::int64("alert_threshold_pct", record.alert_threshold_pct),
                Param::string("actor_id", &record.actor_id),
                Param::string("idempotency_key", &record.idempotency_key),
                Param::timestamp("created_at", record.created_at),
                Param::timestamp("updated_at", record.updated_at),
            ],
        )
        .await?;
        Ok(())
    }

    async fn list_category_budgets(
        &self,
        month: Option<&str>,
    ) -> Result<Vec<CategoryBudgetRecord>> {
        let table = self.qualified_table("category_budgets")?;
        let mut params = Vec::new();
        let month_filter = match month {
            Some(m) => {
                params.push(Param::string("month", m));
                "month_ref = @month OR month_ref IS NULL"
            }
            None => "TRUE",
        };
        let sql = format!(
            "
            SELECT
              budget_id, category_id, subcategory_id, month_ref,
              CAST(amount AS STRING), alert_threshold_pct,
              actor_id, idempotency_key,
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at)
            FROM {table}
            WHERE {month_filter}
            ORDER BY category_id ASC, subcategory_id ASC, month_ref ASC
            ",
        );
        let response = self.run_query_with_params(&sql, &params).await?;
        let mut records = Vec::new();
        for row in response.rows {
            let values = row_values(&row);
            let created_at = required_string(&values, 8, "created_at")?;
            let updated_at = required_string(&values, 9, "updated_at")?;
            records.push(CategoryBudgetRecord {
                budget_id: required_string(&values, 0, "budget_id")?,
                category_id: required_string(&values, 1, "category_id")?,
                subcategory_id: optional_string(&values, 2),
                month_ref: optional_string(&values, 3),
                amount: required_decimal(&values, 4, "amount")?,
                alert_threshold_pct: required_i64(&values, 5, "alert_threshold_pct")?,
                actor_id: required_string(&values, 6, "actor_id")?,
                idempotency_key: required_string(&values, 7, "idempotency_key")?,
                created_at: parse_datetime_or_now(Some(&created_at)),
                updated_at: parse_datetime_or_now(Some(&updated_at)),
            });
        }
        Ok(records)
    }

    async fn budget_status_for_month(&self, month: &str) -> Result<Vec<BudgetStatusRow>> {
        let spend_table = self.qualified_table("v_monthly_spend")?;
        let budget_table = self.qualified_table("category_budgets")?;

        // Fetch spend for the month
        let spend_sql = format!(
            "
            SELECT category_id, CAST(SUM(CAST(expenses AS NUMERIC)) AS STRING)
            FROM {spend_table}
            WHERE month_ref = @month
            GROUP BY category_id
            ",
        );
        let spend_response = self
            .run_query_with_params(&spend_sql, &[Param::string("month", month)])
            .await?;
        let mut spend_by_cat = std::collections::BTreeMap::<String, Decimal>::new();
        for row in spend_response.rows {
            let values = row_values(&row);
            if let (Some(cat), Some(exp)) =
                (optional_string(&values, 0), optional_string(&values, 1))
            {
                let expenses = Decimal::from_str(&exp).unwrap_or(Decimal::ZERO);
                spend_by_cat.insert(cat, expenses);
            }
        }

        // Fetch budgets applicable to this month
        let budget_sql = format!(
            "
            SELECT
              budget_id, category_id, subcategory_id, month_ref,
              CAST(amount AS STRING), alert_threshold_pct,
              actor_id, idempotency_key,
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at)
            FROM {budget_table}
            WHERE month_ref = @month OR month_ref IS NULL
            ORDER BY category_id ASC, subcategory_id ASC,
                     CASE WHEN month_ref IS NOT NULL THEN 0 ELSE 1 END ASC
            ",
        );
        let budget_response = self
            .run_query_with_params(&budget_sql, &[Param::string("month", month)])
            .await?;
        let mut all_records = Vec::new();
        for row in budget_response.rows {
            let values = row_values(&row);
            let created_at = required_string(&values, 8, "created_at")?;
            let updated_at = required_string(&values, 9, "updated_at")?;
            all_records.push(CategoryBudgetRecord {
                budget_id: required_string(&values, 0, "budget_id")?,
                category_id: required_string(&values, 1, "category_id")?,
                subcategory_id: optional_string(&values, 2),
                month_ref: optional_string(&values, 3),
                amount: required_decimal(&values, 4, "amount")?,
                alert_threshold_pct: required_i64(&values, 5, "alert_threshold_pct")?,
                actor_id: required_string(&values, 6, "actor_id")?,
                idempotency_key: required_string(&values, 7, "idempotency_key")?,
                created_at: parse_datetime_or_now(Some(&created_at)),
                updated_at: parse_datetime_or_now(Some(&updated_at)),
            });
        }

        // Dedup: explicit month wins over default
        let mut seen =
            std::collections::BTreeMap::<(String, Option<String>), CategoryBudgetRecord>::new();
        for record in all_records {
            let key = (record.category_id.clone(), record.subcategory_id.clone());
            let entry = seen.entry(key);
            match entry {
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(record);
                }
                std::collections::btree_map::Entry::Occupied(mut e) => {
                    if record.month_ref.is_some() {
                        e.insert(record);
                    }
                }
            }
        }

        let today = Utc::now().date_naive();
        let current_month = today.format("%Y-%m").to_string();
        let (day_of_month, days_in_month) = if month == current_month {
            let day = today.day();
            let first_next = if today.month() == 12 {
                NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
            }
            .unwrap_or(today);
            let last = first_next.checked_sub_days(Days::new(1)).unwrap_or(today);
            (day, last.day())
        } else {
            (0u32, 0u32)
        };

        let mut results = Vec::new();
        for ((cat, _sub), record) in seen {
            let actual = spend_by_cat.get(&cat).copied().unwrap_or(Decimal::ZERO);
            let budget = record.amount;
            let usage_pct = if budget.is_zero() {
                Decimal::ZERO
            } else {
                (actual / budget * Decimal::from(100)).round_dp(2)
            };
            let projected_pct = if month == current_month && day_of_month > 0 {
                let projected = actual / Decimal::from(day_of_month) * Decimal::from(days_in_month);
                if budget.is_zero() {
                    Decimal::ZERO
                } else {
                    (projected / budget * Decimal::from(100)).round_dp(2)
                }
            } else {
                usage_pct
            };
            let alert = usage_pct >= Decimal::from(record.alert_threshold_pct);
            results.push(BudgetStatusRow {
                category_id: cat,
                subcategory_id: record.subcategory_id,
                month_ref: month.to_string(),
                budget_amount: budget,
                actual_amount: actual,
                usage_pct,
                projected_pct,
                alert,
                alert_threshold_pct: record.alert_threshold_pct,
            });
        }
        results.sort_by(|a, b| a.category_id.cmp(&b.category_id));
        Ok(results)
    }

    async fn transactions_on_date(
        &self,
        date: NaiveDate,
        account_id: &str,
        exclude_id: &str,
    ) -> Result<Vec<crate::enrichment::types::ContextTx>> {
        let sql = format!(
            "
            SELECT
              COALESCE(raw_description, description, ''),
              CAST(amount AS STRING),
              JSON_VALUE(metadata_json, '$.pluggy_category') AS pluggy_category,
              SAFE_CAST(JSON_VALUE(metadata_json, '$.raw.order') AS INT64) AS pluggy_order
            FROM {}
            WHERE transaction_date = @date
              AND account_id = @account_id
              AND transaction_id != @exclude_id
            ORDER BY pluggy_order IS NULL, pluggy_order ASC, raw_description ASC
            ",
            self.qualified_table("transactions")?,
        );
        let response = self
            .run_query_with_params(
                &sql,
                &[
                    Param::date("date", date),
                    Param::string("account_id", account_id),
                    Param::string("exclude_id", exclude_id),
                ],
            )
            .await?;
        let mut out = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            let description = required_string(&values, 0, "description")?;
            let amount = required_decimal(&values, 1, "amount")?;
            let pluggy_category = optional_string(&values, 2);
            let order = optional_string(&values, 3).and_then(|raw| raw.parse::<i64>().ok());
            out.push(crate::enrichment::types::ContextTx {
                description,
                amount,
                pluggy_category,
                order,
            });
        }
        Ok(out)
    }

    async fn find_anatomy_donors(
        &self,
        merchant_name: &str,
        exclude_id: &str,
    ) -> Result<Vec<TransactionRecord>> {
        let normalized = merchant_name.trim().to_lowercase();
        let sql = format!(
            "
            SELECT
              transaction_id, account_id, CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''), description, merchant_name, purpose,
              CAST(amount AS STRING), tx_type, category_id, category_source, context,
              classifier_trace, payment_status, source, actor_id, idempotency_key,
              COALESCE(TO_JSON_STRING(metadata_json), '{{}}'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', enrichment_attempted_at, 'UTC')
            FROM {}
            WHERE LOWER(TRIM(COALESCE(NULLIF(TRIM(merchant_name), ''), NULLIF(TRIM(raw_description), '')))) = @normalized
              AND transaction_id != @exclude_id
              AND (
                NULLIF(TRIM(COALESCE(description, '')), '') IS NOT NULL
                OR NULLIF(TRIM(COALESCE(purpose, '')), '') IS NOT NULL
              )
            ORDER BY transaction_date DESC
            LIMIT 5
            ",
            self.qualified_table("transactions")?,
        );
        let response = self
            .run_query_with_params(
                &sql,
                &[
                    Param::string("normalized", normalized),
                    Param::string("exclude_id", exclude_id),
                ],
            )
            .await?;
        response
            .rows
            .iter()
            .map(|row| transaction_record_from_values(&row_values(row)))
            .collect()
    }

    async fn replicable_anatomy_candidates(&self, limit: usize) -> Result<Vec<TransactionRecord>> {
        let sql = format!(
            "
            SELECT
              transaction_id, account_id, CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''), description, merchant_name, purpose,
              CAST(amount AS STRING), tx_type, category_id, category_source, context,
              classifier_trace, payment_status, source, actor_id, idempotency_key,
              COALESCE(TO_JSON_STRING(metadata_json), '{{}}'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', enrichment_attempted_at, 'UTC')
            FROM {}
            WHERE COALESCE(NULLIF(TRIM(merchant_name), ''), NULLIF(TRIM(raw_description), '')) IS NOT NULL
              AND (
                description IS NULL OR TRIM(description) = ''
                OR purpose IS NULL OR TRIM(purpose) = ''
              )
              AND category_id IS NOT NULL
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT @lim
            ",
            self.qualified_table("transactions")?,
        );
        let response = self
            .run_query_with_params(&sql, &[Param::int64("lim", limit as i64)])
            .await?;
        response
            .rows
            .iter()
            .map(|row| transaction_record_from_values(&row_values(row)))
            .collect()
    }

    async fn similar_transactions(
        &self,
        keyword: &str,
        exclude_id: &str,
        only_uncategorized: bool,
    ) -> Result<Vec<TransactionRecord>> {
        let pattern = format!("%{}%", keyword.to_ascii_lowercase());
        let category_filter = if only_uncategorized {
            "AND (category_id IS NULL OR category_source IN ('unclassified', 'fallback', 'pluggy'))"
        } else {
            ""
        };
        let sql = format!(
            "
            SELECT
              transaction_id, account_id, CAST(transaction_date AS STRING),
              COALESCE(raw_description, description, ''), description, merchant_name, purpose,
              CAST(amount AS STRING), tx_type, category_id, category_source, context,
              classifier_trace, payment_status, source, actor_id, idempotency_key,
              COALESCE(TO_JSON_STRING(metadata_json), '{{}}'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', created_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', updated_at, 'UTC'),
              FORMAT_TIMESTAMP('%Y-%m-%dT%H:%M:%E6SZ', enrichment_attempted_at, 'UTC')
            FROM {}
            WHERE LOWER(COALESCE(raw_description, '')) LIKE @pattern
              AND transaction_id != @exclude_id
              {category_filter}
            ORDER BY transaction_date DESC, transaction_id ASC
            ",
            self.qualified_table("transactions")?,
        );
        let response = self
            .run_query_with_params(
                &sql,
                &[
                    Param::string("pattern", pattern),
                    Param::string("exclude_id", exclude_id),
                ],
            )
            .await?;
        response
            .rows
            .iter()
            .map(|row| transaction_record_from_values(&row_values(row)))
            .collect()
    }

    async fn mark_enrichment_attempted(
        &self,
        transaction_id: &str,
        actor_id: &str,
        idempotency_key: &str,
    ) -> Result<()> {
        let sql = format!(
            "UPDATE {} SET enrichment_attempted_at = CURRENT_TIMESTAMP() WHERE transaction_id = @tid",
            self.qualified_table("transactions")?,
        );
        self.run_query_with_params(&sql, &[Param::string("tid", transaction_id)])
            .await?;
        let audit = crate::models::AuditEvent::from_entity(
            "transaction",
            transaction_id,
            "enrich_attempted",
            actor_id,
            idempotency_key,
            serde_json::json!({"enrichment_attempted_at": "CURRENT_TIMESTAMP()"}),
        );
        self.insert_audit_events(&[audit]).await?;
        Ok(())
    }
}

#[cfg(test)]
mod param_smoke {
    //! Live BigQuery smoke tests for the typed-parameter path. Skipped by
    //! default; run with:
    //!
    //!   PHAI_BQ_SMOKE=1 cargo test -p phai-core --test '*' -- --ignored bq_param
    //!
    //! …or simply `cargo test -p phai-core bq_param_smoke -- --ignored`
    //! once `PHAI_BQ_SMOKE=1` is exported (the legacy `FINANCE_OS_BQ_SMOKE` is
    //! still honored). Reads from `~/.config/phai/config.toml` (or the legacy
    //! `~/.config/finance-os/config.toml`).
    use super::*;
    use crate::config::AppConfig;

    async fn store() -> Option<BigQueryStore> {
        if crate::compat::env_var("PHAI_BQ_SMOKE", "FINANCE_OS_BQ_SMOKE").as_deref() != Some("1") {
            return None;
        }
        let paths = crate::config::ConfigPaths::discover().ok()?;
        let config = AppConfig::load(&paths).ok()?;
        BigQueryStore::new(config).await.ok()
    }

    #[tokio::test]
    #[ignore]
    async fn bq_param_scalars_roundtrip() {
        let Some(store) = store().await else {
            eprintln!("PHAI_BQ_SMOKE=1 not set or config missing; skipping");
            return;
        };
        let params = vec![
            Param::string("s", "hello \"world\" 'with' quotes"),
            Param::optional_string::<&str>("ns", None),
            Param::decimal("d", Decimal::new(12345, 2)),
            Param::optional_decimal("nd", None),
            Param::date("dt", NaiveDate::from_ymd_opt(2026, 5, 25).unwrap()),
            Param::timestamp("ts", chrono::Utc::now()),
            Param::int64("i", 42),
            Param::json("j", &json!({"k": "v", "n": 1})),
        ];
        let resp = store
            .run_query_with_params(
                "SELECT @s AS s, @ns AS ns, @d AS d, @nd AS nd, @dt AS dt, @ts AS ts, @i AS i, @j AS j",
                &params,
            )
            .await
            .expect("scalar param query failed");
        assert!(resp.job_complete);
        assert_eq!(resp.rows.len(), 1, "expected one row, got {:?}", resp.rows);
        let row = &resp.rows[0];
        let vals: Vec<Option<String>> = row.f.iter().map(|c| parse_scalar_string(&c.v)).collect();
        eprintln!("scalar roundtrip values = {vals:#?}");
        assert_eq!(vals[0].as_deref(), Some("hello \"world\" 'with' quotes"));
        assert_eq!(vals[1], None);
        assert_eq!(vals[2].as_deref(), Some("123.45"));
        assert_eq!(vals[3], None);
        assert_eq!(vals[4].as_deref(), Some("2026-05-25"));
        assert_eq!(vals[6].as_deref(), Some("42"));
    }

    #[tokio::test]
    #[ignore]
    async fn bq_param_struct_array_unnest() {
        let Some(store) = store().await else {
            eprintln!("PHAI_BQ_SMOKE=1 not set; skipping");
            return;
        };
        let param = batch_array_param(
            "batch",
            vec![
                ("id", BqType::String),
                ("amount", BqType::Numeric),
                ("when", BqType::Date),
                ("meta", BqType::Json),
                ("opt", BqType::String),
            ],
            vec![
                vec![
                    ("id".to_string(), bv_str("row-1")),
                    ("amount".to_string(), bv_dec(Decimal::new(1234, 2))),
                    (
                        "when".to_string(),
                        bv_date(NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()),
                    ),
                    ("meta".to_string(), bv_json(&json!({"x": 1}))),
                    ("opt".to_string(), bv_opt_str(None)),
                ],
                vec![
                    ("id".to_string(), bv_str("row-2")),
                    ("amount".to_string(), bv_dec(Decimal::new(98765, 2))),
                    (
                        "when".to_string(),
                        bv_date(NaiveDate::from_ymd_opt(2026, 5, 2).unwrap()),
                    ),
                    ("meta".to_string(), bv_json(&json!({"x": 2}))),
                    ("opt".to_string(), bv_opt_str(Some("here"))),
                ],
            ],
        );
        let resp = store
            .run_query_with_params(
                "SELECT id, amount, `when`, TO_JSON_STRING(meta) AS meta, opt FROM UNNEST(@batch) ORDER BY id",
                &[param],
            )
            .await
            .expect("UNNEST struct-array query failed");
        assert!(resp.job_complete);
        assert_eq!(resp.rows.len(), 2);
        let r0: Vec<_> = resp.rows[0]
            .f
            .iter()
            .map(|c| parse_scalar_string(&c.v))
            .collect();
        let r1: Vec<_> = resp.rows[1]
            .f
            .iter()
            .map(|c| parse_scalar_string(&c.v))
            .collect();
        eprintln!("row0 = {r0:#?}\nrow1 = {r1:#?}");
        assert_eq!(r0[0].as_deref(), Some("row-1"));
        assert_eq!(r1[0].as_deref(), Some("row-2"));
        assert_eq!(r0[4], None);
        assert_eq!(r1[4].as_deref(), Some("here"));
    }
}
