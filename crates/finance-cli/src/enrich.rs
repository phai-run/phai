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
use finance_core::enrichment::fuzzy::score_to_percent;
use finance_core::enrichment::llm::LlmProvider;
use finance_core::enrichment::pipeline::{mark_attempted, EnrichmentPipeline};
use finance_core::enrichment::replication::{compute_replication, ReplicationOutcome};
use finance_core::enrichment::rule_gen::{build_rule_record, keyword_from_result};
use finance_core::enrichment::types::{CnpjInfo, EnrichmentDecision, EnrichmentResult};
use finance_core::models::{AuditEvent, RuleRecord, TransactionRecord, UncategorizedRow};
use finance_core::storage::{FinanceStore, TransactionAnatomyPatch};
use serde::{Deserialize, Serialize};
use std::io::Write as _;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{timeout, Duration as TokioDuration};

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_DAYS: i64 = 30;
const DEFAULT_MACHINE_TIMEOUT: u64 = 60;
const DEFAULT_RETROACTIVE_THRESHOLD: u8 = 80;

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

    /// Skip rule creation and retroactive matching after a successful
    /// categorization.
    #[arg(long)]
    pub no_rule: bool,

    /// Percentage (0..=100) used as the fuzzy threshold when scanning
    /// past transactions for retroactive application.
    #[arg(long, default_value_t = DEFAULT_RETROACTIVE_THRESHOLD)]
    pub retroactive_threshold: u8,
}

#[derive(Debug, Default)]
struct RunCounters {
    processed: usize,
    auto_applied: usize,
    confirmed: usize,
    skipped: usize,
    reviewed: usize,
    rules_created: usize,
    retroactive_applied: usize,
}

/// Entry point invoked from `TxCommand::Enrich`. Walks the candidate
/// list, runs the pipeline, and dispatches to either the human or the
/// machine flow.
pub async fn run(args: EnrichArgs, config: &AppConfig, store: &dyn FinanceStore) -> Result<()> {
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
        println!("Rules created: {}", counters.rules_created);
        println!("Retroactive applied: {}", counters.retroactive_applied);
    }

    Ok(())
}

// ── Post-sync hook (Phase 5) ─────────────────────────────────────────

/// Outcome of [`enrich_after_sync`]. All counters are non-negative and
/// sum to `processed` (modulo decisions that failed both apply and
/// mark-attempted in the same iteration — those are still counted as
/// `failed` only).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EnrichSummary {
    /// Number of new transactions the hook tried to enrich.
    pub processed: usize,
    /// AutoApply decisions that were persisted.
    pub auto_applied: usize,
    /// Suggest / AskUser decisions — left for a later manual
    /// `finance tx enrich` run. The transaction is marked as
    /// `enrichment_attempted_at = now` so it won't be retried by default,
    /// but its category is unchanged.
    pub deferred: usize,
    /// Errors (LLM down, BrasilAPI down, store failure). The hook keeps
    /// going and never propagates these.
    pub failed: usize,
}

impl EnrichSummary {
    /// Render the canonical PT-BR summary line shown after sync. Splitting
    /// into a helper keeps the format stable and easy to unit-test
    /// without touching stdout.
    pub fn format_summary(&self) -> String {
        format!(
            "Enrichment automático: {} categorizadas | {} adiadas para revisão | {} falhas",
            self.auto_applied, self.deferred, self.failed
        )
    }
}

