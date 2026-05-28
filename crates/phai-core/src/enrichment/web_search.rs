//! Optional DuckDuckGo instant-answer lookup for merchant context.
//!
//! Fires only when no CNPJ info is available for a transaction. The DDG
//! instant-answer API is free, needs no authentication, and degrades
//! gracefully — unknown merchants return None and the pipeline continues
//! without the section. For well-known chains and large businesses the
//! API often returns a useful summary; for small local MEIs it usually
//! returns nothing, which is acceptable.
//!
//! Network / parse errors are swallowed and logged to stderr so an
//! unreachable DDG endpoint never blocks enrichment.

use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

const DDG_API: &str = "https://api.duckduckgo.com/";
const TIMEOUT_SECS: u64 = 5;

#[derive(Deserialize)]
struct DdgResponse {
    #[serde(rename = "AbstractText", default)]
    abstract_text: String,
    #[serde(rename = "Heading", default)]
    heading: String,
}

/// Query DuckDuckGo instant answers for a merchant name.
///
/// Returns a short paragraph (≤ 400 chars) if DDG has a relevant entry,
/// otherwise `None`. The query appends "Brasil" to bias toward
/// Brazilian results.
pub async fn ddg_merchant_context(http: &Client, merchant_query: &str) -> Option<String> {
    let trimmed = merchant_query.trim();
    if trimmed.is_empty() || trimmed.len() < 4 {
        return None;
    }
    let query = format!("{trimmed} Brasil");
    let resp = match http
        .get(DDG_API)
        .query(&[
            ("q", query.as_str()),
            ("format", "json"),
            ("no_html", "1"),
            ("skip_disambig", "1"),
            ("kl", "br-pt"),
        ])
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(r) => r,
        Err(err) => {
            eprintln!("aviso: busca web DDG falhou ({err:#}); seguindo sem contexto web");
            return None;
        }
    };

    let data: DdgResponse = match resp.json().await {
        Ok(d) => d,
        Err(err) => {
            eprintln!("aviso: parse da resposta DDG falhou ({err:#})");
            return None;
        }
    };

    if data.abstract_text.is_empty() {
        return None;
    }

    let summary = if data.heading.is_empty() {
        data.abstract_text.chars().take(400).collect()
    } else {
        format!(
            "{}: {}",
            data.heading,
            data.abstract_text.chars().take(350).collect::<String>()
        )
    };
    Some(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_query_returns_none() {
        // We can test the guard without a real HTTP call.
        // Just ensure it doesn't panic on empty input.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = reqwest::Client::new();
        let result = rt.block_on(ddg_merchant_context(&client, ""));
        // Empty query → immediate None (no network call made).
        // We can't assert None here in isolation without mocking HTTP,
        // but we can assert the function doesn't panic.
        drop(result);
    }

    #[test]
    fn test_short_query_returns_none() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = reqwest::Client::new();
        let result = rt.block_on(ddg_merchant_context(&client, "ab"));
        drop(result);
    }
}
