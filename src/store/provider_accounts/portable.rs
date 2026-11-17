//! Named portable accounts (a Service-held API key at account scope): the
//! owner-locked admission shared with every account writer, and durable
//! revocation evidence for the account vault and each Machine replica.
use super::{Account, Receipt, Registry};
use crate::provider_account::ProviderAccountId;
use crate::store::{StorageBackend, Store};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

/// Lock the owner and load a portable revocation still in progress.
macro_rules! locked_revocation { ($tx:expr, $owner:expr, $operation:expr) => {{
    let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
        .bind($owner).execute(&mut *$tx).await?;
    ensure!(locked.rows_affected() == 1, "account_unavailable");
    let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
        .bind($owner).bind($operation).fetch_one(&mut *$tx).await?;
    let receipt: Receipt = serde_json::from_str(&value)?;
    ensure!(!receipt.account_id.is_default() && !receipt.revocation_completed
        && receipt.revocation.as_ref().is_some_and(|plan| plan.portable_credentials), "portable_revocation_scope_mismatch");
    let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
        .bind($owner).fetch_one(&mut *$tx).await?;
    let registry: Registry = serde_json::from_str(&value)?;
    ensure!(registry.accounts.iter().any(|account| account.id == receipt.account_id && account.revoking && !account.enabled),
        "account_not_revoking");
    receipt
}}; }

/// A Machine enrolled when the account vault was wiped. It may hold a replica.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub machine_id: String,
    pub recipient_fingerprint: String,
}

/// The account vault's wipe generation (0: it never held a key) and the
/// Machines that must acknowledge it. No Apply can be sealed afterwards.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalWipe {
    pub generation: u64,
    pub machines: Vec<Target>,
}

/// Authenticated Machine acknowledgement of the vault's wipe generation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RemoteWipe {
    pub machine_id: String,
    pub recipient_fingerprint: String,
    pub generation: u64,
}

