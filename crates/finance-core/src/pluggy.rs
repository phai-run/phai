use crate::idempotency::{account_idempotency, category_id, pluggy_transaction_idempotency};
use crate::legacy::{load_account_registry, AccountRegistryEntry};
use crate::models::{
    json_object_or_empty, parse_datetime_or_now, AccountRecord, TransactionRecord,
};
use crate::rules::{apply_rules, CompiledRule};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tokio::task::JoinSet;

const PLUGGY_API: &str = "https://api.pluggy.ai";

#[derive(Debug, Clone, Deserialize)]
pub struct PluggyBindingConfig {
    pub id: String,
    #[serde(rename = "pluggyAccountId")]
    pub pluggy_account_id: String,
    #[serde(default, rename = "pluggyItemId")]
    pub pluggy_item_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluggyConfigFile {
    #[serde(rename = "syncStartDate")]
    pub sync_start_date: Option<String>,
    #[serde(default)]
    pub accounts: Vec<PluggyBindingConfig>,
}

fn deserialize_decimal<'de, D>(deserializer: D) -> std::result::Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match &value {
        Value::Number(n) => Decimal::from_str(&n.to_string()).map_err(DeError::custom),
        Value::String(s) => Decimal::from_str(s).map_err(DeError::custom),
        _ => Err(DeError::custom("expected number or string for decimal")),
    }
}

fn deserialize_optional_decimal<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Decimal::from_str(&n.to_string())
            .map(Some)
            .map_err(DeError::custom),
        Some(Value::String(s)) => Decimal::from_str(&s).map(Some).map_err(DeError::custom),
        _ => Err(DeError::custom("expected number or string for decimal")),
    }
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
    #[serde(default, rename = "itemId", alias = "item_id")]
    pub item_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default, rename = "type")]
    pub account_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_decimal")]
    pub balance: Option<Decimal>,
    #[serde(default, rename = "currencyCode", alias = "currency_code")]
    pub currency_code: Option<String>,
    #[serde(default)]
    pub number: Option<String>,
    #[serde(default, rename = "marketingName", alias = "marketing_name")]
    pub marketing_name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, rename = "updatedAt", alias = "updated_at")]
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
    #[serde(deserialize_with = "deserialize_decimal")]
    pub amount: Decimal,
    #[serde(default, rename = "type")]
    pub tx_type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default, rename = "createdAt", alias = "created_at")]
    pub created_at: Option<String>,
    #[serde(default, rename = "updatedAt", alias = "updated_at")]
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

fn normalize_match_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn binding_item_id<'a>(
    binding: &'a PluggyBindingConfig,
    registry: Option<&'a AccountRegistryEntry>,
) -> Option<&'a str> {
    binding
        .pluggy_item_id
        .as_deref()
        .or_else(|| registry.and_then(|entry| entry.pluggy_item_id.as_deref()))
}

/// Returns the single `pluggyItemId` for a binding, validating that
/// any values present in config and accounts.csv agree. If they disagree,
/// we fail explicitly instead of silently preferring one source — rebinding
/// to the wrong item would import another account's data.
pub(crate) fn resolve_binding_item_id<'a>(
    binding: &'a PluggyBindingConfig,
    registry: Option<&'a AccountRegistryEntry>,
) -> Result<Option<&'a str>> {
    let config_value = binding.pluggy_item_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let registry_value = registry
        .and_then(|entry| entry.pluggy_item_id.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (config_value, registry_value) {
        (Some(a), Some(b)) if a != b => Err(anyhow::anyhow!(
            "Binding {}: pluggyItemId diverge entre pluggy-config ({}) e contas.csv ({})",
            binding.id,
            a,
            b
        )),
        (Some(v), _) | (_, Some(v)) => Ok(Some(v)),
        (None, None) => Ok(None),
    }
}

/// Emitted when `resolve_account_payload` had to fall back from
/// the configured `pluggyAccountId` to a rebound one (via `itemId`).
#[derive(Debug, Clone)]
pub struct RebindEvent {
    pub binding_id: String,
    pub internal_account_id: String,
    pub from_pluggy_account_id: String,
    pub to_pluggy_account_id: String,
    pub pluggy_item_id: Option<String>,
}

