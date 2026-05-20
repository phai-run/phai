//! Pipeline orchestrator for transaction enrichment.
//!
//! Glues together the Phase 1 + Phase 2 building blocks:
//!   1. Extract CNPJ + lookup BrasilAPI (2-layer cache)
//!   2. Map Pluggy coarse category → `CategoryHint`
//!   3. Build temporal context (sibling transactions on the same day)
//!   4. Pick up to 3 user-curated few-shot examples
//!   5. Compute pre-LLM heuristics (round number, hour bucket, recurring)
//!   6. Build the PT-BR prompt and call the configured LLM
//!   7. Wrap the result in an [`EnrichmentDecision`] via threshold
//!
//! Failure modes:
//!   - CNPJ lookup errors are logged to stderr and the pipeline
//!     continues with the remaining signals.
//!   - similar_transactions / temporal_context errors degrade
//!     gracefully — empty inputs are acceptable.
//!   - LLM errors bubble up (`anyhow::Error`).

use crate::config::AppConfig;
use crate::enrichment::cnpj::{extract_cnpj, lookup_cnpj};
use crate::enrichment::fuzzy::fuzzy_filter;
use crate::enrichment::heuristics::{base_heuristics, detect_recurring};
use crate::enrichment::llm::{enrich as llm_enrich, LlmProvider};
use crate::enrichment::pluggy_map::map_pluggy_category;
use crate::enrichment::prompt::{build_prompt, clean_description, PromptContext};
use crate::enrichment::types::{
    CategoryHint, CnpjInfo, ContextTx, EnrichmentDecision, EnrichmentResult, FewShotExample,
    Heuristics,
};
use crate::enrichment::web_search::ddg_merchant_context;
use crate::models::{TransactionRecord, UncategorizedRow};
use crate::storage::FinanceStore;
use anyhow::{Context, Result};
use chrono::{Datelike, Timelike};
use moka::future::Cache;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

const MOKA_CAPACITY: u64 = 50_000;
const MOKA_TTL_SECS: u64 = 24 * 60 * 60; // 24 hours
const MOKA_TTI_SECS: u64 = 4 * 60 * 60; // 4 hours
const HTTP_TIMEOUT_SECS: u64 = 10;
const FEW_SHOT_LIMIT: usize = 3;
const TEMPORAL_CONTEXT_LIMIT: usize = 8;

/// User-curated category sources used to filter few-shot examples.
const FEW_SHOT_SOURCES: [&str; 2] = ["manual", "enriched:user"];

/// Enrichment pipeline. Owns the moka L1 cache so it is shared across
/// `run_one` calls within a single CLI invocation. The L2 SQLite cache
/// lives at `sqlite_path` and persists across runs.
pub struct EnrichmentPipeline {
    pub(crate) http: reqwest::Client,
    pub(crate) moka_cache: Cache<String, Option<CnpjInfo>>,
    pub(crate) sqlite_path: PathBuf,
    pub(crate) provider: LlmProvider,
}

impl EnrichmentPipeline {
    /// Construct a pipeline. The LLM provider is resolved from env +
    /// config via [`LlmProvider::from_env_or_config`].
    pub fn new(config: &AppConfig) -> Result<Self> {
        let provider =
            LlmProvider::from_env_or_config(config).context("falha ao resolver provedor LLM")?;
        Self::with_provider(config, provider)
    }

