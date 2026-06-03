//! `fin serve` — local web bridge for the LiveStore React dashboard.
//!
//! Starts an HTTP server that hosts the embedded LiveStore SPA and a plain
//! JSON REST API under `/api`. The frontend is client-only (no LiveStore sync
//! backend); reads come from this bridge and user writes flush back here, where
//! they are applied to the `FinanceStore` with an audit trail.
//!
//! Because `Box<dyn FinanceStore>` is `!Send`, a channel-based store actor runs
//! inside `LocalSet` while the axum router and handlers live in the `Send`
//! world.

use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{Datelike, NaiveDate, Utc};
use phai_core::idempotency::ensure_forecast_idempotency;
use phai_core::migrations::run_migrations;
use phai_core::models::{
    AccountRecord, AuditEvent, CardSummaryRow, ForecastRecord, ForecastTemplateRecord,
    TransactionRecord,
};
use phai_core::storage::{open_store, FinanceStore};
use phai_core::{parse_installment_description, AppConfig, BackendKind};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::LocalSet;
use uuid::Uuid;

const STORE_CHANNEL_CAP: usize = 256;
const LOCAL_BIND_HOST: &str = "127.0.0.1";
const LOCAL_APP_HOST: &str = "phai.localhost";
/// Default number of review-queue rows returned when the caller omits `limit`.
const DEFAULT_REVIEW_QUEUE_LIMIT: usize = 200;
/// Actor id stamped on writes that originate from the web bridge.
const SERVE_ACTOR_ID: &str = "serve-dashboard";
/// Sentinel category id for rows routed to the human-review queue. A row with
/// this category (or no category) is considered unreviewed.
const REVIEW_PENDING_CATEGORY: &str = "_revisar";
/// Category-id prefix that marks a subscription charge. Heuristic: any
/// transaction whose category begins with `assinaturas:` is a subscription.
const SUBSCRIPTION_CATEGORY_PREFIX: &str = "assinaturas:";
/// `payment_status` value for instalment-plan transactions.
const PAYMENT_STATUS_INSTALLMENT: &str = "installment";
/// Default planning window: months of history included in `GET /api/transactions`.
const DEFAULT_TRANSACTIONS_MONTHS_BACK: u32 = 12;
/// Hard cap on rows returned by `GET /api/transactions`.
const DEFAULT_TRANSACTIONS_LIMIT: usize = 5000;

use crate::cashflow_chart::{build_chart_data, ChartData};
use crate::forecast_cmd::materialise_template_forecasts;
use crate::{
    all_transactions_for_review, apply_human_review, load_config, parse_month_ref,
    review_human_rows, HumanReviewPatch, ReviewFilters, ReviewHumanKind,
};

const LEGACY_WEB_CATEGORY_ALIASES: &[(&str, &str)] = &[
    ("airport-and-airlines", "lazer:viagem"),
    ("airlines", "lazer:viagem"),
    ("accomodation", "lazer:viagem"),
    ("automotive", "transporte"),
    ("bank", "financeiro"),
    ("bank-fees", "financeiro:taxas"),
    ("cashback", "receitas:cashback"),
    ("digital-services", "compras:servicos-digitais"),
    ("donations", "outros"),
    ("electricity", "moradia:energia"),
    ("electronics", "compras:eletronicos"),
    ("gambling", "lazer:apostas"),
    ("gaming", "lazer"),
    ("gas", "moradia:gas"),
    ("gas-stations", "transporte:combustivel"),
    ("groceries", "alimentacao:mercado"),
    ("supermarket", "alimentacao:mercado"),
    ("eating-out", "alimentacao:restaurantes"),
    ("restaurants", "alimentacao:restaurantes"),
    ("food-and-drinks", "alimentacao:restaurantes"),
    ("food-and-beverages", "alimentacao:restaurantes"),
    ("food-delivery", "alimentacao:delivery"),
    ("gyms-and-fitness-centers", "saude:atividade-fisica"),
    ("houseware", "compras:casa"),
    ("housing", "moradia"),
    ("insurance", "financeiro:seguros"),
    ("kids-and-toys", "lazer:criancas"),
    ("clothing", "compras:vestuario"),
    ("fashion", "compras:vestuario"),
    ("apparel", "compras:vestuario"),
    ("shopping", "compras"),
    ("retail", "compras"),
    ("e-commerce", "compras"),
    ("online-shopping", "compras:e-commerce"),
    ("office-supplies", "compras"),
    ("parking", "transporte:estacionamento"),
    ("sports-goods", "lazer:esportes"),
    ("hospital-clinics-and-labs", "saude:consulta"),
    ("optometry", "saude:saude-visual"),
    ("pharmacy", "saude:farmacia"),
    ("health", "saude"),
    ("healthcare", "saude"),
    ("school", "educacao"),
    ("online-courses", "educacao:cursos"),
    ("transport", "transporte"),
    ("transportation", "transporte"),
    ("ride-sharing", "transporte:aplicativo"),
    ("taxi-and-ride-hailing", "transporte:aplicativo"),
    ("public-transportation", "transporte:publico"),
    ("tolls-and-in-vehicle-payment", "transporte:pedagio"),
    ("vehicle-maintenance", "transporte:manutencao"),
    ("education", "educacao"),
    ("bookstore", "educacao"),
    ("entertainment", "lazer"),
    ("leisure", "lazer"),
    ("sports-and-fitness", "saude:fitness"),
    ("travel", "lazer:viagem"),
    ("bills-and-utilities", "moradia:contas"),
    ("utilities", "moradia:contas"),
    ("telecommunications", "moradia:contas"),
    ("rent", "moradia:aluguel"),
    ("personal-care", "pessoal:cuidado-fisico"),
    ("personal", "pessoal"),
    ("subscriptions", "assinaturas"),
    ("video-streaming", "assinaturas:streaming"),
    ("same-person-transfer", "transfer-internal"),
    ("transfers", "transfer-internal"),
    ("transfer-pix", "transfer-internal"),
    ("tax-on-financial-operations", "financeiro:iof"),
    ("income", "renda"),
    ("salary", "renda"),
    ("pets", "lazer:pets"),
];

const WEB_CANONICAL_CATEGORY_FAMILIES: &[&str] = &[
    "alimentacao",
    "assinaturas",
    "compras",
    "educacao",
    "financeiro",
    "lazer",
    "moradia",
    "outros",
    "pessoal",
    "receitas",
    "renda",
    "saude",
    "servicos",
    "transferencias",
    "transporte",
];

fn is_web_canonical_category(category_id: &str) -> bool {
    if category_id == REVIEW_PENDING_CATEGORY {
        return true;
    }
    let family = category_id
        .split_once(':')
        .map_or(category_id, |(family, _)| family);
    WEB_CANONICAL_CATEGORY_FAMILIES.contains(&family)
}

fn canonical_web_category_id(category_id: &str) -> String {
    let raw = category_id.trim();
    if raw.is_empty() {
        return String::new();
    }
    let normalized = raw.to_lowercase().replace([' ', '_'], "-");
    if let Some(canonical) = LEGACY_WEB_CATEGORY_ALIASES
        .iter()
        .find_map(|(alias, canonical)| (*alias == normalized).then_some(*canonical))
    {
        return canonical.to_string();
    }
    if is_web_canonical_category(&normalized) {
        return normalized;
    }
    "outros".to_string()
}

