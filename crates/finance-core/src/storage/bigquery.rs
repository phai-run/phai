use super::FinanceStore;
use crate::config::AppConfig;
use crate::models::{
    AccountRecord, AuditEvent, CardSummaryRow, CashflowRow, CategoryRecord, DailyPulseItem,
    ForecastRecord, ForecastVsActualRow, MonthlySpendRow, RuleRecord, TransactionRecord,
    UncategorizedRow,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};
use yup_oauth2::{read_service_account_key, ServiceAccountAuthenticator};

const BIGQUERY_SCOPE: &str = "https://www.googleapis.com/auth/bigquery";

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
        let token = auth.token(&[BIGQUERY_SCOPE]).await?;
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
                return Err(anyhow!("BigQuery job não completou após {MAX_POLL_ATTEMPTS} tentativas"));
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

    async fn upsert_transactions(&self, rows: &[TransactionRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let source = rows
            .iter()
            .map(|row| {
                format!(
                    "SELECT {} AS transaction_id, {} AS account_id, {} AS transaction_date, {} AS description, {} AS amount, {} AS tx_type, {} AS category_id, {} AS category_source, {} AS context, {} AS payment_status, {} AS source, {} AS actor_id, {} AS idempotency_key, {} AS metadata_json, {} AS created_at, {} AS updated_at",
                    sql_string(&row.transaction_id),
                    sql_optional_string(row.account_id.as_deref()),
                    sql_date(row.transaction_date),
                    sql_string(&row.description),
                    sql_decimal(&row.amount),
                    sql_string(&row.tx_type),
                    sql_optional_string(row.category_id.as_deref()),
                    sql_string(&row.category_source),
                    sql_optional_string(row.context.as_deref()),
                    sql_string(&row.payment_status),
                    sql_string(&row.source),
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
            ON target.transaction_id = source.transaction_id
            WHEN MATCHED THEN UPDATE SET
              account_id = source.account_id,
              transaction_date = source.transaction_date,
              description = source.description,
              amount = source.amount,
              tx_type = source.tx_type,
              category_id = source.category_id,
              category_source = source.category_source,
              context = source.context,
              payment_status = source.payment_status,
              source = source.source,
              actor_id = source.actor_id,
              idempotency_key = source.idempotency_key,
              metadata_json = source.metadata_json,
              updated_at = source.updated_at
            WHEN NOT MATCHED THEN INSERT (
              transaction_id, account_id, transaction_date, description, amount, tx_type,
              category_id, category_source, context, payment_status, source, actor_id,
              idempotency_key, metadata_json, created_at, updated_at
            ) VALUES (
              source.transaction_id, source.account_id, source.transaction_date, source.description, source.amount, source.tx_type,
              source.category_id, source.category_source, source.context, source.payment_status, source.source, source.actor_id,
              source.idempotency_key, source.metadata_json, source.created_at, source.updated_at
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
        context: Option<&str>,
        actor_id: &str,
        idempotency_key: &str,
    ) -> Result<()> {
        let sql = format!(
            "
            UPDATE {}
            SET category_id = COALESCE({}, category_id),
                category_source = COALESCE({}, category_source),
                context = COALESCE({}, context),
                actor_id = {},
                idempotency_key = {},
                updated_at = CURRENT_TIMESTAMP()
            WHERE transaction_id = {}
            ",
            self.qualified_table("transactions")?,
            sql_optional_string(category_id),
            sql_optional_string(category_source),
            sql_optional_string(context),
            sql_string(actor_id),
            sql_string(idempotency_key),
            sql_string(transaction_id),
        );
        self.run_query(&sql).await?;
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
            SELECT month_ref, CAST(income AS STRING), CAST(expenses AS STRING), CAST(net AS STRING)
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
                net: required_decimal(&values, 3, "net")?,
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
                transaction_count: required_i64(&values, 4, "transaction_count")?,
            });
        }
        Ok(items)
    }

    async fn uncategorized(&self, limit: usize) -> Result<Vec<UncategorizedRow>> {
        let sql = format!(
            "
            SELECT
              transaction_id,
              CAST(transaction_date AS STRING),
              description,
              CAST(amount AS STRING),
              account_id,
              category_source,
              payment_status,
              source
            FROM {}
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT {}
            ",
            self.qualified_table("v_uncategorized")?,
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
                category_source: required_string(&values, 5, "category_source")?,
                payment_status: required_string(&values, 6, "payment_status")?,
                source: required_string(&values, 7, "source")?,
            });
        }
        Ok(items)
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
}
