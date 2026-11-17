//! Adoption of a legacy default usage card into a named registry account.
//!
//! The named account's id is the card's browser key
//! (`legacy_browsers::account(owner, card)`), so the Machine browser profile
//! stays bound to it with no file moves, and a replay finds the same
//! account. Creation carries the card's browser enablement and share
//! membership (and family bindings) over to the account, atomically under
//! the owner lock. Completion, after the credential moved into the account
//! vault, makes it the registry default. Both stages are registry writes
//! with operation receipts, like every other account change.
use super::super::{StorageBackend, Store};
use super::{
    Account, MutationResult, Receipt, Registry, browser_families, browser_shares::Member,
    label_valid, legacy_browsers, provider_id_valid,
};
use anyhow::Result;
use serde::Serialize;

/// What a creation adopts: `card` into a `provider_id` account named `label`.
#[derive(Clone, Debug, Serialize)]
pub struct Adoption<'a> {
    pub card: &'a str,
    pub provider_id: &'a str,
    pub label: &'a str,
}

impl Registry {
    /// Create (or resume) the adopting account in the registry value.
    /// Returns whether it was created now.
    fn adopt(
        &mut self,
        owner: &str,
        adoption: &Adoption<'_>,
        browser_enabled: bool,
    ) -> std::result::Result<bool, &'static str> {
        let id = legacy_browsers::account(owner, adoption.card);
        if let Some(account) = self.accounts.iter().find(|account| account.id == id) {
            return if account.provider_id != adoption.provider_id {
                Err("account_id_in_use")
            } else if account.available() {
                Ok(false)
            } else {
                Err("account_unavailable")
            };
        }
        let family = browser_families::card(adoption.card).ok_or("invalid_provider")?;
        if !legacy_browsers::card_valid(adoption.card)
            || !provider_id_valid(adoption.provider_id)
            || browser_families::provider(adoption.provider_id) != Some(family)
        {
            return Err("invalid_provider");
        }
        if !label_valid(adoption.label) {
            return Err("invalid_account");
        }
        if self.accounts.len() >= 128 {
            return Err("account_capacity");
        }
        let revision = self.revision.checked_add(1).ok_or("revision_exhausted")?;
        self.accounts.push(Account {
            id,
            provider_id: adoption.provider_id.to_owned(),
            label: adoption.label.trim().to_owned(),
            enabled: true,
            revoking: false,
            identity_fingerprint: None,
            admission_revision: Some(revision),
            browser_enabled,
            portable_credentials: false,
        });
        self.revision = revision;
        Ok(true)
    }

    /// Make the adopting account `id` the default of `providers`.
    fn complete_adoption(
        &mut self,
        id: &crate::provider_account::ProviderAccountId,
        providers: &[String],
    ) -> std::result::Result<(), &'static str> {
        if providers.is_empty() || !providers.iter().all(|id| provider_id_valid(id)) {
            return Err("invalid_provider");
        }
        if !self
            .accounts
            .iter()
            .any(|account| &account.id == id && account.available())
        {
            return Err("account_unavailable");
        }
        for provider in providers {
            self.defaults.insert(provider.clone(), id.clone());
        }
        self.revision = self.revision.checked_add(1).ok_or("revision_exhausted")?;
        Ok(())
    }
}

