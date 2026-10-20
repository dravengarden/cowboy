//! Durable at-most-once native-login dispatch. A reservation never proves send
//! or success. Only the caller receiving a fresh committed claim may dispatch;
//! after an uncertain commit or process loss, replay observes and never resends.
use super::{ProviderAccountId, Registry, StorageBackend, Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};

mod discovery;
mod dispatch;
mod native_admission;
mod native_selection;
pub(crate) use dispatch::DispatchScope;
mod native_binding;
pub(crate) mod native_logout;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub account_id: ProviderAccountId,
    pub machine_id: String,
    pub provider_version: String,
    pub generation_digest: String,
    pub method: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub request_id: String,
    pub provider_id: String,
    pub input: Request,
    /// Durable intent only, never evidence that a native process has exited.
    #[serde(default)]
    pub cancel_requested: bool,
    /// Written only by the fresh claimant after a definite pre-enqueue refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch_rejection: Option<DispatchRejection>,
    /// Historical Machine evidence only; not a runtime grant or dispatch lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_completion: Option<NativeCompletion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_logout: Option<native_logout::Completion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_generation: Option<u64>,
    /// Historical binding fence, not a reusable runtime authorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_binding_revision: Option<i64>,
    /// Enrollment captured at dispatch, retained across Controller restart.
    /// Missing on historical claims: never infer it from current enrollment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch_recipient_fingerprint: Option<String>,
    /// Original account admission revision; recovery must not adopt a newer one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch_registry_revision: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeCompletion {
    pub machine_id: String,
    pub recipient_fingerprint: String,
    pub profile: crate::provider_account::NativeProfileReceipt,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum DispatchRejection {
    #[serde(rename = "authentication_cleanup_required")]
    CleanupRequired,
    #[serde(rename = "authentication_start_rejected")]
    StartRejected,
}

impl Receipt {
    /// A pre-dispatch refusal, not merely absence of a dispatch receipt.
    /// `StartRejected` may follow an attempted enqueue and is deliberately excluded.
    pub(crate) fn never_dispatched(&self) -> bool {
        self.dispatch_rejection == Some(DispatchRejection::CleanupRequired)
            && self.dispatch_recipient_fingerprint.is_none()
            && self.dispatch_registry_revision.is_none()
            && self.native_completion.is_none()
            && self.native_logout.is_none()
            && self.native_binding_revision.is_none()
    }

    pub(crate) fn matches(&self, input: &Request) -> bool {
        &self.input == input
    }
}

#[derive(Debug)]
pub(crate) struct Claim {
    pub receipt: Receipt,
    pub fresh: bool,
}