fn canonical_web_category(category_id: Option<&str>) -> Option<String> {
    category_id
        .map(canonical_web_category_id)
        .filter(|id| !id.trim().is_empty())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeIdentityResponse {
    identity: String,
    backend: String,
}

fn bridge_identity_response(config: &AppConfig) -> BridgeIdentityResponse {
    let backend = config.effective_backend();
    let (backend_label, material) = match backend {
        BackendKind::Bigquery => (
            "bigquery",
            format!(
                "bigquery:{}:{}",
                config.project_id.as_deref().unwrap_or(""),
                config.dataset_id.as_deref().unwrap_or("")
            ),
        ),
        BackendKind::Local => (
            "local",
            format!(
                "local:{}",
                config
                    .local_db_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            ),
        ),
    };
    let digest = Sha256::digest(material.as_bytes());
    BridgeIdentityResponse {
        identity: format!("{backend_label}:{digest:x}"),
        backend: backend_label.to_string(),
    }
}

async fn list_web_category_ids(store: &dyn FinanceStore) -> Result<Vec<String>> {
    let internal_categories = store.internal_categories().await?;
    let ids = store.list_all_category_ids().await?;
    let mut out = BTreeSet::new();
    for id in ids {
        if id == REVIEW_PENDING_CATEGORY || internal_categories.contains(&id) {
            continue;
        }
        let canonical = canonical_web_category_id(&id);
        if canonical.is_empty() || internal_categories.contains(&canonical) {
            continue;
        }
        out.insert(canonical);
    }
    Ok(out.into_iter().collect())
}

// ── Store actor ──────────────────────────────────────────────────────────

/// A single human-review write to apply, carrying the caller's `write_id` so
/// the response can ack/fail each write independently.
struct ReviewWrite {
    write_id: String,
    transaction_id: String,
    patch: HumanReviewPatch,
}

/// Outcome of one human-review write inside a batch.
enum ReviewWriteOutcome {
    Acked(String),
    Failed { write_id: String, error: String },
}

enum StoreRequest {
    GetChartData {
        months_back: usize,
        months_ahead: usize,
        resp: oneshot::Sender<Result<ChartData>>,
    },
    ListForecastTemplates {
        kind: Option<String>,
        status: Option<String>,
        resp: oneshot::Sender<Result<Vec<ForecastTemplateRecord>>>,
    },
    AcceptTemplate {
        template_id: String,
        materialize_months: u32,
        resp: oneshot::Sender<Result<Value>>,
    },
    DismissTemplate {
        template_id: String,
        resp: oneshot::Sender<Result<()>>,
    },
    UpsertForecast {
        record: Box<ForecastRecord>,
        resp: oneshot::Sender<Result<String>>,
    },
    ListCategoryIds {
        resp: oneshot::Sender<Result<Vec<String>>>,
    },
    GetAccounts {
        resp: oneshot::Sender<Result<Vec<AccountRecord>>>,
    },
    /// Per-credit-card cycle state (open/paid), total and credit-limit usage,
    /// for the dashboard card panel (`GET /api/cards`).
    Cards {
        month: Option<String>,
        resp: oneshot::Sender<Result<Vec<CardApiRow>>>,
    },
    ReviewQueue {
        params: ReviewQueueParams,
        resp: oneshot::Sender<Result<Vec<TransactionRecord>>>,
    },
    /// Transactions whose cash month falls in the requested window, used by
    /// the planning workspace (`GET /api/transactions`).
    TransactionsWindow {
        params: TransactionsWindowParams,
        resp: oneshot::Sender<Result<TransactionsWindowResult>>,
    },
    /// Forecasts enriched with the linked template `kind` so each row can carry
    /// the derived `kind`/`draggable` fields (`GET /api/forecasts`).
    ListForecastsEnriched {
        status: Option<String>,
        from: Option<NaiveDate>,
        until: Option<NaiveDate>,
        resp: oneshot::Sender<Result<Vec<ForecastWithKind>>>,
    },
    /// Reschedule a movable forecast to a new due date (`POST /api/forecast/move`).
    MoveForecast {
        forecast_id: String,
        due_date: NaiveDate,
        resp: oneshot::Sender<Result<MoveForecastResult>>,
    },
    ApplyHumanReview {
        writes: Vec<ReviewWrite>,
        resp: oneshot::Sender<Vec<ReviewWriteOutcome>>,
    },
}

async fn store_actor_loop(
    store: Box<dyn FinanceStore>,
    config: AppConfig,
    mut rx: mpsc::Receiver<StoreRequest>,
) {
    while let Some(req) = rx.recv().await {
        match req {
            StoreRequest::GetChartData {
                months_back,
                months_ahead,
                resp,
            } => {
                let result =
                    build_chart_data(store.as_ref(), months_back, months_ahead, true).await;
                let _ = resp.send(result);
            }
            StoreRequest::ListForecastTemplates { kind, status, resp } => {
                let result = store
                    .list_forecast_templates(kind.as_deref(), status.as_deref())
                    .await;
                let _ = resp.send(result);
            }
            StoreRequest::AcceptTemplate {
                template_id,
                materialize_months,
                resp,
            } => {
                let result =
                    handle_accept_template(store.as_ref(), &template_id, materialize_months).await;
                let _ = resp.send(result);
            }
            StoreRequest::DismissTemplate { template_id, resp } => {
                let result = handle_dismiss_template(store.as_ref(), &template_id).await;
                let _ = resp.send(result);
            }
            StoreRequest::UpsertForecast { record, resp } => {
                let result = upsert_forecast(store.as_ref(), *record).await;
                let _ = resp.send(result);
            }
            StoreRequest::ListCategoryIds { resp } => {
                let result = list_web_category_ids(store.as_ref()).await;
                let _ = resp.send(result);
            }
            StoreRequest::GetAccounts { resp } => {
                let result = store.get_accounts().await;
                let _ = resp.send(result);
            }
            StoreRequest::Cards { month, resp } => {
                let result = build_cards_api(store.as_ref(), month.as_deref()).await;
                let _ = resp.send(result);
            }
            StoreRequest::ReviewQueue { params, resp } => {
                let result = load_review_queue(store.as_ref(), params).await;
                let _ = resp.send(result);
            }
            StoreRequest::TransactionsWindow { params, resp } => {
                let result = load_transactions_window(store.as_ref(), params).await;
                let _ = resp.send(result);
            }
            StoreRequest::ListForecastsEnriched {
                status,
                from,
                until,
                resp,
            } => {
                let result =
                    list_forecasts_enriched(store.as_ref(), status.as_deref(), from, until).await;
                let _ = resp.send(result);
            }
            StoreRequest::MoveForecast {
                forecast_id,
                due_date,
                resp,
            } => {
                let result = move_forecast(store.as_ref(), &forecast_id, due_date).await;
                let _ = resp.send(result);
            }
            StoreRequest::ApplyHumanReview { writes, resp } => {
                let outcomes = apply_review_writes(store.as_ref(), &config, writes).await;
                let _ = resp.send(outcomes);
            }
        }
    }
}

/// Resolved review-queue request, ready to run against the store.
struct ReviewQueueParams {
    filters: ReviewFilters,
    include_reviewed: bool,
    limit: usize,
}

async fn load_review_queue(
    store: &dyn FinanceStore,
    params: ReviewQueueParams,
) -> Result<Vec<TransactionRecord>> {
    // Resolve --owner to its account set, mirroring `tx_review_human`.
    let mut filters = params.filters;
    if let Some(owner_name) = filters.owner.clone() {
        let accounts = store.get_accounts().await?;
        let owned: BTreeSet<String> = accounts
            .into_iter()
            .filter(|a| a.owner == owner_name)
            .map(|a| a.account_id)
            .collect();
        if owned.is_empty() {
            anyhow::bail!("owner '{owner_name}' não bate com nenhuma conta");
        }
        filters.owner_accounts = Some(owned);
    }
    // The API does not gate the queue by amount; keep every pending row.
    let min_abs_amount = Decimal::ZERO;
    if params.include_reviewed {
        all_transactions_for_review(store, params.limit, min_abs_amount, &filters).await
    } else {
        review_human_rows(
            store,
            ReviewHumanKind::All,
            params.limit,
            min_abs_amount,
            &filters,
        )
        .await
    }
}

/// Resolved planning-window request for `GET /api/transactions`.
struct TransactionsWindowParams {
    /// How many whole months back from the current month to include.
    months_back: u32,
    /// How many whole months ahead of the current month to include.
    months_ahead: u32,
    /// When `false`, only rows still pending human review are returned.
    include_reviewed: bool,
    /// Page size (rows per page).
    limit: usize,
    /// Row offset for pagination (0-based).
    offset: usize,
}

/// Result of a paginated transaction-window load.
struct TransactionsWindowResult {
    rows: Vec<TxRow>,
    total: usize,
    offset: usize,
    has_more: bool,
}

/// Load transactions whose cash month is within
/// `[now - months_back, now + months_ahead]`, optionally restricted to rows
/// still pending review. Returns a page of up to `limit` rows starting at
/// `offset`, plus the total matching count so the client can paginate.
async fn load_transactions_window(
    store: &dyn FinanceStore,
    params: TransactionsWindowParams,
) -> Result<TransactionsWindowResult> {
    let today = Utc::now().date_naive();
    let cash_from = first_of_month(shift_months(today, -(params.months_back as i64)));
    let cash_until = last_of_month(shift_months(today, params.months_ahead as i64));
    let cash_from_ref = cash_from.format("%Y-%m").to_string();
    let cash_until_ref = cash_until.format("%Y-%m").to_string();
    let posted_from = first_of_month(shift_months(cash_from, -2));
    let mut rows = store
        .effective_transactions_window(None, posted_from, cash_until)
        .await
        .context("effective_transactions_window")?;
    let internal_categories = store
        .internal_categories()
        .await
        .context("internal_categories")?;
    rows.retain(|row| {
        let raw_category = row.category_id.as_deref();
        let canonical_category = canonical_web_category(raw_category);
        !raw_category.is_some_and(|category| internal_categories.contains(category))
            && !canonical_category
                .as_deref()
                .is_some_and(|category| internal_categories.contains(category))
    });
    // ofx-shadowed-by-pluggy rows are already dropped by v_transactions_reportable
    // (migration 038 / ADR-0026); no Rust-side dedup needed here.
    if !params.include_reviewed {
        rows.retain(is_pending_review);
    }
    let accounts = store.get_accounts().await.context("get_accounts")?;
    let lookup = cash_month_lookup(&accounts);
    let tx_rows: Vec<TxRow> = tx_rows_with_cash_month(&rows, &lookup)
        .into_iter()
        .filter(|row| {
            row.month.as_str() >= cash_from_ref.as_str()
                && row.month.as_str() <= cash_until_ref.as_str()
        })
        .collect();
    let total = tx_rows.len();
    let offset = params.offset.min(total);
    let end = (offset + params.limit).min(total);
    let page: Vec<TxRow> = tx_rows
        .into_iter()
        .skip(offset)
        .take(end - offset)
        .collect();
    let has_more = end < total;
    Ok(TransactionsWindowResult {
        rows: page,
        total,
        offset,
        has_more,
    })
}

/// A transaction is "reviewed" when it has a concrete category that is not the
/// review-pending sentinel `_revisar`.
fn is_reviewed(row: &TransactionRecord) -> bool {
    matches!(row.category_id.as_deref(), Some(cat) if cat != REVIEW_PENDING_CATEGORY)
}

fn is_pending_review(row: &TransactionRecord) -> bool {
    !is_reviewed(row)
}

/// Shift `date` by `months` calendar months, clamping the day to the target
/// month's length (e.g. Jan 31 − 1mo → no Feb 31, so the caller normalises via
/// first/last-of-month helpers below).
fn shift_months(date: NaiveDate, months: i64) -> NaiveDate {
    use chrono::Datelike;
    let zero_based = date.year() as i64 * 12 + (date.month0() as i64) + months;
    let year = zero_based.div_euclid(12) as i32;
    let month0 = zero_based.rem_euclid(12) as u32;
    NaiveDate::from_ymd_opt(year, month0 + 1, 1).unwrap_or(date)
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    use chrono::Datelike;
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date)
}

fn last_of_month(date: NaiveDate) -> NaiveDate {
    use chrono::Datelike;
    let (year, month) = (date.year(), date.month());
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|d| d.pred_opt())
        .unwrap_or(date)
}

/// A forecast paired with its derived `kind`/`draggable` planning metadata.
struct ForecastWithKind {
    record: ForecastRecord,
    kind: String,
    draggable: bool,
}

/// `kind` is the linked template's kind, or `"manual"` for template-less
/// forecasts. Installments and subscriptions are pinned to their schedule and
/// therefore not draggable.
fn forecast_kind(template_id: Option<&str>, template_kinds: &HashMap<String, String>) -> String {
    match template_id {
        None => "manual".to_string(),
        Some(id) => template_kinds
            .get(id)
            .cloned()
            .unwrap_or_else(|| "manual".to_string()),
    }
}

fn kind_is_draggable(kind: &str) -> bool {
    !matches!(kind, "installment" | "subscription")
}

impl ForecastWithKind {
    fn new(record: ForecastRecord, template_kinds: &HashMap<String, String>) -> Self {
        let kind = forecast_kind(record.template_id.as_deref(), template_kinds);
        let draggable = kind_is_draggable(&kind);
        Self {
            record,
            kind,
            draggable,
        }
    }

    /// Serialise as the record's snake_case fields plus `kind`/`draggable`.
    fn to_json(&self) -> Value {
        let mut value = serde_json::to_value(&self.record).unwrap_or_default();
        if let Value::Object(map) = &mut value {
            map.insert("kind".into(), Value::String(self.kind.clone()));
            map.insert("draggable".into(), Value::Bool(self.draggable));
        }
        value
    }
}

/// Build a `template_id → kind` map from the full template list (loaded once).
async fn load_template_kinds(store: &dyn FinanceStore) -> Result<HashMap<String, String>> {
    let templates = store
        .list_forecast_templates(None, None)
        .await
        .context("list_forecast_templates")?;
    Ok(templates
        .into_iter()
        .map(|t| (t.template_id, t.kind))
        .collect())
}

async fn list_forecasts_enriched(
    store: &dyn FinanceStore,
    status: Option<&str>,
    from: Option<NaiveDate>,
    until: Option<NaiveDate>,
) -> Result<Vec<ForecastWithKind>> {
    let template_kinds = load_template_kinds(store).await?;
    let forecasts = store
        .list_forecasts(status, from, until)
        .await
        .context("list_forecasts")?;
    Ok(forecasts
        .into_iter()
        .map(|record| ForecastWithKind::new(record, &template_kinds))
        .collect())
}

/// Outcome of a `POST /api/forecast/move` request, mapped to an HTTP status by
/// the handler.
enum MoveForecastResult {
    Moved { forecast_id: String, status: String },
    NotFound,
    NotMovable,
    PastMonth,
}

/// Reschedule a forecast in place. Installments and subscriptions are pinned to
/// their template schedule and are rejected. The target month must be the
/// current month or a future month — past months are rejected. The idempotency
/// key is recomputed from the new due date and the row is upserted under its
/// existing id (not a new row), with an `AuditEvent`.
async fn move_forecast(
    store: &dyn FinanceStore,
    forecast_id: &str,
    due_date: NaiveDate,
) -> Result<MoveForecastResult> {
    let Some(mut record) = store
        .get_forecast(forecast_id)
        .await
        .context("get_forecast")?
    else {
        return Ok(MoveForecastResult::NotFound);
    };
    let template_kinds = load_template_kinds(store).await?;
    let kind = forecast_kind(record.template_id.as_deref(), &template_kinds);
    if !kind_is_draggable(&kind) {
        return Ok(MoveForecastResult::NotMovable);
    }

    // Target must be the current month or a future month — no retroactive moves.
    let now = Utc::now().date_naive();
    let this_month = first_of_month(now);
    if due_date < this_month {
        return Ok(MoveForecastResult::PastMonth);
    }

    record.due_date = Some(due_date);
    record.updated_at = Utc::now();
    record.actor_id = SERVE_ACTOR_ID.into();
    // Recompute the idempotency key from the new due date.
    record.idempotency_key = String::new();
    ensure_forecast_idempotency(&mut record).context("idempotency")?;
    let status = record.status.clone();
    let diff =
        serde_json::to_value(&record).context("falha ao serializar forecast para auditoria")?;
    store
        .upsert_forecasts(&[record])
        .await
        .context("upsert_forecasts")?;
    let event = AuditEvent {
        event_id: Uuid::now_v7().to_string(),
        entity_type: "forecast".into(),
        entity_id: forecast_id.to_string(),
        action: "move".into(),
        actor_id: SERVE_ACTOR_ID.into(),
        event_timestamp: Utc::now(),
        idempotency_key: Uuid::now_v7().to_string(),
        diff_json: diff,
    };
    store
        .insert_audit_events(&[event])
        .await
        .context("audit move_forecast")?;
    Ok(MoveForecastResult::Moved {
        forecast_id: forecast_id.to_string(),
        status,
    })
}