/// Hook invoked by `finance sync` after a successful Pluggy sync. Walks
/// the newly upserted transaction IDs and runs the enrichment pipeline
/// on each one. Designed to be **non-fatal**: any error inside the
/// pipeline (LLM unavailable, BrasilAPI throttled, transient store
/// failure) is logged via `eprintln!` and counted, but never returned.
///
/// `auto_only = true` is the right setting for unattended runs (no TTY,
/// CI, batch jobs): only `EnrichmentDecision::AutoApply` is persisted;
/// Suggest / AskUser decisions are merely `mark_enrichment_attempted` so
/// the next interactive `finance tx enrich` picks them up.
///
/// `auto_only = false` behaves identically today — interactive
/// confirmation during sync is out of scope; we always defer
/// medium/low-confidence decisions.
pub async fn enrich_after_sync(
    config: &AppConfig,
    store: &dyn FinanceStore,
    new_tx_ids: &[String],
    auto_only: bool,
) -> EnrichSummary {
    let _ = auto_only; // currently informational; we always defer non-AutoApply.
    let mut summary = EnrichSummary::default();
    if new_tx_ids.is_empty() {
        return summary;
    }

    let pipeline = match EnrichmentPipeline::new(config) {
        Ok(p) => p,
        Err(err) => {
            eprintln!(
                "aviso: enrichment indisponível ({err:#}); rode `finance tx enrich` manualmente."
            );
            summary.failed = new_tx_ids.len();
            return summary;
        }
    };

    for id in new_tx_ids {
        summary.processed += 1;
        if let Err(err) = enrich_after_sync_one(config, store, &pipeline, id, &mut summary).await {
            eprintln!("aviso: enrichment falhou para {id}: {err:#}");
            summary.failed += 1;
        }
    }
    summary
}

/// Helper that handles a single transaction inside [`enrich_after_sync`].
/// Returning `Err` here just means the caller increments `failed` — the
/// outer loop continues.
async fn enrich_after_sync_one(
    config: &AppConfig,
    store: &dyn FinanceStore,
    pipeline: &EnrichmentPipeline,
    tx_id: &str,
    summary: &mut EnrichSummary,
) -> Result<()> {
    let record = match store.transaction_by_id(tx_id).await? {
        Some(r) => r,
        None => return Ok(()), // disappeared between sync and enrich — nothing to do
    };
    // Skip transactions that already have a category from a strong source.
    // The sync inserted them with whatever Pluggy reported; if the rule
    // engine matched something, we don't want to overwrite. Sources we
    // consider "weak" enough to enrich over: unclassified / fallback /
    // pluggy / empty.
    {
        let source = record.category_source.as_str();
        let weak = matches!(source, "unclassified" | "fallback" | "pluggy" | "");
        if !weak && record.category_id.is_some() {
            return Ok(());
        }
    }

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

    let decision = pipeline.run_one(&row, store).await?;
    match decision {
        EnrichmentDecision::AutoApply { result } => {
            apply_auto_decision(config, store, &row, &result).await?;
            summary.auto_applied += 1;
            // Best-effort rule generation; failures here don't roll back
            // the annotation. Phase 5 intentionally keeps the hook quiet
            // about rule creation status (no stdout chatter during sync).
            if let Err(err) = try_generate_rule(config, store, &result).await {
                eprintln!(
                    "aviso: falha ao gerar regra para {}: {err:#}",
                    row.transaction_id
                );
            }
        }
        EnrichmentDecision::Suggest { .. } | EnrichmentDecision::AskUser { .. } => {
            mark_attempted(store, &row.transaction_id, &config.actor_id).await?;
            summary.deferred += 1;
        }
    }
    Ok(())
}

