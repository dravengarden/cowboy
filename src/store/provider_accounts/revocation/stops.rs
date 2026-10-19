//! Stop evidence does not authorize a session or establish native logout.
#![allow(dead_code)] // Account-wide lifecycle admission remains gated.
use super::super::{ProviderAccountId, Receipt, Registry, StorageBackend, Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Evidence {
    pub account_id: ProviderAccountId,
    pub session_id: String,
    pub machine_id: String,
    pub recipient_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_exit: Option<crate::provider_account::WorkerExitObservation>,
}

impl Evidence {
    pub(crate) fn require_transition_stop(&self, operation: &str) -> Result<()> {
        self.require_tree_exit()?;
        let command = crate::provider_account::account_stop_command_id(
            &format!("migration:{operation}"),
            &self.session_id,
            &self.account_id,
        );
        anyhow::ensure!(
            self.tree_exit
                .as_ref()
                .and_then(|tree| tree.request.stop_command_id.as_ref())
                == Some(&command),
            "transition_stop_command_mismatch"
        );
        Ok(())
    }
    pub(crate) fn require_tree_exit(&self) -> Result<()> {
        let tree = self
            .tree_exit
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("session_tree_exit_unconfirmed"))?;
        anyhow::ensure!(
            tree.request.account_id == self.account_id
                && tree.request.session_id == self.session_id
                && !tree.launch_id.is_nil(),
            "session_tree_exit_scope_mismatch"
        );
        Ok(())
    }
}