async fn apply_review_writes(
    store: &dyn FinanceStore,
    config: &AppConfig,
    writes: Vec<ReviewWrite>,
) -> Vec<ReviewWriteOutcome> {
    let mut outcomes = Vec::with_capacity(writes.len());
    for write in writes {
        // Per-write isolation: one failure must not abort the batch.
        let outcome =
            match apply_human_review(store, config, &write.transaction_id, write.patch).await {
                Ok(_) => ReviewWriteOutcome::Acked(write.write_id),
                Err(e) => ReviewWriteOutcome::Failed {
                    write_id: write.write_id,
                    error: e.to_string(),
                },
            };
        outcomes.push(outcome);
    }
    outcomes
}

async fn upsert_forecast(store: &dyn FinanceStore, mut record: ForecastRecord) -> Result<String> {
    let actor_id = record.actor_id.clone();
    if record.forecast_id.is_empty() {
        record.forecast_id = Uuid::now_v7().to_string();
    }
    ensure_forecast_idempotency(&mut record).context("idempotency")?;
    let forecast_id = record.forecast_id.clone();
    let diff =
        serde_json::to_value(&record).context("falha ao serializar forecast para auditoria")?;
    store
        .upsert_forecasts(&[record])
        .await
        .context("upsert_forecasts")?;
    let event = AuditEvent {
        event_id: Uuid::now_v7().to_string(),
        entity_type: "forecast".into(),
        entity_id: forecast_id.clone(),
        action: "upsert".into(),
        actor_id,
        event_timestamp: Utc::now(),
        idempotency_key: Uuid::now_v7().to_string(),
        diff_json: diff,
    };
    store.insert_audit_events(&[event]).await.context("audit")?;
    Ok(forecast_id)
}

async fn handle_accept_template(
    store: &dyn FinanceStore,
    template_id: &str,
    materialize_months: u32,
) -> Result<Value> {
    let mut template = store
        .get_forecast_template(template_id)
        .await?
        .with_context(|| format!("template não encontrado: {template_id}"))?;
    if template.status != "proposto" {
        anyhow::bail!(
            "template {} está com status '{}' (esperado: 'proposto')",
            template_id,
            template.status
        );
    }
    template.status = "ativo".into();
    store.upsert_forecast_templates(&[template.clone()]).await?;
    let event = AuditEvent {
        event_id: Uuid::now_v7().to_string(),
        entity_type: "forecast_template".into(),
        entity_id: template_id.to_string(),
        action: "accept".into(),
        actor_id: SERVE_ACTOR_ID.into(),
        event_timestamp: Utc::now(),
        idempotency_key: Uuid::now_v7().to_string(),
        diff_json: serde_json::json!({ "status": "ativo" }),
    };
    store
        .insert_audit_events(&[event])
        .await
        .context("audit accept_template")?;
    let count = materialise_template_forecasts(
        store,
        &template,
        materialize_months,
        SERVE_ACTOR_ID,
        Utc::now(),
    )
    .await?;
    Ok(serde_json::json!({
        "template_id": template_id,
        "forecasts_created": count
    }))
}

async fn handle_dismiss_template(store: &dyn FinanceStore, template_id: &str) -> Result<()> {
    let mut template = store
        .get_forecast_template(template_id)
        .await?
        .with_context(|| format!("template não encontrado: {template_id}"))?;
    if template.status == "descartado" {
        return Ok(());
    }
    template.status = "descartado".into();
    store.upsert_forecast_templates(&[template]).await?;
    let event = AuditEvent {
        event_id: Uuid::now_v7().to_string(),
        entity_type: "forecast_template".into(),
        entity_id: template_id.to_string(),
        action: "dismiss".into(),
        actor_id: SERVE_ACTOR_ID.into(),
        event_timestamp: Utc::now(),
        idempotency_key: Uuid::now_v7().to_string(),
        diff_json: serde_json::json!({ "status": "descartado" }),
    };
    store
        .insert_audit_events(&[event])
        .await
        .context("audit dismiss_template")?;
    Ok(())
}

// ── REST DTOs ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ReviewQueueResponse {
    rows: Vec<TxRow>,
}

/// One transaction in the planning workspace shape. Field names are camelCase
/// by contract; the frontend computes all sums client-side.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TxRow {
    id: String,
    account_id: Option<String>,
    posted_at: String,
    /// Decimal serialised as a string (e.g. `"-12.50"`) — never f64.
    amount: String,
    raw_description: String,
    description: Option<String>,
    merchant_name: Option<String>,
    purpose: Option<String>,
    category_id: Option<String>,
    month: String,
    payment_status: String,
    installment_marker: Option<String>,
    reviewed: bool,
    is_installment: bool,
    is_subscription: bool,
}

impl TxRow {
    fn from_record(row: &TransactionRecord) -> Self {
        let category_id = canonical_web_category(row.category_id.as_deref());
        let installment_marker = parse_installment_description(&row.raw_description)
            .or_else(|| {
                row.description
                    .as_deref()
                    .and_then(parse_installment_description)
            })
            .map(|m| format!("{}/{}", m.current, m.total));
        // `isSubscription` heuristic: any category under the `assinaturas:`
        // namespace is treated as a subscription charge.
        let is_subscription = category_id
            .as_deref()
            .is_some_and(|cat| cat.starts_with(SUBSCRIPTION_CATEGORY_PREFIX));
        Self {
            id: row.transaction_id.clone(),
            account_id: row.account_id.clone(),
            posted_at: row.transaction_date.format("%Y-%m-%d").to_string(),
            amount: format!("{:.2}", row.amount.round_dp(2)),
            raw_description: {
                debug_assert!(
                    {
                        let parsed = rust_decimal::Decimal::from_str(&format!(
                            "{:.2}",
                            row.amount.round_dp(2)
                        ))
                        .unwrap_or_default();
                        row.amount == parsed
                    },
                    "amount precision lost for tx {}: {} → {:.2}",
                    row.transaction_id,
                    row.amount,
                    row.amount.round_dp(2)
                );
                row.raw_description.clone()
            },
            description: row.description.clone(),
            merchant_name: row.merchant_name.clone(),
            purpose: row.purpose.clone(),
            category_id,
            month: row.transaction_date.format("%Y-%m").to_string(),
            payment_status: row.payment_status.clone(),
            installment_marker: installment_marker.clone(),
            reviewed: is_reviewed(row),
            is_installment: row.payment_status == PAYMENT_STATUS_INSTALLMENT
                || installment_marker.is_some(),
            is_subscription,
        }
    }
}

/// Parse a `billing_closing_day` / `billing_due_day` value (stored as a string
/// or a number) from account metadata.
fn account_billing_day(meta: &Value, key: &str) -> Option<u32> {
    match meta.get(key)? {
        Value::String(s) => s.trim().parse::<u32>().ok(),
        Value::Number(n) => n.as_u64().map(|d| d as u32),
        _ => None,
    }
    .filter(|d| (1..=31).contains(d))
}

/// `account_id → (is_credit, closing_day, due_day)` for cash_month bucketing.
fn cash_month_lookup(
    accounts: &[AccountRecord],
) -> std::collections::HashMap<String, (bool, Option<u32>, Option<u32>)> {
    accounts
        .iter()
        .map(|a| {
            (
                a.account_id.clone(),
                (
                    a.account_type == "credit",
                    account_billing_day(&a.metadata_json, "billing_closing_day"),
                    account_billing_day(&a.metadata_json, "billing_due_day"),
                ),
            )
        })
        .collect()
}

/// Build the web transaction rows, overriding `month` with the canonical
/// `cash_month` so a card purchase appears under the month its bill is paid
/// (the family cash-flow view — see ADR-0025). Falls back to the posting month
/// when the account is unknown.
fn tx_rows_with_cash_month(
    rows: &[TransactionRecord],
    lookup: &std::collections::HashMap<String, (bool, Option<u32>, Option<u32>)>,
) -> Vec<TxRow> {
    rows.iter()
        .map(|row| {
            let mut tr = TxRow::from_record(row);
            if let Some((is_credit, closing, due)) =
                row.account_id.as_deref().and_then(|id| lookup.get(id))
            {
                tr.month = phai_core::cashflow::cash_month_for(
                    row.transaction_date,
                    *is_credit,
                    *closing,
                    *due,
                );
            }
            tr
        })
        .collect()
}

/// Fetch accounts via the store actor (best-effort: an empty list just means
/// rows keep their posting month). Used to resolve `cash_month`.
async fn fetch_accounts(tx: &mpsc::Sender<StoreRequest>) -> Vec<AccountRecord> {
    let (resp_tx, resp_rx) = oneshot::channel();
    if tx
        .send(StoreRequest::GetAccounts { resp: resp_tx })
        .await
        .is_err()
    {
        return Vec::new();
    }
    resp_rx.await.ok().and_then(Result::ok).unwrap_or_default()
}

/// Per-credit-card cycle state for the dashboard card panel. `state` is
/// `"aberta"` when the card has an open bill with a balance, else `"em-dia"`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CardApiRow {
    account_id: String,
    label: String,
    state: &'static str,
    cycle_month: Option<String>,
    /// Total charged on the open cycle (R$, 2dp string).
    total: String,
    /// Amount still open/owed on the cycle.
    open_amount: String,
    /// Bill due date (YYYY-MM-DD) when `billing_due_day` is known.
    due_date: Option<String>,
    /// Card credit limit and used amount from Pluggy metadata, for a usage bar.
    credit_limit: Option<String>,
    used_amount: Option<String>,
    installment_debt: String,
    installment_month_amount: String,
    installment_ending_amount: String,
    installment_count: usize,
    installments: Vec<CardInstallmentApiRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CardInstallmentApiRow {
    transaction_id: String,
    transaction_date: String,
    description: String,
    amount: String,
    marker: String,
    current: u32,
    total: u32,
    remaining: u32,
    ending_this_month: bool,
}

#[derive(Serialize)]
struct CardsResponse {
    rows: Vec<CardApiRow>,
}

struct CardCreditMeta {
    due_day: Option<u32>,
    credit_limit: Option<f64>,
    used_amount: Option<f64>,
}

