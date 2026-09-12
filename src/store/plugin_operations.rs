//! The uninstall journal and session deletion share one actual DB transaction.
//! The macro keeps the state machine/SQL identical across both storage backends;
//! only transaction durability and existing session timestamp columns differ.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::plugin_operation::resolution::{
    MAX_RESOLUTION_BYTES, ResolutionIntent, ResolutionPermit, ResolutionReceipt,
};
use crate::plugin_operation::{
    MAX_INTENT_BYTES, MAX_OPERATIONS, Operation, Phase, Problem, UninstallIntent,
};
use anyhow::{Context as _, Result, ensure};
use sha2::Digest as _;

#[derive(sqlx::FromRow)]
struct ResolutionRecord {
    operation_id: String,
    resolution_id: String,
    intent: String,
    intent_sha256: String,
    resolved_at_ms: i64,
}

impl ResolutionRecord {
    fn decode(self) -> Result<ResolutionReceipt> {
        ensure!(
            self.intent.len() <= MAX_RESOLUTION_BYTES
                && self.intent_sha256
                    == format!("{:x}", sha2::Sha256::digest(self.intent.as_bytes())),
            "resolution evidence is invalid"
        );
        let intent: ResolutionIntent = serde_json::from_str(&self.intent)
            .map_err(|_| anyhow::anyhow!("invalid resolution intent"))?;
        intent.validate()?;
        ensure!(
            intent.operation_id == self.operation_id
                && intent.resolution_id == self.resolution_id
                && self.resolved_at_ms > 0,
            "resolution identity is invalid"
        );
        Ok(ResolutionReceipt {
            intent,
            resolved_at_ms: self.resolved_at_ms,
        })
    }
}

#[derive(sqlx::FromRow)]
struct Record {
    operation_id: String,
    service_id: String,
    machine_id: String,
    plugin_id: String,
    intent: String,
    intent_sha256: String,
    phase: String,
    problem: Option<String>,
    cause: Option<String>,
    attention_from: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl Record {
    fn decode(self) -> Result<Operation> {
        ensure!(
            self.intent.len() <= MAX_INTENT_BYTES,
            "uninstall intent exceeds its budget"
        );
        ensure!(
            self.intent_sha256 == format!("{:x}", sha2::Sha256::digest(self.intent.as_bytes())),
            "uninstall intent digest mismatch"
        );
        let intent: UninstallIntent = serde_json::from_str(&self.intent)
            .map_err(|_| anyhow::anyhow!("invalid persisted uninstall intent"))?;
        intent.validate()?;
        ensure!(
            intent.operation_id == self.operation_id
                && intent.service_id == self.service_id
                && intent.machine_id == self.machine_id
                && intent.plugin_id == self.plugin_id,
            "uninstall journal identity mismatch"
        );
        let phase = serde_json::from_value(serde_json::Value::String(self.phase))
            .map_err(|_| anyhow::anyhow!("invalid uninstall journal phase"))?;
        let problem = self
            .problem
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid uninstall journal problem"))?;
        let attention_from = self
            .attention_from
            .map(|value| serde_json::from_value(serde_json::Value::String(value)))
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid uninstall interruption phase"))?;
        let cause = self
            .cause
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid uninstall primary failure"))?;
        Ok(Operation {
            intent,
            phase,
            problem,
            cause,
            attention_from,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        })
    }
}

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

impl Store {
    pub(crate) async fn begin_plugin_uninstall(&self, intent: &UninstallIntent) -> Result<()> {
        intent.validate()?;
        ensure!(
            intent.expires_at_ms >= chrono::Utc::now().timestamp_millis(),
            "uninstall approval expired"
        );
        let document = serde_json::to_string(intent)?;
        ensure!(
            document.len() <= MAX_INTENT_BYTES,
            "uninstall intent exceeds its budget"
        );
        dispatch!(self, begin_plugin_uninstall(intent, &document))
    }

    pub(crate) async fn plugin_uninstall_operation(&self, id: &str) -> Result<Option<Operation>> {
        dispatch!(self, plugin_uninstall_operation(id))
    }