    /// Construct with an explicit provider (used by tests and CLI
    /// overrides).
    pub fn with_provider(config: &AppConfig, provider: LlmProvider) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
            .context("falha ao montar reqwest client")?;
        let moka_cache = Cache::builder()
            .max_capacity(MOKA_CAPACITY)
            .time_to_live(Duration::from_secs(MOKA_TTL_SECS))
            .time_to_idle(Duration::from_secs(MOKA_TTI_SECS))
            .build();
        let sqlite_path = config
            .local_db_path
            .clone()
            .context("local_db_path não configurado — necessário para cnpj_cache L2")?;
        Ok(Self {
            http,
            moka_cache,
            sqlite_path,
            provider,
        })
    }

    /// Run the full enrichment pipeline for one transaction.
    pub async fn run_one(
        &self,
        tx: &UncategorizedRow,
        store: &dyn FinanceStore,
    ) -> Result<EnrichmentDecision> {
        let signals = self.gather_signals(tx, store).await;
        let prompt = build_prompt_from_signals(tx, &signals);
        let mut result = llm_enrich(&self.provider, &prompt)
            .await
            .context("falha ao chamar LLM para enriquecimento")?;
        // Apply pluggy confidence_boost if LLM agreed with the hint.
        apply_pluggy_boost(&mut result, &signals.pluggy_hint);
        Ok(EnrichmentDecision::from_result(result))
    }

    /// Run the pipeline over a batch of transaction ids (used by the
    /// post-sync hook in Phase 5 — for Phase 3 this is just a helper).
    pub async fn run_batch(
        &self,
        tx_ids: &[String],
        store: &dyn FinanceStore,
    ) -> Result<Vec<(String, EnrichmentDecision)>> {
        let mut out = Vec::with_capacity(tx_ids.len());
        for id in tx_ids {
            let record = match store.transaction_by_id(id).await? {
                Some(r) => r,
                None => continue,
            };
            // Adapt TransactionRecord → UncategorizedRow for `run_one`.
            let row = UncategorizedRow {
                transaction_id: record.transaction_id.clone(),
                transaction_date: record.transaction_date,
                description: record.raw_description.clone(),
                amount: record.amount,
                account_id: record.account_id.clone(),
                account_label: None,
                tx_type: record.tx_type.clone(),
                category_source: record.category_source.clone(),
                payment_status: record.payment_status.clone(),
                source: record.source.clone(),
                metadata_json: record.metadata_json.clone(),
            };
            let decision = self.run_one(&row, store).await?;
            out.push((record.transaction_id, decision));
        }
        Ok(out)
    }

    /// Retroactive match search: SQL substring prefilter via
    /// `similar_transactions`, then nucleo fuzzy re-scoring with a
    /// percentage threshold. Returns `(record, score)` sorted by score
    /// descending. Only uncategorized / weakly-categorized rows are
    /// considered so we never overwrite a user's manual decision.
    pub async fn find_retroactive_matches(
        &self,
        store: &dyn FinanceStore,
        keyword: &str,
        exclude_id: &str,
        threshold_percent: u8,
    ) -> Result<Vec<(TransactionRecord, u32)>> {
        let candidates = store
            .similar_transactions(keyword, exclude_id, true)
            .await
            .context("similar_transactions falhou no find_retroactive_matches")?;
        Ok(fuzzy_filter(keyword, candidates, threshold_percent))
    }

    /// Pure-data signal gathering. Exposed for tests so the prompt
    /// building stage can be exercised without invoking the LLM.
    pub(crate) async fn gather_signals(
        &self,
        tx: &UncategorizedRow,
        store: &dyn FinanceStore,
    ) -> GatheredSignals {
        // 1. CNPJ extraction + lookup.
        let cnpj_extracted = extract_cnpj(&tx.metadata_json);
        let cnpj_info = match &cnpj_extracted {
            Some(cnpj) => {
                match lookup_cnpj(&self.http, &self.moka_cache, &self.sqlite_path, cnpj).await {
                    Ok(info) => info,
                    Err(err) => {
                        eprintln!("aviso: cnpj lookup falhou ({err:#}); seguindo sem cnpj");
                        None
                    }
                }
            }
            None => None,
        };

        // 2. Pluggy hint.
        let pluggy_category = tx
            .metadata_json
            .pointer("/pluggy_category")
            .and_then(Value::as_str)
            .map(str::to_string);
        let pluggy_hint = pluggy_category
            .as_deref()
            .map(map_pluggy_category)
            .unwrap_or_else(CategoryHint::empty);

        // 3. Temporal context.
        let temporal_context = if let Some(account_id) = tx.account_id.as_deref() {
            store
                .transactions_on_date(tx.transaction_date, account_id, &tx.transaction_id)
                .await
                .unwrap_or_else(|err| {
                    eprintln!("aviso: transactions_on_date falhou: {err:#}");
                    Vec::new()
                })
        } else {
            Vec::new()
        };
        let temporal_context: Vec<ContextTx> = temporal_context
            .into_iter()
            .take(TEMPORAL_CONTEXT_LIMIT)
            .collect();

        // 4. Few-shot examples: similar transactions already curated by user.
        let merchant_token = derive_merchant_token(&tx.description);
        let few_shot = match merchant_token.as_deref() {
            Some(tok) if !tok.is_empty() => {
                match store
                    .similar_transactions(tok, &tx.transaction_id, false)
                    .await
                {
                    Ok(rows) => rows
                        .into_iter()
                        .filter(|t| {
                            t.category_id.is_some()
                                && FEW_SHOT_SOURCES.contains(&t.category_source.as_str())
                        })
                        .take(FEW_SHOT_LIMIT)
                        .map(|t| {
                            let (cat, sub) = split_category_id(t.category_id.as_deref());
                            FewShotExample {
                                description: t.raw_description.clone(),
                                amount: t.amount,
                                category: cat,
                                subcategory: sub,
                            }
                        })
                        .collect(),
                    Err(err) => {
                        eprintln!("aviso: similar_transactions (few-shot) falhou: {err:#}");
                        Vec::new()
                    }
                }
            }
            _ => Vec::new(),
        };

        // 5. Heuristics.
        let hour = extract_hour(&tx.metadata_json);
        let weekday = tx.transaction_date.weekday();
        let mut heuristics = base_heuristics(tx.amount, hour, weekday);
        heuristics.is_recurring = detect_recurring(
            store,
            cnpj_extracted.as_deref(),
            tx.amount,
            &tx.transaction_id,
        )
        .await;

        // 6. Receiver name + document type for the prompt.
        let receiver_name = tx
            .metadata_json
            .pointer("/raw/paymentData/receiver/name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let document_type = tx
            .metadata_json
            .pointer("/raw/paymentData/receiver/documentNumber/type")
            .and_then(Value::as_str)
            .map(str::to_string);

        // 7. Web search for unknown merchants. CNPJ lookup is authoritative
        // and faster, so DDG is only used when CNPJ has no answer.
        let web_context =
            web_context_for_unknown_merchant(&self.http, &tx.description, cnpj_info.is_some())
                .await;

        GatheredSignals {
            pluggy_category,
            pluggy_hint,
            cnpj_info,
            receiver_name,
            document_type,
            heuristics,
            temporal_context,
            few_shot,
            hour,
            web_context,
        }
    }
}

