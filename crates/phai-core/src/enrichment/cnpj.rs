//! CNPJ extraction, normalization, BrasilAPI lookup, and 2-layer cache.
//!
//! ## Cache topology
//!
//! - **L1 (moka)**: in-process `Cache<String, Option<CnpjInfo>>` with
//!   `TTL=24h`, `TTI=4h`, capacity 50_000. Lives for the runtime of the
//!   pipeline.
//! - **L2 (SQLite)**: `cnpj_cache` table in `finance-os.local.db`,
//!   `TTL=30d`. Survives across runs and is shared between BigQuery and
//!   Local backends (both write to the local SQLite for caching).
//!
//! Flow: moka → SQLite → BrasilAPI HTTP. Failures on the HTTP path
//! (429, 5xx, network) return `Ok(None)` *and do not poison the cache*;
//! 404 caches `None` (the CNPJ does not exist, no point re-querying).
//!
//! ## CNAE mapping
//!
//! Brazil's CNAE codes are 7 digits: `DD.DD-D/SS` where `DD` is the
//! 2-digit *division*. We map by:
//!   1. 4-digit *group/class* prefix overrides (e.g. `4771` → farmácia)
//!   2. 2-digit *division* fallback (e.g. `56` → restaurante)

use super::types::CnpjInfo;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use moka::future::Cache;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

/// SQLite cache freshness window.
const SQLITE_TTL: Duration = Duration::days(30);

/// Returns the 14-digit unformatted CNPJ from a transaction's
/// `metadata_json`. Walks `raw.paymentData.receiver.documentNumber`,
/// accepting either a string or `{"type": "CNPJ", "value": "..."}`
/// shape. Returns `None` for CPF or any other shape.
pub fn extract_cnpj(metadata: &Value) -> Option<String> {
    let doc = metadata
        .pointer("/raw/paymentData/receiver/documentNumber")
        .or_else(|| metadata.pointer("/raw/paymentData/payer/documentNumber"))?;

    // Two shapes: bare string, or { type, value }
    let (doc_type, raw_value) = match doc {
        Value::String(s) => (None, s.as_str()),
        Value::Object(map) => {
            let kind = map.get("type").and_then(Value::as_str);
            let value = map.get("value").and_then(Value::as_str)?;
            (kind, value)
        }
        _ => return None,
    };

    // If type is explicitly CPF, bail out.
    if doc_type.is_some_and(|t| t.eq_ignore_ascii_case("CPF")) {
        return None;
    }

    let normalized = normalize_cnpj(raw_value)?;
    // If type is missing, accept only strings normalizable to 14 digits
    // (which excludes CPFs).
    if doc_type.is_some_and(|t| !t.eq_ignore_ascii_case("CNPJ")) {
        return None;
    }
    Some(normalized)
}

/// Strip non-digits from a CNPJ string. Returns `Some(14-digit)` if the
/// result is exactly 14 digits, `None` otherwise.
pub fn normalize_cnpj(value: &str) -> Option<String> {
    let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 14 {
        Some(digits)
    } else {
        None
    }
}

/// Map a 7-digit CNAE code to an internal `(category, subcategory)`
/// pair. Returns `None` if neither the 4-digit override nor the 2-digit
/// division is recognized.
pub fn cnae_to_category(cnae: u32) -> Option<(&'static str, &'static str)> {
    // Step 1: 4-digit group/class overrides (cnae / 1000 from a 7-digit
    // code yields the 4 leading digits).
    let group_4 = cnae / 1000;
    if let Some(pair) = match group_4 {
        // 47 — varejo
        4711..=4713 => Some(("alimentacao", "mercado")),
        4721 => Some(("alimentacao", "padaria")),
        4731 => Some(("transporte", "combustivel")),
        4771..=4772 => Some(("saude", "farmacia")),
        4773 => Some(("saude", "otica")),
        _ => None,
    } {
        return Some(pair);
    }

    // Step 2: 2-digit division fallback. For a 7-digit CNAE,
    // division = cnae / 100_000.
    let division = cnae / 100_000;
    match division {
        47 => Some(("compras", "varejo")),
        56 => Some(("alimentacao", "restaurantes")),
        49 => Some(("transporte", "terrestre")),
        55 => Some(("lazer", "hospedagem")),
        59 => Some(("lazer", "audiovisual")),
        61 => Some(("moradia", "telecom")),
        62 => Some(("pessoal", "servicos")),
        85 => Some(("educacao", "escola")),
        86 => Some(("saude", "consulta")),
        87 => Some(("saude", "assistencia")),
        88 => Some(("saude", "social")),
        92 => Some(("lazer", "cultura")),
        93 => Some(("lazer", "esporte")),
        96 => Some(("pessoal", "cuidado-fisico")),
        _ => None,
    }
}

