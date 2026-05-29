#[cfg(test)]
use crate::models::TransactionSplitPayload;
use anyhow::{anyhow, bail, Context, Result};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::str::FromStr;

fn decimal_from_json_value(value: &Value) -> Result<Decimal> {
    match value {
        Value::Number(number) => Decimal::from_str(&number.to_string())
            .with_context(|| format!("Falha ao parsear decimal '{number}'")),
        Value::String(raw) => Decimal::from_str(raw.trim())
            .with_context(|| format!("Falha ao parsear decimal '{raw}'")),
        _ => Err(anyhow!(
            "Valor decimal inválido: esperado número ou string, recebido {value}"
        )),
    }
}

pub fn deserialize_decimal_from_json<'de, D>(
    deserializer: D,
) -> std::result::Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    decimal_from_json_value(&value).map_err(DeError::custom)
}

pub fn deserialize_optional_decimal_from_json<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => decimal_from_json_value(&raw)
            .map(Some)
            .map_err(DeError::custom),
    }
}

pub fn decimal_to_cents_exact(value: Decimal, field_name: &str) -> Result<i128> {
    let cents = value * Decimal::new(100, 0);
    if cents.fract() != Decimal::ZERO {
        bail!("{field_name} precisa ter no máximo 2 casas decimais: {value}");
    }
    cents
        .to_i128()
        .with_context(|| format!("{field_name} fora do intervalo suportado: {value}"))
}

#[cfg(test)]
pub fn parse_test_split_payload(
    payload_json: &Value,
    parent_amount: Decimal,
) -> Result<TransactionSplitPayload> {
    let payload: TransactionSplitPayload = serde_json::from_value(payload_json.clone())
        .context("Payload de split inválido: formato JSON incompatível")?;
    validate_test_split_payload(&payload, parent_amount)?;
    Ok(payload)
}

#[cfg(test)]
pub fn validate_test_split_payload(
    payload: &TransactionSplitPayload,
    parent_amount: Decimal,
) -> Result<()> {
    if payload.lines.is_empty() {
        bail!("Payload de split inválido: 'lines' é obrigatório e não pode ser vazio");
    }

    let parent_cents = decimal_to_cents_exact(parent_amount, "amount da transação")?;
    let mut lines_total_cents: i128 = 0;

    for (index, line) in payload.lines.iter().enumerate() {
        let line_cents = decimal_to_cents_exact(line.amount, &format!("amount da linha {index}"))?;
        if parent_cents != 0 && line_cents != 0 && line_cents.signum() != parent_cents.signum() {
            bail!(
                "Linha {index} com sinal divergente: esperado mesmo sinal do amount da transação"
            );
        }
        lines_total_cents = lines_total_cents
            .checked_add(line_cents)
            .ok_or_else(|| anyhow!("Soma das linhas de split excede o intervalo suportado"))?;
    }

    if lines_total_cents != parent_cents {
        bail!(
            "Total das linhas ({lines_total_cents} cents) difere do amount da transação ({parent_cents} cents)"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_test_split_payload_accepts_valid_lines_and_optional_items() {
        let payload = json!({
            "lines": [
                {
                    "lineId": "l1",
                    "description": "Combustível",
                    "amount": "120.30",
                    "items": [
                        {
                            "description": "Gasolina aditivada",
                            "quantity": "40.1",
                            "unitPrice": "3.000",
                            "amount": "120.30"
                        }
                    ]
                },
                {
                    "lineId": "l2",
                    "description": "Lavagem",
                    "amount": "9.70"
                }
            ]
        });

        let parsed = parse_test_split_payload(&payload, Decimal::new(13000, 2)).unwrap();
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].items.len(), 1);
    }

    #[test]
    fn parse_test_split_payload_rejects_empty_lines() {
        let payload = json!({ "lines": [] });
        let err = parse_test_split_payload(&payload, Decimal::new(1000, 2)).unwrap_err();
        assert!(err.to_string().contains("'lines' é obrigatório"));
    }

    #[test]
    fn parse_test_split_payload_rejects_amount_mismatch() {
        let payload = json!({
            "lines": [
                { "amount": "10.00" },
                { "amount": "10.01" }
            ]
        });
        let err = parse_test_split_payload(&payload, Decimal::new(2000, 2)).unwrap_err();
        assert!(err.to_string().contains("difere do amount da transação"));
    }

    #[test]
    fn parse_test_split_payload_rejects_wrong_sign() {
        let payload = json!({
            "lines": [
                { "amount": "10.00" },
                { "amount": "-1.00" }
            ]
        });
        let err = parse_test_split_payload(&payload, Decimal::new(900, 2)).unwrap_err();
        assert!(err.to_string().contains("sinal divergente"));
    }

    #[test]
    fn decimal_to_cents_requires_cent_precision() {
        let err = decimal_to_cents_exact(Decimal::new(1234, 3), "teste").unwrap_err();
        assert!(err.to_string().contains("no máximo 2 casas decimais"));
    }
}
