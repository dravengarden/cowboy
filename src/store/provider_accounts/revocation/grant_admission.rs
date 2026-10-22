//! Admission for local vault cleanup, not remote credential delivery.
use super::LocalWipe;
use crate::store::provider_accounts::{Receipt, Registry, grants};
use crate::store::{StorageBackend, Store};
use anyhow::{Result, ensure};

impl Store {
    /// Serialize local cleanup with account edits and grant promotion. The
    /// callback must not await or reenter Store. A commit failure after the
    /// callback is an unknown outcome; retry must inspect the same vault.
    pub async fn with_provider_grant_revocation<T>(
        &self,
        owner: &str,
        operation: &str,
        grant_operation: &str,
        cleanup: impl FnOnce(&grants::Receipt) -> Result<T>,
    ) -> Result<T> {
        self.with_grant_cleanup(
            owner,
            operation,
            grant_operation,
            |_| None,
            |grant| cleanup(grant).map(|result| (result, None)),
        )
        .await
    }

    pub async fn with_provider_local_wipe(
        &self,
        owner: &str,
        operation: &str,
        grant_operation: &str,
        cleanup: impl FnOnce(&grants::Receipt) -> Result<LocalWipe>,
    ) -> Result<LocalWipe> {
        self.with_grant_cleanup(
            owner,
            operation,
            grant_operation,
            |evidence| Some(evidence.clone()),
            |grant| {
                let evidence = cleanup(grant)?;
                Ok((evidence.clone(), Some(evidence)))
            },
        )
        .await
    }

    async fn with_grant_cleanup<T>(
        &self,
        owner: &str,
        operation: &str,
        grant_operation: &str,
        replay: impl FnOnce(&LocalWipe) -> Option<T>,
        cleanup: impl FnOnce(&grants::Receipt) -> Result<(T, Option<LocalWipe>)>,
    ) -> Result<T> {
        macro_rules! revoke { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let mut revocation: Receipt = serde_json::from_str(&value)?;
            let target = revocation.revocation.as_ref().and_then(|plan| plan.grants.iter()
                .find(|target| target.operation_id == grant_operation)).ok_or_else(|| anyhow::anyhow!("grant_revocation_scope_mismatch"))?;
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_grants WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(grant_operation).fetch_one(&mut *tx).await?;
            let grant: grants::Receipt = serde_json::from_str(&value)?;
            ensure!(grant.operation_id == grant_operation && grant.account_id == revocation.account_id
                && grant.machine_id == target.machine_id && !grant.account_id.is_default(), "grant_revocation_scope_mismatch");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            ensure!(registry.accounts.iter().any(|account| account.id == grant.account_id
                && account.provider_id == grant.provider_id && account.revoking && !account.enabled), "account_not_revoking");
            ensure!(!matches!(grant.phase, grants::Phase::Prepared), "grant_cancellation_required");
            if let Some(evidence) = revocation.local_wipes.get(grant_operation)
                && let Some(result) = replay(evidence) {
                tx.commit().await?;
                return Ok(result);
            }
            let plan = revocation.revocation.as_ref().ok_or_else(|| anyhow::anyhow!("revocation_plan_missing"))?;
            for target in &plan.sessions {
                let value: Option<String> = sqlx::query_scalar("SELECT value FROM provider_account_stops WHERE owner_user_id=$1 AND operation_id=$2 AND session_id=$3")
                    .bind(owner).bind(operation).bind(&target.session_id).fetch_optional(&mut *tx).await?;
                let stopped: super::stops::Evidence = serde_json::from_str(
                    &value.ok_or_else(|| anyhow::anyhow!("session_stop_unconfirmed"))?)?;
                ensure!(stopped.account_id == revocation.account_id && stopped.session_id == target.session_id
                    && target.machine_id.as_ref() == Some(&stopped.machine_id), "session_stop_scope_mismatch");
                stopped.require_tree_exit()?;
            }
            for target in &plan.logins {
                let value: String = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                    .bind(owner).bind(&target.operation_id).fetch_one(&mut *tx).await?;
                let login = serde_json::from_str(&value)?;
                validate_login_cleanup(&revocation, target, &login, operation)?;
            }
            let (result, evidence) = cleanup(&grant)?;
            if let Some(evidence) = evidence {
                ensure!(match (&grant.phase, &evidence) {
                    (grants::Phase::Committed { generation }, LocalWipe::Wiped { generation: wiped }) =>
                        grant.expected_generation.checked_add(1) == Some(*generation) && wiped > generation && *wiped < u64::MAX,
                    (grants::Phase::Cancelled, LocalWipe::CancelledDiscarded) => true,
                    _ => false,
                }, "local_wipe_scope_mismatch");
                ensure!(revocation.local_wipes.get(grant_operation).is_none_or(|old| old == &evidence), "local_wipe_conflict");
                revocation.local_wipes.insert(grant_operation.into(), evidence);
                sqlx::query("UPDATE provider_account_operations SET result=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                    .bind(owner).bind(operation).bind(serde_json::to_string(&revocation)?).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            result
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => revoke!(db),
            StorageBackend::Sqlite(db) => revoke!(db),
        })
    }
}

pub(super) fn validate_login_cleanup(
    revocation: &Receipt,
    target: &super::OperationTarget,
    login: &crate::store::provider_accounts::logins::Receipt,
    operation: &str,
) -> Result<()> {
    ensure!(
        login.input.account_id == revocation.account_id
            && login.input.machine_id == target.machine_id
            && login.cancel_requested,
        "login_cleanup_scope_mismatch"
    );
    if login.never_dispatched() {
        return Ok(());
    }
    if let Some(native) = &login.native_logout {
        let scope = &native.observation.request;
        ensure!(
            native.revocation_operation == operation
                && scope.operation_id == operation
                && native.machine_id == target.machine_id
                && scope.account_id == revocation.account_id
                && scope.provider_id == login.provider_id
                && scope.login_request_id == login.request_id
                && login.native_generation == Some(scope.generation)
                && login.dispatch_recipient_fingerprint.as_ref()
                    == Some(&native.recipient_fingerprint)
                && native.observation.succeeded == Some(true)
                && native.observation.profile_cleaned,
            "native_cleanup_unconfirmed"
        );
        ensure!(
            native.observation.process_trees_exited,
            "login_tree_exit_unconfirmed"
        );
        return Ok(());
    }
    ensure!(
        login.native_generation.is_none() && login.native_completion.is_none(),
        "native_cleanup_unconfirmed"
    );
    let evidence = revocation
        .login_stops
        .get(&target.operation_id)
        .ok_or_else(|| anyhow::anyhow!("login_cleanup_unconfirmed"))?;
    let observed = &evidence.observation;
    ensure!(
        evidence.login_operation == target.operation_id
            && evidence.machine_id == target.machine_id
            && login.dispatch_recipient_fingerprint.as_ref()
                == Some(&evidence.recipient_fingerprint)
            && observed.request.account_id == revocation.account_id
            && observed.request.provider_id == login.provider_id
            && observed.request.login_request_id == login.request_id
            && observed.candidate_removed
            && observed.process_started == observed.leader_exited,
        "login_cleanup_scope_mismatch"
    );
    ensure!(
        !observed.process_started || observed.process_tree_exited,
        "login_tree_exit_unconfirmed"
    );
    Ok(())
}
