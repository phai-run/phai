use crate::config::{AppConfig, BackendKind};
use crate::storage::FinanceStore;
use anyhow::Result;

type Migration = (&'static str, &'static str);

const SQLITE_MIGRATIONS: [Migration; 26] = [
    (
        "001_initial",
        include_str!("../../../schema/sqlite/001_initial.sql"),
    ),
    (
        "002_add_forecast",
        include_str!("../../../schema/sqlite/002_add_forecast.sql"),
    ),
    (
        "003_create_views",
        include_str!("../../../schema/sqlite/003_create_views.sql"),
    ),
    (
        "004_create_reporting_views",
        include_str!("../../../schema/sqlite/004_create_reporting_views.sql"),
    ),
    (
        "005_refine_reporting_views",
        include_str!("../../../schema/sqlite/005_refine_reporting_views.sql"),
    ),
    (
        "006_harden",
        include_str!("../../../schema/sqlite/006_harden.sql"),
    ),
    (
        "007_fix_credit_card_sign",
        include_str!("../../../schema/sqlite/007_fix_credit_card_sign.sql"),
    ),
    (
        "008_exclude_internal_from_reports",
        include_str!("../../../schema/sqlite/008_exclude_internal_from_reports.sql"),
    ),
    (
        "009_fix_remaining_credit_card_sign",
        include_str!("../../../schema/sqlite/009_fix_remaining_credit_card_sign.sql"),
    ),
    (
        "010_internal_categories_table",
        include_str!("../../../schema/sqlite/010_internal_categories_table.sql"),
    ),
    (
        "011_reclassify_internal_transfers",
        include_str!("../../../schema/sqlite/011_reclassify_internal_transfers.sql"),
    ),
    (
        "012_effective_transactions_view",
        include_str!("../../../schema/sqlite/012_effective_transactions_view.sql"),
    ),
    (
        "013_display_labels_view",
        include_str!("../../../schema/sqlite/013_display_labels_view.sql"),
    ),
    (
        "014_reportable_transactions_view",
        include_str!("../../../schema/sqlite/014_reportable_transactions_view.sql"),
    ),
    (
        "015_update_reportable_legacy_filter",
        include_str!("../../../schema/sqlite/015_update_reportable_legacy_filter.sql"),
    ),
    (
        "016_account_snapshots",
        include_str!("../../../schema/sqlite/016_account_snapshots.sql"),
    ),
    (
        "017_category_budgets",
        include_str!("../../../schema/sqlite/017_category_budgets.sql"),
    ),
    (
        "018_enrichment",
        include_str!("../../../schema/sqlite/018_enrichment.sql"),
    ),
    (
        "019_card_billing_cycle",
        include_str!("../../../schema/sqlite/019_card_billing_cycle.sql"),
    ),
    (
        "020_card_open_now_fix",
        include_str!("../../../schema/sqlite/020_card_open_now_fix.sql"),
    ),
    (
        "021_normalize_payment_status",
        include_str!("../../../schema/sqlite/021_normalize_payment_status.sql"),
    ),
    (
        "022_consolidate_slug_dashes",
        include_str!("../../../schema/sqlite/022_consolidate_slug_dashes.sql"),
    ),
    (
        "023_revisar_fallback",
        include_str!("../../../schema/sqlite/023_revisar_fallback.sql"),
    ),
    (
        "024_streaming_to_assinaturas",
        include_str!("../../../schema/sqlite/024_streaming_to_assinaturas.sql"),
    ),
    (
        "025_cashback_as_expense_reduction",
        include_str!("../../../schema/sqlite/025_cashback_as_expense_reduction.sql"),
    ),
    (
        "026_drop_phantom_account",
        include_str!("../../../schema/sqlite/026_drop_phantom_account.sql"),
    ),
];

