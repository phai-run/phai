//! Prompt builder for the LLM enrichment call.
//!
//! Goals:
//!   - Compact PT-BR prompt that the model can answer with a single
//!     JSON object (`EnrichmentResult`).
//!   - Pre-strip stop-words + finance noise from the raw description to
//!     save tokens and reduce false signal.
//!   - Bundle all available signals (Pluggy hint, CNPJ, heuristics,
//!     temporal context, few-shot history) in a deterministic order so
//!     the prompt is cacheable across calls.
//!
//! The system prompt expects the model to return JSON matching
//! `crate::enrichment::types::EnrichmentResult` and nothing else.

use crate::enrichment::types::{CategoryHint, CnpjInfo, ContextTx, FewShotExample, Heuristics};
use chrono::{Datelike, NaiveDate, Weekday};
use rust_decimal::Decimal;
use std::fmt::Write as _;
use std::sync::OnceLock;

/// Aggregated input for [`build_prompt`].
pub struct PromptContext<'a> {
    pub description: &'a str,
    pub amount: Decimal,
    pub date: NaiveDate,
    pub hour: Option<u32>,
    pub pluggy_category: Option<&'a str>,
    pub pluggy_hint: Option<&'a CategoryHint>,
    pub cnpj_info: Option<&'a CnpjInfo>,
    pub receiver_name: Option<&'a str>,
    /// "CPF" or "CNPJ" — we suppress the company section when the
    /// counterparty is a natural person.
    pub document_type: Option<&'a str>,
    pub heuristics: &'a Heuristics,
    pub temporal_context: &'a [ContextTx],
    pub few_shot_examples: &'a [FewShotExample],
}

/// Build the full prompt string ready to send to the LLM.
pub fn build_prompt(ctx: &PromptContext) -> String {
    let mut out = String::with_capacity(2048);

    // 1. System role + reasoning-first contract.
    out.push_str(
        "Você é um classificador de transações financeiras pessoais em PT-BR.\n\
         Seu trabalho é categorizar transações usando todos os sinais disponíveis.\n\
         SEMPRE explique seu raciocínio em `reasoning` ANTES de decidir a categoria.\n\
         Se não houver evidência forte, defina `needs_user_input: true` e escreva\n\
         em `user_prompt` uma pergunta curta em PT-BR para o usuário.\n\n",
    );

    // 2. Taxonomy.
    out.push_str("## Taxonomia de categorias\n");
    out.push_str(taxonomy_block());
    out.push('\n');

    // 3. Transaction core fields.
    let cleaned = clean_description(ctx.description);
    out.push_str("## Transação em análise\n");
    let _ = writeln!(out, "- Descrição (limpa): {cleaned}");
    let _ = writeln!(out, "- Descrição (bruta): {}", ctx.description.trim());
    let _ = writeln!(out, "- Valor: R$ {}", ctx.amount);
    let _ = writeln!(
        out,
        "- Data: {} ({})",
        ctx.date,
        weekday_pt(ctx.date.weekday())
    );
    if let Some(hour) = ctx.hour {
        let _ = writeln!(out, "- Hora: {hour:02}h");
    }
    if let Some(pluggy) = ctx.pluggy_category {
        let _ = writeln!(out, "- Categoria Pluggy: {pluggy}");
    }
    if let Some(hint) = ctx.pluggy_hint {
        if let (Some(cat), boost) = (hint.category, hint.confidence_boost) {
            let _ = writeln!(
                out,
                "- Sugestão Pluggy mapeada: {cat}{} (boost={boost:.2})",
                hint.subcategory
                    .map(|s| format!("/{s}"))
                    .unwrap_or_default()
            );
        }
    }
    out.push('\n');

    // 4. CNPJ / CPF block.
    let doc_type = ctx.document_type.unwrap_or("");
    if doc_type.eq_ignore_ascii_case("CNPJ") {
        if let Some(info) = ctx.cnpj_info {
            out.push_str("## Empresa identificada (via CNPJ)\n");
            let _ = writeln!(out, "- Razão social: {}", info.razao_social);
            if let Some(fant) = &info.nome_fantasia {
                if !fant.is_empty() {
                    let _ = writeln!(out, "- Nome fantasia: {fant}");
                }
            }
            let _ = writeln!(
                out,
                "- CNAE primário: {} — {}",
                info.cnae_fiscal, info.cnae_descricao
            );
            if !info.cnaes_secundarios.is_empty() {
                out.push_str("- CNAEs secundários:\n");
                for (code, desc) in &info.cnaes_secundarios {
                    let _ = writeln!(out, "  - {code} — {desc}");
                }
            }
            out.push('\n');
        }
    } else if doc_type.eq_ignore_ascii_case("CPF") {
        out.push_str("## Pessoa física (CPF)\n");
        if let Some(name) = ctx.receiver_name {
            let _ = writeln!(out, "- Nome do recebedor: {name}");
        }
        out.push_str(
            "- Não há base pública para consulta de CPF.\n\
             - Use padrão temporal + valor + dia da semana para inferir.\n\n",
        );
    }

    // 5. Heuristics.
    out.push_str("## Heurísticas\n");
    let _ = writeln!(
        out,
        "- Valor redondo: {}",
        if ctx.heuristics.is_round_number {
            "sim"
        } else {
            "não"
        }
    );
    if let Some(bucket) = ctx.heuristics.hour_bucket {
        let _ = writeln!(out, "- Hour bucket: {:?}", bucket);
    }
    let _ = writeln!(
        out,
        "- Dia da semana: {}",
        weekday_pt(ctx.heuristics.weekday)
    );
    let _ = writeln!(
        out,
        "- Recorrente nos últimos 3 meses: {}",
        if ctx.heuristics.is_recurring {
            "sim"
        } else {
            "não"
        }
    );
    out.push('\n');

    // 6. Temporal context.
    if !ctx.temporal_context.is_empty() {
        out.push_str("## Contexto temporal (mesmas data/conta)\n");
        for tx in ctx.temporal_context {
            let cat = tx
                .pluggy_category
                .as_deref()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            let _ = writeln!(out, "- {} | R$ {}{}", tx.description.trim(), tx.amount, cat);
        }
        out.push('\n');
    }

    // 7. Few-shot examples.
    if !ctx.few_shot_examples.is_empty() {
        out.push_str("## Exemplos do histórico do usuário\n");
        for ex in ctx.few_shot_examples {
            let _ = writeln!(
                out,
                "- \"{}\" | R$ {} → {}:{}",
                ex.description.trim(),
                ex.amount,
                ex.category,
                ex.subcategory
            );
        }
        out.push('\n');
    }

    // 8. Final instruction.
    out.push_str(
        "## Instrução final\n\
         Responda APENAS com JSON do schema EnrichmentResult, sem texto adicional.\n\
         Campos: reasoning, merchant_name, category, subcategory, confidence (0..1),\n\
         needs_user_input, user_prompt.\n",
    );

    out
}

