use crate::idempotency::category_id;
use crate::models::RuleRecord;
use anyhow::{bail, Context, Result};
use deunicode::deunicode;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRule {
    pub rule_id: String,
    pub category_id: String,
    matcher: RuleMatcher,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuleMatcher {
    ContainsAny(Vec<String>),
    Exact(String),
}

pub fn compile_rules(rows: &[RuleRecord]) -> Result<Vec<CompiledRule>> {
    let mut compiled = Vec::new();
    for row in rows
        .iter()
        .filter(|row| row.status.eq_ignore_ascii_case("active"))
    {
        if let Some(rule) = compile_rule(row)? {
            compiled.push(rule);
        }
    }
    compiled.sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
    Ok(compiled)
}

pub fn apply_rules(
    description: &str,
    base_category_id: Option<String>,
    had_base_category: bool,
    rules: &[CompiledRule],
) -> (Option<String>, String) {
    let normalized_description = normalize_text(description);
    if let Some(rule) = rules
        .iter()
        .find(|rule| rule.matches(&normalized_description))
    {
        return (Some(rule.category_id.clone()), "rule".to_string());
    }

    let source = if had_base_category {
        "pluggy"
    } else {
        "unclassified"
    };
    (base_category_id, source.to_string())
}

impl CompiledRule {
    fn matches(&self, normalized_description: &str) -> bool {
        match &self.matcher {
            RuleMatcher::ContainsAny(needles) => needles
                .iter()
                .any(|needle| normalized_description.contains(needle)),
            RuleMatcher::Exact(needle) => normalized_description == needle,
        }
    }
}

fn compile_rule(row: &RuleRecord) -> Result<Option<CompiledRule>> {
    if let Some(rule) = parse_dsl_rule(row)? {
        return Ok(Some(rule));
    }
    parse_json_rule(row)
}

fn parse_dsl_rule(row: &RuleRecord) -> Result<Option<CompiledRule>> {
    let body = row.body.trim();
    if body.starts_with('{') || body.starts_with('[') {
        return Ok(None);
    }

    let normalized = normalize_text(body);
    let prefix = "if description contains ";
    let separator = " then category ";
    if !normalized.starts_with(prefix) {
        bail!(
            "Regra {} usa formato não suportado. Use `if description contains ... then category ...`",
            row.rule_id
        );
    }
    let tail = normalized
        .strip_prefix(prefix)
        .context("Prefixo inválido na regra DSL")?;
    let (needle, target) = tail.split_once(separator).with_context(|| {
        format!(
            "Regra {} inválida. Esperado `if description contains ... then category ...`",
            row.rule_id
        )
    })?;
    build_contains_rule(
        &row.rule_id,
        vec![strip_wrapping_quotes(needle).to_string()],
        normalize_category_target(target)?,
    )
    .map(Some)
}

fn parse_json_rule(row: &RuleRecord) -> Result<Option<CompiledRule>> {
    let value: Value = serde_json::from_str(&row.body)
        .with_context(|| format!("Rule {} contém JSON inválido", row.rule_id))?;

    if let Some(rule) = parse_legacy_context_rule(&row.rule_id, &value)? {
        return Ok(Some(rule));
    }
    if let Some(rule) = parse_legacy_yaml_rule(&row.rule_id, &value)? {
        return Ok(Some(rule));
    }

    Ok(None)
}

fn parse_legacy_context_rule(rule_id: &str, value: &Value) -> Result<Option<CompiledRule>> {
    let match_type = value
        .get("match_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(
        match_type,
        "descricao_contains" | "description_contains" | "descricao_exata" | "description_exact"
    ) {
        return Ok(None);
    }

    let needle = value
        .get("match_value")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(needle) = needle else {
        return Ok(None);
    };
    let category = value
        .get("categoria")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(category) = category else {
        return Ok(None);
    };
    let subcategory = value
        .get("subcategoria")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let category_id = category_id(category, subcategory);
    let normalized_needle = normalize_text(needle);
    let rule = match match_type {
        "descricao_exata" | "description_exact" => {
            build_exact_rule(rule_id, normalized_needle, category_id)?
        }
        _ => build_contains_rule(rule_id, vec![normalized_needle], category_id)?,
    };

    Ok(Some(rule))
}

fn parse_legacy_yaml_rule(rule_id: &str, value: &Value) -> Result<Option<CompiledRule>> {
    let contains_any = value
        .get("match")
        .and_then(|matcher| matcher.get("contains_any"))
        .and_then(Value::as_array);
    let Some(contains_any) = contains_any else {
        return Ok(None);
    };

    let needles = contains_any
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_text)
        .collect::<Vec<_>>();
    if needles.is_empty() {
        bail!("Rule {rule_id} sem `match.contains_any` válido");
    }

    let category = value
        .get("set")
        .and_then(|setter| setter.get("category"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("Rule {rule_id} sem `set.category`"))?;
    let subcategory = value
        .get("set")
        .and_then(|setter| setter.get("subcategory"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    build_contains_rule(rule_id, needles, category_id(category, subcategory)).map(Some)
}

fn build_contains_rule(
    rule_id: &str,
    needles: Vec<String>,
    category_id: String,
) -> Result<CompiledRule> {
    let needles = needles
        .into_iter()
        .map(|needle| needle.trim().to_string())
        .filter(|needle| !needle.is_empty())
        .collect::<Vec<_>>();
    if needles.is_empty() {
        bail!("Rule {rule_id} sem termo de busca");
    }
    if category_id.trim().is_empty() {
        bail!("Rule {rule_id} sem categoria de destino");
    }
    Ok(CompiledRule {
        rule_id: rule_id.to_string(),
        category_id,
        matcher: RuleMatcher::ContainsAny(needles),
    })
}

fn build_exact_rule(rule_id: &str, needle: String, category_id: String) -> Result<CompiledRule> {
    let needle = needle.trim().to_string();
    if needle.is_empty() {
        bail!("Rule {rule_id} sem termo de busca");
    }
    if category_id.trim().is_empty() {
        bail!("Rule {rule_id} sem categoria de destino");
    }
    Ok(CompiledRule {
        rule_id: rule_id.to_string(),
        category_id,
        matcher: RuleMatcher::Exact(needle),
    })
}

fn normalize_category_target(raw: &str) -> Result<String> {
    let trimmed = strip_wrapping_quotes(raw.trim());
    if trimmed.is_empty() {
        bail!("Categoria de destino vazia");
    }
    let mut parts = trimmed.splitn(2, ':');
    let category = parts.next().unwrap_or_default();
    let subcategory = parts.next();
    Ok(category_id(category, subcategory))
}

fn strip_wrapping_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

fn normalize_text(value: &str) -> String {
    deunicode(value).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn rule(rule_id: &str, body: &str) -> RuleRecord {
        RuleRecord {
            rule_id: rule_id.to_string(),
            body: body.to_string(),
            status: "active".to_string(),
            actor_id: "test-actor".to_string(),
            idempotency_key: format!("rule:{rule_id}"),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn parses_dsl_rule_with_quotes() {
        let compiled = compile_rules(&[rule(
            "pix_credito",
            "if description contains \"PIX no Crédito\" then category transfer-internal",
        )])
        .unwrap();

        let (category_id, source) = apply_rules(
            "Valor adicionado na conta via PIX no Credito",
            None,
            false,
            &compiled,
        );
        assert_eq!(category_id.as_deref(), Some("transfer-internal"));
        assert_eq!(source, "rule");
    }

    #[test]
    fn parses_legacy_context_json_rule() {
        let compiled = compile_rules(&[rule(
            "mercado_rule",
            r#"{"match_type":"descricao_contains","match_value":"mercado","categoria":"Alimentacao","subcategoria":"Mercado"}"#,
        )])
        .unwrap();

        let (category_id, source) = apply_rules("Compra no Mercado X", None, false, &compiled);
        assert_eq!(category_id.as_deref(), Some("alimentacao:mercado"));
        assert_eq!(source, "rule");
    }

    #[test]
    fn parses_legacy_exact_json_rule() {
        let compiled = compile_rules(&[rule(
            "bill_rule",
            r#"{"match_type":"descricao_exata","match_value":"Pagamento de fatura","categoria":"credit-card-payment"}"#,
        )])
        .unwrap();

        let (category_id, source) = apply_rules("Pagamento de fatura", None, false, &compiled);
        assert_eq!(category_id.as_deref(), Some("credit-card-payment"));
        assert_eq!(source, "rule");

        let (category_id, source) = apply_rules("Pagamento de fatura Visa", None, false, &compiled);
        assert_eq!(category_id, None);
        assert_eq!(source, "unclassified");
    }

    #[test]
    fn ignores_unsupported_json_rule_shapes() {
        let compiled = compile_rules(&[rule("unknown_rule", r#"{"foo":"bar"}"#)]).unwrap();
        assert!(compiled.is_empty());
    }

    #[test]
    fn first_rule_is_deterministic_by_rule_id() {
        let compiled = compile_rules(&[
            rule(
                "20_secondary",
                "if description contains farmacia then category saude:secundario",
            ),
            rule(
                "10_primary",
                "if description contains farmacia then category saude:farmacia",
            ),
        ])
        .unwrap();

        let (category_id, source) = apply_rules("Farmacia Central", None, false, &compiled);
        assert_eq!(category_id.as_deref(), Some("saude:farmacia"));
        assert_eq!(source, "rule");
    }
}
