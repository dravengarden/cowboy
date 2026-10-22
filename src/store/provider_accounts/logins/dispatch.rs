//! Serialize fresh login enqueue with account revocation and cancellation.
use super::{Claim, Receipt, Registry, Result, StorageBackend, Store};

pub struct DispatchScope {
    pub registry_revision: i64,
    pub recipient_fingerprint: String,
}

impl Store {
    /// Only the fresh claimant may call this once. The synchronous callback
    /// must check the live connection and must not await or reenter Store.
    /// A commit error after the callback is an unknown outcome, never retry-send.
    pub async fn with_provider_login_dispatch<T>(
        &self,
        owner: &str,
        operation: &str,
        claim: &Claim,
        scope: &DispatchScope,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        anyhow::ensure!(claim.fresh, "login_dispatch_not_fresh");
        macro_rules! dispatch { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            anyhow::ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let mut current: Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(current == claim.receipt && !current.cancel_requested
                && current.dispatch_rejection.is_none() && current.native_completion.is_none(), "login_not_admitted");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            anyhow::ensure!(registry.revision == scope.registry_revision && registry.accounts.iter().any(|account|
                account.id == current.input.account_id && account.provider_id == current.provider_id && account.available()), "account_admission_changed");
            if let Some(generation) = current.native_generation {
                let latest: i64 = sqlx::query_scalar("SELECT generation FROM provider_native_generations WHERE owner_user_id=$1 AND account_id=$2 AND machine_id=$3")
                    .bind(owner).bind(current.input.account_id.as_str()).bind(&current.input.machine_id).fetch_one(&mut *tx).await?;
                anyhow::ensure!(u64::try_from(latest)? == generation, "native_generation_superseded");
            }
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&current.input.machine_id).execute(&mut *tx).await?;
            anyhow::ensure!(machine.rows_affected() == 1, "machine_unavailable");
            let key: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&current.input.machine_id).fetch_one(&mut *tx).await?;
            anyhow::ensure!(key.as_ref().is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == scope.recipient_fingerprint), "machine_enrollment_changed");
            current.dispatch_recipient_fingerprint = Some(scope.recipient_fingerprint.clone());
            current.dispatch_registry_revision = Some(scope.registry_revision);
            sqlx::query("UPDATE provider_login_operations SET value=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).bind(serde_json::to_string(&current)?).execute(&mut *tx).await?;
            let result = action()?;
            tx.commit().await?;
            result
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => dispatch!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => dispatch!(db, "revoked_at_ms"),
        })
    }
}
