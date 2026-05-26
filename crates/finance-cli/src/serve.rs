//! `fin serve` — local web dashboard with WebSocket API.
//!
//! Starts an HTTP server that hosts the interactive forecast dashboard
//! and a WebSocket endpoint at `/ws`. Because `Box<dyn FinanceStore>`
//! is `!Send`, a channel-based store actor runs inside `LocalSet` while
//! the axum router and WebSocket handlers live in the `Send` world.

use anyhow::{Context, Result};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use chrono::{NaiveDate, Utc};
use finance_core::idempotency::ensure_forecast_idempotency;
use finance_core::migrations::run_migrations;
use finance_core::models::{
    AccountRecord, AuditEvent, CategoryRecord, ForecastRecord, ForecastTemplateRecord,
};
use finance_core::storage::{open_store, FinanceStore};
use finance_core::AppConfig;
use futures::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::task::LocalSet;
use uuid::Uuid;

use crate::cashflow_chart::{build_chart_data, ChartData};
use crate::forecast_cmd::materialise_template_forecasts;
use crate::load_config;

// ── Store actor ──────────────────────────────────────────────────────────

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
        record: ForecastRecord,
        resp: oneshot::Sender<Result<String>>,
    },
    ListForecasts {
        status: Option<String>,
        from: Option<NaiveDate>,
        until: Option<NaiveDate>,
        resp: oneshot::Sender<Result<Vec<ForecastRecord>>>,
    },
    GetForecast {
        forecast_id: String,
        resp: oneshot::Sender<Result<Option<ForecastRecord>>>,
    },
    GetCategories {
        resp: oneshot::Sender<Result<Vec<CategoryRecord>>>,
    },
    GetAccounts {
        resp: oneshot::Sender<Result<Vec<AccountRecord>>>,
    },
    GetTransactions {
        from: NaiveDate,
        to: NaiveDate,
        resp: oneshot::Sender<Result<Vec<TransactionRecord>>>,
    },
}

use finance_core::models::TransactionRecord;

async fn store_actor_loop(
    store: Box<dyn FinanceStore>,
    mut rx: mpsc::UnboundedReceiver<StoreRequest>,
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
            StoreRequest::UpsertForecast { mut record, resp } => {
                let actor_id = record.actor_id.clone();
                let result: Result<String> = async {
                    if record.forecast_id.is_empty() {
                        record.forecast_id = Uuid::now_v7().to_string();
                    }
                    ensure_forecast_idempotency(&mut record).context("idempotency")?;
                    let forecast_id = record.forecast_id.clone();
                    store
                        .upsert_forecasts(&[record])
                        .await
                        .context("upsert_forecasts")?;
                    // Write a minimal audit event.
                    let event = AuditEvent {
                        event_id: Uuid::now_v7().to_string(),
                        entity_type: "forecast".into(),
                        entity_id: forecast_id.clone(),
                        action: "upsert".into(),
                        actor_id,
                        event_timestamp: Utc::now(),
                        idempotency_key: Uuid::now_v7().to_string(),
                        diff_json: Value::Object(Default::default()),
                    };
                    store.insert_audit_events(&[event]).await.context("audit")?;
                    Ok(forecast_id)
                }
                .await;
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
            StoreRequest::GetForecast { forecast_id, resp } => {
                let result = store.get_forecast(&forecast_id).await;
                let _ = resp.send(result);
            }
            StoreRequest::GetCategories { resp } => {
                let result = store.get_categories().await;
                let _ = resp.send(result);
            }
            StoreRequest::GetAccounts { resp } => {
                let result = store.get_accounts().await;
                let _ = resp.send(result);
            }
            StoreRequest::GetTransactions { from, to, resp } => {
                let result = store.transactions_in_date_range(None, from, to).await;
                let _ = resp.send(result);
            }
        }
    }
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
    let count = materialise_template_forecasts(
        store,
        &template,
        materialize_months,
        "serve-dashboard",
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
    Ok(())
}

