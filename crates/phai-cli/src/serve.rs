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
    extract::{Query, RawQuery, State},
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
use phai_core::{parse_installment_description, AppConfig, BackendKind, ConfigPaths};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch, RwLock};
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
use crate::forecast_cmd::{amount_matches, materialise_template_forecasts};
use crate::serve_cache::ReadCache;
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
    /// Binary version. The web app keys its seed-freshness stamps by this so
    /// an upgraded binary always forces a full reseed of the client store.
    version: String,
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
        version: env!("CARGO_PKG_VERSION").to_string(),
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
    /// Accounts joined with their latest balance snapshot, for `/api/accounts`.
    AccountsWithBalance {
        resp: oneshot::Sender<Result<Vec<AccountRow>>>,
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
    /// Re-amount an existing forecast in place (`POST /api/forecast` with
    /// `forecast_id` — the web planner's envelope upsert).
    PatchForecast {
        forecast_id: String,
        patch: ForecastPatch,
        resp: oneshot::Sender<Result<Option<String>>>,
    },
    /// Soft-delete a manual forecast (`POST /api/forecast/delete`).
    DeleteForecast {
        forecast_id: String,
        resp: oneshot::Sender<Result<DeleteForecastResult>>,
    },
    /// Manually mark a manual forecast as realized by linking a real synced
    /// transaction (`POST /api/forecast/settle`).
    SettleForecast {
        forecast_id: String,
        transaction_id: String,
        resp: oneshot::Sender<Result<SettleForecastResult>>,
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
        // Two-stage dispatch: read-only requests first; anything it hands
        // back is a write and goes through the mutation handler.
        if let Some(req) = try_handle_store_query(store.as_ref(), &config, req).await {
            handle_store_mutation(store.as_ref(), &config, req).await;
        }
    }
}

/// Serve the read-only store requests; hands back writes untouched.
async fn try_handle_store_query(
    store: &dyn FinanceStore,
    config: &AppConfig,
    req: StoreRequest,
) -> Option<StoreRequest> {
    match req {
        StoreRequest::GetChartData {
            months_back,
            months_ahead,
            resp,
        } => {
            let result = build_chart_data(store, months_back, months_ahead, true).await;
            let _ = resp.send(result);
        }
        StoreRequest::ListForecastTemplates { kind, status, resp } => {
            let result = store
                .list_forecast_templates(kind.as_deref(), status.as_deref())
                .await;
            let _ = resp.send(result);
        }
        StoreRequest::ListCategoryIds { resp } => {
            let result = list_web_category_ids(store).await;
            let _ = resp.send(result);
        }
        StoreRequest::GetAccounts { resp } => {
            let result = store.get_accounts().await;
            let _ = resp.send(result);
        }
        StoreRequest::AccountsWithBalance { resp } => {
            let result = build_accounts_api(store, &config.account_labels).await;
            let _ = resp.send(result);
        }
        StoreRequest::Cards { month, resp } => {
            let result = build_cards_api(store, month.as_deref()).await;
            let _ = resp.send(result);
        }
        StoreRequest::ReviewQueue { params, resp } => {
            let result = load_review_queue(store, params).await;
            let _ = resp.send(result);
        }
        StoreRequest::TransactionsWindow { params, resp } => {
            let result = load_transactions_window(store, params, &config.locked_categories).await;
            let _ = resp.send(result);
        }
        StoreRequest::ListForecastsEnriched {
            status,
            from,
            until,
            resp,
        } => {
            let result = list_forecasts_enriched(store, status.as_deref(), from, until).await;
            let _ = resp.send(result);
        }
        write => return Some(write),
    }
    None
}

