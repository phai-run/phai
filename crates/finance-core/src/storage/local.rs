use super::FinanceStore;
use crate::config::AppConfig;
use crate::models::{
    AccountRecord, AuditEvent, CardSummaryRow, CashflowRow, CategoryRecord, DailyPulseItem,
    ForecastRecord, ForecastVsActualRow, MonthlySpendRow, RuleRecord, TransactionRecord,
    UncategorizedRow,
};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use rust_decimal::Decimal;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;

pub struct LocalStore {
    db_path: PathBuf,
}

impl LocalStore {
    pub fn new(config: AppConfig) -> Result<Self> {
        let db_path = config
            .local_db_path
            .clone()
            .context("local_db_path não configurado")?;
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Falha ao criar {}", parent.display()))?;
        }
        Ok(Self { db_path })
    }

    fn connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("Falha ao abrir {}", self.db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .context("Falha ao configurar PRAGMAs do SQLite")?;
        Ok(conn)
    }

    fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
        let exists = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                [table],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        Ok(exists)
    }
}

fn decimal_to_sql(value: &Decimal) -> String {
    value.round_dp(2).to_string()
}

fn parse_decimal(value: String) -> std::result::Result<Decimal, rust_decimal::Error> {
    Decimal::from_str(&value)
}

fn parse_sql_date(
    value: String,
    column_index: usize,
) -> std::result::Result<NaiveDate, rusqlite::Error> {
    NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            column_index,
            rusqlite::types::Type::Text,
            Box::new(err),
        )
    })
}

fn parse_optional_sql_date(
    value: Option<String>,
    column_index: usize,
) -> std::result::Result<Option<NaiveDate>, rusqlite::Error> {
    value
        .map(|raw| parse_sql_date(raw, column_index))
        .transpose()
}

#[async_trait(?Send)]
impl FinanceStore for LocalStore {
    async fn applied_migrations(&self) -> Result<BTreeSet<String>> {
        let conn = self.connection()?;
        if !Self::table_exists(&conn, "schema_versions")? {
            return Ok(BTreeSet::new());
        }
        let mut stmt = conn.prepare("SELECT version FROM schema_versions ORDER BY version")?;
        let versions = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(versions.into_iter().collect())
    }

    async fn apply_sql(&self, sql: &str) -> Result<()> {
        let conn = self.connection()?;
        conn.execute_batch(sql).context("Falha ao aplicar SQL")?;
        Ok(())
    }

