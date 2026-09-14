//! Durable Service installation attempts. SQL and transition rules are shared
//! by `PostgreSQL` and `SQLite`; an existing operation ID never grants execution.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::machine_protocol::plugin_install::{InstallOutcome, InstallReceipt};
use crate::plugin_operation::MAX_OPERATIONS;
use crate::plugin_operation::installation::{
    InstallIntent, InstallOperation, InstallPhase, InstallProblem, MAX_INSTALL_INTENT_BYTES,
};
use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};

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
    attention_from: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
    machine_receipt: Option<String>,
    machine_receipt_sha256: Option<String>,
}

impl Record {
    fn decode(self) -> Result<InstallOperation> {
        let machine_receipt = match (self.machine_receipt, self.machine_receipt_sha256) {
            (None, None) => None,
            (Some(document), Some(checksum)) => {
                ensure!(
                    document.len() <= 8192
                        && checksum == format!("{:x}", Sha256::digest(document.as_bytes())),
                    "invalid install Machine receipt integrity"
                );
                Some(
                    serde_json::from_str(&document)
                        .map_err(|_| anyhow::anyhow!("invalid install Machine receipt"))?,
                )
            }
            _ => anyhow::bail!("incomplete install Machine receipt"),
        };
        ensure!(
            self.intent.len() <= MAX_INSTALL_INTENT_BYTES
                && self.intent_sha256 == format!("{:x}", Sha256::digest(self.intent.as_bytes())),
            "invalid install intent integrity"
        );
        let intent: InstallIntent = serde_json::from_str(&self.intent)
            .map_err(|_| anyhow::anyhow!("invalid install intent"))?;
        ensure!(
            intent.operation_id == self.operation_id
                && intent.service_id == self.service_id
                && intent.machine_id == self.machine_id
                && intent.plugin_id == self.plugin_id,
            "install evidence identity mismatch"
        );
        let phase = serde_json::from_value(serde_json::Value::String(self.phase))
            .map_err(|_| anyhow::anyhow!("invalid install phase"))?;
        let problem = self
            .problem
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid install problem"))?;
        let attention_from = self
            .attention_from
            .map(|value| serde_json::from_value(serde_json::Value::String(value)))
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid install interruption phase"))?;
        let operation = InstallOperation {
            intent,
            phase,
            problem,
            attention_from,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            machine_receipt,
        };
        operation.validate()?;
        Ok(operation)
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
    /// Original forward observation only. Historical recovery needs a fresh,
    /// independent operation; this cannot overwrite terminal/uncertain state.
    pub(crate) async fn record_plugin_install_receipt(
        &self,
        intent: &InstallIntent,
        receipt: &InstallReceipt,
    ) -> Result<InstallOperation> {
        ensure!(
            receipt.matches(&intent.machine_step()?),
            "Machine install receipt target mismatch"
        );
        let document = serde_json::to_string(receipt)?;
        ensure!(
            document.len() <= 8192,
            "Machine install receipt exceeds budget"
        );
        dispatch!(
            self,
            record_plugin_install_receipt(intent, receipt, &document)
        )
    }

    pub(crate) async fn begin_plugin_install(&self, intent: &InstallIntent) -> Result<()> {
        intent.validate()?;
        let document = serde_json::to_string(intent)?;
        ensure!(
            document.len() <= MAX_INSTALL_INTENT_BYTES,
            "install intent exceeds budget"
        );
        dispatch!(self, begin_plugin_install(intent, &document))
    }

    pub(crate) async fn plugin_install_operation(
        &self,
        id: &str,
    ) -> Result<Option<InstallOperation>> {
        dispatch!(self, plugin_install_operation(id))
    }

    pub(crate) async fn plugin_install_history(
        &self,
        service: &str,
        machine: &str,
        plugin: &str,
    ) -> Result<Vec<InstallOperation>> {
        dispatch!(self, plugin_install_history(service, machine, plugin))
    }

    pub(crate) async fn advance_plugin_install(
        &self,
        intent: &InstallIntent,
        from: InstallPhase,
        to: InstallPhase,
        problem: Option<InstallProblem>,
    ) -> Result<()> {
        intent.validate()?;
        ensure!(from.permits(to), "invalid install transition");
        ensure!(
            (to != InstallPhase::NeedsAttention || problem.is_some())
                && (to != InstallPhase::Aborted || problem.is_some())
                && (from != InstallPhase::Installing
                    || to != InstallPhase::Aborted
                    || problem == Some(InstallProblem::TransportNotSent)),
            "install transition lacks evidence"
        );
        dispatch!(self, advance_plugin_install(intent, from, to, problem))
    }

    /// Startup only: no Machine commands or authentication writes. Each
    /// interrupted phase becomes fenced evidence, never a renewed grant.
    pub(crate) async fn recover_plugin_installs(
        &self,
        service: &str,
    ) -> Result<Vec<InstallOperation>> {
        dispatch!(self, recover_plugin_installs(service))
    }
}

macro_rules! implement_journal {
    ($backend:ty, $durability:literal, $lock:literal) => {
        impl $backend {
            async fn begin_plugin_install(&self, intent: &InstallIntent, document: &str) -> Result<()> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let now = chrono::Utc::now().timestamp_millis();
                ensure!(now > 0 && now < intent.expires_at_ms, "install approval expired while waiting");
                let inserted = sqlx::query(
                    "INSERT INTO plugin_install_operations \
                     (operation_id, service_id, machine_id, plugin_id, intent, intent_sha256, phase, created_at_ms, updated_at_ms) \
                     SELECT $1, $2, $3, $4, $5, $6, 'prepared', $7, $7 \
                     WHERE (SELECT COUNT(*) FROM plugin_install_operations) < $8 \
                       AND NOT EXISTS (SELECT 1 FROM plugin_uninstall_operations \
                         WHERE machine_id = $3 AND plugin_id = $4 AND phase NOT IN ('completed', 'compensated', 'aborted'))"
                ).bind(&intent.operation_id).bind(&intent.service_id).bind(&intent.machine_id).bind(&intent.plugin_id)
                    .bind(document).bind(format!("{:x}", Sha256::digest(document.as_bytes())))
                    .bind(now).bind(MAX_OPERATIONS).execute(&mut *tx).await?;
                ensure!(inserted.rows_affected() == 1, "install slot unavailable or evidence capacity exhausted");
                tx.commit().await.context("committing install intent")
            }

            async fn plugin_install_operation(&self, id: &str) -> Result<Option<InstallOperation>> {
                sqlx::query_as::<_, Record>("SELECT * FROM plugin_install_operations WHERE operation_id = $1")
                    .bind(id).fetch_optional(&self.pool).await?.map(Record::decode).transpose()
            }

            async fn plugin_install_history(&self, service: &str, machine: &str, plugin: &str) -> Result<Vec<InstallOperation>> {
                sqlx::query_as::<_, Record>(
                    "SELECT * FROM plugin_install_operations WHERE service_id = $1 AND machine_id = $2 AND plugin_id = $3 \
                     ORDER BY created_at_ms DESC, operation_id DESC LIMIT 32"
                ).bind(service).bind(machine).bind(plugin).fetch_all(&self.pool).await?
                    .into_iter().map(Record::decode).collect()
            }

            async fn advance_plugin_install(&self, intent: &InstallIntent, from: InstallPhase, to: InstallPhase, problem: Option<InstallProblem>) -> Result<()> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let saved = sqlx::query_as::<_, Record>("SELECT * FROM plugin_install_operations WHERE operation_id = $1")
                    .bind(&intent.operation_id).fetch_one(&mut *tx).await?.decode()?;
                ensure!(saved.intent == *intent && saved.phase == from, "install evidence changed");
                let now = chrono::Utc::now().timestamp_millis().max(saved.updated_at_ms);
                InstallOperation {
                    phase: to, problem,
                    attention_from: (to == InstallPhase::NeedsAttention).then_some(from),
                    updated_at_ms: now, ..saved
                }.validate()?;
                let changed = sqlx::query(
                    "UPDATE plugin_install_operations SET phase = $3, problem = $4, \
                     attention_from = CASE WHEN $3 = 'needs_attention' THEN phase ELSE attention_from END, updated_at_ms = $5 \
                     WHERE operation_id = $1 AND phase = $2"
                ).bind(&intent.operation_id).bind(from.as_str()).bind(to.as_str())
                    .bind(problem.map(|p| serde_json::to_string(&p)).transpose()?)
                    .bind(now).execute(&mut *tx).await?;
                ensure!(changed.rows_affected() == 1, "install phase changed");
                tx.commit().await.context("committing install progress")
            }

            async fn record_plugin_install_receipt(&self, intent: &InstallIntent, receipt: &InstallReceipt, document: &str) -> Result<InstallOperation> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let saved = sqlx::query_as::<_, Record>("SELECT * FROM plugin_install_operations WHERE operation_id = $1")
                    .bind(&intent.operation_id).fetch_one(&mut *tx).await?.decode()?;
                ensure!(saved.intent == *intent && saved.phase == InstallPhase::Installing && saved.machine_receipt.is_none(), "install receipt compare-and-swap failed");
                let (phase, problem) = match receipt.outcome {
                    InstallOutcome::Applied { .. } => (InstallPhase::MachineAcknowledged, None),
                    InstallOutcome::Rejected { .. } => (InstallPhase::Aborted, Some(InstallProblem::MachineRejected)),
                    InstallOutcome::Pending { .. } | InstallOutcome::Unknown { .. } => (InstallPhase::NeedsAttention, Some(InstallProblem::UnknownMachineOutcome)),
                };
                let next = InstallOperation {
                    phase, problem,
                    attention_from: (phase == InstallPhase::NeedsAttention).then_some(InstallPhase::Installing),
                    updated_at_ms: chrono::Utc::now().timestamp_millis().max(saved.updated_at_ms),
                    machine_receipt: Some(receipt.clone()), ..saved
                };
                next.validate()?;
                let changed = sqlx::query(
                    "UPDATE plugin_install_operations SET phase = $2, problem = $3, attention_from = $4, updated_at_ms = $5, \
                     machine_receipt = $6, machine_receipt_sha256 = $7 \
                     WHERE operation_id = $1 AND phase = 'installing' AND machine_receipt IS NULL AND machine_receipt_sha256 IS NULL"
                ).bind(&intent.operation_id).bind(next.phase.as_str())
                    .bind(next.problem.map(|p| serde_json::to_string(&p)).transpose()?)
                    .bind(next.attention_from.map(InstallPhase::as_str)).bind(next.updated_at_ms)
                    .bind(document).bind(format!("{:x}", Sha256::digest(document.as_bytes())))
                    .execute(&mut *tx).await?;
                ensure!(changed.rows_affected() == 1, "install receipt changed");
                tx.commit().await.context("committing Machine installation receipt")?;
                Ok(next)
            }

