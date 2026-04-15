use crate::config::{AppConfig, BackendKind};
use crate::storage::FinanceStore;
use anyhow::Result;

type Migration = (&'static str, &'static str);

const SQLITE_MIGRATIONS: [Migration; 11] = [
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
];

const BIGQUERY_MIGRATIONS: [Migration; 11] = [
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