/// Best-effort: copy description/purpose from a prior same-merchant
/// transaction if the current one still has those fields empty.
///
/// Called immediately after the enrichment pipeline writes `merchant_name`
/// so the merchant token is known. Errors are logged to stderr and do NOT
/// propagate — replication is opportunistic and must not abort the enrichment
/// flow.
async fn try_replicate_anatomy(
    store: &dyn FinanceStore,
    transaction_id: &str,
    merchant_name: &str,
    category_id: &str,
    amount: rust_decimal::Decimal,
    actor_id: &str,
) {
    let donors = match store
        .find_anatomy_donors(merchant_name, transaction_id)
        .await
    {
        Ok(d) => d,
        Err(err) => {
            eprintln!("aviso: find_anatomy_donors falhou para {transaction_id}: {err:#}");
            return;
        }
    };
    // New Pluggy transactions always have NULL description and purpose.
    let outcome = compute_replication(
        Some(merchant_name),
        None,
        None,
        Some(category_id),
        amount,
        &donors,
    );
    let rep = match outcome {
        ReplicationOutcome::Replicated(r) => r,
        _ => return,
    };
    let idempotency_key = format!("anatomy_rep:{}:{}", transaction_id, uuid::Uuid::now_v7());
    let patch = finance_core::storage::TransactionAnatomyPatch {
        description: rep.description.as_deref(),
        purpose: rep.purpose.as_deref(),
        ..finance_core::storage::TransactionAnatomyPatch::default()
    };
    if let Err(err) = store
        .update_transaction_anatomy(transaction_id, patch, actor_id, &idempotency_key)
        .await
    {
        eprintln!("aviso: replicação de anatomy falhou para {transaction_id}: {err:#}");
        return;
    }
    let audit = AuditEvent::from_entity(
        "transaction",
        transaction_id,
        "anatomy_replicated",
        actor_id,
        &idempotency_key,
        serde_json::json!({
            "donor_id": rep.donor_id,
            "description_replicated": rep.description.is_some(),
            "purpose_replicated": rep.purpose.is_some(),
        }),
    );
    if let Err(err) = store.insert_audit_events(&[audit]).await {
        eprintln!("aviso: audit event de replicação falhou para {transaction_id}: {err:#}");
    }
}

/// Annotation + audit + mark-attempted for a high-confidence
/// AutoApply during the sync hook. Mirrors [`apply_decision`] but is
/// always non-dry-run and always uses `enriched:llm` as the source.
async fn apply_auto_decision(
    config: &AppConfig,
    store: &dyn FinanceStore,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
) -> Result<()> {
    let category_id = format!("{}:{}", result.category, result.subcategory);
    let idempotency_key = format!("enrich_sync:{}:{}", tx.transaction_id, uuid::Uuid::now_v7());
    store
        .annotate_transaction(
            &tx.transaction_id,
            Some(&category_id),
            Some("enriched:llm"),
            Some(&result.reasoning),
            &config.actor_id,
            &idempotency_key,
        )
        .await
        .context("annotate_transaction falhou no enrich_after_sync")?;
    store
        .update_transaction_anatomy(
            &tx.transaction_id,
            TransactionAnatomyPatch {
                merchant_name: Some(&result.merchant_name),
                classifier_trace: Some(&result.reasoning),
                ..TransactionAnatomyPatch::default()
            },
            &config.actor_id,
            &idempotency_key,
        )
        .await
        .context("update_transaction_anatomy falhou no enrich_after_sync")?;
    try_replicate_anatomy(
        store,
        &tx.transaction_id,
        &result.merchant_name,
        &category_id,
        tx.amount,
        &config.actor_id,
    )
    .await;
    mark_attempted(store, &tx.transaction_id, &config.actor_id).await?;
    let audit = AuditEvent::from_entity(
        "transaction",
        &tx.transaction_id,
        "enrich_auto_sync",
        &config.actor_id,
        &idempotency_key,
        serde_json::json!({
            "category_id": category_id,
            "category_source": "enriched:llm",
            "confidence": result.confidence,
            "merchant_name": result.merchant_name,
            "trigger": "post_sync",
        }),
    );
    store.insert_audit_events(&[audit]).await?;
    Ok(())
}