impl Store {
    pub(crate) async fn provider_login_operation_for_request(
        &self,
        owner: &str,
        request: &str,
    ) -> Result<Option<String>> {
        let query = "SELECT operation_id FROM provider_login_operations WHERE owner_user_id=$1 AND request_id=$2";
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(request)
                    .fetch_optional(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(request)
                    .fetch_optional(&db.pool)
                    .await?
            }
        })
    }
    /// The caller must authenticate the sending Machine and verify its signed
    /// release/identity recipe first. This journal is deliberately separate
    /// from grant admission: recording evidence never enables a session.
    pub(crate) async fn record_native_login_completion(
        &self,
        owner: &str,
        operation: &str,
        completion: &NativeCompletion,
    ) -> Result<Receipt> {
        let profile = &completion.profile;
        anyhow::ensure!(
            profile.generation > 0 && !profile.account_id.is_default(),
            "invalid_native_completion"
        );
        profile
            .identity
            .fingerprint(&profile.provider_id)
            .map_err(anyhow::Error::msg)?;
        macro_rules! record { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            anyhow::ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let mut receipt: Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(receipt.request_id == profile.request_id && receipt.provider_id == profile.provider_id
                && (receipt.native_generation == Some(profile.generation)
                    || (receipt.native_generation.is_none() && receipt.native_completion.as_ref() == Some(completion)))
                && receipt.input.account_id == profile.account_id && receipt.input.machine_id == completion.machine_id
                && receipt.input.method == profile.method && receipt.input.provider_version == profile.provider_version
                && receipt.input.generation_digest == profile.generation_digest, "native_completion_scope_mismatch");
            if let Some(old) = &receipt.native_completion {
                anyhow::ensure!(old == completion, "native_completion_conflict");
                // Idempotent historical read, even after subsequent revocation.
                tx.commit().await?;
                return Ok(receipt);
            }
            anyhow::ensure!(receipt.dispatch_recipient_fingerprint.as_ref().is_none_or(|fingerprint|
                fingerprint == &completion.recipient_fingerprint), "machine_enrollment_changed");
            anyhow::ensure!(!receipt.cancel_requested && receipt.dispatch_rejection.is_none(), "login_not_admitted");
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&completion.machine_id).execute(&mut *tx).await?;
            anyhow::ensure!(machine.rows_affected() == 1, "machine_unavailable");
            let recipient: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&completion.machine_id).fetch_one(&mut *tx).await?;
            anyhow::ensure!(recipient.as_ref().is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == completion.recipient_fingerprint), "machine_enrollment_changed");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            anyhow::ensure!(registry.accounts.iter().any(|account| account.id == profile.account_id
                && account.provider_id == profile.provider_id && account.available()), "account_unavailable");
            receipt.native_completion = Some(completion.clone());
            sqlx::query("UPDATE provider_login_operations SET value=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            receipt
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => record!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => record!(db, "revoked_at_ms"),
        })
    }

    pub(crate) async fn reject_provider_login(
        &self,
        owner: &str,
        operation: &str,
        claim: &Claim,
        reason: DispatchRejection,
    ) -> Result<Receipt> {
        anyhow::ensure!(claim.fresh, "fresh_login_claim_required");
        macro_rules! reject { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            let value: String = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let mut receipt: Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(receipt.request_id == claim.receipt.request_id && receipt.provider_id == claim.receipt.provider_id &&
                receipt.input == claim.receipt.input, "login_claim_mismatch");
            anyhow::ensure!(receipt.dispatch_rejection.is_none_or(|old| old == reason), "login_rejection_mismatch");
            anyhow::ensure!(receipt.native_completion.is_none(), "login_already_completed");
            receipt.dispatch_rejection = Some(reason);
            sqlx::query("UPDATE provider_login_operations SET value=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            receipt
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => reject!(db),
            StorageBackend::Sqlite(db) => reject!(db),
        })
    }

    pub(crate) async fn cancel_provider_login(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<Receipt>> {
        macro_rules! cancel { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            // Same lock order as claim; cancellation cannot create a receipt
            // or mutate the request identity, input hash, or dispatch authority.
            sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            let value: Option<String> = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
            let receipt = if let Some(value) = value {
                let mut receipt: Receipt = serde_json::from_str(&value)?;
                receipt.cancel_requested = true;
                sqlx::query("UPDATE provider_login_operations SET value=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                    .bind(owner).bind(operation).bind(serde_json::to_string(&receipt)?)
                    .execute(&mut *tx).await?;
                Some(receipt)
            } else { None };
            tx.commit().await?;
            receipt
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => cancel!(db),
            StorageBackend::Sqlite(db) => cancel!(db),
        })
    }

    pub(crate) async fn provider_login_receipt(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<Receipt>> {
        let query = "SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2";
        let value: Option<String> = match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(operation)
                    .fetch_optional(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(operation)
                    .fetch_optional(&db.pool)
                    .await?
            }
        };
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub(crate) async fn claim_provider_login(
        &self,
        owner: &str,
        operation: &str,
        input: &Request,
    ) -> Result<Claim> {
        anyhow::ensure!(
            crate::managed_calls::valid_id(operation) && !input.account_id.is_default(),
            "invalid_login_operation"
        );
        for value in [
            &input.machine_id,
            &input.provider_version,
            &input.generation_digest,
            &input.method,
        ] {
            anyhow::ensure!(
                !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control),
                "invalid_login_input"
            );
        }
        let hash = crate::admin::hex_sha256(&serde_json::to_vec(input)?);
        macro_rules! claim { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            anyhow::ensure!(locked.rows_affected() == 1, "account_unavailable");
            let old: Option<(String,String)> = sqlx::query_as("SELECT request_hash,value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
            if let Some((old_hash,value)) = old {
                anyhow::ensure!(old_hash == hash, "operation_reused");
                let receipt = serde_json::from_str(&value)?;
                tx.commit().await?;
                return Ok(Claim { receipt, fresh:false });
            }
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            let account = registry.accounts.iter().find(|account| account.id == input.account_id && account.available())
                .ok_or_else(|| anyhow::anyhow!("account_unavailable"))?;
            let native_generation = if account.provider_id == "claude-code" {
                let generation: i64 = sqlx::query_scalar("INSERT INTO provider_native_generations (owner_user_id,account_id,machine_id,generation) VALUES ($1,$2,$3,1) ON CONFLICT (owner_user_id,account_id,machine_id) DO UPDATE SET generation=provider_native_generations.generation+1 WHERE provider_native_generations.generation < 9223372036854775807 RETURNING generation")
                    .bind(owner).bind(input.account_id.as_str()).bind(&input.machine_id).fetch_one(&mut *tx).await?;
                Some(u64::try_from(generation)?)
            } else { None };
            let receipt = Receipt { request_id:format!("provider-login-{}", uuid::Uuid::new_v4()), provider_id:account.provider_id.clone(), input:input.clone(), cancel_requested:false, dispatch_rejection:None, native_completion:None, native_logout:None, native_generation, native_binding_revision:None, dispatch_recipient_fingerprint:None, dispatch_registry_revision:None };
            sqlx::query("INSERT INTO provider_login_operations (owner_user_id,operation_id,request_hash,request_id,value) VALUES ($1,$2,$3,$4,$5)")
                .bind(owner).bind(operation).bind(&hash).bind(&receipt.request_id).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            Claim { receipt, fresh:true }
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => claim!(db),
            StorageBackend::Sqlite(db) => claim!(db),
        })
    }
}

#[cfg(test)]
mod tests;
