//! The kill switch's history-preserving native logout
//! (`docs/antigravity-real-accounts-design.md`, "Kill switch and logout").
//! Unlike revocation it neither disables the account nor deletes its home:
//! the Machine's evidence of one signed-in generation's logout is recorded
//! on that generation's login receipt, which then admits no session.
use super::native_logout::Completion;
use super::{Receipt, Registry, Result, StorageBackend, Store};
use anyhow::ensure;

/// One Machine's sign-in a control logout targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogoutTarget {
    pub machine_id: String,
    pub login_operation: String,
    pub login_request_id: String,
    pub generation: u64,
    /// The Machine enrollment the sign-in was dispatched to.
    pub recipient_fingerprint: String,
}

impl Store {
    /// Each Machine's newest dispatched native sign-in of `account` that has
    /// no recorded logout, whatever its recorded outcome: the Controller's
    /// receipt is not proof the Machine holds no token, so the Machine's
    /// daemon decides what the logout clears.
    pub async fn native_logout_targets(
        &self,
        owner: &str,
        account: &crate::provider_account::ProviderAccountId,
    ) -> Result<Vec<LogoutTarget>> {
        let query =
            "SELECT operation_id,value FROM provider_login_operations WHERE owner_user_id=$1";
        let rows: Vec<(String, String)> = match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_as(query)
                    .bind(owner)
                    .fetch_all(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_as(query)
                    .bind(owner)
                    .fetch_all(&db.pool)
                    .await?
            }
        };
        let mut newest: std::collections::BTreeMap<String, LogoutTarget> =
            std::collections::BTreeMap::new();
        for (operation, value) in rows {
            let Ok(login) = serde_json::from_str::<Receipt>(&value) else {
                continue;
            };
            let (Some(generation), Some(fingerprint)) = (
                login.native_generation,
                login.dispatch_recipient_fingerprint.clone(),
            ) else {
                continue;
            };
            if login.input.account_id != *account
                || login.dispatch_rejection.is_some()
                || login.native_logout.is_some()
            {
                continue;
            }
            let machine = login.input.machine_id.clone();
            if newest
                .get(&machine)
                .is_some_and(|known| known.generation >= generation)
            {
                continue;
            }
            newest.insert(
                machine.clone(),
                LogoutTarget {
                    machine_id: machine,
                    login_operation: operation,
                    login_request_id: login.request_id,
                    generation,
                    recipient_fingerprint: fingerprint,
                },
            );
        }
        Ok(newest.into_values().collect())
    }

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
                // A completed sign-in, or a dispatched one whose completion the
                // Controller never recorded (the daemon may still hold its
                // token), bound to the enrollment it was dispatched to.
                let signed_in = match login.native_completion.as_ref() {
                    Some(native) => {
                        native.machine_id == completion.machine_id
                            && native.recipient_fingerprint == completion.recipient_fingerprint
                            && native.profile.generation == scope.generation
                    }
                    None => {
                        login.dispatch_recipient_fingerprint.as_ref()
                            == Some(&completion.recipient_fingerprint)
                    }
                };
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