    pub(crate) async fn plugin_uninstall_history(
        &self,
        service: &str,
        machine: &str,
        plugin: &str,
    ) -> Result<Vec<Operation>> {
        dispatch!(self, plugin_uninstall_history(service, machine, plugin))
    }

    /// Before dispatch/HTTP starts. Evidence survives restart, authorization does
    /// not: do not send compensating or forward Machine commands from this scan.
    pub(crate) async fn recover_plugin_uninstalls(&self, service: &str) -> Result<Vec<Operation>> {
        dispatch!(self, recover_plugin_uninstalls(service))
    }

    pub(crate) async fn advance_plugin_uninstall(
        &self,
        id: &str,
        from: Phase,
        to: Phase,
        problem: Option<Problem>,
    ) -> Result<()> {
        ensure!(
            from.permits(to) && to != Phase::Completed,
            "invalid uninstall transition"
        );
        dispatch!(self, advance_plugin_uninstall(id, from, to, problem))
    }

    /// Completion and the exact Service-side deletion are one local commit.
    /// The CAS also serializes with a possibly ambiguous earlier COMMIT.
    pub(crate) async fn commit_plugin_uninstall(&self, intent: &UninstallIntent) -> Result<()> {
        dispatch!(self, commit_plugin_uninstall(intent))
    }

    pub(crate) async fn plugin_uninstall_resolution(
        &self,
        operation: &str,
    ) -> Result<Option<ResolutionReceipt>> {
        dispatch!(self, plugin_uninstall_resolution(operation))
    }

