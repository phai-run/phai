pub mod config;
pub mod enrichment;
pub mod idempotency;
pub mod installments;
pub mod legacy;
pub mod migrations;
pub mod models;
pub mod pluggy;
pub mod rules;
mod split_payload;
pub mod splits;
pub mod storage;

pub use config::{AppConfig, BackendKind, ConfigPaths};
pub use installments::{
    group_into_chains, parse_installment_description, InstallmentChain, InstallmentMarker,
};
pub use models::{
    AccountRecord, AuditEvent, CardSummaryRow, CashflowRow, CategoryRecord, CheckingBalance,
    DailyPulseItem, ForecastRecord, ForecastTemplateRecord, ForecastVsActualRow, MonthlySpendRow,
    RuleRecord, RuntimeMetadata, TransactionRecord, UncategorizedRow,
};
