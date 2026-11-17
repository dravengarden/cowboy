//! Account-vault credential commits in two owner-locked steps.
//!
//! 1. `mark_portable`: the account is available and serves the caller; a
//!    subscription login is still its own durable, uncancelled request,
//!    dispatched under the account's current admission to the Machine that
//!    is still enrolled; the caller's precheck admits the candidate. Only
//!    then is the identity bound (or matched) and the account marked for
//!    revocation wipes, and that commits on its own.
//! 2. The credential write runs under a fresh lock that rechecks the
//!    account, the bound identity and (for a login) the request, and the
//!    login's receipt records the committed vault generation.
//!
//! A vault credential therefore never exists without a durable revocation
//! mark, and a failed or rejected candidate never binds an identity.
use super::{Account, ProviderAccountId, Registry, Result, StorageBackend, Store, ensure};
use crate::provider_account::NativeIdentity;

/// The login a subscription sign-in commit belongs to.
#[derive(Clone, Copy, Debug)]
pub struct SignInLogin<'a> {
    pub request_id: &'a str,
    pub machine_id: &'a str,
}

/// Recheck the durable login a sign-in commit belongs to, under the owner
/// lock. With `revision`, also its dispatch under the current admission.
macro_rules! login_admitted {
    ($tx:expr, $revoked:literal, $owner:expr, $account:expr, $login:expr, $admission:expr) => {{
        let value: Option<String> = sqlx::query_scalar(
            "SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND request_id=$2",
        )
        .bind($owner)
        .bind($login.request_id)
        .fetch_optional(&mut *$tx)
        .await?;
        let receipt: super::super::logins::Receipt = serde_json::from_str(
            &value.ok_or_else(|| anyhow::anyhow!("login_operation_not_found"))?,
        )?;
        ensure!(
            &receipt.input.account_id == $account
                && receipt.input.machine_id == $login.machine_id
                && !receipt.cancel_requested
                && receipt.dispatch_rejection.is_none(),
            "authentication_request_unavailable"
        );
        let fingerprint = receipt
            .dispatch_recipient_fingerprint
            .clone()
            .ok_or_else(|| anyhow::anyhow!("login_enrollment_unknown"))?;
        let key: Option<Option<String>> = sqlx::query_scalar(concat!(
            "SELECT encryption_public_key FROM machines WHERE id=$1 AND ",
            $revoked,
            " IS NULL"
        ))
        .bind($login.machine_id)
        .fetch_optional(&mut *$tx)
        .await?;
        ensure!(
            key.flatten()
                .is_some_and(|key| crate::digest::hex_sha256(key.as_bytes()) == fingerprint),
            "machine_enrollment_changed"
        );
        if let Some(account) = $admission {
            let account: &Account = account;
            let revision = receipt
                .dispatch_registry_revision
                .ok_or_else(|| anyhow::anyhow!("login_admission_unknown"))?;
            ensure!(
                account
                    .admission_revision
                    .is_none_or(|admitted| admitted <= revision),
                "login_admission_revision_changed"
            );
        }
        receipt
    }};
}

impl Store {
    /// Step 1: admit, then bind `identity` and mark the account durably.
    /// Returns the account's highest native generation (the vault floor).
    pub(super) async fn mark_portable(
        &self,
        owner: &str,
        account: &ProviderAccountId,
        identity: Option<&NativeIdentity>,
        serves: &impl Fn(&Account) -> bool,
        login: Option<&SignInLogin<'_>>,
        precheck: impl FnOnce() -> Result<()>,
    ) -> Result<u64> {
        macro_rules! mark { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let mut registry: Registry = serde_json::from_str(&value)?;
            let index = registry.accounts.iter().position(|a| &a.id == account && a.available() && serves(a))
                .ok_or_else(|| anyhow::anyhow!("account_unavailable"))?;
            let mut admitted = match login {
                Some(login) => Some(login_admitted!(tx, $revoked, owner, account, login, Some(&registry.accounts[index]))),
                None => None,
            };
            precheck()?;
            let (revision, marked) = (registry.revision, registry.accounts[index].portable_credentials);
            if let Some(identity) = identity {
                let provider_id = registry.accounts[index].provider_id.clone();
                registry.apply(&super::super::Change::BindIdentity { id: account.clone(), provider_id, identity: identity.clone() })
                    .map_err(|code| anyhow::anyhow!(code))?;
            }
            // This login's own first bind advanced the account's admission: a
            // retry of the same login stays admitted; nothing else moves.
            if let (Some(login), Some(receipt)) = (login, admitted.as_mut())
                && registry.revision != revision
            {
                receipt.dispatch_registry_revision = registry.accounts[index].admission_revision;
                sqlx::query("UPDATE provider_login_operations SET value=$1 WHERE owner_user_id=$2 AND request_id=$3")
                    .bind(serde_json::to_string(&*receipt)?).bind(owner).bind(login.request_id).execute(&mut *tx).await?;
            }
            // Marking alone bumps no revision: no owner-visible field changes.
            registry.accounts[index].portable_credentials = true;
            if registry.revision != revision || !marked {
                sqlx::query("UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3")
                    .bind(registry.revision).bind(serde_json::to_string(&registry)?).bind(owner).execute(&mut *tx).await?;
            }
            let floor: Option<i64> = sqlx::query_scalar("SELECT MAX(generation) FROM provider_native_generations WHERE owner_user_id=$1 AND account_id=$2")
                .bind(owner).bind(account.as_str()).fetch_one(&mut *tx).await?;
            tx.commit().await?;
            u64::try_from(floor.unwrap_or(0))?
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => mark!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => mark!(db, "revoked_at_ms"),
        })
    }

    /// Commit one subscription sign-in: step 1, then `write(floor)` (the vault
    /// write, returning its generation) under a fresh lock that rechecks the
    /// account, its bound identity and the login, which then records the
    /// generation. `precheck` (step 1) admits the live candidate.
    pub async fn commit_subscription_sign_in<T>(
        &self,
        owner: &str,
        account: &ProviderAccountId,
        identity: &NativeIdentity,
        login: SignInLogin<'_>,
        precheck: impl FnOnce() -> Result<()>,
        write: impl FnOnce(u64) -> Result<(T, u64)>,
    ) -> Result<T> {
        let serves = |_: &Account| true;
        let floor = self
            .mark_portable(
                owner,
                account,
                Some(identity),
                &serves,
                Some(&login),
                precheck,
            )
            .await?;
        macro_rules! commit { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            let current = registry.accounts.iter().find(|a| &a.id == account && a.available())
                .ok_or_else(|| anyhow::anyhow!("account_unavailable"))?;
            let fingerprint = identity.fingerprint(&current.provider_id).map_err(|code| anyhow::anyhow!(code))?;
            ensure!(current.portable_credentials && current.identity_fingerprint.as_ref() == Some(&fingerprint), "native_identity_mismatch");
            let mut receipt = login_admitted!(tx, $revoked, owner, account, login, None::<&Account>);
            let (result, generation) = write(floor)?;
            receipt.vault_generation = Some(generation);
            sqlx::query("UPDATE provider_login_operations SET value=$1 WHERE owner_user_id=$2 AND request_id=$3")
                .bind(serde_json::to_string(&receipt)?).bind(owner).bind(login.request_id).execute(&mut *tx).await?;
            tx.commit().await?;
            result
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => commit!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => commit!(db, "revoked_at_ms"),
        })
    }
}
