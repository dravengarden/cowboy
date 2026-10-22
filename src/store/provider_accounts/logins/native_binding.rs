//! Bind verified native identity separately from historical completion evidence.
//! This operation never issues a grant or authorizes a worker.
use super::{NativeCompletion, Receipt, Registry, Result, StorageBackend, Store};
use crate::store::provider_accounts::Change;

impl Store {
    pub async fn bind_native_login_identity(
        &self,
        owner: &str,
        operation: &str,
        completion: &NativeCompletion,
        expected_revision: i64,
    ) -> Result<Registry> {
        let profile = &completion.profile;
        let fingerprint = profile
            .identity
            .fingerprint(&profile.provider_id)
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            profile.provider_id == "claude-code" && !profile.account_id.is_default(),
            "native_account_required"
        );
        macro_rules! bind { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            anyhow::ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let mut receipt: Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(!receipt.cancel_requested && receipt.dispatch_rejection.is_none(), "login_not_admitted");
            anyhow::ensure!(receipt.native_completion.as_ref() == Some(completion)
                && receipt.native_generation == Some(profile.generation), "native_completion_scope_mismatch");
            let generation: i64 = sqlx::query_scalar("SELECT generation FROM provider_native_generations WHERE owner_user_id=$1 AND account_id=$2 AND machine_id=$3")
                .bind(owner).bind(profile.account_id.as_str()).bind(&completion.machine_id).fetch_one(&mut *tx).await?;
            anyhow::ensure!(u64::try_from(generation)? == profile.generation, "native_generation_superseded");
            // Same owner -> Machine lock order as grant admission and completion.
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&completion.machine_id).execute(&mut *tx).await?;
            anyhow::ensure!(machine.rows_affected() == 1, "machine_unavailable");
            let recipient: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&completion.machine_id).fetch_one(&mut *tx).await?;
            anyhow::ensure!(recipient.as_ref().is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == completion.recipient_fingerprint), "machine_enrollment_changed");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let mut registry: Registry = serde_json::from_str(&value)?;
            let account = registry.accounts.iter().find(|account| account.id == profile.account_id
                && account.provider_id == profile.provider_id && account.available()).ok_or_else(|| anyhow::anyhow!("account_unavailable"))?;
            if let Some(revision) = receipt.native_binding_revision {
                // Only this operation's durable binding can justify a retry.
                // A matching identity from another login cannot replace it.
                anyhow::ensure!(account.admits_binding(revision, registry.revision)
                    && account.identity_fingerprint.as_ref() == Some(&fingerprint), "native_binding_changed");
                tx.commit().await?;
                return Ok(registry);
            }
            anyhow::ensure!(receipt.dispatch_registry_revision.is_none_or(|original| original == expected_revision), "login_admission_revision_changed");
            anyhow::ensure!(registry.revision == expected_revision, "revision_conflict");
            registry.apply(&Change::BindIdentity { id:profile.account_id.clone(), provider_id:profile.provider_id.clone(), identity:profile.identity.clone() })
                .map_err(anyhow::Error::msg)?;
            sqlx::query("UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3")
                .bind(registry.revision).bind(serde_json::to_string(&registry)?).bind(owner).execute(&mut *tx).await?;
            receipt.native_binding_revision = Some(registry.revision);
            sqlx::query("UPDATE provider_login_operations SET value=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            registry
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => bind!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => bind!(db, "revoked_at_ms"),
        })
    }
}