async fn web_context_for_unknown_merchant(
    http: &reqwest::Client,
    description: &str,
    has_cnpj_info: bool,
) -> Option<String> {
    if has_cnpj_info {
        return None;
    }
    let query = clean_description(description);
    if query.trim().len() < 4 {
        return None;
    }
    ddg_merchant_context(http, &query).await
}

/// Output of [`EnrichmentPipeline::gather_signals`].
pub(crate) struct GatheredSignals {
    pub pluggy_category: Option<String>,
    pub pluggy_hint: CategoryHint,
    pub cnpj_info: Option<CnpjInfo>,
    pub receiver_name: Option<String>,
    pub document_type: Option<String>,
    pub heuristics: Heuristics,
    pub temporal_context: Vec<ContextTx>,
    pub few_shot: Vec<FewShotExample>,
    pub hour: Option<u32>,
    /// DuckDuckGo instant-answer snippet for unknown merchants (None when
    /// CNPJ info is already available or DDG returns nothing).
    pub web_context: Option<String>,
}

fn build_prompt_from_signals(tx: &UncategorizedRow, s: &GatheredSignals) -> String {
    let ctx = PromptContext {
        description: &tx.description,
        amount: tx.amount,
        date: tx.transaction_date,
        hour: s.hour,
        pluggy_category: s.pluggy_category.as_deref(),
        pluggy_hint: Some(&s.pluggy_hint),
        cnpj_info: s.cnpj_info.as_ref(),
        receiver_name: s.receiver_name.as_deref(),
        document_type: s.document_type.as_deref(),
        heuristics: &s.heuristics,
        temporal_context: &s.temporal_context,
        few_shot_examples: &s.few_shot,
        web_context: s.web_context.as_deref(),
    };
    build_prompt(&ctx)
}

fn apply_pluggy_boost(result: &mut EnrichmentResult, hint: &CategoryHint) {
    if let Some(cat) = hint.category {
        if result.category == cat {
            // Only boost when LLM picked the same broad category.
            let new = (result.confidence + hint.confidence_boost).min(1.0);
            result.confidence = new;
        }
    }
}

