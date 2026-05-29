use anyhow::Result;
use phai_core::models::{
    CardSummaryRow, CashflowRow, ForecastVsActualRow, MonthlySpendRow, UncategorizedRow,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct ReviewPayload {
    pub generated_at: String,
    pub cashflow: Vec<CashflowRow>,
    pub monthly_spend: Vec<MonthlySpendRow>,
    pub card_summary: Vec<CardSummaryRow>,
    pub forecast_vs_actual: Vec<ForecastVsActualRow>,
    pub uncategorized_count: i64,
    pub uncategorized: Vec<UncategorizedRow>,
}

pub fn generate_html(payload: &ReviewPayload) -> Result<String> {
    let json = serde_json::to_string(payload)?;
    let template = include_str!("review_template.html");
    Ok(template.replace("__REVIEW_DATA__", &json))
}
