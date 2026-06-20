//! WhatsApp sync message formatter and response parser.
//!
//! Replaces the old inline `render_sync_notify_summary` with a structured
//! message that separates financial snapshot, category grouping, card-payment
//! dedup, and review prompts. Also provides a release-notes formatter and a
//! parser for user text responses.
//!
//! All money is `rust_decimal::Decimal`; no floats except in cosmetic
//! percentage rounding.
//!
//! Several public functions (parser, release notes, category alias resolver)
//! are called from external integration layers (OpenClaw, webhook handlers) and
//! not from within this crate, hence the dead_code allowance.

#![allow(dead_code)]

use chrono::{Datelike, NaiveDate, Timelike, Utc};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::human_format::{
    brl, category_emoji, category_family, family_label, short_description, truncate_with_ellipsis,
};

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Renders the WhatsApp sync notification message.
pub fn render_sync_message(input: &SyncMessageInput) -> String {
    let mut out = String::new();
    let local = input.sync_time.naive_local();

    if input.new_transactions.is_empty() {
        render_empty_message(&mut out, &local, &input.version, &input.hostname);
        return out.trim_end().to_string();
    }

    let real = compute_real_expenses(&input.new_transactions, &input.accounts);
    let count = input.new_transactions.len();

    // Header
    let _ = writeln!(
        out,
        "💰 *{count} nova{plural} transaç{plural_ao}* · {weekday} {:02}/{month} {:02}:{:02}",
        local.date().day(),
        local.time().hour(),
        local.time().minute(),
        weekday = weekday_pt(local.date().weekday().num_days_from_monday()),
        month = month_pt_short(local.date().month()),
        plural = if count == 1 { "" } else { "s" },
        plural_ao = if count == 1 { "ão" } else { "ões" },
    );
    let _ = writeln!(out);

    render_financial_snapshot(&mut out, input.balance, &real, input.review_items.len());
    render_top_categories(&mut out, &real);
    render_card_payment_note(&mut out, &real);
    render_date_groups(&mut out, &input.new_transactions, &real);
    render_review_block(&mut out, &input.review_items);
    render_footer(&mut out, &input.version, &input.hostname);

    out.trim_end().to_string()
}

fn render_empty_message(
    out: &mut String,
    local: &chrono::NaiveDateTime,
    version: &str,
    hostname: &str,
) {
    let _ = writeln!(
        out,
        "💰 *Sync · {:02}/{} {:02}:{:02}*",
        local.date().day(),
        month_pt_short(local.date().month()),
        local.time().hour(),
        local.time().minute(),
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "_sem novidades_");
    let _ = writeln!(out);
    let _ = writeln!(out, "_{version} · {hostname} · Pluggy sincronizado_",);
}

fn render_financial_snapshot(
    out: &mut String,
    balance: Option<Decimal>,
    real: &RealExpenseCalc,
    review_count: usize,
) {
    if let Some(balance) = balance {
        let _ = writeln!(out, "🏦 Saldo em conta: *{}*", brl(balance));
    }
    if real.card_payment_total > Decimal::ZERO && real.card_payment_compensated {
        let _ = writeln!(
            out,
            "💳 Pagamento de fatura: *{} compensado*",
            brl(real.card_payment_total)
        );
    }
    let _ = writeln!(
        out,
        "🧾 Despesa real nova: *{}*",
        brl(real.real_expense_total)
    );
    if review_count > 0 {
        let _ = writeln!(
            out,
            "⚠️ Para revisar: *{review_count} transaç{plural}*",
            plural = if review_count == 1 { "ão" } else { "ões" }
        );
    }
    let _ = writeln!(out);
}

fn render_top_categories(out: &mut String, real: &RealExpenseCalc) {
    if real.categories.is_empty() {
        return;
    }
    let _ = writeln!(out, "*Top categorias*");
    let mut sorted: Vec<(&String, &Decimal)> = real.categories.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    const MAX_CATS: usize = 5;
    let show = sorted.iter().take(MAX_CATS);
    let mut shown_total = Decimal::ZERO;
    for (family, total) in show {
        let pct = real.category_pct.get(*family).copied().unwrap_or(0);
        let emoji = category_emoji(Some(family), Some(**total));
        let label = family_label(family);
        let _ = writeln!(
            out,
            "{emoji} {label}: *{total_brl}* · *{pct}%*",
            total_brl = brl(**total)
        );
        shown_total += **total;
    }
    if sorted.len() > MAX_CATS {
        let other_total = real.real_expense_total - shown_total;
        if other_total > Decimal::ZERO {
            let other_pct = real
                .category_pct
                .get("_other")
                .copied()
                .unwrap_or_else(|| pct_of(other_total, real.real_expense_total));
            let _ = writeln!(
                out,
                "• Outras: *{other_brl}* · *{other_pct}%*",
                other_brl = brl(other_total)
            );
        }
    }
    let _ = writeln!(out);
}

fn render_card_payment_note(out: &mut String, real: &RealExpenseCalc) {
    if real.card_payment_total > Decimal::ZERO && real.card_payment_compensated {
        let _ = writeln!(
            out,
            "💳 O pagamento de fatura foi tratado como *movimentação interna* e não entrou como despesa real."
        );
        let _ = writeln!(out);
    }
}

fn render_date_groups(
    out: &mut String,
    transactions: &[SyncSummaryTransaction],
    real: &RealExpenseCalc,
) {
    let date_groups = group_transactions(transactions, real);
    const MAX_DAYS: usize = 2;
    let show_days = date_groups.iter().take(MAX_DAYS);
    for dg in show_days {
        let _ = writeln!(out, "*{}*", short_br_date(dg.date));
        let _ = writeln!(out);
        for cg in &dg.categories {
            let _ = writeln!(out, "{} *{}*", cg.emoji, family_label(&cg.category_family));
            for tx in &cg.transactions {
                let _ = writeln!(out, "• {} · *{}*", tx.description, brl(tx.amount));
                if tx.is_card_payment {
                    let _ = writeln!(out, "  Status: compensado, não conta como despesa real");
                }
            }
            let _ = writeln!(out);
        }
    }
    if date_groups.len() > MAX_DAYS {
        let remaining = date_groups.len() - MAX_DAYS;
        let _ = writeln!(
            out,
            "_… e mais {remaining} dia{plural} com novas transações_",
            plural = if remaining == 1 { "" } else { "s" }
        );
        let _ = writeln!(out);
    }
}