/// Crude merchant token: longest alphanumeric word in the description
/// (≥4 chars) that is not a finance stop-word. Used to seed
/// `similar_transactions` for few-shot mining.
fn derive_merchant_token(description: &str) -> Option<String> {
    const NOISE: &[&str] = &[
        "PIX",
        "TRANSFERENCIA",
        "TRANSFERÊNCIA",
        "PAGAMENTO",
        "COMPRA",
        "LTDA",
        "SA",
        "DEBITO",
        "DÉBITO",
        "CREDITO",
        "CRÉDITO",
        "ENVIADA",
        "RECEBIDA",
        "TRANSF",
    ];
    description
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 4)
        .filter(|t| {
            let upper = t.to_uppercase();
            !NOISE.contains(&upper.as_str())
        })
        .max_by_key(|t| t.len())
        .map(|t| t.to_lowercase())
}

/// Split a `cat:subcat` category id. Defaults subcat to `outros` when
/// missing (defensive — the column should always carry both parts).
fn split_category_id(category_id: Option<&str>) -> (String, String) {
    match category_id {
        Some(s) => {
            let mut parts = s.splitn(2, ':');
            let cat = parts.next().unwrap_or("outros").to_string();
            let sub = parts.next().unwrap_or("outros").to_string();
            (cat, sub)
        }
        None => ("outros".to_string(), "outros".to_string()),
    }
}

/// Best-effort hour extraction from transaction metadata (ISO 8601).
///
/// Priority: `creditCardMetadata.purchaseDate` (most precise — actual
/// purchase time) → `raw.date` → `raw.createdAt`.
fn extract_hour(metadata: &Value) -> Option<u32> {
    let raw = metadata
        .pointer("/raw/creditCardMetadata/purchaseDate")
        .or_else(|| metadata.pointer("/raw/date"))
        .or_else(|| metadata.pointer("/raw/createdAt"))
        .and_then(Value::as_str)?;
    let parsed = chrono::DateTime::parse_from_rfc3339(raw).ok()?;
    Some(parsed.hour())
}

