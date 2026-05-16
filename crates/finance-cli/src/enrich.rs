//! `finance tx enrich` — interactive + machine-readable enrichment CLI.
//!
//! Two interaction modes:
//!   - **Human** (default): renders each transaction with the LLM
//!     suggestion and asks [Y/n/s/c] (or free-text for low confidence).
//!   - **Machine** (`--machine`): emits one NDJSON decision per
//!     transaction to stdout (flushed), blocks reading one JSON line
//!     from stdin per transaction with a configurable timeout. Designed
//!     for OpenClaw / agent integration.
//!
//! Phase 3 deliberately does NOT create rules or apply retroactive
//! matching — those land in Phase 4.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{Duration as ChronoDuration, Utc};
use clap::Args;
use finance_core::config::AppConfig;
use finance_core::enrichment::llm::LlmProvider;
use finance_core::enrichment::pipeline::{mark_attempted, EnrichmentPipeline};
use finance_core::enrichment::types::{
    CnpjInfo, EnrichmentDecision, EnrichmentResult,
};
use finance_core::models::{AuditEvent, UncategorizedRow};
use finance_core::storage::FinanceStore;
use serde::{Deserialize, Serialize};
use std::io::Write as _;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{timeout, Duration as TokioDuration};

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_DAYS: i64 = 30;
const DEFAULT_MACHINE_TIMEOUT: u64 = 60;

#[derive(Args, Debug, Clone)]
pub struct EnrichArgs {
    /// Look back N days for uncategorized transactions.
    #[arg(long, default_value_t = DEFAULT_DAYS)]
    pub days: i64,

    /// Max transactions to process this run.
    #[arg(long, default_value_t = DEFAULT_LIMIT)]
    pub limit: usize,

    /// Show analysis without applying any changes.
    #[arg(long)]
    pub dry_run: bool,

    /// Apply all confidence >= AUTO_THRESHOLD without prompting.
    #[arg(long)]
    pub auto: bool,

    /// Re-process transactions that were already attempted.
    #[arg(long)]
    pub retry: bool,

    /// Override LLM provider (anthropic | openai | deepseek | ollama).
    #[arg(long)]
    pub provider: Option<String>,

    /// Override LLM model.
    #[arg(long)]
    pub model: Option<String>,

    /// Machine-readable output (NDJSON to stdout, JSON decisions from stdin).
    #[arg(long)]
    pub machine: bool,

    /// Timeout (seconds) waiting for a JSON decision from stdin in
    /// machine mode.
    #[arg(long, default_value_t = DEFAULT_MACHINE_TIMEOUT)]
    pub machine_timeout: u64,

    /// Process a single specific transaction by ID (skips the
    /// days/limit filter).
    #[arg(long)]
    pub transaction_id: Option<String>,
}

#[derive(Debug, Default)]
struct RunCounters {
    processed: usize,
    auto_applied: usize,
    confirmed: usize,
    skipped: usize,
    reviewed: usize,
}

/// Entry point invoked from `TxCommand::Enrich`. Walks the candidate
/// list, runs the pipeline, and dispatches to either the human or the
/// machine flow.
pub async fn run(
    args: EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
) -> Result<()> {
    // Provider/model overrides land as env vars so the pipeline picks
    // them up via `LlmProvider::from_env_or_config`.
    let mut overlay = config.clone();
    if let Some(p) = &args.provider {
        overlay.llm_provider = Some(p.clone());
    }
    if let Some(m) = &args.model {
        overlay.llm_model = Some(m.clone());
    }
    let provider = LlmProvider::from_env_or_config(&overlay)?;
    let pipeline = EnrichmentPipeline::with_provider(&overlay, provider)?;

    let candidates = collect_candidates(&args, store).await?;
    if candidates.is_empty() {
        if args.machine {
            emit_ndjson_line(&MachineOutput::Done {
                processed: 0,
                auto_applied: 0,
                confirmed: 0,
                skipped: 0,
                reviewed: 0,
            })?;
        } else {
            println!("Nenhuma transação elegível para enriquecimento.");
        }
        return Ok(());
    }

    let mut counters = RunCounters::default();

    if args.machine {
        run_machine(&args, config, store, &pipeline, &candidates, &mut counters).await?;
    } else {
        run_human(&args, config, store, &pipeline, &candidates, &mut counters).await?;
    }

    // Final summary.
    if args.machine {
        emit_ndjson_line(&MachineOutput::Done {
            processed: counters.processed,
            auto_applied: counters.auto_applied,
            confirmed: counters.confirmed,
            skipped: counters.skipped,
            reviewed: counters.reviewed,
        })?;
    } else {
        println!();
        println!("Processed: {}", counters.processed);
        println!("Auto-applied: {}", counters.auto_applied);
        println!("Confirmed: {}", counters.confirmed);
        println!("Skipped: {}", counters.skipped);
        println!("Marked for review: {}", counters.reviewed);
    }

    Ok(())
}