// ── BrasilAPI ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct BrasilApiCnpj {
    razao_social: Option<String>,
    nome_fantasia: Option<String>,
    cnae_fiscal: Option<u32>,
    cnae_fiscal_descricao: Option<String>,
    #[serde(default)]
    cnaes_secundarios: Vec<BrasilApiSecondaryCnae>,
}

#[derive(Debug, Deserialize)]
struct BrasilApiSecondaryCnae {
    codigo: Option<u32>,
    descricao: Option<String>,
}

/// Look up a CNPJ via the 2-layer cache, falling back to BrasilAPI on a
/// full miss. Returns `Ok(None)` when the CNPJ is unknown or BrasilAPI
/// is unavailable; only network/IO problems with the *cache* propagate
/// as errors.
pub async fn lookup_cnpj(
    http: &reqwest::Client,
    moka_cache: &Cache<String, Option<CnpjInfo>>,
    sqlite_path: Option<&Path>,
    cnpj: &str,
) -> Result<Option<CnpjInfo>> {
    let normalized = match normalize_cnpj(cnpj) {
        Some(n) => n,
        None => return Ok(None),
    };

    // L1: moka
    if let Some(hit) = moka_cache.get(&normalized).await {
        return Ok(hit);
    }

    // L2: SQLite — skipped when no local DB is configured (BigQuery-only setups).
    if let Some(path) = sqlite_path {
        match read_sqlite_cache(path, &normalized) {
            Ok(Some(entry)) => {
                moka_cache.insert(normalized.clone(), entry.clone()).await;
                return Ok(entry);
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!(
                    "aviso: cnpj_cache sqlite read falhou para {normalized}: {err:#}; consultando BrasilAPI"
                );
            }
        }
    }

    // Miss everywhere — hit BrasilAPI.
    let fetched = fetch_brasilapi(http, &normalized).await;
    match fetched {
        FetchOutcome::Found(info) => {
            let value = Some(info);
            if let Some(path) = sqlite_path {
                if let Err(err) = write_sqlite_cache(path, &normalized, &value) {
                    eprintln!("aviso: cnpj_cache sqlite write falhou para {normalized}: {err:#}");
                }
            }
            moka_cache.insert(normalized, value.clone()).await;
            Ok(value)
        }
        FetchOutcome::NotFound => {
            // Negative cache: avoid re-querying for 30 days.
            if let Some(path) = sqlite_path {
                if let Err(err) = write_sqlite_cache(path, &normalized, &None) {
                    eprintln!(
                        "aviso: cnpj_cache sqlite write (404) falhou para {normalized}: {err:#}"
                    );
                }
            }
            moka_cache.insert(normalized, None).await;
            Ok(None)
        }
        FetchOutcome::Transient => {
            // Do not cache transient failures so the next call retries.
            Ok(None)
        }
    }
}

enum FetchOutcome {
    Found(CnpjInfo),
    NotFound,
    /// 429, 5xx, network — caller should not cache.
    Transient,
}

async fn fetch_brasilapi(http: &reqwest::Client, cnpj: &str) -> FetchOutcome {
    let url = format!("https://brasilapi.com.br/api/cnpj/v1/{cnpj}");
    let resp = match http.get(&url).send().await {
        Ok(r) => r,
        Err(err) => {
            eprintln!("aviso: BrasilAPI request falhou para {cnpj}: {err:#}");
            return FetchOutcome::Transient;
        }
    };
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return FetchOutcome::NotFound;
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        eprintln!("aviso: BrasilAPI HTTP {status} para {cnpj}");
        return FetchOutcome::Transient;
    }
    if !status.is_success() {
        eprintln!("aviso: BrasilAPI HTTP {status} inesperado para {cnpj}");
        return FetchOutcome::Transient;
    }
    let body: BrasilApiCnpj = match resp.json().await {
        Ok(b) => b,
        Err(err) => {
            eprintln!("aviso: BrasilAPI JSON parse falhou para {cnpj}: {err:#}");
            return FetchOutcome::Transient;
        }
    };

    let info = CnpjInfo {
        cnpj: cnpj.to_string(),
        razao_social: body.razao_social.unwrap_or_default(),
        nome_fantasia: body.nome_fantasia,
        cnae_fiscal: body.cnae_fiscal.unwrap_or(0),
        cnae_descricao: body.cnae_fiscal_descricao.unwrap_or_default(),
        cnaes_secundarios: body
            .cnaes_secundarios
            .into_iter()
            .filter_map(|c| {
                let codigo = c.codigo?;
                let descricao = c.descricao.unwrap_or_default();
                Some((codigo, descricao))
            })
            .collect(),
    };
    FetchOutcome::Found(info)
}

