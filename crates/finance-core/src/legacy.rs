use crate::idempotency::{
    account_idempotency, category_id, ensure_forecast_idempotency, pluggy_transaction_idempotency,
    rule_idempotency,
};
use crate::models::{
    decimal_from_str, parse_datetime_or_now, AccountRecord, CategoryRecord, ForecastRecord,
    RuleRecord, TransactionRecord,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct LegacyImportBundle {
    pub accounts: Vec<AccountRecord>,
    pub transactions: Vec<TransactionRecord>,
    pub rules: Vec<RuleRecord>,
    pub categories: Vec<CategoryRecord>,
    pub forecasts: Vec<ForecastRecord>,
}

#[derive(Debug, Clone)]
pub struct AccountRegistryEntry {
    pub account_id: String,
    pub owner: String,
    pub account_type: String,
    pub bank: String,
    pub label: String,
    pub pluggy_account_id: Option<String>,
    pub pluggy_item_id: Option<String>,
    pub metadata_json: Value,
}

fn read_csv_rows(path: &Path) -> Result<Vec<BTreeMap<String, String>>> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .with_context(|| format!("Falha ao abrir {}", path.display()))?;
    let headers = reader
        .headers()
        .with_context(|| format!("Falha ao ler cabeçalho de {}", path.display()))?
        .clone();
    let mut rows = Vec::new();
    for record in reader.records() {
        let record =
            record.with_context(|| format!("Falha ao ler registro CSV em {}", path.display()))?;
        let mut row = BTreeMap::new();
        for (index, header) in headers.iter().enumerate() {
            row.insert(
                header.to_string(),
                record.get(index).unwrap_or_default().to_string(),
            );
        }
        rows.push(row);
    }
    Ok(rows)
}

fn parse_month_date(value: Option<&str>, day: Option<&str>) -> Option<NaiveDate> {
    let month = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|raw| NaiveDate::parse_from_str(&format!("{raw}-01"), "%Y-%m-%d").ok())?;
    let day = day
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|raw| raw.parse::<u32>().ok())
        .unwrap_or(1);
    NaiveDate::from_ymd_opt(month.year(), month.month(), day)
        .or_else(|| NaiveDate::from_ymd_opt(month.year(), month.month(), 1))
}

fn categories_from_pair(
    actor_id: &str,
    updated_at: DateTime<Utc>,
    seen: &mut BTreeMap<String, CategoryRecord>,
    category: Option<&str>,
    subcategory: Option<&str>,
    metadata_json: Value,
) {
    let category = category.map(str::trim).filter(|value| !value.is_empty());
    if let Some(category_name) = category {
        let parent_id = category_id(category_name, None);
        seen.entry(parent_id.clone())
            .or_insert_with(|| CategoryRecord {
                category_id: parent_id.clone(),
                name: category_name.to_string(),
                parent_category_id: None,
                metadata_json: json!({"source": "legacy"}),
                actor_id: actor_id.to_string(),
                updated_at,
            });
        if let Some(subcategory_name) = subcategory.map(str::trim).filter(|value| !value.is_empty())
        {
            let child_id = category_id(category_name, Some(subcategory_name));
            seen.entry(child_id.clone())
                .or_insert_with(|| CategoryRecord {
                    category_id: child_id,
                    name: subcategory_name.to_string(),
                    parent_category_id: Some(parent_id),
                    metadata_json,
                    actor_id: actor_id.to_string(),
                    updated_at,
                });
        }
    }
}

pub fn load_account_registry(
    accounts_csv: &Path,
) -> Result<BTreeMap<String, AccountRegistryEntry>> {
    let mut registry = BTreeMap::new();
    if !accounts_csv.exists() {
        return Ok(registry);
    }
    for row in read_csv_rows(accounts_csv)? {
        let account_id = row
            .get("id")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        // Skip blank-id rows. Previously the importer would create a row
        // with `account_id=""` from a malformed line in the CSV, polluting
        // every `get_accounts()` call. The bug bled an empty-string account
        // into the production database; migration 026/027 cleans the
        // existing artefact.
        if account_id.is_empty() {
            continue;
        }
        let entry = AccountRegistryEntry {
            account_id,
            owner: row.get("owner").cloned().unwrap_or_default(),
            account_type: row.get("type").cloned().unwrap_or_default(),
            bank: row.get("bank").cloned().unwrap_or_default(),
            label: row.get("label").cloned().unwrap_or_default(),
            pluggy_account_id: row
                .get("pluggy_account_id")
                .cloned()
                .filter(|value| !value.trim().is_empty()),
            pluggy_item_id: row
                .get("pluggy_item_id")
                .cloned()
                .filter(|value| !value.trim().is_empty()),
            metadata_json: json!({
                "billing_closing_day": row.get("billing_closing_day").cloned().unwrap_or_default(),
                "billing_due_day": row.get("billing_due_day").cloned().unwrap_or_default(),
                "source": "legacy_accounts_csv",
            }),
        };
        registry.insert(entry.account_id.clone(), entry.clone());
        if let Some(pluggy_account_id) = &entry.pluggy_account_id {
            registry.insert(format!("pluggy:{pluggy_account_id}"), entry.clone());
        }
    }
    Ok(registry)
}