/// Due date (YYYY-MM-DD) for a cycle `"YYYY-MM"` given the billing due day,
/// clamped to the month length.
fn cycle_due_date(month_ref: &str, due_day: u32) -> Option<String> {
    let mut parts = month_ref.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let next_first = if m == 12 {
        NaiveDate::from_ymd_opt(y + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(y, m + 1, 1)?
    };
    let last_day = next_first.pred_opt()?.day();
    let day = due_day.min(last_day);
    Some(
        NaiveDate::from_ymd_opt(y, m, day)?
            .format("%Y-%m-%d")
            .to_string(),
    )
}

/// Assemble the card panel: each active credit account with the selected card
/// cycle when `month_ref` is provided, otherwise with the currently-open bill
/// (from `cards_open_now`, ADR-0010). Cards with no selected-cycle charges are
/// reported `state="em-dia"`.
async fn build_cards_api(
    store: &dyn FinanceStore,
    month_ref: Option<&str>,
) -> Result<Vec<CardApiRow>> {
    let today = chrono::Local::now().date_naive();
    let accounts = store.get_accounts().await?;
    let summaries = match month_ref {
        Some(month) => store.card_summary(Some(month)).await?,
        None => store.cards_open_now().await?,
    };
    let summary_by_account: std::collections::HashMap<&str, &CardSummaryRow> = summaries
        .iter()
        .map(|r| (r.account_id.as_str(), r))
        .collect();
    let mut installments_by_account = match month_ref {
        Some(month) => card_installments_by_account_for_cash_month(store, month, &accounts).await?,
        None => std::collections::HashMap::new(),
    };

    let mut rows = Vec::new();
    for a in accounts
        .iter()
        .filter(|a| a.account_type == "credit" && a.status.eq_ignore_ascii_case("active"))
    {
        let installments = installments_by_account
            .remove(&a.account_id)
            .unwrap_or_default();
        rows.push(card_api_row_for_account(
            a,
            summary_by_account.get(a.account_id.as_str()).copied(),
            card_credit_meta(a),
            installments,
            today,
        ));
    }
    rows.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(rows)
}

fn card_credit_meta(account: &AccountRecord) -> CardCreditMeta {
    let credit_limit = account
        .metadata_json
        .pointer("/raw/creditData/creditLimit")
        .and_then(serde_json::Value::as_f64);
    let available = account
        .metadata_json
        .pointer("/raw/creditData/availableCreditLimit")
        .and_then(serde_json::Value::as_f64);
    CardCreditMeta {
        due_day: account_billing_day(&account.metadata_json, "billing_due_day"),
        credit_limit,
        used_amount: match (credit_limit, available) {
            (Some(c), Some(av)) => Some(c - av),
            _ => None,
        },
    }
}

fn card_label(account: &AccountRecord) -> String {
    if account.label.is_empty() {
        account.account_id.clone()
    } else {
        account.label.clone()
    }
}

fn selected_card_state(
    due_date: Option<&str>,
    open_amount: Decimal,
    today: NaiveDate,
) -> &'static str {
    if due_date
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .is_some_and(|d| d < today)
    {
        "fechada"
    } else if open_amount > Decimal::ZERO {
        "aberta"
    } else {
        "fechada"
    }
}

fn card_api_row_for_account(
    account: &AccountRecord,
    summary: Option<&CardSummaryRow>,
    meta: CardCreditMeta,
    installments: Vec<CardInstallmentApiRow>,
    today: NaiveDate,
) -> CardApiRow {
    let (installment_debt, installment_month_amount, installment_ending_amount) =
        card_installment_totals(&installments);
    match summary {
        Some(row) => {
            let due_date = meta.due_day.and_then(|d| cycle_due_date(&row.month_ref, d));
            CardApiRow {
                account_id: account.account_id.clone(),
                label: card_label(account),
                state: selected_card_state(due_date.as_deref(), row.open_amount, today),
                cycle_month: Some(row.month_ref.clone()),
                total: format!("{:.2}", row.total_charges),
                open_amount: format!("{:.2}", row.open_amount),
                due_date,
                credit_limit: meta.credit_limit.map(|c| format!("{c:.2}")),
                used_amount: meta.used_amount.map(|u| format!("{u:.2}")),
                installment_debt: format!("{:.2}", installment_debt),
                installment_month_amount: format!("{:.2}", installment_month_amount),
                installment_ending_amount: format!("{:.2}", installment_ending_amount),
                installment_count: installments.len(),
                installments,
            }
        }
        None => CardApiRow {
            account_id: account.account_id.clone(),
            label: card_label(account),
            state: "em-dia",
            cycle_month: None,
            total: "0.00".to_string(),
            open_amount: "0.00".to_string(),
            due_date: None,
            credit_limit: meta.credit_limit.map(|c| format!("{c:.2}")),
            used_amount: meta.used_amount.map(|u| format!("{u:.2}")),
            installment_debt: format!("{:.2}", installment_debt),
            installment_month_amount: format!("{:.2}", installment_month_amount),
            installment_ending_amount: format!("{:.2}", installment_ending_amount),
            installment_count: installments.len(),
            installments,
        },
    }
}

async fn card_installments_by_account_for_cash_month(
    store: &dyn FinanceStore,
    month_ref: &str,
    accounts: &[AccountRecord],
) -> Result<std::collections::HashMap<String, Vec<CardInstallmentApiRow>>> {
    let month_start = parse_month_ref(month_ref)?;
    let cash_until = last_of_month(month_start);
    let posted_from = first_of_month(shift_months(month_start, -2));
    let mut rows = store
        .effective_transactions_window(None, posted_from, cash_until)
        .await
        .context("card effective_transactions_window")?;
    let internal_categories = store
        .internal_categories()
        .await
        .context("card internal_categories")?;
    rows.retain(|row| {
        let raw_category = row.category_id.as_deref();
        let canonical_category = canonical_web_category(raw_category);
        !raw_category.is_some_and(|category| internal_categories.contains(category))
            && !canonical_category
                .as_deref()
                .is_some_and(|category| internal_categories.contains(category))
    });
    // ofx-shadowed-by-pluggy rows are already dropped by v_transactions_reportable
    // (migration 038 / ADR-0026); no Rust-side dedup needed here.

    let lookup = cash_month_lookup(accounts);
    let mut by_account: std::collections::HashMap<String, Vec<CardInstallmentApiRow>> =
        std::collections::HashMap::new();
    for row in &rows {
        let Some(account_id) = row.account_id.as_deref() else {
            continue;
        };
        let Some((is_credit, closing, due)) = lookup.get(account_id) else {
            continue;
        };
        if !*is_credit {
            continue;
        }
        let cash_month =
            phai_core::cashflow::cash_month_for(row.transaction_date, *is_credit, *closing, *due);
        if cash_month != month_ref {
            continue;
        }
        let Some(installment) = card_installment_api_row(row) else {
            continue;
        };
        by_account
            .entry(account_id.to_string())
            .or_default()
            .push(installment);
    }
    for rows in by_account.values_mut() {
        rows.sort_by(|a, b| {
            a.description
                .cmp(&b.description)
                .then_with(|| a.marker.cmp(&b.marker))
                .then_with(|| a.transaction_id.cmp(&b.transaction_id))
        });
    }
    Ok(by_account)
}

fn card_installment_api_row(row: &TransactionRecord) -> Option<CardInstallmentApiRow> {
    let marker = parse_installment_description(&row.raw_description)
        .or_else(|| {
            row.description
                .as_deref()
                .and_then(parse_installment_description)
        })
        .or_else(|| {
            row.merchant_name
                .as_deref()
                .and_then(parse_installment_description)
        })
        .or_else(|| {
            row.metadata_json
                .get("raw")
                .and_then(|r| r.get("descriptionRaw"))
                .and_then(|v| v.as_str())
                .and_then(parse_installment_description)
        })?;
    let remaining = marker.total - marker.current + 1;
    Some(CardInstallmentApiRow {
        transaction_id: row.transaction_id.clone(),
        transaction_date: row.transaction_date.format("%Y-%m-%d").to_string(),
        description: row
            .description
            .clone()
            .or_else(|| row.merchant_name.clone())
            .unwrap_or_else(|| row.raw_description.clone()),
        amount: format!("{:.2}", row.amount.round_dp(2).abs()),
        marker: format!("{}/{}", marker.current, marker.total),
        current: marker.current,
        total: marker.total,
        remaining,
        ending_this_month: marker.current == marker.total,
    })
}

fn card_installment_totals(rows: &[CardInstallmentApiRow]) -> (Decimal, Decimal, Decimal) {
    rows.iter().fold(
        (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO),
        |(debt, month, ending), row| {
            let amount = Decimal::from_str(&row.amount).unwrap_or(Decimal::ZERO);
            (
                debt + amount * Decimal::from(row.remaining),
                month + amount,
                if row.ending_this_month {
                    ending + amount
                } else {
                    ending
                },
            )
        },
    )
}

#[derive(Serialize)]
struct TransactionsResponse {
    rows: Vec<TxRow>,
    total: usize,
    offset: usize,
    #[serde(rename = "hasMore")]
    has_more: bool,
}

/// Account summary for the accounts picker.
#[derive(Serialize)]
struct AccountRow {
    id: String,
    label: String,
    owner: String,
}

impl AccountRow {
    fn from_record(account: &AccountRecord) -> Self {
        let label = if account.label.trim().is_empty() {
            account.account_id.clone()
        } else {
            account.label.clone()
        };
        Self {
            id: account.account_id.clone(),
            label,
            owner: account.owner.clone(),
        }
    }
}

#[derive(Serialize)]
struct AccountsResponse {
    rows: Vec<AccountRow>,
}

#[derive(Serialize)]
struct CategoriesResponse {
    ids: Vec<String>,
}

// ── REST query/body params ───────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct ReviewQueueQuery {
    month: Option<String>,
    owner: Option<String>,
    account_id: Option<String>,
    merchant: Option<String>,
    category: Option<String>,
    #[serde(default)]
    include_reviewed: bool,
    limit: Option<usize>,
}

#[derive(Deserialize, Default)]
struct ChartQuery {
    months_back: Option<usize>,
    months_ahead: Option<usize>,
}

#[derive(Deserialize, Default)]
struct CardsQuery {
    month: Option<String>,
}

#[derive(Deserialize, Default)]
struct TransactionsQuery {
    months_back: Option<u32>,
    months_ahead: Option<u32>,
    include_reviewed: Option<bool>,
    limit: Option<usize>,
    offset: Option<usize>,
}

#[derive(Deserialize, Default)]
struct ForecastsQuery {
    status: Option<String>,
    from: Option<String>,
    until: Option<String>,
}

#[derive(Deserialize, Default)]
struct TemplatesQuery {
    kind: Option<String>,
    status: Option<String>,
}

/// camelCase patch as sent by the frontend.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewPatchBody {
    description: Option<String>,
    merchant_name: Option<String>,
    purpose: Option<String>,
    category_id: Option<String>,
}

