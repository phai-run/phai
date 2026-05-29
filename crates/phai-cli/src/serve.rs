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
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{NaiveDate, Utc};
use phai_core::idempotency::ensure_forecast_idempotency;
use phai_core::migrations::run_migrations;
use phai_core::models::{
    AccountRecord, AuditEvent, ForecastRecord, ForecastTemplateRecord, TransactionRecord,
};
use phai_core::storage::{open_store, FinanceStore};
use phai_core::AppConfig;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::task::LocalSet;
use uuid::Uuid;

const STORE_CHANNEL_CAP: usize = 64;
const LOCAL_BIND_HOST: &str = "127.0.0.1";
const LOCAL_APP_HOST: &str = "meuapp.localhost";
/// Default number of review-queue rows returned when the caller omits `limit`.
const DEFAULT_REVIEW_QUEUE_LIMIT: usize = 200;
/// Actor id stamped on writes that originate from the web bridge.
const SERVE_ACTOR_ID: &str = "serve-dashboard";

use crate::cashflow_chart::{build_chart_data, ChartData};
use crate::forecast_cmd::materialise_template_forecasts;
use crate::{
    all_transactions_for_review, apply_human_review, load_config, review_human_rows,
    HumanReviewPatch, ReviewFilters, ReviewHumanKind,
};

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
    ListForecasts {
        status: Option<String>,
        from: Option<NaiveDate>,
        until: Option<NaiveDate>,
        resp: oneshot::Sender<Result<Vec<ForecastRecord>>>,
    },
    ListCategoryIds {
        resp: oneshot::Sender<Result<Vec<String>>>,
    },
    GetAccounts {
        resp: oneshot::Sender<Result<Vec<AccountRecord>>>,
    },
    ReviewQueue {
        params: ReviewQueueParams,
        resp: oneshot::Sender<Result<Vec<TransactionRecord>>>,
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
            StoreRequest::ListForecasts {
                status,
                from,
                until,
                resp,
            } => {
                let result = store.list_forecasts(status.as_deref(), from, until).await;
                let _ = resp.send(result);
            }
            StoreRequest::ListCategoryIds { resp } => {
                let result = store
                    .list_all_category_ids()
                    .await
                    .map(|ids| ids.into_iter().collect());
                let _ = resp.send(result);
            }
            StoreRequest::GetAccounts { resp } => {
                let result = store.get_accounts().await;
                let _ = resp.send(result);
            }
            StoreRequest::ReviewQueue { params, resp } => {
                let result = load_review_queue(store.as_ref(), params).await;
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
    let diff = serde_json::to_value(&record).unwrap_or_default();
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

/// One row of the review queue, in the shape the frontend consumes. Field
/// names are camelCase by contract.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewQueueRow {
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
}

impl ReviewQueueRow {
    fn from_record(row: &TransactionRecord) -> Self {
        Self {
            id: row.transaction_id.clone(),
            account_id: row.account_id.clone(),
            posted_at: row.transaction_date.format("%Y-%m-%d").to_string(),
            amount: format!("{:.2}", row.amount.round_dp(2)),
            raw_description: row.raw_description.clone(),
            description: row.description.clone(),
            merchant_name: row.merchant_name.clone(),
            purpose: row.purpose.clone(),
            category_id: row.category_id.clone(),
            month: row.transaction_date.format("%Y-%m").to_string(),
        }
    }
}

#[derive(Serialize)]
struct ReviewQueueResponse {
    rows: Vec<ReviewQueueRow>,
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
struct AcceptTemplateBody {
    template_id: String,
    materialize_months: Option<u32>,
}

#[derive(Deserialize)]
struct DismissTemplateBody {
    template_id: String,
}

// ── HTTP handlers ────────────────────────────────────────────────────────

type Store = State<Arc<mpsc::Sender<StoreRequest>>>;

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
        Ok(Ok(rows)) => Json(ReviewQueueResponse {
            rows: rows.iter().map(ReviewQueueRow::from_record).collect(),
        })
        .into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn get_categories(State(tx): Store) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
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

async fn get_accounts(State(tx): Store) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
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
    if tx
        .send(StoreRequest::ListForecasts {
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
        // ForecastRecord serialises as snake_case (its derived serde shape);
        // the frontend adapts.
        Ok(Ok(forecasts)) => Json(serde_json::json!({ "forecasts": forecasts })).into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn get_forecast_templates(
    State(tx): Store,
    Query(q): Query<TemplatesQuery>,
) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
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
            Json(serde_json::json!({ "forecast_id": forecast_id })).into_response()
        }
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(_) => actor_silent(),
    }
}

async fn post_accept_template(
    State(tx): Store,
    Json(body): Json<AcceptTemplateBody>,
) -> impl IntoResponse {
    let (resp_tx, resp_rx) = oneshot::channel();
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
    let actor_config = config.clone();

    // Build the channel before entering LocalSet.
    let (store_tx, store_rx) = mpsc::channel::<StoreRequest>(STORE_CHANNEL_CAP);

    let local = LocalSet::new();

    // Spawn the !Send store actor on the local set.
    local.spawn_local(async move {
        let store = open_store(&actor_config).await?;
        run_migrations(store.as_ref(), &actor_config).await?;
        store_actor_loop(store, actor_config, store_rx).await;
        Ok::<_, anyhow::Error>(())
    });

    let app_state = Arc::new(store_tx);

    // All `/api` routes are guarded by the same-origin check so a malicious
    // page cannot drive the bridge via the user's browser (CSRF). Requests
    // without an Origin header (curl, direct integration) are allowed.
    let api = Router::new()
        .route("/api", get(api_status))
        .route("/api/review-queue", get(get_review_queue))
        .route("/api/categories", get(get_categories))
        .route("/api/accounts", get(get_accounts))
        .route("/api/chart", get(get_chart))
        .route("/api/forecasts", get(get_forecasts))
        .route("/api/forecast-templates", get(get_forecast_templates))
        .route("/api/events", post(post_events))
        .route("/api/forecast", post(post_forecast))
        .route("/api/forecast-template/accept", post(post_accept_template))
        .route(
            "/api/forecast-template/dismiss",
            post(post_dismiss_template),
        )
        .layer(axum::middleware::from_fn(guard_origin))
        .with_state(app_state);

    let app = api
        // Serve the embedded LiveStore SPA for everything else (index + assets
        // + client-side routes).
        .fallback(crate::serve_assets::static_handler);

    let addr = format!("{LOCAL_BIND_HOST}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("falha ao escutar em {addr}"))?;

    println!("🌐 Dashboard em http://{LOCAL_APP_HOST}:{port}");
    println!("   Pressione Ctrl+C para parar");

    local
        .run_until(async move {
            axum::serve(listener, app)
                .await
                .context("servidor web parou")
        })
        .await?;

    Ok(())
}

/// Reject `/api` requests whose `Origin` is not localhost. Runs before every
/// API handler.
async fn guard_origin(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !is_origin_allowed(req.headers()) {
        return (StatusCode::FORBIDDEN, "Origin não permitida").into_response();
    }
    next.run(req).await
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
                || origin.starts_with("http://meuapp.localhost:")
                || origin == "null"
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
            HeaderValue::from_static("http://meuapp.localhost:8080"),
        );
        assert!(is_origin_allowed(&h));
    }

    #[test]
    fn null_origin_allowed() {
        let mut h = HeaderMap::new();
        h.insert("origin", HeaderValue::from_static("null"));
        assert!(is_origin_allowed(&h));
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

    #[test]
    fn review_queue_row_serialises_camel_case() {
        let row = ReviewQueueRow::from_record(&sample_record());
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
        // snake_case keys must NOT leak.
        assert!(value.get("account_id").is_none());
        assert!(value.get("raw_description").is_none());
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
}
