use crate::idempotency::{account_idempotency, category_id, pluggy_transaction_idempotency};
use crate::legacy::{load_account_registry, AccountRegistryEntry};
use crate::models::{json_object_or_empty, AccountRecord, TransactionRecord};
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::task::JoinSet;

const PLUGGY_API: &str = "https://api.pluggy.ai";

#[derive(Debug, Clone, Deserialize)]
pub struct PluggyBindingConfig {
    pub id: String,
    #[serde(rename = "pluggyAccountId")]
    pub pluggy_account_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluggyConfigFile {
    #[serde(rename = "syncStartDate")]
    pub sync_start_date: Option<String>,
    #[serde(default)]
    pub accounts: Vec<PluggyBindingConfig>,
}

#[derive(Debug, Deserialize)]
pub struct PluggyFixture {
    #[serde(default)]
    pub accounts: Vec<PluggyAccountPayload>,
    #[serde(default)]
    pub transactions: Vec<PluggyTransactionPayload>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluggyAccountPayload {
    pub id: String,
    #[serde(default, rename = "itemId")]
    pub item_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default, rename = "type")]
    pub account_type: Option<String>,
    #[serde(default)]
    pub balance: Option<f64>,
    #[serde(default, rename = "currencyCode")]
    pub currency_code: Option<String>,
    #[serde(default)]
    pub number: Option<String>,
    #[serde(default, rename = "marketingName")]
    pub marketing_name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluggyTransactionPayload {
    pub id: String,
    #[serde(rename = "accountId")]
    pub account_id: String,
    pub date: String,
    pub description: String,
    pub amount: f64,
    #[serde(default, rename = "type")]
    pub tx_type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default, rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Deserialize)]