/// Serve the store requests that write (template decisions, forecast
/// upserts/moves/patches, human review). Query requests never reach here —
/// `try_handle_store_query` consumes them.
async fn handle_store_mutation(store: &dyn FinanceStore, config: &AppConfig, req: StoreRequest) {
    match req {
        StoreRequest::AcceptTemplate {
            template_id,
            materialize_months,
            resp,
        } => {
            let result = handle_accept_template(store, &template_id, materialize_months).await;
            let _ = resp.send(result);
        }
        StoreRequest::DismissTemplate { template_id, resp } => {
            let result = handle_dismiss_template(store, &template_id).await;
            let _ = resp.send(result);
        }
        StoreRequest::UpsertForecast { record, resp } => {
            let result = upsert_forecast(store, *record).await;
            let _ = resp.send(result);
        }
        StoreRequest::MoveForecast {
            forecast_id,
            due_date,
            resp,
        } => {
            let result = move_forecast(store, &forecast_id, due_date).await;
            let _ = resp.send(result);
        }
        StoreRequest::PatchForecast {
            forecast_id,
            patch,
            resp,
        } => {
            let result = patch_forecast(store, &forecast_id, patch).await;
            let _ = resp.send(result);
        }
        StoreRequest::DeleteForecast { forecast_id, resp } => {
            let result = delete_forecast(store, &forecast_id).await;
            let _ = resp.send(result);
        }
        StoreRequest::SettleForecast {
            forecast_id,
            transaction_id,
            resp,
        } => {
            let result = settle_forecast(store, &forecast_id, &transaction_id).await;
            let _ = resp.send(result);
        }
        StoreRequest::ApplyHumanReview { writes, resp } => {
            let outcomes = apply_review_writes(store, config, writes).await;
            let _ = resp.send(outcomes);
        }
        _ => debug_assert!(false, "query request leaked past try_handle_store_query"),
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

/// Whether a transaction's category is on the configured fixed/locked list.
/// A list entry matches the exact `parent:sub` id or the `parent` (locking the
/// whole parent category).
fn category_is_locked(category_id: Option<&str>, locked: &[String]) -> bool {
    let Some(cat) = category_id else {
        return false;
    };
    let parent = cat.split(':').next().unwrap_or(cat);
    locked.iter().any(|e| e == cat || e == parent)
}

/// Load transactions whose cash month is within
/// `[now - months_back, now + months_ahead]`, optionally restricted to rows
/// still pending review. Returns a page of up to `limit` rows starting at
/// `offset`, plus the total matching count so the client can paginate.
async fn load_transactions_window(
    store: &dyn FinanceStore,
    params: TransactionsWindowParams,
    locked_categories: &[String],
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
    // Per-transaction tier overrides (ADR-0032) merged onto the seed rows.
    let tier_overrides: std::collections::HashMap<String, String> = store
        .commitment_tier_overrides()
        .await
        .context("commitment_tier_overrides")?
        .into_iter()
        .collect();
    let mut tx_rows: Vec<TxRow> = tx_rows_with_cash_month(&rows, &lookup)
        .into_iter()
        .filter(|row| {
            row.month.as_str() >= cash_from_ref.as_str()
                && row.month.as_str() <= cash_until_ref.as_str()
        })
        .collect();
    for row in &mut tx_rows {
        // An explicit per-transaction override wins; otherwise a category on the
        // configured fixed list is served as `locked` so it drops out of planning
        // for past and future rows alike (ADR-0030/0032).
        row.commitment_tier = tier_overrides.get(&row.id).cloned().or_else(|| {
            category_is_locked(row.category_id.as_deref(), locked_categories)
                .then(|| "locked".to_string())
        });
    }
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

enum DeleteForecastResult {
    Deleted { forecast_id: String, status: String },
    NotFound,
    NotManual,
}

enum SettleForecastResult {
    Settled { forecast_id: String, status: String },
    NotFound,
    NotManual,
    TransactionNotFound,
    AmountMismatch,
    SignMismatch,
}

fn manual_forecast_mut(record: &mut ForecastRecord) -> Option<&mut ForecastRecord> {
    (record.template_id.is_none()).then_some(record)
}

fn forecast_metadata_obj(record: &mut ForecastRecord) -> &mut serde_json::Map<String, Value> {
    if !record.metadata_json.is_object() {
        record.metadata_json = json!({});
    }
    record
        .metadata_json
        .as_object_mut()
        .expect("forecast metadata_json must be an object")
}

fn preserve_predicted_amount(record: &mut ForecastRecord) {
    let predicted = record.amount.to_string();
    let meta = forecast_metadata_obj(record);
    meta.entry("predicted_amount".to_string())
        .or_insert(Value::String(predicted));
}

fn stamp_realization_metadata(record: &mut ForecastRecord, tx: &TransactionRecord, source: &str) {
    preserve_predicted_amount(record);
    let actual = tx.amount.to_string();
    let predicted = forecast_metadata_obj(record)
        .get("predicted_amount")
        .and_then(Value::as_str)
        .unwrap_or(actual.as_str())
        .to_string();
    let variance = (tx.amount - Decimal::from_str(&predicted).unwrap_or(record.amount)).to_string();
    let meta = forecast_metadata_obj(record);
    meta.insert(
        "ui_role".to_string(),
        Value::String("planned_transaction".to_string()),
    );
    meta.insert("realized_amount".to_string(), Value::String(actual));
    meta.insert(
        "realized_transaction_date".to_string(),
        Value::String(tx.transaction_date.to_string()),
    );
    meta.insert(
        "realized_transaction_description".to_string(),
        Value::String(tx.display_description().to_string()),
    );
    meta.insert(
        "realization_source".to_string(),
        Value::String(source.to_string()),
    );
    meta.insert("amount_variance".to_string(), Value::String(variance));
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

/// Field updates for an existing forecast; `None` keeps the stored value.
struct ForecastPatch {
    amount: Decimal,
    due_date: Option<NaiveDate>,
    description: Option<String>,
    category_id: Option<String>,
    account_id: Option<String>,
}

/// Re-amount (and optionally re-date/relabel) an existing forecast in place,
/// preserving provenance (template link, metadata, status, creator). This is
/// the "upsert" half of the web planner's envelope writes: the budget total
/// for a (category, month) must replace the old envelope, never stack a new
/// forecast on top of it. Returns `None` when the id is unknown.
async fn patch_forecast(
    store: &dyn FinanceStore,
    forecast_id: &str,
    patch: ForecastPatch,
) -> Result<Option<String>> {
    let Some(mut record) = store
        .get_forecast(forecast_id)
        .await
        .context("get_forecast")?
    else {
        return Ok(None);
    };
    record.amount = patch.amount;
    if let Some(due) = patch.due_date {
        record.due_date = Some(due);
    }
    if let Some(description) = patch.description {
        record.description = description;
    }
    if let Some(category_id) = patch.category_id {
        record.category_id = Some(category_id);
    }
    if let Some(account_id) = patch.account_id {
        record.account_id = Some(account_id);
    }
    record.updated_at = Utc::now();
    // The key derives from (actor, description, due_date) — recompute like
    // move_forecast so a re-dated envelope stays deduplicatable.
    record.idempotency_key = String::new();
    ensure_forecast_idempotency(&mut record).context("idempotency")?;
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
        action: "upsert".into(),
        actor_id: SERVE_ACTOR_ID.into(),
        event_timestamp: Utc::now(),
        idempotency_key: Uuid::now_v7().to_string(),
        diff_json: diff,
    };
    store
        .insert_audit_events(&[event])
        .await
        .context("audit patch_forecast")?;
    Ok(Some(forecast_id.to_string()))
}

async fn delete_forecast(
    store: &dyn FinanceStore,
    forecast_id: &str,
) -> Result<DeleteForecastResult> {
    let Some(mut record) = store
        .get_forecast(forecast_id)
        .await
        .context("get_forecast")?
    else {
        return Ok(DeleteForecastResult::NotFound);
    };
    if manual_forecast_mut(&mut record).is_none() {
        return Ok(DeleteForecastResult::NotManual);
    }
    record.status = "descartado".to_string();
    record.updated_at = Utc::now();
    let discarded_at = record.updated_at.to_rfc3339();
    let meta = forecast_metadata_obj(&mut record);
    meta.insert("discarded_at".to_string(), Value::String(discarded_at));
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
        action: "discard".into(),
        actor_id: SERVE_ACTOR_ID.into(),
        event_timestamp: Utc::now(),
        idempotency_key: Uuid::now_v7().to_string(),
        diff_json: diff,
    };
    store
        .insert_audit_events(&[event])
        .await
        .context("audit delete_forecast")?;
    Ok(DeleteForecastResult::Deleted {
        forecast_id: forecast_id.to_string(),
        status,
    })
}

async fn settle_forecast(
    store: &dyn FinanceStore,
    forecast_id: &str,
    transaction_id: &str,
) -> Result<SettleForecastResult> {
    let Some(mut record) = store
        .get_forecast(forecast_id)
        .await
        .context("get_forecast")?
    else {
        return Ok(SettleForecastResult::NotFound);
    };
    if manual_forecast_mut(&mut record).is_none() {
        return Ok(SettleForecastResult::NotManual);
    }
    let Some(tx) = store
        .transaction_by_id(transaction_id)
        .await
        .context("transaction_by_id")?
    else {
        return Ok(SettleForecastResult::TransactionNotFound);
    };
    if (record.amount > Decimal::ZERO) != (tx.amount > Decimal::ZERO) {
        return Ok(SettleForecastResult::SignMismatch);
    }
    if !amount_matches(record.amount, tx.amount) {
        return Ok(SettleForecastResult::AmountMismatch);
    }
    if record.account_id.is_none() {
        record.account_id = tx.account_id.clone();
    }
    stamp_realization_metadata(&mut record, &tx, "manual");
    record.amount = tx.amount;
    record.status = "realizado".to_string();
    record.realized_transaction_id = Some(tx.transaction_id.clone());
    record.realized_at = Some(Utc::now());
    record.updated_at = Utc::now();
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
        action: "settle".into(),
        actor_id: SERVE_ACTOR_ID.into(),
        event_timestamp: Utc::now(),
        idempotency_key: Uuid::now_v7().to_string(),
        diff_json: diff,
    };
    store
        .insert_audit_events(&[event])
        .await
        .context("audit settle_forecast")?;
    Ok(SettleForecastResult::Settled {
        forecast_id: forecast_id.to_string(),
        status,
    })
}

async fn upsert_forecast(store: &dyn FinanceStore, mut record: ForecastRecord) -> Result<String> {
    let actor_id = record.actor_id.clone();
    let is_create = record.forecast_id.is_empty();
    ensure_forecast_idempotency(&mut record).context("idempotency")?;
    // Dedup creates: the web sync queue's flush guard is per-mount, so two
    // tabs/mounts (or a retry) can fire the same create twice. Without this the
    // MERGE keys on forecast_id (freshly generated each call) and stacks a
    // duplicate row. Return the existing forecast instead.
    if is_create {
        if let Some(existing) = store
            .find_forecast_by_idempotency_key(&record.idempotency_key)
            .await
            .context("find_forecast_by_idempotency_key")?
        {
            return Ok(existing.forecast_id);
        }
        record.forecast_id = Uuid::now_v7().to_string();
    }
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
    /// Per-transaction commitment-tier override (ADR-0032); `None` = derived.
    commitment_tier: Option<String>,
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
            commitment_tier: None,
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

/// Account summary for the accounts picker + per-account balance display.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountRow {
    id: String,
    label: String,
    owner: String,
    /// `checking`, `card`, … — lets the UI show only checking-account balances.
    account_type: String,
    /// Latest known balance (decimal string) from the most recent snapshot;
    /// null when no snapshot exists yet.
    balance: Option<String>,
}

impl AccountRow {
    fn from_record(account: &AccountRecord, balance: Option<&Decimal>) -> Self {
        let label = if account.label.trim().is_empty() {
            account.account_id.clone()
        } else {
            account.label.clone()
        };
        Self {
            id: account.account_id.clone(),
            label,
            owner: account.owner.clone(),
            account_type: account.account_type.clone(),
            balance: balance.map(|b| b.to_string()),
        }
    }
}

#[derive(Serialize)]
struct AccountsResponse {
    rows: Vec<AccountRow>,
}

/// Accounts joined with their latest balance snapshot (per-account checking
/// balance), so the UI can show each account, not just the consolidated total.
async fn build_accounts_api(
    store: &dyn FinanceStore,
    account_labels: &std::collections::HashMap<String, String>,
) -> Result<Vec<AccountRow>> {
    let accounts = store.get_accounts().await.context("get_accounts")?;
    let balances: std::collections::HashMap<String, Decimal> = store
        .latest_account_snapshots()
        .await
        .context("latest_account_snapshots")?
        .into_iter()
        .filter_map(|s| s.balance.map(|b| (s.account_id, b)))
        .collect();
    Ok(accounts
        .iter()
        .map(|a| {
            let mut row = AccountRow::from_record(a, balances.get(&a.account_id));
            // A configured friendly name overrides Pluggy's raw (often duplicate)
            // bank label so household accounts are distinguishable.
            if let Some(custom) = account_labels.get(&a.account_id) {
                row.label = custom.clone();
            }
            row
        })
        .collect())
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
    /// Commitment-tier override (ADR-0032): "locked"|"cancellable"|"variable";
    /// an empty string clears the override back to the derived tier.
    commitment_tier: Option<String>,
}

impl ReviewPatchBody {
    fn into_patch(self) -> HumanReviewPatch {
        HumanReviewPatch {
            description: self.description,
            merchant_name: self.merchant_name,
            purpose: self.purpose,
            category_id: self.category_id,
            commitment_tier: self.commitment_tier,
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
    /// When set, the request re-amounts that existing forecast in place
    /// (envelope upsert from the web planner) instead of creating a new one.
    forecast_id: Option<String>,
    #[serde(default)]
    description: String,
    amount: String,
    due_date: Option<String>,
    category_id: Option<String>,
    account_id: Option<String>,
    ui_role: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveForecastBody {
    forecast_id: String,
    due_date: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteForecastBody {
    forecast_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettleForecastBody {
    forecast_id: String,
    transaction_id: String,
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

/// Shared axum state for the `/api` handlers: the (hot-swappable) store-actor
/// sender plus the in-memory read cache.
#[derive(Clone)]
struct BridgeState {
    tx: Arc<RwLock<mpsc::Sender<StoreRequest>>>,
    cache: ReadCache,
    /// Pushes a freshly activated config to the store actor, which restarts
    /// against the new backend without restarting the process.
    config_tx: Arc<watch::Sender<AppConfig>>,
    /// On-disk config location, for persisting the activated config + key.
    paths: Arc<ConfigPaths>,
    /// Current activation state, surfaced by `/api/status` and updated by
    /// `/api/activate`.
    activation: Arc<RwLock<ActivationStatus>>,
}

/// What `/api/status` reports to the web app so it can choose between the
/// onboarding screen and the dashboard.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivationStatus {
    activated: bool,
    label: Option<String>,
    project_id: Option<String>,
    dataset_id: Option<String>,
    /// Whether the Pluggy "sync" button is available (a pluggy config is set).
    sync_available: bool,
}

/// Body of `POST /api/activate`: the pasted/attached invite plus its passphrase.
#[derive(Deserialize)]
struct ActivateRequest {
    token: String,
    passphrase: String,
}

type Store = State<BridgeState>;

/// Build a JSON error response with the given status.
fn error_response(status: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

/// A cached `200 application/json` response built from raw bytes.
fn cached_json_response(body: Arc<[u8]>) -> axum::response::Response {
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        body.to_vec(),
    )
        .into_response()
}

/// Serve a read either from cache or by computing it once. On a fresh miss the
/// closure runs, and only a successful body (`Ok`) is serialized, cached under
/// `key`, and returned. Errors are returned verbatim and never cached.
async fn cached_read<F, Fut, T>(
    cache: &ReadCache,
    key: String,
    compute: F,
) -> axum::response::Response
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, axum::response::Response>>,
    T: Serialize,
{
    if let Some(body) = cache.get(&key) {
        return cached_json_response(body);
    }
    match compute().await {
        Ok(value) => match serde_json::to_vec(&value) {
            Ok(bytes) => {
                let body: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
                cache.store(key, body.clone());
                cached_json_response(body)
            }
            Err(e) => internal_error(e),
        },
        Err(resp) => resp,
    }
}

/// Log an internal failure server-side and return a generic 500. The error
/// itself (anyhow context chains leak the DB path, BigQuery project/dataset and
/// raw driver text) must never reach the client.
fn internal_error(e: impl std::fmt::Display) -> axum::response::Response {
    eprintln!("[phai serve] erro interno: {e}");
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "erro interno")
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
    State(state): Store,
    RawQuery(raw_query): RawQuery,
    Query(q): Query<ReviewQueueQuery>,
) -> impl IntoResponse {
    let key = ReadCache::key("/api/review-queue", raw_query.as_deref());
    cached_read(&state.cache, key, || async move {
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
        let tx = clone_actor_tx(&state.tx).await;
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
            return Err(actor_unavailable());
        }
        match resp_rx.await {
            Ok(Ok(rows)) => {
                let lookup = cash_month_lookup(&accounts);
                Ok(ReviewQueueResponse {
                    rows: tx_rows_with_cash_month(&rows, &lookup),
                })
            }
            Ok(Err(e)) => Err(internal_error(e)),
            Err(_) => Err(actor_silent()),
        }
    })
    .await
}

async fn get_transactions(
    State(state): Store,
    RawQuery(raw_query): RawQuery,
    Query(q): Query<TransactionsQuery>,
) -> impl IntoResponse {
    let key = ReadCache::key("/api/transactions", raw_query.as_deref());
    cached_read(&state.cache, key, || async move {
        let params = TransactionsWindowParams {
            months_back: q.months_back.unwrap_or(DEFAULT_TRANSACTIONS_MONTHS_BACK),
            months_ahead: q.months_ahead.unwrap_or(0),
            include_reviewed: q.include_reviewed.unwrap_or(true),
            limit: q.limit.unwrap_or(DEFAULT_TRANSACTIONS_LIMIT),
            offset: q.offset.unwrap_or(0),
        };
        let tx = clone_actor_tx(&state.tx).await;
        let (resp_tx, resp_rx) = oneshot::channel();
        if tx
            .send(StoreRequest::TransactionsWindow {
                params,
                resp: resp_tx,
            })
            .await
            .is_err()
        {
            return Err(actor_unavailable());
        }
        match resp_rx.await {
            Ok(Ok(result)) => Ok(TransactionsResponse {
                rows: result.rows,
                total: result.total,
                offset: result.offset,
                has_more: result.has_more,
            }),
            Ok(Err(e)) => Err(internal_error(e)),
            Err(_) => Err(actor_silent()),
        }
    })
    .await
}

async fn get_categories(State(state): Store) -> impl IntoResponse {
    let key = ReadCache::key("/api/categories", None);
    cached_read(&state.cache, key, || async move {
        let (resp_tx, resp_rx) = oneshot::channel();
        let tx = clone_actor_tx(&state.tx).await;
        if tx
            .send(StoreRequest::ListCategoryIds { resp: resp_tx })
            .await
            .is_err()
        {
            return Err(actor_unavailable());
        }
        match resp_rx.await {
            Ok(Ok(ids)) => Ok(CategoriesResponse { ids }),
            Ok(Err(e)) => Err(internal_error(e)),
            Err(_) => Err(actor_silent()),
        }
    })
    .await
}

async fn get_cards(
    State(state): Store,
    RawQuery(raw_query): RawQuery,
    Query(q): Query<CardsQuery>,
) -> impl IntoResponse {
    if let Some(month) = q.month.as_deref() {
        if let Err(e) = parse_month_ref(month) {
            return error_response(StatusCode::BAD_REQUEST, e.to_string());
        }
    }
    let key = ReadCache::key("/api/cards", raw_query.as_deref());
    cached_read(&state.cache, key, || async move {
        let (resp_tx, resp_rx) = oneshot::channel();
        let tx = clone_actor_tx(&state.tx).await;
        if tx
            .send(StoreRequest::Cards {
                month: q.month,
                resp: resp_tx,
            })
            .await
            .is_err()
        {
            return Err(actor_unavailable());
        }
        match resp_rx.await {
            Ok(Ok(rows)) => Ok(CardsResponse { rows }),
            Ok(Err(e)) => Err(internal_error(e)),
            Err(_) => Err(actor_silent()),
        }
    })
    .await
}

async fn get_accounts(State(state): Store) -> impl IntoResponse {
    let key = ReadCache::key("/api/accounts", None);
    cached_read(&state.cache, key, || async move {
        let (resp_tx, resp_rx) = oneshot::channel();
        let tx = clone_actor_tx(&state.tx).await;
        if tx
            .send(StoreRequest::AccountsWithBalance { resp: resp_tx })
            .await
            .is_err()
        {
            return Err(actor_unavailable());
        }
        match resp_rx.await {
            Ok(Ok(rows)) => Ok(AccountsResponse { rows }),
            Ok(Err(e)) => Err(internal_error(e)),
            Err(_) => Err(actor_silent()),
        }
    })
    .await
}

async fn get_chart(
    State(state): Store,
    RawQuery(raw_query): RawQuery,
    Query(q): Query<ChartQuery>,
) -> impl IntoResponse {
    let key = ReadCache::key("/api/chart", raw_query.as_deref());
    cached_read(&state.cache, key, || async move {
        let (resp_tx, resp_rx) = oneshot::channel();
        let tx = clone_actor_tx(&state.tx).await;
        if tx
            .send(StoreRequest::GetChartData {
                months_back: q.months_back.unwrap_or(6),
                months_ahead: q.months_ahead.unwrap_or(6),
                resp: resp_tx,
            })
            .await
            .is_err()
        {
            return Err(actor_unavailable());
        }
        match resp_rx.await {
            Ok(Ok(chart)) => Ok(chart),
            Ok(Err(e)) => Err(internal_error(e)),
            Err(_) => Err(actor_silent()),
        }
    })
    .await
}

async fn get_forecasts(
    State(state): Store,
    RawQuery(raw_query): RawQuery,
    Query(q): Query<ForecastsQuery>,
) -> impl IntoResponse {
    let from = match parse_opt_date(q.from.as_deref()) {
        Ok(d) => d,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let until = match parse_opt_date(q.until.as_deref()) {
        Ok(d) => d,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let key = ReadCache::key("/api/forecasts", raw_query.as_deref());
    cached_read(&state.cache, key, || async move {
        let (resp_tx, resp_rx) = oneshot::channel();
        let tx = clone_actor_tx(&state.tx).await;
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
            return Err(actor_unavailable());
        }
        match resp_rx.await {
            // Each forecast keeps its snake_case fields and gains computed
            // `kind`/`draggable` planning metadata; the frontend adapts.
            Ok(Ok(forecasts)) => {
                let rows: Vec<Value> = forecasts.iter().map(ForecastWithKind::to_json).collect();
                Ok(serde_json::json!({ "forecasts": rows }))
            }
            Ok(Err(e)) => Err(internal_error(e)),
            Err(_) => Err(actor_silent()),
        }
    })
    .await
}

async fn get_forecast_templates(
    State(state): Store,
    RawQuery(raw_query): RawQuery,
    Query(q): Query<TemplatesQuery>,
) -> impl IntoResponse {
    let key = ReadCache::key("/api/forecast-templates", raw_query.as_deref());
    cached_read(&state.cache, key, || async move {
        let (resp_tx, resp_rx) = oneshot::channel();
        let tx = clone_actor_tx(&state.tx).await;
        if tx
            .send(StoreRequest::ListForecastTemplates {
                kind: q.kind,
                status: q.status,
                resp: resp_tx,
            })
            .await
            .is_err()
        {
            return Err(actor_unavailable());
        }
        match resp_rx.await {
            // ForecastTemplateRecord serialises as snake_case; the frontend adapts.
            Ok(Ok(templates)) => Ok(serde_json::json!({ "templates": templates })),
            Ok(Err(e)) => Err(internal_error(e)),
            Err(_) => Err(actor_silent()),
        }
    })
    .await
}

/// Upper bound on a single `/api/events` batch. Each write is applied serially
/// on the single store actor (DB read + write + audit insert), so an unbounded
/// batch would stall every other request. The UI flushes far fewer than this.
const MAX_EVENT_WRITES: usize = 1000;

/// `GET /api/status` — does this machine have an activated backend yet? The web
/// app uses this to choose between the onboarding screen and the dashboard.
async fn get_status(State(state): Store) -> impl IntoResponse {
    let status = state.activation.read().await.clone();
    Json(status)
}

/// `POST /api/activate` — decrypt an invite with its passphrase, persist the
/// embedded BigQuery service account + config, and hot-swap the store actor so
/// the running process serves the shared dataset without a restart.
async fn post_activate(
    State(state): Store,
    Json(req): Json<ActivateRequest>,
) -> axum::response::Response {
    let invite = match phai_core::open_invite(req.token.trim(), &req.passphrase) {
        Ok(invite) => invite,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, format!("{e:#}")),
    };

    let config = match persist_activation(&state.paths, &invite) {
        Ok(config) => config,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    };

    // Point the store actor at the new backend and drop stale (empty/local) reads.
    let _ = state.config_tx.send(config.clone());
    state.cache.bust();

    *state.activation.write().await = ActivationStatus {
        activated: true,
        label: Some(invite.label.clone()),
        project_id: config.project_id.clone(),
        dataset_id: config.dataset_id.clone(),
        sync_available: config.pluggy_config_path.is_some(),
    };

    Json(serde_json::json!({ "ok": true, "label": invite.label })).into_response()
}

/// Persist a decrypted invite: write the embedded service-account key (0600),
/// then save a BigQuery [`AppConfig`] pointing at it. Returns the saved config.
fn persist_activation(paths: &ConfigPaths, invite: &phai_core::Invite) -> Result<AppConfig> {
    paths.ensure()?;
    let sa_path = paths.config_dir.join("service-account.json");
    let sa_bytes =
        serde_json::to_vec_pretty(&invite.service_account).context("serializar service account")?;
    write_secret_file(&sa_path, &sa_bytes)?;

    let mut config = AppConfig::load(paths).unwrap_or_default();
    config.backend = BackendKind::Bigquery;
    config.project_id = Some(invite.project_id.clone());
    config.dataset_id = Some(invite.dataset_id.clone());
    config.actor_id = invite.actor_id.clone();
    config.service_account_path = Some(sa_path);
    config.local_db_path = None;
    config.save(paths)?;
    Ok(config)
}

/// Write `bytes` to `path`, restricting it to the owner (0600) on Unix — used
/// for the activated service-account key and config.
fn write_secret_file(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).with_context(|| format!("Falha ao gravar {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Falha ao definir permissões em {}", path.display()))?;
    }
    Ok(())
}

/// `POST /api/sync` — pull fresh transactions from Pluggy by running the same
/// `phai sync pluggy --json-summary` the CLI/cron uses, as a subprocess. Pluggy
/// credentials are loaded from the configured dotenv at request time (kept off
/// the daemon plist); the JSON summary (new-transaction count + list) is
/// returned to the web app.
async fn post_sync(State(state): Store) -> axum::response::Response {
    let config = state.config_tx.borrow().clone();
    let Some(pluggy_config) = config.pluggy_config_path.clone() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "sync Pluggy não configurado (defina pluggy_config_path)",
        );
    };

    let mut creds: Vec<(String, String)> = Vec::new();
    if let Some(env_path) = &config.pluggy_env_path {
        match std::fs::read_to_string(env_path) {
            Ok(body) => creds = parse_dotenv(&body),
            Err(e) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("falha ao ler pluggy env: {e}"),
                )
            }
        }
    }

    let exe = std::env::current_exe().unwrap_or_else(|_| "phai".into());
    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("sync")
            .arg("pluggy")
            .arg("--json-summary")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            // Never let the long-running sync trigger a self-update mid-request.
            .env("PHAI_NO_AUTO_UPDATE", "1");
        for (k, v) in creds {
            cmd.env(k, v);
        }
        cmd.output()
    })
    .await;

    let output = match output {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("falha ao executar sync: {e}"),
            )
        }
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("sync interrompido: {e}"),
            )
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr.lines().rev().take(4).collect::<Vec<_>>();
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "sync falhou: {}",
                tail.into_iter().rev().collect::<Vec<_>>().join(" ")
            ),
        );
    }

    // Fresh data landed in the store — drop the read cache so the next reads
    // (and the web reseed) reflect it.
    state.cache.bust();

    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
        Ok(json) => Json(json).into_response(),
        Err(_) => Json(serde_json::json!({ "ok": true })).into_response(),
    }
}

