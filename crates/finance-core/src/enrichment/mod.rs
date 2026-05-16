//! Phase 1 of the transaction-enrichment pipeline.
//!
//! Layout:
//!   - [`types`]: shared structs (`CnpjInfo`, `CategoryHint`, `ContextTx`,
//!     `Heuristics`, `HourBucket`).
//!   - [`cnpj`]: extraction, normalization, BrasilAPI lookup, 2-layer
//!     cache, CNAE → category mapping.
//!   - [`pluggy_map`]: coarse Pluggy category → internal category hint.
//!   - [`context`]: temporal-context helpers.
//!   - [`heuristics`]: cheap pre-LLM features.

pub mod cnpj;
pub mod context;
pub mod heuristics;
pub mod pluggy_map;
pub mod types;

pub use types::{CategoryHint, CnpjInfo, ContextTx, HourBucket, Heuristics};