struct PluggyAuthResponse {
    #[serde(rename = "apiKey")]
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct PaginatedResponse<T> {
    #[serde(default)]
    results: Vec<T>,
    #[serde(default, rename = "totalPages")]
    total_pages: Option<u32>,
}

fn parse_datetime(value: Option<&str>) -> DateTime<Utc> {
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

fn parse_date(value: &str) -> Result<NaiveDate> {
    let raw = value.get(0..10).unwrap_or(value);
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .with_context(|| format!("Falha ao parsear data Pluggy {value}"))
}

fn account_type_from_payload(payload: &PluggyAccountPayload) -> String {
    payload
        .account_type
        .clone()
        .or_else(|| payload.subtype.clone())
        .unwrap_or_else(|| "unknown".to_string())
        .to_ascii_lowercase()
}

fn build_account_record(
    payload: PluggyAccountPayload,
    binding: &PluggyBindingConfig,
    registry: Option<&AccountRegistryEntry>,
    actor_id: &str,
) -> AccountRecord {
    let updated_at = parse_datetime(payload.updated_at.as_deref());
    let registry_id = registry.map(|entry| entry.account_id.clone());
    let account_id = registry_id.unwrap_or_else(|| binding.id.clone());
    AccountRecord {
        account_id: account_id.clone(),
        owner: registry
            .map(|entry| entry.owner.clone())
            .unwrap_or_else(|| "shared".to_string()),
        account_type: registry
            .map(|entry| entry.account_type.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| account_type_from_payload(&payload)),
        bank: registry
            .map(|entry| entry.bank.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "pluggy".to_string()),
        label: registry
            .map(|entry| entry.label.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| payload.name.clone().unwrap_or_else(|| binding.id.clone())),
        pluggy_account_id: Some(payload.id.clone()),
        pluggy_item_id: payload.item_id.clone(),
        status: payload
            .status
            .clone()
            .unwrap_or_else(|| "active".to_string())
            .to_ascii_lowercase(),
        actor_id: actor_id.to_string(),
        idempotency_key: account_idempotency(&account_id),
        metadata_json: json!({
            "name": payload.name,
            "subtype": payload.subtype,
            "raw_type": payload.account_type,
            "balance": payload.balance,
            "currency_code": payload.currency_code,
            "number": payload.number,
            "marketing_name": payload.marketing_name,
            "raw": json_object_or_empty(Some(payload.extra)),
        }),
        created_at: updated_at,
        updated_at,
    }
}

fn build_transaction_record(
    payload: PluggyTransactionPayload,
    binding: &PluggyBindingConfig,
    registry: Option<&AccountRegistryEntry>,
    actor_id: &str,
) -> Result<TransactionRecord> {
    let category_name = payload
        .category
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let category_key = category_name.map(|value| category_id(value, None));
    let transaction_date = parse_date(&payload.date)?;
    let amount = rust_decimal::Decimal::from_f64_retain(payload.amount)
        .context("Falha ao converter amount Pluggy para decimal")?;
    let created_at = parse_datetime(payload.created_at.as_deref());
    let updated_at = parse_datetime(payload.updated_at.as_deref());
    Ok(TransactionRecord {
        transaction_id: payload.id.clone(),
        account_id: registry
            .map(|entry| entry.account_id.clone())
            .or_else(|| Some(binding.id.clone())),
        transaction_date,
        description: payload.description.clone(),
        amount,
        tx_type: payload
            .tx_type
            .clone()
            .unwrap_or_else(|| {
                if payload.amount.is_sign_negative() {
                    "debit".to_string()
                } else {
                    "credit".to_string()
                }
            })
            .to_ascii_lowercase(),
        category_id: category_key,
        category_source: if payload.category.is_some() {
            "pluggy".to_string()
        } else {
            "unclassified".to_string()
        },
        context: None,
        payment_status: payload
            .status
            .clone()
            .unwrap_or_else(|| "posted".to_string())
            .to_ascii_lowercase(),
        source: "pluggy".to_string(),
        actor_id: actor_id.to_string(),
        idempotency_key: pluggy_transaction_idempotency(&payload.id),
        metadata_json: json!({
            "pluggy_account_id": payload.account_id,
            "pluggy_category": payload.category,
            "raw": json_object_or_empty(Some(payload.extra)),
        }),
        created_at,
        updated_at,
    })
}

async fn authenticate(client: &Client) -> Result<String> {
    let client_id = std::env::var("PLUGGY_CLIENT_ID").context("PLUGGY_CLIENT_ID ausente")?;
    let client_secret =
        std::env::var("PLUGGY_CLIENT_SECRET").context("PLUGGY_CLIENT_SECRET ausente")?;
    let response = client
        .post(format!("{PLUGGY_API}/auth"))
        .json(&json!({
            "clientId": client_id,
            "clientSecret": client_secret,
        }))
        .send()
        .await
        .context("Falha ao autenticar no Pluggy")?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Auth Pluggy falhou: {body}"));
    }
    let body: PluggyAuthResponse = response
        .json()
        .await
        .context("JSON inválido no auth Pluggy")?;
    Ok(body.api_key)
}

async fn fetch_account_details(
    client: &Client,
    api_key: &str,
    account_id: &str,
) -> Result<PluggyAccountPayload> {
    let response = client
        .get(format!("{PLUGGY_API}/accounts/{account_id}"))
        .header("X-API-KEY", api_key)
        .send()
        .await
        .with_context(|| format!("Falha ao consultar conta Pluggy {account_id}"))?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Consulta da conta Pluggy {account_id} falhou: {body}"
        ));
    }
    response
        .json()
        .await
        .context("JSON inválido ao consultar conta Pluggy")
}

async fn fetch_transactions(
    client: &Client,
    api_key: &str,
    account_id: &str,
    from: &str,
    to: &str,
) -> Result<Vec<PluggyTransactionPayload>> {
    let mut page = 1;
    let mut total_pages = 1;
    let mut all = Vec::new();

    while page <= total_pages {
        let response = client
            .get(format!(
                "{PLUGGY_API}/transactions?accountId={account_id}&from={from}&to={to}&pageSize=500&page={page}"
            ))
            .header("X-API-KEY", api_key)
            .send()
            .await
            .with_context(|| format!("Falha ao consultar transações Pluggy {account_id}"))?;
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Consulta de transações Pluggy {account_id} falhou: {body}"
            ));
        }
        let body: PaginatedResponse<PluggyTransactionPayload> = response
            .json()
            .await
            .context("JSON inválido ao consultar transações Pluggy")?;
        total_pages = body.total_pages.unwrap_or(1);
        all.extend(body.results);
        page += 1;
    }

    Ok(all)
}

