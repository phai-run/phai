//! Shared types for Phase 1 enrichment: CNPJ lookup, pluggy hints,
//! temporal context, heuristic features.
//!
//! The pipeline (Phase 3) and LLM layer (Phase 2) consume these types but
//! they live here so the storage layer can also produce `ContextTx` rows.

use chrono::Weekday;
use rust_decimal::Decimal;
use schemars::JsonSchema;
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

/// Result returned by the LLM enrichment call.
///
/// `reasoning` is intentionally the **first** field so the LLM emits its
/// chain-of-thought before committing to a classification (calibration
/// trick — keeps the model honest about low-confidence cases).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EnrichmentResult {
    /// Curto raciocínio em PT-BR explicando como chegou na categoria.
    pub reasoning: String,
    /// Nome limpo do estabelecimento (sem "LTDA", "PIX", etc.).
    pub merchant_name: String,
    /// Categoria principal (taxonomia interna).
    pub category: String,
    /// Subcategoria dentro da categoria principal.
    pub subcategory: String,
    /// Confiança no intervalo [0.0, 1.0].
    pub confidence: f32,
    /// `true` quando o LLM acredita que precisa de input humano para
    /// classificar com segurança.
    pub needs_user_input: bool,
    /// Pergunta em PT-BR para o usuário caso `needs_user_input` seja
    /// `true`.
    pub user_prompt: Option<String>,
}

/// Pipeline decision derived from an `EnrichmentResult` by applying
/// confidence thresholds.
#[derive(Debug, Clone)]
pub enum EnrichmentDecision {
    /// `confidence >= AUTO_THRESHOLD` and `!needs_user_input` →
    /// aplica automaticamente.
    AutoApply { result: EnrichmentResult },
    /// `confidence >= SUGGEST_THRESHOLD` → sugere ao usuário.
    Suggest { result: EnrichmentResult },
    /// Caso contrário → pergunta ao usuário usando `user_prompt`.
    AskUser { result: EnrichmentResult },
}

/// Threshold above which the pipeline auto-applies the LLM suggestion.
pub const AUTO_THRESHOLD: f32 = 0.85;
/// Threshold above which the pipeline merely suggests the result.
pub const SUGGEST_THRESHOLD: f32 = 0.60;

impl EnrichmentDecision {
    /// Apply the threshold ladder. `needs_user_input == true` overrides
    /// the auto-apply branch — high confidence is not enough if the
    /// model itself flagged ambiguity.
    pub fn from_result(result: EnrichmentResult) -> Self {
        if result.confidence >= AUTO_THRESHOLD && !result.needs_user_input {
            Self::AutoApply { result }
        } else if result.confidence >= SUGGEST_THRESHOLD {
            Self::Suggest { result }
        } else {
            Self::AskUser { result }
        }
    }
}

/// Historical user-labelled transaction used as a few-shot example in
/// the prompt. Selected by lexical similarity to the transaction under
/// review.
#[derive(Debug, Clone)]
pub struct FewShotExample {
    pub description: String,
    pub amount: Decimal,
    pub category: String,
    pub subcategory: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_result(confidence: f32, needs_user_input: bool) -> EnrichmentResult {
        EnrichmentResult {
            reasoning: "porque sim".to_string(),
            merchant_name: "Sapiens".to_string(),
            category: "alimentacao".to_string(),
            subcategory: "restaurantes".to_string(),
            confidence,
            needs_user_input,
            user_prompt: None,
        }
    }

    #[test]
    fn test_enrichment_decision_auto_when_high_confidence() {
        let decision = EnrichmentDecision::from_result(sample_result(0.92, false));
        assert!(matches!(decision, EnrichmentDecision::AutoApply { .. }));
    }

    #[test]
    fn test_enrichment_decision_suggest_when_medium() {
        let decision = EnrichmentDecision::from_result(sample_result(0.72, false));
        assert!(matches!(decision, EnrichmentDecision::Suggest { .. }));
    }

    #[test]
    fn test_enrichment_decision_ask_when_low() {
        let decision = EnrichmentDecision::from_result(sample_result(0.45, false));
        assert!(matches!(decision, EnrichmentDecision::AskUser { .. }));
    }

    #[test]
    fn test_enrichment_decision_ask_when_needs_user_input_overrides_high_confidence() {
        // confidence = 0.9 should normally AutoApply, but needs_user_input
        // forces at most Suggest.
        let decision = EnrichmentDecision::from_result(sample_result(0.9, true));
        match decision {
            EnrichmentDecision::Suggest { .. } => {}
            other => panic!("expected Suggest, got {other:?}"),
        }
    }

    #[test]
    fn test_enrichment_result_deserializes_with_reasoning_first() {
        // Ensure `reasoning` is the first key when serializing (this is
        // what the LLM will see in the example/tool calling context).
        // schemars `BTreeMap` sorts keys alphabetically, so we check
        // serialization output (`serde` preserves struct field order).
        let value = EnrichmentResult {
            reasoning: "x".into(),
            merchant_name: "m".into(),
            category: "c".into(),
            subcategory: "s".into(),
            confidence: 0.5,
            needs_user_input: false,
            user_prompt: None,
        };
        let serialized = serde_json::to_string(&value).unwrap();
        let reasoning_pos = serialized.find("\"reasoning\"").expect("has key");
        let merchant_pos = serialized.find("\"merchant_name\"").expect("has key");
        assert!(
            reasoning_pos < merchant_pos,
            "reasoning must serialize before merchant_name (got {serialized})"
        );

        // Round-trip a JSON payload.
        let json = r#"{
            "reasoning": "domingo de feira no Sapiens",
            "merchant_name": "Sapiens",
            "category": "alimentacao",
            "subcategory": "restaurantes",
            "confidence": 0.91,
            "needs_user_input": false,
            "user_prompt": null
        }"#;
        let parsed: EnrichmentResult = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.merchant_name, "Sapiens");
        assert!((parsed.confidence - 0.91).abs() < 1e-6);

        // And the schema does include `reasoning` as a property.
        let schema = schemars::schema_for!(EnrichmentResult);
        let obj = schema.schema.object.expect("object schema");
        assert!(obj.properties.contains_key("reasoning"));
    }
}