/// Strip Portuguese stop-words and finance-domain noise tokens. Returns
/// a space-separated lower-cased string of remaining tokens (preserving
/// proper-noun-looking segments such as `Sapiens`).
pub fn clean_description(description: &str) -> String {
    let extra = extra_noise();
    let stop = pt_stop_words();

    description
        .split(|c: char| !c.is_alphanumeric())
        .filter(|tok| !tok.is_empty())
        .filter(|tok| {
            let lower = tok.to_lowercase();
            !stop.contains(&lower) && !extra.contains(lower.as_str())
        })
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn pt_stop_words() -> &'static std::collections::HashSet<String> {
    static CELL: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    CELL.get_or_init(|| {
        stop_words::get(stop_words::LANGUAGE::Portuguese)
            .into_iter()
            .map(|w| w.to_lowercase())
            .collect()
    })
}

fn extra_noise() -> &'static std::collections::HashSet<&'static str> {
    static CELL: OnceLock<std::collections::HashSet<&'static str>> = OnceLock::new();
    CELL.get_or_init(|| {
        [
            "ltda",
            "s.a.",
            "sa",
            "me",
            "epp",
            "transferencia",
            "transferência",
            "pix",
            "pagamento",
            "pgto",
            "compra",
            "no",
            "debito",
            "débito",
            "credito",
            "crédito",
            "enviada",
            "recebida",
            "transf",
        ]
        .into_iter()
        .collect()
    })
}

fn weekday_pt(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "segunda-feira",
        Weekday::Tue => "terça-feira",
        Weekday::Wed => "quarta-feira",
        Weekday::Thu => "quinta-feira",
        Weekday::Fri => "sexta-feira",
        Weekday::Sat => "sábado",
        Weekday::Sun => "domingo",
    }
}

fn taxonomy_block() -> &'static str {
    "- alimentacao: restaurantes, mercado, padaria, delivery, bar