pub async fn sync_pluggy(
    actor_id: &str,
    pluggy_config_path: &Path,
    accounts_csv_path: Option<&Path>,
    fixture_path: Option<&Path>,
    from_override: Option<&str>,
    to_date: &str,
) -> Result<(Vec<AccountRecord>, Vec<TransactionRecord>)> {
    let registry = accounts_csv_path
        .filter(|path| path.exists())
        .map(load_account_registry)
        .transpose()?
        .unwrap_or_default();
    let config_raw = fs::read_to_string(pluggy_config_path)
        .with_context(|| format!("Falha ao ler {}", pluggy_config_path.display()))?;
    let config: PluggyConfigFile =
        serde_json::from_str(&config_raw).context("Falha ao parsear pluggy-config.json")?;
    let from = from_override
        .map(|value| value.to_string())
        .or(config.sync_start_date.clone())
        .unwrap_or_else(|| "2025-12-01".to_string());

    if let Some(fixture_path) = fixture_path {
        let fixture_raw = fs::read_to_string(fixture_path)
            .with_context(|| format!("Falha ao ler fixture {}", fixture_path.display()))?;
        let fixture: PluggyFixture =
            serde_json::from_str(&fixture_raw).context("Falha ao parsear fixture Pluggy")?;
        let account_map = fixture
            .accounts
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect::<BTreeMap<_, _>>();
        let mut accounts = Vec::new();
        let mut transactions = Vec::new();
        for binding in &config.accounts {
            let payload = account_map
                .get(&binding.pluggy_account_id)
                .cloned()
                .with_context(|| {
                    format!(
                        "Fixture não contém conta Pluggy {}",
                        binding.pluggy_account_id
                    )
                })?;
            let registry_entry = registry
                .get(&binding.id)
                .or_else(|| registry.get(&format!("pluggy:{}", binding.pluggy_account_id)));
            accounts.push(build_account_record(
                payload,
                binding,
                registry_entry,
                actor_id,
            ));
        }
        for payload in fixture.transactions {
            let binding = config
                .accounts
                .iter()
                .find(|item| item.pluggy_account_id == payload.account_id)
                .with_context(|| {
                    format!(
                        "Transação de fixture sem binding para conta {}",
                        payload.account_id
                    )
                })?;
            let registry_entry = registry
                .get(&binding.id)
                .or_else(|| registry.get(&format!("pluggy:{}", binding.pluggy_account_id)));
            transactions.push(build_transaction_record(
                payload,
                binding,
                registry_entry,
                actor_id,
            )?);
        }
        return Ok((accounts, transactions));
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .context("Falha ao construir cliente HTTP do Pluggy")?;
    let api_key = authenticate(&client).await?;
    let mut accounts = Vec::new();
    let mut transactions = Vec::new();
    let mut tasks = JoinSet::new();

    for binding in config.accounts.clone() {
        let registry_entry = registry
            .get(&binding.id)
            .or_else(|| registry.get(&format!("pluggy:{}", binding.pluggy_account_id)))
            .cloned();
        let client = client.clone();
        let api_key = api_key.clone();
        let from = from.clone();
        let to = to_date.to_string();
        let actor_id = actor_id.to_string();
        tasks.spawn(async move {
            let account =
                fetch_account_details(&client, &api_key, &binding.pluggy_account_id).await?;
            let account_record =
                build_account_record(account, &binding, registry_entry.as_ref(), &actor_id);
            let account_transactions =
                fetch_transactions(&client, &api_key, &binding.pluggy_account_id, &from, &to)
                    .await?;
            let transaction_records = account_transactions
                .into_iter()
                .map(|payload| {
                    build_transaction_record(payload, &binding, registry_entry.as_ref(), &actor_id)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok::<_, anyhow::Error>((account_record, transaction_records))
        });
    }

    while let Some(result) = tasks.join_next().await {
        let (account, mut account_transactions) = result
            .context("Task Pluggy falhou ao sincronizar conta")??;
        accounts.push(account);
        transactions.append(&mut account_transactions);
    }

    Ok((accounts, transactions))
}
