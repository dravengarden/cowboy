//! Account-sandbox removal evidence of a revocation. Revoking a named-only
//! (Antigravity) account must destroy what the Machine's account-sandbox
//! daemon created for it: the account's persistent home (its history) and
//! its system user. The kill switch's logout deliberately keeps both, so a
//! revocation is complete only once every Machine that may hold them
//! reported the daemon's `remove_account` done for exactly this revocation.
//! The request is journaled by construction: its operation ID derives from
//! the durable revocation operation, account and Machine, so every retry
//! (including after a lost reply or a restart) repeats the same destructive
//! request, and the daemon replays its recorded outcome.
use crate::provider_account::ProviderAccountId;
use crate::store::provider_accounts::{Receipt, Registry};
use crate::store::{StorageBackend, Store};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The wire request and observation of the Machine's `remove_account`
/// (`MachineCommand::AccountSandboxRemove`).
pub use crate::provider_account::{
    AccountRemoveObservation as RemoveObservation, AccountRemoveRequest as RemoveRequest,
};

/// One Machine's removal receipt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub machine_id: String,
    pub observation: RemoveObservation,
    pub observed_at_ms: i64,
    /// The Machine enrollment that answered: one that dispatched a planned
    /// sign-in of the account there, so a re-enrolled Machine never
    /// certifies the removal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient_fingerprint: Option<String>,
}

/// The deterministic operation ID of one Machine's removal.
#[must_use]
pub fn operation_id(revocation: &str, account: &ProviderAccountId, machine: &str) -> String {
    use sha2::Digest as _;
    let scope = serde_json::to_vec(&("account-remove", revocation, account, machine))
        .expect("removal scope serializes");
    format!("account-remove-{:x}", sha2::Sha256::digest(scope))
}

/// The account's Provider, as captured with the revocation.
fn provider(receipt: &Receipt) -> Option<&str> {
    receipt
        .registry
        .accounts
        .iter()
        .find(|account| account.id == receipt.account_id)
        .map(|account| account.provider_id.as_str())
}

/// The Machines that may hold the account's sandbox: every Machine it signed
/// in on. The daemon creates an account's sandbox only where its credential
/// lives, and a session runs only on a Machine holding a sign-in, so session
/// placements add none. Empty for any other Provider. A Machine that never
/// ran the account's sandbox answers the removal confirmed (nothing exists).
#[must_use]
pub fn required_machines(receipt: &Receipt) -> BTreeSet<String> {
    let (Some(plan), Some(provider)) = (receipt.revocation.as_ref(), provider(receipt)) else {
        return BTreeSet::new();
    };
    if !crate::provider_account::named_only(provider) {
        return BTreeSet::new();
    }
    plan.logins
        .iter()
        .map(|target| target.machine_id.clone())
        .filter(|machine| machine != "local")
        .collect()
}

/// The removal request of `machine` for this revocation.
#[must_use]
pub fn request(receipt: &Receipt, revocation: &str, machine: &str) -> Option<RemoveRequest> {
    Some(RemoveRequest {
        operation_id: operation_id(revocation, &receipt.account_id, machine),
        provider_id: provider(receipt)?.to_owned(),
        account_id: receipt.account_id.clone(),
    })
}

/// Machines whose removal receipt is still missing.
#[must_use]
pub fn missing(receipt: &Receipt) -> Vec<String> {
    required_machines(receipt)
        .into_iter()
        .filter(|machine| !receipt.account_removals.contains_key(machine))
        .collect()
}

impl Store {
    /// Record one Machine's removal receipt. It must answer exactly this
    /// revocation's request for that Machine with both removals confirmed;
    /// identical replay grants no authority.
    pub async fn record_provider_account_removal(
        &self,
        owner: &str,
        revocation: &str,
        evidence: &Evidence,
        connection_is_current: impl FnOnce() -> bool + Send,
    ) -> Result<Evidence> {
        let mut check = Some(connection_is_current);
        let mut run = || check.take().expect("connection_is_current invoked once")();
        self.record_provider_account_removal_inner(owner, revocation, evidence, &mut run)
            .await
    }

    async fn record_provider_account_removal_inner(
        &self,
        owner: &str,
        revocation: &str,
        evidence: &Evidence,
        connection_is_current: &mut (dyn FnMut() -> bool + Send),
    ) -> Result<Evidence> {
        macro_rules! record { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(revocation).fetch_one(&mut *tx).await?;
            let mut receipt: Receipt = serde_json::from_str(&value)?;
            if let Some(old) = receipt.account_removals.get(&evidence.machine_id) {
                ensure!(old.observation == evidence.observation, "account_removal_conflict");
                tx.commit().await?;
                return Ok(old.clone());
            }
            ensure!(required_machines(&receipt).contains(&evidence.machine_id)
                && request(&receipt, revocation, &evidence.machine_id).as_ref() == Some(&evidence.observation.request),
                "account_removal_scope_mismatch");
            ensure!(evidence.observation.home_removed && evidence.observation.user_removed, "account_removal_unconfirmed");
            // Bound to the enrollment that dispatched a planned sign-in there.
            let mut enrolled = BTreeSet::new();
            let mut never_dispatched = true;
            for target in receipt.revocation.iter().flat_map(|plan| &plan.logins)
                .filter(|target| target.machine_id == evidence.machine_id)
            {
                let login: Option<String> = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                    .bind(owner).bind(&target.operation_id).fetch_optional(&mut *tx).await?;
                match login.and_then(|value| serde_json::from_str::<super::super::logins::Receipt>(&value).ok()) {
                    Some(login) => {
                        never_dispatched &= login.never_dispatched();
                        enrolled.extend(login.dispatch_recipient_fingerprint);
                    }
                    None => never_dispatched = false,
                }
            }
            // Every planned sign-in there was refused before enqueue: the
            // daemon never saw the account, so nothing exists to remove and
            // no enrollment can (or needs to) answer.
            let proven_absent = never_dispatched && evidence.recipient_fingerprint.is_none();
            if !proven_absent {
                let key: Option<String> = sqlx::query_scalar(concat!("SELECT encryption_public_key FROM machines WHERE id=$1 AND ", $revoked, " IS NULL"))
                    .bind(&evidence.machine_id).fetch_optional(&mut *tx).await?.flatten();
                let current = key.map(|key| crate::digest::hex_sha256(key.as_bytes()));
                ensure!(current.as_ref().is_some_and(|current| enrolled.contains(current))
                    && evidence.recipient_fingerprint == current, "machine_enrollment_changed");
            }
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            ensure!(registry.accounts.iter().any(|account| account.id == receipt.account_id
                && account.revoking && !account.enabled), "account_not_revoking");
            if !proven_absent {
                let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                    .bind(&evidence.machine_id).execute(&mut *tx).await?;
                ensure!(machine.rows_affected() == 1, "machine_unavailable");
                ensure!(connection_is_current(), "machine_connection_changed");
            }
            receipt.account_removals.insert(evidence.machine_id.clone(), evidence.clone());
            sqlx::query("UPDATE provider_account_operations SET result=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(revocation).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            evidence.clone()
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => record!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => record!(db, "revoked_at_ms"),
        })
    }
}