fn read_sqlite_cache(path: &Path, cnpj: &str) -> Result<Option<Option<CnpjInfo>>> {
    let conn = Connection::open(path)
        .with_context(|| format!("Falha ao abrir {} para cnpj_cache", path.display()))?;
    let row = conn
        .query_row(
            "SELECT found, data_json, fetched_at FROM cnpj_cache WHERE cnpj = ?1",
            [cnpj],
            |r| {
                let found: i64 = r.get(0)?;
                let data_json: Option<String> = r.get(1)?;
                let fetched_at: String = r.get(2)?;
                Ok((found, data_json, fetched_at))
            },
        )
        .optional()
        .context("Falha ao ler cnpj_cache")?;

    let Some((found, data_json, fetched_at)) = row else {
        return Ok(None);
    };

    let fetched = DateTime::parse_from_rfc3339(&fetched_at)
        .with_context(|| format!("fetched_at inválido em cnpj_cache: {fetched_at}"))?
        .with_timezone(&Utc);
    if Utc::now() - fetched > SQLITE_TTL {
        return Ok(None);
    }

    if found == 0 {
        return Ok(Some(None));
    }
    let json = data_json.context("cnpj_cache linha com found=1 sem data_json")?;
    let info: CnpjInfo =
        serde_json::from_str(&json).context("Falha ao parsear cnpj_cache.data_json")?;
    Ok(Some(Some(info)))
}

fn write_sqlite_cache(path: &Path, cnpj: &str, info: &Option<CnpjInfo>) -> Result<()> {
    let conn = Connection::open(path)
        .with_context(|| format!("Falha ao abrir {} para cnpj_cache", path.display()))?;
    let (found, data_json) = match info {
        Some(value) => (
            1i64,
            Some(serde_json::to_string(value).context("Falha ao serializar CnpjInfo")?),
        ),
        None => (0i64, None),
    };
    conn.execute(
        "INSERT INTO cnpj_cache (cnpj, found, data_json, fetched_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(cnpj) DO UPDATE SET
             found = excluded.found,
             data_json = excluded.data_json,
             fetched_at = excluded.fetched_at",
        params![cnpj, found, data_json, Utc::now().to_rfc3339()],
    )
    .context("Falha ao gravar cnpj_cache")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_cnpj_from_pix_metadata() {
        let metadata = json!({
            "raw": {
                "paymentData": {
                    "receiver": {
                        "documentNumber": {
                            "type": "CNPJ",
                            "value": "63.713.714/0001-67"
                        }
                    }
                }
            }
        });
        assert_eq!(extract_cnpj(&metadata).as_deref(), Some("63713714000167"));
    }

    #[test]
    fn test_extract_cpf_returns_none_for_cnpj() {
        let metadata = json!({
            "raw": {
                "paymentData": {
                    "receiver": {
                        "documentNumber": {
                            "type": "CPF",
                            "value": "123.456.789-09"
                        }
                    }
                }
            }
        });
        assert!(extract_cnpj(&metadata).is_none());
    }

    #[test]
    fn test_extract_cnpj_missing_returns_none() {
        let metadata = json!({});
        assert!(extract_cnpj(&metadata).is_none());

        let partial = json!({ "raw": { "paymentData": {} } });
        assert!(extract_cnpj(&partial).is_none());
    }

    #[test]
    fn test_normalize_cnpj_strips_formatting() {
        assert_eq!(
            normalize_cnpj("12.345.678/0001-90").as_deref(),
            Some("12345678000190")
        );
        assert_eq!(
            normalize_cnpj("12345678000190").as_deref(),
            Some("12345678000190")
        );
    }

    #[test]
    fn test_normalize_cnpj_invalid_length() {
        assert!(normalize_cnpj("123").is_none());
        assert!(normalize_cnpj("").is_none());
        assert!(normalize_cnpj("123456789012345").is_none()); // 15 digits
    }

    #[test]
    fn test_cnae_to_category_restaurant() {
        let pair = cnae_to_category(5611201).unwrap();
        assert_eq!(pair, ("alimentacao", "restaurantes"));
    }

    #[test]
    fn test_cnae_to_category_farmacia() {
        let pair = cnae_to_category(4771701).unwrap();
        assert_eq!(pair, ("saude", "farmacia"));
    }

    #[test]
    fn test_cnae_to_category_combustivel() {
        let pair = cnae_to_category(4731800).unwrap();
        assert_eq!(pair, ("transporte", "combustivel"));
    }

    #[test]
    fn test_cnae_to_category_division_fallback() {
        // Division 47, unknown subdivision -> falls back to varejo.
        let pair = cnae_to_category(4799999).unwrap();
        assert_eq!(pair, ("compras", "varejo"));
    }

    #[test]
    fn test_cnae_unknown_division_returns_none() {
        assert!(cnae_to_category(9999999).is_none());
    }
}