fn render_review_block(out: &mut String, review_items: &[ReviewItem]) {
    if review_items.is_empty() {
        return;
    }
    let _ = writeln!(out, "*Preciso da sua ajuda*");
    let _ = writeln!(out);

    const MAX_REVIEW: usize = 5;
    let show = review_items.iter().take(MAX_REVIEW);
    for item in show {
        let _ = writeln!(out, "*{}) {}*", item.index, item.description);
        let _ = writeln!(out, "Valor: *{}*", brl(item.amount));

        if let Some(ref cats) = item.suggested_category {
            let sub = item
                .suggested_subcategory
                .as_deref()
                .map(|s| format!(" > {s}"))
                .unwrap_or_default();
            let _ = writeln!(out, "Sugestão: *{cats}{sub}*");
        }
        if let Some(ref name) = item.suggested_name {
            let _ = writeln!(out, "Nome sugerido: *{name}*");
        }
        if let Some(ref rec) = item.is_recurring {
            let _ = writeln!(out, "Recorrente: {rec}");
        }
        let _ = writeln!(out);
    }

    if review_items.len() > MAX_REVIEW {
        let remaining = review_items.len() - MAX_REVIEW;
        let _ = writeln!(
            out,
            "_… e mais {remaining} transaç{plural} para revisar_",
            plural = if remaining == 1 { "ão" } else { "ões" }
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "*Responda*");
    let _ = writeln!(out, "`1 ok` para aceitar a sugestão");
    let _ = writeln!(out, "`2 mercado` para definir categoria");
    let _ = writeln!(out, "`3 mercado recorrente` para marcar como recorrente");
    let _ = writeln!(out, "`2 nome: Empório Pantanal` para corrigir o nome");
    let _ = writeln!(out, "`todos ok` para aceitar tudo");
    let _ = writeln!(out);
}

fn render_footer(out: &mut String, version: &str, hostname: &str) {
    let _ = writeln!(out, "_{version} · {hostname} · Pluggy sincronizado_",);
}

/// Renders a WhatsApp-friendly release notes message.
/// Only call this after a successful self-update.
pub fn render_release_notes(
    prev_version: &str,
    new_version: &str,
    changelog_body: &str,
    hostname: &str,
) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "🛠️ *phai atualizado*");
    let _ = writeln!(out);
    let _ = writeln!(out, "Versão: *{prev_version} → {new_version}*");
    let _ = writeln!(out);

    // Parse sections from release-please CHANGELOG markdown
    let sections = parse_changelog_sections(changelog_body);

    for (label, items) in &sections {
        let _ = writeln!(out, "*{label}*");
        for item in items {
            let _ = writeln!(out, "• {item}");
        }
    }

    let local = Utc::now().naive_local();
    let date = local.date();
    let time = local.time();
    let month = month_pt_short(date.month());
    let _ = writeln!(out);
    let _ = writeln!(out, "Ambiente: {hostname}");
    let _ = writeln!(
        out,
        "Atualizado em: {:02}/{month} às {:02}:{:02}",
        date.day(),
        time.hour(),
        time.minute(),
    );

    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Response parser
// ---------------------------------------------------------------------------

/// The parsed result of a user's WhatsApp text response.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedCommand {
    /// Accept suggestion for a single item: `1 ok`
    Accept { index: u32 },
    /// Accept all suggestions: `todos ok`
    AcceptAll,
    /// Set category via alias: `2 mercado`
    SetCategory { index: u32, alias: String },
    /// Set category + mark recurring: `3 mercado recorrente`
    SetCategoryRecurring { index: u32, alias: String },
    /// Set normalized name: `2 nome: Empório Pantanal`
    SetName { index: u32, name: String },
    /// Structured fields: `1 categoria: Casa | subcategoria: Limpeza | recorrente: não`
    Structured {
        index: u32,
        fields: StructuredFields,
    },
    /// Mark as recurring: `3 recorrente`
    SetRecurring { index: u32 },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StructuredFields {
    pub nome: Option<String>,
    pub categoria: Option<String>,
    pub subcategoria: Option<String>,
    pub recorrente: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parse a user's WhatsApp text response into a `ParsedCommand`.
pub fn parse_user_response(input: &str) -> Result<ParsedCommand, ParseError> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err(ParseError {
            message: "resposta vazia".into(),
        });
    }

    // `todos ok`
    if trimmed.eq_ignore_ascii_case("todos ok") {
        return Ok(ParsedCommand::AcceptAll);
    }

    // Try: `<index> ok`
    if let Some(cmd) = try_parse_accept(trimmed) {
        return Ok(cmd);
    }

    // Try: `<index> nome: <name>` (before structured so `nome:` isn't
    // misinterpreted as a structured field)
    if let Some(cmd) = try_parse_set_name(trimmed) {
        return Ok(cmd);
    }

    // Try structured format: `1 categoria: Casa | subcategoria: Limpeza | recorrente: não`
    if let Some(cmd) = try_parse_structured(trimmed) {
        return Ok(cmd);
    }

    // Try: `<index> <alias> recorrente`
    if let Some(cmd) = try_parse_category_recurring(trimmed) {
        return Ok(cmd);
    }

    // Try: `<index> recorrente`
    if let Some(cmd) = try_parse_recurring_only(trimmed) {
        return Ok(cmd);
    }

    // Try: `<index> <alias>`
    if let Some(cmd) = try_parse_category_alias(trimmed) {
        return Ok(cmd);
    }

    Err(ParseError {
        message: format!("não foi possível interpretar: '{trimmed}'"),
    })
}

fn try_parse_accept(input: &str) -> Option<ParsedCommand> {
    let (num_str, rest) = split_first_token(input)?;
    let rest = rest.trim();
    if rest.eq_ignore_ascii_case("ok") {
        let index = num_str.parse::<u32>().ok()?;
        Some(ParsedCommand::Accept { index })
    } else {
        None
    }
}

fn try_parse_set_name(input: &str) -> Option<ParsedCommand> {
    let (num_str, rest) = split_first_token(input)?;
    let rest = rest.trim();
    if let Some(name) = rest
        .strip_prefix("nome:")
        .or_else(|| rest.strip_prefix("nome :"))
    {
        let name = name.trim();
        if !name.is_empty() {
            let index = num_str.parse::<u32>().ok()?;
            return Some(ParsedCommand::SetName {
                index,
                name: name.to_string(),
            });
        }
    }
    None
}

fn try_parse_category_recurring(input: &str) -> Option<ParsedCommand> {
    let (num_str, rest) = split_first_token(input)?;
    let rest = rest.trim().to_lowercase();
    if let Some(alias) = rest.strip_suffix(" recorrente") {
        let alias = alias.trim();
        if !alias.is_empty() {
            let index = num_str.parse::<u32>().ok()?;
            return Some(ParsedCommand::SetCategoryRecurring {
                index,
                alias: alias.to_string(),
            });
        }
    }
    None
}

fn try_parse_recurring_only(input: &str) -> Option<ParsedCommand> {
    let (num_str, rest) = split_first_token(input)?;
    if rest.trim().eq_ignore_ascii_case("recorrente") {
        let index = num_str.parse::<u32>().ok()?;
        Some(ParsedCommand::SetRecurring { index })
    } else {
        None
    }
}

fn try_parse_category_alias(input: &str) -> Option<ParsedCommand> {
    let (num_str, rest) = split_first_token(input)?;
    let alias = rest.trim();
    if alias.is_empty()
        || alias.eq_ignore_ascii_case("ok")
        || alias.eq_ignore_ascii_case("recorrente")
        || alias.starts_with("nome:")
        || alias.starts_with("nome :")
    {
        return None;
    }
    let index = num_str.parse::<u32>().ok()?;
    Some(ParsedCommand::SetCategory {
        index,
        alias: alias.to_string(),
    })
}

fn try_parse_structured(input: &str) -> Option<ParsedCommand> {
    let (num_str, rest) = split_first_token(input)?;
    let rest = rest.trim();

    // Must contain at least one `:` to be structured
    if !rest.contains(':') {
        return None;
    }

    let index = num_str.parse::<u32>().ok()?;
    let mut fields = StructuredFields::default();

    for part in rest.split('|') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once(':') {
            let key = key.trim().to_lowercase();
            let value = value.trim();
            match key.as_str() {
                "nome" => fields.nome = Some(value.to_string()),
                "categoria" => fields.categoria = Some(value.to_string()),
                "subcategoria" => fields.subcategoria = Some(value.to_string()),
                "recorrente" => {
                    fields.recorrente = Some(parse_bool(value));
                }
                _ => {}
            }
        }
    }

    if fields.nome.is_none()
        && fields.categoria.is_none()
        && fields.subcategoria.is_none()
        && fields.recorrente.is_none()
    {
        return None;
    }

    Some(ParsedCommand::Structured { index, fields })
}