/// Lock the owner, and return a stored receipt for a replayed operation.
macro_rules! locked_replay { ($tx:expr, $owner:expr, $operation:expr, $hash:expr) => {{
    sqlx::query("INSERT INTO provider_account_registries (owner_user_id,revision,value) VALUES ($1,0,$2) ON CONFLICT (owner_user_id) DO UPDATE SET revision=provider_account_registries.revision")
        .bind($owner).bind(serde_json::to_string(&Registry::default())?).execute(&mut *$tx).await?;
    let old: Option<(String, String)> = sqlx::query_as("SELECT request_hash,result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
        .bind($owner).bind($operation).fetch_optional(&mut *$tx).await?;
    if let Some((old_hash, result)) = old {
        return Ok(if old_hash == $hash { MutationResult::Applied(serde_json::from_str(&result)?) }
            else { MutationResult::Invalid("operation_reused") });
    }
    let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
        .bind($owner).fetch_one(&mut *$tx).await?;
    serde_json::from_str::<Registry>(&value)?
}}; }

/// Persist the registry and the operation receipt, then commit.
macro_rules! commit_receipt { ($tx:expr, $owner:expr, $operation:expr, $hash:expr, $registry:expr, $id:expr) => {{
    sqlx::query("UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3")
        .bind($registry.revision).bind(serde_json::to_string(&$registry)?).bind($owner).execute(&mut *$tx).await?;
    let receipt = Receipt { registry: $registry, account_id: $id, ..Receipt::default() };
    sqlx::query("INSERT INTO provider_account_operations (owner_user_id,operation_id,request_hash,result) VALUES ($1,$2,$3,$4)")
        .bind($owner).bind($operation).bind(&$hash).bind(serde_json::to_string(&receipt)?).execute(&mut *$tx).await?;
    $tx.commit().await?;
    MutationResult::Applied(receipt)
}}; }

impl Store {
    /// Stage 1: create the adopting account (or resume an unfinished
    /// adoption of the same card), carrying the card's browser enablement,
    /// share membership and family bindings over to it.
    pub async fn adopt_legacy_card(
        &self,
        owner: &str,
        operation: &str,
        adoption: &Adoption<'_>,
    ) -> Result<MutationResult> {
        if !crate::managed_calls::valid_id(operation) {
            return Ok(MutationResult::Invalid("invalid_argument"));
        }
        let hash = crate::digest::hex_sha256(&serde_json::to_vec(&("adopt", adoption))?);
        let id = legacy_browsers::account(owner, adoption.card);
        let card = Member::card(adoption.card).encode();
        let member = Member::account(id.clone()).encode();
        macro_rules! write { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            let mut registry = locked_replay!(tx, owner, operation, hash);
            let enabled: Option<String> = sqlx::query_scalar("SELECT card_id FROM provider_legacy_browsers WHERE owner_user_id=$1 AND card_id=$2")
                .bind(owner).bind(adoption.card).fetch_optional(&mut *tx).await?;
            let created = match registry.adopt(owner, adoption, enabled.is_some()) {
                Ok(created) => created, Err(code) => return Ok(MutationResult::Invalid(code)),
            };
            if created {
                let share: Option<(String, Option<String>)> = sqlx::query_as("SELECT m.share_id,s.pending_reset FROM provider_browser_share_members m JOIN provider_browser_shares s ON s.owner_user_id=m.owner_user_id AND s.share_id=m.share_id WHERE m.owner_user_id=$1 AND m.member=$2")
                    .bind(owner).bind(&card).fetch_optional(&mut *tx).await?;
                if let Some((share_id, pending)) = share {
                    // Membership is frozen while a reset of the profile may run.
                    if pending.is_some() { return Ok(MutationResult::Invalid("browser_share_reset_pending")); }
                    sqlx::query("UPDATE provider_browser_share_members SET member=$3 WHERE owner_user_id=$1 AND member=$2")
                        .bind(owner).bind(&card).bind(&member).execute(&mut *tx).await?;
                    sqlx::query("UPDATE provider_browser_shares SET revision=revision+1 WHERE owner_user_id=$1 AND share_id=$2")
                        .bind(owner).bind(&share_id).execute(&mut *tx).await?;
                }
                // The same identity and family: whatever the card's cookies
                // bound in any share now belongs to the account.
                sqlx::query("UPDATE provider_browser_share_bindings SET member=$3 WHERE owner_user_id=$1 AND member=$2")
                    .bind(owner).bind(&card).bind(&member).execute(&mut *tx).await?;
                sqlx::query("DELETE FROM provider_legacy_browsers WHERE owner_user_id=$1 AND card_id=$2")
                    .bind(owner).bind(adoption.card).execute(&mut *tx).await?;
            }
            commit_receipt!(tx, owner, operation, hash, registry, id)
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => write!(db),
            StorageBackend::Sqlite(db) => write!(db),
        })
    }

    /// Stage 2: the credential moved. Make the account the default of
    /// `providers` (its Provider and every member of its credential scope).
    pub async fn complete_legacy_adoption(
        &self,
        owner: &str,
        operation: &str,
        id: &crate::provider_account::ProviderAccountId,
        providers: &[String],
    ) -> Result<MutationResult> {
        if !crate::managed_calls::valid_id(operation) {
            return Ok(MutationResult::Invalid("invalid_argument"));
        }
        let hash = crate::digest::hex_sha256(&serde_json::to_vec(&("complete", id, providers))?);
        macro_rules! write {
            ($db:expr) => {{
                let mut tx = $db.pool.begin().await?;
                let mut registry = locked_replay!(tx, owner, operation, hash);
                if let Err(code) = registry.complete_adoption(id, providers) {
                    return Ok(MutationResult::Invalid(code));
                }
                commit_receipt!(tx, owner, operation, hash, registry, id.clone())
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => write!(db),
            StorageBackend::Sqlite(db) => write!(db),
        })
    }
}

#[cfg(test)]
mod tests;