/// Fetch the working set of transactions, applying `--days`, `--limit`,
/// `--retry`, and `--transaction-id`.
async fn collect_candidates(
    args: &EnrichArgs,
    store: &dyn FinanceStore,
) -> Result<Vec<UncategorizedRow>> {
    if let Some(id) = &args.transaction_id {
        let record = store
            .transaction_by_id(id)
            .await?
            .ok_or_else(|| anyhow!("Transação {id} não encontrada"))?;
        if !args.retry && record.enrichment_attempted_at.is_some() {
            bail!(
                "Transação {id} já foi processada (use --retry para re-processar)"
            );
        }
        return Ok(vec![UncategorizedRow {
            transaction_id: record.transaction_id,
            transaction_date: record.transaction_date,
            description: record.description,
            amount: record.amount,
            account_id: record.account_id,
            account_label: None,
            tx_type: record.tx_type,
            category_source: record.category_source,
            payment_status: record.payment_status,
            source: record.source,
            metadata_json: record.metadata_json,
        }]);
    }

    // Pull a bit more than `limit` so we can drop already-attempted
    // ones before honouring the cap. 4x is enough — a real run rarely
    // overlaps that much with prior runs.
    let raw_limit = args.limit.saturating_mul(4).max(args.limit);
    let mut rows = store.uncategorized(raw_limit).await?;

    let cutoff = (Utc::now() - ChronoDuration::days(args.days)).date_naive();
    rows.retain(|r| r.transaction_date >= cutoff);

    if !args.retry {
        // We don't have enrichment_attempted_at on UncategorizedRow, so
        // probe each candidate via transaction_by_id. This keeps the
        // SQL surface unchanged and is fine at limit ~20.
        let mut kept = Vec::with_capacity(rows.len());
        for row in rows {
            let attempted = match store.transaction_by_id(&row.transaction_id).await? {
                Some(rec) => rec.enrichment_attempted_at.is_some(),
                None => false,
            };
            if !attempted {
                kept.push(row);
            }
            if kept.len() >= args.limit {
                break;
            }
        }
        Ok(kept)
    } else {
        rows.truncate(args.limit);
        Ok(rows)
    }
}

// ── Human mode ────────────────────────────────────────────────────────

async fn run_human(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    pipeline: &EnrichmentPipeline,
    candidates: &[UncategorizedRow],
    counters: &mut RunCounters,
) -> Result<()> {
    let total = candidates.len();
    for (idx, tx) in candidates.iter().enumerate() {
        counters.processed += 1;
        println!();
        let decision = match pipeline.run_one(tx, store).await {
            Ok(d) => d,
            Err(err) => {
                eprintln!(
                    "[{}/{}] {} | erro: {err:#}",
                    idx + 1,
                    total,
                    tx.description
                );
                continue;
            }
        };

        match decision {
            EnrichmentDecision::AutoApply { result } => {
                print_suggestion(idx + 1, total, tx, &result, "AutoApply");
                if args.auto {
                    apply_decision(args, config, store, tx, &result, true).await?;
                    counters.auto_applied += 1;
                } else {
                    let _ = confirm_prompt(args, config, store, tx, &result, counters).await?;
                }
            }
            EnrichmentDecision::Suggest { result } => {
                print_suggestion(idx + 1, total, tx, &result, "Suggest");
                let _ = confirm_prompt(args, config, store, tx, &result, counters).await?;
            }
            EnrichmentDecision::AskUser { result } => {
                print_low_confidence(idx + 1, total, tx, &result);
                let _ = free_text_prompt(args, config, store, tx, &result, counters).await?;
            }
        }
    }
    Ok(())
}

