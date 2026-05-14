#![allow(dead_code)]

use chrono::NaiveDate;
use rust_decimal::Decimal;

// ── Basic formatting ─────────────────────────────────────────────────────────

/// Wrap a string in WhatsApp bold markers.
pub fn bold(s: &str) -> String {
    format!("*{s}*")
}

/// Format a Decimal as Brazilian currency (no sign).  R$ 1.234,56
pub fn brl(value: Decimal) -> String {
    let rounded = format!("{:.2}", value.abs().round_dp(2)).replace('.', ",");
    // Insert thousands separator
    let (integer_part, decimal_part) = rounded.split_once(',').unwrap_or((&rounded, "00"));
    let with_sep = insert_thousands(integer_part);
    format!("R$ {with_sep},{decimal_part}")
}

/// Format a Decimal with sign: negative → -R$ X, zero/positive → R$ X (no +).
pub fn brl_signed(value: Decimal) -> String {
    let base = brl(value);
    if value.is_sign_negative() {
        format!("-{base}")
    } else {
        base
    }
}

fn insert_thousands(s: &str) -> String {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if len <= 3 {
        return s.to_string();
    }
    let mut out = String::with_capacity(len + len / 3);
    let rem = len % 3;
    for (i, &b) in bytes.iter().enumerate() {
        if i != 0 && (i % 3 == rem) {
            out.push('.');
        }
        out.push(b as char);
    }
    out
}

/// Format a NaiveDate as dd/mm/yyyy.
pub fn short_date(date: NaiveDate) -> String {
    date.format("%d/%m/%Y").to_string()
}

/// Format a percentage value with one decimal place.  e.g. 12.3%
pub fn pct(value: Decimal) -> String {
    format!("{:.1}%", value.round_dp(1))
}

/// Convert a "YYYY-MM" string to a Portuguese month label.  "2026-05" → "maio/2026"
pub fn month_label(month_ref: &str) -> String {
    let parts: Vec<&str> = month_ref.splitn(2, '-').collect();
    if parts.len() != 2 {
        return month_ref.to_string();
    }
    let year = parts[0];
    let month_num: u32 = parts[1].parse().unwrap_or(0);
    let month_name = match month_num {
        1 => "janeiro",
        2 => "fevereiro",
        3 => "março",
        4 => "abril",
        5 => "maio",
        6 => "junho",
        7 => "julho",
        8 => "agosto",
        9 => "setembro",
        10 => "outubro",
        11 => "novembro",
        12 => "dezembro",
        _ => month_ref,
    };
    format!("{month_name}/{year}")
}

// ── Category helpers ──────────────────────────────────────────────────────────

/// Extract the top-level family from a category_id.
/// "alimentacao:mercado" → Some("alimentacao")
pub fn category_family(category_id: Option<&str>) -> Option<String> {
    let raw = category_id?.trim();
    if raw.is_empty() {
        return None;
    }
    let normalized = raw.replace([':', '-', '>'], " ");
    normalized
        .split_whitespace()
        .next()
        .map(|part| part.to_string())
}

/// Emoji for a category family.
pub fn category_emoji(category_id: Option<&str>, amount: Option<Decimal>) -> &'static str {
    let family = category_family(category_id);
    if family.as_deref() == Some("receitas")
        || family.as_deref() == Some("salario")
        || amount.is_some_and(|v| v > Decimal::ZERO)
    {
        "💰"
    } else if family.as_deref().is_some_and(|f| f.starts_with("transfer")) {
        "🔁"
    } else if family.as_deref() == Some("assinaturas") {
        "🔂"
    } else if matches!(family.as_deref(), Some("moradia" | "casa")) {
        "🏠"
    } else if family.as_deref() == Some("alimentacao") {
        "🍽️"
    } else if family.as_deref() == Some("saude") {
        "🩺"
    } else if matches!(family.as_deref(), Some("transporte" | "mobilidade")) {
        "🚗"
    } else if family.as_deref() == Some("educacao") {
        "📚"
    } else if family.as_deref() == Some("lazer") {
        "🎉"
    } else if family.as_deref() == Some("investimentos") {
        "📈"
    } else if family.as_deref() == Some("financeiro") {
        "🧾"
    } else if family.is_none() {
        "❓"
    } else {
        "💸"
    }
}

/// Human-readable label for a family.
pub fn family_label(family: &str) -> String {
    family.replace('-', " ")
}

/// Truncate and clean up a description for inline display.
pub fn short_description(desc: &str) -> String {
    let cleaned: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() > 40 {
        let truncated: String = cleaned.chars().take(38).collect();
        format!("{truncated}…")
    } else {
        cleaned
    }
}

/// Section header line.  "🍽️ *Alimentação*"
pub fn section_header(emoji: &str, title: &str) -> String {
    format!("{emoji} {}", bold(title))
}

/// Category subtotal line.  "  alimentacao > mercado   R$ 123,45"
pub fn category_subtotal(id: &str, total: Decimal) -> String {
    let humanized = id.replace(':', " > ").replace('-', " ");
    format!("  {humanized}   {}", brl(total))
}