fn parse_bool(value: &str) -> bool {
    match value.trim().to_lowercase().as_str() {
        "sim" | "s" | "yes" | "y" | "true" | "1" => true,
        "não" | "nao" | "n" | "no" | "false" | "0" => false,
        _ => false,
    }
}

/// Split "1 ok" → ("1", "ok"), " 2   mercado " → ("2", "mercado").
fn split_first_token(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    let first_space = trimmed.find(char::is_whitespace)?;
    let num = &trimmed[..first_space];
    let rest = trimmed[first_space..].trim();
    if num.is_empty() {
        return None;
    }
    Some((num, rest))
}

// ---------------------------------------------------------------------------
// Category aliases — maps user shorthand to (category, subcategory)
// ---------------------------------------------------------------------------

/// Resolve a user-facing category alias to `(category, subcategory)`.
/// These are generic Brazilian-Portuguese terms — no personal patterns.
pub fn resolve_category_alias(alias: &str) -> Option<(&'static str, &'static str)> {
    match alias.to_lowercase().trim() {
        // Alimentação
        "mercado" | "supermercado" | "feira" => Some(("Alimentação", "Mercado")),
        "restaurante" | "bar" | "lanchonete" | "padaria" => Some(("Alimentação", "Restaurante")),
        "delivery" | "ifood" => Some(("Alimentação", "Delivery")),
        "acougue" | "açougue" => Some(("Alimentação", "Açougue")),

        // Saúde
        "farmacia" | "farmácia" | "drogaria" => Some(("Saúde", "Farmácia")),
        "medico" | "médico" | "consulta" => Some(("Saúde", "Consulta")),
        "academia" | "fitness" => Some(("Saúde", "Fitness")),
        "exame" | "laboratorio" | "laboratório" => Some(("Saúde", "Exames")),

        // Transporte
        "combustivel" | "combustível" | "gasolina" | "posto" => {
            Some(("Transporte", "Combustível"))
        }
        "uber" | "99" | "cabify" => Some(("Transporte", "Aplicativo")),
        "estacionamento" => Some(("Transporte", "Estacionamento")),
        "onibus" | "ônibus" | "metro" | "metrô" | "trem" => Some(("Transporte", "Público")),

        // Casa
        "casa" => Some(("Casa", "")),
        "limpeza" => Some(("Casa", "Limpeza")),
        "aluguel" => Some(("Moradia", "Aluguel")),
        "condominio" | "condomínio" => Some(("Moradia", "Condomínio")),
        "conta" | "contas" | "agua" | "água" | "luz" | "gas" | "gás" | "internet" => {
            Some(("Moradia", "Contas"))
        }

        // Educação
        "educacao" | "educação" | "escola" | "faculdade" | "curso" => Some(("Educação", "")),
        "livro" | "livraria" => Some(("Educação", "Livros")),

        // Assinaturas
        "assinaturas" | "assinatura" => Some(("Assinaturas", "")),
        "streaming" | "netflix" | "spotify" => Some(("Assinaturas", "Streaming")),

        // Lazer
        "lazer" => Some(("Lazer", "")),
        "cinema" | "teatro" | "show" => Some(("Lazer", "Entretenimento")),
        "viagem" | "hotel" | "passagem" => Some(("Lazer", "Viagem")),

        // Compras
        "compras" => Some(("Compras", "")),
        "roupa" | "vestuario" | "vestuário" | "calcado" | "calçado" => {
            Some(("Compras", "Vestuário"))
        }
        "eletronico" | "eletrônico" | "eletrodomestico" | "eletrodoméstico" => {
            Some(("Compras", "Eletrônicos"))
        }

        // Serviços
        "servicos" | "serviços" => Some(("Serviços", "")),
        "manutencao" | "manutenção" | "reparo" | "conserto" => Some(("Serviços", "Manutenção")),

        // Outros
        "pessoal" | "cabeleireiro" | "barbearia" | "estetica" | "estética" => {
            Some(("Pessoal", ""))
        }
        "pet" | "veterinario" | "veterinário" => Some(("Lazer", "Pet")),
        "investimentos" | "investimento" => Some(("Investimentos", "")),
        "saude" | "saúde" => Some(("Saúde", "")),
        "transporte" => Some(("Transporte", "")),
        "moradia" => Some(("Moradia", "")),

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Real expense calculation
// ---------------------------------------------------------------------------

struct RealExpenseCalc {
    real_expense_total: Decimal,
    card_payment_total: Decimal,
    card_payment_compensated: bool,
    /// category family → total (real expenses only, positive amounts)
    categories: BTreeMap<String, Decimal>,
    /// category family → integer percentage
    category_pct: BTreeMap<String, u32>,
    /// transaction descriptions flagged as card payments
    card_payment_ids: BTreeSet<String>,
}

fn compute_real_expenses(
    transactions: &[SyncSummaryTransaction],
    accounts: &[phai_core::models::AccountRecord],
) -> RealExpenseCalc {
    let card_payment_pairs = detect_card_payment_pairs(transactions, accounts);

    // Collect ids involved in card payment pairs
    let mut card_payment_ids: BTreeSet<String> = BTreeSet::new();
    for (tx_id, _match_id, _amt) in &card_payment_pairs {
        card_payment_ids.insert(tx_id.clone());
    }

    let mut real_total = Decimal::ZERO;
    let mut card_total = Decimal::ZERO;
    let mut categories: BTreeMap<String, Decimal> = BTreeMap::new();

    for tx in transactions {
        let amount = match decimal_from_str(&tx.amount) {
            Ok(a) => a,
            Err(_) => continue,
        };

        // Only debit (negative) transactions count as expenses
        if amount >= Decimal::ZERO {
            continue;
        }
        let abs_amount = amount.abs();

        // Skip card payment pairs
        if card_payment_ids.contains(&tx.transaction_id) {
            card_total += abs_amount;
            continue;
        }

        // Skip internal transfers
        if is_internal_transfer(tx) {
            continue;
        }

        // Skip cashback / phantom
        if is_cashback_or_phantom(tx) {
            continue;
        }

        real_total += abs_amount;

        // Categorize
        let family =
            category_family(tx.category_id.as_deref()).unwrap_or_else(|| "outros".to_string());
        *categories.entry(family).or_insert(Decimal::ZERO) += abs_amount;
    }

    // Compute percentages
    let mut category_pct = BTreeMap::new();
    if real_total > Decimal::ZERO {
        for (family, total) in &categories {
            let pct = pct_of(*total, real_total);
            category_pct.insert(family.clone(), pct);
        }
    }

    RealExpenseCalc {
        real_expense_total: real_total,
        card_payment_total: card_total,
        card_payment_compensated: !card_payment_pairs.is_empty() && card_total > Decimal::ZERO,
        categories,
        category_pct,
        card_payment_ids,
    }
}

/// Detect matching debit+credit pairs that represent internal card payments.
/// A pair is: debit from checking account, matching credit to credit card,
/// same amount (1% tolerance), on same or adjacent dates.
fn detect_card_payment_pairs(
    transactions: &[SyncSummaryTransaction],
    accounts: &[phai_core::models::AccountRecord],
) -> Vec<(String, String, Decimal)> {
    let checking_ids: BTreeSet<&str> = accounts
        .iter()
        .filter(|a| phai_core::models::is_checking_account_type(&a.account_type))
        .map(|a| a.account_id.as_str())
        .collect();
    let credit_ids: BTreeSet<&str> = accounts
        .iter()
        .filter(|a| a.account_type == "credit")
        .map(|a| a.account_id.as_str())
        .collect();

    let debits: Vec<&SyncSummaryTransaction> = transactions
        .iter()
        .filter(|tx| {
            tx.account_id
                .as_deref()
                .is_some_and(|aid| checking_ids.contains(aid))
                && tx.amount.starts_with('-')
        })
        .collect();
    let credits: Vec<&SyncSummaryTransaction> = transactions
        .iter()
        .filter(|tx| {
            tx.account_id
                .as_deref()
                .is_some_and(|aid| credit_ids.contains(aid))
                && !tx.amount.starts_with('-')
        })
        .collect();

    let mut pairs = Vec::new();
    for debit in &debits {
        let debit_amt = match decimal_from_str(&debit.amount) {
            Ok(a) => a.abs(),
            Err(_) => continue,
        };
        let debit_date = match NaiveDate::parse_from_str(&debit.transaction_date, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => continue,
        };

        for credit in &credits {
            let credit_amt = match decimal_from_str(&credit.amount) {
                Ok(a) => a,
                Err(_) => continue,
            };
            let credit_date = match NaiveDate::parse_from_str(&credit.transaction_date, "%Y-%m-%d")
            {
                Ok(d) => d,
                Err(_) => continue,
            };

            // Same or adjacent date, matching amounts within 1%
            let date_diff = (debit_date - credit_date).num_days().abs();
            if date_diff <= 1 && amounts_match(debit_amt, credit_amt) {
                pairs.push((
                    debit.transaction_id.clone(),
                    credit.transaction_id.clone(),
                    debit_amt,
                ));
            }
        }
    }
    pairs
}

fn amounts_match(a: Decimal, b: Decimal) -> bool {
    if a == b {
        return true;
    }
    let diff = (a - b).abs();
    let max = a.max(b);
    if max == Decimal::ZERO {
        return true;
    }
    // 1% tolerance
    diff / max <= Decimal::new(1, 2)
}

fn is_internal_transfer(tx: &SyncSummaryTransaction) -> bool {
    let t = tx.tx_type.to_lowercase();
    t.contains("transfer") || t.contains("transferência") || t.contains("transferencia")
}

fn is_cashback_or_phantom(tx: &SyncSummaryTransaction) -> bool {
    let desc = tx.description.to_lowercase();
    desc.contains("cashback")
        || desc.contains("estorno")
        || desc.contains("chargeback")
        || desc.contains("phantom")
}

fn decimal_from_str(s: &str) -> Result<Decimal, rust_decimal::Error> {
    s.parse::<Decimal>()
}

// ---------------------------------------------------------------------------
// Transaction grouping
// ---------------------------------------------------------------------------

struct DateGroup {
    date: NaiveDate,
    categories: Vec<CategoryGroup>,
}

struct CategoryGroup {
    category_family: String,
    emoji: &'static str,
    transactions: Vec<GroupedTx>,
}

struct GroupedTx {
    description: String,
    amount: Decimal,
    is_card_payment: bool,
}

fn group_transactions(
    transactions: &[SyncSummaryTransaction],
    real: &RealExpenseCalc,
) -> Vec<DateGroup> {
    // Group by date
    let mut by_date: BTreeMap<NaiveDate, Vec<&SyncSummaryTransaction>> = BTreeMap::new();
    for tx in transactions {
        if let Ok(date) = NaiveDate::parse_from_str(&tx.transaction_date, "%Y-%m-%d") {
            by_date.entry(date).or_default().push(tx);
        }
    }

    let mut date_groups = Vec::new();
    for (date, txs) in &by_date {
        let mut cat_groups: BTreeMap<String, CategoryGroup> = BTreeMap::new();

        for tx in txs {
            let amount = decimal_from_str(&tx.amount).ok().map(|a| a.abs());
            let amount = match amount {
                Some(a) if a > Decimal::ZERO => a,
                _ => continue,
            };
            let is_card_payment = real.card_payment_ids.contains(&tx.transaction_id);

            let family = category_family(tx.category_id.as_deref()).unwrap_or_else(|| {
                if is_card_payment {
                    "financeiro".to_string()
                } else {
                    "outros".to_string()
                }
            });

            // Map card payments to a "Cartão" group
            let display_family = if is_card_payment {
                "cartao".to_string()
            } else {
                family.clone()
            };

            let entry = cat_groups
                .entry(display_family.clone())
                .or_insert_with(|| CategoryGroup {
                    emoji: if is_card_payment {
                        "💳"
                    } else {
                        category_emoji(Some(&family), Some(Decimal::ZERO - amount))
                    },
                    category_family: display_family,
                    transactions: Vec::new(),
                });

            let label = tx
                .context
                .as_deref()
                .map(|c| c.to_string())
                .unwrap_or_else(|| short_description(&tx.description));
            let label = truncate_with_ellipsis(&label, 35);

            entry.transactions.push(GroupedTx {
                description: label,
                amount,
                is_card_payment,
            });
        }

        if !cat_groups.is_empty() {
            // Sort: card payments last
            let mut cats: Vec<CategoryGroup> = cat_groups.into_values().collect();
            cats.sort_by(|a, b| {
                let a_card = a
                    .transactions
                    .first()
                    .map(|t| t.is_card_payment)
                    .unwrap_or(false);
                let b_card = b
                    .transactions
                    .first()
                    .map(|t| t.is_card_payment)
                    .unwrap_or(false);
                match (a_card, b_card) {
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Equal,
                }
            });
            date_groups.push(DateGroup {
                date: *date,
                categories: cats,
            });
        }
    }

    // BTreeMap iterates ascending → oldest first. Reverse so most recent
    // transactions appear first in the message.
    date_groups.reverse();
    date_groups
}

// ---------------------------------------------------------------------------
// Review item selection
// ---------------------------------------------------------------------------

/// A transaction that needs the user's attention.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ReviewItem {
    pub index: u32,
    pub transaction_id: String,
    pub description: String,
    pub amount: Decimal,
    pub date: NaiveDate,
    pub suggested_category: Option<String>,
    pub suggested_subcategory: Option<String>,
    pub suggested_name: Option<String>,
    pub is_recurring: Option<String>,
}

/// Build the review items list from new transactions.
pub fn build_review_items(transactions: &[SyncSummaryTransaction]) -> Vec<ReviewItem> {
    let merchant_prefixes = [
        "Mp *",
        "Mp*",
        "Pag*",
        "Pag *",
        "Mercadopago",
        "MercadoPago",
        "Mercado Pago",
        "Stone",
        "Cielo",
        "Rede",
        "PicPay",
        "Pic Pay",
        "SumUp",
        "Sum Up",
        "Getnet",
        "Ton",
        "TON",
    ];

    let mut items = Vec::new();
    let mut index = 0u32;

    for tx in transactions {
        let amount = match decimal_from_str(&tx.amount) {
            Ok(a) if a < Decimal::ZERO => a.abs(),
            _ => continue, // only debits
        };

        let mut needs_review = false;

        // Condition 1: missing or weak category
        if tx.category_id.is_none()
            || matches!(tx.category_source.as_str(), "unclassified" | "fallback")
        {
            needs_review = true;
        }

        // Condition 2: obscure merchant description
        let desc_upper = tx.description.to_uppercase();
        let has_merchant_prefix = merchant_prefixes
            .iter()
            .any(|p| desc_upper.starts_with(&p.to_uppercase()));
        if has_merchant_prefix {
            needs_review = true;
        }

        // Condition 3: truncated description (abrupt end < 30 chars, no natural ending)
        if is_truncated_description(&tx.description) {
            needs_review = true;
        }

        if !needs_review {
            continue;
        }

        index += 1;
        let date = NaiveDate::parse_from_str(&tx.transaction_date, "%Y-%m-%d").ok();

        // Build suggestion
        let (suggested_cat, suggested_sub) = suggest_category(tx);
        let suggested_name = suggest_name(&tx.description);

        items.push(ReviewItem {
            index,
            transaction_id: tx.transaction_id.clone(),
            description: short_description(&tx.description),
            amount,
            date: date.unwrap_or_else(|| Utc::now().date_naive()),
            suggested_category: suggested_cat,
            suggested_subcategory: suggested_sub,
            suggested_name,
            is_recurring: None, // requires historical data; skip for now
        });
    }

    items
}

fn is_truncated_description(desc: &str) -> bool {
    let desc = desc.trim();
    let len = desc.chars().count();
    // Truncated if it ends mid-word around common truncation points
    if !(25..=40).contains(&len) {
        return false;
    }
    // Check if it ends without a natural break (space, punctuation)
    if let Some(last_char) = desc.chars().last() {
        if last_char.is_whitespace() || last_char == '.' || last_char == ')' {
            return false;
        }
    }
    // Check for common truncation patterns
    let last_few: String = desc.chars().rev().take(4).collect::<String>();
    let last_few: String = last_few.chars().rev().collect();
    !last_few.contains(' ')
}

/// Suggest a category based on what's already known about the transaction.
///
/// Does NOT embed merchant-specific keywords — user patterns live in the
/// `rules` table, not in shared code (AGENTS.md privacy rule).
fn suggest_category(tx: &SyncSummaryTransaction) -> (Option<String>, Option<String>) {
    // If transaction already has a category from rules or Pluggy, surface it.
    if let Some(ref cat_id) = tx.category_id {
        let family = category_family(Some(cat_id));
        let sub = cat_id.split(':').nth(1).map(|s| s.to_string());
        return (family, sub);
    }

    // Generic keyword hints from the description — only category-level signals,
    // never specific merchant names.
    let desc = tx.description.to_lowercase();
    if desc.contains("farmacia") || desc.contains("farmácia") || desc.contains("drogaria") {
        (Some("Saúde".to_string()), Some("Farmácia".to_string()))
    } else if desc.contains("restaurante")
        || desc.contains("lanchonete")
        || desc.contains("padaria")
        || desc.contains("pizzaria")
    {
        (
            Some("Alimentação".to_string()),
            Some("Restaurante".to_string()),
        )
    } else if desc.contains("mercado") || desc.contains("supermercado") {
        (Some("Alimentação".to_string()), Some("Mercado".to_string()))
    } else if desc.contains("combustivel") || desc.contains("combustível") || desc.contains("posto")
    {
        (
            Some("Transporte".to_string()),
            Some("Combustível".to_string()),
        )
    } else if desc.contains("uber") || desc.contains("99 ") || desc.contains("cabify") {
        (
            Some("Transporte".to_string()),
            Some("Aplicativo".to_string()),
        )
    } else if desc.contains("academia") || desc.contains("gym") {
        (Some("Saúde".to_string()), Some("Fitness".to_string()))
    } else if desc.contains("streaming")
        || desc.contains("netflix")
        || desc.contains("spotify")
        || desc.contains("prime video")
    {
        (
            Some("Assinaturas".to_string()),
            Some("Streaming".to_string()),
        )
    } else {
        (None, None)
    }
}

/// Attempt to clean up a merchant name.
fn suggest_name(description: &str) -> Option<String> {
    let cleaned = description.trim();

    // Strip known acquirer/gateway prefixes (Brazilian market).
    // These are generic infrastructure identifiers, not personal data.
    let prefixes = [
        "Mp *",
        "Mp*",
        "MP *",
        "MP*",
        "Mercadopago",
        "MercadoPago",
        "Mercado Pago",
        "Pag*",
        "Pag *",
        "PagSeguro",
        "Pag Seguro",
        "Pagamento",
        "Pagamento de",
        "Stone",
        "Stone *",
        "Cielo",
        "Cielo *",
        "Rede",
        "Rede *",
        "PicPay",
        "Pic Pay",
        "SumUp",
        "Sum Up",
        "Getnet",
        "GetNet",
        "Ton",
        "TON",
        "Ton *",
        "Pix *",
        "PIX *",
        "Ifood *",
        "iFood *",
        "Uber *",
        "Uber",
        "99 *",
        "99Pop",
        "Rappi",
        "Rappi *",
    ];
    let mut name = cleaned.to_string();
    for prefix in &prefixes {
        if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
            name = name[prefix.len()..].trim().to_string();
            break;
        }
    }

    // Skip if nothing changed
    if name.eq_ignore_ascii_case(cleaned) {
        return None;
    }
    if name.is_empty() {
        return None;
    }

    // Title-case
    let title = name
        .split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    if title.eq_ignore_ascii_case(cleaned) {
        None
    } else {
        Some(title)
    }
}