fn print_suggestion(
    idx: usize,
    total: usize,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
    label: &str,
) {
    println!(
        "[{}/{}] {} | R$ {} | {} ({})",
        idx,
        total,
        tx.description.trim(),
        tx.amount,
        tx.transaction_date,
        label,
    );
    if let Some(cat) = tx
        .metadata_json
        .pointer("/pluggy_category")
        .and_then(|v| v.as_str())
    {
        println!("  Pluggy: {cat}");
    }
    println!(
        "  → Sugestão: {}:{} (confiança {:.0}%)",
        result.category,
        result.subcategory,
        (result.confidence * 100.0).round()
    );
    if !result.reasoning.trim().is_empty() {
        println!("    Razão: {}", result.reasoning.trim());
    }
}

fn print_low_confidence(idx: usize, total: usize, tx: &UncategorizedRow, result: &EnrichmentResult) {
    println!(
        "[{}/{}] {} | R$ {} | {}",
        idx,
        total,
        tx.description.trim(),
        tx.amount,
        tx.transaction_date,
    );
    println!(
        "  Confiança baixa ({:.0}%).",
        (result.confidence * 100.0).round()
    );
    if let Some(prompt) = result.user_prompt.as_deref() {
        if !prompt.trim().is_empty() {
            println!("  → \"{}\"", prompt.trim());
        }
    } else if !result.reasoning.trim().is_empty() {
        println!("  → {}", result.reasoning.trim());
    }
}

async fn confirm_prompt(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
    counters: &mut RunCounters,
) -> Result<bool> {
    print!("  Aplicar? [Y/n/s/c] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_lowercase();
    match answer.as_str() {
        "" | "y" | "yes" => {
            apply_decision(args, config, store, tx, result, false).await?;
            counters.confirmed += 1;
            Ok(true)
        }
        "n" | "no" => {
            counters.skipped += 1;
            Ok(false)
        }
        "s" | "skip" => {
            // skip + mark attempted (so we won't re-try)
            if !args.dry_run {
                mark_attempted(store, &tx.transaction_id, &config.actor_id).await?;
            }
            counters.reviewed += 1;
            Ok(false)
        }
        "c" | "custom" => {
            print!("  Categoria (formato cat:subcat): ");
            std::io::stdout().flush().ok();
            let mut custom = String::new();
            std::io::stdin().read_line(&mut custom)?;
            let custom = custom.trim().to_string();
            apply_custom_category(args, config, store, tx, result, &custom, counters).await
        }
        _ => {
            counters.skipped += 1;
            Ok(false)
        }
    }
}

async fn free_text_prompt(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
    counters: &mut RunCounters,
) -> Result<bool> {
    print!("  Sua resposta (categoria livre ou Enter p/ pular): ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_string();
    if answer.is_empty() {
        if !args.dry_run {
            mark_attempted(store, &tx.transaction_id, &config.actor_id).await?;
        }
        counters.reviewed += 1;
        println!("  ignored, mark for manual review");
        return Ok(false);
    }
    apply_custom_category(args, config, store, tx, result, &answer, counters).await
}

async fn apply_custom_category(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
    custom: &str,
    counters: &mut RunCounters,
) -> Result<bool> {
    if let Some((cat, sub)) = parse_category(custom) {
        let mut r = result.clone();
        r.category = cat;
        r.subcategory = sub;
        apply_decision(args, config, store, tx, &r, false).await?;
        counters.confirmed += 1;
        Ok(true)
    } else {
        println!("  formato inválido (esperado cat:subcat); ignored, mark for manual review");
        if !args.dry_run {
            mark_attempted(store, &tx.transaction_id, &config.actor_id).await?;
        }
        counters.reviewed += 1;
        Ok(false)
    }
}

fn parse_category(input: &str) -> Option<(String, String)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.splitn(2, ':');
    let cat = parts.next()?.trim();
    let sub = parts.next()?.trim();
    if cat.is_empty() || sub.is_empty() {
        return None;
    }
    Some((cat.to_string(), sub.to_string()))
}

/// Persist a decision: annotate_transaction + mark_enrichment_attempted
/// + audit event.
///
/// `is_auto` selects between `enriched:llm` and `enriched:user` as
/// `category_source`.
async fn apply_decision(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
    is_auto: bool,
) -> Result<()> {
    if args.dry_run {
        return Ok(());
    }
    let category_id = format!("{}:{}", result.category, result.subcategory);
    let source = if is_auto { "enriched:llm" } else { "enriched:user" };
    let idempotency_key = format!(
        "enrich:{}:{}",
        tx.transaction_id,
        uuid::Uuid::now_v7()
    );
    store
        .annotate_transaction(
            &tx.transaction_id,
            Some(&category_id),
            Some(source),
            Some(&result.reasoning),
            &config.actor_id,
            &idempotency_key,
        )
        .await?;
    mark_attempted(store, &tx.transaction_id, &config.actor_id).await?;
    let audit = AuditEvent::from_entity(
        "transaction",
        &tx.transaction_id,
        if is_auto { "enrich_auto" } else { "enrich_confirmed" },
        &config.actor_id,
        &idempotency_key,
        serde_json::json!({
            "category_id": category_id,
            "category_source": source,
            "confidence": result.confidence,
            "merchant_name": result.merchant_name,
        }),
    );
    store.insert_audit_events(&[audit]).await?;
    Ok(())
}

// ── Machine mode ──────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MachineOutput<'a> {
    #[serde(rename = "decision")]
    Decision {
        transaction_id: &'a str,
        description: &'a str,
        amount: String,
        date: String,
        pluggy_category: Option<&'a str>,
        cnpj_info: Option<&'a CnpjInfo>,
        suggestion: MachineSuggestion<'a>,
        user_prompt: Option<&'a str>,
        decision_type: &'a str,
    },
    #[serde(rename = "timeout")]
    Timeout { transaction_id: &'a str },
    #[serde(rename = "done")]
    Done {
        processed: usize,
        auto_applied: usize,
        confirmed: usize,
        skipped: usize,
        reviewed: usize,
    },
}

#[derive(Debug, Serialize)]
struct MachineSuggestion<'a> {
    merchant_name: &'a str,
    category: &'a str,
    subcategory: &'a str,
    confidence: f32,
    reasoning: &'a str,
    needs_user_input: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "action", rename_all = "snake_case")]