pub struct SyncPluggyParams<'a> {
    pub actor_id: &'a str,
    pub pluggy_config_path: &'a Path,
    pub accounts_csv_path: Option<&'a Path>,
    pub fixture_path: Option<&'a Path>,
    pub from_override: Option<&'a str>,
    pub to_date: &'a str,
    pub rules: &'a [CompiledRule],
    pub internal_categories: &'a BTreeSet<String>,
    pub api_base_url: Option<&'a str>,
}

fn score_account_candidate(
    payload: &PluggyAccountPayload,
    binding: &PluggyBindingConfig,
    registry: Option<&AccountRegistryEntry>,
) -> i32 {
    let mut score = 0;

    if let Some(item_id) = binding_item_id(binding, registry) {
        if payload.item_id.as_deref() == Some(item_id) {
            score += 40;
        }
    }

    if let Some(entry) = registry {
        if !entry.account_type.trim().is_empty()
            && entry
                .account_type
                .eq_ignore_ascii_case(&account_type_from_payload(payload))
        {
            score += 20;
        }

        if let Some(name) = payload.name.as_deref() {
            let payload_name = normalize_match_key(name);
            let label = normalize_match_key(&entry.label);
            if !payload_name.is_empty() && !label.is_empty() {
                if payload_name == label {
                    score += 15;
                } else if payload_name.contains(&label) || label.contains(&payload_name) {
                    score += 8;
                }
            }
        }
    }

    if let Some(name) = payload.name.as_deref() {
        let payload_name = normalize_match_key(name);
        let binding_id = normalize_match_key(&binding.id);
        if !payload_name.is_empty()
            && !binding_id.is_empty()
            && (payload_name.contains(&binding_id) || binding_id.contains(&payload_name))
        {
            score += 4;
        }
    }

    if payload
        .status
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("active"))
    {
        score += 1;
    }

    score
}

