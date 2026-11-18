//! The kill switch's history-preserving native logout
//! (`docs/antigravity-real-accounts-design.md`, "Kill switch and logout").
//! Unlike revocation it neither disables the account nor deletes its home:
//! the Machine's evidence of one signed-in generation's logout is recorded
//! on that generation's login receipt, which then admits no session.
use super::native_logout::Completion;
use super::{Receipt, Registry, Result, StorageBackend, Store};
use anyhow::ensure;

impl Store {
    /// Record the Machine's logout outcome for the login `login_operation`.
    /// Evidence is recorded whatever the account's current control state:
    /// the Machine has already acted. An exact replay is read-only; a
    /// different recorded logout of the same generation is a conflict.
    pub async fn record_control_logout(
        &self,
        owner: &str,
        login_operation: &str,
        completion: &Completion,
    ) -> Result<Receipt> {
        let scope = &completion.observation.request;
        ensure!(
            !scope.account_id.is_default()
                && scope.generation > 0
                && completion.observation.succeeded.is_some()
                && scope.operation_id == completion.revocation_operation,
            "invalid_native_logout_completion"
        );
        macro_rules! record {
            ($db:expr) => {{
                let mut tx = $db.pool.begin().await?;
                let locked = sqlx::query(
                    "UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1",
                )
                .bind(owner)
                .execute(&mut *tx)
                .await?;
                ensure!(locked.rows_affected() == 1, "account_unavailable");
                let value: String = sqlx::query_scalar(
                    "SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2",
                )
                .bind(owner)
                .bind(login_operation)
                .fetch_one(&mut *tx)
                .await?;
                let mut login: Receipt = serde_json::from_str(&value)?;
                if let Some(old) = &login.native_logout {
                    ensure!(old == completion, "native_logout_conflict");
                    tx.commit().await?;
                    return Ok(login);
                }
                let signed_in = login.native_completion.as_ref().is_some_and(|native| {
                    native.machine_id == completion.machine_id
                        && native.recipient_fingerprint == completion.recipient_fingerprint
                        && native.profile.generation == scope.generation
                });
                ensure!(
                    login.input.account_id == scope.account_id
                        && login.provider_id == scope.provider_id
                        && login.input.machine_id == completion.machine_id
                        && login.request_id == scope.login_request_id
                        && login.native_generation == Some(scope.generation)
                        && login.dispatch_rejection.is_none()
                        && signed_in,
                    "native_logout_scope_mismatch"
                );
                let value: String = sqlx::query_scalar(
                    "SELECT value FROM provider_account_registries WHERE owner_user_id=$1",
                )
                .bind(owner)
                .fetch_one(&mut *tx)
                .await?;
                let registry: Registry = serde_json::from_str(&value)?;
                ensure!(
                    registry.accounts.iter().any(|account| account.id == scope.account_id
                        && account.provider_id == scope.provider_id),
                    "account_unavailable"
                );
                login.native_logout = Some(completion.clone());
                sqlx::query(
                    "UPDATE provider_login_operations SET value=$3 WHERE owner_user_id=$1 AND operation_id=$2",
                )
                .bind(owner)
                .bind(login_operation)
                .bind(serde_json::to_string(&login)?)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                login
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => record!(db),
            StorageBackend::Sqlite(db) => record!(db),
        })
    }
}
