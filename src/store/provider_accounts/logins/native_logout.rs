//! Historical native command outcome, not account-wide completion or authority.
use super::{Receipt, Registry, Result, StorageBackend, Store};
use anyhow::ensure;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    pub revocation_operation: String,
    pub machine_id: String,
    pub recipient_fingerprint: String,
    pub observation: crate::provider_account::NativeLogoutObservation,
}

impl Store {
    /// Enqueue synchronously while owner and enrollment remain locked. The
    /// proposal has no outcome yet; the callback must return the response
    /// future without polling it. Recorded failure or confirmed cleanup
    /// suppresses redispatch; success alone still permits scoped cleanup.
    pub async fn with_native_logout_dispatch<T>(
        &self,
        owner: &str,
        login_operation: &str,
        proposal: &Completion,
        dispatch: impl FnOnce() -> Result<T>,
    ) -> Result<Option<T>> {
        ensure!(
            proposal.observation.succeeded.is_none()
                && !proposal.observation.profile_cleaned
                && !proposal.observation.process_trees_exited,
            "logout_outcome_not_expected"
        );
        self.native_logout_admission(owner, login_operation, proposal, false, || {
            dispatch().map(Some)
        })
        .await
        .map(|(_, pending)| pending)
    }

    /// Admit a fresh observation under owner/enrollment locks. Exact historical
    /// replay is read-only, including after later enrollment changes.
    pub async fn record_native_logout_completion(
        &self,
        owner: &str,
        login_operation: &str,
        completion: &Completion,
        connection_is_current: impl FnOnce() -> bool,
    ) -> Result<Receipt> {
        self.native_logout_admission(owner, login_operation, completion, true, || {
            ensure!(connection_is_current(), "machine_connection_changed");
            Ok(())
        })
        .await
        .map(|(receipt, ())| receipt)
    }

    async fn native_logout_admission<T: Default>(
        &self,
        owner: &str,
        login_operation: &str,
        completion: &Completion,
        record: bool,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<(Receipt, T)> {
        let scope = &completion.observation.request;
        ensure!(
            !scope.account_id.is_default()
                && scope.generation > 0
                && (!record || completion.observation.succeeded.is_some())
                && (!completion.observation.profile_cleaned
                    || completion.observation.succeeded == Some(true))
                && (!completion.observation.process_trees_exited
                    || completion.observation.succeeded.is_some())
                && scope.operation_id == completion.revocation_operation,
            "invalid_native_logout_completion"
        );
        macro_rules! record { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(login_operation).fetch_one(&mut *tx).await?;
            let mut login: Receipt = serde_json::from_str(&value)?;
            if let Some(old) = &login.native_logout {
                let mut expected = completion.clone();
                if !record {
                    expected.observation.succeeded = old.observation.succeeded;
                    expected.observation.profile_cleaned = old.observation.profile_cleaned;
                    expected.observation.process_trees_exited = old.observation.process_trees_exited;
                }
                let cleanup_pending = old.observation.succeeded == Some(true) && !old.observation.profile_cleaned;
                let cleanup_progress = record && cleanup_pending && expected.observation.profile_cleaned;
                if cleanup_progress {
                    expected.observation.profile_cleaned = false;
                }
                ensure!(old == &expected, "native_logout_conflict");
                if !cleanup_progress && (record || !cleanup_pending) {
                    tx.commit().await?;
                    return Ok((login, T::default()));
                }
            }
            ensure!(login.input.account_id == scope.account_id && login.provider_id == scope.provider_id
                && login.input.machine_id == completion.machine_id && login.request_id == scope.login_request_id
                && login.native_generation == Some(scope.generation) && login.cancel_requested
                && login.dispatch_rejection.is_none()
                && login.dispatch_recipient_fingerprint.as_ref() == Some(&completion.recipient_fingerprint), "native_logout_scope_mismatch");
            let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(&completion.revocation_operation).fetch_one(&mut *tx).await?;
            let revocation: super::super::Receipt = serde_json::from_str(&value)?;
            let plan = revocation.revocation.as_ref().ok_or_else(|| anyhow::anyhow!("revocation_plan_missing"))?;
            ensure!(revocation.account_id == scope.account_id && plan.logins.iter().any(|target|
                target.operation_id == login_operation && target.machine_id == completion.machine_id), "native_logout_not_planned");
            for target in &plan.sessions {
                let value: Option<String> = sqlx::query_scalar("SELECT value FROM provider_account_stops WHERE owner_user_id=$1 AND operation_id=$2 AND session_id=$3")
                    .bind(owner).bind(&completion.revocation_operation).bind(&target.session_id).fetch_optional(&mut *tx).await?;
                let stopped: super::super::revocation::stops::Evidence = serde_json::from_str(
                    &value.ok_or_else(|| anyhow::anyhow!("session_stop_unconfirmed"))?)?;
                ensure!(stopped.account_id == scope.account_id && stopped.session_id == target.session_id
                    && target.machine_id.as_ref() == Some(&stopped.machine_id), "session_stop_scope_mismatch");
                stopped.require_tree_exit()?;
            }
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            ensure!(registry.accounts.iter().any(|account| account.id == scope.account_id
                && account.provider_id == scope.provider_id && account.revoking && !account.enabled), "account_not_revoking");
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&completion.machine_id).execute(&mut *tx).await?;
            ensure!(machine.rows_affected() == 1, "machine_unavailable");
            let key: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&completion.machine_id).fetch_one(&mut *tx).await?;
            ensure!(key.is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == completion.recipient_fingerprint), "machine_enrollment_changed");
            let output = action()?;
            if record {
                login.native_logout = Some(completion.clone());
                sqlx::query("UPDATE provider_login_operations SET value=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                    .bind(owner).bind(login_operation).bind(serde_json::to_string(&login)?).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            (login, output)
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => record!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => record!(db, "revoked_at_ms"),
        })
    }
}