    /// Only the separately authenticated, finite local resolution can take this
    /// transition. Ordinary phase advancement still cannot leave `NeedsAttention`.
    pub(crate) async fn resolve_plugin_uninstall(
        &self,
        permit: &ResolutionPermit,
    ) -> Result<ResolutionReceipt> {
        permit.intent().validate()?;
        ensure!(permit.within_budget(), "resolution approval expired");
        dispatch!(self, resolve_plugin_uninstall(permit))
    }
}

macro_rules! implement_journal {
    ($backend:ty, $durability:literal, $lock:literal, $delete:literal) => {
        impl $backend {
            async fn plugin_uninstall_resolution(&self, operation: &str) -> Result<Option<ResolutionReceipt>> {
                sqlx::query_as::<_, ResolutionRecord>(
                    "SELECT * FROM plugin_uninstall_resolutions WHERE operation_id = $1"
                ).bind(operation).fetch_optional(&self.pool).await?
                    .map(ResolutionRecord::decode).transpose()
            }

            async fn resolve_plugin_uninstall(&self, permit: &ResolutionPermit) -> Result<ResolutionReceipt> {
                let intent = permit.intent();
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                // Acquire the row/write lock BEFORE reading (also on SQLite).
                // The ordinary executor cannot advance this phase. A late live
                // Prepared -> StoppingSessions CAS loses to this resolution.
                let locked = sqlx::query(
                    "UPDATE plugin_uninstall_operations SET phase = phase \
                     WHERE operation_id = $1 AND phase = 'needs_attention' AND attention_from = 'prepared'"
                ).bind(&intent.operation_id).execute(&mut *tx).await?;
                ensure!(locked.rows_affected() == 1, "resolution phase changed");
                let before = sqlx::query_as::<_, Record>(
                    "SELECT * FROM plugin_uninstall_operations WHERE operation_id = $1"
                ).bind(&intent.operation_id).fetch_one(&mut *tx).await?.decode()?;
                ensure!(intent.matches(&before)?, "resolution evidence changed");
                ensure!(permit.within_budget(), "resolution approval expired while waiting");
                let document = serde_json::to_string(intent)?;
                ensure!(document.len() <= MAX_RESOLUTION_BYTES, "resolution evidence exceeds budget");
                let now = chrono::Utc::now().timestamp_millis();
                sqlx::query(
                    "INSERT INTO plugin_uninstall_resolutions \
                     (operation_id, resolution_id, intent, intent_sha256, resolved_at_ms) VALUES ($1, $2, $3, $4, $5)"
                ).bind(&intent.operation_id).bind(&intent.resolution_id).bind(&document)
                    .bind(format!("{:x}", sha2::Sha256::digest(document.as_bytes())))
                    .bind(now).execute(&mut *tx).await?;
                let changed = sqlx::query(
                    "UPDATE plugin_uninstall_operations SET phase = 'aborted', updated_at_ms = $2 WHERE operation_id = $1"
                ).bind(&intent.operation_id).bind(now).execute(&mut *tx).await?;
                ensure!(changed.rows_affected() == 1, "resolution completion was not persisted");
                // No session UPDATE, Machine RPC, auth mutation or credential
                // projection occurs here. Keep the original cause and intent.
                ensure!(permit.within_budget(), "resolution approval expired before commit");
                tx.commit().await.context("committing no-effect uninstall resolution")?;
                Ok(ResolutionReceipt { intent: intent.clone(), resolved_at_ms: now })
            }

            async fn begin_plugin_uninstall(&self, intent: &UninstallIntent, document: &str) -> Result<()> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let now = chrono::Utc::now().timestamp_millis();
                let inserted = sqlx::query(
                    "INSERT INTO plugin_uninstall_operations \
                     (operation_id, service_id, machine_id, plugin_id, intent, intent_sha256, phase, created_at_ms, updated_at_ms) \
                     SELECT $1, $2, $3, $4, $5, $6, 'prepared', $7, $7 \
                     WHERE (SELECT COUNT(*) FROM plugin_uninstall_operations) < $8",
                )
                .bind(&intent.operation_id).bind(&intent.service_id).bind(&intent.machine_id).bind(&intent.plugin_id)
                .bind(document).bind(format!("{:x}", sha2::Sha256::digest(document.as_bytes())))
                .bind(now).bind(MAX_OPERATIONS).execute(&mut *tx).await?;
                ensure!(inserted.rows_affected() == 1, "uninstall journal capacity exhausted; retained evidence requires review");
                tx.commit().await.context("committing uninstall intent")
            }

            async fn plugin_uninstall_operation(&self, id: &str) -> Result<Option<Operation>> {
                sqlx::query_as::<_, Record>("SELECT * FROM plugin_uninstall_operations WHERE operation_id = $1")
                    .bind(id).fetch_optional(&self.pool).await?.map(Record::decode).transpose()
            }

            async fn plugin_uninstall_history(&self, service: &str, machine: &str, plugin: &str) -> Result<Vec<Operation>> {
                sqlx::query_as::<_, Record>(
                    "SELECT * FROM plugin_uninstall_operations WHERE service_id = $1 AND machine_id = $2 AND plugin_id = $3 \
                     ORDER BY created_at_ms DESC, operation_id DESC LIMIT 32",
                ).bind(service).bind(machine).bind(plugin).fetch_all(&self.pool).await?
                    .into_iter().map(Record::decode).collect()
            }

            async fn recover_plugin_uninstalls(&self, service: &str) -> Result<Vec<Operation>> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let rows = sqlx::query_as::<_, Record>(
                    "SELECT * FROM plugin_uninstall_operations WHERE phase NOT IN ('completed', 'compensated', 'aborted') LIMIT 4097",
                ).fetch_all(&mut *tx).await?;
                ensure!(rows.len() <= usize::try_from(MAX_OPERATIONS)?, "uninstall recovery budget exceeded");
                let mut operations = rows.into_iter().map(Record::decode).collect::<Result<Vec<_>>>()?;
                ensure!(operations.iter().all(|op| op.intent.service_id == service), "unfinished uninstall belongs to another Service");
                let now = chrono::Utc::now().timestamp_millis();
                sqlx::query(
                    "UPDATE plugin_uninstall_operations SET attention_from = phase, phase = 'needs_attention', problem = $1, updated_at_ms = $2 \
                     WHERE phase NOT IN ('completed', 'compensated', 'aborted', 'needs_attention')",
                ).bind(serde_json::to_string(&Problem::Interrupted)?).bind(now).execute(&mut *tx).await?;
                for op in &mut operations {
                    if op.phase != Phase::NeedsAttention {
                        op.attention_from = Some(op.phase);
                        op.phase = Phase::NeedsAttention;
                        op.problem = Some(Problem::Interrupted);
                        op.updated_at_ms = now;
                    }
                }
                tx.commit().await?;
                Ok(operations)
            }

            async fn advance_plugin_uninstall(&self, id: &str, from: Phase, to: Phase, problem: Option<Problem>) -> Result<()> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let changed = sqlx::query(
                    "UPDATE plugin_uninstall_operations SET attention_from = CASE WHEN $3 = 'needs_attention' AND phase <> 'needs_attention' THEN phase ELSE attention_from END, \
                     cause = CASE WHEN $3 = 'restoring_machine' THEN $4 ELSE cause END, phase = $3, problem = $4, updated_at_ms = $5 \
                     WHERE operation_id = $1 AND phase = $2",
                ).bind(id).bind(from.as_str()).bind(to.as_str())
                    .bind(problem.map(|p| serde_json::to_string(&p)).transpose()?)
                    .bind(chrono::Utc::now().timestamp_millis()).execute(&mut *tx).await?;
                ensure!(changed.rows_affected() == 1, "uninstall phase changed; reconcile the same operation");
                tx.commit().await.context("committing uninstall progress")
            }