/// Parse a minimal dotenv (`KEY=VALUE` lines, `#` comments, optional quotes).
fn parse_dotenv(body: &str) -> Vec<(String, String)> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let v = v.trim().trim_matches('"').trim_matches('\'');
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

async fn post_events(State(state): Store, Json(body): Json<EventsBody>) -> impl IntoResponse {
    if body.writes.len() > MAX_EVENT_WRITES {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("batch de eventos excede o limite de {MAX_EVENT_WRITES} writes"),
        );
    }
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
    let tx = clone_actor_tx(&state.tx).await;
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
            // Any acked write mutated the store — invalidate cached reads.
            if !acked.is_empty() {
                state.cache.bust();
            }
            Json(EventsResponse { acked, failed }).into_response()
        }
        Err(_) => actor_silent(),
    }
}

/// Route a `ForecastPatch` through the store actor and map the outcome to an
/// HTTP response (the update half of `POST /api/forecast`).
async fn patch_forecast_response(
    state: &BridgeState,
    forecast_id: String,
    patch: ForecastPatch,
) -> axum::response::Response {
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&state.tx).await;
    if tx
        .send(StoreRequest::PatchForecast {
            forecast_id,
            patch,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(Some(forecast_id))) => {
            state.cache.bust();
            Json(serde_json::json!({ "forecastId": forecast_id })).into_response()
        }
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "forecast não encontrado"),
        Ok(Err(e)) => internal_error(e),
        Err(_) => actor_silent(),
    }
}

