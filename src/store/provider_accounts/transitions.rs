//! Durable migration intent only. No stop, binding update or launch authority.
#![allow(dead_code)] // Public migration waits for Machine acceptance/recovery.
use super::{StorageBackend, Store};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub session_id: String,
    pub account_id: crate::provider_account::ProviderAccountId,
    pub machine_id: String,
    pub provider: String,
    pub from_version: String,
    pub from_digest: String,
    pub from_generation: u64,
    pub target_login_operation: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Intent {
    pub request: Request,
    pub target: super::logins::NativeCompletion,
}

/// The caller resolves this from the exact trusted target package. Never accept
/// behavior supplied by an untrusted UI or a stale Provider catalog projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BindingCommit {
    pub behavior: cowboy_provider_sdk::ProviderBehaviorContract,
}

impl Store {
    /// One database snapshot of durable progress, not live worker state or
    /// authority to resume. Never returns credentials or native identity data.
    pub(crate) async fn native_session_transition_stage(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<String>> {
        let sql = "SELECT CASE WHEN t.finished THEN 'completed' WHEN EXISTS (SELECT 1 FROM provider_session_transition_commits c WHERE c.owner_user_id=t.owner_user_id AND c.operation_id=t.operation_id) THEN 'binding_committed' WHEN EXISTS (SELECT 1 FROM provider_session_transition_stops s WHERE s.owner_user_id=t.owner_user_id AND s.operation_id=t.operation_id) THEN 'stop_confirmed' ELSE 'planned' END FROM provider_session_transitions t WHERE t.owner_user_id=$1 AND t.operation_id=$2";
        match &self.backend {
            StorageBackend::Postgres(db) => Ok(sqlx::query_scalar(sql)
                .bind(owner)
                .bind(operation)
                .fetch_optional(&db.pool)
                .await?),
            StorageBackend::Sqlite(db) => Ok(sqlx::query_scalar(sql)
                .bind(owner)
                .bind(operation)
                .fetch_optional(&db.pool)
                .await?),
        }
    }

    /// Historical completion, not permission to dispatch or start a worker.
    pub(crate) async fn native_session_transition_finished(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<bool> {
        let sql = "SELECT finished FROM provider_session_transitions WHERE owner_user_id=$1 AND operation_id=$2";
        let finished: Option<bool> = match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_scalar(sql)
                    .bind(owner)
                    .bind(operation)
                    .fetch_optional(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_scalar(sql)
                    .bind(owner)
                    .bind(operation)
                    .fetch_optional(&db.pool)
                    .await?
            }
        };
        Ok(finished.unwrap_or(false))
    }
    pub(crate) async fn native_session_transition_commit(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<BindingCommit>> {
        let sql = "SELECT value FROM provider_session_transition_commits WHERE owner_user_id=$1 AND operation_id=$2";
        let value: Option<String> = match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_scalar(sql)
                    .bind(owner)
                    .bind(operation)
                    .fetch_optional(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_scalar(sql)
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
    /// Historical evidence only, never permission to change or restart a session.
    pub(crate) async fn native_session_transition_stop(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<super::revocation::stops::Evidence>> {
        let query = "SELECT value FROM provider_session_transition_stops WHERE owner_user_id=$1 AND operation_id=$2";
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
    /// Read a historical intent, not permission to dispatch or resume a worker.
    pub(crate) async fn native_session_transition(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<Intent>> {
        let sql = "SELECT value FROM provider_session_transitions WHERE owner_user_id=$1 AND operation_id=$2";
        let value: Option<String> = match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_scalar(sql)
                    .bind(owner)
                    .bind(operation)
                    .fetch_optional(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_scalar(sql)
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
}

impl Intent {
    pub(super) fn validate(&self) -> Result<()> {
        let request = &self.request;
        let target = &self.target;
        ensure!(
            crate::managed_calls::valid_id(&request.session_id)
                && crate::managed_calls::valid_id(&request.target_login_operation)
                && !request.account_id.is_default()
                && request.provider == "claude-code"
                && request.from_generation > 0
                && request.from_generation < target.profile.generation
                && request.account_id == target.profile.account_id
                && request.machine_id == target.machine_id
                && request.provider == target.profile.provider_id,
            "native_transition_scope_mismatch"
        );
        Ok(())
    }
}