- transporte: combustivel, taxi-app, transporte-publico, estacionamento, manutencao
- saude: farmacia, consulta, exame, plano
- educacao: escola, curso, livros
- moradia: aluguel, condominio, energia, agua, internet, manutencao
- lazer: viagem, evento, streaming, hobby
- pessoal: vestuario, cuidado-fisico, presentes, servicos
- compras: varejo, eletronicos, casa
- financeiro: investimento, emprestimo, tarifa, imposto
- renda: salario, freelance, reembolso
- outros: nao-categorizado
"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrichment::types::{CnpjInfo, ContextTx, Heuristics, HourBucket};
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    fn base_heuristics() -> Heuristics {
        Heuristics {
            is_round_number: false,
            hour_bucket: Some(HourBucket::Tarde),
            weekday: Weekday::Sun,
            is_recurring: false,
        }
    }

    fn ctx_with<'a>(
        cnpj_info: Option<&'a CnpjInfo>,
        receiver_name: Option<&'a str>,
        doc_type: Option<&'a str>,
        ctx_txs: &'a [ContextTx],
        examples: &'a [FewShotExample],
        heuristics: &'a Heuristics,
    ) -> PromptContext<'a> {
        PromptContext {
            description: "Sapiens Parque Restaurant LTDA",
            amount: Decimal::new(-2590, 2),
            date: NaiveDate::from_ymd_opt(2026, 5, 3).unwrap(),
            hour: Some(13),
            pluggy_category: Some("Eating out"),
            pluggy_hint: None,
            cnpj_info,
            receiver_name,
            document_type: doc_type,
            heuristics,
            temporal_context: ctx_txs,
            few_shot_examples: examples,
        }
    }

    fn sample_cnpj() -> CnpjInfo {
        CnpjInfo {
            cnpj: "12345678000199".to_string(),
            razao_social: "SAPIENS PARQUE GASTRONOMIA LTDA".to_string(),
            nome_fantasia: Some("Sapiens".to_string()),
            cnae_fiscal: 5611201,
            cnae_descricao: "Restaurantes e similares".to_string(),
            cnaes_secundarios: vec![
                (5620104, "Fornecimento de alimentos preparados".to_string()),
                (5612100, "Serviços ambulantes de alimentação".to_string()),
            ],
        }
    }

    #[test]
    fn test_build_prompt_includes_temporal_context() {
        let h = base_heuristics();
        let ctx_txs = vec![
            ContextTx {
                description: "Oliva Cheese Bar".to_string(),
                amount: Decimal::new(-4500, 2),
                pluggy_category: Some("Eating out".to_string()),
                order: Some(1),
            },
            ContextTx {
                description: "Brasil Berry Natural F".to_string(),
                amount: Decimal::new(-3100, 2),
                pluggy_category: None,
                order: Some(2),
            },
        ];
        let prompt = build_prompt(&ctx_with(None, None, None, &ctx_txs, &[], &h));
        assert!(prompt.contains("Oliva Cheese Bar"));
        assert!(prompt.contains("Brasil Berry Natural F"));
        assert!(prompt.contains("Contexto temporal"));
    }

    #[test]
    fn test_build_prompt_includes_cnpj_info() {
        let h = base_heuristics();
        let cnpj = sample_cnpj();
        let prompt = build_prompt(&ctx_with(Some(&cnpj), None, Some("CNPJ"), &[], &[], &h));
        assert!(prompt.contains("SAPIENS PARQUE GASTRONOMIA LTDA"));
        assert!(prompt.contains("Restaurantes e similares"));
        assert!(prompt.contains("5611201"));
        assert!(prompt.contains("Empresa identificada"));
    }

    #[test]
    fn test_build_prompt_cpf_no_cnpj_section() {
        let h = base_heuristics();
        let prompt = build_prompt(&ctx_with(
            None,
            Some("JOICE ANTONIA DA SILVA"),
            Some("CPF"),
            &[],
            &[],
            &h,
        ));
        assert!(!prompt.contains("Empresa identificada"));
        assert!(prompt.contains("Pessoa física"));
        assert!(prompt.contains("JOICE ANTONIA DA SILVA"));
    }

    #[test]
    fn test_build_prompt_few_shot_examples_rendered() {
        let h = base_heuristics();
        let examples = vec![
            FewShotExample {
                description: "Sapiens Parque".to_string(),
                amount: Decimal::new(-3000, 2),
                category: "alimentacao".to_string(),
                subcategory: "restaurantes".to_string(),
            },
            FewShotExample {
                description: "Oliva Cheese".to_string(),
                amount: Decimal::new(-4500, 2),
                category: "alimentacao".to_string(),
                subcategory: "restaurantes".to_string(),
            },
        ];
        let prompt = build_prompt(&ctx_with(None, None, None, &[], &examples, &h));
        assert!(prompt.contains("Exemplos do histórico"));
        assert!(prompt.contains("Sapiens Parque"));
        assert!(prompt.contains("alimentacao:restaurantes"));
    }

    #[test]
    fn test_build_prompt_multi_cnae_listed() {
        let h = base_heuristics();
        let cnpj = sample_cnpj();
        let prompt = build_prompt(&ctx_with(Some(&cnpj), None, Some("CNPJ"), &[], &[], &h));
        assert!(prompt.contains("CNAEs secundários"));
        assert!(prompt.contains("5620104"));
        assert!(prompt.contains("5612100"));
        assert!(prompt.contains("Fornecimento de alimentos preparados"));
    }

    #[test]
    fn test_clean_description_removes_stopwords() {
        let cleaned = clean_description("PIX TRANSFERENCIA PARA SAPIENS LTDA");
        let lower = cleaned.to_lowercase();
        assert!(!lower.contains("pix"));
        assert!(!lower.contains("transferencia"));
        assert!(!lower.contains("ltda"));
        // "para" is a Portuguese stop-word.
        assert!(!lower.contains(" para "));
    }

    #[test]
    fn test_clean_description_preserves_merchant_name() {
        let cleaned = clean_description("COMPRA NO DEBITO SAPIENS PARQUE");
        // Merchant tokens must survive even though "compra"/"no"/"debito"
        // are stripped.
        let lower = cleaned.to_lowercase();
        assert!(lower.contains("sapiens"));
        assert!(lower.contains("parque"));
        assert!(!lower.contains("compra"));
        assert!(!lower.contains("debito"));
    }
}
