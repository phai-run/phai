//! Transaction-enrichment pipeline.
//!
//! Layout:
//!   - [`types`]: shared structs (`CnpjInfo`, `CategoryHint`, `ContextTx`,
//!     `Heuristics`, `HourBucket`, `EnrichmentResult`, `EnrichmentDecision`).
//!   - [`cnpj`]: extraction, normalization, BrasilAPI lookup, 2-layer
//!     cache, CNAE → category mapping.
//!   - [`pluggy_map`]: coarse Pluggy category → internal category hint.
//!   - [`context`]: temporal-context helpers.
//!   - [`heuristics`]: cheap pre-LLM features.
//!   - [`prompt`] (Phase 2): builds the PT-BR LLM prompt with
//!     stop-words cleaning, multi-CNAE disambiguation, few-shot
//!     examples, temporal context, and heuristics.
//!   - [`llm`] (Phase 2): provider selection (Anthropic, OpenAI,
//!     Deepseek, Ollama) and structured enrichment call via rig-core.

pub mod cnpj;
pub mod context;
pub mod fuzzy;
pub mod heuristics;
pub mod llm;
pub mod pipeline;
pub mod pluggy_map;
pub mod prompt;
pub mod rule_gen;
pub mod types;
pub mod web_search;

pub use types::{
    CategoryHint, CnpjInfo, ContextTx, EnrichmentDecision, EnrichmentResult, FewShotExample,
    Heuristics, HourBucket, AUTO_THRESHOLD, SUGGEST_THRESHOLD,
};