enum MachineAction {
    Confirm,
    Skip,
    MarkReviewed,
    Custom {
        category: String,
        subcategory: String,
    },
}

#[derive(Debug, Deserialize)]
struct MachineRequest {
    transaction_id: String,
    #[serde(flatten)]
    action: MachineAction,
}

async fn run_machine(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    pipeline: &EnrichmentPipeline,
    candidates: &[UncategorizedRow],
    counters: &mut RunCounters,
) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);

    for tx in candidates {
        counters.processed += 1;
        let decision = match pipeline.run_one(tx, store).await {
            Ok(d) => d,
            Err(err) => {
                // Emit a "timeout"-shaped error line for the agent.
                eprintln!(
                    "warn: enrichment failed for {}: {err:#}",
                    tx.transaction_id
                );
                counters.skipped += 1;
                continue;
            }
        };

        let (result, decision_type) = match &decision {
            EnrichmentDecision::AutoApply { result } => (result, "AutoApply"),
            EnrichmentDecision::Suggest { result } => (result, "Suggest"),
            EnrichmentDecision::AskUser { result } => (result, "AskUser"),
        };

        let pluggy_category = tx
            .metadata_json
            .pointer("/pluggy_category")
            .and_then(|v| v.as_str());

        // Tee out the decision NDJSON.
        let mut pipeline_cache: Option<CnpjInfo> = None;
        let cnpj_info = lookup_cached_cnpj(tx, &mut pipeline_cache);
        let payload = MachineOutput::Decision {
            transaction_id: &tx.transaction_id,
            description: &tx.description,
            amount: format!("{}", tx.amount),
            date: tx.transaction_date.to_string(),
            pluggy_category,
            cnpj_info,
            suggestion: MachineSuggestion {
                merchant_name: &result.merchant_name,
                category: &result.category,
                subcategory: &result.subcategory,
                confidence: result.confidence,
                reasoning: &result.reasoning,
                needs_user_input: result.needs_user_input,
            },
            user_prompt: result.user_prompt.as_deref(),
            decision_type,
        };
        emit_ndjson_line(&payload)?;

        // Block on stdin for one action line.
        match read_machine_request(&mut reader, args.machine_timeout).await? {
            MachineRead::Eof => {
                // Agent disconnected. Emit done and stop.
                return Ok(());
            }
            MachineRead::Timeout => {
                emit_ndjson_line(&MachineOutput::Timeout {
                    transaction_id: &tx.transaction_id,
                })?;
                return Err(anyhow!(
                    "Timeout aguardando resposta para {}",
                    tx.transaction_id
                ));
            }
            MachineRead::Parsed(req) => {
                handle_machine_action(args, config, store, tx, result, &req, counters).await?;
            }
        }
    }
    Ok(())
}

