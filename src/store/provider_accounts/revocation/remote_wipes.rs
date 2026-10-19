//! Historical authenticated Machine wipe evidence; never a dispatch grant.
use crate::store::provider_accounts::{Receipt, Registry, grants};
use crate::store::{StorageBackend, Store};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Evidence {
    pub grant_operation: String,
    pub machine_id: String,
    pub recipient_fingerprint: String,
    pub generation: u64,
}

#[cfg(test)]
mod tests;

impl Store {
    /// The caller verifies the exact sealed wipe ACK. First recording rechecks
    /// enrollment and its live connection under owner/Machine locks. Identical
    /// historical replay grants no authority and does not call the callback.
    pub(crate) async fn record_provider_remote_wipe(
        &self,
        owner: &str,
        operation: &str,
        evidence: &Evidence,
        connection_is_current: impl FnOnce() -> bool,
    ) -> Result<Evidence> {
        ensure!(
            evidence.generation > 0 && evidence.generation < u64::MAX,
            "invalid_wipe_generation"
        );
        macro_rules! record { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let mut receipt: Receipt = serde_json::from_str(&value)?;
            if let Some(old) = receipt.remote_wipes.get(&evidence.grant_operation) {
                ensure!(old == evidence, "remote_wipe_conflict");
                tx.commit().await?;
                return Ok(old.clone());
            }
            ensure!(!receipt.account_id.is_default() && receipt.revocation.as_ref().is_some_and(|plan|
                plan.grants.iter().any(|target| target.operation_id == evidence.grant_operation
                    && target.machine_id == evidence.machine_id)), "remote_wipe_scope_mismatch");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_grants WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(&evidence.grant_operation).fetch_one(&mut *tx).await?;
            let grant: grants::Receipt = serde_json::from_str(&value)?;
            ensure!(grant.account_id == receipt.account_id && grant.operation_id == evidence.grant_operation
                && grant.machine_id == evidence.machine_id && grant.recipient_fingerprint == evidence.recipient_fingerprint,
                "remote_wipe_scope_mismatch");
            ensure!(matches!(grant.phase, grants::Phase::Committed { generation }
                if grant.expected_generation.checked_add(1) == Some(generation) && evidence.generation > generation),
                "remote_wipe_generation_mismatch");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            ensure!(registry.accounts.iter().any(|account| account.id == grant.account_id
                && account.provider_id == grant.provider_id && account.revoking && !account.enabled), "account_not_revoking");
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&evidence.machine_id).execute(&mut *tx).await?;
            ensure!(machine.rows_affected() == 1, "machine_unavailable");
            let key: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&evidence.machine_id).fetch_one(&mut *tx).await?;
            ensure!(key.is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == evidence.recipient_fingerprint), "machine_enrollment_changed");
            ensure!(connection_is_current(), "machine_connection_changed");
            receipt.remote_wipes.insert(evidence.grant_operation.clone(), evidence.clone());
            sqlx::query("UPDATE provider_account_operations SET result=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            evidence.clone()
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => record!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => record!(db, "revoked_at_ms"),
        })
    }
}
