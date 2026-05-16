//! Shared types for Phase 1 enrichment: CNPJ lookup, pluggy hints,
//! temporal context, heuristic features.
//!
//! The pipeline (Phase 3) and LLM layer (Phase 2) consume these types but
//! they live here so the storage layer can also produce `ContextTx` rows.

use chrono::Weekday;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Information returned by the BrasilAPI `cnpj/v1/{cnpj}` endpoint.
///
/// Stored in both the in-memory (moka) cache and the SQLite `cnpj_cache`
/// table for cross-run reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CnpjInfo {
    /// 14 digit CNPJ, unformatted.
    pub cnpj: String,
    pub razao_social: String,
    #[serde(default)]
    pub nome_fantasia: Option<String>,
    /// Primary CNAE (7 digits, `XX.XX-X/XX`).
    pub cnae_fiscal: u32,
    pub cnae_descricao: String,
    /// Secondary CNAEs returned by BrasilAPI. May be empty.
    #[serde(default)]
    pub cnaes_secundarios: Vec<(u32, String)>,
}

/// Hint derived from Pluggy's coarse category. `confidence_boost` is
/// added to the LLM's confidence score when category/subcategory agree
/// with the LLM output.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryHint {
    pub category: Option<&'static str>,
    pub subcategory: Option<&'static str>,
    /// Boost in [0.0, 0.30] for agreement reinforcement.
    pub confidence_boost: f32,
}

impl CategoryHint {
    pub const fn empty() -> Self {
        Self {
            category: None,
            subcategory: None,
            confidence_boost: 0.0,
        }
    }
}

/// Sibling transactions on the same day/account, used as temporal
/// context in the LLM prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextTx {
    pub description: String,
    pub amount: Decimal,
    #[serde(default)]
    pub pluggy_category: Option<String>,
    #[serde(default)]
    pub order: Option<i64>,
}

/// Time-of-day bucket derived from a 24h hour. Phase 1 only needs the
/// four broad buckets — the LLM can ask for finer granularity if needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HourBucket {
    Madrugada,
    Manha,
    Tarde,
    Noite,
}

/// Heuristic features computed per transaction before the LLM is called.
/// `is_recurring` is filled in by the pipeline via `similar_transactions`.
#[derive(Debug, Clone)]
pub struct Heuristics {
    pub is_round_number: bool,
    pub hour_bucket: Option<HourBucket>,
    pub weekday: Weekday,
    pub is_recurring: bool,
}