// ── WebSocket protocol ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct WsRequest {
    id: String,
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Serialize)]
struct WsResponse {
    id: String,
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl WsResponse {
    fn success(id: String, msg_type: &str, payload: Value) -> Self {
        Self {
            id,
            msg_type: msg_type.into(),
            payload: Some(payload),
            error: None,
        }
    }

    fn error(id: String, msg: &str) -> Self {
        Self {
            id,
            msg_type: "error".into(),
            payload: None,
            error: Some(msg.into()),
        }
    }
}

async fn handle_socket(socket: WebSocket, tx: mpsc::UnboundedSender<StoreRequest>) {
    let (mut sender, mut receiver) = socket.split();

    while let Some(Ok(msg)) = receiver.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };

        let request: WsRequest = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                let resp = WsResponse::error("unknown".into(), &e.to_string());
                let _ = sender
                    .send(Message::Text(
                        serde_json::to_string(&resp).unwrap_or_default().into(),
                    ))
                    .await;
                continue;
            }
        };

        let response = process_request(request, &tx).await;
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&response).unwrap_or_default().into(),
            ))
            .await;
    }
}

async fn process_request(req: WsRequest, tx: &mpsc::UnboundedSender<StoreRequest>) -> WsResponse {
    match req.msg_type.as_str() {
        "get_chart_data" => {
            let months_back = req.payload["months_back"].as_u64().unwrap_or(6) as usize;
            let months_ahead = req.payload["months_ahead"].as_u64().unwrap_or(6) as usize;
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::GetChartData {
                    months_back,
                    months_ahead,
                    resp: resp_tx,
                })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(chart_data)) => WsResponse::success(
                    req.id,
                    "chart_data",
                    serde_json::to_value(chart_data).unwrap_or_default(),
                ),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        "list_forecast_templates" => {
            let kind = req.payload["kind"].as_str().map(|s| s.to_string());
            let status = req.payload["status"].as_str().map(|s| s.to_string());
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::ListForecastTemplates {
                    kind,
                    status,
                    resp: resp_tx,
                })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(templates)) => WsResponse::success(
                    req.id,
                    "forecast_templates",
                    serde_json::json!({ "templates": templates }),
                ),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        "accept_template" => {
            let Some(template_id) = req.payload["template_id"].as_str() else {
                return WsResponse::error(req.id, "template_id é obrigatório");
            };
            let materialize_months = req.payload["materialize_months"].as_u64().unwrap_or(6) as u32;
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::AcceptTemplate {
                    template_id: template_id.into(),
                    materialize_months,
                    resp: resp_tx,
                })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(result)) => WsResponse::success(req.id, "template_accepted", result),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        "dismiss_template" => {
            let Some(template_id) = req.payload["template_id"].as_str() else {
                return WsResponse::error(req.id, "template_id é obrigatório");
            };
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::DismissTemplate {
                    template_id: template_id.into(),
                    resp: resp_tx,
                })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(())) => WsResponse::success(
                    req.id,
                    "template_dismissed",
                    serde_json::json!({ "template_id": template_id }),
                ),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        "upsert_forecast" => {
            let description = req.payload["description"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let amount_str = req.payload["amount"].as_str().unwrap_or("0");
            let amount = Decimal::from_str(amount_str).unwrap_or(Decimal::ZERO);
            let due_date = req.payload["due_date"]
                .as_str()
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
            let record = ForecastRecord {
                forecast_id: String::new(),
                due_date,
                description,
                amount,
                category_id: req.payload["category_id"].as_str().map(|s| s.to_string()),
                account_id: req.payload["account_id"].as_str().map(|s| s.to_string()),
                status: "ativo".into(),
                recurrence: None,
                actor_id: "serve-dashboard".into(),
                idempotency_key: String::new(),
                metadata_json: Value::Object(Default::default()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                template_id: None,
                realized_transaction_id: None,
                realized_at: None,
            };
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::UpsertForecast {
                    record,
                    resp: resp_tx,
                })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(forecast_id)) => WsResponse::success(
                    req.id,
                    "forecast_upserted",
                    serde_json::json!({ "forecast_id": forecast_id }),
                ),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        "list_forecasts" => {
            let status = req.payload["status"].as_str().map(|s| s.to_string());
            let from = req.payload["from"]
                .as_str()
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
            let until = req.payload["until"]
                .as_str()
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::ListForecasts {
                    status,
                    from,
                    until,
                    resp: resp_tx,
                })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(forecasts)) => WsResponse::success(
                    req.id,
                    "forecasts",
                    serde_json::json!({ "forecasts": forecasts }),
                ),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        "get_forecast" => {
            let Some(forecast_id) = req.payload["forecast_id"].as_str() else {
                return WsResponse::error(req.id, "forecast_id é obrigatório");
            };
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::GetForecast {
                    forecast_id: forecast_id.into(),
                    resp: resp_tx,
                })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(fc)) => WsResponse::success(
                    req.id,
                    "forecast",
                    serde_json::to_value(fc).unwrap_or_default(),
                ),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        "get_categories" => {
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::GetCategories { resp: resp_tx })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(cats)) => WsResponse::success(
                    req.id,
                    "categories",
                    serde_json::json!({ "categories": cats }),
                ),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        "get_accounts" => {
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::GetAccounts { resp: resp_tx })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(accounts)) => WsResponse::success(
                    req.id,
                    "accounts",
                    serde_json::json!({ "accounts": accounts }),
                ),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        "get_transactions" => {
            let Some(from_str) = req.payload["from"].as_str() else {
                return WsResponse::error(req.id, "from é obrigatório");
            };
            let Some(to_str) = req.payload["to"].as_str() else {
                return WsResponse::error(req.id, "to é obrigatório");
            };
            let Ok(from) = NaiveDate::parse_from_str(from_str, "%Y-%m-%d") else {
                return WsResponse::error(req.id, "from inválido (use YYYY-MM-DD)");
            };
            let Ok(to) = NaiveDate::parse_from_str(to_str, "%Y-%m-%d") else {
                return WsResponse::error(req.id, "to inválido (use YYYY-MM-DD)");
            };
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(StoreRequest::GetTransactions {
                    from,
                    to,
                    resp: resp_tx,
                })
                .is_err()
            {
                return WsResponse::error(req.id, "store actor morreu");
            }
            match resp_rx.await {
                Ok(Ok(txs)) => WsResponse::success(
                    req.id,
                    "transactions",
                    serde_json::json!({ "transactions": txs }),
                ),
                Ok(Err(e)) => WsResponse::error(req.id, &e.to_string()),
                Err(_) => WsResponse::error(req.id, "store actor não respondeu"),
            }
        }
        _ => WsResponse::error(req.id, &format!("tipo desconhecido: {}", req.msg_type)),
    }
}

// ── HTTP handlers ────────────────────────────────────────────────────────

async fn dashboard_page() -> impl IntoResponse {
    Html(include_str!("serve_dashboard.html"))
}

// ── Entry point ──────────────────────────────────────────────────────────

pub async fn run(port: u16, host: &str) -> Result<()> {
    let (_, config) = load_config().await?;
    let config: AppConfig = config;

    // Build the channel before entering LocalSet.
    let (store_tx, store_rx) = mpsc::unbounded_channel::<StoreRequest>();

    let local = LocalSet::new();

    // Spawn the !Send store actor on the local set.
    local.spawn_local(async move {
        let store = open_store(&config).await?;
        run_migrations(store.as_ref(), &config).await?;
        store_actor_loop(store, store_rx).await;
        Ok::<_, anyhow::Error>(())
    });

    let app_state = Arc::new(store_tx);

    let app = Router::new()
        .route("/", get(dashboard_page))
        .route("/ws", get(ws_handler))
        .with_state(app_state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("falha ao escutar em {addr}"))?;

    println!("🌐 Dashboard em http://{addr}");
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

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(tx): State<Arc<mpsc::UnboundedSender<StoreRequest>>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, (*tx).clone()))
}
