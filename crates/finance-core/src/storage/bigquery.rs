use super::{FinanceStore, TransactionAnatomyPatch};
use crate::config::AppConfig;
use crate::models::{
    parse_datetime_or_now, AccountRecord, AccountSnapshotRecord, AuditEvent, BudgetStatusRow,
    CardClosedTransactionRow, CardSummaryRow, CashflowRow, CategoryBudgetRecord, CategoryRecord,
    DailyPulseItem, ForecastRecord, ForecastVsActualRow, MonthlySpendRow, RuleRecord,
    TransactionContextRow, TransactionRecord, UncategorizedRow,
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
const DRIVE_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";
const BIGQUERY_SCOPES: &[&str] = &[BIGQUERY_SCOPE, DRIVE_READONLY_SCOPE];

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
        let token = self.bearer_token().await?;
        let response = self
            .client
            .post(self.query_endpoint()?)
            .bearer_auth(&token)
            .json(&QueryRequest {
                query: sql,
                use_legacy_sql: false,
                timeout_ms: 30_000,
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
}

fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\0', "")
}

fn sql_string(value: &str) -> String {
    format!("'{}'", escape_string(value))
}

fn sql_optional_string(value: Option<&str>) -> String {
    value
        .map(|text| format!("CAST({} AS STRING)", sql_string(text)))
        .unwrap_or_else(|| "CAST(NULL AS STRING)".to_string())
}

fn sql_decimal(value: &Decimal) -> String {
    format!(
        "CAST({} AS NUMERIC)",
        sql_string(&value.round_dp(2).to_string())
    )
}

fn sql_date(value: NaiveDate) -> String {
    format!("DATE '{}'", value.format("%Y-%m-%d"))
}

fn sql_optional_date(value: Option<NaiveDate>) -> String {
    value.map(sql_date).unwrap_or_else(|| "NULL".to_string())
}

fn sql_timestamp(value: chrono::DateTime<Utc>) -> String {
    format!("TIMESTAMP({})", sql_string(&value.to_rfc3339()))
}

fn sql_json(value: &Value) -> String {
    format!("PARSE_JSON({})", sql_string(&value.to_string()))
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
            USING (SELECT {} AS version, CURRENT_TIMESTAMP() AS applied_at) source
            ON target.version = source.version
            WHEN MATCHED THEN UPDATE SET applied_at = source.applied_at
            WHEN NOT MATCHED THEN INSERT (version, applied_at) VALUES (source.version, source.applied_at)
            ",
            self.qualified_table("schema_versions")?,
            sql_string(version),
        );
        self.run_query(&sql).await?;
        Ok(())
    }

    async fn upsert_accounts(&self, rows: &[AccountRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let source = rows
            .iter()
            .map(|row| {
                format!(
                    "SELECT {} AS account_id, {} AS owner, {} AS account_type, {} AS bank, {} AS label, {} AS pluggy_account_id, {} AS pluggy_item_id, {} AS status, {} AS actor_id, {} AS idempotency_key, {} AS metadata_json, {} AS created_at, {} AS updated_at",
                    sql_string(&row.account_id),
                    sql_string(&row.owner),
                    sql_string(&row.account_type),
                    sql_string(&row.bank),
                    sql_string(&row.label),
                    sql_optional_string(row.pluggy_account_id.as_deref()),
                    sql_optional_string(row.pluggy_item_id.as_deref()),
                    sql_string(&row.status),
                    sql_string(&row.actor_id),
                    sql_string(&row.idempotency_key),
                    sql_json(&row.metadata_json),
                    sql_timestamp(row.created_at),
                    sql_timestamp(row.updated_at),
                )
            })
            .collect::<Vec<_>>()
            .join("\nUNION ALL\n");

        let sql = format!(
            "
            MERGE {} target
            USING ({source}) source
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
        self.run_query(&sql).await?;
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
        let source = rows
            .iter()
            .map(|row| {
                format!(
                    "SELECT {} AS snapshot_id, {} AS account_id, {} AS snapshot_date, {} AS balance, {} AS credit_limit, {} AS currency_code, {} AS source, {} AS actor_id, {} AS idempotency_key, {} AS metadata_json, {} AS created_at",
                    sql_string(&row.snapshot_id),
                    sql_string(&row.account_id),
                    sql_date(row.snapshot_date),
                    row.balance.as_ref().map(sql_decimal).unwrap_or_else(|| "CAST(NULL AS NUMERIC)".to_string()),
                    row.credit_limit.as_ref().map(sql_decimal).unwrap_or_else(|| "CAST(NULL AS NUMERIC)".to_string()),
                    sql_optional_string(row.currency_code.as_deref()),
                    sql_string(&row.source),
                    sql_string(&row.actor_id),
                    sql_string(&row.idempotency_key),
                    sql_json(&row.metadata_json),
                    sql_timestamp(row.created_at),
                )
            })
            .collect::<Vec<_>>()
            .join("\nUNION ALL\n");

        let sql = format!(
            "
            MERGE {} target
            USING ({source}) source
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
        self.run_query(&sql).await?;
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
            WHERE LOWER(COALESCE(raw_description, '')) LIKE {}
               OR LOWER(COALESCE(description, '')) LIKE {}
               OR LOWER(COALESCE(merchant_name, '')) LIKE {}
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT {}
            ",
            self.qualified_table("transactions")?,
            sql_string(&pattern),
            sql_string(&pattern),
            sql_string(&pattern),
            limit,
        );
        let response = self.run_query(&sql).await?;
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
            WHERE description IS NULL
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT {}
            ",
            self.qualified_table("transactions")?,
            limit,
        );
        let response = self.run_query(&sql).await?;
        response
            .rows
            .iter()
            .map(|row| transaction_record_from_values(&row_values(row)))
            .collect()
    }

    async fn pending_human_descriptions(&self, limit: usize) -> Result<Vec<TransactionRecord>> {
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
            LIMIT {}
            ",
            self.qualified_table("transactions")?,
            limit,
        );
        let response = self.run_query(&sql).await?;
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
            LIMIT {}
            ",
            self.qualified_table("transactions")?,
            limit,
        );
        let response = self.run_query(&sql).await?;
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
              AND ABS(amount) >= {}
              AND category_id IS NOT NULL
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT {}
            ",
            self.qualified_table("transactions")?,
            sql_decimal(&min_abs_amount.abs()),
            limit,
        );
        let response = self.run_query(&sql).await?;
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
            self.qualified_table("transactions")?,
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
            self.qualified_table("transactions")?,
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
              AND ABS(amount) >= {}
              AND category_id IS NOT NULL
            ",
            self.qualified_table("transactions")?,
            sql_decimal(&min_abs_amount.abs()),
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

    async fn upsert_transactions(&self, rows: &[TransactionRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let source = rows
            .iter()
            .map(|row| {
                format!(
                    "SELECT {} AS transaction_id, {} AS account_id, {} AS transaction_date, {} AS raw_description, {} AS description, {} AS merchant_name, {} AS purpose, {} AS amount, {} AS amount_cents, {} AS tx_type, {} AS category_id, {} AS category_source, {} AS context, {} AS classifier_trace, {} AS payment_status, {} AS source, {} AS actor_id, {} AS idempotency_key, {} AS metadata_json, {} AS created_at, {} AS updated_at, {} AS enrichment_attempted_at",
                    sql_string(&row.transaction_id),
                    sql_optional_string(row.account_id.as_deref()),
                    sql_date(row.transaction_date),
                    sql_string(&row.raw_description),
                    sql_optional_string(row.description.as_deref()),
                    sql_optional_string(row.merchant_name.as_deref()),
                    sql_optional_string(row.purpose.as_deref()),
                    sql_decimal(&row.amount),
                    (row.amount * Decimal::from(100_i64)).round().to_i64().unwrap_or(0),
                    sql_string(&row.tx_type),
                    sql_optional_string(row.category_id.as_deref()),
                    sql_string(&row.category_source),
                    sql_optional_string(row.context.as_deref()),
                    sql_optional_string(row.classifier_trace.as_deref()),
                    sql_string(&row.payment_status),
                    sql_string(&row.source),
                    sql_string(&row.actor_id),
                    sql_string(&row.idempotency_key),
                    sql_json(&row.metadata_json),
                    sql_timestamp(row.created_at),
                    sql_timestamp(row.updated_at),
                    row.enrichment_attempted_at
                        .map(sql_timestamp)
                        .unwrap_or_else(|| "CAST(NULL AS TIMESTAMP)".to_string()),
                )
            })
            .collect::<Vec<_>>()
            .join("\nUNION ALL\n");

        let sql = format!(
            "
            MERGE {} target
            USING ({source}) source
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
        self.run_query(&sql).await?;
        Ok(rows.len())
    }

    async fn upsert_rules(&self, rows: &[RuleRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let source = rows
            .iter()
            .map(|row| {
                format!(
                    "SELECT {} AS rule_id, {} AS body, {} AS status, {} AS actor_id, {} AS idempotency_key, {} AS created_at, {} AS updated_at",
                    sql_string(&row.rule_id),
                    sql_string(&row.body),
                    sql_string(&row.status),
                    sql_string(&row.actor_id),
                    sql_string(&row.idempotency_key),
                    sql_timestamp(row.created_at),
                    sql_timestamp(row.updated_at),
                )
            })
            .collect::<Vec<_>>()
            .join("\nUNION ALL\n");
        let sql = format!(
            "
            MERGE {} target
            USING ({source}) source
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
        self.run_query(&sql).await?;
        Ok(rows.len())
    }

    async fn upsert_categories(&self, rows: &[CategoryRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let source = rows
            .iter()
            .map(|row| {
                format!(
                    "SELECT {} AS category_id, {} AS name, {} AS parent_category_id, {} AS metadata_json, {} AS actor_id, {} AS updated_at",
                    sql_string(&row.category_id),
                    sql_string(&row.name),
                    sql_optional_string(row.parent_category_id.as_deref()),
                    sql_json(&row.metadata_json),
                    sql_string(&row.actor_id),
                    sql_timestamp(row.updated_at),
                )
            })
            .collect::<Vec<_>>()
            .join("\nUNION ALL\n");
        let sql = format!(
            "
            MERGE {} target
            USING ({source}) source
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
        self.run_query(&sql).await?;
        Ok(rows.len())
    }

    async fn upsert_forecasts(&self, rows: &[ForecastRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let source = rows
            .iter()
            .map(|row| {
                format!(
                    "SELECT {} AS forecast_id, {} AS due_date, {} AS description, {} AS amount, {} AS category_id, {} AS account_id, {} AS status, {} AS recurrence, {} AS actor_id, {} AS idempotency_key, {} AS metadata_json, {} AS created_at, {} AS updated_at",
                    sql_string(&row.forecast_id),
                    sql_optional_date(row.due_date),
                    sql_string(&row.description),
                    sql_decimal(&row.amount),
                    sql_optional_string(row.category_id.as_deref()),
                    sql_optional_string(row.account_id.as_deref()),
                    sql_string(&row.status),
                    sql_optional_string(row.recurrence.as_deref()),
                    sql_string(&row.actor_id),
                    sql_string(&row.idempotency_key),
                    sql_json(&row.metadata_json),
                    sql_timestamp(row.created_at),
                    sql_timestamp(row.updated_at),
                )
            })
            .collect::<Vec<_>>()
            .join("\nUNION ALL\n");
        let sql = format!(
            "
            MERGE {} target
            USING ({source}) source
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
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              forecast_id, due_date, description, amount, category_id, account_id, status,
              recurrence, actor_id, idempotency_key, metadata_json, created_at, updated_at
            ) VALUES (
              source.forecast_id, source.due_date, source.description, source.amount, source.category_id, source.account_id, source.status,
              source.recurrence, source.actor_id, source.idempotency_key, source.metadata_json, source.created_at, source.updated_at
            )
            ",
            self.qualified_table("forecast")?,
        );
        self.run_query(&sql).await?;
        Ok(rows.len())
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
              FORMAT_TIMESTAMP('%FT%T%Ez', updated_at)
            FROM {}
            WHERE status = 'ativo'
              AND due_date IS NOT NULL
              AND due_date BETWEEN DATE {} AND DATE {}
            ORDER BY due_date ASC, amount DESC
            ",
            self.qualified_table("forecast")?,
            sql_string(&from.format("%Y-%m-%d").to_string()),
            sql_string(&until.format("%Y-%m-%d").to_string()),
        );
        let response = self.run_query(&sql).await?;
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

        let split_source = format!(
            "SELECT {} AS split_id, {} AS parent_transaction_id, {} AS payload_hash, {} AS status, {} AS source, {} AS notes, {} AS actor_id, {} AS idempotency_key, {} AS metadata_json, {} AS created_at, {} AS updated_at",
            sql_string(&split.split_id),
            sql_string(&split.parent_transaction_id),
            sql_string(&split.payload_hash),
            sql_string(&split.status),
            sql_string(&split.source),
            sql_optional_string(split.notes.as_deref()),
            sql_string(&split.actor_id),
            sql_string(&split.idempotency_key),
            sql_json(&split.metadata_json),
            sql_timestamp(split.created_at),
            sql_timestamp(split.updated_at),
        );
        let line_source = lines
            .iter()
            .map(|row| {
                format!(
                    "SELECT {} AS split_line_id, {} AS split_id, {} AS parent_transaction_id, {} AS line_index, {} AS description, {} AS amount, {} AS category_id, {} AS category_source, {} AS context, {} AS status, {} AS actor_id, {} AS idempotency_key, {} AS metadata_json, {} AS created_at, {} AS updated_at",
                    sql_string(&row.split_line_id),
                    sql_string(&row.split_id),
                    sql_string(&row.parent_transaction_id),
                    row.line_index,
                    sql_string(&row.description),
                    sql_decimal(&row.amount),
                    sql_optional_string(row.category_id.as_deref()),
                    sql_string(&row.category_source),
                    sql_optional_string(row.context.as_deref()),
                    sql_string(&row.status),
                    sql_string(&row.actor_id),
                    sql_string(&row.idempotency_key),
                    sql_json(&row.metadata_json),
                    sql_timestamp(row.created_at),
                    sql_timestamp(row.updated_at),
                )
            })
            .collect::<Vec<_>>()
            .join("\nUNION ALL\n");
        let item_statement = if items.is_empty() {
            String::new()
        } else {
            let item_source = items
                .iter()
                .map(|row| {
                    let quantity = row
                        .quantity
                        .as_ref()
                        .map(sql_decimal)
                        .unwrap_or_else(|| "CAST(NULL AS NUMERIC)".to_string());
                    let unit_price = row
                        .unit_price
                        .as_ref()
                        .map(sql_decimal)
                        .unwrap_or_else(|| "CAST(NULL AS NUMERIC)".to_string());
                    let total_price = row
                        .total_price
                        .as_ref()
                        .map(sql_decimal)
                        .unwrap_or_else(|| "CAST(NULL AS NUMERIC)".to_string());
                    format!(
                        "SELECT {} AS receipt_item_id, {} AS parent_transaction_id, {} AS split_id, {} AS split_line_id, {} AS item_index, {} AS description, {} AS quantity, {} AS unit, {} AS unit_price, {} AS total_price, {} AS code, {} AS store_name, {} AS status, {} AS actor_id, {} AS idempotency_key, {} AS metadata_json, {} AS created_at, {} AS updated_at",
                        sql_string(&row.receipt_item_id),
                        sql_string(&row.parent_transaction_id),
                        sql_optional_string(row.split_id.as_deref()),
                        sql_optional_string(row.split_line_id.as_deref()),
                        row.item_index,
                        sql_string(&row.description),
                        quantity,
                        sql_optional_string(row.unit.as_deref()),
                        unit_price,
                        total_price,
                        sql_optional_string(row.code.as_deref()),
                        sql_optional_string(row.store_name.as_deref()),
                        sql_string(&row.status),
                        sql_string(&row.actor_id),
                        sql_string(&row.idempotency_key),
                        sql_json(&row.metadata_json),
                        sql_timestamp(row.created_at),
                        sql_timestamp(row.updated_at),
                    )
                })
                .collect::<Vec<_>>()
                .join("\nUNION ALL\n");
            format!(
                "
                MERGE {} target
                USING ({item_source}) source
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
            WHERE parent_transaction_id = {}
              AND status = 'active'
              AND split_id != {};

            UPDATE {}
            SET status = 'inactive', updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = {}
              AND status = 'active'
              AND split_id != {};

            UPDATE {}
            SET status = 'inactive', updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = {}
              AND status = 'active'
              AND COALESCE(split_id, '') != {};

            MERGE {} target
            USING ({split_source}) source
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
            USING ({line_source}) source
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
            sql_string(&split.parent_transaction_id),
            sql_string(&split.split_id),
            self.qualified_table("transaction_split_lines")?,
            sql_string(&split.parent_transaction_id),
            sql_string(&split.split_id),
            self.qualified_table("receipt_items")?,
            sql_string(&split.parent_transaction_id),
            sql_string(&split.split_id),
            self.qualified_table("transaction_splits")?,
            self.qualified_table("transaction_split_lines")?,
        );
        self.run_query(&sql).await?;
        Ok(())
    }

    async fn insert_audit_events(&self, rows: &[AuditEvent]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let source = rows
            .iter()
            .map(|row| {
                format!(
                    "SELECT {} AS event_id, {} AS entity_type, {} AS entity_id, {} AS action, {} AS actor_id, {} AS event_timestamp, {} AS idempotency_key, {} AS diff_json",
                    sql_string(&row.event_id),
                    sql_string(&row.entity_type),
                    sql_string(&row.entity_id),
                    sql_string(&row.action),
                    sql_string(&row.actor_id),
                    sql_timestamp(row.event_timestamp),
                    sql_string(&row.idempotency_key),
                    sql_json(&row.diff_json),
                )
            })
            .collect::<Vec<_>>()
            .join("\nUNION ALL\n");
        let sql = format!(
            "
            INSERT INTO {} (event_id, entity_type, entity_id, action, actor_id, event_timestamp, idempotency_key, diff_json)
            {source}
            ",
            self.qualified_table("audit_log")?,
        );
        self.run_query(&sql).await?;
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
        let sql = format!(
            "
            UPDATE {}
            SET category_id = COALESCE({}, category_id),
                category_source = COALESCE({}, category_source),
                classifier_trace = COALESCE({}, classifier_trace),
                actor_id = {},
                idempotency_key = {},
                updated_at = CURRENT_TIMESTAMP()
            WHERE transaction_id = {}
            ",
            self.qualified_table("transactions")?,
            sql_optional_string(category_id),
            sql_optional_string(category_source),
            sql_optional_string(classifier_trace),
            sql_string(actor_id),
            sql_string(idempotency_key),
            sql_string(transaction_id),
        );
        let resp = self.run_query(&sql).await?;
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
        let sql = format!(
            "
            UPDATE {}
            SET description = COALESCE({}, description),
                merchant_name = COALESCE({}, merchant_name),
                purpose = COALESCE({}, purpose),
                classifier_trace = COALESCE({}, classifier_trace),
                context = COALESCE({}, context),
                actor_id = {},
                idempotency_key = {},
                updated_at = CURRENT_TIMESTAMP()
            WHERE transaction_id = {}
            ",
            self.qualified_table("transactions")?,
            sql_optional_string(patch.description),
            sql_optional_string(patch.merchant_name),
            sql_optional_string(patch.purpose),
            sql_optional_string(patch.classifier_trace),
            sql_optional_string(patch.description),
            sql_string(actor_id),
            sql_string(idempotency_key),
            sql_string(transaction_id),
        );
        let resp = self.run_query(&sql).await?;
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
        let id_array = ids
            .iter()
            .map(|value| sql_string(value))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "
            SELECT transaction_id
            FROM {}
            WHERE transaction_id IN UNNEST([{id_array}])
            ",
            self.qualified_table("transactions")?,
        );
        let response = self.run_query(&sql).await?;
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
            WHERE transaction_id = {}
            LIMIT 1
            ",
            self.qualified_table("transactions")?,
            sql_string(transaction_id),
        );
        let response = self.run_query(&sql).await?;
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
            WHERE parent_transaction_id = {}
              AND status = 'active'
            ORDER BY updated_at DESC, split_id DESC
            LIMIT 1
            ",
            self.qualified_table("transaction_splits")?,
            sql_string(transaction_id),
        );
        let split_response = self.run_query(&split_sql).await?;
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
            WHERE split_id = {}
              AND status = 'active'
            ORDER BY line_index ASC
            ",
            self.qualified_table("transaction_split_lines")?,
            sql_string(&active_split.split_id),
        );
        let line_response = self.run_query(&lines_sql).await?;
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
            WHERE split_id = {}
              AND status = 'active'
            ORDER BY item_index ASC
            ",
            self.qualified_table("receipt_items")?,
            sql_string(&active_split.split_id),
        );
        let item_response = self.run_query(&items_sql).await?;
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
                actor_id = {},
                idempotency_key = {},
                updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = {}
              AND status = 'active';

            UPDATE {}
            SET status = 'inactive',
                actor_id = {},
                idempotency_key = {},
                updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = {}
              AND status = 'active';

            UPDATE {}
            SET status = 'inactive',
                actor_id = {},
                idempotency_key = {},
                updated_at = CURRENT_TIMESTAMP()
            WHERE parent_transaction_id = {}
              AND status = 'active';
            ",
            self.qualified_table("transaction_splits")?,
            sql_string(actor_id),
            sql_string(idempotency_key),
            sql_string(transaction_id),
            self.qualified_table("transaction_split_lines")?,
            sql_string(actor_id),
            sql_string(idempotency_key),
            sql_string(transaction_id),
            self.qualified_table("receipt_items")?,
            sql_string(actor_id),
            sql_string(idempotency_key),
            sql_string(transaction_id),
        );
        self.run_query(&sql).await?;
        Ok(())
    }

    async fn split_candidates(&self, since: NaiveDate) -> Result<Vec<SplitCandidateRow>> {
        let sql = format!(
            "
            WITH unsplit_transactions AS (
              SELECT t.*
              FROM {} t
              WHERE t.transaction_date >= {}
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
            sql_date(since),
            self.qualified_table("transaction_splits")?,
            self.qualified_table("split_review_policies")?,
        );
        let response = self.run_query(&sql).await?;
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
        let query_filter = if query.trim().is_empty() {
            String::new()
        } else {
            format!(
                "AND STRPOS(LOWER(i.description), LOWER({})) > 0",
                sql_string(query.trim())
            )
        };
        let since_filter = since
            .map(|value| format!("AND t.transaction_date >= {}", sql_date(value)))
            .unwrap_or_default();
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
        let response = self.run_query(&sql).await?;
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
            WHERE transaction_date >= {}
            ORDER BY transaction_date DESC, amount ASC, transaction_id ASC
            ",
            self.qualified_table("v_daily_pulse")?,
            sql_date(since),
        );
        let response = self.run_query(&sql).await?;
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
        since: NaiveDate,
        until: NaiveDate,
    ) -> Result<Vec<TransactionRecord>> {
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
            WHERE transaction_date >= {}
              AND transaction_date <= {}
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            ",
            self.qualified_table("v_transactions_reportable")?,
            sql_date(since),
            sql_date(until),
        );
        let response = self.run_query(&sql).await?;
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
        let account_clause = account_id
            .map(|id| format!("AND account_id = {}", sql_string(id)))
            .unwrap_or_default();
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
            WHERE transaction_date >= {}
              AND transaction_date <= {}
              {account_clause}
            ORDER BY transaction_date ASC, transaction_id ASC
            ",
            self.qualified_table("transactions")?,
            sql_date(from),
            sql_date(to),
        );
        let response = self.run_query(&sql).await?;
        let mut items = Vec::with_capacity(response.rows.len());
        for row in response.rows {
            let values = row_values(&row);
            items.push(transaction_record_from_values(&values)?);
        }
        Ok(items)
    }

    async fn monthly_spend(&self, month_ref: Option<&str>) -> Result<Vec<MonthlySpendRow>> {
        let where_clause = month_ref
            .map(|value| format!("WHERE month_ref = {}", sql_string(value)))
            .unwrap_or_default();
        let sql = format!(
            "
            SELECT month_ref, category_id, account_id, CAST(expenses AS STRING), CAST(expense_count AS STRING)
            FROM {}
            {where_clause}
            ORDER BY month_ref DESC, expenses DESC, category_id ASC, account_id ASC
            ",
            self.qualified_table("v_monthly_spend")?,
        );
        let response = self.run_query(&sql).await?;
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
        let sql = format!(
            "
            SELECT month_ref,
                   CAST(income AS STRING),
                   CAST(expenses AS STRING),
                   CAST(expense_reduction AS STRING),
                   CAST(net AS STRING)
            FROM {}
            ORDER BY month_ref DESC
            LIMIT {}
            ",
            self.qualified_table("v_cashflow")?,
            months,
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
            });
        }
        Ok(items)
    }

    async fn forecast_vs_actual(
        &self,
        month_ref: Option<&str>,
    ) -> Result<Vec<ForecastVsActualRow>> {
        let where_clause = month_ref
            .map(|value| format!("WHERE month_ref = {}", sql_string(value)))
            .unwrap_or_default();
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
        let response = self.run_query(&sql).await?;
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
        let where_clause = month_ref
            .map(|value| format!("WHERE month_ref = {}", sql_string(value)))
            .unwrap_or_default();
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
        let where_month = month_ref
            .map(|value| {
                format!(
                    "AND FORMAT_DATE('%Y-%m', t.transaction_date) = {}",
                    sql_string(value)
                )
            })
            .unwrap_or_default();
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
        let response = self.run_query(&sql).await?;
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
        let where_month = month_ref
            .map(|value| {
                format!(
                    "AND FORMAT_DATE('%Y-%m', t.transaction_date) = {}",
                    sql_string(value)
                )
            })
            .unwrap_or_default();
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
        let response = self.run_query(&sql).await?;
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
        // BigQuery doesn't enforce UNIQUE; use MERGE for upsert by business key
        let sub_key = record
            .subcategory_id
            .as_deref()
            .map(sql_string)
            .unwrap_or_else(|| "CAST(NULL AS STRING)".to_string());
        let month_key = record
            .month_ref
            .as_deref()
            .map(sql_string)
            .unwrap_or_else(|| "CAST(NULL AS STRING)".to_string());
        let sql = format!(
            "
            MERGE {table} target
            USING (SELECT
              {budget_id} AS budget_id,
              {category_id} AS category_id,
              {subcategory_id} AS subcategory_id,
              {month_ref} AS month_ref,
              {amount} AS amount,
              {alert_threshold_pct} AS alert_threshold_pct,
              {actor_id} AS actor_id,
              {idempotency_key} AS idempotency_key,
              {created_at} AS created_at,
              {updated_at} AS updated_at
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
            budget_id = sql_string(&record.budget_id),
            category_id = sql_string(&record.category_id),
            subcategory_id = sub_key,
            month_ref = month_key,
            amount = sql_decimal(&record.amount),
            alert_threshold_pct = record.alert_threshold_pct,
            actor_id = sql_string(&record.actor_id),
            idempotency_key = sql_string(&record.idempotency_key),
            created_at = sql_timestamp(record.created_at),
            updated_at = sql_timestamp(record.updated_at),
        );
        self.run_query(&sql).await?;
        Ok(())
    }

    async fn list_category_budgets(
        &self,
        month: Option<&str>,
    ) -> Result<Vec<CategoryBudgetRecord>> {
        let table = self.qualified_table("category_budgets")?;
        let month_filter = match month {
            Some(m) => format!("month_ref = {} OR month_ref IS NULL", sql_string(m)),
            None => "TRUE".to_string(),
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
        let response = self.run_query(&sql).await?;
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
            WHERE month_ref = {month}
            GROUP BY category_id
            ",
            month = sql_string(month),
        );
        let spend_response = self.run_query(&spend_sql).await?;
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
            WHERE month_ref = {month} OR month_ref IS NULL
            ORDER BY category_id ASC, subcategory_id ASC,
                     CASE WHEN month_ref IS NOT NULL THEN 0 ELSE 1 END ASC
            ",
            month = sql_string(month),
        );
        let budget_response = self.run_query(&budget_sql).await?;
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
            WHERE transaction_date = {}
              AND account_id = {}
              AND transaction_id != {}
            ORDER BY pluggy_order IS NULL, pluggy_order ASC, raw_description ASC
            ",
            self.qualified_table("transactions")?,
            sql_date(date),
            sql_string(account_id),
            sql_string(exclude_id),
        );
        let response = self.run_query(&sql).await?;
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
            WHERE LOWER(COALESCE(raw_description, '')) LIKE {}
              AND transaction_id != {}
              {category_filter}
            ORDER BY transaction_date DESC, transaction_id ASC
            ",
            self.qualified_table("transactions")?,
            sql_string(&pattern),
            sql_string(exclude_id),
        );
        let response = self.run_query(&sql).await?;
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
            "UPDATE {} SET enrichment_attempted_at = CURRENT_TIMESTAMP() WHERE transaction_id = {}",
            self.qualified_table("transactions")?,
            sql_string(transaction_id),
        );
        self.run_query(&sql).await?;
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