            async fn commit_plugin_uninstall(&self, intent: &UninstallIntent) -> Result<()> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                // Take the write lock before reading the saved intent. It also
                // makes a concurrent compensation lose against a committed delete.
                let changed = sqlx::query(
                    "UPDATE plugin_uninstall_operations SET phase = 'completed', problem = NULL, updated_at_ms = $2 \
                     WHERE operation_id = $1 AND phase = 'machine_uninstalled'",
                ).bind(&intent.operation_id).bind(chrono::Utc::now().timestamp_millis()).execute(&mut *tx).await?;
                ensure!(changed.rows_affected() == 1, "uninstall completion precondition changed");
                let saved = sqlx::query_as::<_, Record>("SELECT * FROM plugin_uninstall_operations WHERE operation_id = $1")
                    .bind(&intent.operation_id).fetch_one(&mut *tx).await?.decode()?;
                ensure!(&saved.intent == intent, "uninstall intent changed before commit");
                let now = chrono::Utc::now().timestamp_millis();
                for session in &intent.session_ids {
                    let changed = sqlx::query($delete).bind(session).bind(now).bind(intent.purge_after_ms)
                        .bind(&intent.machine_id).bind(&intent.plugin_id).execute(&mut *tx).await?;
                    ensure!(changed.rows_affected() == 1, "uninstall session identity or deletion state changed");
                }
                tx.commit().await.context("committing uninstall and session deletion")
            }
        }
    };
}

implement_journal!(
    PostgresStorage,
    "SET LOCAL synchronous_commit = on",
    "LOCK TABLE plugin_uninstall_operations IN SHARE ROW EXCLUSIVE MODE",
    "UPDATE sessions SET deleted_at = to_timestamp($2::bigint::double precision / 1000), purge_after_at = to_timestamp($3::bigint::double precision / 1000) \
     WHERE id = $1 AND machine_id = $4 AND provider = $5 AND deleted_at IS NULL"
);
implement_journal!(
    SqliteStorage,
    "SELECT 1",
    // Reserve the WAL writer before any journal SELECT, even for an empty
    // table. Upgrading a deferred read transaction can fail immediately with
    // SQLITE_BUSY/BUSY_SNAPSHOT instead of honoring busy_timeout when startup
    // sweepers write concurrently. This statement changes no rows.
    "UPDATE plugin_uninstall_operations SET phase = phase WHERE 0",
    "UPDATE sessions SET deleted_at_ms = $2, purge_after_at_ms = $3 \
     WHERE id = $1 AND machine_id = $4 AND provider = $5 AND deleted_at_ms IS NULL"
);

#[cfg(test)]
mod tests;