/// Mark `enrichment_attempted_at` on a transaction. Thin wrapper around
/// the store method so callers don't have to thread the actor_id /
/// idempotency_key plumbing.
pub async fn mark_attempted(
    store: &dyn FinanceStore,
    transaction_id: &str,
    actor_id: &str,
) -> Result<()> {
    let idempotency_key = format!("enrich_attempted:{transaction_id}:{}", uuid::Uuid::now_v7());
    store
        .mark_enrichment_attempted(transaction_id, actor_id, &idempotency_key)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrichment::types::{AUTO_THRESHOLD, SUGGEST_THRESHOLD};
    use crate::models::AuditEvent;
    use crate::models::{
        AccountRecord, AccountSnapshotRecord, BudgetStatusRow, CardClosedTransactionRow,
        CardSummaryRow, CashflowRow, CategoryBudgetRecord, CategoryRecord, DailyPulseItem,
        ForecastRecord, ForecastVsActualRow, MonthlySpendRow, RuleRecord, TransactionContextRow,
        TransactionRecord, UncategorizedRow,
    };
    use crate::splits::{
        ItemPriceRow, ReceiptItemRecord, SplitCandidateRow, TransactionSplitDetail,
        TransactionSplitLineRecord, TransactionSplitRecord,
    };
    use crate::storage::TransactionAnatomyPatch;
    use async_trait::async_trait;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    type AnnotateCall = (String, Option<String>, Option<String>);

    // ── Mock FinanceStore: only the methods used by pipeline matter ──
    #[derive(Default)]
    struct MockStore {
        pub on_date: Vec<ContextTx>,
        pub similar: Vec<TransactionRecord>,
        pub mark_attempted_calls: Mutex<Vec<String>>,
        pub annotated: Mutex<Vec<AnnotateCall>>,
    }

    #[async_trait(?Send)]
    impl FinanceStore for MockStore {
        async fn applied_migrations(&self) -> Result<BTreeSet<String>> {
            Ok(BTreeSet::new())
        }
        async fn apply_sql(&self, _sql: &str) -> Result<()> {
            Ok(())
        }
        async fn record_migration(&self, _v: &str) -> Result<()> {
            Ok(())
        }
        async fn upsert_accounts(&self, _: &[AccountRecord]) -> Result<usize> {
            Ok(0)
        }
        async fn get_accounts(&self) -> Result<Vec<AccountRecord>> {
            Ok(vec![])
        }
        async fn insert_account_snapshots(&self, _: &[AccountSnapshotRecord]) -> Result<usize> {
            Ok(0)
        }
        async fn upsert_transactions(&self, _: &[TransactionRecord]) -> Result<usize> {
            Ok(0)
        }
        async fn upsert_rules(&self, _: &[RuleRecord]) -> Result<usize> {
            Ok(0)
        }
        async fn upsert_categories(&self, _: &[CategoryRecord]) -> Result<usize> {
            Ok(0)
        }
        async fn upsert_forecasts(&self, _: &[ForecastRecord]) -> Result<usize> {
            Ok(0)
        }
        async fn upcoming_forecasts(
            &self,
            _: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<ForecastRecord>> {
            Ok(vec![])
        }
        async fn cards_open_now(&self) -> Result<Vec<crate::models::CardSummaryRow>> {
            Ok(vec![])
        }
        async fn latest_account_snapshots(&self) -> Result<Vec<AccountSnapshotRecord>> {
            Ok(vec![])
        }
        async fn apply_transaction_split(
            &self,
            _split: &TransactionSplitRecord,
            _lines: &[TransactionSplitLineRecord],
            _items: &[ReceiptItemRecord],
        ) -> Result<()> {
            Ok(())
        }
        async fn insert_audit_events(&self, _: &[AuditEvent]) -> Result<usize> {
            Ok(0)
        }
        async fn annotate_transaction(
            &self,
            transaction_id: &str,
            category_id: Option<&str>,
            category_source: Option<&str>,
            _context: Option<&str>,
            _actor_id: &str,
            _idempotency_key: &str,
        ) -> Result<()> {
            self.annotated.lock().unwrap().push((
                transaction_id.to_string(),
                category_id.map(|s| s.to_string()),
                category_source.map(|s| s.to_string()),
            ));
            Ok(())
        }
        async fn update_transaction_anatomy(
            &self,
            _: &str,
            _: TransactionAnatomyPatch<'_>,
            _: &str,
            _: &str,
        ) -> Result<()> {
            Ok(())
        }
        async fn find_transactions_by_description(
            &self,
            _: &str,
            _: usize,
        ) -> Result<Vec<TransactionRecord>> {
            Ok(vec![])
        }
        async fn latest_uncategorized_transactions(
            &self,
            _: usize,
        ) -> Result<Vec<TransactionRecord>> {
            Ok(vec![])
        }
        async fn pending_human_descriptions(&self, _: usize) -> Result<Vec<TransactionRecord>> {
            Ok(vec![])
        }
        async fn pending_merchants(&self, _: usize) -> Result<Vec<TransactionRecord>> {
            Ok(vec![])
        }
        async fn pending_purposes(&self, _: Decimal, _: usize) -> Result<Vec<TransactionRecord>> {
            Ok(vec![])
        }
        async fn count_pending_human_descriptions(&self) -> Result<i64> {
            Ok(0)
        }
        async fn count_pending_merchants(&self) -> Result<i64> {
            Ok(0)
        }
        async fn count_pending_purposes(&self, _: Decimal) -> Result<i64> {
            Ok(0)
        }
        async fn existing_transaction_ids(&self, _: &[String]) -> Result<BTreeSet<String>> {
            Ok(BTreeSet::new())
        }
        async fn transaction_by_id(&self, _: &str) -> Result<Option<TransactionRecord>> {
            Ok(None)
        }
        async fn transaction_split_detail(
            &self,
            _: &str,
        ) -> Result<Option<TransactionSplitDetail>> {
            Ok(None)
        }
        async fn clear_transaction_split(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> Result<()> {
            Ok(())
        }
        async fn split_candidates(&self, _: NaiveDate) -> Result<Vec<SplitCandidateRow>> {
            Ok(vec![])
        }
        async fn item_prices(&self, _: &str, _: Option<NaiveDate>) -> Result<Vec<ItemPriceRow>> {
            Ok(vec![])
        }
        async fn all_rules(&self) -> Result<Vec<RuleRecord>> {
            Ok(vec![])
        }
        async fn active_rules(&self) -> Result<Vec<RuleRecord>> {
            Ok(vec![])
        }
        async fn internal_categories(&self) -> Result<BTreeSet<String>> {
            Ok(BTreeSet::new())
        }
        async fn transactions_with_context(&self, _: usize) -> Result<Vec<TransactionContextRow>> {
            Ok(vec![])
        }
        async fn count_transactions_with_context(&self) -> Result<i64> {
            Ok(0)
        }
        async fn latest_pluggy_transaction_date(&self) -> Result<Option<NaiveDate>> {
            Ok(None)
        }
        async fn daily_pulse(&self, _: NaiveDate) -> Result<Vec<DailyPulseItem>> {
            Ok(vec![])
        }
        async fn effective_transactions_window(
            &self,
            _: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<TransactionRecord>> {
            Ok(vec![])
        }
        async fn transactions_in_date_range(
            &self,
            _: Option<&str>,
            _: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<TransactionRecord>> {
            Ok(vec![])
        }
        async fn monthly_spend(&self, _: Option<&str>) -> Result<Vec<MonthlySpendRow>> {
            Ok(vec![])
        }
        async fn cashflow(&self, _: usize) -> Result<Vec<CashflowRow>> {
            Ok(vec![])
        }
        async fn forecast_vs_actual(&self, _: Option<&str>) -> Result<Vec<ForecastVsActualRow>> {
            Ok(vec![])
        }
        async fn card_summary(&self, _: Option<&str>) -> Result<Vec<CardSummaryRow>> {
            Ok(vec![])
        }
        async fn card_closed_transactions(
            &self,
            _: Option<&str>,
        ) -> Result<Vec<CardClosedTransactionRow>> {
            Ok(vec![])
        }
        async fn card_reportable_transactions(
            &self,
            _: Option<&str>,
        ) -> Result<Vec<CardClosedTransactionRow>> {
            Ok(vec![])
        }
        async fn uncategorized(&self, _: usize) -> Result<Vec<UncategorizedRow>> {
            Ok(vec![])
        }
        async fn count_uncategorized(&self) -> Result<i64> {
            Ok(0)
        }
        async fn count_rows(&self, _: &str) -> Result<i64> {
            Ok(0)
        }
        async fn upsert_category_budget(&self, _: &CategoryBudgetRecord) -> Result<()> {
            Ok(())
        }
        async fn list_category_budgets(
            &self,
            _: Option<&str>,
        ) -> Result<Vec<CategoryBudgetRecord>> {
            Ok(vec![])
        }
        async fn budget_status_for_month(&self, _: &str) -> Result<Vec<BudgetStatusRow>> {
            Ok(vec![])
        }
        async fn transactions_on_date(
            &self,
            _: NaiveDate,
            _: &str,
            _: &str,
        ) -> Result<Vec<ContextTx>> {
            Ok(self.on_date.clone())
        }
        async fn similar_transactions(
            &self,
            _: &str,
            _: &str,
            _: bool,
        ) -> Result<Vec<TransactionRecord>> {
            Ok(self.similar.clone())
        }
        async fn mark_enrichment_attempted(
            &self,
            transaction_id: &str,
            _actor_id: &str,
            _idempotency_key: &str,
        ) -> Result<()> {
            self.mark_attempted_calls
                .lock()
                .unwrap()
                .push(transaction_id.to_string());
            Ok(())
        }
    }

    fn build_pipeline(tmpdir: &tempfile::TempDir) -> EnrichmentPipeline {
        let config = AppConfig {
            local_db_path: Some(tmpdir.path().join("test.db")),
            ..AppConfig::default()
        };
        // Use a fake Ollama provider — these tests never call enrich(),
        // they exercise gather_signals + build_prompt_from_signals only.
        let provider = LlmProvider::Ollama {
            base_url: "http://127.0.0.1:1".to_string(),
            model: "test-model".to_string(),
        };
        EnrichmentPipeline::with_provider(&config, provider).unwrap()
    }

    fn cnpj_tx() -> UncategorizedRow {
        UncategorizedRow {
            transaction_id: "tx-1".to_string(),
            transaction_date: NaiveDate::from_ymd_opt(2026, 5, 3).unwrap(),
            description: "PIX SAPIENS LTDA".to_string(),
            amount: Decimal::new(-2590, 2),
            account_id: Some("acc-1".to_string()),
            account_label: None,
            tx_type: "debit".to_string(),
            category_source: "unclassified".to_string(),
            payment_status: "confirmed".to_string(),
            source: "pluggy".to_string(),
            metadata_json: json!({
                "pluggy_category": "Eating out",
                "raw": {
                    "paymentData": {
                        "receiver": {
                            "name": "SAPIENS PARQUE LTDA",
                            "documentNumber": {
                                "type": "CNPJ",
                                "value": "12.345.678/0001-90"
                            }
                        }
                    },
                    "order": 1
                }
            }),
        }
    }

    fn tx_record(id: &str, desc: &str, cents: i64, source: &str) -> TransactionRecord {
        TransactionRecord {
            transaction_id: id.to_string(),
            account_id: None,
            transaction_date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            raw_description: desc.to_string(),
            description: None,
            merchant_name: None,
            purpose: None,
            amount: Decimal::new(cents, 2),
            tx_type: "debit".to_string(),
            category_id: Some("alimentacao:restaurantes".to_string()),
            category_source: source.to_string(),
            context: None,
            classifier_trace: None,
            payment_status: "confirmed".to_string(),
            source: "pluggy".to_string(),
            actor_id: "u".to_string(),
            idempotency_key: "k".to_string(),
            metadata_json: json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            enrichment_attempted_at: None,
            amount_cents: None,
        }
    }

    fn cpf_tx() -> UncategorizedRow {
        let mut tx = cnpj_tx();
        tx.metadata_json = json!({
            "raw": {
                "paymentData": {
                    "receiver": {
                        "name": "JOICE ANTONIA",
                        "documentNumber": {
                            "type": "CPF",
                            "value": "123.456.789-09"
                        }
                    }
                }
            }
        });
        tx
    }

    #[tokio::test]
    async fn test_pipeline_gather_signals_cpf_no_cnpj_lookup() {
        let tmp = tempfile::tempdir().unwrap();
        let pipeline = build_pipeline(&tmp);
        let store = MockStore::default();
        let tx = cpf_tx();
        let signals = pipeline.gather_signals(&tx, &store).await;
        assert!(
            signals.cnpj_info.is_none(),
            "CPF must not trigger BrasilAPI"
        );
        assert_eq!(signals.document_type.as_deref(), Some("CPF"));
        assert_eq!(signals.receiver_name.as_deref(), Some("JOICE ANTONIA"));
    }

    #[tokio::test]
    async fn test_pipeline_gather_signals_temporal_context_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let pipeline = build_pipeline(&tmp);
        let store = MockStore {
            on_date: vec![
                ContextTx {
                    description: "Oliva Cheese Bar".to_string(),
                    amount: Decimal::new(-4500, 2),
                    pluggy_category: Some("Eating out".to_string()),
                    order: Some(0),
                },
                ContextTx {
                    description: "Brasil Berry".to_string(),
                    amount: Decimal::new(-3100, 2),
                    pluggy_category: None,
                    order: Some(2),
                },
            ],
            ..MockStore::default()
        };
        let tx = cpf_tx();
        let signals = pipeline.gather_signals(&tx, &store).await;
        assert_eq!(signals.temporal_context.len(), 2);
        assert!(signals.temporal_context[0].description.contains("Oliva"));
    }

    #[tokio::test]
    async fn test_pipeline_gather_signals_pluggy_hint_applied() {
        let tmp = tempfile::tempdir().unwrap();
        let pipeline = build_pipeline(&tmp);
        let store = MockStore::default();
        let tx = cnpj_tx();
        let signals = pipeline.gather_signals(&tx, &store).await;
        assert_eq!(signals.pluggy_category.as_deref(), Some("Eating out"));
        assert_eq!(signals.pluggy_hint.category, Some("alimentacao"));
        assert_eq!(signals.pluggy_hint.subcategory, Some("restaurantes"));
    }

    #[tokio::test]
    async fn test_pipeline_few_shot_filters_to_user_curated() {
        let tmp = tempfile::tempdir().unwrap();
        let pipeline = build_pipeline(&tmp);
        let store = MockStore {
            similar: vec![
                // Manual: included
                tx_record("old-1", "Sapiens Manual", -3000, "manual"),
                // Rule-based: excluded
                tx_record("old-2", "Sapiens Rule", -1500, "rule"),
                // enriched:user: included
                tx_record("old-3", "Sapiens Confirmed", -2200, "enriched:user"),
            ],
            ..MockStore::default()
        };
        let tx = cpf_tx();
        let signals = pipeline.gather_signals(&tx, &store).await;
        assert_eq!(signals.few_shot.len(), 2);
        assert!(signals
            .few_shot
            .iter()
            .all(|f| f.category == "alimentacao" && f.subcategory == "restaurantes"));
    }

    #[test]
    fn test_apply_pluggy_boost_only_on_agreement() {
        let mut r = EnrichmentResult {
            reasoning: "x".into(),
            merchant_name: "Sapiens".into(),
            category: "alimentacao".into(),
            subcategory: "restaurantes".into(),
            confidence: 0.80,
            needs_user_input: false,
            user_prompt: None,
        };
        let hint = CategoryHint {
            category: Some("alimentacao"),
            subcategory: Some("restaurantes"),
            confidence_boost: 0.10,
        };
        apply_pluggy_boost(&mut r, &hint);
        assert!((r.confidence - 0.90).abs() < 1e-6);

        // Disagreement: no boost.
        let mut r2 = EnrichmentResult {
            reasoning: "x".into(),
            merchant_name: "Sapiens".into(),
            category: "transporte".into(),
            subcategory: "combustivel".into(),
            confidence: 0.70,
            needs_user_input: false,
            user_prompt: None,
        };
        apply_pluggy_boost(&mut r2, &hint);
        assert!((r2.confidence - 0.70).abs() < 1e-6);
    }

    #[test]
    fn test_threshold_ladder_matches_pipeline_output() {
        // Threshold routing is what the pipeline ultimately returns.
        let auto = EnrichmentDecision::from_result(EnrichmentResult {
            reasoning: "x".into(),
            merchant_name: "m".into(),
            category: "c".into(),
            subcategory: "s".into(),
            confidence: AUTO_THRESHOLD + 0.01,
            needs_user_input: false,
            user_prompt: None,
        });
        assert!(matches!(auto, EnrichmentDecision::AutoApply { .. }));
        let suggest = EnrichmentDecision::from_result(EnrichmentResult {
            reasoning: "x".into(),
            merchant_name: "m".into(),
            category: "c".into(),
            subcategory: "s".into(),
            confidence: SUGGEST_THRESHOLD + 0.01,
            needs_user_input: false,
            user_prompt: None,
        });
        assert!(matches!(suggest, EnrichmentDecision::Suggest { .. }));
        let ask = EnrichmentDecision::from_result(EnrichmentResult {
            reasoning: "x".into(),
            merchant_name: "m".into(),
            category: "c".into(),
            subcategory: "s".into(),
            confidence: 0.10,
            needs_user_input: true,
            user_prompt: Some("ajuda?".into()),
        });
        assert!(matches!(ask, EnrichmentDecision::AskUser { .. }));
    }

    #[test]
    fn test_derive_merchant_token_picks_longest_non_noise() {
        assert_eq!(
            derive_merchant_token("PIX TRANSFERENCIA SAPIENS PARQUE LTDA").as_deref(),
            Some("sapiens")
        );
        assert_eq!(
            derive_merchant_token("COMPRA NO DEBITO BRASIL BERRY").as_deref(),
            Some("brasil")
        );
    }

    #[tokio::test]
    async fn test_find_retroactive_matches_threshold_respected() {
        let tmp = tempfile::tempdir().unwrap();
        let pipeline = build_pipeline(&tmp);
        let store = MockStore {
            similar: vec![
                tx_record("near", "Sapiens Parque Loja", -1000, "unclassified"),
                tx_record("far", "Posto Shell BR-101", -5000, "unclassified"),
            ],
            ..MockStore::default()
        };
        // High threshold filters out the unrelated row.
        let matches = pipeline
            .find_retroactive_matches(&store, "sapiens", "current", 60)
            .await
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0.transaction_id, "near");

        // Low threshold may keep both; in any case scores must be
        // sorted descending and the "near" row must come first.
        let matches = pipeline
            .find_retroactive_matches(&store, "sapiens", "current", 0)
            .await
            .unwrap();
        assert!(!matches.is_empty());
        for w in matches.windows(2) {
            assert!(w[0].1 >= w[1].1);
        }
        assert_eq!(matches[0].0.transaction_id, "near");
    }

    #[test]
    fn test_split_category_id() {
        assert_eq!(
            split_category_id(Some("alimentacao:restaurantes")),
            ("alimentacao".to_string(), "restaurantes".to_string())
        );
        assert_eq!(
            split_category_id(None),
            ("outros".to_string(), "outros".to_string())
        );
    }
}