/// Idempotent rule creation invoked from the sync hook. Skips if a rule
/// with the same body already exists. Returns `Ok(())` even when no
/// rule is created (e.g. invalid keyword).
async fn try_generate_rule(
    config: &AppConfig,
    store: &dyn FinanceStore,
    result: &EnrichmentResult,
) -> Result<()> {
    let rule = match build_rule_record(result, &config.actor_id) {
        Ok(r) => r,
        Err(_) => return Ok(()), // keyword too short / stopword
    };
    let existing = store.active_rules().await.context("active_rules falhou")?;
    if existing.iter().any(|r| r.body == rule.body) {
        return Ok(());
    }
    store
        .upsert_rules(std::slice::from_ref(&rule))
        .await
        .context("upsert_rules falhou")?;
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
            bail!("Transação {id} já foi processada (use --retry para re-processar)");
        }
        return Ok(vec![UncategorizedRow {
            transaction_id: record.transaction_id,
            transaction_date: record.transaction_date,
            description: record.raw_description,
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
                eprintln!("[{}/{}] {} | erro: {err:#}", idx + 1, total, tx.description);
                continue;
            }
        };

        match decision {
            EnrichmentDecision::AutoApply { result } => {
                print_suggestion(idx + 1, total, tx, &result, "AutoApply");
                if args.auto {
                    apply_decision(args, config, store, tx, &result, true).await?;
                    counters.auto_applied += 1;
                    post_apply_rule_and_retroactive_human(
                        args, config, store, pipeline, tx, &result, counters,
                    )
                    .await?;
                } else {
                    let _ = confirm_prompt(args, config, store, pipeline, tx, &result, counters)
                        .await?;
                }
            }
            EnrichmentDecision::Suggest { result } => {
                print_suggestion(idx + 1, total, tx, &result, "Suggest");
                let _ =
                    confirm_prompt(args, config, store, pipeline, tx, &result, counters).await?;
            }
            EnrichmentDecision::AskUser { result } => {
                print_low_confidence(idx + 1, total, tx, &result);
                let _ =
                    free_text_prompt(args, config, store, pipeline, tx, &result, counters).await?;
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

fn print_low_confidence(
    idx: usize,
    total: usize,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
) {
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
    pipeline: &EnrichmentPipeline,
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
            post_apply_rule_and_retroactive_human(
                args, config, store, pipeline, tx, result, counters,
            )
            .await?;
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
            apply_custom_category(args, config, store, pipeline, tx, result, &custom, counters)
                .await
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
    pipeline: &EnrichmentPipeline,
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
    apply_custom_category(args, config, store, pipeline, tx, result, &answer, counters).await
}

#[allow(clippy::too_many_arguments)]
async fn apply_custom_category(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    pipeline: &EnrichmentPipeline,
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
        post_apply_rule_and_retroactive_human(args, config, store, pipeline, tx, &r, counters)
            .await?;
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
    let source = if is_auto {
        "enriched:llm"
    } else {
        "enriched:user"
    };
    let idempotency_key = format!("enrich:{}:{}", tx.transaction_id, uuid::Uuid::now_v7());
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
    store
        .update_transaction_anatomy(
            &tx.transaction_id,
            TransactionAnatomyPatch {
                merchant_name: Some(&result.merchant_name),
                classifier_trace: Some(&result.reasoning),
                ..TransactionAnatomyPatch::default()
            },
            &config.actor_id,
            &idempotency_key,
        )
        .await?;
    try_replicate_anatomy(
        store,
        &tx.transaction_id,
        &result.merchant_name,
        &category_id,
        tx.amount,
        &config.actor_id,
    )
    .await;
    mark_attempted(store, &tx.transaction_id, &config.actor_id).await?;
    let audit = AuditEvent::from_entity(
        "transaction",
        &tx.transaction_id,
        if is_auto {
            "enrich_auto"
        } else {
            "enrich_confirmed"
        },
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
    #[serde(rename = "retroactive")]
    Retroactive {
        transaction_id: &'a str,
        keyword: &'a str,
        category: &'a str,
        subcategory: &'a str,
        matches: Vec<RetroactiveMatch<'a>>,
    },
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
struct RetroactiveMatch<'a> {
    transaction_id: &'a str,
    description: &'a str,
    amount: String,
    date: String,
    score: u32,
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
                eprintln!("warn: enrichment failed for {}: {err:#}", tx.transaction_id);
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
                handle_machine_action(
                    args,
                    config,
                    store,
                    pipeline,
                    &mut reader,
                    tx,
                    result,
                    &req,
                    counters,
                )
                .await?;
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

#[allow(clippy::too_many_arguments)]
async fn handle_machine_action(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    pipeline: &EnrichmentPipeline,
    reader: &mut BufReader<tokio::io::Stdin>,
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
            post_apply_rule_and_retroactive_machine(
                args, config, store, pipeline, reader, tx, result, counters,
            )
            .await?;
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
        MachineAction::Custom {
            category,
            subcategory,
        } => {
            let mut r = result.clone();
            r.category = category.clone();
            r.subcategory = subcategory.clone();
            apply_decision(args, config, store, tx, &r, false).await?;
            counters.confirmed += 1;
            post_apply_rule_and_retroactive_machine(
                args, config, store, pipeline, reader, tx, &r, counters,
            )
            .await?;
        }
    }
    Ok(())
}

// ── Rule creation + retroactive (Phase 4) ────────────────────────────

/// Try to build a `RuleRecord` from the enrichment result and upsert it
/// if no existing active rule already carries the same body. Returns
/// `Some(keyword)` when callers should proceed with the retroactive
/// search, or `None` if rule creation was skipped (duplicate, disabled,
/// or invalid keyword).
async fn ensure_rule(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    _tx: &UncategorizedRow,
    result: &EnrichmentResult,
    counters: &mut RunCounters,
) -> Result<Option<String>> {
    if args.no_rule {
        return Ok(None);
    }
    let rule: RuleRecord = match build_rule_record(result, &config.actor_id) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("  (regra não gerada: {err})");
            return Ok(None);
        }
    };
    let keyword = keyword_from_result(result)?;
    if !args.dry_run {
        let existing = store
            .active_rules()
            .await
            .context("falha ao carregar regras ativas")?;
        if existing.iter().any(|r| r.body == rule.body) {
            println!("  Regra equivalente já existe — pulando criação.");
            return Ok(Some(keyword));
        }
        store
            .upsert_rules(std::slice::from_ref(&rule))
            .await
            .context("falha ao gravar regra gerada")?;
        counters.rules_created += 1;
        println!("  Regra criada: {}", rule.body);
    } else {
        println!("  (dry-run) regra que seria criada: {}", rule.body);
    }
    Ok(Some(keyword))
}

/// Apply `category_id` to every retroactive `match` using
/// `category_source = "enriched:retroactive"`. Emits one audit event per
/// transaction.
async fn apply_retroactive_batch(
    config: &AppConfig,
    store: &dyn FinanceStore,
    matches: &[(TransactionRecord, u32)],
    result: &EnrichmentResult,
    counters: &mut RunCounters,
) -> Result<usize> {
    let category_id = format!("{}:{}", result.category, result.subcategory);
    let mut applied = 0usize;
    for (rec, score) in matches {
        let idempotency_key = format!(
            "enrich_retroactive:{}:{}",
            rec.transaction_id,
            uuid::Uuid::now_v7()
        );
        store
            .annotate_transaction(
                &rec.transaction_id,
                Some(&category_id),
                Some("enriched:retroactive"),
                Some(&result.reasoning),
                &config.actor_id,
                &idempotency_key,
            )
            .await
            .with_context(|| format!("annotate_transaction falhou para {}", rec.transaction_id))?;
        store
            .update_transaction_anatomy(
                &rec.transaction_id,
                TransactionAnatomyPatch {
                    merchant_name: Some(&result.merchant_name),
                    classifier_trace: Some(&result.reasoning),
                    ..TransactionAnatomyPatch::default()
                },
                &config.actor_id,
                &idempotency_key,
            )
            .await
            .with_context(|| {
                format!(
                    "update_transaction_anatomy falhou para {}",
                    rec.transaction_id
                )
            })?;
        store
            .mark_enrichment_attempted(&rec.transaction_id, &config.actor_id, &idempotency_key)
            .await?;
        let audit = AuditEvent::from_entity(
            "transaction",
            &rec.transaction_id,
            "enrich_retroactive",
            &config.actor_id,
            &idempotency_key,
            serde_json::json!({
                "category_id": category_id,
                "category_source": "enriched:retroactive",
                "score": score,
                "merchant_name": result.merchant_name,
            }),
        );
        store.insert_audit_events(&[audit]).await?;
        applied += 1;
    }
    counters.retroactive_applied += applied;
    Ok(applied)
}

/// Human-mode: ensure rule then prompt the user to confirm retroactive
/// application of any fuzzy-matched past transactions.
async fn post_apply_rule_and_retroactive_human(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    pipeline: &EnrichmentPipeline,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
    counters: &mut RunCounters,
) -> Result<()> {
    let keyword = match ensure_rule(args, config, store, tx, result, counters).await? {
        Some(k) => k,
        None => return Ok(()),
    };
    let threshold = args.retroactive_threshold;
    let matches = pipeline
        .find_retroactive_matches(store, &keyword, &tx.transaction_id, threshold)
        .await?;
    if matches.is_empty() {
        return Ok(());
    }
    println!(
        "  Encontrei {} transações similares. Aplicar \"{}:{}\" a todas? [Y/n]",
        matches.len(),
        result.category,
        result.subcategory
    );
    for (rec, score) in &matches {
        println!(
            "    {} | {} | R$ {} (score: {})",
            rec.transaction_date,
            rec.raw_description.trim(),
            rec.amount,
            score_to_percent(*score)
        );
    }
    print!("  > ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_lowercase();
    let accept = matches!(answer.as_str(), "" | "y" | "yes");
    if !accept {
        println!("  Retroativo ignorado.");
        return Ok(());
    }
    if args.dry_run {
        println!(
            "  (dry-run) seria(m) aplicada(s) {} transações.",
            matches.len()
        );
        return Ok(());
    }
    let applied = apply_retroactive_batch(config, store, &matches, result, counters).await?;
    println!("  {applied} transações retroativas atualizadas.");
    Ok(())
}

/// Machine-mode: emit a `retroactive` NDJSON line with the candidate
/// list and wait for the agent's `confirm`/`skip` decision.
#[allow(clippy::too_many_arguments)]
async fn post_apply_rule_and_retroactive_machine(
    args: &EnrichArgs,
    config: &AppConfig,
    store: &dyn FinanceStore,
    pipeline: &EnrichmentPipeline,
    reader: &mut BufReader<tokio::io::Stdin>,
    tx: &UncategorizedRow,
    result: &EnrichmentResult,
    counters: &mut RunCounters,
) -> Result<()> {
    let keyword = match ensure_rule(args, config, store, tx, result, counters).await? {
        Some(k) => k,
        None => return Ok(()),
    };
    let threshold = args.retroactive_threshold;
    let matches = pipeline
        .find_retroactive_matches(store, &keyword, &tx.transaction_id, threshold)
        .await?;
    if matches.is_empty() {
        return Ok(());
    }
    let payload_matches: Vec<RetroactiveMatch> = matches
        .iter()
        .map(|(rec, score)| RetroactiveMatch {
            transaction_id: &rec.transaction_id,
            description: &rec.raw_description,
            amount: format!("{}", rec.amount),
            date: rec.transaction_date.to_string(),
            score: *score,
        })
        .collect();
    emit_ndjson_line(&MachineOutput::Retroactive {
        transaction_id: &tx.transaction_id,
        keyword: &keyword,
        category: &result.category,
        subcategory: &result.subcategory,
        matches: payload_matches,
    })?;
    match read_machine_request(reader, args.machine_timeout).await? {
        MachineRead::Eof => Ok(()),
        MachineRead::Timeout => {
            emit_ndjson_line(&MachineOutput::Timeout {
                transaction_id: &tx.transaction_id,
            })?;
            Err(anyhow!(
                "Timeout aguardando resposta retroativa para {}",
                tx.transaction_id
            ))
        }
        MachineRead::Parsed(req) => {
            match req.action {
                MachineAction::Confirm => {
                    if !args.dry_run {
                        apply_retroactive_batch(config, store, &matches, result, counters).await?;
                    }
                }
                MachineAction::Skip | MachineAction::MarkReviewed => {}
                MachineAction::Custom {
                    category,
                    subcategory,
                } => {
                    // Agent overrode the category for the batch — apply
                    // with the override.
                    let mut overridden = result.clone();
                    overridden.category = category;
                    overridden.subcategory = subcategory;
                    if !args.dry_run {
                        apply_retroactive_batch(config, store, &matches, &overridden, counters)
                            .await?;
                    }
                }
            }
            Ok(())
        }
    }
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
            MachineAction::Custom {
                category,
                subcategory,
            } => {
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

    #[test]
    fn test_machine_retroactive_serialization() {
        let matches = vec![RetroactiveMatch {
            transaction_id: "old-1",
            description: "Sapiens Parque",
            amount: "-18.50".into(),
            date: "2026-03-12".into(),
            score: 184,
        }];
        let payload = MachineOutput::Retroactive {
            transaction_id: "tx-1",
            keyword: "sapiens",
            category: "alimentacao",
            subcategory: "restaurantes",
            matches,
        };
        let s = serde_json::to_string(&payload).unwrap();
        assert!(s.contains("\"type\":\"retroactive\""));
        assert!(s.contains("\"transaction_id\":\"tx-1\""));
        assert!(s.contains("\"keyword\":\"sapiens\""));
        assert!(s.contains("\"category\":\"alimentacao\""));
        assert!(s.contains("\"subcategory\":\"restaurantes\""));
        assert!(s.contains("\"matches\""));
        assert!(s.contains("\"score\":184"));
        assert!(s.contains("\"description\":\"Sapiens Parque\""));
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

    // ── Phase 5 — post-sync hook tests ───────────────────────────────

    #[test]
    fn test_enrich_summary_format() {
        let s = EnrichSummary {
            processed: 42,
            auto_applied: 18,
            deferred: 22,
            failed: 2,
        };
        assert_eq!(
            s.format_summary(),
            "Enrichment automático: 18 categorizadas | 22 adiadas para revisão | 2 falhas"
        );
    }

    #[test]
    fn test_enrich_summary_format_all_zero() {
        let s = EnrichSummary::default();
        assert_eq!(
            s.format_summary(),
            "Enrichment automático: 0 categorizadas | 0 adiadas para revisão | 0 falhas"
        );
    }

    use crate::enrich::test_support::{clear_llm_env, NoopStore};
    use finance_core::config::AppConfig;

    #[tokio::test]
    async fn test_enrich_after_sync_skips_when_no_ids() {
        // Empty new_tx_ids → all counters zero, store never touched.
        // Env vars don't matter because `EnrichmentPipeline::new` is not
        // even invoked on the empty path.
        let config = AppConfig::default();
        let store = NoopStore::default();
        let summary = enrich_after_sync(&config, &store, &[], false).await;
        assert_eq!(summary, EnrichSummary::default());
        assert_eq!(*store.transaction_by_id_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_enrich_after_sync_non_fatal_on_pipeline_error() {
        // Clear every LLM env var so `LlmProvider::from_env_or_config`
        // fails. `enrich_after_sync` must catch the error, mark every id
        // as `failed`, and still return a summary (never panic, never
        // propagate). Without `#[serial]` this race-conditions against
        // other tests that touch the same env vars.
        clear_llm_env();
        // local_db_path also None — even if LLM env did slip through,
        // `with_provider` would still fail downstream. Belt and braces.
        let config = AppConfig::default();
        let store = NoopStore::default();
        let ids = vec!["tx-a".to_string(), "tx-b".to_string(), "tx-c".to_string()];
        let summary = enrich_after_sync(&config, &store, &ids, true).await;
        assert_eq!(
            summary.processed, 0,
            "pipeline init failure should short-circuit before per-tx loop"
        );
        assert_eq!(summary.auto_applied, 0);
        assert_eq!(summary.deferred, 0);
        assert_eq!(summary.failed, 3);
        // Store must NEVER be touched when pipeline init fails.
        assert_eq!(*store.transaction_by_id_calls.lock().unwrap(), 0);
    }
}

// ── Test support (only compiled under cfg(test)) ─────────────────────

#[cfg(test)]
mod test_support {
    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::NaiveDate;
    use finance_core::enrichment::types::ContextTx;
    use finance_core::models::{
        AccountRecord, AccountSnapshotRecord, AuditEvent, BudgetStatusRow,
        CardClosedTransactionRow, CardSummaryRow, CashflowRow, CategoryBudgetRecord,
        CategoryRecord, CheckingBalance, DailyPulseItem, ForecastRecord, ForecastVsActualRow,
        MonthlySpendRow, RuleRecord, TransactionContextRow, TransactionRecord, UncategorizedRow,
    };
    use finance_core::splits::{
        ItemPriceRow, ReceiptItemRecord, SplitCandidateRow, TransactionSplitDetail,
        TransactionSplitLineRecord, TransactionSplitRecord,
    };
    use finance_core::storage::{FinanceStore, TransactionAnatomyPatch};
    use rust_decimal::Decimal;
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    /// Clear every LLM-related env var. Mirrors the helper used in
    /// `finance-core::enrichment::llm::tests`. Caller must hold the
    /// `#[serial_test::serial]` lock.
    pub fn clear_llm_env() {
        const VARS: [&str; 6] = [
            "FINANCE_LLM_PROVIDER",
            "FINANCE_LLM_MODEL",
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "DEEPSEEK_API_KEY",
            "OLLAMA_BASE_URL",
        ];
        for v in VARS {
            // SAFETY: caller serializes with #[serial].
            unsafe {
                std::env::remove_var(v);
            }
        }
    }

    /// Trivial `FinanceStore` that returns empty results for every read
    /// and counts calls to `transaction_by_id`. Phase-5 tests only need
    /// to assert that the hook doesn't reach the store when the
    /// pipeline can't be built.
    #[derive(Default)]
    pub struct NoopStore {
        pub transaction_by_id_calls: Mutex<usize>,
    }

    #[async_trait(?Send)]
    impl FinanceStore for NoopStore {
        async fn applied_migrations(&self) -> Result<BTreeSet<String>> {
            Ok(BTreeSet::new())
        }
        async fn apply_sql(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn record_migration(&self, _: &str) -> Result<()> {
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
        async fn cards_open_now(&self) -> Result<Vec<finance_core::models::CardSummaryRow>> {
            Ok(vec![])
        }
        async fn latest_account_snapshots(&self) -> Result<Vec<AccountSnapshotRecord>> {
            Ok(vec![])
        }
        async fn apply_transaction_split(
            &self,
            _: &TransactionSplitRecord,
            _: &[TransactionSplitLineRecord],
            _: &[ReceiptItemRecord],
        ) -> Result<()> {
            Ok(())
        }
        async fn insert_audit_events(&self, _: &[AuditEvent]) -> Result<usize> {
            Ok(0)
        }
        async fn annotate_transaction(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
            _: Option<&str>,
            _: &str,
            _: &str,
        ) -> Result<()> {
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
            *self.transaction_by_id_calls.lock().unwrap() += 1;
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
        async fn list_all_category_ids(&self) -> Result<BTreeSet<String>> {
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
        async fn cashflow_month(&self, _: &str) -> Result<CashflowRow> {
            Ok(CashflowRow {
                month_ref: String::new(),
                income: Decimal::ZERO,
                expenses: Decimal::ZERO,
                expense_reduction: Decimal::ZERO,
                net: Decimal::ZERO,
                opening_balance: None,
                closing_balance: None,
            })
        }
        async fn checking_balance_at(&self, _: NaiveDate) -> Result<Option<CheckingBalance>> {
            Ok(None)
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
            Ok(vec![])
        }
        async fn similar_transactions(
            &self,
            _: &str,
            _: &str,
            _: bool,
        ) -> Result<Vec<TransactionRecord>> {
            Ok(vec![])
        }
        async fn mark_enrichment_attempted(&self, _: &str, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        async fn find_anatomy_donors(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<finance_core::models::TransactionRecord>> {
            Ok(vec![])
        }
        async fn replicable_anatomy_candidates(
            &self,
            _: usize,
        ) -> Result<Vec<finance_core::models::TransactionRecord>> {
            Ok(vec![])
        }
    }
}
