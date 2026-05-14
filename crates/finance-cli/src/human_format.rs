#![allow(dead_code)]

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;

/// Wraps text in WhatsApp bold markers.
pub fn bold(s: &str) -> String {
    format!("*{s}*")
}

/// Formats a Decimal as BRL currency with sign, e.g. `+R$ 1.234,56`.
pub fn brl(value: Decimal) -> String {
    let sign = if value.is_sign_negative() { "-" } else { "+" };
    let rounded = format!("{:.2}", value.abs().round_dp(2)).replace('.', ",");
    format!("{sign}R$ {rounded}")
}

/// Formats a Decimal as BRL currency always showing sign (same as `brl`).
pub fn brl_signed(value: Decimal) -> String {
    brl(value)
}

/// Formats a NaiveDate as `dd/mmm` (e.g. `13/abr`).
pub fn short_date(date: NaiveDate) -> String {
    let month_abbr = match date.month() {
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
        _ => "???",
    };
    format!("{:02}/{}", date.day(), month_abbr)
}

/// Returns the first segment of a colon/dash-separated category ID (the "family").
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

/// Maps a category family to a representative emoji.
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

/// Returns a human-friendly label for a category family.
pub fn family_label(category_id: Option<&str>) -> String {
    category_id
        .map(|c| c.replace(':', " › ").replace('-', " "))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "sem categoria".to_string())
}

/// Trims and collapses whitespace in a description, truncated to `max_chars`.
pub fn short_description(s: &str, max_chars: usize) -> String {
    let normalized: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        normalized
    } else {
        let truncated: String = normalized
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect();
        format!("{truncated}…")
    }
}

/// Returns a bold section header line, e.g. `*── Assinaturas ──*`.
pub fn section_header(title: &str) -> String {
    bold(&format!("── {title} ──"))
}

/// Returns a formatted category subtotal line.
pub fn category_subtotal(category_id: Option<&str>, amount: Decimal) -> String {
    let emoji = category_emoji(category_id, None);
    let label = family_label(category_id);
    format!("{emoji} {label}: {}", brl(amount))
}

/// Returns a progress bar string like `[▓▓▓▓░░░░░░] 40%`.
/// `pct` is clamped to 0–100+.
pub fn progress_bar(pct: i64) -> String {
    const TOTAL: usize = 10;
    let clamped = pct.max(0) as usize;
    let filled = (clamped * TOTAL / 100).min(TOTAL);
    let empty = TOTAL - filled;
    let bar: String = "▓".repeat(filled) + &"░".repeat(empty);
    format!("[{bar}] {pct}%")
}

/// Formats a month ref like "2026-03" as a Portuguese label, e.g. "mar/26".
pub fn month_label(month_ref: &str) -> String {
    // month_ref expected as "YYYY-MM"
    let parts: Vec<&str> = month_ref.splitn(2, '-').collect();
    if parts.len() != 2 {
        return month_ref.to_string();
    }
    let year = parts[0];
    let month: u32 = parts[1].parse().unwrap_or(0);
    let short_year = if year.len() >= 4 { &year[2..] } else { year };
    let abbr = match month {
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
        _ => "???",
    };
    format!("{abbr}/{short_year}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn test_bold() {
        assert_eq!(bold("hello"), "*hello*");
    }

    #[test]
    fn test_brl_positive() {
        assert_eq!(brl(d("1234.56")), "+R$ 1234,56");
    }

    #[test]
    fn test_brl_negative() {
        assert_eq!(brl(d("-99.90")), "-R$ 99,90");
    }

    #[test]
    fn test_short_date() {
        let date = NaiveDate::from_ymd_opt(2026, 4, 13).unwrap();
        assert_eq!(short_date(date), "13/abr");
    }

    #[test]
    fn test_progress_bar_zero() {
        assert_eq!(progress_bar(0), "[░░░░░░░░░░] 0%");
    }

    #[test]
    fn test_progress_bar_full() {
        assert_eq!(progress_bar(100), "[▓▓▓▓▓▓▓▓▓▓] 100%");
    }

    #[test]
    fn test_progress_bar_40() {
        assert_eq!(progress_bar(40), "[▓▓▓▓░░░░░░] 40%");
    }

    #[test]
    fn test_progress_bar_over() {
        // over budget: capped at 10 filled bars, label shows actual pct
        assert_eq!(progress_bar(150), "[▓▓▓▓▓▓▓▓▓▓] 150%");
    }

    #[test]
    fn test_month_label() {
        assert_eq!(month_label("2026-03"), "mar/26");
        assert_eq!(month_label("2026-12"), "dez/26");
    }

    #[test]
    fn test_short_description_truncates() {
        let s = "Lorem ipsum dolor sit amet";
        assert_eq!(short_description(s, 11), "Lorem ipsu…");
    }
}