fn select_account_candidate(
    candidates: Vec<PluggyAccountPayload>,
    binding: &PluggyBindingConfig,
    registry: Option<&AccountRegistryEntry>,
) -> Result<PluggyAccountPayload> {
    let item_id = binding_item_id(binding, registry).unwrap_or("desconhecido");
    if candidates.is_empty() {
        anyhow::bail!(
            "Nenhuma conta Pluggy encontrada para itemId {item_id} (binding {})",
            binding.id
        );
    }
    if candidates.len() == 1 {
        return Ok(candidates.into_iter().next().expect("single candidate"));
    }

    let mut ranked = candidates
        .into_iter()
        .map(|payload| {
            let score = score_account_candidate(&payload, binding, registry);
            (score, payload)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.id.cmp(&right.1.id))
    });

    let top_score = ranked.first().map(|entry| entry.0).unwrap_or_default();
    let next_score = ranked.get(1).map(|entry| entry.0).unwrap_or(i32::MIN);
    if top_score <= 0 || top_score == next_score {
        let candidates = ranked
            .iter()
            .take(5)
            .map(|(_, payload)| {
                format!(
                    "{}:{}:{}",
                    payload.id,
                    payload
                        .name
                        .clone()
                        .unwrap_or_else(|| "sem-nome".to_string()),
                    account_type_from_payload(payload)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "Rebind Pluggy ambíguo para binding {} via itemId {item_id}; candidatos: {candidates}",
            binding.id
        );
    }

    Ok(ranked.remove(0).1)
}

fn build_account_record(
    payload: PluggyAccountPayload,
    binding: &PluggyBindingConfig,
    registry: Option<&AccountRegistryEntry>,
    actor_id: &str,
) -> AccountRecord {
    let updated_at = parse_datetime_or_now(payload.updated_at.as_deref());
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
        pluggy_item_id: payload
            .item_id
            .clone()
            .or_else(|| binding.pluggy_item_id.clone())
            .or_else(|| registry.and_then(|entry| entry.pluggy_item_id.clone())),
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
    rules: &[CompiledRule],
    internal_categories: &BTreeSet<String>,
) -> Result<TransactionRecord> {
    let category_name = payload
        .category
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let category_key = category_name.map(|value| category_id(value, None));
    let (category_id, category_source) = apply_rules(
        &payload.description,
        category_key,
        payload.category.is_some(),
        rules,
    );
    let transaction_date = parse_date(&payload.date)?;

    let tx_type = payload
        .tx_type
        .clone()
        .unwrap_or_else(|| {
            if payload.amount.is_sign_negative() {
                "debit".to_string()
            } else {
                "credit".to_string()
            }
        })
        .to_ascii_lowercase();

    // Pluggy returns credit-card debits with positive amounts (debt added to card),
    // sometimes without a type field. Normalize to negative so the invariant
    // "negative = expense" holds everywhere. Only keep positive amounts for
    // genuine credits (cashback, refunds).
    let is_credit_account = registry
        .map(|r| r.account_type.as_str() == "credit")
        .unwrap_or(false);
    let is_genuine_credit = category_id
        .as_deref()
        .is_some_and(|c| matches!(c, "cashback" | "refund") || internal_categories.contains(c));
    let (amount, tx_type) =
        if is_credit_account && payload.amount.is_sign_positive() && !is_genuine_credit {
            (-payload.amount, "debit".to_string())
        } else {
            (payload.amount, tx_type)
        };

    let created_at = parse_datetime_or_now(payload.created_at.as_deref());
    let updated_at = parse_datetime_or_now(payload.updated_at.as_deref());
    Ok(TransactionRecord {
        transaction_id: payload.id.clone(),
        account_id: registry
            .map(|entry| entry.account_id.clone())
            .or_else(|| Some(binding.id.clone())),
        transaction_date,
        description: payload.description.clone(),
        amount,
        tx_type,
        category_id,
        category_source,
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

async fn authenticate(client: &Client, base_url: &str) -> Result<String> {
    let client_id = std::env::var("PLUGGY_CLIENT_ID").context("PLUGGY_CLIENT_ID ausente")?;
    let client_secret =
        std::env::var("PLUGGY_CLIENT_SECRET").context("PLUGGY_CLIENT_SECRET ausente")?;
    let response = client
        .post(format!("{base_url}/auth"))
        .json(&json!({
            "clientId": client_id,
            "clientSecret": client_secret,
        }))
        .send()
        .await
        .context("Falha ao autenticar no Pluggy")?;
    if !response.status().is_success() {
        let status = response.status();
        let _body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Auth Pluggy falhou com status {status} — verifique PLUGGY_CLIENT_ID e PLUGGY_CLIENT_SECRET"
        ));
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
    base_url: &str,
) -> Result<Option<PluggyAccountPayload>> {
    let response = client
        .get(format!("{base_url}/accounts/{account_id}"))
        .header("X-API-KEY", api_key)
        .send()
        .await
        .with_context(|| format!("Falha ao consultar conta Pluggy {account_id}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
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
        .map(Some)
}

async fn fetch_accounts_by_item(
    client: &Client,
    api_key: &str,
    item_id: &str,
    base_url: &str,
) -> Result<Vec<PluggyAccountPayload>> {
    let mut page = 1;
    let mut total_pages = 1;
    let mut all = Vec::new();

    while page <= total_pages {
        let response = client
            .get(format!("{base_url}/accounts"))
            .query(&[
                ("itemId", item_id),
                ("pageSize", "500"),
                ("page", &page.to_string()),
            ])
            .header("X-API-KEY", api_key)
            .send()
            .await
            .with_context(|| format!("Falha ao consultar contas Pluggy para itemId {item_id}"))?;
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Consulta de contas Pluggy para itemId {item_id} falhou: {body}"
            ));
        }
        let body: PaginatedResponse<PluggyAccountPayload> = response
            .json()
            .await
            .context("JSON inválido ao consultar contas Pluggy por itemId")?;
        total_pages = body.total_pages.unwrap_or(1);
        all.extend(body.results);
        page += 1;
    }

    Ok(all)
}

async fn resolve_account_payload(
    client: &Client,
    api_key: &str,
    binding: &PluggyBindingConfig,
    registry: Option<&AccountRegistryEntry>,
    base_url: &str,
) -> Result<PluggyAccountPayload> {
    if let Some(account) =
        fetch_account_details(client, api_key, &binding.pluggy_account_id, base_url).await?
    {
        return Ok(account);
    }

    let item_id = resolve_binding_item_id(binding, registry)?.with_context(|| {
        format!(
            "Conta Pluggy {} retornou 404 e binding {} não possui pluggyItemId",
            binding.pluggy_account_id, binding.id
        )
    })?;
    let candidates = fetch_accounts_by_item(client, api_key, item_id, base_url).await?;
    select_account_candidate(candidates, binding, registry)
}

async fn fetch_transactions(
    client: &Client,
    api_key: &str,
    account_id: &str,
    from: &str,
    to: &str,
    base_url: &str,
) -> Result<Vec<PluggyTransactionPayload>> {
    let mut page = 1;
    let mut total_pages = 1;
    let mut all = Vec::new();

    while page <= total_pages {
        let response = client
            .get(format!("{base_url}/transactions"))
            .query(&[
                ("accountId", account_id),
                ("from", from),
                ("to", to),
                ("pageSize", "500"),
                ("page", &page.to_string()),
            ])
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
    params: SyncPluggyParams<'_>,
) -> Result<(Vec<AccountRecord>, Vec<TransactionRecord>, Vec<RebindEvent>)> {
    let SyncPluggyParams {
        actor_id,
        pluggy_config_path,
        accounts_csv_path,
        fixture_path,
        from_override,
        to_date,
        rules,
        internal_categories,
        api_base_url,
    } = params;
    let base_url = api_base_url.unwrap_or(PLUGGY_API);

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
        let fixture_accounts = fixture.accounts;
        let account_map = fixture_accounts
            .iter()
            .cloned()
            .map(|row| (row.id.clone(), row))
            .collect::<BTreeMap<_, _>>();
        let mut accounts = Vec::new();
        let mut transactions = Vec::new();
        let mut rebind_events = Vec::new();
        let mut resolved_accounts =
            BTreeMap::<String, (PluggyBindingConfig, Option<AccountRegistryEntry>)>::new();
        let mut resolved_binding_ids = BTreeMap::<String, String>::new();
        for binding in &config.accounts {
            let registry_entry = registry
                .get(&binding.id)
                .or_else(|| registry.get(&format!("pluggy:{}", binding.pluggy_account_id)));
            // Validate pluggyItemId cross-source consistency upfront.
            let resolved_item_id =
                resolve_binding_item_id(binding, registry_entry)?.map(str::to_string);
            let payload = account_map
                .get(&binding.pluggy_account_id)
                .cloned()
                .or_else(|| {
                    let item_id = resolved_item_id.as_deref()?;
                    let candidates = fixture_accounts
                        .iter()
                        .filter(|row| row.item_id.as_deref() == Some(item_id))
                        .cloned()
                        .collect::<Vec<_>>();
                    select_account_candidate(candidates, binding, registry_entry).ok()
                })
                .with_context(|| {
                    format!(
                        "Fixture não contém conta Pluggy {} nem candidatos para itemId {:?}",
                        binding.pluggy_account_id, resolved_item_id
                    )
                })?;
            // Detect binding collisions onto the same resolved pluggyAccountId.
            if let Some(prev_binding_id) =
                resolved_binding_ids.insert(payload.id.clone(), binding.id.clone())
            {
                return Err(anyhow::anyhow!(
                    "Colisão de bindings Pluggy: {} e {} resolvem para a mesma conta {}",
                    prev_binding_id,
                    binding.id,
                    payload.id
                ));
            }
            if payload.id != binding.pluggy_account_id {
                rebind_events.push(RebindEvent {
                    binding_id: binding.id.clone(),
                    internal_account_id: binding.id.clone(),
                    from_pluggy_account_id: binding.pluggy_account_id.clone(),
                    to_pluggy_account_id: payload.id.clone(),
                    pluggy_item_id: resolved_item_id.clone(),
                });
            }
            resolved_accounts.insert(
                payload.id.clone(),
                (binding.clone(), registry_entry.cloned()),
            );
            accounts.push(build_account_record(
                payload,
                binding,
                registry_entry,
                actor_id,
            ));
        }
        for payload in fixture.transactions {
            let (binding, registry_entry) = resolved_accounts
                .get(&payload.account_id)
                .with_context(|| {
                    format!(
                        "Transação de fixture sem binding para conta {}",
                        payload.account_id
                    )
                })?;
            transactions.push(build_transaction_record(
                payload,
                binding,
                registry_entry.as_ref(),
                actor_id,
                rules,
                internal_categories,
            )?);
        }
        return Ok((accounts, transactions, rebind_events));
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .context("Falha ao construir cliente HTTP do Pluggy")?;
    let api_key = authenticate(&client, base_url).await?;

    // Phase 1: resolve every binding to a Pluggy account payload upfront.
    // We do this serially before fetching transactions so we can detect
    // collisions (two bindings resolving to the same pluggyAccountId) and
    // validate cross-source itemId consistency before touching any data.
    let mut resolved: Vec<(
        PluggyBindingConfig,
        Option<AccountRegistryEntry>,
        PluggyAccountPayload,
    )> = Vec::with_capacity(config.accounts.len());
    let mut rebind_events: Vec<RebindEvent> = Vec::new();
    for binding in config.accounts.clone() {
        let registry_entry = registry
            .get(&binding.id)
            .or_else(|| registry.get(&format!("pluggy:{}", binding.pluggy_account_id)))
            .cloned();
        // Ensure config and CSV agree on pluggyItemId before any HTTP call.
        let item_id_for_audit =
            resolve_binding_item_id(&binding, registry_entry.as_ref())?.map(str::to_string);
        let payload =
            resolve_account_payload(&client, &api_key, &binding, registry_entry.as_ref(), base_url)
                .await?;
        if payload.id != binding.pluggy_account_id {
            rebind_events.push(RebindEvent {
                binding_id: binding.id.clone(),
                internal_account_id: binding.id.clone(),
                from_pluggy_account_id: binding.pluggy_account_id.clone(),
                to_pluggy_account_id: payload.id.clone(),
                pluggy_item_id: item_id_for_audit,
            });
        }
        resolved.push((binding, registry_entry, payload));
    }

    // Detect two bindings resolving onto the same pluggyAccountId.
    let mut seen = BTreeMap::<String, String>::new();
    for (binding, _, payload) in &resolved {
        if let Some(prev_binding_id) = seen.insert(payload.id.clone(), binding.id.clone()) {
            return Err(anyhow::anyhow!(
                "Colisão de bindings Pluggy: {} e {} resolvem para a mesma conta {}",
                prev_binding_id,
                binding.id,
                payload.id
            ));
        }
    }

    // Phase 2: fetch transactions in parallel once bindings are validated.
    let mut accounts = Vec::new();
    let mut transactions = Vec::new();
    let mut tasks = JoinSet::new();
    for (binding, registry_entry, payload) in resolved {
        let client = client.clone();
        let api_key = api_key.clone();
        let from = from.clone();
        let to = to_date.to_string();
        let actor_id = actor_id.to_string();
        let rules = rules.to_vec();
        let internal_categories = internal_categories.clone();
        let base_url = base_url.to_string();
        tasks.spawn(async move {
            let resolved_account_id = payload.id.clone();
            let account_record =
                build_account_record(payload, &binding, registry_entry.as_ref(), &actor_id);
            let account_transactions =
                fetch_transactions(&client, &api_key, &resolved_account_id, &from, &to, &base_url)
                    .await?;
            let transaction_records = account_transactions
                .into_iter()
                .map(|payload| {
                    build_transaction_record(
                        payload,
                        &binding,
                        registry_entry.as_ref(),
                        &actor_id,
                        &rules,
                        &internal_categories,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            Ok::<_, anyhow::Error>((account_record, transaction_records))
        });
    }

    while let Some(result) = tasks.join_next().await {
        let (account, mut account_transactions) =
            result.context("Task Pluggy falhou ao sincronizar conta")??;
        accounts.push(account);
        transactions.append(&mut account_transactions);
    }

    Ok((accounts, transactions, rebind_events))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding(id: &str, acc: &str, item: Option<&str>) -> PluggyBindingConfig {
        PluggyBindingConfig {
            id: id.to_string(),
            pluggy_account_id: acc.to_string(),
            pluggy_item_id: item.map(str::to_string),
        }
    }

    fn registry_entry(item: Option<&str>) -> AccountRegistryEntry {
        AccountRegistryEntry {
            account_id: "acc".to_string(),
            owner: "o".to_string(),
            account_type: "checking".to_string(),
            bank: "b".to_string(),
            label: "l".to_string(),
            pluggy_account_id: None,
            pluggy_item_id: item.map(str::to_string),
            metadata_json: json!({}),
        }
    }

    #[test]
    fn resolve_binding_item_id_accepts_matching_sources() {
        let b = binding("a", "pa", Some("item-1"));
        let r = registry_entry(Some("item-1"));
        assert_eq!(
            resolve_binding_item_id(&b, Some(&r)).unwrap(),
            Some("item-1")
        );
    }

    #[test]
    fn resolve_binding_item_id_prefers_config_only() {
        let b = binding("a", "pa", Some("item-1"));
        let r = registry_entry(None);
        assert_eq!(
            resolve_binding_item_id(&b, Some(&r)).unwrap(),
            Some("item-1")
        );
    }

    #[test]
    fn resolve_binding_item_id_accepts_registry_only() {
        let b = binding("a", "pa", None);
        let r = registry_entry(Some("item-9"));
        assert_eq!(
            resolve_binding_item_id(&b, Some(&r)).unwrap(),
            Some("item-9")
        );
    }

    #[test]
    fn resolve_binding_item_id_rejects_divergent_sources() {
        let b = binding("a", "pa", Some("item-1"));
        let r = registry_entry(Some("item-2"));
        let err = resolve_binding_item_id(&b, Some(&r)).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("diverge"), "expected divergence error, got: {msg}");
    }

    #[test]
    fn resolve_binding_item_id_returns_none_when_absent() {
        let b = binding("a", "pa", None);
        let r = registry_entry(None);
        assert_eq!(resolve_binding_item_id(&b, Some(&r)).unwrap(), None);
        assert_eq!(resolve_binding_item_id(&b, None).unwrap(), None);
    }

    #[tokio::test]
    async fn http_rebind_via_item_id_on_404() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // Safety: test-only env vars; this test is not parallel-hostile
        // because the values are only consumed inside authenticate().
        unsafe {
            std::env::set_var("PLUGGY_CLIENT_ID", "test-id");
            std::env::set_var("PLUGGY_CLIENT_SECRET", "test-secret");
        }

        Mock::given(method("POST"))
            .and(path("/auth"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"apiKey": "test-key"})),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/accounts/old-acct"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/accounts"))
            .and(query_param("itemId", "item-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [{
                    "id": "new-acct",
                    "itemId": "item-1",
                    "name": "Primary Checking",
                    "type": "checking",
                    "status": "ACTIVE"
                }],
                "totalPages": 1
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/transactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [{
                    "id": "tx-001",
                    "accountId": "new-acct",
                    "date": "2026-04-01",
                    "description": "Test transaction",
                    "amount": -100.00,
                    "status": "POSTED",
                    "createdAt": "2026-04-01T12:00:00.000Z",
                    "updatedAt": "2026-04-01T12:00:00.000Z"
                }],
                "totalPages": 1
            })))
            .mount(&server)
            .await;

        let temp = tempfile::TempDir::new().unwrap();
        let config_path = temp.path().join("pluggy-config.json");
        std::fs::write(
            &config_path,
            serde_json::to_string(&json!({
                "syncStartDate": "2026-03-01",
                "accounts": [{
                    "id": "primary_checking",
                    "pluggyAccountId": "old-acct",
                    "pluggyItemId": "item-1"
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let params = SyncPluggyParams {
            actor_id: "test-actor",
            pluggy_config_path: &config_path,
            accounts_csv_path: None,
            fixture_path: None,
            from_override: Some("2026-03-01"),
            to_date: "2026-04-15",
            rules: &[],
            internal_categories: &BTreeSet::new(),
            api_base_url: Some(&server.uri()),
        };

        let (accounts, transactions, rebinds) = sync_pluggy(params).await.unwrap();

        assert_eq!(accounts.len(), 1);
        assert_eq!(
            accounts[0].pluggy_account_id,
            Some("new-acct".to_string())
        );
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].transaction_id, "tx-001");
        assert_eq!(rebinds.len(), 1);
        assert_eq!(rebinds[0].from_pluggy_account_id, "old-acct");
        assert_eq!(rebinds[0].to_pluggy_account_id, "new-acct");
    }
}