// ---------------------------------------------------------------------------
// Sync message input
// ---------------------------------------------------------------------------

/// All data needed to render the sync notification message.
#[allow(dead_code)]
pub struct SyncMessageInput {
    pub new_transactions: Vec<SyncSummaryTransaction>,
    pub accounts: Vec<phai_core::models::AccountRecord>,
    pub snapshots: Vec<phai_core::models::AccountSnapshotRecord>,
    pub review_items: Vec<ReviewItem>,
    pub balance: Option<Decimal>,
    pub sync_time: chrono::DateTime<Utc>,
    pub version: String,
    pub hostname: String,
}

/// Re-export the sync summary transaction type so callers can use it directly.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SyncSummaryTransaction {
    pub transaction_id: String,
    pub transaction_date: String,
    pub description: String,
    pub amount: String,
    pub tx_type: String,
    pub category_id: Option<String>,
    pub category_source: String,
    pub context: Option<String>,
    pub account_id: Option<String>,
    pub account_label: Option<String>,
    pub payment_status: String,
    pub source: String,
    pub metadata_json: serde_json::Value,
}

// ---------------------------------------------------------------------------
// CHANGELOG section parser (for release notes)
// ---------------------------------------------------------------------------

fn parse_changelog_sections(body: &str) -> Vec<(String, Vec<String>)> {
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut current_label = String::new();
    let mut current_items: Vec<String> = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Section headers: ### Features, ### Bug Fixes, etc.
        if trimmed.starts_with("### ") {
            // Flush previous section
            if !current_label.is_empty() && !current_items.is_empty() {
                sections.push((
                    std::mem::take(&mut current_label),
                    std::mem::take(&mut current_items),
                ));
            }
            let header = trimmed.trim_start_matches("### ").trim();
            current_label = map_section_label(header);
        } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let item = trimmed[2..].trim();
            if !item.is_empty() {
                current_items.push(item.to_string());
            }
        }
    }

    // Flush last section
    if !current_label.is_empty() && !current_items.is_empty() {
        sections.push((current_label, current_items));
    }

    sections
}

