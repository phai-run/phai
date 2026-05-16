//! Generate a DSL `RuleRecord` from a successful [`EnrichmentResult`].
//!
//! Phase 4 wires this into the CLI: after a category is applied (auto
//! or user-confirmed), we mint a deterministic rule so the next sync
//! catches the same merchant without involving the LLM again.
//!
//! Keyword choice: `result.merchant_name` lowercased and trimmed.
//! `merchant_name` is the LLM's cleaned merchant string (no "LTDA",
//! "PIX", etc.), so it's a much safer matcher than raw description
//! tokens.

use crate::enrichment::types::EnrichmentResult;
use crate::models::RuleRecord;
use anyhow::{bail, Result};
use chrono::Utc;

const MIN_KEYWORD_LEN: usize = 3;

/// Extract the keyword used both for the rule body and for retroactive
/// fuzzy matching. Lowercased, trimmed.
///
/// Errors when the merchant_name is empty or shorter than
/// [`MIN_KEYWORD_LEN`] (after trimming) — short keywords produce
/// indiscriminate rules.
pub fn keyword_from_result(result: &EnrichmentResult) -> Result<String> {
    let kw = result.merchant_name.trim().to_lowercase();
    if kw.is_empty() {
        bail!("merchant_name vazio — sem keyword para gerar regra");
    }
    if kw.chars().count() < MIN_KEYWORD_LEN {
        bail!(
            "keyword muito curta ({} chars): {:?}",
            kw.chars().count(),
            kw
        );
    }
    Ok(kw)
}

/// Slugify a keyword for the rule_id suffix. Collapses non-alphanumeric
/// runs to a single `_` and strips leading/trailing underscores.
fn slugify(keyword: &str) -> String {
    let mut out = String::with_capacity(keyword.len());
    let mut last_underscore = true; // start in skipping mode so leading non-alnum is dropped
    for c in keyword.chars() {
        if c.is_alphanumeric() {
            for low in c.to_lowercase() {
                out.push(low);
            }
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    // strip trailing _
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// `enriched_{slug}` — deterministic so re-running on the same merchant
/// idempotently overwrites the same rule row (upsert-friendly).
pub fn generate_rule_id(keyword: &str) -> String {
    format!("enriched_{}", slugify(keyword))
}

/// DSL body. Format:
///   `if description contains "{kw}" then category {cat}:{sub}`
/// or  `if description contains "{kw}" then category {cat}` when
/// subcategory is empty.
pub fn generate_rule_body(result: &EnrichmentResult) -> Result<String> {
    let kw = keyword_from_result(result)?;
    let cat = result.category.trim();
    let sub = result.subcategory.trim();
    let body = if sub.is_empty() {
        format!("if description contains \"{kw}\" then category {cat}")
    } else {
        format!(
            "if description contains \"{kw}\" then category {cat}:{sub}"
        )
    };
    Ok(body)
}

/// Build the full `RuleRecord` ready for upsert. `actor_id` carries the
/// audit identity; `idempotency_key` mirrors the convention used by the
/// `rule add` CLI (`rule:{rule_id}`).
pub fn build_rule_record(result: &EnrichmentResult, actor_id: &str) -> Result<RuleRecord> {
    let kw = keyword_from_result(result)?;
    let rule_id = generate_rule_id(&kw);
    let body = generate_rule_body(result)?;
    let now = Utc::now();
    Ok(RuleRecord {
        rule_id: rule_id.clone(),
        body,
        status: "active".to_string(),
        actor_id: actor_id.to_string(),
        idempotency_key: format!("rule:{rule_id}"),
        created_at: now,
        updated_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(merchant: &str, cat: &str, sub: &str) -> EnrichmentResult {
        EnrichmentResult {
            reasoning: "x".into(),
            merchant_name: merchant.into(),
            category: cat.into(),
            subcategory: sub.into(),
            confidence: 0.9,
            needs_user_input: false,
            user_prompt: None,
        }
    }

    #[test]
    fn test_keyword_basic_lowercase() {
        let r = sample("Sapiens", "alimentacao", "restaurantes");
        assert_eq!(keyword_from_result(&r).unwrap(), "sapiens");
    }

    #[test]
    fn test_keyword_short_errors() {
        let r = sample("Ab", "x", "y");
        assert!(keyword_from_result(&r).is_err());
        let r = sample("", "x", "y");
        assert!(keyword_from_result(&r).is_err());
        let r = sample("   ", "x", "y");
        assert!(keyword_from_result(&r).is_err());
    }

    #[test]
    fn test_generate_rule_id_slug() {
        assert_eq!(generate_rule_id("sapiens"), "enriched_sapiens");
        assert_eq!(
            generate_rule_id("sapiens parque & café"),
            "enriched_sapiens_parque_café"
        );
        assert_eq!(
            generate_rule_id("Sapiens Parque & Café"),
            "enriched_sapiens_parque_café"
        );
        assert_eq!(generate_rule_id("--weird---name--"), "enriched_weird_name");
    }

    #[test]
    fn test_rule_body_with_subcategory() {
        let r = sample("Sapiens", "alimentacao", "restaurantes");
        assert_eq!(
            generate_rule_body(&r).unwrap(),
            "if description contains \"sapiens\" then category alimentacao:restaurantes"
        );
    }

    #[test]
    fn test_rule_body_without_subcategory() {
        let r = sample("Sapiens", "alimentacao", "");
        assert_eq!(
            generate_rule_body(&r).unwrap(),
            "if description contains \"sapiens\" then category alimentacao"
        );
    }

    #[test]
    fn test_duplicate_rule_check_compares_body() {
        // The CLI's duplicate-rule check matches on RuleRecord.body, so
        // two enrichment results with the same merchant/category must
        // produce byte-identical bodies (regardless of timestamps).
        let a = sample("Sapiens", "alimentacao", "restaurantes");
        let b = sample("sapiens", "alimentacao", "restaurantes");
        assert_eq!(generate_rule_body(&a).unwrap(), generate_rule_body(&b).unwrap());

        let c = sample("Sapiens", "lazer", "passeio");
        assert_ne!(generate_rule_body(&a).unwrap(), generate_rule_body(&c).unwrap());
    }

    #[test]
    fn test_build_rule_record_idempotency_key_format() {
        let r = sample("Sapiens", "alimentacao", "restaurantes");
        let rec = build_rule_record(&r, "user-1").unwrap();
        assert_eq!(rec.rule_id, "enriched_sapiens");
        assert_eq!(rec.idempotency_key, "rule:enriched_sapiens");
        assert_eq!(rec.actor_id, "user-1");
        assert_eq!(rec.status, "active");
        assert!(rec.body.contains("sapiens"));
    }
}