impl ReviewPatchBody {
    fn into_patch(self) -> HumanReviewPatch {
        HumanReviewPatch {
            description: self.description,
            merchant_name: self.merchant_name,
            purpose: self.purpose,
            category_id: self.category_id,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventWrite {
    write_id: String,
    transaction_id: String,
    patch: ReviewPatchBody,
}

#[derive(Deserialize)]
struct EventsBody {
    #[serde(default)]
    writes: Vec<EventWrite>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventFailure {
    write_id: String,
    error: String,
}

#[derive(Serialize)]
struct EventsResponse {
    acked: Vec<String>,
    failed: Vec<EventFailure>,
}

#[derive(Deserialize)]
struct ForecastBody {
    #[serde(default)]
    description: String,
    amount: String,
    due_date: Option<String>,
    category_id: Option<String>,
    account_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveForecastBody {
    forecast_id: String,
    due_date: String,
}

#[derive(Deserialize)]
struct AcceptTemplateBody {
    template_id: String,
    materialize_months: Option<u32>,
}

#[derive(Deserialize)]
struct DismissTemplateBody {
    template_id: String,
}

// ── HTTP handlers ────────────────────────────────────────────────────────

type Store = State<Arc<RwLock<mpsc::Sender<StoreRequest>>>>;

/// Build a JSON error response with the given status.
fn error_response(status: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn actor_unavailable() -> axum::response::Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "store actor indisponível",
    )
}

fn actor_silent() -> axum::response::Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "store actor não respondeu",
    )
}

/// Clone the current store-actor sender. The read-lock is held only for the
/// cheap clone so concurrent requests are not serialised.
async fn clone_actor_tx(
    tx: &Arc<RwLock<mpsc::Sender<StoreRequest>>>,
) -> mpsc::Sender<StoreRequest> {
    tx.read().await.clone()
}

async fn api_status() -> Json<Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn get_review_queue(
    State(tx): Store,
    Query(q): Query<ReviewQueueQuery>,
) -> impl IntoResponse {
    let filters = ReviewFilters {
        month: q.month,
        account_id: q.account_id,
        owner_accounts: None,
        owner: q.owner,
        merchant: q.merchant,
        category: q
            .category
            .as_deref()
            .map(|value| crate::category_key_from_input(value, None)),
    };
    let params = ReviewQueueParams {
        filters,
        include_reviewed: q.include_reviewed,
        limit: q.limit.unwrap_or(DEFAULT_REVIEW_QUEUE_LIMIT),
    };
    let tx = clone_actor_tx(&tx).await;
    let accounts = fetch_accounts(&tx).await;
    let (resp_tx, resp_rx) = oneshot::channel();
    if tx
        .send(StoreRequest::ReviewQueue {
            params,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(rows)) => {
            let lookup = cash_month_lookup(&accounts);
            Json(ReviewQueueResponse {
                rows: tx_rows_with_cash_month(&rows, &lookup),
            })
            .into_response()
        }
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn get_transactions(
    State(tx): Store,
    Query(q): Query<TransactionsQuery>,
) -> impl IntoResponse {
    let params = TransactionsWindowParams {
        months_back: q.months_back.unwrap_or(DEFAULT_TRANSACTIONS_MONTHS_BACK),
        months_ahead: q.months_ahead.unwrap_or(0),
        include_reviewed: q.include_reviewed.unwrap_or(true),
        limit: q.limit.unwrap_or(DEFAULT_TRANSACTIONS_LIMIT),
        offset: q.offset.unwrap_or(0),
    };
    let tx = clone_actor_tx(&tx).await;
    let (resp_tx, resp_rx) = oneshot::channel();
    if tx
        .send(StoreRequest::TransactionsWindow {
            params,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(result)) => Json(TransactionsResponse {
            rows: result.rows,
            total: result.total,
            offset: result.offset,
            has_more: result.has_more,
        })
        .into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn get_categories(State(tx): Store) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::ListCategoryIds { resp: resp_tx })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(ids)) => Json(CategoriesResponse { ids }).into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn get_cards(State(tx): Store, Query(q): Query<CardsQuery>) -> impl IntoResponse {
    if let Some(month) = q.month.as_deref() {
        if let Err(e) = parse_month_ref(month) {
            return error_response(StatusCode::BAD_REQUEST, e.to_string());
        }
    }
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::Cards {
            month: q.month,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(rows)) => Json(CardsResponse { rows }).into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn get_accounts(State(tx): Store) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::GetAccounts { resp: resp_tx })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(accounts)) => Json(AccountsResponse {
            rows: accounts.iter().map(AccountRow::from_record).collect(),
        })
        .into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn get_chart(State(tx): Store, Query(q): Query<ChartQuery>) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::GetChartData {
            months_back: q.months_back.unwrap_or(6),
            months_ahead: q.months_ahead.unwrap_or(6),
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(chart)) => Json(chart).into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn get_forecasts(State(tx): Store, Query(q): Query<ForecastsQuery>) -> impl IntoResponse {
    let from = match parse_opt_date(q.from.as_deref()) {
        Ok(d) => d,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let until = match parse_opt_date(q.until.as_deref()) {
        Ok(d) => d,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::ListForecastsEnriched {
            status: q.status,
            from,
            until,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        // Each forecast keeps its snake_case fields and gains computed
        // `kind`/`draggable` planning metadata; the frontend adapts.
        Ok(Ok(forecasts)) => {
            let rows: Vec<Value> = forecasts.iter().map(ForecastWithKind::to_json).collect();
            Json(serde_json::json!({ "forecasts": rows })).into_response()
        }
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn get_forecast_templates(
    State(tx): Store,
    Query(q): Query<TemplatesQuery>,
) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::ListForecastTemplates {
            kind: q.kind,
            status: q.status,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        // ForecastTemplateRecord serialises as snake_case; the frontend adapts.
        Ok(Ok(templates)) => Json(serde_json::json!({ "templates": templates })).into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn post_events(State(tx): Store, Json(body): Json<EventsBody>) -> impl IntoResponse {
    let writes: Vec<ReviewWrite> = body
        .writes
        .into_iter()
        .map(|w| ReviewWrite {
            write_id: w.write_id,
            transaction_id: w.transaction_id,
            patch: w.patch.into_patch(),
        })
        .collect();
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::ApplyHumanReview {
            writes,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(outcomes) => {
            let mut acked = Vec::new();
            let mut failed = Vec::new();
            for outcome in outcomes {
                match outcome {
                    ReviewWriteOutcome::Acked(id) => acked.push(id),
                    ReviewWriteOutcome::Failed { write_id, error } => {
                        failed.push(EventFailure { write_id, error })
                    }
                }
            }
            Json(EventsResponse { acked, failed }).into_response()
        }
        Err(_) => actor_silent(),
    }
}

async fn post_forecast(State(tx): Store, Json(body): Json<ForecastBody>) -> impl IntoResponse {
    let amount = match Decimal::from_str(body.amount.trim()) {
        Ok(d) => d,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("amount inválido: '{}' (use formato: -250.00)", body.amount),
            )
        }
    };
    let due_date = match parse_opt_date(body.due_date.as_deref()) {
        Ok(d) => d,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    // Input-length guardrails: prevent extremely long strings from landing in
    // the database. The limits match typical UX constraints (description is a
    // short label, not a free-text note).
    const MAX_DESC_LEN: usize = 500;
    const MAX_ID_LEN: usize = 100;
    if body.description.len() > MAX_DESC_LEN {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "description muito longo ({} caracteres; máximo {})",
                body.description.len(),
                MAX_DESC_LEN
            ),
        );
    }
    if body
        .category_id
        .as_deref()
        .is_some_and(|c| c.len() > MAX_ID_LEN)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "category_id muito longo ({} caracteres; máximo {})",
                body.category_id.as_deref().unwrap().len(),
                MAX_ID_LEN
            ),
        );
    }
    if body
        .account_id
        .as_deref()
        .is_some_and(|a| a.len() > MAX_ID_LEN)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "account_id muito longo ({} caracteres; máximo {})",
                body.account_id.as_deref().unwrap().len(),
                MAX_ID_LEN
            ),
        );
    }
    let record = Box::new(ForecastRecord {
        forecast_id: String::new(),
        due_date,
        description: body.description,
        amount,
        category_id: body.category_id,
        account_id: body.account_id,
        status: "ativo".into(),
        recurrence: None,
        actor_id: SERVE_ACTOR_ID.into(),
        idempotency_key: String::new(),
        metadata_json: Value::Object(Default::default()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        template_id: None,
        realized_transaction_id: None,
        realized_at: None,
    });
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::UpsertForecast {
            record,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(forecast_id)) => {
            Json(serde_json::json!({ "forecastId": forecast_id })).into_response()
        }
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn post_forecast_move(
    State(tx): Store,
    Json(body): Json<MoveForecastBody>,
) -> impl IntoResponse {
    let due_date = match NaiveDate::parse_from_str(body.due_date.trim(), "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("dueDate inválida: '{}'", body.due_date),
            )
        }
    };
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::MoveForecast {
            forecast_id: body.forecast_id,
            due_date,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(MoveForecastResult::Moved {
            forecast_id,
            status,
        })) => Json(serde_json::json!({
            "forecastId": forecast_id,
            "dueDate": due_date.format("%Y-%m-%d").to_string(),
            "status": status,
        }))
        .into_response(),
        Ok(Ok(MoveForecastResult::NotFound)) => {
            error_response(StatusCode::NOT_FOUND, "forecast não encontrado")
        }
        Ok(Ok(MoveForecastResult::NotMovable)) => error_response(
            StatusCode::BAD_REQUEST,
            "forecast não é movível (parcelamento/assinatura)",
        ),
        Ok(Ok(MoveForecastResult::PastMonth)) => error_response(
            StatusCode::BAD_REQUEST,
            "não é possível mover forecast para um mês passado",
        ),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn post_accept_template(
    State(tx): Store,
    Json(body): Json<AcceptTemplateBody>,
) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::AcceptTemplate {
            template_id: body.template_id,
            materialize_months: body.materialize_months.unwrap_or(6),
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(result)) => Json(result).into_response(),
        Ok(Err(e)) => error_response(StatusCode::BAD_REQUEST, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn post_dismiss_template(
    State(tx): Store,
    Json(body): Json<DismissTemplateBody>,
) -> impl IntoResponse {
    let template_id = body.template_id.clone();
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&tx).await;
    if tx
        .send(StoreRequest::DismissTemplate {
            template_id: body.template_id,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(())) => Json(serde_json::json!({ "template_id": template_id })).into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

/// Parse an optional `YYYY-MM-DD` date. Returns the offending value as the
/// error so callers can build a 400 response.
fn parse_opt_date(value: Option<&str>) -> Result<Option<NaiveDate>, String> {
    match value {
        None | Some("") => Ok(None),
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| format!("data inválida: '{s}'")),
    }
}

// ── Entry point ──────────────────────────────────────────────────────────

pub async fn run(port: u16) -> Result<()> {
    let (_, config) = load_config().await?;
    let config: AppConfig = config;
    let bridge_identity = Arc::new(bridge_identity_response(&config));
    let actor_config = config.clone();

    // Shared sender: the store actor replaces the inner sender on each restart
    // so handlers always reach the live actor. Initialised with a dummy channel
    // (immediately replaced by the actor on first start).
    let store_tx: Arc<RwLock<mpsc::Sender<StoreRequest>>> = Arc::new(RwLock::new(
        mpsc::channel::<StoreRequest>(STORE_CHANNEL_CAP).0,
    ));

    let local = LocalSet::new();

    // Spawn the !Send store actor on the local set with restart-on-failure.
    // If open_store / run_migrations fails (e.g. transient disk full), the
    // actor sleeps 1 s and retries with a fresh channel. Handlers that were
    // mid-flight with the old sender get "actor unavailable" and the client
    // retries — the window is ~1 s.
    let actor_tx = store_tx.clone();
    local.spawn_local(async move {
        loop {
            let (tx, rx) = mpsc::channel::<StoreRequest>(STORE_CHANNEL_CAP);
            *actor_tx.write().await = tx;

            match async {
                let store = open_store(&actor_config).await?;
                run_migrations(store.as_ref(), &actor_config).await?;
                store_actor_loop(store, actor_config.clone(), rx).await;
                Ok::<_, anyhow::Error>(())
            }
            .await
            {
                Ok(()) => break, // clean shutdown
                Err(e) => {
                    eprintln!(
                        "[phai serve] store actor caiu: {e:#}\n\
                         [phai serve] reiniciando em 1s..."
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });

    let app_state = store_tx;

    // All `/api` routes are guarded by the same-origin check so a malicious
    // page cannot drive the bridge via the user's browser (CSRF). Requests
    // without an Origin header (curl, direct integration) are allowed.
    let api = Router::new()
        .route("/api", get(api_status))
        .route(
            "/api/identity",
            get({
                let bridge_identity = bridge_identity.clone();
                move || {
                    let bridge_identity = bridge_identity.clone();
                    async move { Json((*bridge_identity).clone()) }
                }
            }),
        )
        .route("/api/review-queue", get(get_review_queue))
        .route("/api/transactions", get(get_transactions))
        .route("/api/categories", get(get_categories))
        .route("/api/accounts", get(get_accounts))
        .route("/api/cards", get(get_cards))
        .route("/api/chart", get(get_chart))
        .route("/api/forecasts", get(get_forecasts))
        .route("/api/forecast-templates", get(get_forecast_templates))
        .route("/api/events", post(post_events))
        .route("/api/forecast", post(post_forecast))
        .route("/api/forecast/move", post(post_forecast_move))
        .route("/api/forecast-template/accept", post(post_accept_template))
        .route(
            "/api/forecast-template/dismiss",
            post(post_dismiss_template),
        )
        .layer(axum::middleware::from_fn(guard_origin))
        // Operation log (debug builds only): method, path, status, latency.
        .layer(axum::middleware::from_fn(log_ops))
        // Security headers on every response — outermost so they are never
        // skipped even if an inner layer short-circuits.
        .layer(axum::middleware::from_fn(security_headers))
        .with_state(app_state);

    let app = api
        // Serve the embedded LiveStore SPA for everything else (index + assets
        // + client-side routes).
        .fallback(crate::serve_assets::static_handler);

    let addr = format!("{LOCAL_BIND_HOST}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| {
            if port < 1024 {
                format!("falha ao escutar em {addr} — a porta {port} é privilegiada; rode com sudo")
            } else {
                format!("falha ao escutar em {addr}")
            }
        })?;

    // Port 80 is implicit in the URL the browser shows.
    let url = if port == 80 {
        format!("http://{LOCAL_APP_HOST}")
    } else {
        format!("http://{LOCAL_APP_HOST}:{port}")
    };
    println!("🌐 phai em {url}");
    if cfg!(debug_assertions) {
        println!("   (build debug — log de operações ativo)");
    }
    println!("   Pressione Ctrl+C para parar");
    open_browser(&url);

    local
        .run_until(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .context("servidor web parou")
        })
        .await?;

    Ok(())
}

/// Returns a future that completes when the process receives SIGINT or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    eprintln!("\n[phai serve] encerrando...");
}

/// Reject `/api` requests whose `Origin` is not localhost. Runs before every
/// API handler.
async fn guard_origin(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !is_origin_allowed(req.headers()) {
        let origin = req
            .headers()
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<missing>");
        eprintln!(
            "[phai serve] origem rejeitada: {origin} — {} {}",
            req.method(),
            req.uri().path()
        );
        return (StatusCode::FORBIDDEN, "Origin não permitida").into_response();
    }
    next.run(req).await
}

/// Add baseline security headers to every response.
async fn security_headers(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("interest-cohort=()"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
             font-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; \
             frame-ancestors 'none'",
        ),
    );
    resp
}

/// Log every `/api` operation in debug builds: method, path, status, latency.
/// No-op in release builds (the closure is elided by the compiler).
async fn log_ops(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !cfg!(debug_assertions) {
        return next.run(req).await;
    }
    let method = req.method().clone();
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let started = std::time::Instant::now();
    let resp = next.run(req).await;
    eprintln!(
        "[phai serve] {method} {path} → {} ({} ms)",
        resp.status().as_u16(),
        started.elapsed().as_millis()
    );
    resp
}

/// Open the web app in the user's default browser. Best-effort — failures are
/// logged (debug) and never block the server. When running under `sudo`, open
/// as the invoking user so the browser attaches to their GUI session.
fn open_browser(url: &str) {
    use std::process::Command;
    let result = if cfg!(target_os = "macos") {
        match std::env::var("SUDO_USER") {
            Ok(user) if !user.is_empty() => Command::new("sudo")
                .args(["-u", &user, "open", url])
                .spawn(),
            _ => Command::new("open").arg(url).spawn(),
        }
    } else if cfg!(target_os = "linux") {
        // Try xdg-open first (X11/Wayland with xdg-utils)
        let xdg = Command::new("xdg-open").arg(url).spawn();
        match xdg {
            Ok(child) => Ok(child),
            Err(_) => {
                // Fall back to gio open (GNOME/Wayland without xdg-utils)
                let gio = Command::new("gio").args(["open", url]).spawn();
                match gio {
                    Ok(child) => Ok(child),
                    Err(_) => {
                        // Fall back to wslview (WSL)
                        Command::new("wslview").arg(url).spawn()
                    }
                }
            }
        }
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(e) = result {
        eprintln!("[phai serve] não consegui abrir o browser automaticamente: {e}");
    }
}

/// Permite apenas conexões de localhost ou sem Origin (curl, integração direta).
/// Rejeita qualquer outro Origin para prevenir Cross-Site Request Forgery.
fn is_origin_allowed(headers: &HeaderMap) -> bool {
    match headers.get("origin") {
        None => true,
        Some(v) => {
            let origin = v.to_str().unwrap_or("");
            origin.starts_with("http://localhost:")
                || origin.starts_with("http://127.0.0.1:")
                // phai.localhost with an explicit port, or bare (port 80 → no
                // port in the Origin header).
                || origin.starts_with("http://phai.localhost:")
                || origin == "http://phai.localhost"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    // ── is_origin_allowed ──────────────────────────────────────────────────

    #[test]
    fn origin_absent_is_allowed() {
        assert!(is_origin_allowed(&HeaderMap::new()));
    }

    #[test]
    fn localhost_origin_allowed() {
        let mut h = HeaderMap::new();
        h.insert("origin", HeaderValue::from_static("http://localhost:8080"));
        assert!(is_origin_allowed(&h));
    }

    #[test]
    fn loopback_origin_allowed() {
        let mut h = HeaderMap::new();
        h.insert("origin", HeaderValue::from_static("http://127.0.0.1:8080"));
        assert!(is_origin_allowed(&h));
    }

    #[test]
    fn localhost_alias_origin_allowed() {
        let mut h = HeaderMap::new();
        h.insert(
            "origin",
            HeaderValue::from_static("http://phai.localhost:8080"),
        );
        assert!(is_origin_allowed(&h));
    }

    #[test]
    fn localhost_alias_bare_origin_allowed() {
        // Port 80 → the browser omits the port from the Origin header.
        let mut h = HeaderMap::new();
        h.insert("origin", HeaderValue::from_static("http://phai.localhost"));
        assert!(is_origin_allowed(&h));
    }

    #[test]
    fn null_origin_is_rejected() {
        let mut h = HeaderMap::new();
        h.insert("origin", HeaderValue::from_static("null"));
        assert!(!is_origin_allowed(&h));
    }

    #[test]
    fn external_origin_rejected() {
        let mut h = HeaderMap::new();
        h.insert(
            "origin",
            HeaderValue::from_static("https://evil.example.com"),
        );
        assert!(!is_origin_allowed(&h));
    }

    #[test]
    fn lan_ip_origin_rejected() {
        let mut h = HeaderMap::new();
        h.insert(
            "origin",
            HeaderValue::from_static("http://192.168.1.100:8080"),
        );
        assert!(!is_origin_allowed(&h));
    }

    // ── DTO serialisation contract (camelCase) ─────────────────────────────

    fn sample_record() -> TransactionRecord {
        TransactionRecord {
            transaction_id: "tx-1".into(),
            account_id: Some("acc-1".into()),
            transaction_date: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
            raw_description: "RAW MERCHANT 123".into(),
            description: Some("Almoço".into()),
            merchant_name: Some("Restaurante".into()),
            purpose: Some("Trabalho".into()),
            amount: Decimal::from_str("-12.5").unwrap(),
            amount_cents: Some(-1250),
            tx_type: "debit".into(),
            category_id: Some("alimentacao:restaurante".into()),
            category_source: "manual".into(),
            context: None,
            classifier_trace: None,
            payment_status: "settled".into(),
            source: "test".into(),
            actor_id: "test".into(),
            idempotency_key: "test".into(),
            metadata_json: Value::Object(Default::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            enrichment_attempted_at: None,
        }
    }

    /// A card purchase served to the web is bucketed under its bill's payment
    /// month (cash_month), not its posting month — the family cash-flow view.
    #[test]
    fn tx_rows_use_cash_month_for_card_purchases() {
        let mut card = sample_account("cc-1", "Card", "me");
        card.account_type = "credit".into();
        card.metadata_json = serde_json::json!({
            "billing_closing_day": "3",
            "billing_due_day": "10",
        });
        let mut rec = sample_record();
        rec.account_id = Some("cc-1".into());
        // 2026-04-28 (day 28 > closing 3) closes in the May cycle, due May 10.
        rec.transaction_date = NaiveDate::from_ymd_opt(2026, 4, 28).unwrap();

        let lookup = cash_month_lookup(&[card]);
        let rows = tx_rows_with_cash_month(&[rec], &lookup);
        assert_eq!(rows[0].month, "2026-05");
    }

    /// A non-card purchase keeps its posting month.
    #[test]
    fn tx_rows_keep_posting_month_for_non_card() {
        let checking = sample_account("acc-1", "Checking", "me");
        let rec = sample_record(); // dated 2026-03-15 on acc-1
        let lookup = cash_month_lookup(&[checking]);
        let rows = tx_rows_with_cash_month(&[rec], &lookup);
        assert_eq!(rows[0].month, "2026-03");
    }

    fn sample_account(account_id: &str, label: &str, owner: &str) -> AccountRecord {
        AccountRecord {
            account_id: account_id.into(),
            owner: owner.into(),
            account_type: "checking".into(),
            bank: "test".into(),
            label: label.into(),
            pluggy_account_id: None,
            pluggy_item_id: None,
            status: "active".into(),
            actor_id: "test".into(),
            idempotency_key: "test".into(),
            metadata_json: Value::Object(Default::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_forecast(forecast_id: &str, template_id: Option<&str>) -> ForecastRecord {
        ForecastRecord {
            forecast_id: forecast_id.into(),
            due_date: Some(NaiveDate::from_ymd_opt(2026, 4, 10).unwrap()),
            description: "Aluguel".into(),
            amount: Decimal::from_str("-100.00").unwrap(),
            category_id: Some("moradia:aluguel".into()),
            account_id: Some("acc-1".into()),
            status: "ativo".into(),
            recurrence: None,
            actor_id: "test".into(),
            idempotency_key: String::new(),
            metadata_json: Value::Object(Default::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            template_id: template_id.map(Into::into),
            realized_transaction_id: None,
            realized_at: None,
        }
    }

    #[test]
    fn tx_row_from_record_satisfies_camel_case_contract() {
        let row = TxRow::from_record(&sample_record());
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(value["id"], "tx-1");
        assert_eq!(value["accountId"], "acc-1");
        assert_eq!(value["postedAt"], "2026-03-15");
        assert_eq!(value["amount"], "-12.50");
        assert_eq!(value["rawDescription"], "RAW MERCHANT 123");
        assert_eq!(value["description"], "Almoço");
        assert_eq!(value["merchantName"], "Restaurante");
        assert_eq!(value["purpose"], "Trabalho");
        assert_eq!(value["categoryId"], "alimentacao:restaurante");
        assert_eq!(value["month"], "2026-03");
        assert_eq!(value["installmentMarker"], serde_json::Value::Null);
        // snake_case keys must NOT leak.
        assert!(value.get("account_id").is_none());
        assert!(value.get("raw_description").is_none());
        assert!(value.get("installment_marker").is_none());
    }

    // ── TxRow contract (camelCase + flag derivation) ──────────────────────

    #[test]
    fn tx_row_serialises_camel_case_with_flags() {
        let value = serde_json::to_value(TxRow::from_record(&sample_record())).unwrap();
        assert_eq!(value["id"], "tx-1");
        assert_eq!(value["accountId"], "acc-1");
        assert_eq!(value["postedAt"], "2026-03-15");
        assert_eq!(value["amount"], "-12.50");
        assert_eq!(value["rawDescription"], "RAW MERCHANT 123");
        assert_eq!(value["month"], "2026-03");
        assert_eq!(value["paymentStatus"], "settled");
        assert_eq!(value["installmentMarker"], serde_json::Value::Null);
        // sample_record has a concrete category → reviewed; not installment;
        // not under `assinaturas:` → not a subscription.
        assert_eq!(value["reviewed"], true);
        assert_eq!(value["isInstallment"], false);
        assert_eq!(value["isSubscription"], false);
        // snake_case keys must NOT leak.
        assert!(value.get("payment_status").is_none());
        assert!(value.get("is_installment").is_none());
    }

    #[test]
    fn tx_row_unreviewed_when_category_missing_or_sentinel() {
        let mut none_cat = sample_record();
        none_cat.category_id = None;
        assert!(!TxRow::from_record(&none_cat).reviewed);

        let mut sentinel = sample_record();
        sentinel.category_id = Some(REVIEW_PENDING_CATEGORY.into());
        assert!(!TxRow::from_record(&sentinel).reviewed);
    }

    #[test]
    fn tx_row_installment_and_subscription_flags() {
        let mut installment = sample_record();
        installment.payment_status = PAYMENT_STATUS_INSTALLMENT.into();
        assert!(TxRow::from_record(&installment).is_installment);

        let mut marked_installment = sample_record();
        marked_installment.raw_description = "LOJA SINTETICA 2/5".into();
        marked_installment.payment_status = "settled".into();
        let row = TxRow::from_record(&marked_installment);
        assert!(row.is_installment);
        assert_eq!(row.installment_marker.as_deref(), Some("2/5"));

        let mut subscription = sample_record();
        subscription.category_id = Some("assinaturas:streaming".into());
        let row = TxRow::from_record(&subscription);
        assert!(row.is_subscription);
        assert!(row.reviewed);
    }

    #[test]
    fn tx_row_normalizes_legacy_english_category_for_web() {
        let mut record = sample_record();
        record.category_id = Some("shopping".into());

        let row = TxRow::from_record(&record);

        assert_eq!(row.category_id.as_deref(), Some("compras"));
    }

    #[test]
    fn web_category_list_normalizes_legacy_english_ids() {
        assert_eq!(
            canonical_web_category_id("Eating out"),
            "alimentacao:restaurantes"
        );
        assert_eq!(
            canonical_web_category_id("Groceries"),
            "alimentacao:mercado"
        );
        assert_eq!(canonical_web_category_id("Shopping"), "compras");
        assert_eq!(
            canonical_web_category_id("airport-and-airlines"),
            "lazer:viagem"
        );
        assert_eq!(
            canonical_web_category_id("same-person-transfer"),
            "transfer-internal"
        );
        assert_eq!(canonical_web_category_id("unknown-english"), "outros");
        assert_eq!(
            canonical_web_category_id("saude:farmacia"),
            "saude:farmacia"
        );
    }

    // ── month window helpers ──────────────────────────────────────────────

    #[test]
    fn month_window_helpers_span_full_months() {
        let d = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        assert_eq!(
            first_of_month(d),
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
        );
        assert_eq!(
            last_of_month(d),
            NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()
        );
        // Crossing a year boundary backwards.
        let back = shift_months(NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(), -2);
        assert_eq!(back, NaiveDate::from_ymd_opt(2025, 11, 1).unwrap());
        // February end-of-month for a non-leap year.
        let feb = last_of_month(NaiveDate::from_ymd_opt(2026, 2, 5).unwrap());
        assert_eq!(feb, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());
    }

    // ── forecast kind / draggable derivation ──────────────────────────────

    #[test]
    fn forecast_kind_defaults_to_manual_without_template() {
        let kinds = HashMap::new();
        assert_eq!(forecast_kind(None, &kinds), "manual");
        assert!(kind_is_draggable("manual"));
    }

    #[test]
    fn forecast_kind_reads_linked_template_and_pins_recurring() {
        let mut kinds = HashMap::new();
        kinds.insert("tpl-inst".to_string(), "installment".to_string());
        kinds.insert("tpl-sub".to_string(), "subscription".to_string());
        kinds.insert("tpl-fixed".to_string(), "fixed".to_string());

        assert_eq!(forecast_kind(Some("tpl-inst"), &kinds), "installment");
        assert!(!kind_is_draggable("installment"));
        assert!(!kind_is_draggable("subscription"));
        assert!(kind_is_draggable("fixed"));
        // Unknown template id falls back to manual (still draggable).
        assert_eq!(forecast_kind(Some("missing"), &kinds), "manual");
    }

    #[test]
    fn forecast_with_kind_serialises_record_plus_computed_fields() {
        let kinds = HashMap::new();
        let enriched = ForecastWithKind::new(sample_forecast("f-1", None), &kinds);
        let value = enriched.to_json();
        // Existing snake_case record fields survive.
        assert_eq!(value["forecast_id"], "f-1");
        assert_eq!(value["amount"], "-100.00");
        // Computed planning fields added.
        assert_eq!(value["kind"], "manual");
        assert_eq!(value["draggable"], true);
    }

    #[test]
    fn account_row_serialises_fields() {
        let acc = sample_account("acc-1", "Conta Corrente", "alice");
        let value = serde_json::to_value(AccountRow::from_record(&acc)).unwrap();
        assert_eq!(value["id"], "acc-1");
        assert_eq!(value["label"], "Conta Corrente");
        assert_eq!(value["owner"], "alice");
    }

    #[test]
    fn account_row_falls_back_to_id_when_label_blank() {
        let acc = sample_account("acc-9", "   ", "bob");
        let row = AccountRow::from_record(&acc);
        assert_eq!(row.label, "acc-9");
    }

    #[test]
    fn events_response_uses_camel_case_failure() {
        let resp = EventsResponse {
            acked: vec!["w1".into()],
            failed: vec![EventFailure {
                write_id: "w2".into(),
                error: "boom".into(),
            }],
        };
        let value = serde_json::to_value(&resp).unwrap();
        assert_eq!(value["acked"][0], "w1");
        assert_eq!(value["failed"][0]["writeId"], "w2");
        assert_eq!(value["failed"][0]["error"], "boom");
    }

    // ── review patch mapping ───────────────────────────────────────────────

    #[test]
    fn review_patch_body_maps_camel_case_fields() {
        let body: ReviewPatchBody = serde_json::from_value(serde_json::json!({
            "description": "d",
            "merchantName": "m",
            "purpose": "p",
            "categoryId": "c"
        }))
        .unwrap();
        let patch = body.into_patch();
        assert_eq!(patch.description.as_deref(), Some("d"));
        assert_eq!(patch.merchant_name.as_deref(), Some("m"));
        assert_eq!(patch.purpose.as_deref(), Some("p"));
        assert_eq!(patch.category_id.as_deref(), Some("c"));
    }

    #[test]
    fn event_write_parses_full_body() {
        let body: EventsBody = serde_json::from_value(serde_json::json!({
            "writes": [
                { "writeId": "w1", "transactionId": "tx-1", "patch": { "merchantName": "Foo" } }
            ]
        }))
        .unwrap();
        assert_eq!(body.writes.len(), 1);
        assert_eq!(body.writes[0].write_id, "w1");
        assert_eq!(body.writes[0].transaction_id, "tx-1");
        assert_eq!(body.writes[0].patch.merchant_name.as_deref(), Some("Foo"));
    }

    // ── Decimal parse guard ────────────────────────────────────────────────

    #[test]
    fn decimal_parse_rejects_non_numeric() {
        let result = rust_decimal::Decimal::from_str("R$ 250,00");
        assert!(result.is_err(), "must reject locale-formatted strings");
    }

    #[test]
    fn decimal_parse_accepts_dot_notation() {
        let d = rust_decimal::Decimal::from_str("-250.00").unwrap();
        assert_eq!(d.to_string(), "-250.00");
    }

    #[test]
    fn parse_opt_date_handles_empty_and_valid() {
        assert!(parse_opt_date(None).unwrap().is_none());
        assert!(parse_opt_date(Some("")).unwrap().is_none());
        assert_eq!(
            parse_opt_date(Some("2026-01-02")).unwrap(),
            Some(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap())
        );
        assert!(parse_opt_date(Some("nope")).is_err());
    }

    // ── ForecastRecord serialization for audit diff ────────────────────────

    #[test]
    fn forecast_record_serializes_to_non_empty_diff() {
        let record = ForecastRecord {
            forecast_id: "fc-test-1".into(),
            due_date: Some(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()),
            description: "Internet 500Mbps".into(),
            amount: rust_decimal::Decimal::new(-11990, 2),
            category_id: Some("moradia:internet".into()),
            account_id: Some("conjunta".into()),
            status: "ativo".into(),
            recurrence: None,
            actor_id: "serve-dashboard".into(),
            idempotency_key: "test-key".into(),
            metadata_json: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            template_id: None,
            realized_transaction_id: None,
            realized_at: None,
        };
        let diff = serde_json::to_value(&record).expect("ForecastRecord must serialize");
        assert!(
            diff.as_object().is_some_and(|o| !o.is_empty()),
            "audit diff must contain forecast fields, got: {diff}"
        );
        assert_eq!(diff["forecast_id"], "fc-test-1");
        assert_eq!(diff["description"], "Internet 500Mbps");
        assert_eq!(diff["amount"], "-119.90");
    }

    // ── Behavioral: ApplyHumanReview against a real SQLite store ────────────

    async fn temp_store() -> (tempfile::TempDir, AppConfig, Box<dyn FinanceStore>) {
        let dir = tempfile::tempdir().unwrap();
        let config = AppConfig {
            local_db_path: Some(dir.path().join("test.db")),
            actor_id: "test-actor".into(),
            ..AppConfig::default()
        };
        let store = open_store(&config).await.unwrap();
        run_migrations(store.as_ref(), &config).await.unwrap();
        (dir, config, store)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_review_writes_persists_patch_and_isolates_failures() {
        let (_dir, config, store) = temp_store().await;
        let mut record = sample_record();
        // Start with no human description so the patch is observable.
        record.description = None;
        record.merchant_name = None;
        store.upsert_transactions(&[record]).await.unwrap();

        let writes = vec![
            ReviewWrite {
                write_id: "w-ok".into(),
                transaction_id: "tx-1".into(),
                patch: HumanReviewPatch {
                    description: Some("Almoço de trabalho".into()),
                    merchant_name: Some("Bistrô".into()),
                    purpose: None,
                    category_id: None,
                },
            },
            ReviewWrite {
                // Unknown transaction → must fail without aborting the batch.
                write_id: "w-missing".into(),
                transaction_id: "tx-does-not-exist".into(),
                patch: HumanReviewPatch {
                    description: Some("x".into()),
                    merchant_name: None,
                    purpose: None,
                    category_id: None,
                },
            },
        ];

        let outcomes = apply_review_writes(store.as_ref(), &config, writes).await;
        assert_eq!(outcomes.len(), 2);
        assert!(matches!(
            &outcomes[0],
            ReviewWriteOutcome::Acked(id) if id == "w-ok"
        ));
        assert!(matches!(
            &outcomes[1],
            ReviewWriteOutcome::Failed { write_id, .. } if write_id == "w-missing"
        ));

        let stored = store.transaction_by_id("tx-1").await.unwrap().unwrap();
        assert_eq!(stored.description.as_deref(), Some("Almoço de trabalho"));
        assert_eq!(stored.merchant_name.as_deref(), Some("Bistrô"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn review_queue_filters_by_month() {
        let (_dir, _config, store) = temp_store().await;
        let mut record = sample_record();
        record.description = None;
        record.merchant_name = None;
        store.upsert_transactions(&[record]).await.unwrap();

        // sample_record is dated 2026-03; a non-matching month yields nothing.
        let other = load_review_queue(
            store.as_ref(),
            ReviewQueueParams {
                filters: ReviewFilters {
                    month: Some("2020-01".into()),
                    ..ReviewFilters::default()
                },
                include_reviewed: false,
                limit: 50,
            },
        )
        .await
        .unwrap();
        assert!(other.iter().all(|r| r.transaction_id != "tx-1"));

        let matching = load_review_queue(
            store.as_ref(),
            ReviewQueueParams {
                filters: ReviewFilters {
                    month: Some("2026-03".into()),
                    ..ReviewFilters::default()
                },
                include_reviewed: false,
                limit: 50,
            },
        )
        .await
        .unwrap();
        assert!(matching.iter().any(|r| r.transaction_id == "tx-1"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transactions_window_excludes_internal_categories() {
        let (_dir, _config, store) = temp_store().await;
        let today = Utc::now().date_naive();

        let mut external = sample_record();
        external.transaction_id = "tx-external".into();
        external.transaction_date = today;
        external.category_id = Some("alimentacao:mercado".into());

        let mut internal = sample_record();
        internal.transaction_id = "tx-internal".into();
        internal.transaction_date = today;
        internal.category_id = Some("credit-card-payment".into());

        store
            .upsert_transactions(&[external, internal])
            .await
            .unwrap();

        let result = load_transactions_window(
            store.as_ref(),
            TransactionsWindowParams {
                months_back: 0,
                months_ahead: 0,
                include_reviewed: true,
                limit: 50,
                offset: 0,
            },
        )
        .await
        .unwrap();

        let ids: BTreeSet<_> = result.rows.iter().map(|row| row.id.as_str()).collect();
        assert!(ids.contains("tx-external"));
        assert!(!ids.contains("tx-internal"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transactions_window_dedupes_ofx_shadowed_by_pluggy() {
        let (_dir, _config, store) = temp_store().await;
        let today = Utc::now().date_naive();

        let mut pluggy = sample_record();
        pluggy.transaction_id = "tx-pluggy".into();
        pluggy.transaction_date = today;
        pluggy.raw_description = "Compra Exemplo".into();
        pluggy.amount = Decimal::new(-11603, 2);
        pluggy.source = "pluggy".into();
        pluggy.category_id = Some("alimentacao:mercado".into());

        let mut ofx = pluggy.clone();
        ofx.transaction_id = "tx-ofx-duplicate".into();
        ofx.source = "ofx".into();

        let mut ofx_unique = pluggy.clone();
        ofx_unique.transaction_id = "tx-ofx-unique".into();
        ofx_unique.raw_description = "Compra Unica".into();
        ofx_unique.source = "ofx".into();

        store
            .upsert_transactions(&[pluggy, ofx, ofx_unique])
            .await
            .unwrap();

        let result = load_transactions_window(
            store.as_ref(),
            TransactionsWindowParams {
                months_back: 0,
                months_ahead: 0,
                include_reviewed: true,
                limit: 50,
                offset: 0,
            },
        )
        .await
        .unwrap();

        let ids: BTreeSet<_> = result.rows.iter().map(|row| row.id.as_str()).collect();
        assert!(ids.contains("tx-pluggy"));
        assert!(ids.contains("tx-ofx-unique"));
        assert!(!ids.contains("tx-ofx-duplicate"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transactions_window_includes_prior_posted_card_purchase_for_current_cash_month() {
        let (_dir, _config, store) = temp_store().await;
        let today = Utc::now().date_naive();
        let current_month = first_of_month(today);
        let previous_month = shift_months(current_month, -1);
        let current_ref = current_month.format("%Y-%m").to_string();

        let mut card = sample_account("cc-1", "Card", "me");
        card.account_type = "credit".into();
        card.metadata_json = serde_json::json!({
            "billing_closing_day": "3",
            "billing_due_day": "10",
        });
        store.upsert_accounts(&[card]).await.unwrap();

        let mut card_purchase = sample_record();
        card_purchase.transaction_id = "tx-card-prior-posted".into();
        card_purchase.account_id = Some("cc-1".into());
        card_purchase.transaction_date =
            NaiveDate::from_ymd_opt(previous_month.year(), previous_month.month(), 28).unwrap();
        card_purchase.category_id = Some("educacao:cursos".into());

        let mut prior_checking = sample_record();
        prior_checking.transaction_id = "tx-prior-checking".into();
        prior_checking.transaction_date = card_purchase.transaction_date;
        prior_checking.category_id = Some("alimentacao:mercado".into());

        store
            .upsert_transactions(&[card_purchase, prior_checking])
            .await
            .unwrap();

        let result = load_transactions_window(
            store.as_ref(),
            TransactionsWindowParams {
                months_back: 0,
                months_ahead: 0,
                include_reviewed: true,
                limit: 50,
                offset: 0,
            },
        )
        .await
        .unwrap();

        let ids: BTreeSet<_> = result.rows.iter().map(|row| row.id.as_str()).collect();
        assert!(ids.contains("tx-card-prior-posted"));
        assert!(!ids.contains("tx-prior-checking"));
        assert_eq!(
            result
                .rows
                .iter()
                .find(|row| row.id == "tx-card-prior-posted")
                .map(|row| row.month.as_str()),
            Some(current_ref.as_str())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn web_category_list_filters_review_sentinel_and_internal_aliases() {
        let (_dir, _config, store) = temp_store().await;
        let today = Utc::now().date_naive();

        let mut legacy = sample_record();
        legacy.transaction_id = "tx-legacy".into();
        legacy.transaction_date = today;
        legacy.category_id = Some("airport-and-airlines".into());

        let mut review = sample_record();
        review.transaction_id = "tx-review".into();
        review.transaction_date = today;
        review.category_id = Some(REVIEW_PENDING_CATEGORY.into());

        let mut internal_alias = sample_record();
        internal_alias.transaction_id = "tx-internal-alias".into();
        internal_alias.transaction_date = today;
        internal_alias.category_id = Some("same-person-transfer".into());

        store
            .upsert_transactions(&[legacy, review, internal_alias])
            .await
            .unwrap();

        let ids = list_web_category_ids(store.as_ref()).await.unwrap();

        assert!(ids.contains(&"lazer:viagem".to_string()));
        assert!(!ids.contains(&REVIEW_PENDING_CATEGORY.to_string()));
        assert!(!ids.contains(&"transfer-internal".to_string()));
        assert!(!ids.contains(&"same-person-transfer".to_string()));
    }

    // ── Behavioral: MoveForecast against a real SQLite store ───────────────

    fn installment_template(template_id: &str) -> ForecastTemplateRecord {
        ForecastTemplateRecord {
            template_id: template_id.into(),
            kind: "installment".into(),
            description: "Parcelamento".into(),
            merchant_pattern: None,
            category_id: Some("compras:eletronicos".into()),
            account_id: Some("acc-1".into()),
            amount: Decimal::from_str("-50.00").unwrap(),
            amount_lower: None,
            amount_upper: None,
            cadence: "monthly".into(),
            next_due_day: Some(10),
            start_date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            end_date: None,
            remaining_count: Some(3),
            source: "manual".into(),
            confidence: None,
            status: "ativo".into(),
            metadata_json: Value::Object(Default::default()),
            actor_id: "test".into(),
            idempotency_key: "tpl-idem".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn move_forecast_reschedules_manual_in_place() {
        let (_dir, _config, store) = temp_store().await;
        let mut forecast = sample_forecast("f-manual", None);
        ensure_forecast_idempotency(&mut forecast).unwrap();
        store.upsert_forecasts(&[forecast]).await.unwrap();

        let new_due = first_of_month(shift_months(Utc::now().date_naive(), 1));
        let outcome = move_forecast(store.as_ref(), "f-manual", new_due)
            .await
            .unwrap();
        match outcome {
            MoveForecastResult::Moved {
                forecast_id,
                status,
            } => {
                assert_eq!(forecast_id, "f-manual");
                assert_eq!(status, "ativo");
            }
            _ => panic!("expected Moved"),
        }

        // Persisted in place under the same id with the new due date.
        let stored = store.get_forecast("f-manual").await.unwrap().unwrap();
        assert_eq!(stored.due_date, Some(new_due));
        // No duplicate row was created.
        let all = store.list_forecasts(None, None, None).await.unwrap();
        assert_eq!(
            all.iter().filter(|f| f.forecast_id == "f-manual").count(),
            1
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn move_forecast_rejects_installment_kind() {
        let (_dir, _config, store) = temp_store().await;
        store
            .upsert_forecast_templates(&[installment_template("tpl-inst")])
            .await
            .unwrap();
        let mut forecast = sample_forecast("f-inst", Some("tpl-inst"));
        ensure_forecast_idempotency(&mut forecast).unwrap();
        let original_due = forecast.due_date;
        store.upsert_forecasts(&[forecast]).await.unwrap();

        let outcome = move_forecast(
            store.as_ref(),
            "f-inst",
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, MoveForecastResult::NotMovable));

        // Untouched: due date unchanged.
        let stored = store.get_forecast("f-inst").await.unwrap().unwrap();
        assert_eq!(stored.due_date, original_due);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn move_forecast_returns_not_found_for_unknown_id() {
        let (_dir, _config, store) = temp_store().await;
        let outcome = move_forecast(
            store.as_ref(),
            "nope",
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, MoveForecastResult::NotFound));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn move_forecast_rejects_past_month() {
        let (_dir, _config, store) = temp_store().await;
        let mut forecast = sample_forecast("f-manual", None);
        ensure_forecast_idempotency(&mut forecast).unwrap();
        store.upsert_forecasts(&[forecast]).await.unwrap();

        // Move to a date that is definitely in the past (year 2020).
        let past_due = NaiveDate::from_ymd_opt(2020, 3, 15).unwrap();
        let outcome = move_forecast(store.as_ref(), "f-manual", past_due)
            .await
            .unwrap();
        assert!(matches!(outcome, MoveForecastResult::PastMonth));

        // Due date unchanged.
        let stored = store.get_forecast("f-manual").await.unwrap().unwrap();
        assert_eq!(
            stored.due_date,
            Some(NaiveDate::from_ymd_opt(2026, 4, 10).unwrap())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn move_forecast_allows_current_and_future_month() {
        let (_dir, _config, store) = temp_store().await;
        let mut forecast = sample_forecast("f-manual", None);
        ensure_forecast_idempotency(&mut forecast).unwrap();
        let original_due = forecast.due_date;
        store.upsert_forecasts(&[forecast]).await.unwrap();

        // Move to the current month (first-of-month from now).
        let current_month = first_of_month(Utc::now().date_naive());
        let outcome = move_forecast(store.as_ref(), "f-manual", current_month)
            .await
            .unwrap();
        assert!(matches!(outcome, MoveForecastResult::Moved { .. }));

        // Due date updated.
        let stored = store.get_forecast("f-manual").await.unwrap().unwrap();
        assert_eq!(stored.due_date, Some(current_month));
        assert_ne!(stored.due_date, original_due);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn move_forecast_allows_far_future_month() {
        let (_dir, _config, store) = temp_store().await;
        let mut forecast = sample_forecast("f-manual", None);
        ensure_forecast_idempotency(&mut forecast).unwrap();
        store.upsert_forecasts(&[forecast]).await.unwrap();

        // Move to a date far in the future (year 2099).
        let future_due = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();
        let outcome = move_forecast(store.as_ref(), "f-manual", future_due)
            .await
            .unwrap();
        assert!(matches!(outcome, MoveForecastResult::Moved { .. }));

        let stored = store.get_forecast("f-manual").await.unwrap().unwrap();
        assert_eq!(stored.due_date, Some(future_due));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_forecasts_enriched_attaches_kind_and_draggable() {
        let (_dir, _config, store) = temp_store().await;
        store
            .upsert_forecast_templates(&[installment_template("tpl-inst")])
            .await
            .unwrap();
        let mut manual = sample_forecast("f-manual", None);
        ensure_forecast_idempotency(&mut manual).unwrap();
        let mut inst = sample_forecast("f-inst", Some("tpl-inst"));
        ensure_forecast_idempotency(&mut inst).unwrap();
        store.upsert_forecasts(&[manual, inst]).await.unwrap();

        let rows = list_forecasts_enriched(store.as_ref(), None, None, None)
            .await
            .unwrap();
        let by_id = |id: &str| rows.iter().find(|r| r.record.forecast_id == id).unwrap();
        assert_eq!(by_id("f-manual").kind, "manual");
        assert!(by_id("f-manual").draggable);
        assert_eq!(by_id("f-inst").kind, "installment");
        assert!(!by_id("f-inst").draggable);
    }
}
