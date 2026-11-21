//! Historical Machine browser revocation evidence: the Machine reported the
//! account's browser hibernated and its profile deleted for this revocation.
//! Never dispatch authority, and never proof about native credentials.
use crate::store::provider_accounts::{Receipt, Registry};
use crate::store::{StorageBackend, Store};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub machine_id: String,
    pub instance_id: String,
    /// The Machine-side operation id the revocation was issued under.
    pub operation_id: String,
    /// Profile revision after the Machine deleted the profile.
    pub profile_revision: u64,
    pub observed_at_ms: i64,
}

impl Store {
    /// First recording revalidates the plan membership, the admission fence
    /// and the live Machine under the owner and Machine locks. Identical
    /// historical replay grants no authority.
    pub async fn record_provider_browser_revocation(
        &self,
        owner: &str,
        operation: &str,
        evidence: &Evidence,
        connection_is_current: impl FnOnce() -> bool,
    ) -> Result<Evidence> {
        macro_rules! record { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let mut receipt: Receipt = serde_json::from_str(&value)?;
            if let Some(old) = receipt.browser_revocations.get(&evidence.machine_id) {
                ensure!(old == evidence, "browser_revocation_conflict");
                tx.commit().await?;
                return Ok(old.clone());
            }
            ensure!(!receipt.account_id.is_default() && receipt.revocation.as_ref().is_some_and(|plan|
                plan.browsers.iter().any(|target| target.machine_id == evidence.machine_id
                    && target.instance_id == evidence.instance_id)), "browser_revocation_scope_mismatch");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            ensure!(registry.accounts.iter().any(|account| account.id == receipt.account_id
                && account.revoking && !account.enabled), "account_not_revoking");
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&evidence.machine_id).execute(&mut *tx).await?;
            ensure!(machine.rows_affected() == 1, "machine_unavailable");
            ensure!(connection_is_current(), "machine_connection_changed");
            receipt.browser_revocations.insert(evidence.machine_id.clone(), evidence.clone());
            sqlx::query("UPDATE provider_account_operations SET result=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM browser_placements WHERE owner_user_id=$1 AND account_id=$2 AND machine_id=$3")
                .bind(owner).bind(receipt.account_id.as_str()).bind(&evidence.machine_id).execute(&mut *tx).await?;
            tx.commit().await?;
            evidence.clone()
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => record!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => record!(db, "revoked_at_ms"),
        })
    }
}