// CNPJ info caching is handled inside the pipeline's moka cache. The
// machine output reuses whatever the pipeline already fetched by
// re-running extract_cnpj on the metadata; in Phase 3 we keep the
// payload lean and skip re-querying BrasilAPI just for the NDJSON.
// We expose `None` for now — the agent can re-derive it from
// `metadata_json` if needed. Phase 4 may attach the cached entry.
fn lookup_cached_cnpj<'a>(
    _tx: &UncategorizedRow,
    _cache: &'a mut Option<CnpjInfo>,
) -> Option<&'a CnpjInfo> {
    None
}

enum MachineRead {
    Eof,
    Timeout,
    Parsed(MachineRequest),
}

async fn read_machine_request(
    reader: &mut BufReader<tokio::io::Stdin>,
    timeout_secs: u64,
) -> Result<MachineRead> {
    let mut line = String::new();
    let deadline = TokioDuration::from_secs(timeout_secs);
    let res = timeout(deadline, reader.read_line(&mut line)).await;
    match res {
        Err(_) => Ok(MachineRead::Timeout),
        Ok(Err(err)) => Err(err).context("stdin read failed"),
        Ok(Ok(0)) => Ok(MachineRead::Eof),
        Ok(Ok(_)) => {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                // Treat empty lines as skip-equivalents.
                return Ok(MachineRead::Eof);
            }
            let req: MachineRequest = serde_json::from_str(trimmed)
                .with_context(|| format!("JSON inválido na entrada: {trimmed}"))?;
            Ok(MachineRead::Parsed(req))
        }
    }
}

async fn handle_machine_action(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
    req: &MachineRequest,
    counters: &mut RunCounters,
) -> Result<()> {
    if req.transaction_id != tx.transaction_id {
        eprintln!(
            "warn: stdin transaction_id mismatch (got {}, expected {}). Treating as skip.",
            req.transaction_id, tx.transaction_id
        );
        counters.skipped += 1;
        return Ok(());
    }
    match &req.action {
        MachineAction::Confirm => {
            apply_decision(args, config, store, tx, result, false).await?;
            counters.confirmed += 1;
        }
        MachineAction::Skip => {
            counters.skipped += 1;
        }
        MachineAction::MarkReviewed => {
            if !args.dry_run {
                mark_attempted(store, &tx.transaction_id, &config.actor_id).await?;
            }
            counters.reviewed += 1;
        }
        MachineAction::Custom { category, subcategory } => {
            let mut r = result.clone();
            r.category = category.clone();
            r.subcategory = subcategory.clone();
            apply_decision(args, config, store, tx, &r, false).await?;
            counters.confirmed += 1;
        }
    }
    Ok(())
}