pub fn load_legacy_bundle(finance_root: &Path, actor_id: &str) -> Result<LegacyImportBundle> {
    let now = Utc::now();
    let accounts_csv = finance_root.join("data/contas.csv");
    let context_csv = finance_root.join("contexto_transacoes.csv");
    let forecast_csv = finance_root.join("data/forecast_templates.csv");
    let rules_yaml = finance_root.join("rules.yaml");
    let data_root = finance_root.join("data");

    let registry = load_account_registry(&accounts_csv)?;
    let mut bundle = LegacyImportBundle::default();
    let mut categories = BTreeMap::<String, CategoryRecord>::new();
    let mut account_ids = BTreeSet::new();

    for entry in registry.values() {
        if !account_ids.insert(entry.account_id.clone()) {
            continue;
        }
        bundle.accounts.push(AccountRecord {
            account_id: entry.account_id.clone(),
            owner: entry.owner.clone(),
            account_type: entry.account_type.clone(),
            bank: entry.bank.clone(),
            label: entry.label.clone(),
            pluggy_account_id: entry.pluggy_account_id.clone(),
            pluggy_item_id: entry.pluggy_item_id.clone(),
            status: "active".to_string(),
            actor_id: actor_id.to_string(),
            idempotency_key: account_idempotency(&entry.account_id),
            metadata_json: entry.metadata_json.clone(),
            created_at: now,
            updated_at: now,
        });
    }

    if data_root.exists() {
        let mut yearly_files = Vec::<PathBuf>::new();
        for entry in fs::read_dir(&data_root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let candidate = path.join("transacoes.csv");
                if candidate.exists() {
                    yearly_files.push(candidate);
                }
            }
        }
        yearly_files.sort();

        for file in yearly_files {
            for row in read_csv_rows(&file)? {
                let transaction_id = row
                    .get("pluggy_id")
                    .cloned()
                    .filter(|value| !value.trim().is_empty())
                    .context("Transação legacy sem pluggy_id")?;
                let transaction_date = row
                    .get("data")
                    .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                    .context("Transação legacy sem data válida")?;
                let amount =
                    decimal_from_str(row.get("valor").map(String::as_str).unwrap_or_default())?;
                let created_at = parse_datetime_or_now(
                    row.get("pluggy_created_at")
                        .map(String::as_str)
                        .or_else(|| row.get("data_hora_iso").map(String::as_str)),
                );
                let updated_at = parse_datetime_or_now(
                    row.get("pluggy_updated_at")
                        .map(String::as_str)
                        .or_else(|| row.get("data_hora_iso").map(String::as_str)),
                );
                let category = row
                    .get("categoria")
                    .map(String::as_str)
                    .filter(|value| !value.trim().is_empty());
                let subcategory = row
                    .get("subcategoria")
                    .map(String::as_str)
                    .filter(|value| !value.trim().is_empty());
                categories_from_pair(
                    actor_id,
                    updated_at,
                    &mut categories,
                    category,
                    subcategory,
                    json!({"source": "legacy_transactions"}),
                );
                let tx_type = row
                    .get("tipo")
                    .map(|value| value.to_ascii_lowercase())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| {
                        if amount.is_sign_negative() {
                            "debit".to_string()
                        } else {
                            "credit".to_string()
                        }
                    });
                bundle.transactions.push(TransactionRecord {
                    transaction_id: transaction_id.clone(),
                    account_id: row
                        .get("conta_id")
                        .cloned()
                        .filter(|value| !value.trim().is_empty()),
                    transaction_date,
                    description: row.get("descricao").cloned().unwrap_or_default(),
                    amount,
                    tx_type,
                    category_id: category.map(|value| category_id(value, subcategory)),
                    category_source: row
                        .get("classificacao_fonte")
                        .cloned()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| "legacy".to_string()),
                    context: row
                        .get("contexto_finalidade")
                        .cloned()
                        .filter(|value| !value.trim().is_empty()),
                    payment_status: row
                        .get("status_pagamento")
                        .cloned()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| "unknown".to_string()),
                    source: row
                        .get("pluggy_account_id")
                        .filter(|value| !value.trim().is_empty())
                        .map(|_| "pluggy".to_string())
                        .unwrap_or_else(|| "legacy".to_string()),
                    actor_id: actor_id.to_string(),
                    idempotency_key: pluggy_transaction_idempotency(&transaction_id),
                    metadata_json: json!({
                        "legacy_mes_ref": row.get("mes_ref").cloned().unwrap_or_default(),
                        "legacy_fatura_mes_ref": row.get("fatura_mes_ref").cloned().unwrap_or_default(),
                        "legacy_descricao_raw": row.get("descricao_raw").cloned().unwrap_or_default(),
                        "legacy_classificacao_regra": row.get("classificacao_regra").cloned().unwrap_or_default(),
                        "legacy_conta_owner": row.get("conta_owner").cloned().unwrap_or_default(),
                        "raw_transaction_json": row.get("raw_transaction_json").cloned().unwrap_or_default(),
                    }),
                    created_at,
                    updated_at,
                    enrichment_attempted_at: None,
                });
            }
        }
    }

    if rules_yaml.exists() {
        let raw = fs::read_to_string(&rules_yaml)
            .with_context(|| format!("Falha ao ler {}", rules_yaml.display()))?;
        let doc: Value = serde_yaml::from_str(&raw).context("Falha ao parsear rules.yaml")?;
        if let Some(items) = doc.get("categories").and_then(Value::as_array) {
            for item in items {
                let rule_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("legacy_rule");
                let category = item
                    .get("set")
                    .and_then(|value| value.get("category"))
                    .and_then(Value::as_str);
                let subcategory = item
                    .get("set")
                    .and_then(|value| value.get("subcategory"))
                    .and_then(Value::as_str);
                categories_from_pair(
                    actor_id,
                    now,
                    &mut categories,
                    category,
                    subcategory,
                    json!({"source": "legacy_rules"}),
                );
                bundle.rules.push(RuleRecord {
                    rule_id: rule_id.to_string(),
                    body: serde_json::to_string(item)?,
                    status: "active".to_string(),
                    actor_id: actor_id.to_string(),
                    idempotency_key: rule_idempotency(rule_id),
                    created_at: now,
                    updated_at: now,
                });
            }
        }
    }

    if context_csv.exists() {
        for row in read_csv_rows(&context_csv)? {
            let match_type = row.get("match_type").cloned().unwrap_or_default();
            let match_value = row.get("match_value").cloned().unwrap_or_default();
            let valor_match = row.get("valor_match").cloned().unwrap_or_default();
            let rule_id = format!(
                "context:{}:{}:{}",
                match_type,
                match_value
                    .chars()
                    .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
                    .collect::<String>()
                    .trim_matches('-'),
                valor_match
            )
            .trim_end_matches(':')
            .to_string();
            categories_from_pair(
                actor_id,
                now,
                &mut categories,
                row.get("categoria").map(String::as_str),
                row.get("subcategoria").map(String::as_str),
                json!({"source": "legacy_context_csv"}),
            );
            bundle.rules.push(RuleRecord {
                rule_id: rule_id.clone(),
                body: serde_json::to_string(&row)?,
                status: if row.get("ativo").map(String::as_str) == Some("0") {
                    "inactive".to_string()
                } else {
                    "active".to_string()
                },
                actor_id: actor_id.to_string(),
                idempotency_key: rule_idempotency(&rule_id),
                created_at: now,
                updated_at: now,
            });
        }
    }

    if forecast_csv.exists() {
        for row in read_csv_rows(&forecast_csv)? {
            let category = row.get("categoria").map(String::as_str);
            let subcategory = row.get("subcategoria").map(String::as_str);
            categories_from_pair(
                actor_id,
                now,
                &mut categories,
                category,
                subcategory,
                json!({"source": "legacy_forecast"}),
            );
            let mut forecast = ForecastRecord {
                forecast_id: row.get("id").cloned().unwrap_or_default(),
                due_date: parse_month_date(
                    row.get("inicio_mes").map(String::as_str),
                    row.get("dia_vencimento").map(String::as_str),
                ),
                description: row.get("descricao").cloned().unwrap_or_default(),
                amount: decimal_from_str(row.get("valor").map(String::as_str).unwrap_or_default())?,
                category_id: category.map(|value| category_id(value, subcategory)),
                account_id: row
                    .get("conta_id")
                    .cloned()
                    .filter(|value| !value.trim().is_empty()),
                status: row
                    .get("status")
                    .cloned()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "active".to_string()),
                recurrence: row
                    .get("frequencia")
                    .cloned()
                    .filter(|value| !value.trim().is_empty()),
                actor_id: actor_id.to_string(),
                idempotency_key: String::new(),
                metadata_json: json!({
                    "tipo": row.get("tipo").cloned().unwrap_or_default(),
                    "dia_vencimento": row.get("dia_vencimento").cloned().unwrap_or_default(),
                    "inicio_mes": row.get("inicio_mes").cloned().unwrap_or_default(),
                    "fim_mes": row.get("fim_mes").cloned().unwrap_or_default(),
                    "parcelas_total": row.get("parcelas_total").cloned().unwrap_or_default(),
                    "match_contains": row.get("match_contains").cloned().unwrap_or_default(),
                    "impacta_total": row.get("impacta_total").cloned().unwrap_or_default(),
                    "origem": row.get("origem").cloned().unwrap_or_default(),
                    "notas": row.get("notas").cloned().unwrap_or_default(),
                }),
                created_at: now,
                updated_at: now,
            };
            ensure_forecast_idempotency(&mut forecast)?;
            bundle.forecasts.push(forecast);
        }
    }

    bundle.categories = categories.into_values().collect();
    Ok(bundle)
}
