//! Temporal-context helpers: extract Pluggy's per-day `order` index from
//! a transaction's metadata, and build a [`ContextTx`] row from a stored
//! transaction.
//!
//! Querying actual sibling transactions lives on the `FinanceStore`
//! trait (`transactions_on_date`); this module is just the pure logic.

use super::types::ContextTx;
use crate::models::TransactionRecord;
use serde_json::Value;

/// Pluggy includes a stable per-day `order` integer inside
/// `metadata_json.raw.order`. We use it to order temporal context so the
/// LLM can reason about "what came just before/after this transaction".
pub fn extract_order(metadata: &Value) -> Option<i64> {
    metadata.pointer("/raw/order").and_then(Value::as_i64)
}

/// Read the Pluggy coarse category from `metadata_json.pluggy_category`.
pub fn extract_pluggy_category(metadata: &Value) -> Option<String> {
    metadata
        .pointer("/pluggy_category")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Build a context row from a stored transaction.
pub fn build_context_tx(record: &TransactionRecord) -> ContextTx {
    ContextTx {
        description: record.description.clone(),
        amount: record.amount,
        pluggy_category: extract_pluggy_category(&record.metadata_json),
        order: extract_order(&record.metadata_json),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_order_present() {
        let metadata = json!({ "raw": { "order": 5 } });
        assert_eq!(extract_order(&metadata), Some(5));
    }

    #[test]
    fn test_extract_order_missing() {
        let metadata = json!({});
        assert!(extract_order(&metadata).is_none());

        let no_raw = json!({ "pluggy_category": "Eating out" });
        assert!(extract_order(&no_raw).is_none());
    }

    #[test]
    fn test_extract_pluggy_category() {
        let metadata = json!({ "pluggy_category": "Groceries" });
        assert_eq!(extract_pluggy_category(&metadata).as_deref(), Some("Groceries"));

        let missing = json!({});
        assert!(extract_pluggy_category(&missing).is_none());
    }
}
