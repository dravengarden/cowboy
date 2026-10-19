//! Internal promotion journal. A prepared intent is not runtime authority.
//! The caller must verify native evidence and persist an operation-bound vault
//! receipt before finishing. Distribution still requires current enrollment and
//! account admission; a historical receipt alone must never authorize it.
#![allow(dead_code)] // Named login remains gated until vault receipts and revocation are wired.

use super::{Change, NativeIdentity, ProviderAccountId, Registry, StorageBackend, Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Prepare {
    pub expected_revision: i64,
    pub account_id: ProviderAccountId,
    pub provider_id: String,
    pub machine_id: String,
    /// Hash of the enrolled recipient key, not a Machine display name.
    pub recipient_fingerprint: String,
    /// Hash of the exact verified candidate, including its signed release.
    pub candidate_fingerprint: String,
    pub expected_generation: u64,
    pub identity: NativeIdentity,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Phase {
    Prepared,
    Committed { generation: u64 },
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub operation_id: String,
    pub account_id: ProviderAccountId,
    pub provider_id: String,
    pub machine_id: String,
    pub recipient_fingerprint: String,
    pub candidate_fingerprint: String,
    pub identity_fingerprint: String,
    pub expected_generation: u64,
    pub registry_revision: i64,
    pub phase: Phase,
}

impl Receipt {
    pub(crate) fn commitment(&self) -> Result<String> {
        let mut prepared = self.clone();
        prepared.phase = Phase::Prepared;
        Ok(crate::admin::hex_sha256(&serde_json::to_vec(&(
            "cowboy-provider-grant-commit-v1",
            prepared,
        ))?))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Applied(Receipt),
    Rejected(&'static str),
}

/// Borrowed only during a DB-serialized admission callback. No public decoder
/// or constructor and no reusable authorization token escapes the transaction.
pub(crate) struct Admission {
    receipt: Receipt,
    recipient: String,
}

impl Admission {
    pub(crate) fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    pub(crate) fn recipient(&self) -> &str {
        &self.recipient
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl Store {
    /// Locator only. Callers must perform current admission before using it.
    pub(crate) async fn provider_grant_route(
        &self,
        commitment: &str,
    ) -> Result<Option<(String, String)>> {
        let query =
            "SELECT owner_user_id,operation_id FROM provider_grant_routes WHERE commitment=$1";
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_as(query)
                    .bind(commitment)
                    .fetch_optional(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_as(query)
                    .bind(commitment)
                    .fetch_optional(&db.pool)
                    .await?
            }
        })
    }

    /// Linearize one synchronous action against account edits and Machine
    /// revocation. Callbacks must not await or reenter the Store. Subsequent
    /// revocation still needs its own runtime cleanup; this is not a lease.
    pub(crate) async fn with_provider_grant_admission<T>(
        &self,
        owner: &str,
        operation: &str,
        machine: &str,
        action: impl FnOnce(&Admission) -> Result<T>,
    ) -> Result<T> {
        macro_rules! admit { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            anyhow::ensure!(locked.rows_affected() == 1, "Provider account unavailable");
            let locked = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(machine).execute(&mut *tx).await?;
            anyhow::ensure!(locked.rows_affected() == 1, "Provider grant Machine unavailable");
            let recipient: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(machine).fetch_one(&mut *tx).await?;
            let recipient = recipient.ok_or_else(|| anyhow::anyhow!("Provider grant Machine has no recipient"))?;
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_grants WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let receipt: Receipt = serde_json::from_str(&value)?;
            anyhow::ensure!(matches!(receipt.phase, Phase::Committed { .. })
                && receipt.machine_id == machine
                && receipt.recipient_fingerprint == crate::admin::hex_sha256(recipient.as_bytes()),
                "Provider grant is not committed for this enrollment");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            anyhow::ensure!(registry.revision == receipt.registry_revision
                && registry.accounts.iter().any(|account| account.id == receipt.account_id
                    && account.provider_id == receipt.provider_id && account.available()
                    && account.identity_fingerprint.as_ref() == Some(&receipt.identity_fingerprint)),
                "Provider grant account admission changed");
            let result = action(&Admission { receipt, recipient })?;
            tx.commit().await?;
            result
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => admit!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => admit!(db, "revoked_at_ms"),
        })
    }

    /// Recovery lookup only; this receipt never authorizes credential delivery.
    pub(crate) async fn provider_grant_receipt(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<Receipt>> {
        let query =
            "SELECT value FROM provider_account_grants WHERE owner_user_id=$1 AND operation_id=$2";
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

    pub(crate) async fn prepare_provider_grant(
        &self,
        owner: &str,
        operation: &str,
        input: &Prepare,
    ) -> Result<Outcome> {
        if !crate::managed_calls::valid_id(operation)
            || !crate::managed_calls::valid_id(&input.machine_id)
            || input.account_id.is_default()
            || input.expected_revision < 0
            || input.expected_generation.checked_add(1).is_none()
            || !digest(&input.recipient_fingerprint)
            || !digest(&input.candidate_fingerprint)
        {
            return Ok(Outcome::Rejected("invalid_argument"));
        }
        let identity_fingerprint = match input.identity.fingerprint(&input.provider_id) {
            Ok(value) => value,
            Err(code) => return Ok(Outcome::Rejected(code)),
        };
        let request_hash = crate::admin::hex_sha256(&serde_json::to_vec(input)?);
        macro_rules! prepare { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            // Same owner-row lock as metadata mutation; PostgreSQL and SQLite
            // serialize disable/default/bind and promotion preparation alike.
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            if locked.rows_affected() != 1 { return Ok(Outcome::Rejected("account_unavailable")); }
            let old: Option<(String, String)> = sqlx::query_as("SELECT request_hash,value FROM provider_account_grants WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
            if let Some((hash, value)) = old {
                return Ok(if hash == request_hash { Outcome::Applied(serde_json::from_str(&value)?) }
                    else { Outcome::Rejected("operation_reused") });
            }
            // Lock in the same owner -> Machine order as current admission.
            // Verify enrollment before binding an immutable native identity.
            let machine = sqlx::query(concat!("UPDATE machines SET encryption_public_key=encryption_public_key WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&input.machine_id).execute(&mut *tx).await?;
            if machine.rows_affected() != 1 { return Ok(Outcome::Rejected("machine_unavailable")); }
            let recipient: Option<String> = sqlx::query_scalar("SELECT encryption_public_key FROM machines WHERE id=$1")
                .bind(&input.machine_id).fetch_one(&mut *tx).await?;
            if recipient.as_ref().is_none_or(|key| crate::admin::hex_sha256(key.as_bytes()) != input.recipient_fingerprint) {
                return Ok(Outcome::Rejected("machine_enrollment_changed"));
            }
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let mut registry: Registry = serde_json::from_str(&value)?;
            if registry.revision != input.expected_revision { return Ok(Outcome::Rejected("revision_conflict")); }
            if let Err(code) = registry.apply(&Change::BindIdentity {
                id: input.account_id.clone(), provider_id: input.provider_id.clone(), identity: input.identity.clone(),
            }) { return Ok(Outcome::Rejected(code)); }
            let receipt = Receipt {
                operation_id: operation.into(), account_id: input.account_id.clone(), provider_id: input.provider_id.clone(),
                machine_id: input.machine_id.clone(), recipient_fingerprint: input.recipient_fingerprint.clone(),
                candidate_fingerprint: input.candidate_fingerprint.clone(), identity_fingerprint: identity_fingerprint.clone(),
                expected_generation: input.expected_generation, registry_revision: registry.revision, phase: Phase::Prepared,
            };
            sqlx::query("UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3")
                .bind(registry.revision).bind(serde_json::to_string(&registry)?).bind(owner).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO provider_account_grants (owner_user_id,operation_id,request_hash,value) VALUES ($1,$2,$3,$4)")
                .bind(owner).bind(operation).bind(&request_hash).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            Outcome::Applied(receipt)
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => prepare!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => prepare!(db, "revoked_at_ms"),
        })
    }

    /// Finalize only the exact durable intent. The supplied receipt is compared
    /// with stored fields; callers cannot change recipient, account or candidate.
    /// Metadata changes during login fence this attempt, including disable/enable
    /// ABA. Repeating a committed receipt observes history, not fresh authority.
    pub(crate) async fn finish_verified_provider_grant(
        &self,
        owner: &str,
        intent: &Receipt,
        proof: &crate::provider_service::grants::VerifiedCommit,
    ) -> Result<Outcome> {
        let generation = proof.generation_for(intent)?;
        self.finish_provider_grant(owner, intent, generation).await
    }

    async fn finish_provider_grant(
        &self,
        owner: &str,
        intent: &Receipt,
        generation: u64,
    ) -> Result<Outcome> {
        self.resolve_provider_grant(owner, intent, Some(generation))
            .await
    }

    /// Cancel preparation, not an already committed grant. Vault cleanup must
    /// reconcile the returned terminal state before removing any credentials.
    pub(crate) async fn cancel_provider_grant(
        &self,
        owner: &str,
        intent: &Receipt,
    ) -> Result<Outcome> {
        self.resolve_provider_grant(owner, intent, None).await
    }

    async fn resolve_provider_grant(
        &self,
        owner: &str,
        intent: &Receipt,
        generation: Option<u64>,
    ) -> Result<Outcome> {
        if intent.phase != Phase::Prepared
            || generation
                .is_some_and(|value| intent.expected_generation.checked_add(1) != Some(value))
        {
            return Ok(Outcome::Rejected("grant_receipt_mismatch"));
        }
        macro_rules! finish { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            let old: Option<String> = sqlx::query_scalar("SELECT value FROM provider_account_grants WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(&intent.operation_id).fetch_optional(&mut *tx).await?;
            let Some(old) = old else { return Ok(Outcome::Rejected("grant_not_found")); };
            let mut receipt: Receipt = serde_json::from_str(&old)?;
            let mut prepared = receipt.clone();
            prepared.phase = Phase::Prepared;
            if &prepared != intent { return Ok(Outcome::Rejected("grant_receipt_mismatch")); }
            if receipt.phase != Phase::Prepared { return Ok(Outcome::Applied(receipt)); }
            let value: Option<String> = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_optional(&mut *tx).await?;
            let registry: Option<Registry> = value.map(|value| serde_json::from_str(&value)).transpose()?;
            let current = registry.is_some_and(|registry| registry.revision == intent.registry_revision
                && registry.accounts.iter().any(|account| account.id == intent.account_id
                    && account.provider_id == intent.provider_id && account.available()
                    && account.identity_fingerprint.as_ref() == Some(&intent.identity_fingerprint)));
            receipt.phase = match generation {
                Some(generation) if current => Phase::Committed { generation },
                _ => Phase::Cancelled,
            };
            sqlx::query("UPDATE provider_account_grants SET value=$1 WHERE owner_user_id=$2 AND operation_id=$3")
                .bind(serde_json::to_string(&receipt)?).bind(owner).bind(&intent.operation_id).execute(&mut *tx).await?;
            if matches!(receipt.phase, Phase::Committed { .. }) {
                sqlx::query("INSERT INTO provider_grant_routes (commitment,owner_user_id,operation_id) VALUES ($1,$2,$3)")
                    .bind(receipt.commitment()?).bind(owner).bind(&receipt.operation_id).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            Outcome::Applied(receipt)
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => finish!(db),
            StorageBackend::Sqlite(db) => finish!(db),
        })
    }
}
