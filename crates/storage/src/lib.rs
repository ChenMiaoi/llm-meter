use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use chrono::{DateTime, Utc};
use llm_meter_core::*;
use rusqlite::{Connection as Sqlite, OptionalExtension, Transaction, params};
use rust_decimal::Decimal;
use uuid::Uuid;

const MIGRATION_1: &str = include_str!("../migrations/0001_initial.sql");
const MIGRATION_2: &str = include_str!("../migrations/0002_reset_credits.sql");
type BudgetRow = (String, String, String, String, String, String, String, i64);

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid stored uuid: {0}")]
    Uuid(#[from] uuid::Error),
    #[error("invalid stored timestamp: {0}")]
    Time(#[from] chrono::ParseError),
    #[error("invalid decimal: {0}")]
    Decimal(#[from] rust_decimal::Error),
    #[error("invalid domain value: {0}")]
    Domain(#[from] DomainError),
    #[error("sync batch contains records for another connection")]
    CrossConnectionBatch,
    #[error("repository lock poisoned")]
    Poisoned,
}

pub struct Repository {
    db: Mutex<Sqlite>,
}

impl Repository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db = Sqlite::open(path)?;
        Self::configure(&db)?;
        let repo = Self { db: Mutex::new(db) };
        repo.migrate()?;
        Ok(repo)
    }

    pub fn in_memory() -> Result<Self, StorageError> {
        let db = Sqlite::open_in_memory()?;
        Self::configure(&db)?;
        let repo = Self { db: Mutex::new(db) };
        repo.migrate()?;
        Ok(repo)
    }

    fn db(&self) -> Result<MutexGuard<'_, Sqlite>, StorageError> {
        self.db.lock().map_err(|_| StorageError::Poisoned)
    }

    fn configure(db: &Sqlite) -> Result<(), StorageError> {
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        db.pragma_update(None, "busy_timeout", 5000)?;
        Ok(())
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let mut db = self.db()?;
        let tx = db.transaction()?;
        tx.execute_batch(MIGRATION_1)?;
        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(1, ?1)",
            [Utc::now().to_rfc3339()],
        )?;
        tx.execute_batch(MIGRATION_2)?;
        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(2, ?1)",
            [Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32, StorageError> {
        Ok(self.db()?.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get::<_, u32>(0),
        )?)
    }

    pub fn upsert_provider(&self, manifest: &ProviderManifest) -> Result<(), StorageError> {
        self.db()?.execute("INSERT INTO providers(id,display_name,adapter_version) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name,adapter_version=excluded.adapter_version", params![manifest.provider_id, manifest.display_name, manifest.adapter_version])?;
        Ok(())
    }

    pub fn insert_credential_ref(&self, r: &CredentialRef) -> Result<(), StorageError> {
        self.db()?.execute("INSERT INTO credential_refs(id,backend,service_name,secret_key,created_at) VALUES(?1,?2,?3,?4,?5)", params![r.id.to_string(), r.backend, r.service_name, r.secret_key, r.created_at.to_rfc3339()])?;
        Ok(())
    }

    pub fn add_connection(&self, c: &Connection) -> Result<(), StorageError> {
        self.db()?.execute("INSERT INTO connections(id,provider_id,connection_type,display_name,account_external_id,status,credential_ref_id,created_at,updated_at,last_success_at,last_error_code,disabled_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)", params![c.id.to_string(),c.provider_id,c.connection_type,c.display_name,c.account_external_id,js(&c.status)?,c.credential_ref_id.map(|v|v.to_string()),c.created_at.to_rfc3339(),c.updated_at.to_rfc3339(),c.last_success_at.map(|v|v.to_rfc3339()),c.last_error_code,c.disabled_at.map(|v|v.to_rfc3339())])?;
        Ok(())
    }

    pub fn add_authenticated_connection(
        &self,
        c: &Connection,
        credential: Option<&CredentialRef>,
    ) -> Result<(), StorageError> {
        let mut db = self.db()?;
        let tx = db.transaction()?;
        if let Some(r) = credential {
            tx.execute("INSERT INTO credential_refs(id,backend,service_name,secret_key,created_at) VALUES(?1,?2,?3,?4,?5)",params![r.id.to_string(),r.backend,r.service_name,r.secret_key,r.created_at.to_rfc3339()])?;
        }
        tx.execute("INSERT INTO connections(id,provider_id,connection_type,display_name,account_external_id,status,credential_ref_id,created_at,updated_at,last_success_at,last_error_code,disabled_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![c.id.to_string(),c.provider_id,c.connection_type,c.display_name,c.account_external_id,js(&c.status)?,c.credential_ref_id.map(|v|v.to_string()),c.created_at.to_rfc3339(),c.updated_at.to_rfc3339(),c.last_success_at.map(|v|v.to_rfc3339()),c.last_error_code,c.disabled_at.map(|v|v.to_rfc3339())])?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_connections(&self) -> Result<Vec<Connection>, StorageError> {
        let db = self.db()?;
        let mut stmt = db.prepare("SELECT id,provider_id,connection_type,display_name,account_external_id,status,credential_ref_id,created_at,updated_at,last_success_at,last_error_code,disabled_at FROM connections ORDER BY created_at")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get(10)?,
                r.get::<_, Option<String>>(11)?,
            ))
        })?;
        rows.map(|row| {
            let r = row?;
            Ok(Connection {
                id: Uuid::parse_str(&r.0)?,
                provider_id: r.1,
                connection_type: r.2,
                display_name: r.3,
                account_external_id: r.4,
                status: serde_json::from_str(&r.5)?,
                credential_ref_id: r.6.map(|v| Uuid::parse_str(&v)).transpose()?,
                created_at: dt(&r.7)?,
                updated_at: dt(&r.8)?,
                last_success_at: r.9.map(|v| dt(&v)).transpose()?,
                last_error_code: r.10,
                disabled_at: r.11.map(|v| dt(&v)).transpose()?,
            })
        })
        .collect()
    }

    pub fn connection(&self, id: Uuid) -> Result<Option<Connection>, StorageError> {
        Ok(self.list_connections()?.into_iter().find(|c| c.id == id))
    }

    pub fn accounts(&self, connection_id: Uuid) -> Result<Vec<AccountRecord>, StorageError> {
        let db = self.db()?;
        let mut statement = db.prepare(
            "SELECT id,external_id,display_name,account_type FROM accounts WHERE connection_id=?1 ORDER BY external_id",
        )?;
        let rows = statement.query_map([connection_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, external_id, display_name, account_type) = row?;
            Ok(AccountRecord {
                id: Uuid::parse_str(&id)?,
                connection_id,
                external_id,
                display_name,
                account_type,
            })
        })
        .collect()
    }

    pub fn products(&self, connection_id: Uuid) -> Result<Vec<ProductRecord>, StorageError> {
        let db = self.db()?;
        let mut statement = db.prepare(
            "SELECT id,product_key,display_name FROM products WHERE connection_id=?1 ORDER BY product_key",
        )?;
        let rows = statement.query_map([connection_id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.map(|row| {
            let (id, product_key, display_name) = row?;
            Ok(ProductRecord {
                id: Uuid::parse_str(&id)?,
                connection_id,
                product_key,
                display_name,
            })
        })
        .collect()
    }

    pub fn set_connection_status(
        &self,
        id: Uuid,
        status: ConnectionStatus,
    ) -> Result<(), StorageError> {
        self.db()?.execute(
            "UPDATE connections SET status=?2,updated_at=?3 WHERE id=?1",
            params![id.to_string(), js(&status)?, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn credential_ref(&self, id: Uuid) -> Result<Option<CredentialRef>, StorageError> {
        let row: Option<(String,String,String,String,String)> = self.db()?.query_row("SELECT id,backend,service_name,secret_key,created_at FROM credential_refs WHERE id=?1", [id.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        row.map(|r| {
            Ok(CredentialRef {
                id: Uuid::parse_str(&r.0)?,
                backend: r.1,
                service_name: r.2,
                secret_key: r.3,
                created_at: dt(&r.4)?,
            })
        })
        .transpose()
    }

    /// Commits all normalized provider output and the cursor atomically.
    pub fn commit_sync_batch(
        &self,
        connection_id: Uuid,
        stream: &str,
        batch: &SyncBatch,
    ) -> Result<(), StorageError> {
        if batch
            .account_updates
            .iter()
            .any(|value| value.connection_id != connection_id)
            || batch
                .product_updates
                .iter()
                .any(|value| value.connection_id != connection_id)
            || batch
                .metric_samples
                .iter()
                .any(|value| value.connection_id != connection_id)
            || batch
                .quota_windows
                .iter()
                .any(|value| value.connection_id != connection_id)
            || batch
                .rate_limit_reset_credits
                .as_ref()
                .is_some_and(|summary| {
                    summary.connection_id != connection_id
                        || summary.credits.as_ref().is_some_and(|credits| {
                            credits
                                .iter()
                                .any(|credit| credit.connection_id != connection_id)
                        })
                })
            || batch
                .capability_snapshot
                .as_ref()
                .is_some_and(|value| value.connection_id != connection_id)
        {
            return Err(StorageError::CrossConnectionBatch);
        }
        for metric in &batch.metric_samples {
            metric.validate()?;
        }
        let normalized_quotas = batch
            .quota_windows
            .iter()
            .cloned()
            .map(QuotaWindow::normalize)
            .collect::<Result<Vec<_>, _>>()?;
        let mut db = self.db()?;
        let tx = db.transaction()?;
        for a in &batch.account_updates {
            tx.execute("INSERT INTO accounts(id,connection_id,external_id,display_name,account_type) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(connection_id,external_id) DO UPDATE SET display_name=excluded.display_name,account_type=excluded.account_type",params![a.id.to_string(),a.connection_id.to_string(),a.external_id,a.display_name,a.account_type])?;
        }
        for p in &batch.product_updates {
            tx.execute("INSERT INTO products(id,connection_id,product_key,display_name) VALUES(?1,?2,?3,?4) ON CONFLICT(connection_id,product_key) DO UPDATE SET display_name=excluded.display_name",params![p.id.to_string(),p.connection_id.to_string(),p.product_key,p.display_name])?;
        }
        if let Some(c) = &batch.capability_snapshot {
            tx.execute("INSERT INTO connection_capabilities(connection_id,flags,observed_at) VALUES(?1,?2,?3) ON CONFLICT(connection_id) DO UPDATE SET flags=excluded.flags,observed_at=excluded.observed_at",params![c.connection_id.to_string(),c.capabilities.bits() as i64,c.observed_at.to_rfc3339()])?;
        }
        for m in &batch.metric_samples {
            insert_metric(&tx, m)?;
        }
        for q in &normalized_quotas {
            insert_quota(&tx, q)?;
        }
        if let Some(summary) = &batch.rate_limit_reset_credits {
            tx.execute(
                "INSERT INTO rate_limit_reset_credit_summaries(connection_id,available_count,details_available,observed_at) VALUES(?1,?2,?3,?4) ON CONFLICT(connection_id) DO UPDATE SET available_count=excluded.available_count,details_available=excluded.details_available,observed_at=excluded.observed_at",
                params![
                    connection_id.to_string(),
                    summary.available_count as i64,
                    summary.credits.is_some() as i64,
                    summary.observed_at.to_rfc3339()
                ],
            )?;
            tx.execute(
                "DELETE FROM rate_limit_reset_credits WHERE connection_id=?1",
                [connection_id.to_string()],
            )?;
            if let Some(credits) = &summary.credits {
                for credit in credits {
                    tx.execute(
                        "INSERT INTO rate_limit_reset_credits(id,connection_id,reset_type,status,granted_at,expires_at,title,description) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                        params![
                            credit.id,
                            connection_id.to_string(),
                            credit.reset_type,
                            credit.status,
                            credit.granted_at.to_rfc3339(),
                            credit.expires_at.map(|value| value.to_rfc3339()),
                            credit.title,
                            credit.description
                        ],
                    )?;
                }
            }
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO sync_states(connection_id,stream_name,cursor,last_attempt_at,last_success_at,error_count) VALUES(?1,?2,?3,?4,?4,0) ON CONFLICT(connection_id,stream_name) DO UPDATE SET cursor=excluded.cursor,last_attempt_at=excluded.last_attempt_at,last_success_at=excluded.last_success_at,next_retry_at=NULL,error_count=0",params![connection_id.to_string(),stream,batch.next_cursor.as_ref().map(|v|&v.0),now])?;
        tx.execute("UPDATE connections SET status='\"ready\"',last_success_at=?2,updated_at=?2,last_error_code=NULL WHERE id=?1",params![connection_id.to_string(),now])?;
        tx.commit()?;
        Ok(())
    }

    pub fn mark_sync_error(
        &self,
        connection_id: Uuid,
        stream: &str,
        status: ConnectionStatus,
        code: &str,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<(), StorageError> {
        let mut db = self.db()?;
        let tx = db.transaction()?;
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO sync_states(connection_id,stream_name,last_attempt_at,next_retry_at,error_count) VALUES(?1,?2,?3,?4,1) ON CONFLICT(connection_id,stream_name) DO UPDATE SET last_attempt_at=excluded.last_attempt_at,next_retry_at=excluded.next_retry_at,error_count=error_count+1",params![connection_id.to_string(),stream,now,retry_at.map(|v|v.to_rfc3339())])?;
        tx.execute(
            "UPDATE connections SET status=?2,last_error_code=?3,updated_at=?4 WHERE id=?1",
            params![connection_id.to_string(), js(&status)?, code, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn sync_cursor(
        &self,
        connection_id: Uuid,
        stream: &str,
    ) -> Result<Option<SyncCursor>, StorageError> {
        Ok(self
            .db()?
            .query_row(
                "SELECT cursor FROM sync_states WHERE connection_id=?1 AND stream_name=?2",
                params![connection_id.to_string(), stream],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .map(SyncCursor))
    }

    pub fn sync_last_attempt(
        &self,
        connection_id: Uuid,
        stream: &str,
    ) -> Result<Option<DateTime<Utc>>, StorageError> {
        let value: Option<String> = self
            .db()?
            .query_row(
                "SELECT last_attempt_at FROM sync_states WHERE connection_id=?1 AND stream_name=?2",
                params![connection_id.to_string(), stream],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(value.map(|v| dt(&v)).transpose()?)
    }

    pub fn sync_retry_state(
        &self,
        connection_id: Uuid,
        stream: &str,
    ) -> Result<(Option<DateTime<Utc>>, u32), StorageError> {
        let value: Option<(Option<String>, i64)> = self
            .db()?
            .query_row(
                "SELECT next_retry_at,error_count FROM sync_states WHERE connection_id=?1 AND stream_name=?2",
                params![connection_id.to_string(), stream],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match value {
            Some((retry, count)) => Ok((retry.map(|v| dt(&v)).transpose()?, count.max(0) as u32)),
            None => Ok((None, 0)),
        }
    }

    pub fn prune(
        &self,
        now: DateTime<Utc>,
        raw_days: i64,
        hourly_days: i64,
        provider_events_days: i64,
    ) -> Result<usize, StorageError> {
        let db = self.db()?;
        let raw = (now - chrono::Duration::days(raw_days)).to_rfc3339();
        let hourly = (now - chrono::Duration::days(hourly_days)).to_rfc3339();
        let events = (now - chrono::Duration::days(provider_events_days)).to_rfc3339();
        let mut deleted = 0;
        deleted+=db.execute("DELETE FROM metric_samples WHERE observed_at < ?1 AND (period_start IS NULL OR period_end IS NULL OR (julianday(period_end)-julianday(period_start))*24.0 < 1.0)",[&raw])?;
        deleted+=db.execute("DELETE FROM metric_samples WHERE observed_at < ?1 AND period_start IS NOT NULL AND period_end IS NOT NULL AND (julianday(period_end)-julianday(period_start))*24.0 >= 1.0 AND (julianday(period_end)-julianday(period_start))*24.0 < 24.0",[&hourly])?;
        deleted += db.execute(
            "DELETE FROM provider_events WHERE observed_at < ?1",
            [&events],
        )?;
        Ok(deleted)
    }

    pub fn remove_connection(&self, id: Uuid) -> Result<Option<CredentialRef>, StorageError> {
        let mut db = self.db()?;
        let tx = db.transaction()?;
        let ref_id: Option<String> = tx
            .query_row(
                "SELECT credential_ref_id FROM connections WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let cref = if let Some(rid) = ref_id {
            let row=tx.query_row("SELECT id,backend,service_name,secret_key,created_at FROM credential_refs WHERE id=?1",[&rid],|r|Ok((r.get::<_,String>(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get::<_,String>(4)?))).optional()?;
            match row {
                Some(r) => Some(CredentialRef {
                    id: Uuid::parse_str(&r.0)?,
                    backend: r.1,
                    service_name: r.2,
                    secret_key: r.3,
                    created_at: dt(&r.4)?,
                }),
                None => None,
            }
        } else {
            None
        };
        tx.execute("DELETE FROM connections WHERE id=?1", [id.to_string()])?;
        if let Some(r) = &cref {
            tx.execute(
                "DELETE FROM credential_refs WHERE id=?1",
                [r.id.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(cref)
    }

    pub fn capabilities(&self, id: Uuid) -> Result<Option<CapabilitySnapshot>, StorageError> {
        let row: Option<(i64, String)> = self
            .db()?
            .query_row(
                "SELECT flags,observed_at FROM connection_capabilities WHERE connection_id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(f, t)| {
            Ok(CapabilitySnapshot {
                connection_id: id,
                capabilities: Capabilities::from_bits_retain(f as u64),
                observed_at: dt(&t)?,
            })
        })
        .transpose()
    }

    pub fn metrics(
        &self,
        id: Uuid,
        key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MetricSample>, StorageError> {
        let db = self.db()?;
        let sql = if key.is_some() {
            "SELECT id,connection_id,metric_key,value,unit,scope,period_start,period_end,observed_at,provenance,dimensions_json,source_metric,dedup_key FROM metric_samples WHERE connection_id=?1 AND metric_key=?2 ORDER BY observed_at DESC LIMIT ?3"
        } else {
            "SELECT id,connection_id,metric_key,value,unit,scope,period_start,period_end,observed_at,provenance,dimensions_json,source_metric,dedup_key FROM metric_samples WHERE connection_id=?1 AND (?2 IS NULL) ORDER BY observed_at DESC LIMIT ?3"
        };
        let mut stmt = db.prepare(sql)?;
        let rows = stmt.query_map(params![id.to_string(), key, limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
                r.get::<_, String>(11)?,
                r.get::<_, String>(12)?,
            ))
        })?;
        rows.map(|x| {
            let r = x?;
            Ok(MetricSample {
                id: Uuid::parse_str(&r.0)?,
                connection_id: Uuid::parse_str(&r.1)?,
                metric_key: MetricKey(r.2),
                value: r.3.parse()?,
                unit: serde_json::from_str(&r.4)?,
                scope: serde_json::from_str(&r.5)?,
                period_start: r.6.map(|v| dt(&v)).transpose()?,
                period_end: r.7.map(|v| dt(&v)).transpose()?,
                observed_at: dt(&r.8)?,
                provenance: serde_json::from_str(&r.9)?,
                dimensions: serde_json::from_str(&r.10)?,
                source_metric: r.11,
                dedup_key: r.12,
            })
        })
        .collect()
    }

    pub fn quotas(&self, id: Uuid) -> Result<Vec<QuotaWindow>, StorageError> {
        let db = self.db()?;
        let mut s=db.prepare("SELECT id,provider_limit_id,display_name,window_kind,window_start,window_end,resets_at,used_ratio,remaining_ratio,used_value,limit_value,unit,provenance,observed_at FROM quota_windows WHERE connection_id=?1 ORDER BY observed_at DESC")?;
        let rows = s.query_map([id.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<String>>(11)?,
                r.get::<_, String>(12)?,
                r.get::<_, String>(13)?,
            ))
        })?;
        rows.map(|x| {
            let r = x?;
            Ok(QuotaWindow {
                id: Uuid::parse_str(&r.0)?,
                connection_id: id,
                provider_limit_id: r.1,
                display_name: r.2,
                window_kind: serde_json::from_str(&r.3)?,
                window_start: r.4.map(|v| dt(&v)).transpose()?,
                window_end: r.5.map(|v| dt(&v)).transpose()?,
                resets_at: r.6.map(|v| dt(&v)).transpose()?,
                used_ratio: dec(r.7)?,
                remaining_ratio: dec(r.8)?,
                used_value: dec(r.9)?,
                limit_value: dec(r.10)?,
                unit: r.11.map(|v| serde_json::from_str(&v)).transpose()?,
                provenance: serde_json::from_str(&r.12)?,
                observed_at: dt(&r.13)?,
            })
        })
        .collect()
    }

    pub fn budget(&self, connection_id: Uuid) -> Result<Option<Budget>, StorageError> {
        let row: Option<BudgetRow> = self
            .db()?
            .query_row(
                "SELECT id,connection_id,amount,currency,period,warning_ratio,critical_ratio,enabled FROM budgets WHERE connection_id=?1 LIMIT 1",
                [connection_id.to_string()],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)),
            )
            .optional()?;
        row.map(|r| {
            Ok(Budget {
                id: Uuid::parse_str(&r.0)?,
                connection_id: Uuid::parse_str(&r.1)?,
                amount: r.2.parse()?,
                currency: r.3,
                period: serde_json::from_str(&r.4)?,
                warning_ratio: r.5.parse()?,
                critical_ratio: r.6.parse()?,
                enabled: r.7 != 0,
            })
        })
        .transpose()
    }

    pub fn rate_limit_reset_credits(
        &self,
        connection_id: Uuid,
    ) -> Result<Option<RateLimitResetCredits>, StorageError> {
        let summary: Option<(i64, i64, String)> = self
            .db()?
            .query_row(
                "SELECT available_count,details_available,observed_at FROM rate_limit_reset_credit_summaries WHERE connection_id=?1",
                [connection_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((available_count, details_available, observed_at)) = summary else {
            return Ok(None);
        };
        let credits = if details_available != 0 {
            let db = self.db()?;
            let mut statement = db.prepare(
                "SELECT id,reset_type,status,granted_at,expires_at,title,description FROM rate_limit_reset_credits WHERE connection_id=?1 ORDER BY CASE WHEN expires_at IS NULL THEN 1 ELSE 0 END, expires_at",
            )?;
            let rows = statement.query_map([connection_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })?;
            Some(
                rows.map(|row| {
                    let row = row?;
                    Ok(RateLimitResetCredit {
                        id: row.0,
                        connection_id,
                        reset_type: row.1,
                        status: row.2,
                        granted_at: dt(&row.3)?,
                        expires_at: row.4.map(|value| dt(&value)).transpose()?,
                        title: row.5,
                        description: row.6,
                    })
                })
                .collect::<Result<Vec<_>, StorageError>>()?,
            )
        } else {
            None
        };
        Ok(Some(RateLimitResetCredits {
            connection_id,
            available_count: available_count.max(0) as u64,
            credits,
            observed_at: dt(&observed_at)?,
        }))
    }

    pub fn set_budget(&self, budget: &Budget) -> Result<(), StorageError> {
        self.db()?.execute(
            "INSERT INTO budgets(id,connection_id,amount,currency,period,warning_ratio,critical_ratio,enabled) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(connection_id) DO UPDATE SET amount=excluded.amount,currency=excluded.currency,period=excluded.period,warning_ratio=excluded.warning_ratio,critical_ratio=excluded.critical_ratio,enabled=excluded.enabled",
            params![budget.id.to_string(),budget.connection_id.to_string(),budget.amount.to_string(),budget.currency,js(&budget.period)?,budget.warning_ratio.to_string(),budget.critical_ratio.to_string(),budget.enabled as i64],
        )?;
        Ok(())
    }

    pub fn alerts(&self, connection_id: Option<Uuid>) -> Result<Vec<AlertRule>, StorageError> {
        let db = self.db()?;
        let mut stmt=db.prepare("SELECT id,connection_id,kind,threshold,state,last_triggered_at,suppressed_until FROM alerts WHERE (?1 IS NULL OR connection_id=?1) ORDER BY kind")?;
        let rows = stmt.query_map([connection_id.map(|v| v.to_string())], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        rows.map(|x| {
            let r = x?;
            Ok(AlertRule {
                id: Uuid::parse_str(&r.0)?,
                connection_id: Uuid::parse_str(&r.1)?,
                kind: serde_json::from_str(&r.2)?,
                threshold: r.3.parse()?,
                state: serde_json::from_str(&r.4)?,
                last_triggered_at: r.5.map(|v| dt(&v)).transpose()?,
                suppressed_until: r.6.map(|v| dt(&v)).transpose()?,
            })
        })
        .collect()
    }

    pub fn upsert_alert(&self, alert: &AlertRule) -> Result<(), StorageError> {
        self.db()?.execute("INSERT INTO alerts(id,connection_id,kind,threshold,state,last_triggered_at,suppressed_until) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,threshold=excluded.threshold,state=excluded.state,last_triggered_at=excluded.last_triggered_at,suppressed_until=excluded.suppressed_until",params![alert.id.to_string(),alert.connection_id.to_string(),js(&alert.kind)?,alert.threshold.to_string(),js(&alert.state)?,alert.last_triggered_at.map(|v|v.to_rfc3339()),alert.suppressed_until.map(|v|v.to_rfc3339())])?;
        Ok(())
    }
}

fn insert_metric(tx: &Transaction<'_>, m: &MetricSample) -> Result<(), StorageError> {
    tx.execute("INSERT INTO metric_samples(id,connection_id,metric_key,value,unit,scope,period_start,period_end,observed_at,provenance,dimensions_json,source_metric,dedup_key) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(dedup_key) DO UPDATE SET value=excluded.value,observed_at=excluded.observed_at,provenance=excluded.provenance",params![m.id.to_string(),m.connection_id.to_string(),m.metric_key.0,m.value.to_string(),js(&m.unit)?,js(&m.scope)?,m.period_start.map(|v|v.to_rfc3339()),m.period_end.map(|v|v.to_rfc3339()),m.observed_at.to_rfc3339(),js(&m.provenance)?,serde_json::to_string(&m.dimensions)?,m.source_metric,m.dedup_key])?;
    Ok(())
}
fn insert_quota(tx: &Transaction<'_>, q: &QuotaWindow) -> Result<(), StorageError> {
    tx.execute("INSERT INTO quota_windows(id,connection_id,provider_limit_id,display_name,window_kind,window_start,window_end,resets_at,used_ratio,remaining_ratio,used_value,limit_value,unit,provenance,observed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15) ON CONFLICT(connection_id,provider_limit_id) DO UPDATE SET display_name=excluded.display_name,window_kind=excluded.window_kind,window_start=excluded.window_start,window_end=excluded.window_end,resets_at=excluded.resets_at,used_ratio=excluded.used_ratio,remaining_ratio=excluded.remaining_ratio,used_value=excluded.used_value,limit_value=excluded.limit_value,unit=excluded.unit,provenance=excluded.provenance,observed_at=excluded.observed_at",params![q.id.to_string(),q.connection_id.to_string(),q.provider_limit_id,q.display_name,js(&q.window_kind)?,q.window_start.map(|v|v.to_rfc3339()),q.window_end.map(|v|v.to_rfc3339()),q.resets_at.map(|v|v.to_rfc3339()),q.used_ratio.map(|v|v.to_string()),q.remaining_ratio.map(|v|v.to_string()),q.used_value.map(|v|v.to_string()),q.limit_value.map(|v|v.to_string()),q.unit.as_ref().map(js).transpose()?,js(&q.provenance)?,q.observed_at.to_rfc3339()])?;
    Ok(())
}
fn js<T: serde::Serialize>(v: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(v)
}
fn dt(v: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    Ok(DateTime::parse_from_rfc3339(v)?.with_timezone(&Utc))
}
fn dec(v: Option<String>) -> Result<Option<Decimal>, rust_decimal::Error> {
    v.map(|x| x.parse()).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrates_and_cascades() {
        let r = Repository::in_memory().unwrap();
        assert_eq!(r.schema_version().unwrap(), 2);
        let now = Utc::now();
        let c = Connection {
            id: Uuid::new_v4(),
            provider_id: "mock".into(),
            connection_type: "test".into(),
            display_name: "Mock".into(),
            account_external_id: None,
            status: ConnectionStatus::Ready,
            credential_ref_id: None,
            created_at: now,
            updated_at: now,
            last_success_at: None,
            last_error_code: None,
            disabled_at: None,
        };
        r.add_connection(&c).unwrap();
        assert_eq!(r.list_connections().unwrap().len(), 1);
        r.remove_connection(c.id).unwrap();
        assert!(r.list_connections().unwrap().is_empty());
    }
    #[test]
    fn cursor_commits_with_batch() {
        let r = Repository::in_memory().unwrap();
        let now = Utc::now();
        let c = Connection {
            id: Uuid::new_v4(),
            provider_id: "mock".into(),
            connection_type: "test".into(),
            display_name: "Mock".into(),
            account_external_id: None,
            status: ConnectionStatus::Syncing,
            credential_ref_id: None,
            created_at: now,
            updated_at: now,
            last_success_at: None,
            last_error_code: None,
            disabled_at: None,
        };
        r.add_connection(&c).unwrap();
        let b = SyncBatch {
            next_cursor: Some(SyncCursor("next".into())),
            ..Default::default()
        };
        r.commit_sync_batch(c.id, "usage", &b).unwrap();
        assert_eq!(r.sync_cursor(c.id, "usage").unwrap().unwrap().0, "next");
    }

    #[test]
    fn reset_credit_details_round_trip() {
        let r = Repository::in_memory().unwrap();
        let now = Utc::now();
        let c = Connection {
            id: Uuid::new_v4(),
            provider_id: "openai".into(),
            connection_type: "chatgpt_subscription".into(),
            display_name: "ChatGPT".into(),
            account_external_id: None,
            status: ConnectionStatus::Syncing,
            credential_ref_id: None,
            created_at: now,
            updated_at: now,
            last_success_at: None,
            last_error_code: None,
            disabled_at: None,
        };
        r.add_connection(&c).unwrap();
        r.commit_sync_batch(
            c.id,
            "usage",
            &SyncBatch {
                rate_limit_reset_credits: Some(RateLimitResetCredits {
                    connection_id: c.id,
                    available_count: 2,
                    credits: Some(vec![RateLimitResetCredit {
                        id: "credit-1".into(),
                        connection_id: c.id,
                        reset_type: "codexRateLimits".into(),
                        status: "available".into(),
                        granted_at: now,
                        expires_at: Some(now + chrono::Duration::days(30)),
                        title: Some("Full reset".into()),
                        description: None,
                    }]),
                    observed_at: now,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let summary = r.rate_limit_reset_credits(c.id).unwrap().unwrap();
        assert_eq!(summary.available_count, 2);
        assert_eq!(summary.credits.unwrap()[0].id, "credit-1");
    }

    #[test]
    fn rejected_cross_connection_batch_does_not_advance_cursor() {
        let r = Repository::in_memory().unwrap();
        let now = Utc::now();
        let c = Connection {
            id: Uuid::new_v4(),
            provider_id: "mock".into(),
            connection_type: "test".into(),
            display_name: "Mock".into(),
            account_external_id: None,
            status: ConnectionStatus::Syncing,
            credential_ref_id: None,
            created_at: now,
            updated_at: now,
            last_success_at: None,
            last_error_code: None,
            disabled_at: None,
        };
        r.add_connection(&c).unwrap();
        let batch = SyncBatch {
            account_updates: vec![AccountRecord {
                id: Uuid::new_v4(),
                connection_id: Uuid::new_v4(),
                external_id: "wrong".into(),
                display_name: None,
                account_type: None,
            }],
            next_cursor: Some(SyncCursor("must-not-commit".into())),
            ..Default::default()
        };
        assert!(matches!(
            r.commit_sync_batch(c.id, "usage", &batch),
            Err(StorageError::CrossConnectionBatch)
        ));
        assert!(r.sync_cursor(c.id, "usage").unwrap().is_none());
    }
    #[test]
    fn retention_preserves_daily_aggregates() {
        let r = Repository::in_memory().unwrap();
        let now = Utc::now();
        let c = Connection {
            id: Uuid::new_v4(),
            provider_id: "mock".into(),
            connection_type: "test".into(),
            display_name: "Mock".into(),
            account_external_id: None,
            status: ConnectionStatus::Ready,
            credential_ref_id: None,
            created_at: now,
            updated_at: now,
            last_success_at: None,
            last_error_code: None,
            disabled_at: None,
        };
        r.add_connection(&c).unwrap();
        let old = now - chrono::Duration::days(200);
        let mut samples = Vec::new();
        for (name, duration) in [
            ("raw", None),
            ("hour", Some(chrono::Duration::hours(1))),
            ("day", Some(chrono::Duration::days(1))),
        ] {
            let mut m = MetricSample {
                id: Uuid::new_v4(),
                connection_id: c.id,
                metric_key: MetricKey(MetricKey::TOKEN_TOTAL.into()),
                value: Decimal::ONE,
                unit: MetricUnit::Token,
                scope: MetricScope::Account,
                period_start: duration.map(|_| old),
                period_end: duration.map(|d| old + d),
                observed_at: old,
                provenance: Provenance::ProviderReported,
                dimensions: std::collections::BTreeMap::new(),
                source_metric: name.into(),
                dedup_key: String::new(),
            };
            m.dedup_key = m.compute_dedup_key();
            samples.push(m);
        }
        r.commit_sync_batch(
            c.id,
            "usage",
            &SyncBatch {
                metric_samples: samples,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.prune(now, 30, 180, 30).unwrap(), 2);
        let remaining = r.metrics(c.id, None, 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].source_metric, "day");
    }
}
