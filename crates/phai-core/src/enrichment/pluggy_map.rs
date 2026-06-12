//! Pluggy coarse category -> internal category hint.
//!
//! Pluggy returns categories like "Eating out", "Groceries", "Pharmacy",
//! "Transfers". These map to our internal `categoria:subcategoria` pairs
//! with a confidence boost reflecting how specific the mapping is.
//!
//! Mappings cover the 20 categories that have appeared in real Pluggy
//! payloads observed in production (≥0.5% of categorized transactions).
//! Unknown labels return [`CategoryHint::empty`].

use super::types::CategoryHint;

/// Map a Pluggy coarse category string (case-sensitive — Pluggy is
/// consistent) to a [`CategoryHint`]. Unknown labels return
/// [`CategoryHint::empty`].
pub fn map_pluggy_category(cat: &str) -> CategoryHint {
    match cat {
        // ── alimentação ───────────────────────────────────────────────
        "Eating out" => hint("alimentacao", Some("restaurantes"), 0.10),
        "Groceries" => hint("alimentacao", Some("mercado"), 0.05),
        "Food and beverages" => hint("alimentacao", None, 0.05),

        // ── saúde ─────────────────────────────────────────────────────
        "Pharmacy" => hint("saude", Some("farmacia"), 0.15),
        "Health" => hint("saude", None, 0.08),
        "Healthcare" => hint("saude", None, 0.08),

        // ── transporte ────────────────────────────────────────────────
        "Gas stations" => hint("transporte", Some("combustivel"), 0.15),
        "Transportation" => hint("transporte", None, 0.05),
        "Ride-sharing" => hint("transporte", Some("taxi-app"), 0.12),
        "Public transportation" => hint("transporte", Some("transporte-publico"), 0.10),

        // ── educação ──────────────────────────────────────────────────
        "Education" => hint("educacao", None, 0.08),

        // ── lazer / entretenimento ────────────────────────────────────
        "Entertainment" => hint("lazer", None, 0.05),
        "Sports and fitness" => hint("pessoal", Some("cuidado-fisico"), 0.08),
        "Travel" => hint("lazer", Some("viagem"), 0.10),

        // ── moradia / utilities ───────────────────────────────────────
        "Bills and utilities" => hint("moradia", Some("contas"), 0.05),
        "Rent" => hint("moradia", Some("aluguel"), 0.15),

        // ── pessoal ───────────────────────────────────────────────────
        "Personal care" => hint("pessoal", Some("cuidado-fisico"), 0.08),
        "Shopping" => hint("compras", None, 0.05),

        // ── neutros: nunca usar sozinhos ──────────────────────────────
        "Transfers" => CategoryHint::empty(),
        "Income" => CategoryHint::empty(),

        // ── unmatched ─────────────────────────────────────────────────
        _ => CategoryHint::empty(),
    }
}

const fn hint(
    category: &'static str,
    subcategory: Option<&'static str>,
    boost: f32,
) -> CategoryHint {
    CategoryHint {
        category: Some(category),
        subcategory,
        confidence_boost: boost,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pluggy_map_eating_out() {
        let h = map_pluggy_category("Eating out");
        assert_eq!(h.category, Some("alimentacao"));
        assert_eq!(h.subcategory, Some("restaurantes"));
        assert!((h.confidence_boost - 0.10).abs() < 1e-6);
    }

    #[test]
    fn test_pluggy_map_pharmacy() {
        let h = map_pluggy_category("Pharmacy");
        assert_eq!(h.category, Some("saude"));
        assert_eq!(h.subcategory, Some("farmacia"));
        assert!((h.confidence_boost - 0.15).abs() < 1e-6);
    }

    #[test]
    fn test_pluggy_map_transfers_no_boost() {
        let h = map_pluggy_category("Transfers");
        assert!(h.category.is_none());
        assert!(h.subcategory.is_none());
        assert_eq!(h.confidence_boost, 0.0);
    }

    #[test]
    fn test_pluggy_map_income_no_boost() {
        let h = map_pluggy_category("Income");
        assert!(h.category.is_none());
        assert_eq!(h.confidence_boost, 0.0);
    }

    #[test]
    fn test_pluggy_map_unknown() {
        let h = map_pluggy_category("Random Junk That Is Not A Category");
        assert!(h.category.is_none());
        assert!(h.subcategory.is_none());
        assert_eq!(h.confidence_boost, 0.0);
    }

    #[test]
    fn test_pluggy_map_education_subcategory_none() {
        let h = map_pluggy_category("Education");
        assert_eq!(h.category, Some("educacao"));
        assert!(h.subcategory.is_none());
    }

    #[test]
    fn test_pluggy_map_uses_llm_taxonomy_subcategories() {
        let ride = map_pluggy_category("Ride-sharing");
        assert_eq!(ride.category, Some("transporte"));
        assert_eq!(ride.subcategory, Some("taxi-app"));

        let public = map_pluggy_category("Public transportation");
        assert_eq!(public.category, Some("transporte"));
        assert_eq!(public.subcategory, Some("transporte-publico"));

        let fitness = map_pluggy_category("Sports and fitness");
        assert_eq!(fitness.category, Some("pessoal"));
        assert_eq!(fitness.subcategory, Some("cuidado-fisico"));
    }
}
