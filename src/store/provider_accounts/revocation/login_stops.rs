//! Historical temporary-login cleanup evidence, never process-tree authority.
use crate::store::provider_accounts::{Receipt, Registry, logins};
use crate::store::{StorageBackend, Store};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Evidence {
    pub login_operation: String,
    pub machine_id: String,
    pub recipient_fingerprint: String,
    pub observation: crate::provider_account::LoginStopObservation,
}

#[cfg(test)]
mod tests;

impl Store {
    /// First recording revalidates original admission and live enrollment under
    /// the owner/Machine locks. Identical historical replay grants no authority.
    pub(crate) async fn record_provider_login_stop(
        &self,
        owner: &str,
        operation: &str,
        evidence: &Evidence,
        connection_is_current: impl FnOnce() -> bool,
    ) -> Result<Evidence> {
        let observed = &evidence.observation;
        ensure!(
            observed.candidate_removed
                && observed.process_started == observed.leader_exited
                && (!observed.process_tree_exited || observed.leader_exited),
            "login_cleanup_unconfirmed"
        );
        macro_rules! record { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let mut receipt: Receipt = serde_json::from_str(&value)?;
            if let Some(old) = receipt.login_stops.get(&evidence.login_operation) {
                ensure!(old == evidence, "login_stop_conflict");
                tx.commit().await?;
                return Ok(old.clone());
            }
            ensure!(!receipt.account_id.is_default() && receipt.revocation.as_ref().is_some_and(|plan|
                plan.logins.iter().any(|target| target.operation_id == evidence.login_operation
                    && target.machine_id == evidence.machine_id)), "login_stop_scope_mismatch");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(&evidence.login_operation).fetch_one(&mut *tx).await?;
            let login: logins::Receipt = serde_json::from_str(&value)?;
            ensure!(login.input.account_id == receipt.account_id && login.input.account_id == observed.request.account_id
                && login.input.machine_id == evidence.machine_id && login.provider_id == observed.request.provider_id
                && login.request_id == observed.request.login_request_id
                && login.dispatch_recipient_fingerprint.as_ref() == Some(&evidence.recipient_fingerprint), "login_stop_scope_mismatch");
            ensure!(login.cancel_requested && login.dispatch_rejection.is_none()
                && login.native_generation.is_none() && login.native_completion.is_none(), "login_stop_not_admitted");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            ensure!(registry.accounts.iter().any(|account| account.id == receipt.account_id
                && account.provider_id == login.provider_id && account.revoking && !account.enabled), "account_not_revoking");
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&evidence.machine_id).execute(&mut *tx).await?;
            ensure!(machine.rows_affected() == 1, "machine_unavailable");
            let key: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&evidence.machine_id).fetch_one(&mut *tx).await?;
            ensure!(key.is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == evidence.recipient_fingerprint), "machine_enrollment_changed");
            ensure!(connection_is_current(), "machine_connection_changed");
            receipt.login_stops.insert(evidence.login_operation.clone(), evidence.clone());
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