impl Store {
    /// Queue an idempotent stop while account and enrollment admission is
    /// locked. The callback must check the live runtime and must not await or
    /// reenter Store. A commit error after enqueue is an unknown outcome.
    pub(crate) async fn dispatch_provider_account_stop<T>(
        &self,
        owner: &str,
        operation: &str,
        scope: &Evidence,
        enqueue: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        anyhow::ensure!(!scope.account_id.is_default(), "named_account_required");
        macro_rules! dispatch { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            anyhow::ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let receipt: Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(receipt.account_id == scope.account_id && receipt.revocation.as_ref().is_some_and(|plan|
                plan.sessions.iter().any(|target| target.session_id == scope.session_id
                    && target.machine_id.as_ref() == Some(&scope.machine_id))), "stop_scope_mismatch");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            anyhow::ensure!(registry.accounts.iter().any(|account| account.id == scope.account_id
                && account.revoking && !account.enabled), "account_not_revoking");
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&scope.machine_id).execute(&mut *tx).await?;
            anyhow::ensure!(machine.rows_affected() == 1, "machine_unavailable");
            let key: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&scope.machine_id).fetch_one(&mut *tx).await?;
            anyhow::ensure!(key.is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == scope.recipient_fingerprint), "machine_enrollment_changed");
            let result = enqueue()?;
            tx.commit().await?;
            result
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => dispatch!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => dispatch!(db, "revoked_at_ms"),
        })
    }

    /// Historical read only, including after enrollment changes. Never use this
    /// receipt to authorize dispatch to a currently enrolled Machine.
    pub(crate) async fn provider_account_stop(
        &self,
        owner: &str,
        operation: &str,
        session: &str,
    ) -> Result<Option<Evidence>> {
        let query = "SELECT value FROM provider_account_stops WHERE owner_user_id=$1 AND operation_id=$2 AND session_id=$3";
        let value: Option<String> = match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(operation)
                    .bind(session)
                    .fetch_optional(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(operation)
                    .bind(session)
                    .fetch_optional(&db.pool)
                    .await?
            }
        };
        Ok(value
            .map(|value| serde_json::from_str(&value))
            .transpose()?)
    }

    /// Call only after the scoped transport reports Stopped. The synchronous
    /// callback must revalidate that result's authenticated live connection;
    /// it runs after validation reads under owner and Machine locks. It must not
    /// await or reenter Store. Identical historical evidence is idempotent even
    /// after enrollment changes, without invoking the admission callback.
    pub(crate) async fn record_provider_account_stop(
        &self,
        owner: &str,
        operation: &str,
        evidence: &Evidence,
        connection_is_current: impl FnOnce() -> bool,
    ) -> Result<Evidence> {
        anyhow::ensure!(!evidence.account_id.is_default(), "named_account_required");
        anyhow::ensure!(
            evidence
                .tree_exit
                .as_ref()
                .is_none_or(|tree| tree.request.account_id == evidence.account_id
                    && tree.request.session_id == evidence.session_id
                    && !tree.launch_id.is_nil()),
            "worker_exit_scope_mismatch"
        );
        macro_rules! record { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            anyhow::ensure!(locked.rows_affected() == 1, "account_unavailable");
            let old: Option<String> = sqlx::query_scalar("SELECT value FROM provider_account_stops WHERE owner_user_id=$1 AND operation_id=$2 AND session_id=$3")
                .bind(owner).bind(operation).bind(&evidence.session_id).fetch_optional(&mut *tx).await?;
            if let Some(old) = old {
                let old: Evidence = serde_json::from_str(&old)?;
                if &old == evidence { return Ok(old); }
                let mut upgraded = old.clone();
                upgraded.tree_exit.clone_from(&evidence.tree_exit);
                anyhow::ensure!(old.tree_exit.is_none() && evidence.tree_exit.is_some()
                    && &upgraded == evidence, "stop_evidence_conflict");
                // A first tree observation is a fresh admission below, never
                // a historical replay that can skip enrollment/connection checks.
            } else {
                anyhow::ensure!(evidence.tree_exit.is_none(), "session_stop_unconfirmed");
            }
            let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let receipt: Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(receipt.account_id == evidence.account_id && receipt.revocation.as_ref().is_some_and(|plan|
                plan.sessions.iter().any(|target| target.session_id == evidence.session_id
                    && target.machine_id.as_ref() == Some(&evidence.machine_id))), "stop_scope_mismatch");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            anyhow::ensure!(registry.accounts.iter().any(|account| account.id == evidence.account_id
                && account.revoking && !account.enabled), "account_not_revoking");
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&evidence.machine_id).execute(&mut *tx).await?;
            anyhow::ensure!(machine.rows_affected() == 1, "machine_unavailable");
            let key: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&evidence.machine_id).fetch_one(&mut *tx).await?;
            anyhow::ensure!(key.is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == evidence.recipient_fingerprint), "machine_enrollment_changed");
            anyhow::ensure!(connection_is_current(), "machine_connection_changed");
            sqlx::query("INSERT INTO provider_account_stops (owner_user_id,operation_id,session_id,value) VALUES ($1,$2,$3,$4) ON CONFLICT (owner_user_id,operation_id,session_id) DO UPDATE SET value=excluded.value")
                .bind(owner).bind(operation).bind(&evidence.session_id).bind(serde_json::to_string(evidence)?).execute(&mut *tx).await?;
            tx.commit().await?;
            evidence.clone()
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => record!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => record!(db, "revoked_at_ms"),
        })
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    #[test]
    fn cleanup_requires_scoped_tree_exit_not_leader_ack() {
        let account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let mut evidence = Evidence {
            account_id,
            session_id: "session".into(),
            machine_id: "machine".into(),
            recipient_fingerprint: "key".into(),
            tree_exit: None,
        };
        assert_eq!(
            evidence.require_tree_exit().unwrap_err().to_string(),
            "session_tree_exit_unconfirmed"
        );
        let tree = crate::provider_account::WorkerExitObservation {
            request: crate::provider_account::WorkerExitRequest {
                stop_command_id: None,
                account_id: evidence.account_id.clone(),
                session_id: evidence.session_id.clone(),
            },
            launch_id: uuid::Uuid::new_v4(),
        };
        evidence.tree_exit = Some(tree.clone());
        evidence.require_tree_exit().unwrap();
        for kind in ["session", "account", "launch"] {
            let mut wrong = tree.clone();
            match kind {
                "session" => wrong.request.session_id = "other".into(),
                "account" => {
                    wrong.request.account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
                }
                _ => wrong.launch_id = uuid::Uuid::nil(),
            }
            evidence.tree_exit = Some(wrong);
            assert_eq!(
                evidence.require_tree_exit().unwrap_err().to_string(),
                "session_tree_exit_scope_mismatch"
            );
        }
    }
}