fn map_section_label(header: &str) -> String {
    match header.to_lowercase().as_str() {
        "features" | "feature" => "Novidade".to_string(),
        "bug fixes" | "fixes" | "fix" => "Correções".to_string(),
        "performance" | "improvements" => "Melhorias".to_string(),
        "infrastructure" | "dependencies" | "chore" => "Infraestrutura".to_string(),
        other => {
            // Capitalize first letter
            let mut chars = other.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

fn weekday_pt(day_num: u32) -> &'static str {
    match day_num {
        0 => "seg",
        1 => "ter",
        2 => "qua",
        3 => "qui",
        4 => "sex",
        5 => "sáb",
        _ => "dom",
    }
}

fn month_pt_short(month: u32) -> &'static str {
    match month {
        1 => "jan",
        2 => "fev",
        3 => "mar",
        4 => "abr",
        5 => "mai",
        6 => "jun",
        7 => "jul",
        8 => "ago",
        9 => "set",
        10 => "out",
        11 => "nov",
        12 => "dez",
        _ => "??",
    }
}

fn short_br_date(date: NaiveDate) -> String {
    format!("{:02}/{}", date.day(), month_pt_short(date.month()))
}

fn pct_of(part: Decimal, total: Decimal) -> u32 {
    if total == Decimal::ZERO {
        return 0;
    }
    let pct = (part / total * Decimal::from(100)).round_dp(0);
    pct.try_into().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn make_tx(
        id: &str,
        date: &str,
        description: &str,
        amount: &str,
        tx_type: &str,
        category_id: Option<&str>,
        category_source: &str,
        account_id: Option<&str>,
        _account_type: Option<&str>,
    ) -> SyncSummaryTransaction {
        SyncSummaryTransaction {
            transaction_id: id.to_string(),
            transaction_date: date.to_string(),
            description: description.to_string(),
            amount: amount.to_string(),
            tx_type: tx_type.to_string(),
            category_id: category_id.map(|s| s.to_string()),
            category_source: category_source.to_string(),
            context: None,
            account_id: account_id.map(|s| s.to_string()),
            account_label: None,
            payment_status: "posted".to_string(),
            source: "pluggy".to_string(),
            metadata_json: serde_json::json!({}),
        }
    }

    fn make_account(id: &str, account_type: &str) -> phai_core::models::AccountRecord {
        phai_core::models::AccountRecord {
            account_id: id.to_string(),
            owner: "test".to_string(),
            account_type: account_type.to_string(),
            bank: "test".to_string(),
            label: "".to_string(),
            pluggy_account_id: None,
            pluggy_item_id: None,
            status: "active".to_string(),
            actor_id: "test".to_string(),
            idempotency_key: "test".to_string(),
            metadata_json: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Real expense calculation
    // -----------------------------------------------------------------------

    #[test]
    fn test_real_expense_excludes_card_payment() {
        let txs = vec![
            make_tx(
                "1",
                "2026-05-18",
                "Pagamento de fatura",
                "-6069.89",
                "payment",
                None,
                "unclassified",
                Some("checking1"),
                None,
            ),
            make_tx(
                "2",
                "2026-05-18",
                "Pagamento recebido",
                "+6069.89",
                "credit",
                None,
                "unclassified",
                Some("card1"),
                None,
            ),
            make_tx(
                "3",
                "2026-05-18",
                "Grupo Madero",
                "-77.00",
                "debit",
                Some("alimentacao:restaurante"),
                "rule",
                Some("checking1"),
                None,
            ),
        ];
        let accounts = vec![
            make_account("checking1", "checking"),
            make_account("card1", "credit"),
        ];
        let real = compute_real_expenses(&txs, &accounts);
        assert_eq!(real.real_expense_total, Decimal::new(7700, 2)); // 77.00
        assert_eq!(real.card_payment_total, Decimal::new(606989, 2)); // 6069.89
        assert!(real.card_payment_compensated);
    }

    #[test]
    fn test_real_expense_excludes_internal_transfer() {
        let txs = vec![
            make_tx(
                "1",
                "2026-05-18",
                "Transferência entre contas",
                "-500.00",
                "transfer",
                None,
                "unclassified",
                Some("checking1"),
                None,
            ),
            make_tx(
                "2",
                "2026-05-18",
                "Mercado",
                "-100.00",
                "debit",
                Some("alimentacao:mercado"),
                "rule",
                Some("checking1"),
                None,
            ),
        ];
        let accounts = vec![make_account("checking1", "checking")];
        let real = compute_real_expenses(&txs, &accounts);
        assert_eq!(real.real_expense_total, Decimal::new(10000, 2)); // 100.00
                                                                     // Transfer should not count
    }

    #[test]
    fn test_real_expense_excludes_cashback() {
        let txs = vec![
            make_tx(
                "1",
                "2026-05-18",
                "Cashback XP",
                "-50.00",
                "debit",
                Some("cashback"),
                "rule",
                Some("checking1"),
                None,
            ),
            make_tx(
                "2",
                "2026-05-18",
                "Mercado",
                "-100.00",
                "debit",
                Some("alimentacao:mercado"),
                "rule",
                Some("checking1"),
                None,
            ),
        ];
        let accounts = vec![make_account("checking1", "checking")];
        let real = compute_real_expenses(&txs, &accounts);
        // Cashback should not count as real expense
        assert_eq!(real.real_expense_total, Decimal::new(10000, 2)); // 100.00
    }

    #[test]
    fn test_real_expense_correct_total_six_transactions() {
        // The exact scenario from the spec
        let txs = vec![
            make_tx(
                "1",
                "2026-05-18",
                "Pagamento de fatura",
                "-6069.89",
                "payment",
                None,
                "unclassified",
                Some("checking1"),
                None,
            ),
            make_tx(
                "2",
                "2026-05-18",
                "Pagamento recebido",
                "+6069.89",
                "credit",
                None,
                "unclassified",
                Some("card1"),
                None,
            ),
            make_tx(
                "3",
                "2026-05-18",
                "Grupo Madero",
                "-77.00",
                "debit",
                Some("alimentacao:restaurante"),
                "rule",
                Some("checking1"),
                None,
            ),
            make_tx(
                "4",
                "2026-05-17",
                "Supermercados Imperatr",
                "-25.88",
                "debit",
                Some("alimentacao:mercado"),
                "rule",
                Some("checking1"),
                None,
            ),
            make_tx(
                "5",
                "2026-05-17",
                "Mp *Emporiopantan",
                "-53.00",
                "debit",
                None,
                "unclassified",
                Some("checking1"),
                None,
            ),
            make_tx(
                "6",
                "2026-05-17",
                "Gimpel",
                "-67.87",
                "debit",
                Some("alimentacao:mercado"),
                "rule",
                Some("checking1"),
                None,
            ),
        ];
        let accounts = vec![
            make_account("checking1", "checking"),
            make_account("card1", "credit"),
        ];
        let real = compute_real_expenses(&txs, &accounts);

        // Total: 77.00 + 25.88 + 53.00 + 67.87 = 223.75
        assert_eq!(real.real_expense_total, Decimal::new(22375, 2));
        assert_eq!(real.card_payment_total, Decimal::new(606989, 2));
        assert!(real.card_payment_compensated);

        // Categories: 3 tx have alimentacao category, 1 is uncategorized ("outros")
        let alimentacao = real.categories.get("alimentacao").unwrap();
        // 77.00 + 25.88 + 67.87 = 170.75
        assert_eq!(*alimentacao, Decimal::new(17075, 2));
        let outros = real.categories.get("outros").unwrap();
        // 53.00
        assert_eq!(*outros, Decimal::new(5300, 2));
    }

    #[test]
    fn test_category_percentages() {
        let txs = vec![
            make_tx(
                "1",
                "2026-05-18",
                "Grupo Madero",
                "-77.00",
                "debit",
                Some("alimentacao:restaurante"),
                "rule",
                Some("checking1"),
                None,
            ),
            make_tx(
                "2",
                "2026-05-17",
                "Mercado X",
                "-146.75",
                "debit",
                Some("alimentacao:mercado"),
                "rule",
                Some("checking1"),
                None,
            ),
        ];
        let accounts = vec![make_account("checking1", "checking")];
        let real = compute_real_expenses(&txs, &accounts);

        // Total: 223.75
        // Alimentacao: 223.75 = 100% (both are under alimentacao family)
        // Actually, let me check: category_family("alimentacao:restaurante") → "alimentacao"
        // category_family("alimentacao:mercado") → "alimentacao"
        // So both map to the same family, total = 223.75, 100%
        let pct = real.category_pct.get("alimentacao").copied().unwrap_or(0);
        assert_eq!(pct, 100);
    }

    // -----------------------------------------------------------------------
    // Card payment detection
    // -----------------------------------------------------------------------

    #[test]
    fn test_detects_card_payment_pair() {
        let txs = vec![
            make_tx(
                "1",
                "2026-05-18",
                "Pagamento de fatura",
                "-6069.89",
                "payment",
                None,
                "unclassified",
                Some("checking1"),
                None,
            ),
            make_tx(
                "2",
                "2026-05-18",
                "Pagamento recebido cartão",
                "+6069.89",
                "credit",
                None,
                "unclassified",
                Some("card1"),
                None,
            ),
        ];
        let accounts = vec![
            make_account("checking1", "checking"),
            make_account("card1", "credit"),
        ];
        let pairs = detect_card_payment_pairs(&txs, &accounts);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "1");
        assert_eq!(pairs[0].1, "2");
    }

    #[test]
    fn test_adjacent_date_card_payment() {
        let txs = vec![
            make_tx(
                "1",
                "2026-05-17",
                "Pagamento de fatura",
                "-6069.89",
                "payment",
                None,
                "unclassified",
                Some("checking1"),
                None,
            ),
            make_tx(
                "2",
                "2026-05-18",
                "Pagamento recebido cartão",
                "+6069.89",
                "credit",
                None,
                "unclassified",
                Some("card1"),
                None,
            ),
        ];
        let accounts = vec![
            make_account("checking1", "checking"),
            make_account("card1", "credit"),
        ];
        let pairs = detect_card_payment_pairs(&txs, &accounts);
        assert_eq!(pairs.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Review selection
    // -----------------------------------------------------------------------

    #[test]
    fn test_review_includes_uncategorized() {
        let txs = vec![make_tx(
            "1",
            "2026-05-18",
            "Loja Desconhecida",
            "-50.00",
            "debit",
            None,
            "unclassified",
            Some("checking1"),
            None,
        )];
        let items = build_review_items(&txs);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].index, 1);
    }

    #[test]
    fn test_review_includes_merchant_prefix() {
        let txs = vec![make_tx(
            "1",
            "2026-05-18",
            "Mp *Emporiopantan",
            "-53.00",
            "debit",
            Some("alimentacao:mercado"),
            "rule",
            Some("checking1"),
            None,
        )];
        let items = build_review_items(&txs);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_review_excludes_clean_transaction() {
        let txs = vec![make_tx(
            "1",
            "2026-05-18",
            "Grupo Madero",
            "-77.00",
            "debit",
            Some("alimentacao:restaurante"),
            "rule",
            Some("checking1"),
            None,
        )];
        let items = build_review_items(&txs);
        assert_eq!(items.len(), 0);
    }

    #[test]
    fn test_review_excludes_positive_amounts() {
        let txs = vec![make_tx(
            "1",
            "2026-05-18",
            "Salário",
            "+5000.00",
            "credit",
            None,
            "unclassified",
            Some("checking1"),
            None,
        )];
        let items = build_review_items(&txs);
        assert_eq!(items.len(), 0);
    }

    #[test]
    fn test_suggested_name_only_when_different() {
        // "Mp *Emporiopantan" → "Emporiopantan" should be suggested
        let name = suggest_name("Mp *Emporiopantan");
        assert_eq!(name.as_deref(), Some("Emporiopantan"));

        // "Grupo Madero" — no prefix to strip, should be None
        let name2 = suggest_name("Grupo Madero");
        assert_eq!(name2, None);
    }

    #[test]
    fn test_suggested_name_strips_multi_prefixes() {
        let name = suggest_name("Pag* Restaurante X");
        assert_eq!(name.as_deref(), Some("Restaurante X"));
    }

    // -----------------------------------------------------------------------
    // Response parser
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_accept_single() {
        let cmd = parse_user_response("1 ok").unwrap();
        assert_eq!(cmd, ParsedCommand::Accept { index: 1 });
    }

    #[test]
    fn test_parse_accept_all() {
        let cmd = parse_user_response("todos ok").unwrap();
        assert_eq!(cmd, ParsedCommand::AcceptAll);
    }

    #[test]
    fn test_parse_todos_ok_case_insensitive() {
        let cmd = parse_user_response("TODOS OK").unwrap();
        assert_eq!(cmd, ParsedCommand::AcceptAll);
    }

    #[test]
    fn test_parse_set_category() {
        let cmd = parse_user_response("2 mercado").unwrap();
        assert_eq!(
            cmd,
            ParsedCommand::SetCategory {
                index: 2,
                alias: "mercado".to_string()
            }
        );
    }

    #[test]
    fn test_parse_set_category_recurring() {
        let cmd = parse_user_response("3 mercado recorrente").unwrap();
        assert_eq!(
            cmd,
            ParsedCommand::SetCategoryRecurring {
                index: 3,
                alias: "mercado".to_string()
            }
        );
    }

    #[test]
    fn test_parse_set_name() {
        let cmd = parse_user_response("2 nome: Empório Pantanal").unwrap();
        assert_eq!(
            cmd,
            ParsedCommand::SetName {
                index: 2,
                name: "Empório Pantanal".to_string()
            }
        );
    }

    #[test]
    fn test_parse_set_name_with_space() {
        let cmd = parse_user_response("2 nome : Empório Pantanal").unwrap();
        assert_eq!(
            cmd,
            ParsedCommand::SetName {
                index: 2,
                name: "Empório Pantanal".to_string()
            }
        );
    }

    #[test]
    fn test_parse_structured() {
        let cmd =
            parse_user_response("1 categoria: Casa | subcategoria: Limpeza | recorrente: não")
                .unwrap();
        assert_eq!(
            cmd,
            ParsedCommand::Structured {
                index: 1,
                fields: StructuredFields {
                    categoria: Some("Casa".to_string()),
                    subcategoria: Some("Limpeza".to_string()),
                    recorrente: Some(false),
                    nome: None,
                }
            }
        );
    }

    #[test]
    fn test_parse_structured_different_order() {
        let cmd = parse_user_response(
            "1 recorrente: sim | nome: Netflix | categoria: Assinaturas | subcategoria: Streaming",
        )
        .unwrap();
        assert_eq!(
            cmd,
            ParsedCommand::Structured {
                index: 1,
                fields: StructuredFields {
                    nome: Some("Netflix".to_string()),
                    categoria: Some("Assinaturas".to_string()),
                    subcategoria: Some("Streaming".to_string()),
                    recorrente: Some(true),
                }
            }
        );
    }

    #[test]
    fn test_parse_recurring_only() {
        let cmd = parse_user_response("3 recorrente").unwrap();
        assert_eq!(cmd, ParsedCommand::SetRecurring { index: 3 });
    }

    #[test]
    fn test_parse_empty_returns_error() {
        assert!(parse_user_response("").is_err());
        assert!(parse_user_response("  ").is_err());
    }

    // -----------------------------------------------------------------------
    // Category alias resolution
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_alias_mercado() {
        let (cat, sub) = resolve_category_alias("mercado").unwrap();
        assert_eq!(cat, "Alimentação");
        assert_eq!(sub, "Mercado");
    }

    #[test]
    fn test_resolve_alias_restaurante() {
        let (cat, sub) = resolve_category_alias("restaurante").unwrap();
        assert_eq!(cat, "Alimentação");
        assert_eq!(sub, "Restaurante");
    }

    #[test]
    fn test_resolve_alias_farmacia() {
        let (cat, sub) = resolve_category_alias("farmacia").unwrap();
        assert_eq!(cat, "Saúde");
        assert_eq!(sub, "Farmácia");
    }

    #[test]
    fn test_resolve_alias_unknown() {
        assert!(resolve_category_alias("nonexistent").is_none());
    }

    // -----------------------------------------------------------------------
    // Release notes rendering
    // -----------------------------------------------------------------------

    #[test]
    fn test_render_release_notes_contains_version() {
        let body = "### Features\n- Item 1\n- Item 2\n### Bug Fixes\n- Fix 1";
        let output = render_release_notes("1.1.0", "1.2.0", body, "test.local");
        assert!(output.contains("1.1.0"));
        assert!(output.contains("1.2.0"));
        assert!(output.contains("Novidade"));
        assert!(output.contains("Item 1"));
        assert!(output.contains("Correções"));
        assert!(output.contains("Fix 1"));
        assert!(output.contains("test.local"));
    }

    #[test]
    fn test_parse_changelog_sections() {
        let body = "### Features\n- Alpha\n- Beta\n\n### Bug Fixes\n- Gamma";
        let sections = parse_changelog_sections(body);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "Novidade");
        assert_eq!(sections[0].1, vec!["Alpha", "Beta"]);
        assert_eq!(sections[1].0, "Correções");
        assert_eq!(sections[1].1, vec!["Gamma"]);
    }

    // -----------------------------------------------------------------------
    // Full message rendering (integration-style)
    // -----------------------------------------------------------------------

    fn make_sync_input() -> SyncMessageInput {
        let txs = vec![
            make_tx(
                "1",
                "2026-05-18",
                "Pagamento de fatura",
                "-6069.89",
                "payment",
                None,
                "unclassified",
                Some("checking1"),
                None,
            ),
            make_tx(
                "2",
                "2026-05-18",
                "Pagamento recebido",
                "+6069.89",
                "credit",
                None,
                "unclassified",
                Some("card1"),
                None,
            ),
            make_tx(
                "3",
                "2026-05-18",
                "Grupo Madero",
                "-77.00",
                "debit",
                Some("alimentacao:restaurante"),
                "rule",
                Some("checking1"),
                None,
            ),
            make_tx(
                "4",
                "2026-05-17",
                "Supermercados Imperatr",
                "-25.88",
                "debit",
                Some("alimentacao:mercado"),
                "rule",
                Some("checking1"),
                None,
            ),
            make_tx(
                "5",
                "2026-05-17",
                "Mp *Emporiopantan",
                "-53.00",
                "debit",
                None,
                "unclassified",
                Some("checking1"),
                None,
            ),
            make_tx(
                "6",
                "2026-05-17",
                "Gimpel",
                "-67.87",
                "debit",
                Some("alimentacao:mercado"),
                "rule",
                Some("checking1"),
                None,
            ),
        ];
        let accounts = vec![
            make_account("checking1", "checking"),
            make_account("card1", "credit"),
        ];
        let review_items = build_review_items(&txs);

        SyncMessageInput {
            balance: Some(Decimal::new(1243018, 2)), // 12430.18
            new_transactions: txs,
            accounts,
            snapshots: vec![],
            review_items,
            sync_time: Utc::now(),
            version: "1.2.0".to_string(),
            hostname: "test.local".to_string(),
        }
    }

    #[test]
    fn test_render_full_message_contains_balance() {
        let input = make_sync_input();
        let msg = render_sync_message(&input);
        assert!(msg.contains("Saldo em conta"));
        assert!(msg.contains("12.430,18"));
    }

    #[test]
    fn test_render_full_message_card_payment_compensated() {
        let input = make_sync_input();
        let msg = render_sync_message(&input);
        assert!(msg.contains("compensado"));
        assert!(msg.contains("movimentação interna"));
    }

    #[test]
    fn test_render_full_message_has_review_items() {
        let input = make_sync_input();
        let msg = render_sync_message(&input);
        assert!(msg.contains("Preciso da sua ajuda"));
        assert!(msg.contains("Supermercados Imperatr"));
        assert!(msg.contains("Mp *Emporiopantan"));
    }

    #[test]
    fn test_render_full_message_has_response_instructions() {
        let input = make_sync_input();
        let msg = render_sync_message(&input);
        assert!(msg.contains("`1 ok`"));
        assert!(msg.contains("`todos ok`"));
        assert!(msg.contains("`2 mercado`"));
    }

    #[test]
    fn test_render_omits_balance_when_none() {
        let mut input = make_sync_input();
        input.balance = None;
        let msg = render_sync_message(&input);
        assert!(!msg.contains("Saldo em conta"));
    }

    #[test]
    fn test_render_omits_review_when_none() {
        let txs = vec![make_tx(
            "1",
            "2026-05-18",
            "Grupo Madero",
            "-77.00",
            "debit",
            Some("alimentacao:restaurante"),
            "rule",
            Some("checking1"),
            None,
        )];
        let accounts = vec![make_account("checking1", "checking")];
        let review_items = build_review_items(&txs);
        let input = SyncMessageInput {
            balance: Some(Decimal::new(100000, 2)),
            new_transactions: txs,
            accounts,
            snapshots: vec![],
            review_items,
            sync_time: Utc::now(),
            version: "1.2.0".to_string(),
            hostname: "test.local".to_string(),
        };
        let msg = render_sync_message(&input);
        assert!(!msg.contains("Preciso da sua ajuda"));
    }

    #[test]
    fn test_render_footer_contains_version() {
        let input = make_sync_input();
        let msg = render_sync_message(&input);
        assert!(msg.contains("1.2.0"));
        assert!(msg.contains("Pluggy sincronizado"));
    }
}