/// Emit a single NDJSON line to stdout and flush. Returns an error if
/// stdout is broken (caller's responsibility to bail).
fn emit_ndjson_line<T: Serialize>(value: &T) -> Result<()> {
    let line = serde_json::to_string(value)?;
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{line}")?;
    handle.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_machine_decision_serialization() {
        let cnpj = CnpjInfo {
            cnpj: "12345678000190".into(),
            razao_social: "SAPIENS LTDA".into(),
            nome_fantasia: None,
            cnae_fiscal: 5611201,
            cnae_descricao: "Restaurantes".into(),
            cnaes_secundarios: vec![],
        };
        let payload = MachineOutput::Decision {
            transaction_id: "tx-1",
            description: "Sapiens",
            amount: "-25.90".into(),
            date: "2026-05-04".into(),
            pluggy_category: Some("Groceries"),
            cnpj_info: Some(&cnpj),
            suggestion: MachineSuggestion {
                merchant_name: "Sapiens",
                category: "alimentacao",
                subcategory: "restaurantes",
                confidence: 0.91,
                reasoning: "porque sim",
                needs_user_input: false,
            },
            user_prompt: None,
            decision_type: "AutoApply",
        };
        let s = serde_json::to_string(&payload).unwrap();
        // Sanity checks: tagged variant, key fields present.
        assert!(s.contains("\"type\":\"decision\""));
        assert!(s.contains("\"transaction_id\":\"tx-1\""));
        assert!(s.contains("\"decision_type\":\"AutoApply\""));
        assert!(s.contains("\"category\":\"alimentacao\""));
        assert!(s.contains("\"subcategory\":\"restaurantes\""));
        assert!(s.contains("\"cnae_fiscal\":5611201"));
    }

    #[test]
    fn test_machine_stdin_action_parsing_confirm() {
        let raw = r#"{"transaction_id":"abc","action":"confirm"}"#;
        let req: MachineRequest = serde_json::from_str(raw).unwrap();
        assert_eq!(req.transaction_id, "abc");
        assert!(matches!(req.action, MachineAction::Confirm));
    }

    #[test]
    fn test_machine_stdin_action_parsing_skip() {
        let raw = r#"{"transaction_id":"abc","action":"skip"}"#;
        let req: MachineRequest = serde_json::from_str(raw).unwrap();
        assert!(matches!(req.action, MachineAction::Skip));
    }

    #[test]
    fn test_machine_stdin_action_parsing_mark_reviewed() {
        let raw = r#"{"transaction_id":"abc","action":"mark_reviewed"}"#;
        let req: MachineRequest = serde_json::from_str(raw).unwrap();
        assert!(matches!(req.action, MachineAction::MarkReviewed));
    }

    #[test]
    fn test_machine_stdin_action_parsing_custom() {
        let raw = r#"{"transaction_id":"abc","action":"custom","category":"alimentacao","subcategory":"restaurantes"}"#;
        let req: MachineRequest = serde_json::from_str(raw).unwrap();
        match req.action {
            MachineAction::Custom { category, subcategory } => {
                assert_eq!(category, "alimentacao");
                assert_eq!(subcategory, "restaurantes");
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn test_summary_line_format() {
        let payload = MachineOutput::Done {
            processed: 5,
            auto_applied: 2,
            confirmed: 1,
            skipped: 1,
            reviewed: 1,
        };
        let s = serde_json::to_string(&payload).unwrap();
        assert!(s.contains("\"type\":\"done\""));
        assert!(s.contains("\"processed\":5"));
        assert!(s.contains("\"auto_applied\":2"));
        assert!(s.contains("\"confirmed\":1"));
        assert!(s.contains("\"skipped\":1"));
        assert!(s.contains("\"reviewed\":1"));
    }

    #[test]
    fn test_parse_category_valid() {
        let (c, s) = parse_category("alimentacao:restaurantes").unwrap();
        assert_eq!(c, "alimentacao");
        assert_eq!(s, "restaurantes");
    }

    #[test]
    fn test_parse_category_invalid() {
        assert!(parse_category("").is_none());
        assert!(parse_category("alimentacao").is_none());
        assert!(parse_category(":sub").is_none());
        assert!(parse_category("cat:").is_none());
    }

    #[test]
    fn test_machine_timeout_payload_serializes() {
        let payload = MachineOutput::Timeout {
            transaction_id: "tx-9",
        };
        let s = serde_json::to_string(&payload).unwrap();
        assert!(s.contains("\"type\":\"timeout\""));
        assert!(s.contains("\"transaction_id\":\"tx-9\""));
    }

    // ensure metadata_json round-trips don't break our pointer reads
    #[test]
    fn test_metadata_pluggy_category_pointer() {
        let meta = json!({"pluggy_category": "Eating out"});
        assert_eq!(
            meta.pointer("/pluggy_category").and_then(|v| v.as_str()),
            Some("Eating out")
        );
    }
}
