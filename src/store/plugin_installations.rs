//! Durable Service installation attempts. SQL and transition rules are shared
//! by `PostgreSQL` and `SQLite`; an existing operation ID never grants execution.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::machine_protocol::plugin_install::{InstallOutcome, InstallReceipt};
use crate::plugin_operation::MAX_OPERATIONS;
use crate::plugin_operation::installation::{
    InstallIntent, InstallOperation, InstallPhase, InstallProblem, InstallStagingResolutionIntent,
    InstallStagingResolutionPermit, InstallStagingResolutionReceipt, MAX_INSTALL_INTENT_BYTES,
    MAX_INSTALL_STAGING_RESOLUTION_BYTES,
};
use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};

#[derive(sqlx::FromRow)]
struct StagingResolutionRecord {
    operation_id: String,
    resolution_id: String,
    intent: String,
    intent_sha256: String,
    resolved_at_ms: i64,
}

impl StagingResolutionRecord {
    fn decode(self) -> Result<InstallStagingResolutionReceipt> {
        ensure!(
            self.intent.len() <= MAX_INSTALL_STAGING_RESOLUTION_BYTES
                && self.intent_sha256 == format!("{:x}", Sha256::digest(self.intent.as_bytes())),
            "invalid install staging resolution integrity"
        );
        let intent: InstallStagingResolutionIntent = serde_json::from_str(&self.intent)
            .map_err(|_| anyhow::anyhow!("invalid install staging resolution"))?;
        intent.validate()?;
        ensure!(
            intent.operation_id == self.operation_id
                && intent.resolution_id == self.resolution_id
                && self.resolved_at_ms > 0,
            "invalid install staging resolution identity"
        );
        Ok(InstallStagingResolutionReceipt {
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
    attention_from: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
    machine_receipt: Option<String>,
    machine_receipt_sha256: Option<String>,
    staging_resolved_at_ms: Option<i64>,
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
    pub(crate) async fn plugin_install_staging_resolution(
        &self,
        operation: &str,
    ) -> Result<Option<InstallStagingResolutionReceipt>> {
        dispatch!(self, plugin_install_staging_resolution(operation))
    }

    /// Commit a separately authorized conclusion that one exact Machine
    /// attempt failed while still in content-addressed staging. The original
    /// operation and receipt remain unchanged for audit and deduplication.
    pub(crate) async fn resolve_plugin_install_staging(
        &self,
        permit: &InstallStagingResolutionPermit,
        before: &InstallOperation,
    ) -> Result<InstallStagingResolutionReceipt> {
        before.validate()?;
        let intent = permit.intent();
        ensure!(
            permit.within_budget() && intent.matches(before, &intent.observed_target)?,
            "installation staging resolution is not authorized"
        );
        let document = serde_json::to_string(intent)?;
        ensure!(
            document.len() <= MAX_INSTALL_STAGING_RESOLUTION_BYTES,
            "install staging resolution exceeds budget"
        );
        dispatch!(
            self,
            resolve_plugin_install_staging(permit, before, &document)
        )
    }

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

    /// Fresh recovery observation for one already-fenced attempt. The caller
    /// supplies the complete snapshot it inspected; the transaction compares
    /// that snapshot under the journal lock before accepting a terminal receipt.
    /// This never recreates the original installation authority or dispatches a
    /// Machine effect.
    pub(crate) async fn reconcile_plugin_install_receipt(
        &self,
        before: &InstallOperation,
        receipt: &InstallReceipt,
    ) -> Result<InstallOperation> {
        before.validate()?;
        ensure!(
            before.intent.schema == 2
                && before.phase == InstallPhase::NeedsAttention
                && matches!(
                    before.attention_from,
                    Some(InstallPhase::Installing | InstallPhase::MachineAcknowledged)
                )
                && (before.attention_from == Some(InstallPhase::MachineAcknowledged)
                    || before.machine_receipt.as_ref().is_none_or(|saved| {
                        matches!(&saved.outcome, InstallOutcome::Pending { .. })
                    }))
                && receipt.matches(&before.intent.machine_step()?)
                && matches!(
                    receipt.outcome,
                    InstallOutcome::Applied { .. } | InstallOutcome::Rejected { .. }
                ),
            "installation is not eligible for terminal receipt reconciliation"
        );
        if before.attention_from == Some(InstallPhase::MachineAcknowledged) {
            ensure!(
                before.machine_receipt.as_ref() == Some(receipt)
                    && matches!(receipt.outcome, InstallOutcome::Applied { .. }),
                "acknowledged installation recovery must retain its applied receipt"
            );
        } else if let Some(saved) = &before.machine_receipt {
            ensure!(
                !matches!(receipt.outcome, InstallOutcome::Rejected { .. })
                    || matches!(
                        &saved.outcome,
                        InstallOutcome::Pending {
                            phase: crate::machine_protocol::plugin_install::InstallPhase::Prepared
                        }
                    ),
                "rejected installation receipt cannot follow staging"
            );
        }
        let document = serde_json::to_string(receipt)?;
        ensure!(
            document.len() <= 8192,
            "Machine install receipt exceeds budget"
        );
        dispatch!(
            self,
            reconcile_plugin_install_receipt(before, receipt, &document)
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
            async fn plugin_install_staging_resolution(&self, operation: &str) -> Result<Option<InstallStagingResolutionReceipt>> {
                let Some(record) = sqlx::query_as::<_, StagingResolutionRecord>(
                    "SELECT * FROM plugin_install_staging_resolutions WHERE operation_id = $1"
                ).bind(operation).fetch_optional(&self.pool).await? else {
                    return Ok(None);
                };
                let resolution = record.decode()?;
                let saved = sqlx::query_as::<_, Record>(
                    "SELECT * FROM plugin_install_operations WHERE operation_id = $1"
                ).bind(operation).fetch_one(&self.pool).await?;
                ensure!(
                    saved.staging_resolved_at_ms == Some(resolution.resolved_at_ms),
                    "install staging resolution marker is invalid"
                );
                let operation = saved.decode()?;
                ensure!(
                    resolution.intent.matches(&operation, &resolution.intent.observed_target)?,
                    "install staging resolution does not match its operation"
                );
                Ok(Some(resolution))
            }

            async fn resolve_plugin_install_staging(
                &self,
                permit: &InstallStagingResolutionPermit,
                before: &InstallOperation,
                document: &str,
            ) -> Result<InstallStagingResolutionReceipt> {
                let intent = permit.intent();
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let locked = sqlx::query(
                    "UPDATE plugin_install_operations SET phase = phase \
                     WHERE operation_id = $1 AND phase = 'needs_attention'"
                ).bind(&intent.operation_id).execute(&mut *tx).await?;
                ensure!(locked.rows_affected() == 1, "install staging resolution phase changed");
                let saved = sqlx::query_as::<_, Record>(
                    "SELECT * FROM plugin_install_operations WHERE operation_id = $1"
                ).bind(&intent.operation_id).fetch_one(&mut *tx).await?;
                ensure!(saved.staging_resolved_at_ms.is_none(), "install staging slot is already resolved");
                let saved = saved.decode()?;
                ensure!(
                    saved == *before && intent.matches(&saved, &intent.observed_target)?,
                    "install staging resolution evidence changed"
                );
                ensure!(permit.within_budget(), "install staging resolution approval expired while waiting");
                let now = chrono::Utc::now().timestamp_millis();
                let inserted = sqlx::query(
                    "INSERT INTO plugin_install_staging_resolutions \
                     (operation_id, resolution_id, intent, intent_sha256, resolved_at_ms) \
                     VALUES ($1, $2, $3, $4, $5)"
                ).bind(&intent.operation_id).bind(&intent.resolution_id).bind(document)
                    .bind(format!("{:x}", Sha256::digest(document.as_bytes())))
                    .bind(now).execute(&mut *tx).await?;
                ensure!(inserted.rows_affected() == 1, "install staging resolution was not persisted");
                let released = sqlx::query(
                    "UPDATE plugin_install_operations SET staging_resolved_at_ms = $2 \
                     WHERE operation_id = $1 AND phase = 'needs_attention' \
                       AND staging_resolved_at_ms IS NULL"
                ).bind(&intent.operation_id).bind(now).execute(&mut *tx).await?;
                ensure!(released.rows_affected() == 1, "install staging slot was not released");
                ensure!(permit.within_budget(), "install staging resolution approval expired before commit");
                tx.commit().await.context("committing install staging resolution")?;
                Ok(InstallStagingResolutionReceipt { intent: intent.clone(), resolved_at_ms: now })
            }

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

            async fn reconcile_plugin_install_receipt(&self, before: &InstallOperation, receipt: &InstallReceipt, document: &str) -> Result<InstallOperation> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let saved = sqlx::query_as::<_, Record>("SELECT * FROM plugin_install_operations WHERE operation_id = $1")
                    .bind(&before.intent.operation_id).fetch_one(&mut *tx).await?.decode()?;
                ensure!(saved == *before, "install reconciliation evidence changed");
                let (phase, problem) = match receipt.outcome {
                    InstallOutcome::Applied { .. } => (InstallPhase::MachineAcknowledged, None),
                    InstallOutcome::Rejected { .. } => (InstallPhase::Aborted, Some(InstallProblem::MachineRejected)),
                    InstallOutcome::Pending { .. } | InstallOutcome::Unknown { .. } => anyhow::bail!("nonterminal receipt cannot resolve installation"),
                };
                let next = InstallOperation {
                    phase,
                    problem,
                    attention_from: None,
                    updated_at_ms: chrono::Utc::now().timestamp_millis().max(saved.updated_at_ms),
                    machine_receipt: Some(receipt.clone()),
                    ..saved
                };
                next.validate()?;
                let changed = sqlx::query(
                    "UPDATE plugin_install_operations SET phase = $2, problem = $3, attention_from = NULL, updated_at_ms = $4, \
                     machine_receipt = $5, machine_receipt_sha256 = $6 \
                     WHERE operation_id = $1 AND phase = 'needs_attention' AND updated_at_ms = $7"
                ).bind(&before.intent.operation_id).bind(next.phase.as_str())
                    .bind(next.problem.map(|p| serde_json::to_string(&p)).transpose()?)
                    .bind(next.updated_at_ms).bind(document)
                    .bind(format!("{:x}", Sha256::digest(document.as_bytes())))
                    .bind(before.updated_at_ms).execute(&mut *tx).await?;
                ensure!(changed.rows_affected() == 1, "install reconciliation changed");
                tx.commit().await.context("committing reconciled Machine installation receipt")?;
                Ok(next)
            }

            async fn recover_plugin_installs(&self, service: &str) -> Result<Vec<InstallOperation>> {
                let mut tx = self.pool.begin().await?;
                sqlx::query($durability).execute(&mut *tx).await?;
                sqlx::query($lock).execute(&mut *tx).await?;
                let rows = sqlx::query_as::<_, Record>("SELECT * FROM plugin_install_operations ORDER BY operation_id ASC LIMIT 4097")
                    .fetch_all(&mut *tx).await?;
                ensure!(rows.len() <= usize::try_from(MAX_OPERATIONS)?, "install recovery budget exceeded");
                let resolution_markers = rows.iter()
                    .filter_map(|record| record.staging_resolved_at_ms.map(|time| (record.operation_id.clone(), time)))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let mut operations = rows.into_iter().map(Record::decode).collect::<Result<Vec<_>>>()?;
                let resolution_rows = sqlx::query_as::<_, StagingResolutionRecord>(
                    "SELECT * FROM plugin_install_staging_resolutions ORDER BY operation_id ASC LIMIT 4097"
                ).fetch_all(&mut *tx).await?;
                ensure!(resolution_rows.len() <= usize::try_from(MAX_OPERATIONS)?, "install staging resolution recovery budget exceeded");
                let resolutions = resolution_rows.into_iter()
                    .map(StagingResolutionRecord::decode)
                    .collect::<Result<Vec<_>>>()?;
                let mut resolved = std::collections::BTreeSet::new();
                for resolution in resolutions {
                    let operation = operations.iter()
                        .find(|operation| operation.intent.operation_id == resolution.intent.operation_id)
                        .context("install staging resolution lost its operation")?;
                    ensure!(
                        resolution.intent.matches(operation, &resolution.intent.observed_target)?
                            && resolution_markers.get(&resolution.intent.operation_id) == Some(&resolution.resolved_at_ms)
                            && resolved.insert(resolution.intent.operation_id.clone()),
                        "install staging resolution does not match its operation"
                    );
                }
                ensure!(
                    resolved.len() == resolution_markers.len(),
                    "install staging resolution marker lost its audit"
                );
                operations.retain(|op| {
                    !op.phase.terminal() && !resolved.contains(&op.intent.operation_id)
                });
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
    "LOCK TABLE plugin_uninstall_operations, plugin_install_operations, plugin_install_staging_resolutions IN SHARE ROW EXCLUSIVE MODE"
);
implement_journal!(
    SqliteStorage,
    "SELECT 1",
    "UPDATE plugin_install_operations SET phase = phase WHERE 0"
);

#[cfg(test)]
mod tests;