    async fn record_migration(&self, version: &str) -> Result<()> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT OR REPLACE INTO schema_versions (version, applied_at) VALUES (?1, ?2)",
            params![version, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    async fn upsert_accounts(&self, rows: &[AccountRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "
            INSERT INTO accounts (
              account_id, owner, account_type, bank, label, pluggy_account_id, pluggy_item_id,
              status, actor_id, idempotency_key, metadata_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(account_id) DO UPDATE SET
              owner = excluded.owner,
              account_type = excluded.account_type,
              bank = excluded.bank,
              label = excluded.label,
              pluggy_account_id = excluded.pluggy_account_id,
              pluggy_item_id = excluded.pluggy_item_id,
              status = excluded.status,
              actor_id = excluded.actor_id,
              idempotency_key = excluded.idempotency_key,
              metadata_json = excluded.metadata_json,
              updated_at = excluded.updated_at
            ",
        )?;
        for row in rows {
            stmt.execute(params![
                row.account_id,
                row.owner,
                row.account_type,
                row.bank,
                row.label,
                row.pluggy_account_id,
                row.pluggy_item_id,
                row.status,
                row.actor_id,
                row.idempotency_key,
                row.metadata_json.to_string(),
                row.created_at.to_rfc3339(),
                row.updated_at.to_rfc3339(),
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(rows.len())
    }

    async fn upsert_transactions(&self, rows: &[TransactionRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "
            INSERT INTO transactions (
              transaction_id, account_id, transaction_date, description, amount, tx_type,
              category_id, category_source, context, payment_status, source, actor_id,
              idempotency_key, metadata_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ON CONFLICT(transaction_id) DO UPDATE SET
              account_id = excluded.account_id,
              transaction_date = excluded.transaction_date,
              description = excluded.description,
              amount = excluded.amount,
              tx_type = excluded.tx_type,
              category_id = excluded.category_id,
              category_source = excluded.category_source,
              context = excluded.context,
              payment_status = excluded.payment_status,
              source = excluded.source,
              actor_id = excluded.actor_id,
              idempotency_key = excluded.idempotency_key,
              metadata_json = excluded.metadata_json,
              updated_at = excluded.updated_at
            ",
        )?;
        for row in rows {
            stmt.execute(params![
                row.transaction_id,
                row.account_id,
                row.transaction_date.format("%Y-%m-%d").to_string(),
                row.description,
                decimal_to_sql(&row.amount),
                row.tx_type,
                row.category_id,
                row.category_source,
                row.context,
                row.payment_status,
                row.source,
                row.actor_id,
                row.idempotency_key,
                row.metadata_json.to_string(),
                row.created_at.to_rfc3339(),
                row.updated_at.to_rfc3339(),
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(rows.len())
    }

    async fn upsert_rules(&self, rows: &[RuleRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "
            INSERT INTO rules (
              rule_id, body, status, actor_id, idempotency_key, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(rule_id) DO UPDATE SET
              body = excluded.body,
              status = excluded.status,
              actor_id = excluded.actor_id,
              idempotency_key = excluded.idempotency_key,
              updated_at = excluded.updated_at
            ",
        )?;
        for row in rows {
            stmt.execute(params![
                row.rule_id,
                row.body,
                row.status,
                row.actor_id,
                row.idempotency_key,
                row.created_at.to_rfc3339(),
                row.updated_at.to_rfc3339(),
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(rows.len())
    }

    async fn upsert_categories(&self, rows: &[CategoryRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "
            INSERT INTO categories (
              category_id, name, parent_category_id, metadata_json, actor_id, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(category_id) DO UPDATE SET
              name = excluded.name,
              parent_category_id = excluded.parent_category_id,
              metadata_json = excluded.metadata_json,
              actor_id = excluded.actor_id,
              updated_at = excluded.updated_at
            ",
        )?;
        for row in rows {
            stmt.execute(params![
                row.category_id,
                row.name,
                row.parent_category_id,
                row.metadata_json.to_string(),
                row.actor_id,
                row.updated_at.to_rfc3339(),
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(rows.len())
    }

    async fn upsert_forecasts(&self, rows: &[ForecastRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "
            INSERT INTO forecast (
              forecast_id, due_date, description, amount, category_id, account_id,
              status, recurrence, actor_id, idempotency_key, metadata_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(forecast_id) DO UPDATE SET
              due_date = excluded.due_date,
              description = excluded.description,
              amount = excluded.amount,
              category_id = excluded.category_id,
              account_id = excluded.account_id,
              status = excluded.status,
              recurrence = excluded.recurrence,
              actor_id = excluded.actor_id,
              idempotency_key = excluded.idempotency_key,
              metadata_json = excluded.metadata_json,
              updated_at = excluded.updated_at
            ",
        )?;
        for row in rows {
            stmt.execute(params![
                row.forecast_id,
                row.due_date
                    .map(|value| value.format("%Y-%m-%d").to_string()),
                row.description,
                decimal_to_sql(&row.amount),
                row.category_id,
                row.account_id,
                row.status,
                row.recurrence,
                row.actor_id,
                row.idempotency_key,
                row.metadata_json.to_string(),
                row.created_at.to_rfc3339(),
                row.updated_at.to_rfc3339(),
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(rows.len())
    }

    async fn insert_audit_events(&self, rows: &[AuditEvent]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "
            INSERT OR IGNORE INTO audit_log (
              event_id, entity_type, entity_id, action, actor_id, event_timestamp, idempotency_key, diff_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ",
        )?;
        for row in rows {
            stmt.execute(params![
                row.event_id,
                row.entity_type,
                row.entity_id,
                row.action,
                row.actor_id,
                row.event_timestamp.to_rfc3339(),
                row.idempotency_key,
                row.diff_json.to_string(),
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(rows.len())
    }

    async fn annotate_transaction(
        &self,
        transaction_id: &str,
        category_id: Option<&str>,
        category_source: Option<&str>,
        context: Option<&str>,
        actor_id: &str,
        idempotency_key: &str,
    ) -> Result<()> {
        let conn = self.connection()?;
        let affected = conn.execute(
            "
            UPDATE transactions
            SET category_id = COALESCE(?1, category_id),
                category_source = COALESCE(?2, category_source),
                context = COALESCE(?3, context),
                actor_id = ?4,
                idempotency_key = ?5,
                updated_at = ?6
            WHERE transaction_id = ?7
            ",
            params![
                category_id,
                category_source,
                context,
                actor_id,
                idempotency_key,
                Utc::now().to_rfc3339(),
                transaction_id,
            ],
        )?;
        if affected == 0 {
            bail!("Transação {transaction_id} não encontrada");
        }
        Ok(())
    }

    async fn existing_transaction_ids(&self, ids: &[String]) -> Result<BTreeSet<String>> {
        if ids.is_empty() {
            return Ok(BTreeSet::new());
        }
        let conn = self.connection()?;
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT transaction_id FROM transactions WHERE transaction_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(ids.iter()), |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().collect())
    }

    async fn latest_pluggy_transaction_date(&self) -> Result<Option<NaiveDate>> {
        let conn = self.connection()?;
        let value = conn.query_row(
            "SELECT MAX(transaction_date) FROM transactions WHERE source = 'pluggy'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )?;
        value
            .map(|raw| parse_sql_date(raw, 0))
            .transpose()
            .context("Falha ao ler última data Pluggy no SQLite")
    }

    async fn daily_pulse(&self, since: NaiveDate) -> Result<Vec<DailyPulseItem>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT transaction_id, transaction_date, description, CAST(amount AS TEXT), category_id, source, payment_status, account_id
            FROM v_daily_pulse
            WHERE transaction_date >= ?1
            ORDER BY transaction_date DESC, amount ASC, transaction_id ASC
            ",
        )?;
        let rows = stmt
            .query_map([since.format("%Y-%m-%d").to_string()], |row| {
                let transaction_date = row.get::<_, String>(1)?;
                let amount = row.get::<_, String>(3)?;
                Ok(DailyPulseItem {
                    transaction_id: row.get(0)?,
                    transaction_date: NaiveDate::parse_from_str(&transaction_date, "%Y-%m-%d")
                        .map_err(|err| {
                            rusqlite::Error::FromSqlConversionFailure(
                                1,
                                rusqlite::types::Type::Text,
                                Box::new(err),
                            )
                        })?,
                    description: row.get(2)?,
                    amount: parse_decimal(amount).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    category_id: row.get(4)?,
                    source: row.get(5)?,
                    payment_status: row.get(6)?,
                    account_id: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn monthly_spend(&self, month_ref: Option<&str>) -> Result<Vec<MonthlySpendRow>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT month_ref, category_id, account_id, CAST(expenses AS TEXT), expense_count
            FROM v_monthly_spend
            WHERE (?1 IS NULL OR month_ref = ?1)
            ORDER BY month_ref DESC, expenses DESC, category_id ASC, account_id ASC
            ",
        )?;
        let rows = stmt
            .query_map([month_ref], |row| {
                let expenses = row.get::<_, String>(3)?;
                Ok(MonthlySpendRow {
                    month_ref: row.get(0)?,
                    category_id: row.get(1)?,
                    account_id: row.get(2)?,
                    expenses: parse_decimal(expenses).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    expense_count: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn cashflow(&self, months: usize) -> Result<Vec<CashflowRow>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT month_ref, CAST(income AS TEXT), CAST(expenses AS TEXT), CAST(net AS TEXT)
            FROM v_cashflow
            ORDER BY month_ref DESC
            LIMIT ?1
            ",
        )?;
        let rows = stmt
            .query_map([months as i64], |row| {
                let income = row.get::<_, String>(1)?;
                let expenses = row.get::<_, String>(2)?;
                let net = row.get::<_, String>(3)?;
                Ok(CashflowRow {
                    month_ref: row.get(0)?,
                    income: parse_decimal(income).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    expenses: parse_decimal(expenses).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    net: parse_decimal(net).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn forecast_vs_actual(
        &self,
        month_ref: Option<&str>,
    ) -> Result<Vec<ForecastVsActualRow>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              forecast_id,
              month_ref,
              due_date,
              description,
              account_id,
              category_id,
              CAST(forecast_amount AS TEXT),
              CAST(actual_amount AS TEXT),
              CAST(variance AS TEXT),
              status
            FROM v_forecast_vs_actual
            WHERE (?1 IS NULL OR month_ref = ?1)
            ORDER BY month_ref DESC, due_date ASC, description ASC
            ",
        )?;
        let rows = stmt
            .query_map([month_ref], |row| {
                let forecast_amount = row.get::<_, String>(6)?;
                let actual_amount = row.get::<_, String>(7)?;
                let variance = row.get::<_, String>(8)?;
                Ok(ForecastVsActualRow {
                    forecast_id: row.get(0)?,
                    month_ref: row.get(1)?,
                    due_date: parse_optional_sql_date(row.get(2)?, 2)?,
                    description: row.get(3)?,
                    account_id: row.get(4)?,
                    category_id: row.get(5)?,
                    forecast_amount: parse_decimal(forecast_amount).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    actual_amount: parse_decimal(actual_amount).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    variance: parse_decimal(variance).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            8,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    status: row.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn card_summary(&self, month_ref: Option<&str>) -> Result<Vec<CardSummaryRow>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              month_ref,
              account_id,
              CAST(total_charges AS TEXT),
              CAST(open_amount AS TEXT),
              transaction_count
            FROM v_card_summary
            WHERE (?1 IS NULL OR month_ref = ?1)
            ORDER BY month_ref DESC, total_charges DESC, account_id ASC
            ",
        )?;
        let rows = stmt
            .query_map([month_ref], |row| {
                let total_charges = row.get::<_, String>(2)?;
                let open_amount = row.get::<_, String>(3)?;
                Ok(CardSummaryRow {
                    month_ref: row.get(0)?,
                    account_id: row.get(1)?,
                    total_charges: parse_decimal(total_charges).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    open_amount: parse_decimal(open_amount).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    transaction_count: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn uncategorized(&self, limit: usize) -> Result<Vec<UncategorizedRow>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              transaction_id,
              transaction_date,
              description,
              CAST(amount AS TEXT),
              account_id,
              category_source,
              payment_status,
              source
            FROM v_uncategorized
            ORDER BY transaction_date DESC, ABS(amount) DESC, transaction_id ASC
            LIMIT ?1
            ",
        )?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                let transaction_date = row.get::<_, String>(1)?;
                let amount = row.get::<_, String>(3)?;
                Ok(UncategorizedRow {
                    transaction_id: row.get(0)?,
                    transaction_date: parse_sql_date(transaction_date, 1)?,
                    description: row.get(2)?,
                    amount: parse_decimal(amount).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    account_id: row.get(4)?,
                    category_source: row.get(5)?,
                    payment_status: row.get(6)?,
                    source: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn count_rows(&self, table: &str) -> Result<i64> {
        super::validate_table_name(table)?;
        let conn = self.connection()?;
        let sql = format!("SELECT COUNT(*) FROM [{table}]");
        let count = conn.query_row(&sql, [], |row| row.get(0))?;
        Ok(count)
    }
}