            async fn recover_plugin_installs(&self, service: &str) -> Result<Vec<InstallOperation>> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let rows = sqlx::query_as::<_, Record>("SELECT * FROM plugin_install_operations ORDER BY operation_id ASC LIMIT 4097")
                    .fetch_all(&mut *tx).await?;
                ensure!(rows.len() <= usize::try_from(MAX_OPERATIONS)?, "install recovery budget exceeded");
                let mut operations = rows.into_iter().map(Record::decode).collect::<Result<Vec<_>>>()?;
                operations.retain(|op| !op.phase.terminal());
                ensure!(operations.iter().all(|op| op.intent.service_id == service), "unfinished install belongs to another Service");
                let now = chrono::Utc::now().timestamp_millis();
                ensure!(now > 0, "invalid recovery time");
                for op in &mut operations {
                    if op.phase == InstallPhase::NeedsAttention { continue; }
                    let changed = sqlx::query(
                        "UPDATE plugin_install_operations SET attention_from = phase, phase = 'needs_attention', problem = $2, updated_at_ms = $3 \
                         WHERE operation_id = $1 AND phase = $4"
                    ).bind(&op.intent.operation_id).bind(serde_json::to_string(&InstallProblem::Interrupted)?)
                        .bind(now.max(op.updated_at_ms)).bind(op.phase.as_str()).execute(&mut *tx).await?;
                    ensure!(changed.rows_affected() == 1, "install recovery changed");
                    op.attention_from = Some(op.phase);
                    op.phase = InstallPhase::NeedsAttention;
                    op.problem = Some(InstallProblem::Interrupted);
                    op.updated_at_ms = now.max(op.updated_at_ms);
                    op.validate()?;
                }
                tx.commit().await.context("committing install recovery fences")?;
                Ok(operations)
            }
        }
    };
}

implement_journal!(
    PostgresStorage,
    "SET LOCAL synchronous_commit = on",
    "LOCK TABLE plugin_uninstall_operations, plugin_install_operations IN SHARE ROW EXCLUSIVE MODE"
);
implement_journal!(
    SqliteStorage,
    "SELECT 1",
    "UPDATE plugin_install_operations SET phase = phase WHERE 0"
);

#[cfg(test)]
mod tests;