/// Input-length guardrails: prevent extremely long strings from landing in
/// the database. The limits match typical UX constraints (description is a
/// short label, not a free-text note). `Some` carries the 400 to return.
fn validate_forecast_body_lengths(body: &ForecastBody) -> Option<axum::response::Response> {
    const MAX_DESC_LEN: usize = 500;
    const MAX_ID_LEN: usize = 100;
    let too_long = |field: &str, len: usize, max: usize| {
        Some(error_response(
            StatusCode::BAD_REQUEST,
            format!("{field} muito longo ({len} caracteres; máximo {max})"),
        ))
    };
    if body.description.len() > MAX_DESC_LEN {
        return too_long("description", body.description.len(), MAX_DESC_LEN);
    }
    if let Some(len) = body.category_id.as_deref().map(str::len) {
        if len > MAX_ID_LEN {
            return too_long("category_id", len, MAX_ID_LEN);
        }
    }
    if let Some(len) = body.account_id.as_deref().map(str::len) {
        if len > MAX_ID_LEN {
            return too_long("account_id", len, MAX_ID_LEN);
        }
    }
    None
}

async fn post_forecast(State(state): Store, Json(body): Json<ForecastBody>) -> impl IntoResponse {
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
    if let Some(resp) = validate_forecast_body_lengths(&body) {
        return resp;
    }
    // Envelope upsert: a forecast_id re-amounts that forecast in place
    // instead of stacking a new one onto the (category, month) budget.
    if let Some(forecast_id) = body
        .forecast_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        let description = body.description.trim();
        let patch = ForecastPatch {
            amount,
            due_date,
            description: (!description.is_empty()).then(|| description.to_string()),
            category_id: body.category_id.clone(),
            account_id: body.account_id.clone(),
        };
        return patch_forecast_response(&state, forecast_id.to_string(), patch).await;
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
        metadata_json: body
            .ui_role
            .filter(|role| !role.trim().is_empty())
            .map(|role| json!({ "ui_role": role }))
            .unwrap_or_else(|| Value::Object(Default::default())),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        template_id: None,
        realized_transaction_id: None,
        realized_at: None,
    });
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&state.tx).await;
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
            state.cache.bust();
            Json(serde_json::json!({ "forecastId": forecast_id })).into_response()
        }
        Ok(Err(e)) => internal_error(e),
        Err(_) => actor_silent(),
    }
}