/// What one admission holds the owner's account lock for.
pub enum Use<'a> {
    /// A key commit: marks the account before its vault first holds a key.
    Commit,
    /// A subscription credential commit (sign-in or native migration): marks
    /// the account like `Commit` and binds the verified identity first. A
    /// different identity than the bound one is refused.
    SignIn(&'a crate::provider_account::NativeIdentity),
    /// Persist a new session row before it is projected or launched.
    Insert(&'a crate::model::SessionMeta),
    /// Lock an exact persisted binding for a bounded write or projection.
    Existing(&'a crate::model::SessionMeta),
}

/// Revocation evidence still missing for a portable account key, as
/// `(kind, target)` pairs.
pub(super) fn missing(receipt: &Receipt) -> Vec<(&'static str, String)> {
    if !receipt
        .revocation
        .as_ref()
        .is_some_and(|plan| plan.portable_credentials)
    {
        return Vec::new();
    }
    let Some(local) = &receipt.portable_wipe else {
        return vec![(
            "portable_vault_wipe",
            receipt.account_id.as_str().to_owned(),
        )];
    };
    local
        .machines
        .iter()
        .filter(|target| {
            !receipt
                .portable_remote_wipes
                .get(&target.machine_id)
                .is_some_and(|wipe| {
                    wipe.recipient_fingerprint == target.recipient_fingerprint
                        && wipe.generation >= local.generation
                })
        })
        .map(|target| ("portable_replica_wipe", target.machine_id.clone()))
        .collect()
}

impl Store {
    /// Run `action` while the owner's account lock holds: the account is
    /// available, serves the caller (`serves`), and for sessions the exact
    /// row is inserted or locked. Revocation, disabling and key commits take
    /// the same lock, so none can complete inside `action`. `action` must not
    /// reenter Store; it is bounded to a vault check plus one write/projection.
    pub async fn with_portable_account<T, F: std::future::Future<Output = Result<T>>>(
        &self,
        owner: &str,
        account: &ProviderAccountId,
        usage: Use<'_>,
        serves: impl Fn(&Account) -> bool,
        action: impl FnOnce() -> F,
    ) -> Result<T> {
        ensure!(!account.is_default(), "named_account_required");
        if let Use::Insert(m) | Use::Existing(m) = usage {
            ensure!(
                m.owner_user_id.as_deref() == Some(owner)
                    && &m.provider_account_id == account
                    && !m.provider_version.is_empty()
                    && !m.provider_generation_digest.is_empty()
                    && m.provider_auth_generation
                        .is_some_and(|generation| generation > 0),
                "portable_session_scope_mismatch"
            );
        }
        macro_rules! admit { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let mut registry: Registry = serde_json::from_str(&value)?;
            let index = registry.accounts.iter().position(|a| &a.id == account && a.available() && serves(a));
            let index = index.ok_or_else(|| anyhow::anyhow!("account_unavailable"))?;
            match usage {
                Use::Commit | Use::SignIn(_) => {
                    let (revision, marked) = (registry.revision, registry.accounts[index].portable_credentials);
                    if let Use::SignIn(identity) = usage {
                        let provider_id = registry.accounts[index].provider_id.clone();
                        registry.apply(&super::Change::BindIdentity { id: account.clone(), provider_id, identity: identity.clone() })
                            .map_err(|code| anyhow::anyhow!(code))?;
                    }
                    // Durable before the vault write. Marking alone bumps no
                    // revision, as no owner-visible field changes.
                    registry.accounts[index].portable_credentials = true;
                    if registry.revision != revision || !marked {
                        sqlx::query("UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3")
                            .bind(registry.revision).bind(serde_json::to_string(&registry)?).bind(owner).execute(&mut *tx).await?;
                    }
                }
                Use::Insert(m) => {
                    ensure!(m.status == crate::agent_model::Status::Starting, "portable_session_scope_mismatch");
                    sqlx::query("INSERT INTO sessions(id,provider,provider_version,provider_generation_digest,provider_auth_generation,provider_behavior,machine_id,workspace_id,workspace_name,workspace_source_path,cwd,title,origin,status,next_seq,system,owner_user_id,execution_binding,provider_account_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,0,$15,$16,$17,$18)")
                        .bind(&m.id).bind(&m.provider).bind(&m.provider_version).bind(&m.provider_generation_digest)
                        .bind(m.provider_auth_generation.map(i64::try_from).transpose()?)
                        .bind(m.provider_behavior.as_ref().map(serde_json::to_value).transpose()?)
                        .bind(&m.machine_id).bind(m.workspace_id.as_deref()).bind(m.workspace_name.as_deref())
                        .bind(m.workspace_source_path.as_deref()).bind(&m.cwd).bind(crate::store::strip_nul_str(&m.title))
                        .bind(crate::store::origin_to_str(m.origin)).bind(crate::store::status_to_str(m.status))
                        .bind(m.system).bind(m.owner_user_id.as_deref())
                        .bind(m.execution_binding.as_ref().map(crate::execution_environment::ExecutionBinding::record))
                        .bind(m.provider_account_id.as_str()).execute(&mut *tx).await?;
                }
                Use::Existing(m) => {
                    let row = sqlx::query("UPDATE sessions SET id=id WHERE id=$1 AND owner_user_id=$2 AND provider_account_id=$3 AND machine_id=$4 AND provider=$5 AND provider_version=$6 AND provider_generation_digest=$7 AND provider_auth_generation=$8")
                        .bind(&m.id).bind(owner).bind(account.as_str()).bind(&m.machine_id).bind(&m.provider)
                        .bind(&m.provider_version).bind(&m.provider_generation_digest)
                        .bind(m.provider_auth_generation.map(i64::try_from).transpose()?).execute(&mut *tx).await?;
                    ensure!(row.rows_affected() == 1, "portable_session_binding_changed");
                }
            }
            let result = action().await?;
            tx.commit().await?;
            result
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => admit!(db),
            StorageBackend::Sqlite(db) => admit!(db),
        })
    }

    /// The owner of a named account. Machine refresh evidence names only the
    /// account; it is admitted under this owner's lock, never another's.
    pub async fn provider_account_owner(
        &self,
        account: &ProviderAccountId,
    ) -> Result<Option<String>> {
        let query =
            "SELECT owner_user_id,value FROM provider_account_registries ORDER BY owner_user_id";
        let rows: Vec<(String, String)> = match &self.backend {
            StorageBackend::Postgres(db) => sqlx::query_as(query).fetch_all(&db.pool).await?,
            StorageBackend::Sqlite(db) => sqlx::query_as(query).fetch_all(&db.pool).await?,
        };
        for (owner, value) in rows {
            let registry: Registry = serde_json::from_str(&value)?;
            if registry
                .accounts
                .iter()
                .any(|candidate| &candidate.id == account)
            {
                return Ok(Some(owner));
            }
        }
        Ok(None)
    }

    /// The highest native login generation the account ever reserved on any
    /// Machine (0 if none). Vault generations are issued above it, so a
    /// vault-bound session never names a native profile.
    pub async fn native_generation_floor(
        &self,
        owner: &str,
        account: &ProviderAccountId,
    ) -> Result<u64> {
        let query = "SELECT MAX(generation) FROM provider_native_generations WHERE owner_user_id=$1 AND account_id=$2";
        let value: Option<i64> = match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(account.as_str())
                    .fetch_one(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(account.as_str())
                    .fetch_one(&db.pool)
                    .await?
            }
        };
        Ok(u64::try_from(value.unwrap_or(0))?)
    }

    /// Record the wiped account vault generation and fix the Machines that
    /// must acknowledge it. Identical replay returns the original record.
    pub async fn record_portable_local_wipe(
        &self,
        owner: &str,
        operation: &str,
        generation: u64,
    ) -> Result<LocalWipe> {
        ensure!(generation < u64::MAX, "invalid_wipe_generation");
        macro_rules! record { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let mut receipt = locked_revocation!(tx, owner, operation);
            if let Some(old) = &receipt.portable_wipe {
                ensure!(old.generation == generation, "portable_wipe_conflict");
                return Ok(old.clone());
            }
            let machines: Vec<(String, String)> = sqlx::query_as(concat!("SELECT id,encryption_public_key FROM machines WHERE ", $revoked, " IS NULL AND encryption_public_key IS NOT NULL ORDER BY id"))
                .fetch_all(&mut *tx).await?;
            let wipe = LocalWipe { generation, machines: if generation == 0 { Vec::new() } else {
                machines.into_iter().map(|(machine_id, key)| Target { machine_id, recipient_fingerprint: crate::digest::hex_sha256(key.as_bytes()) }).collect()
            } };
            receipt.portable_wipe = Some(wipe.clone());
            sqlx::query("UPDATE provider_account_operations SET result=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            wipe
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => record!(db, "revoked_at"),
            StorageBackend::Sqlite(db) => record!(db, "revoked_at_ms"),
        })
    }

    /// Record one Machine's authenticated wipe acknowledgement. The Machine
    /// must be a fixed target, still enrolled with the same key and connected.
    pub async fn record_portable_remote_wipe(
        &self,
        owner: &str,
        operation: &str,
        evidence: &RemoteWipe,
        connection_is_current: impl FnOnce() -> bool,
    ) -> Result<RemoteWipe> {
        macro_rules! record { ($db:expr, $revoked:literal) => {{
            let mut tx = $db.pool.begin().await?;
            let mut receipt = locked_revocation!(tx, owner, operation);
            if let Some(old) = receipt.portable_remote_wipes.get(&evidence.machine_id) {
                ensure!(old == evidence, "portable_wipe_conflict");
                return Ok(old.clone());
            }
            let local = receipt.portable_wipe.as_ref().ok_or_else(|| anyhow::anyhow!("portable_vault_wipe_missing"))?;
            ensure!(local.machines.iter().any(|target| target.machine_id == evidence.machine_id
                && target.recipient_fingerprint == evidence.recipient_fingerprint)
                && evidence.generation >= local.generation && evidence.generation < u64::MAX, "portable_wipe_scope_mismatch");
            let key: Option<String> = sqlx::query_scalar(concat!("SELECT encryption_public_key FROM machines WHERE id=$1 AND ", $revoked, " IS NULL"))
                .bind(&evidence.machine_id).fetch_optional(&mut *tx).await?.flatten();
            ensure!(key.is_some_and(|key| crate::digest::hex_sha256(key.as_bytes()) == evidence.recipient_fingerprint), "machine_enrollment_changed");
            ensure!(connection_is_current(), "machine_connection_changed");
            receipt.portable_remote_wipes.insert(evidence.machine_id.clone(), evidence.clone());
            sqlx::query("UPDATE provider_account_operations SET result=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
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
mod sign_in_tests;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::provider_accounts::{Change, Mutation, MutationResult};

    const OWNER: &str = "fedcba9876543210fedcba9876543210";

    async fn fixture(root: &std::path::Path) -> (Store, String) {
        let url = format!("sqlite://{}", root.join("db.sqlite3").display());
        let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
        store.migrate().await.unwrap();
        let key = {
            use base64::Engine as _;
            let public = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([6; 32]));
            base64::engine::general_purpose::STANDARD.encode(public.as_bytes())
        };
        let token = store
            .create_machine_enrollment("hawk", "hawk", 60)
            .await
            .unwrap();
        store
            .consume_machine_enrollment(&token, &format!("ssh-ed25519 {key}"), &key)
            .await
            .unwrap();
        (store, crate::digest::hex_sha256(key.as_bytes()))
    }

    async fn mutate(store: &Store, operation: &str, revision: i64, change: Change) -> Receipt {
        let input = Mutation {
            expected_revision: revision,
            change,
        };
        match store
            .mutate_provider_accounts(OWNER, operation, &input)
            .await
            .unwrap()
        {
            MutationResult::Applied(receipt) => receipt,
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn revocation_wipes_a_portable_key_everywhere_before_completing() {
        let root = tempfile::tempdir().unwrap();
        let (store, fingerprint) = fixture(root.path()).await;
        let create = Change::Create {
            provider_id: "claude-deepseek".into(),
            label: "DeepSeek".into(),
        };
        let id = mutate(&store, "create", 0, create).await.account_id;
        let commit = |store: &Store| {
            let (store, id) = (store.clone(), id.clone());
            async move {
                store
                    .with_portable_account(OWNER, &id, Use::Commit, |_| true, || async { Ok(()) })
                    .await
            }
        };
        commit(&store).await.unwrap();
        let registry = store.provider_accounts(OWNER).await.unwrap();
        assert!(registry.accounts[0].portable_credentials);
        // The marker is not an owner-visible edit.
        assert_eq!(registry.revision, 1);
        let revoke = mutate(
            &store,
            "revoke",
            1,
            Change::BeginRevocation { id: id.clone() },
        )
        .await;
        assert!(revoke.revocation.as_ref().unwrap().portable_credentials);
        // A revoking account never receives another key.
        assert!(commit(&store).await.is_err());
        assert!(
            !store
                .complete_provider_revocation(OWNER, "revoke")
                .await
                .unwrap()
        );
        let wiped = store
            .record_portable_local_wipe(OWNER, "revoke", 2)
            .await
            .unwrap();
        assert_eq!(
            wiped.machines,
            vec![Target {
                machine_id: "hawk".into(),
                recipient_fingerprint: fingerprint.clone()
            }]
        );
        assert!(
            store
                .record_portable_local_wipe(OWNER, "revoke", 3)
                .await
                .is_err()
        );
        assert!(
            !store
                .complete_provider_revocation(OWNER, "revoke")
                .await
                .unwrap()
        );
        let evidence = |generation| RemoteWipe {
            machine_id: "hawk".into(),
            recipient_fingerprint: fingerprint.clone(),
            generation,
        };
        for (wipe, connected) in [(evidence(1), true), (evidence(2), false)] {
            assert!(
                store
                    .record_portable_remote_wipe(OWNER, "revoke", &wipe, || connected)
                    .await
                    .is_err()
            );
        }
        let mut stranger = evidence(2);
        stranger.recipient_fingerprint = "other".into();
        assert!(
            store
                .record_portable_remote_wipe(OWNER, "revoke", &stranger, || true)
                .await
                .is_err()
        );
        store
            .record_portable_remote_wipe(OWNER, "revoke", &evidence(2), || true)
            .await
            .unwrap();
        assert!(
            store
                .complete_provider_revocation(OWNER, "revoke")
                .await
                .unwrap()
        );
        let readiness = store
            .provider_revocation_readiness(OWNER, "revoke")
            .await
            .unwrap();
        assert!(readiness.missing.is_empty());
    }
}