const BIGQUERY_MIGRATIONS: [Migration; 27] = [
    (
        "001_initial",
        include_str!("../../../schema/bigquery/001_initial.sql"),
    ),
    (
        "002_add_forecast",
        include_str!("../../../schema/bigquery/002_add_forecast.sql"),
    ),
    (
        "003_create_views",
        include_str!("../../../schema/bigquery/003_create_views.sql"),
    ),
    (
        "004_create_reporting_views",
        include_str!("../../../schema/bigquery/004_create_reporting_views.sql"),
    ),
    (
        "005_refine_reporting_views",
        include_str!("../../../schema/bigquery/005_refine_reporting_views.sql"),
    ),
    (
        "006_harden",
        include_str!("../../../schema/bigquery/006_harden.sql"),
    ),
    (
        "007_fix_credit_card_sign",
        include_str!("../../../schema/bigquery/007_fix_credit_card_sign.sql"),
    ),
    (
        "008_exclude_internal_from_reports",
        include_str!("../../../schema/bigquery/008_exclude_internal_from_reports.sql"),
    ),
    (
        "009_fix_remaining_credit_card_sign",
        include_str!("../../../schema/bigquery/009_fix_remaining_credit_card_sign.sql"),
    ),
    (
        "010_internal_categories_table",
        include_str!("../../../schema/bigquery/010_internal_categories_table.sql"),
    ),
    (
        "011_reclassify_internal_transfers",
        include_str!("../../../schema/bigquery/011_reclassify_internal_transfers.sql"),
    ),
    (
        "012_effective_transactions_view",
        include_str!("../../../schema/bigquery/012_effective_transactions_view.sql"),
    ),
    (
        "013_display_labels_view",
        include_str!("../../../schema/bigquery/013_display_labels_view.sql"),
    ),
    (
        "014_transaction_splits",
        include_str!("../../../schema/bigquery/014_transaction_splits.sql"),
    ),
    (
        "015_reportable_transactions_view",
        include_str!("../../../schema/bigquery/015_reportable_transactions_view.sql"),
    ),
    (
        "016_update_reportable_legacy_filter",
        include_str!("../../../schema/bigquery/016_update_reportable_legacy_filter.sql"),
    ),
    (
        "017_account_snapshots",
        include_str!("../../../schema/bigquery/017_account_snapshots.sql"),
    ),
    (
        "018_category_budgets",
        include_str!("../../../schema/bigquery/018_category_budgets.sql"),
    ),
    (
        "019_enrichment",
        include_str!("../../../schema/bigquery/019_enrichment.sql"),
    ),
    (
        "020_card_billing_cycle",
        include_str!("../../../schema/bigquery/020_card_billing_cycle.sql"),
    ),
    (
        "021_card_open_now_fix",
        include_str!("../../../schema/bigquery/021_card_open_now_fix.sql"),
    ),
    (
        "022_normalize_payment_status",
        include_str!("../../../schema/bigquery/022_normalize_payment_status.sql"),
    ),
    (
        "023_consolidate_slug_dashes",
        include_str!("../../../schema/bigquery/023_consolidate_slug_dashes.sql"),
    ),
    (
        "024_revisar_fallback",
        include_str!("../../../schema/bigquery/024_revisar_fallback.sql"),
    ),
    (
        "025_streaming_to_assinaturas",
        include_str!("../../../schema/bigquery/025_streaming_to_assinaturas.sql"),
    ),
    (
        "026_cashback_as_expense_reduction",
        include_str!("../../../schema/bigquery/026_cashback_as_expense_reduction.sql"),
    ),
    (
        "027_drop_phantom_account",
        include_str!("../../../schema/bigquery/027_drop_phantom_account.sql"),
    ),
];

fn backend_migrations(backend: BackendKind) -> &'static [Migration] {
    match backend {
        BackendKind::Bigquery => &BIGQUERY_MIGRATIONS,
        BackendKind::Local => &SQLITE_MIGRATIONS,
    }
}

fn render_sql(config: &AppConfig, sql: &str) -> Result<String> {
    let mut rendered = sql.to_string();
    if matches!(config.effective_backend(), BackendKind::Bigquery) {
        rendered = rendered.replace("{{project_id}}", config.project_id()?);
        rendered = rendered.replace("{{dataset_id}}", config.dataset_id()?);
    }
    Ok(rendered)
}

pub async fn run_migrations(store: &dyn FinanceStore, config: &AppConfig) -> Result<Vec<String>> {
    let applied = store.applied_migrations().await?;
    let mut executed = Vec::new();

    for (version, raw_sql) in backend_migrations(config.effective_backend()) {
        if applied.contains(*version) {
            continue;
        }
        let sql = render_sql(config, raw_sql)?;
        store.apply_sql(&sql).await?;
        store.record_migration(version).await?;
        executed.push((*version).to_string());
    }

    Ok(executed)
}

#[cfg(test)]
mod tests {
    use super::{BIGQUERY_MIGRATIONS, SQLITE_MIGRATIONS};

    #[test]
    fn bigquery_migrations_include_transaction_splits() {
        assert!(BIGQUERY_MIGRATIONS
            .iter()
            .any(|(version, _)| *version == "014_transaction_splits"));
    }

    #[test]
    fn sqlite_migrations_do_not_include_transaction_splits() {
        assert!(SQLITE_MIGRATIONS
            .iter()
            .all(|(version, _)| *version != "014_transaction_splits"));
    }

    #[test]
    fn sqlite_migrations_include_category_budgets() {
        assert!(SQLITE_MIGRATIONS
            .iter()
            .any(|(version, _)| *version == "017_category_budgets"));
    }

    #[test]
    fn bigquery_migrations_include_category_budgets() {
        assert!(BIGQUERY_MIGRATIONS
            .iter()
            .any(|(version, _)| *version == "018_category_budgets"));
    }
}