async fn post_forecast_move(
    State(state): Store,
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
    let tx = clone_actor_tx(&state.tx).await;
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
        })) => {
            state.cache.bust();
            Json(serde_json::json!({
                "forecastId": forecast_id,
                "dueDate": due_date.format("%Y-%m-%d").to_string(),
                "status": status,
            }))
            .into_response()
        }
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
        Ok(Err(e)) => internal_error(e),
        Err(_) => actor_silent(),
    }
}

async fn post_forecast_delete(
    State(state): Store,
    Json(body): Json<DeleteForecastBody>,
) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&state.tx).await;
    if tx
        .send(StoreRequest::DeleteForecast {
            forecast_id: body.forecast_id,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(DeleteForecastResult::Deleted {
            forecast_id,
            status,
        })) => {
            state.cache.bust();
            Json(json!({ "forecastId": forecast_id, "status": status })).into_response()
        }
        Ok(Ok(DeleteForecastResult::NotFound)) => {
            error_response(StatusCode::NOT_FOUND, "forecast não encontrado")
        }
        Ok(Ok(DeleteForecastResult::NotManual)) => error_response(
            StatusCode::CONFLICT,
            "apenas forecasts manuais podem ser excluídos pela web",
        ),
        Ok(Err(e)) => internal_error(e),
        Err(_) => actor_silent(),
    }
}

async fn post_forecast_settle(
    State(state): Store,
    Json(body): Json<SettleForecastBody>,
) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&state.tx).await;
    if tx
        .send(StoreRequest::SettleForecast {
            forecast_id: body.forecast_id,
            transaction_id: body.transaction_id,
            resp: resp_tx,
        })
        .await
        .is_err()
    {
        return actor_unavailable();
    }
    match resp_rx.await {
        Ok(Ok(SettleForecastResult::Settled {
            forecast_id,
            status,
        })) => {
            state.cache.bust();
            Json(json!({ "forecastId": forecast_id, "status": status })).into_response()
        }
        Ok(Ok(SettleForecastResult::NotFound)) => {
            error_response(StatusCode::NOT_FOUND, "forecast não encontrado")
        }
        Ok(Ok(SettleForecastResult::NotManual)) => error_response(
            StatusCode::CONFLICT,
            "apenas forecasts manuais podem ser efetivados manualmente pela web",
        ),
        Ok(Ok(SettleForecastResult::TransactionNotFound)) => {
            error_response(StatusCode::NOT_FOUND, "transação não encontrada")
        }
        Ok(Ok(SettleForecastResult::AmountMismatch)) => error_response(
            StatusCode::CONFLICT,
            "transação fora da tolerância de valor para este forecast",
        ),
        Ok(Ok(SettleForecastResult::SignMismatch)) => error_response(
            StatusCode::CONFLICT,
            "transação com sinal incompatível com o forecast",
        ),
        Ok(Err(e)) => internal_error(e),
        Err(_) => actor_silent(),
    }
}

