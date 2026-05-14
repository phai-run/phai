use super::FinanceStore;
use crate::config::AppConfig;
use crate::models::{
    parse_datetime_or_now, AccountRecord, AccountSnapshotRecord, AuditEvent, BudgetStatusRow,
    CardClosedTransactionRow, CardSummaryRow, CashflowRow, CategoryBudgetRecord, CategoryRecord,
    DailyPulseItem, ForecastRecord, ForecastVsActualRow, MonthlySpendRow, RuleRecord,
    TransactionContextRow, TransactionRecord, UncategorizedRow,
};
use crate::splits::{
    ItemPriceRow, ReceiptItemRecord, SplitCandidateRow, TransactionSplitDetail,
    TransactionSplitLineRecord, TransactionSplitRecord,
};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use chrono::{Datelike, Days, NaiveDate, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use rust_decimal::Decimal;
use serde_json::Value;
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

fn parse_sql_json(
    value: String,
    column_index: usize,
) -> std::result::Result<Value, rusqlite::Error> {
    serde_json::from_str(&value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            column_index,
            rusqlite::types::Type::Text,
            Box::new(err),
        )
    })
}

fn split_bigquery_only_error() -> anyhow::Error {
    anyhow::anyhow!("transaction split/detailing is supported only on the BigQuery backend")
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

    async fn insert_account_snapshots(&self, rows: &[AccountSnapshotRecord]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "
            INSERT OR IGNORE INTO account_snapshots (
              snapshot_id, account_id, snapshot_date, balance, credit_limit, currency_code,
              source, actor_id, idempotency_key, metadata_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ",
        )?;
        for row in rows {
            stmt.execute(params![
                row.snapshot_id,
                row.account_id,
                row.snapshot_date.format("%Y-%m-%d").to_string(),
                row.balance.as_ref().map(decimal_to_sql),
                row.credit_limit.as_ref().map(decimal_to_sql),
                row.currency_code,
                row.source,
                row.actor_id,
                row.idempotency_key,
                row.metadata_json.to_string(),
                row.created_at.to_rfc3339(),
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

    async fn apply_transaction_split(
        &self,
        _split: &TransactionSplitRecord,
        _lines: &[TransactionSplitLineRecord],
        _items: &[ReceiptItemRecord],
    ) -> Result<()> {
        Err(split_bigquery_only_error())
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

    async fn find_transactions_by_description(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<TransactionRecord>> {
        let conn = self.connection()?;
        let pattern = format!("%{}%", query.to_ascii_lowercase());
        let mut stmt = conn.prepare(
            "
            SELECT
              transaction_id, account_id, transaction_date, description, CAST(amount AS TEXT),
              tx_type, category_id, category_source, context, payment_status, source,
              actor_id, idempotency_key, metadata_json, created_at, updated_at
            FROM transactions
            WHERE LOWER(description) LIKE ?1
            ORDER BY transaction_date DESC, ABS(CAST(amount AS REAL)) DESC, transaction_id ASC
            LIMIT ?2
            ",
        )?;
        let rows = stmt
            .query_map(params![pattern, limit as i64], |row| {
                let transaction_date = row.get::<_, String>(2)?;
                let amount = row.get::<_, String>(4)?;
                let metadata_json = row.get::<_, String>(13)?;
                let created_at = row.get::<_, String>(14)?;
                let updated_at = row.get::<_, String>(15)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    transaction_date,
                    row.get::<_, String>(3)?,
                    amount,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    metadata_json,
                    created_at,
                    updated_at,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(
                |(
                    transaction_id,
                    account_id,
                    transaction_date,
                    description,
                    amount,
                    tx_type,
                    category_id,
                    category_source,
                    context,
                    payment_status,
                    source,
                    actor_id,
                    idempotency_key,
                    metadata_json,
                    created_at,
                    updated_at,
                )| {
                    Ok(TransactionRecord {
                        transaction_id,
                        account_id,
                        transaction_date: parse_sql_date(transaction_date, 2)
                            .map_err(|e| anyhow::anyhow!("{e}"))?,
                        description,
                        amount: parse_decimal(amount).map_err(|e| anyhow::anyhow!("{e}"))?,
                        tx_type,
                        category_id,
                        category_source,
                        context,
                        payment_status,
                        source,
                        actor_id,
                        idempotency_key,
                        metadata_json: serde_json::from_str(&metadata_json)
                            .unwrap_or_else(|_| serde_json::json!({})),
                        created_at: parse_datetime_or_now(Some(&created_at)),
                        updated_at: parse_datetime_or_now(Some(&updated_at)),
                    })
                },
            )
            .collect()
    }

    async fn latest_uncategorized_transactions(
        &self,
        limit: usize,
    ) -> Result<Vec<TransactionRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              transaction_id, account_id, transaction_date, description, CAST(amount AS TEXT),
              tx_type, category_id, category_source, context, payment_status, source,
              actor_id, idempotency_key, metadata_json, created_at, updated_at
            FROM transactions
            WHERE context IS NULL
            ORDER BY transaction_date DESC, ABS(CAST(amount AS REAL)) DESC, transaction_id ASC
            LIMIT ?1
            ",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                let transaction_date = row.get::<_, String>(2)?;
                let amount = row.get::<_, String>(4)?;
                let metadata_json = row.get::<_, String>(13)?;
                let created_at = row.get::<_, String>(14)?;
                let updated_at = row.get::<_, String>(15)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    transaction_date,
                    row.get::<_, String>(3)?,
                    amount,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    metadata_json,
                    created_at,
                    updated_at,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(
                |(
                    transaction_id,
                    account_id,
                    transaction_date,
                    description,
                    amount,
                    tx_type,
                    category_id,
                    category_source,
                    context,
                    payment_status,
                    source,
                    actor_id,
                    idempotency_key,
                    metadata_json,
                    created_at,
                    updated_at,
                )| {
                    Ok(TransactionRecord {
                        transaction_id,
                        account_id,
                        transaction_date: parse_sql_date(transaction_date, 2)
                            .map_err(|e| anyhow::anyhow!("{e}"))?,
                        description,
                        amount: parse_decimal(amount).map_err(|e| anyhow::anyhow!("{e}"))?,
                        tx_type,
                        category_id,
                        category_source,
                        context,
                        payment_status,
                        source,
                        actor_id,
                        idempotency_key,
                        metadata_json: serde_json::from_str(&metadata_json)
                            .unwrap_or_else(|_| serde_json::json!({})),
                        created_at: parse_datetime_or_now(Some(&created_at)),
                        updated_at: parse_datetime_or_now(Some(&updated_at)),
                    })
                },
            )
            .collect()
    }

    async fn transaction_by_id(&self, transaction_id: &str) -> Result<Option<TransactionRecord>> {
        let conn = self.connection()?;
        let row = conn
            .query_row(
                "
                SELECT
                  transaction_id, account_id, transaction_date, description, CAST(amount AS TEXT),
                  tx_type, category_id, category_source, context, payment_status, source,
                  actor_id, idempotency_key, metadata_json, created_at, updated_at
                FROM transactions
                WHERE transaction_id = ?1
                ",
                [transaction_id],
                |row| {
                    let transaction_date = row.get::<_, String>(2)?;
                    let amount = row.get::<_, String>(4)?;
                    let metadata_json = row.get::<_, String>(13)?;
                    let created_at = row.get::<_, String>(14)?;
                    let updated_at = row.get::<_, String>(15)?;
                    Ok(TransactionRecord {
                        transaction_id: row.get(0)?,
                        account_id: row.get(1)?,
                        transaction_date: parse_sql_date(transaction_date, 2)?,
                        description: row.get(3)?,
                        amount: parse_decimal(amount).map_err(|err| {
                            rusqlite::Error::FromSqlConversionFailure(
                                4,
                                rusqlite::types::Type::Text,
                                Box::new(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    err.to_string(),
                                )),
                            )
                        })?,
                        tx_type: row.get(5)?,
                        category_id: row.get(6)?,
                        category_source: row.get(7)?,
                        context: row.get(8)?,
                        payment_status: row.get(9)?,
                        source: row.get(10)?,
                        actor_id: row.get(11)?,
                        idempotency_key: row.get(12)?,
                        metadata_json: parse_sql_json(metadata_json, 13)?,
                        created_at: parse_datetime_or_now(Some(&created_at)),
                        updated_at: parse_datetime_or_now(Some(&updated_at)),
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    async fn transaction_split_detail(
        &self,
        _transaction_id: &str,
    ) -> Result<Option<TransactionSplitDetail>> {
        Err(split_bigquery_only_error())
    }

    async fn clear_transaction_split(
        &self,
        _transaction_id: &str,
        _actor_id: &str,
        _idempotency_key: &str,
        _reason: Option<&str>,
    ) -> Result<()> {
        Err(split_bigquery_only_error())
    }

    async fn split_candidates(&self, _since: NaiveDate) -> Result<Vec<SplitCandidateRow>> {
        Err(split_bigquery_only_error())
    }

    async fn item_prices(
        &self,
        _query: &str,
        _since: Option<NaiveDate>,
    ) -> Result<Vec<ItemPriceRow>> {
        Err(split_bigquery_only_error())
    }

    async fn all_rules(&self) -> Result<Vec<RuleRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT rule_id, body, status, actor_id, idempotency_key, created_at, updated_at
            FROM rules
            ORDER BY rule_id ASC
            ",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let created_at: String = row.get(5)?;
                let updated_at: String = row.get(6)?;
                Ok(RuleRecord {
                    rule_id: row.get(0)?,
                    body: row.get(1)?,
                    status: row.get(2)?,
                    actor_id: row.get(3)?,
                    idempotency_key: row.get(4)?,
                    created_at: parse_datetime_or_now(Some(&created_at)),
                    updated_at: parse_datetime_or_now(Some(&updated_at)),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
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

    async fn active_rules(&self) -> Result<Vec<RuleRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT rule_id, body, status, actor_id, idempotency_key, created_at, updated_at
            FROM rules
            WHERE LOWER(status) = 'active'
            ORDER BY rule_id ASC
            ",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let created_at: String = row.get(5)?;
                let updated_at: String = row.get(6)?;
                Ok(RuleRecord {
                    rule_id: row.get(0)?,
                    body: row.get(1)?,
                    status: row.get(2)?,
                    actor_id: row.get(3)?,
                    idempotency_key: row.get(4)?,
                    created_at: parse_datetime_or_now(Some(&created_at)),
                    updated_at: parse_datetime_or_now(Some(&updated_at)),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn internal_categories(&self) -> Result<BTreeSet<String>> {
        let conn = self.connection()?;
        let exists = Self::table_exists(&conn, "internal_categories")?;
        if !exists {
            return Ok(BTreeSet::new());
        }
        let mut stmt = conn.prepare(
            "
            SELECT category_id
            FROM internal_categories
            ORDER BY category_id ASC
            ",
        )?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().collect())
    }

    async fn transactions_with_context(&self, limit: usize) -> Result<Vec<TransactionContextRow>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              t.transaction_id,
              t.transaction_date,
              t.display_label,
              CAST(t.amount AS TEXT),
              t.account_id,
              a.label,
              t.category_id,
              t.context,
              t.payment_status,
              t.source
            FROM v_transactions_reportable t
            LEFT JOIN accounts a ON a.account_id = t.account_id
            WHERE t.context IS NOT NULL
              AND TRIM(t.context) <> ''
            ORDER BY t.transaction_date DESC, ABS(t.amount) DESC, t.transaction_id ASC
            LIMIT ?1
            ",
        )?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                let transaction_date = row.get::<_, String>(1)?;
                let amount = row.get::<_, String>(3)?;
                Ok(TransactionContextRow {
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
                    account_label: row.get(5)?,
                    category_id: row.get(6)?,
                    context: row.get(7)?,
                    payment_status: row.get(8)?,
                    source: row.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn count_transactions_with_context(&self) -> Result<i64> {
        let conn = self.connection()?;
        let count = conn.query_row(
            "SELECT COUNT(*) FROM v_transactions_reportable
             WHERE context IS NOT NULL
               AND TRIM(context) <> ''",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
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

    async fn effective_transactions_window(
        &self,
        since: NaiveDate,
        until: NaiveDate,
    ) -> Result<Vec<TransactionRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              transaction_id, account_id, transaction_date, description, CAST(amount AS TEXT),
              tx_type, category_id, category_source, context, payment_status, source,
              actor_id, idempotency_key, metadata_json, created_at, updated_at
            FROM v_transactions_reportable
            WHERE transaction_date >= ?1
              AND transaction_date <= ?2
            ORDER BY transaction_date DESC, ABS(CAST(amount AS REAL)) DESC, transaction_id ASC
            ",
        )?;
        let rows = stmt
            .query_map(
                [
                    since.format("%Y-%m-%d").to_string(),
                    until.format("%Y-%m-%d").to_string(),
                ],
                |row| {
                    let transaction_date = row.get::<_, String>(2)?;
                    let amount = row.get::<_, String>(4)?;
                    let metadata_json = row.get::<_, String>(13)?;
                    let created_at = row.get::<_, String>(14)?;
                    let updated_at = row.get::<_, String>(15)?;
                    Ok(TransactionRecord {
                        transaction_id: row.get(0)?,
                        account_id: row.get(1)?,
                        transaction_date: parse_sql_date(transaction_date, 2)?,
                        description: row.get(3)?,
                        amount: parse_decimal(amount).map_err(|err| {
                            rusqlite::Error::FromSqlConversionFailure(
                                4,
                                rusqlite::types::Type::Text,
                                Box::new(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    err.to_string(),
                                )),
                            )
                        })?,
                        tx_type: row.get(5)?,
                        category_id: row.get(6)?,
                        category_source: row.get(7)?,
                        context: row.get(8)?,
                        payment_status: row.get(9)?,
                        source: row.get(10)?,
                        actor_id: row.get(11)?,
                        idempotency_key: row.get(12)?,
                        metadata_json: parse_sql_json(metadata_json, 13)?,
                        created_at: parse_datetime_or_now(Some(&created_at)),
                        updated_at: parse_datetime_or_now(Some(&updated_at)),
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn transactions_in_date_range(
        &self,
        account_id: Option<&str>,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<TransactionRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              transaction_id, account_id, transaction_date, description, CAST(amount AS TEXT),
              tx_type, category_id, category_source, context, payment_status, source,
              actor_id, idempotency_key, metadata_json, created_at, updated_at
            FROM transactions
            WHERE transaction_date >= ?1
              AND transaction_date <= ?2
              AND (?3 IS NULL OR account_id = ?3)
            ORDER BY transaction_date ASC, transaction_id ASC
            ",
        )?;
        let rows = stmt
            .query_map(
                rusqlite::params![
                    from.format("%Y-%m-%d").to_string(),
                    to.format("%Y-%m-%d").to_string(),
                    account_id,
                ],
                |row| {
                    let transaction_date = row.get::<_, String>(2)?;
                    let amount = row.get::<_, String>(4)?;
                    let metadata_json = row.get::<_, String>(13)?;
                    let created_at = row.get::<_, String>(14)?;
                    let updated_at = row.get::<_, String>(15)?;
                    Ok(TransactionRecord {
                        transaction_id: row.get(0)?,
                        account_id: row.get(1)?,
                        transaction_date: parse_sql_date(transaction_date, 2)?,
                        description: row.get(3)?,
                        amount: parse_decimal(amount).map_err(|err| {
                            rusqlite::Error::FromSqlConversionFailure(
                                4,
                                rusqlite::types::Type::Text,
                                Box::new(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    err.to_string(),
                                )),
                            )
                        })?,
                        tx_type: row.get(5)?,
                        category_id: row.get(6)?,
                        category_source: row.get(7)?,
                        context: row.get(8)?,
                        payment_status: row.get(9)?,
                        source: row.get(10)?,
                        actor_id: row.get(11)?,
                        idempotency_key: row.get(12)?,
                        metadata_json: parse_sql_json(metadata_json, 13)?,
                        created_at: parse_datetime_or_now(Some(&created_at)),
                        updated_at: parse_datetime_or_now(Some(&updated_at)),
                    })
                },
            )?
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

    async fn card_closed_transactions(
        &self,
        month_ref: Option<&str>,
    ) -> Result<Vec<CardClosedTransactionRow>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              strftime('%Y-%m', t.transaction_date) AS month_ref,
              t.account_id,
              t.transaction_id,
              t.transaction_date,
              t.display_label,
              t.description,
              CAST(ABS(CAST(t.amount AS REAL)) AS TEXT) AS closed_amount,
              t.category_id,
              t.payment_status,
              t.metadata_json
            FROM v_transactions_reportable t
            JOIN accounts a ON a.account_id = t.account_id
            WHERE a.account_type = 'credit'
              AND CAST(t.amount AS REAL) < 0
              AND t.payment_status NOT IN ('pending', 'em_aberto', 'parcial')
              AND COALESCE(t.category_id, '') NOT IN (SELECT category_id FROM internal_categories)
              AND (?1 IS NULL OR strftime('%Y-%m', t.transaction_date) = ?1)
            ORDER BY t.transaction_date DESC, ABS(CAST(t.amount AS REAL)) DESC, t.transaction_id ASC
            ",
        )?;
        let rows = stmt
            .query_map([month_ref], |row| {
                let transaction_date = row.get::<_, String>(3)?;
                let amount = row.get::<_, String>(6)?;
                let metadata_json = row.get::<_, String>(9)?;
                Ok(CardClosedTransactionRow {
                    month_ref: row.get(0)?,
                    account_id: row.get(1)?,
                    transaction_id: row.get(2)?,
                    transaction_date: parse_sql_date(transaction_date, 3)?,
                    label: row.get(4)?,
                    description: row.get(5)?,
                    amount: parse_decimal(amount).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    category_id: row.get(7)?,
                    payment_status: row.get(8)?,
                    metadata_json: parse_sql_json(metadata_json, 9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn card_reportable_transactions(
        &self,
        month_ref: Option<&str>,
    ) -> Result<Vec<CardClosedTransactionRow>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              strftime('%Y-%m', t.transaction_date) AS month_ref,
              t.account_id,
              t.transaction_id,
              t.transaction_date,
              t.display_label,
              t.description,
              CAST(ABS(CAST(t.amount AS REAL)) AS TEXT) AS amount,
              t.category_id,
              t.payment_status,
              t.metadata_json
            FROM v_transactions_reportable t
            JOIN accounts a ON a.account_id = t.account_id
            WHERE a.account_type = 'credit'
              AND CAST(t.amount AS REAL) < 0
              AND COALESCE(t.category_id, '') NOT IN (SELECT category_id FROM internal_categories)
              AND (?1 IS NULL OR strftime('%Y-%m', t.transaction_date) = ?1)
            ORDER BY t.transaction_date DESC, ABS(CAST(t.amount AS REAL)) DESC, t.transaction_id ASC
            ",
        )?;
        let rows = stmt
            .query_map([month_ref], |row| {
                let transaction_date = row.get::<_, String>(3)?;
                let amount = row.get::<_, String>(6)?;
                let metadata_json = row.get::<_, String>(9)?;
                Ok(CardClosedTransactionRow {
                    month_ref: row.get(0)?,
                    account_id: row.get(1)?,
                    transaction_id: row.get(2)?,
                    transaction_date: parse_sql_date(transaction_date, 3)?,
                    label: row.get(4)?,
                    description: row.get(5)?,
                    amount: parse_decimal(amount).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    category_id: row.get(7)?,
                    payment_status: row.get(8)?,
                    metadata_json: parse_sql_json(metadata_json, 9)?,
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
              t.transaction_id,
              t.transaction_date,
              t.display_label,
              CAST(t.amount AS TEXT),
              t.account_id,
              a.label,
              t.tx_type,
              t.category_source,
              t.payment_status,
              t.source,
              t.metadata_json
            FROM v_transactions_reportable t
            LEFT JOIN accounts a ON a.account_id = t.account_id
            WHERE t.category_id IS NULL
               OR t.category_source IN ('unclassified', 'fallback')
            ORDER BY t.transaction_date DESC, ABS(t.amount) DESC, t.transaction_id ASC
            LIMIT ?1
            ",
        )?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                let transaction_date = row.get::<_, String>(1)?;
                let amount = row.get::<_, String>(3)?;
                let metadata_json = row.get::<_, String>(10)?;
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
                    account_label: row.get(5)?,
                    tx_type: row.get(6)?,
                    category_source: row.get(7)?,
                    payment_status: row.get(8)?,
                    source: row.get(9)?,
                    metadata_json: parse_sql_json(metadata_json, 10)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn count_uncategorized(&self) -> Result<i64> {
        let conn = self.connection()?;
        let count = conn.query_row(
            "SELECT COUNT(*) FROM v_transactions_reportable
             WHERE category_id IS NULL
                OR category_source IN ('unclassified', 'fallback')",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    async fn count_rows(&self, table: &str) -> Result<i64> {
        super::validate_table_name(table)?;
        let conn = self.connection()?;
        let sql = format!("SELECT COUNT(*) FROM [{table}]");
        let count = conn.query_row(&sql, [], |row| row.get(0))?;
        Ok(count)
    }

    async fn upsert_category_budget(&self, record: &CategoryBudgetRecord) -> Result<()> {
        let conn = self.connection()?;
        conn.execute(
            "
            INSERT INTO category_budgets (
              budget_id, category_id, subcategory_id, month_ref, amount,
              alert_threshold_pct, actor_id, idempotency_key, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(category_id, COALESCE(subcategory_id, ''), COALESCE(month_ref, '_default')) DO UPDATE SET
              budget_id = excluded.budget_id,
              amount = excluded.amount,
              alert_threshold_pct = excluded.alert_threshold_pct,
              actor_id = excluded.actor_id,
              idempotency_key = excluded.idempotency_key,
              updated_at = excluded.updated_at
            ",
            params![
                record.budget_id,
                record.category_id,
                record.subcategory_id,
                record.month_ref,
                decimal_to_sql(&record.amount),
                record.alert_threshold_pct,
                record.actor_id,
                record.idempotency_key,
                record.created_at.to_rfc3339(),
                record.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    async fn list_category_budgets(
        &self,
        month: Option<&str>,
    ) -> Result<Vec<CategoryBudgetRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "
            SELECT
              budget_id, category_id, subcategory_id, month_ref,
              CAST(amount AS TEXT), alert_threshold_pct,
              actor_id, idempotency_key, created_at, updated_at
            FROM category_budgets
            WHERE (?1 IS NULL OR month_ref = ?1 OR month_ref IS NULL)
            ORDER BY category_id ASC, subcategory_id ASC, month_ref ASC
            ",
        )?;
        let rows = stmt
            .query_map([month], |row| {
                let amount = row.get::<_, String>(4)?;
                let created_at: String = row.get(8)?;
                let updated_at: String = row.get(9)?;
                Ok(CategoryBudgetRecord {
                    budget_id: row.get(0)?,
                    category_id: row.get(1)?,
                    subcategory_id: row.get(2)?,
                    month_ref: row.get(3)?,
                    amount: parse_decimal(amount).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    alert_threshold_pct: row.get(5)?,
                    actor_id: row.get(6)?,
                    idempotency_key: row.get(7)?,
                    created_at: parse_datetime_or_now(Some(&created_at)),
                    updated_at: parse_datetime_or_now(Some(&updated_at)),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn budget_status_for_month(&self, month: &str) -> Result<Vec<BudgetStatusRow>> {
        let conn = self.connection()?;
        // Fetch spend for this month aggregated by category (no subcategory split in v_monthly_spend)
        let mut spend_stmt = conn.prepare(
            "
            SELECT category_id, CAST(SUM(CAST(expenses AS REAL)) AS TEXT)
            FROM v_monthly_spend
            WHERE month_ref = ?1
            GROUP BY category_id
            ",
        )?;
        let mut spend_by_cat = std::collections::BTreeMap::<String, Decimal>::new();
        let spend_rows = spend_stmt
            .query_map([month], |row| {
                let cat: String = row.get(0)?;
                let expenses: String = row.get(1)?;
                Ok((cat, expenses))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (cat, expenses_str) in spend_rows {
            let expenses = parse_decimal(expenses_str).unwrap_or(Decimal::ZERO);
            spend_by_cat.insert(cat, expenses);
        }

        // Fetch budgets: explicit month wins over default
        let mut budget_stmt = conn.prepare(
            "
            SELECT
              budget_id, category_id, subcategory_id, month_ref,
              CAST(amount AS TEXT), alert_threshold_pct,
              actor_id, idempotency_key, created_at, updated_at
            FROM category_budgets
            WHERE month_ref = ?1 OR month_ref IS NULL
            ORDER BY category_id ASC, subcategory_id ASC,
                     CASE WHEN month_ref IS NOT NULL THEN 0 ELSE 1 END ASC
            ",
        )?;
        let budget_rows = budget_stmt
            .query_map([month], |row| {
                let amount: String = row.get(4)?;
                let created_at: String = row.get(8)?;
                let updated_at: String = row.get(9)?;
                Ok(CategoryBudgetRecord {
                    budget_id: row.get(0)?,
                    category_id: row.get(1)?,
                    subcategory_id: row.get(2)?,
                    month_ref: row.get(3)?,
                    amount: parse_decimal(amount).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            Box::new(io::Error::new(io::ErrorKind::InvalidData, err.to_string())),
                        )
                    })?,
                    alert_threshold_pct: row.get(5)?,
                    actor_id: row.get(6)?,
                    idempotency_key: row.get(7)?,
                    created_at: parse_datetime_or_now(Some(&created_at)),
                    updated_at: parse_datetime_or_now(Some(&updated_at)),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Dedup: explicit month_ref takes precedence over default (null)
        let mut seen =
            std::collections::BTreeMap::<(String, Option<String>), CategoryBudgetRecord>::new();
        for record in budget_rows {
            let key = (record.category_id.clone(), record.subcategory_id.clone());
            let entry = seen.entry(key);
            match entry {
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(record);
                }
                std::collections::btree_map::Entry::Occupied(mut e) => {
                    // Explicit month_ref wins over default (null)
                    if record.month_ref.is_some() {
                        e.insert(record);
                    }
                }
            }
        }

        // Compute projection factors
        let today = Utc::now().date_naive();
        let current_month = today.format("%Y-%m").to_string();
        let (day_of_month, days_in_month) = if month == current_month {
            let day = today.day();
            // Last day of month: advance to next month day 1, subtract 1
            let first_next = if today.month() == 12 {
                NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
            }
            .unwrap_or(today);
            let last = first_next.checked_sub_days(Days::new(1)).unwrap_or(today);
            (day, last.day())
        } else {
            (0u32, 0u32)
        };

        let mut results = Vec::new();
        for ((cat, _sub), record) in seen {
            let actual = spend_by_cat.get(&cat).copied().unwrap_or(Decimal::ZERO);
            let budget = record.amount;
            let usage_pct = if budget.is_zero() {
                Decimal::ZERO
            } else {
                (actual / budget * Decimal::from(100)).round_dp(2)
            };
            let projected_pct = if month == current_month && day_of_month > 0 {
                let projected = actual / Decimal::from(day_of_month) * Decimal::from(days_in_month);
                if budget.is_zero() {
                    Decimal::ZERO
                } else {
                    (projected / budget * Decimal::from(100)).round_dp(2)
                }
            } else {
                usage_pct
            };
            let alert = usage_pct >= Decimal::from(record.alert_threshold_pct);
            results.push(BudgetStatusRow {
                category_id: cat,
                subcategory_id: record.subcategory_id,
                month_ref: month.to_string(),
                budget_amount: budget,
                actual_amount: actual,
                usage_pct,
                projected_pct,
                alert,
                alert_threshold_pct: record.alert_threshold_pct,
            });
        }
        results.sort_by(|a, b| a.category_id.cmp(&b.category_id));
        Ok(results)
    }
}