async fn post_accept_template(
    State(state): Store,
    Json(body): Json<AcceptTemplateBody>,
) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&state.tx).await;
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
        Ok(Ok(result)) => {
            state.cache.bust();
            Json(result).into_response()
        }
        Ok(Err(e)) => error_response(StatusCode::BAD_REQUEST, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn post_dismiss_template(
    State(state): Store,
    Json(body): Json<DismissTemplateBody>,
) -> impl IntoResponse {
    let template_id = body.template_id.clone();
    let (resp_tx, resp_rx) = oneshot::channel();
    let tx = clone_actor_tx(&state.tx).await;
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
        Ok(Ok(())) => {
            state.cache.bust();
            Json(serde_json::json!({ "template_id": template_id })).into_response()
        }
        Ok(Err(e)) => internal_error(e),
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
    let (paths, config) = load_config().await?;
    let paths = Arc::new(paths);
    let config: AppConfig = config;
    let bridge_identity = Arc::new(bridge_identity_response(&config));

    // A machine is "activated" once a config file has been written — either by
    // the owner's CLI `auth setup` or by `/api/activate`. A fresh install has no
    // config file yet, so the web app shows onboarding instead of the dashboard.
    let activation = Arc::new(RwLock::new(ActivationStatus {
        activated: paths.config_file.exists(),
        label: None,
        project_id: config.project_id.clone(),
        dataset_id: config.dataset_id.clone(),
        sync_available: config.pluggy_config_path.is_some(),
    }));

    // The store actor watches this for config changes. `/api/activate` sends a
    // new (BigQuery) config here, and the actor restarts against it in place.
    let (config_tx, config_rx) = watch::channel(config.clone());
    let config_tx = Arc::new(config_tx);

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
    // retries — the window is ~1 s. When `/api/activate` publishes a new config,
    // the actor cancels the running store and reopens against the new backend.
    let actor_tx = store_tx.clone();
    let mut config_rx = config_rx;
    local.spawn_local(async move {
        loop {
            let actor_config = config_rx.borrow().clone();
            let (tx, rx) = mpsc::channel::<StoreRequest>(STORE_CHANNEL_CAP);
            *actor_tx.write().await = tx;

            let run_store = async {
                let store = open_store(&actor_config).await?;
                run_migrations(store.as_ref(), &actor_config).await?;
                store_actor_loop(store, actor_config.clone(), rx).await;
                Ok::<_, anyhow::Error>(())
            };

            tokio::select! {
                result = run_store => match result {
                    Ok(()) => break, // clean shutdown (all senders dropped)
                    Err(e) => {
                        eprintln!(
                            "[phai serve] store actor caiu: {e:#}\n\
                             [phai serve] reiniciando em 1s..."
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                },
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        break; // config sender dropped → process shutting down
                    }
                    // New config activated: fall through to reopen the store.
                }
            }
        }
    });

    let app_state = BridgeState {
        tx: store_tx,
        cache: ReadCache::default(),
        config_tx,
        paths,
        activation,
    };

    // All `/api` routes are guarded by the same-origin check so a malicious
    // page cannot drive the bridge via the user's browser (CSRF). Requests
    // without an Origin header (curl, direct integration) are allowed.
    let api = Router::new()
        .route("/api", get(api_status))
        .route("/api/status", get(get_status))
        .route("/api/activate", post(post_activate))
        .route("/api/sync", post(post_sync))
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
        .route("/api/forecast/delete", post(post_forecast_delete))
        .route("/api/forecast/move", post(post_forecast_move))
        .route("/api/forecast/settle", post(post_forecast_settle))
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

/// Reject `/api` requests that are not addressed to a loopback host or whose
/// `Origin` is not localhost. Runs before every API handler.
///
/// The `Host` check is the anti-DNS-rebinding defense: an attacker page on
/// `evil.com` that rebinds DNS to `127.0.0.1` reaches us with `Host: evil.com`,
/// and same-origin browser GETs carry no `Origin` header — so the origin check
/// alone never fires on reads. Pinning `Host` to the loopback names we actually
/// serve closes that hole.
async fn guard_origin(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !is_host_allowed(req.headers()) {
        let host = req
            .headers()
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<missing>");
        eprintln!(
            "[phai serve] host rejeitado: {host} — {} {}",
            req.method(),
            req.uri().path()
        );
        return (StatusCode::FORBIDDEN, "Host não permitido").into_response();
    }
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

/// Loopback hosts we answer to. The bind address is always `127.0.0.1`
/// (`LOCAL_BIND_HOST`), and the app is reached via `phai.localhost` or plain
/// `localhost`/`127.0.0.1`; anything else in the `Host` header is a rebinding
/// attempt or a misrouted request.
fn host_allowed(host: &str) -> bool {
    // Strip the optional `:port`. Loopback hosts never contain `[` (no IPv6
    // literal — we bind v4 only), so splitting on the first `:` is safe.
    let hostname = host.split(':').next().unwrap_or(host);
    matches!(hostname, "127.0.0.1" | "localhost" | "phai.localhost")
}

/// Reject requests whose `Host` header is not a loopback name. A missing `Host`
/// is allowed (HTTP/1.0 / direct-socket integrations); browsers always send one,
/// so the rebinding vector is covered.
fn is_host_allowed(headers: &HeaderMap) -> bool {
    match headers.get("host") {
        None => true,
        Some(v) => v.to_str().map(host_allowed).unwrap_or(false),
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

    // ── activation persistence ─────────────────────────────────────────────

    fn temp_paths(dir: &std::path::Path) -> ConfigPaths {
        ConfigPaths {
            config_dir: dir.to_path_buf(),
            data_dir: dir.to_path_buf(),
            config_file: dir.join("config.toml"),
            local_db_file: dir.join("phai.local.db"),
        }
    }

    #[test]
    fn category_is_locked_matches_exact_sub_or_whole_parent() {
        let locked = vec!["moradia:aluguel".to_string(), "educacao".to_string()];
        // exact sub match
        assert!(category_is_locked(Some("moradia:aluguel"), &locked));
        // whole-parent match locks every sub
        assert!(category_is_locked(Some("educacao:escola"), &locked));
        assert!(category_is_locked(Some("educacao"), &locked));
        // siblings of a sub-only entry stay unlocked
        assert!(!category_is_locked(Some("moradia:servicos"), &locked));
        assert!(!category_is_locked(Some("alimentacao:mercado"), &locked));
        assert!(!category_is_locked(None, &locked));
    }

    #[test]
    fn parse_dotenv_reads_keys_skips_comments_and_strips_quotes() {
        let body = "# pluggy creds\nPLUGGY_CLIENT_ID=abc123\n\nPLUGGY_CLIENT_SECRET=\"s3cr3t\"\n  TRAILING = 'x' \n";
        let parsed = parse_dotenv(body);
        assert_eq!(
            parsed,
            vec![
                ("PLUGGY_CLIENT_ID".to_string(), "abc123".to_string()),
                ("PLUGGY_CLIENT_SECRET".to_string(), "s3cr3t".to_string()),
                ("TRAILING".to_string(), "x".to_string()),
            ]
        );
    }

    #[test]
    fn persist_activation_writes_bigquery_config_and_locked_key() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = temp_paths(tmp.path());
        let invite = phai_core::Invite::new(
            "demo-proj",
            "phai",
            "esposa",
            "rw",
            serde_json::json!({"type": "service_account", "private_key": "FAKE"}),
            "MacBook Esposa",
        );

        let config = persist_activation(&paths, &invite).unwrap();

        assert!(matches!(config.backend, BackendKind::Bigquery));
        assert_eq!(config.project_id.as_deref(), Some("demo-proj"));
        assert_eq!(config.dataset_id.as_deref(), Some("phai"));
        assert_eq!(config.actor_id, "esposa");
        assert!(config.local_db_path.is_none());

        let sa_path = tmp.path().join("service-account.json");
        assert!(sa_path.exists(), "service account key must be written");
        // Config is reloadable and stays BigQuery on the next boot.
        let reloaded = AppConfig::load(&paths).unwrap();
        assert!(matches!(reloaded.backend, BackendKind::Bigquery));
        assert_eq!(reloaded.actor_id, "esposa");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&sa_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "key must be owner-only");
        }
    }

    #[test]
    fn activation_round_trips_from_a_sealed_invite() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = temp_paths(tmp.path());
        let invite = phai_core::Invite::new(
            "p",
            "phai",
            "esposa",
            "rw",
            serde_json::json!({"type": "service_account"}),
            "Device",
        );
        let token = phai_core::seal_invite(&invite, "s3nha").unwrap();

        // The bridge opens the token exactly as `/api/activate` does.
        let opened = phai_core::open_invite(&token, "s3nha").unwrap();
        let config = persist_activation(&paths, &opened).unwrap();
        assert_eq!(config.project_id.as_deref(), Some("p"));
    }

    // ── bridge_identity_response ───────────────────────────────────────────

    #[test]
    fn identity_response_includes_binary_version() {
        let response = bridge_identity_response(&AppConfig::default());
        assert_eq!(response.version, env!("CARGO_PKG_VERSION"));
    }

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

    // ── is_host_allowed (anti-DNS-rebinding) ───────────────────────────────

    #[test]
    fn host_absent_is_allowed() {
        assert!(is_host_allowed(&HeaderMap::new()));
    }

    #[test]
    fn loopback_hosts_allowed() {
        for value in [
            "127.0.0.1",
            "127.0.0.1:80",
            "localhost",
            "localhost:8080",
            "phai.localhost",
            "phai.localhost:80",
        ] {
            let mut h = HeaderMap::new();
            h.insert("host", HeaderValue::from_str(value).unwrap());
            assert!(is_host_allowed(&h), "host {value} should be allowed");
        }
    }

    #[test]
    fn rebinding_host_rejected() {
        // DNS rebinding: attacker domain resolves to 127.0.0.1 but the Host
        // header still carries the attacker's name.
        for value in ["evil.example.com", "evil.example.com:80", "192.168.1.5"] {
            let mut h = HeaderMap::new();
            h.insert("host", HeaderValue::from_str(value).unwrap());
            assert!(!is_host_allowed(&h), "host {value} must be rejected");
        }
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

    #[tokio::test(flavor = "current_thread")]
    async fn build_accounts_api_joins_latest_balance_snapshot() {
        let (_dir, _config, store) = temp_store().await;
        let checking = sample_account("acc-checking", "Nubank", "felipe");
        let mut card = sample_account("acc-card", "Card", "felipe");
        card.account_type = "credit".into();
        store.upsert_accounts(&[checking, card]).await.unwrap();

        use phai_core::models::AccountSnapshotRecord;
        let snap = |id: &str, bal: i64| AccountSnapshotRecord {
            snapshot_id: format!("snap-{id}"),
            account_id: id.into(),
            snapshot_date: NaiveDate::from_ymd_opt(2026, 6, 20).unwrap(),
            balance: Some(Decimal::new(bal, 2)),
            credit_limit: None,
            currency_code: Some("BRL".into()),
            source: "pluggy".into(),
            actor_id: "test".into(),
            idempotency_key: format!("idem-{id}"),
            metadata_json: serde_json::Value::Object(Default::default()),
            created_at: Utc::now(),
        };
        store
            .insert_account_snapshots(&[snap("acc-checking", 294742), snap("acc-card", 700000)])
            .await
            .unwrap();

        // A configured label override wins over Pluggy's raw label.
        let labels = std::collections::HashMap::from([(
            "acc-checking".to_string(),
            "Nubank Felipe".to_string(),
        )]);
        let rows = build_accounts_api(store.as_ref(), &labels).await.unwrap();
        let checking = rows.iter().find(|r| r.id == "acc-checking").unwrap();
        assert_eq!(checking.account_type, "checking");
        assert_eq!(checking.balance.as_deref(), Some("2947.42"));
        assert_eq!(checking.label, "Nubank Felipe");
        let card = rows.iter().find(|r| r.id == "acc-card").unwrap();
        assert_eq!(card.account_type, "credit");
        assert_eq!(card.balance.as_deref(), Some("7000.00"));
        assert_eq!(card.label, "Card"); // no override → raw label
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
        let value =
            serde_json::to_value(AccountRow::from_record(&acc, Some(&Decimal::new(12345, 2))))
                .unwrap();
        assert_eq!(value["id"], "acc-1");
        assert_eq!(value["label"], "Conta Corrente");
        assert_eq!(value["owner"], "alice");
        assert_eq!(value["accountType"], "checking");
        assert_eq!(value["balance"], "123.45");
    }

    #[test]
    fn account_row_falls_back_to_id_when_label_blank() {
        let acc = sample_account("acc-9", "   ", "bob");
        let row = AccountRow::from_record(&acc, None);
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

    // ── ForecastBody contract (envelope upsert) ────────────────────────────

    #[test]
    fn forecast_body_accepts_optional_forecast_id() {
        let body: ForecastBody = serde_json::from_str(
            r#"{"amount":"-450.00","due_date":"2026-07-31","category_id":"alimentacao","forecast_id":"f-1"}"#,
        )
        .unwrap();
        assert_eq!(body.forecast_id.as_deref(), Some("f-1"));

        let body: ForecastBody =
            serde_json::from_str(r#"{"amount":"-450.00","forecast_id":null}"#).unwrap();
        assert!(body.forecast_id.is_none());
    }

    // ── Behavioral: forecast patch against a real SQLite store ─────────────

    #[tokio::test(flavor = "current_thread")]
    async fn patch_forecast_reamounts_in_place_preserving_provenance() {
        let (_dir, _config, store) = temp_store().await;
        let mut envelope = sample_forecast("f-env-1", Some("tpl-1"));
        envelope.category_id = Some("moradia".into());
        upsert_forecast(store.as_ref(), envelope).await.unwrap();

        let patched = patch_forecast(
            store.as_ref(),
            "f-env-1",
            ForecastPatch {
                amount: Decimal::from_str("-222.33").unwrap(),
                due_date: Some(NaiveDate::from_ymd_opt(2026, 4, 30).unwrap()),
                description: None,
                category_id: None,
                account_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(patched.as_deref(), Some("f-env-1"));

        let stored = store.get_forecast("f-env-1").await.unwrap().unwrap();
        assert_eq!(stored.amount, Decimal::from_str("-222.33").unwrap());
        assert_eq!(
            stored.due_date,
            Some(NaiveDate::from_ymd_opt(2026, 4, 30).unwrap())
        );
        // Untouched fields keep their original values.
        assert_eq!(stored.description, "Aluguel");
        assert_eq!(stored.category_id.as_deref(), Some("moradia"));
        assert_eq!(stored.template_id.as_deref(), Some("tpl-1"));
        assert_eq!(stored.status, "ativo");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn patch_forecast_unknown_id_returns_none() {
        let (_dir, _config, store) = temp_store().await;
        let patched = patch_forecast(
            store.as_ref(),
            "missing",
            ForecastPatch {
                amount: Decimal::from_str("-1.00").unwrap(),
                due_date: None,
                description: None,
                category_id: None,
                account_id: None,
            },
        )
        .await
        .unwrap();
        assert!(patched.is_none());
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

    fn dedup_sample_forecast() -> ForecastRecord {
        ForecastRecord {
            forecast_id: String::new(),
            due_date: Some(chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
            description: "Aluguel".into(),
            amount: Decimal::from_str("-2500.00").unwrap(),
            category_id: Some("moradia:aluguel".into()),
            account_id: Some("acc-1".into()),
            status: "ativo".into(),
            recurrence: None,
            actor_id: "test-actor".into(),
            idempotency_key: String::new(),
            metadata_json: serde_json::Value::Object(Default::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            template_id: None,
            realized_transaction_id: None,
            realized_at: None,
        }
    }

    // Regression: a retried/duplicated create POST (same logical forecast, empty
    // forecast_id) must not stack a second row. The web sync queue's flush guard
    // is per-mount, so two tabs/mounts can fire the same create twice; dedup must
    // happen server-side on the idempotency key.
    #[tokio::test(flavor = "current_thread")]
    async fn upsert_forecast_dedups_duplicate_creates_by_idempotency_key() {
        let (_dir, _config, store) = temp_store().await;
        let id1 = upsert_forecast(store.as_ref(), dedup_sample_forecast())
            .await
            .unwrap();
        let id2 = upsert_forecast(store.as_ref(), dedup_sample_forecast())
            .await
            .unwrap();
        assert_eq!(id1, id2, "duplicate create must return the existing id");
        let all = store.list_forecasts(None, None, None).await.unwrap();
        assert_eq!(all.len(), 1, "duplicate create must not stack a second row");
    }

    // Regression (audit B1): the forecast table carries a DUAL status vocabulary
    // where `ativo` (legacy pt-BR) and `active` (en) coexist for the same logical
    // state. The idempotency key is status-agnostic by construction (status is not
    // one of its inputs), so two creates of the same logical forecast that differ
    // ONLY in this vocab — one `ativo`, one `active` — must still collapse to a
    // single row. If status ever leaks into the key, this duplicate would survive.
    #[tokio::test(flavor = "current_thread")]
    async fn upsert_forecast_dedups_across_ativo_active_status_vocab() {
        let (_dir, _config, store) = temp_store().await;

        let mut first = dedup_sample_forecast();
        first.status = "ativo".into();
        let id1 = upsert_forecast(store.as_ref(), first).await.unwrap();

        // Same logical forecast, but tagged with the English vocab variant.
        let mut second = dedup_sample_forecast();
        second.status = "active".into();
        let id2 = upsert_forecast(store.as_ref(), second).await.unwrap();

        assert_eq!(
            id1, id2,
            "a create differing only in the ativo/active vocab must dedup to the existing id"
        );
        let all = store.list_forecasts(None, None, None).await.unwrap();
        assert_eq!(
            all.len(),
            1,
            "the ativo/active vocab split must not let a duplicate forecast survive"
        );
        // The lookup ignores discard state but not these two; both are live, so a
        // status filter must treat them as a single deduped row regardless of vocab.
        let live = store
            .list_forecasts(None, None, None)
            .await
            .unwrap()
            .into_iter()
            .filter(|f| matches!(f.status.to_lowercase().as_str(), "ativo" | "active"))
            .count();
        assert_eq!(live, 1, "exactly one live forecast across the vocab split");
    }

    // Regression (audit B1): materialised forecasts and their backing template
    // live in two tables (`forecast` + `forecast_template`). Re-materialising the
    // same template — e.g. a re-accept, or a retried POST — must be idempotent on
    // the `forecast` table: the deterministic per-month forecast_id keeps the MERGE
    // from stacking duplicate rows, even though the template row is a distinct
    // entity that legitimately coexists with the forecasts it produces.
    #[tokio::test(flavor = "current_thread")]
    async fn rematerialising_template_does_not_stack_duplicate_forecasts() {
        use crate::forecast_cmd::materialise_template_forecasts;
        use phai_core::models::ForecastTemplateRecord;

        let (_dir, _config, store) = temp_store().await;
        let now = Utc::now();
        let template = ForecastTemplateRecord {
            template_id: "tpl-rent".into(),
            kind: "fixed".into(),
            description: "Aluguel".into(),
            merchant_pattern: None,
            category_id: Some("moradia:aluguel".into()),
            account_id: Some("acc-1".into()),
            amount: Decimal::from_str("-2500.00").unwrap(),
            amount_lower: None,
            amount_upper: None,
            cadence: "monthly".into(),
            next_due_day: Some(1),
            start_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            end_date: None,
            remaining_count: None,
            source: "manual".into(),
            confidence: None,
            status: "ativo".into(),
            metadata_json: Value::Object(Default::default()),
            actor_id: "test-actor".into(),
            idempotency_key: "template:tpl-rent".into(),
            created_at: now,
            updated_at: now,
        };
        store
            .upsert_forecast_templates(std::slice::from_ref(&template))
            .await
            .unwrap();

        let first = materialise_template_forecasts(store.as_ref(), &template, 3, "test-actor", now)
            .await
            .unwrap();
        assert_eq!(first, 3, "first materialisation creates three months");

        // Re-materialise the same template; deterministic forecast_ids must MERGE
        // in place rather than stack new rows.
        materialise_template_forecasts(store.as_ref(), &template, 3, "test-actor", now)
            .await
            .unwrap();

        let forecasts = store.list_forecasts(None, None, None).await.unwrap();
        assert_eq!(
            forecasts.len(),
            3,
            "re-materialising a template must not stack duplicate forecast rows"
        );
        // The template row is a distinct entity and must survive alongside its
        // materialised forecasts — the cross-table split is intentional, not a dup.
        let templates = store.list_forecast_templates(None, None).await.unwrap();
        assert_eq!(
            templates.len(),
            1,
            "the backing template row coexists with its forecasts"
        );
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
                    commitment_tier: None,
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
                    commitment_tier: None,
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
            &[],
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
            &[],
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
            &[],
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

    #[tokio::test(flavor = "current_thread")]
    async fn delete_forecast_discards_manual_row() {
        let (_dir, _config, store) = temp_store().await;
        let mut forecast = sample_forecast("f-manual", None);
        ensure_forecast_idempotency(&mut forecast).unwrap();
        store.upsert_forecasts(&[forecast]).await.unwrap();

        let outcome = delete_forecast(store.as_ref(), "f-manual").await.unwrap();
        assert!(matches!(
            outcome,
            DeleteForecastResult::Deleted {
                ref forecast_id, ..
            } if forecast_id == "f-manual"
        ));

        let stored = store.get_forecast("f-manual").await.unwrap().unwrap();
        assert_eq!(stored.status, "descartado");
        assert_eq!(
            stored
                .metadata_json
                .get("discarded_at")
                .and_then(|value| value.as_str())
                .map(|value| !value.is_empty()),
            Some(true)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn settle_forecast_links_real_transaction_and_keeps_predicted_amount() {
        let (_dir, _config, store) = temp_store().await;
        let record = sample_record();
        let mut forecast = sample_forecast("f-manual", None);
        forecast.amount = Decimal::from_str("-12.00").unwrap();
        forecast.description = "Almoço".into();
        forecast.due_date = Some(record.transaction_date);
        ensure_forecast_idempotency(&mut forecast).unwrap();
        store.upsert_transactions(&[record]).await.unwrap();
        store.upsert_forecasts(&[forecast]).await.unwrap();

        let outcome = settle_forecast(store.as_ref(), "f-manual", "tx-1")
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            SettleForecastResult::Settled {
                ref forecast_id, ..
            } if forecast_id == "f-manual"
        ));

        let stored = store.get_forecast("f-manual").await.unwrap().unwrap();
        assert_eq!(stored.status, "realizado");
        assert_eq!(stored.realized_transaction_id.as_deref(), Some("tx-1"));
        assert_eq!(stored.amount, Decimal::from_str("-12.50").unwrap());
        assert_eq!(
            stored
                .metadata_json
                .get("predicted_amount")
                .and_then(|value| value.as_str()),
            Some("-12.00")
        );
        assert_eq!(
            stored
                .metadata_json
                .get("realized_amount")
                .and_then(|value| value.as_str()),
            Some("-12.5")
        );
    }

    // ── cached_read get-or-compute wiring ──────────────────────────────────

    /// A fresh key runs the closure and serializes the body; the same key on a
    /// second call returns the cached bytes without re-running the closure.
    #[tokio::test(flavor = "current_thread")]
    async fn cached_read_computes_once_then_serves_from_cache() {
        let cache = ReadCache::default();
        let key = ReadCache::key("/api/categories", None);
        let calls = std::cell::Cell::new(0);

        let first = cached_read(&cache, key.clone(), || async {
            calls.set(calls.get() + 1);
            Ok::<_, axum::response::Response>(CategoriesResponse {
                ids: vec!["alimentacao".into()],
            })
        })
        .await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(calls.get(), 1);

        // Second call hits the cache: the closure must not run again.
        let _second = cached_read(&cache, key.clone(), || async {
            calls.set(calls.get() + 1);
            Ok::<_, axum::response::Response>(CategoriesResponse { ids: vec![] })
        })
        .await;
        assert_eq!(calls.get(), 1, "second read must be served from cache");

        // A bust forces the closure to run again.
        cache.bust();
        let _third = cached_read(&cache, key, || async {
            calls.set(calls.get() + 1);
            Ok::<_, axum::response::Response>(CategoriesResponse { ids: vec![] })
        })
        .await;
        assert_eq!(calls.get(), 2, "bust must force a re-query");
    }

    /// An error outcome is returned verbatim and never cached: the next call
    /// re-runs the closure.
    #[tokio::test(flavor = "current_thread")]
    async fn cached_read_does_not_cache_errors() {
        let cache = ReadCache::default();
        let key = ReadCache::key("/api/accounts", None);
        let calls = std::cell::Cell::new(0);

        for _ in 0..2 {
            let resp = cached_read(&cache, key.clone(), || async {
                calls.set(calls.get() + 1);
                Err::<CategoriesResponse, _>(internal_error("boom"))
            })
            .await;
            assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
        assert_eq!(calls.get(), 2, "errors must never be cached");
    }
}
