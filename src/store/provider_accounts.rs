//! User-owned subscription metadata, independent of Machine placement and secrets.
use super::{StorageBackend, Store};
use crate::provider_account::{NativeIdentity, ProviderAccountId};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) mod grants;
pub(crate) mod logins;
pub(crate) mod revocation;
pub(crate) mod transitions;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Account {
    pub id: ProviderAccountId,
    pub provider_id: String,
    pub label: String,
    /// Account admission. Disabling invalidates existing bindings as well as
    /// new logins/sessions, without proving worker exit or credential revocation.
    pub enabled: bool,
    /// Durable admission fence; never evidence of worker exit or vendor logout.
    #[serde(default)]
    pub revoking: bool,
    /// Hash of the verified native identity, never a credential or display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_fingerprint: Option<String>,
    /// Registry revision of this account's last authority change. Missing on
    /// legacy records: those keep conservative whole-registry admission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_revision: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Registry {
    pub revision: i64,
    pub accounts: Vec<Account>,
    pub defaults: BTreeMap<String, ProviderAccountId>,
}

impl Account {
    pub(crate) fn admits_binding(&self, bound: i64, registry: i64) -> bool {
        bound >= 0
            && bound <= registry
            && self
                .admission_revision
                .map_or(bound == registry, |revision| {
                    revision >= 0 && revision <= bound
                })
    }

    pub(crate) fn available(&self) -> bool {
        self.enabled && !self.revoking
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mutation {
    pub expected_revision: i64,
    pub change: Change,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Change {
    /// Internal lifecycle entry. Public metadata edits cannot start or clear it.
    #[serde(skip_deserializing)]
    #[allow(dead_code, reason = "account lifecycle orchestration remains gated")]
    BeginRevocation {
        id: ProviderAccountId,
    },
    /// Internal promotion only. Public HTTP/MCP deserialization cannot produce
    /// this variant. The native verifier must establish identity before calling.
    #[serde(skip_deserializing)]
    #[allow(dead_code)] // Native verifier integration is still rollout-gated.
    BindIdentity {
        id: ProviderAccountId,
        provider_id: String,
        identity: NativeIdentity,
    },
    Create {
        provider_id: String,
        label: String,
    },
    Rename {
        id: ProviderAccountId,
        label: String,
    },
    SetEnabled {
        id: ProviderAccountId,
        enabled: bool,
    },
    SetDefault {
        provider_id: String,
        id: ProviderAccountId,
    },
}

impl NativeIdentity {
    pub(crate) fn fingerprint(&self, provider: &str) -> std::result::Result<String, &'static str> {
        let valid = |value: &str| {
            !value.is_empty()
                && value.len() <= 512
                && value.trim() == value
                && !value.chars().any(char::is_control)
        };
        if !valid(&self.authority)
            || !valid(&self.subject)
            || self.tenant.as_deref().is_some_and(|tenant| !valid(tenant))
        {
            return Err("invalid_native_identity");
        }
        // JSON tuple encoding separates fields unambiguously; include a domain
        // and Provider so equal native IDs from different systems cannot alias.
        let bytes = serde_json::to_vec(&(
            "cowboy-native-account-v1",
            provider,
            &self.authority,
            &self.subject,
            &self.tenant,
        ))
        .map_err(|_| "invalid_native_identity")?;
        Ok(crate::admin::hex_sha256(&bytes))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Receipt {
    /// Local lifecycle completion only; the account admission fence stays set.
    #[serde(default)]
    pub revocation_completed: bool,
    pub registry: Registry,
    pub account_id: ProviderAccountId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation: Option<revocation::Plan>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub remote_wipes: BTreeMap<String, revocation::remote_wipes::Evidence>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub local_wipes: BTreeMap<String, revocation::LocalWipe>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub login_stops: BTreeMap<String, revocation::login_stops::Evidence>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MutationResult {
    Applied(Receipt),
    Conflict(Registry),
    Invalid(&'static str),
}

fn provider_id_valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn label_valid(label: &str) -> bool {
    !label.trim().is_empty() && label.len() <= 128 && !label.chars().any(char::is_control)
}

impl Registry {
    fn begin_revocation(
        &mut self,
        id: &ProviderAccountId,
    ) -> std::result::Result<bool, &'static str> {
        let account = self
            .accounts
            .iter_mut()
            .find(|account| &account.id == id)
            .ok_or("account_not_found")?;
        if account.revoking {
            return Ok(false);
        }
        account.admission_revision =
            Some(self.revision.checked_add(1).ok_or("revision_exhausted")?);
        account.revoking = true;
        account.enabled = false;
        self.defaults.retain(|_, value| value != id);
        Ok(true)
    }

    fn set_enabled(
        &mut self,
        id: &ProviderAccountId,
        enabled: bool,
    ) -> std::result::Result<(), &'static str> {
        if !enabled && self.defaults.values().any(|value| value == id) {
            return Err("account_is_default");
        }
        let account = self
            .accounts
            .iter_mut()
            .find(|account| &account.id == id)
            .ok_or("account_not_found")?;
        if enabled && account.revoking {
            return Err("account_revoking");
        }
        if account.enabled != enabled {
            account.admission_revision =
                Some(self.revision.checked_add(1).ok_or("revision_exhausted")?);
            account.enabled = enabled;
        }
        Ok(())
    }

    fn apply(&mut self, change: &Change) -> std::result::Result<ProviderAccountId, &'static str> {
        let id = match change {
            Change::BeginRevocation { id } => {
                if !self.begin_revocation(id)? {
                    return Ok(id.clone());
                }
                id.clone()
            }
            Change::BindIdentity {
                id,
                provider_id,
                identity,
            } => {
                let fingerprint = identity.fingerprint(provider_id)?;
                if self.accounts.iter().any(|account| {
                    &account.id != id
                        && &account.provider_id == provider_id
                        && account.identity_fingerprint.as_ref() == Some(&fingerprint)
                }) {
                    return Err("native_identity_already_bound");
                }
                let account = self
                    .accounts
                    .iter_mut()
                    .find(|account| {
                        &account.id == id
                            && &account.provider_id == provider_id
                            && account.available()
                    })
                    .ok_or("account_unavailable")?;
                if let Some(bound) = &account.identity_fingerprint {
                    return if bound == &fingerprint {
                        Ok(id.clone())
                    } else {
                        Err("native_identity_mismatch")
                    };
                }
                account.identity_fingerprint = Some(fingerprint);
                account.admission_revision =
                    Some(self.revision.checked_add(1).ok_or("revision_exhausted")?);
                id.clone()
            }
            Change::Create { provider_id, label } => {
                if !provider_id_valid(provider_id) || !label_valid(label) {
                    return Err("invalid_account");
                }
                if self.accounts.len() >= 128 {
                    return Err("account_capacity");
                }
                let id = ProviderAccountId::try_from(uuid::Uuid::new_v4().to_string())
                    .expect("canonical UUID");
                self.accounts.push(Account {
                    id: id.clone(),
                    provider_id: provider_id.clone(),
                    label: label.trim().to_owned(),
                    enabled: true,
                    revoking: false,
                    identity_fingerprint: None,
                    admission_revision: Some(
                        self.revision.checked_add(1).ok_or("revision_exhausted")?,
                    ),
                });
                id
            }
            Change::Rename { id, label } => {
                if !label_valid(label) {
                    return Err("invalid_account");
                }
                label.trim().clone_into(
                    &mut self
                        .accounts
                        .iter_mut()
                        .find(|a| &a.id == id)
                        .ok_or("account_not_found")?
                        .label,
                );
                id.clone()
            }
            Change::SetEnabled { id, enabled } => {
                self.set_enabled(id, *enabled)?;
                id.clone()
            }
            Change::SetDefault { provider_id, id } => {
                if !provider_id_valid(provider_id) {
                    return Err("invalid_provider");
                }
                if id.is_default() {
                    self.defaults.remove(provider_id);
                } else {
                    self.accounts
                        .iter()
                        .find(|a| &a.id == id && &a.provider_id == provider_id && a.available())
                        .ok_or("account_unavailable")?;
                    self.defaults.insert(provider_id.clone(), id.clone());
                }
                id.clone()
            }
        };
        self.revision = self.revision.checked_add(1).ok_or("revision_exhausted")?;
        Ok(id)
    }

    /// Resolve only at creation. Recovery uses the persisted session identity.
    pub(crate) fn resolve(
        &self,
        provider: &str,
        explicit: Option<&ProviderAccountId>,
    ) -> std::result::Result<ProviderAccountId, &'static str> {
        let id = explicit
            .cloned()
            .or_else(|| self.defaults.get(provider).cloned())
            .unwrap_or_default();
        if !id.is_default()
            && !self
                .accounts
                .iter()
                .any(|a| a.id == id && a.provider_id == provider && a.available())
        {
            return Err("account_unavailable");
        }
        Ok(id)
    }
}

impl Store {
    pub(crate) async fn provider_accounts(&self, owner: &str) -> Result<Registry> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT value FROM provider_account_registries WHERE owner_user_id=$1",
                )
                .bind(owner)
                .fetch_optional(&$db.pool)
                .await?
            };
        }
        let value = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        Ok(value
            .map(|value| serde_json::from_str(&value))
            .transpose()?
            .unwrap_or_default())
    }

    pub(crate) async fn provider_account_receipt(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<Receipt>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_scalar::<_, String>("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_optional(&$db.pool).await?
        }; }
        let value = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        Ok(value
            .map(|value| serde_json::from_str(&value))
            .transpose()?)
    }

    pub(crate) async fn mutate_provider_accounts(
        &self,
        owner: &str,
        operation: &str,
        input: &Mutation,
    ) -> Result<MutationResult> {
        if input.expected_revision < 0 || !crate::managed_calls::valid_id(operation) {
            return Ok(MutationResult::Invalid("invalid_argument"));
        }
        let hash = crate::admin::hex_sha256(&serde_json::to_vec(input)?);
        macro_rules! write { ($db:expr, $cancel:literal, $cancel_grants:literal, $login_account:literal, $grant_account:literal) => {{
            let mut tx = $db.pool.begin().await?;
            // This write acquires the owner lock before any reads, on both DBs.
            sqlx::query("INSERT INTO provider_account_registries (owner_user_id,revision,value) VALUES ($1,0,$2) ON CONFLICT (owner_user_id) DO UPDATE SET revision=provider_account_registries.revision")
                .bind(owner).bind(serde_json::to_string(&Registry::default())?).execute(&mut *tx).await?;
            let old: Option<(String,String)> = sqlx::query_as("SELECT request_hash,result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
            if let Some((old_hash, result)) = old {
                return Ok(if old_hash == hash { MutationResult::Applied(serde_json::from_str(&result)?) }
                    else { MutationResult::Invalid("operation_reused") });
            }
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let mut registry: Registry = serde_json::from_str(&value)?;
            if registry.revision != input.expected_revision { return Ok(MutationResult::Conflict(registry)); }
            let account_id = match registry.apply(&input.change) {
                Ok(id) => id, Err(code) => return Ok(MutationResult::Invalid(code)),
            };
            let revocation = if matches!(input.change, Change::BeginRevocation { .. }) {
                // Capture even offline/revoked recipients before cancellation.
                // These locators are work to reconcile, never cleanup evidence.
                let logins: Vec<(String, String)> = sqlx::query_as(concat!("SELECT operation_id,value FROM provider_login_operations WHERE owner_user_id=$1 AND ", $login_account, "=$2 ORDER BY operation_id"))
                    .bind(owner).bind(account_id.as_str()).fetch_all(&mut *tx).await?;
                let grants: Vec<(String, String)> = sqlx::query_as(concat!("SELECT operation_id,value FROM provider_account_grants WHERE owner_user_id=$1 AND ", $grant_account, "=$2 ORDER BY operation_id"))
                    .bind(owner).bind(account_id.as_str()).fetch_all(&mut *tx).await?;
                let sessions: Vec<(String, Option<String>)> = sqlx::query_as("SELECT id,machine_id FROM sessions WHERE owner_user_id=$1 AND provider_account_id=$2 ORDER BY id")
                    .bind(owner).bind(account_id.as_str()).fetch_all(&mut *tx).await?;
                Some(revocation::Plan::capture(&account_id, logins, grants, sessions)?)
            } else { None };
            if matches!(input.change, Change::BeginRevocation { .. }) {
                // Same transaction and owner lock as the admission fence. This
                // is cancellation intent only, not native process-exit evidence.
                sqlx::query($cancel).bind(owner).bind(account_id.as_str())
                    .execute(&mut *tx).await?;
                // Resolve pending promotions under the same owner lock used
                // by grant commit. Committed grants retain their evidence and
                // still require vault/replica revocation, never cancellation.
                sqlx::query($cancel_grants).bind(owner).bind(account_id.as_str())
                    .execute(&mut *tx).await?;
            }
            sqlx::query("UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3")
                .bind(registry.revision).bind(serde_json::to_string(&registry)?).bind(owner).execute(&mut *tx).await?;
            let receipt = Receipt { registry, account_id, revocation, revocation_completed: false, remote_wipes: BTreeMap::new(), local_wipes: BTreeMap::new(), login_stops: BTreeMap::new() };
            sqlx::query("INSERT INTO provider_account_operations (owner_user_id,operation_id,request_hash,result) VALUES ($1,$2,$3,$4)")
                .bind(owner).bind(operation).bind(&hash).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            MutationResult::Applied(receipt)
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => write!(
                db,
                "UPDATE provider_login_operations SET value=jsonb_set(value::jsonb, '{cancel_requested}', 'true'::jsonb)::text WHERE owner_user_id=$1 AND value::jsonb #>> '{input,account_id}'=$2",
                "UPDATE provider_account_grants SET value=jsonb_set(value::jsonb, '{phase}', '{\"state\":\"cancelled\"}'::jsonb)::text WHERE owner_user_id=$1 AND value::jsonb ->> 'account_id'=$2 AND value::jsonb #>> '{phase,state}'='prepared'",
                "value::jsonb #>> '{input,account_id}'",
                "value::jsonb ->> 'account_id'"
            ),
            StorageBackend::Sqlite(db) => write!(
                db,
                "UPDATE provider_login_operations SET value=json_set(value, '$.cancel_requested', json('true')) WHERE owner_user_id=$1 AND json_extract(value, '$.input.account_id')=$2",
                "UPDATE provider_account_grants SET value=json_set(value, '$.phase', json('{\"state\":\"cancelled\"}')) WHERE owner_user_id=$1 AND json_extract(value, '$.account_id')=$2 AND json_extract(value, '$.phase.state')='prepared'",
                "json_extract(value, '$.input.account_id')",
                "json_extract(value, '$.account_id')"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn reserve_before_revocation(store: &Store, id: &ProviderAccountId) -> logins::Claim {
        let owner = "revocation-owner";
        store
            .claim_provider_login(
                owner,
                "before-revoke",
                &logins::Request {
                    account_id: id.clone(),
                    machine_id: "machine".into(),
                    provider_version: "fixture".into(),
                    generation_digest: "sha256:fixture".into(),
                    method: "native".into(),
                },
            )
            .await
            .unwrap()
    }

    async fn assert_revoked_dispatch(store: &Store, reserved: &logins::Claim) {
        let owner = "revocation-owner";
        let plan = store
            .provider_account_receipt(owner, "revoke")
            .await
            .unwrap()
            .unwrap()
            .revocation
            .unwrap();
        assert_eq!(
            plan.logins,
            vec![revocation::OperationTarget {
                operation_id: "before-revoke".into(),
                machine_id: "machine".into(),
            }]
        );
        assert!(plan.grants.is_empty());
        assert_eq!(plan.sessions.len(), 1);
        assert_eq!(plan.sessions[0].session_id, "revocation-deleted-session");
        assert!(plan.sessions[0].machine_id.is_some());
        let mut expected = reserved.receipt.clone();
        expected.cancel_requested = true;
        assert_eq!(
            store
                .provider_login_receipt(owner, "before-revoke")
                .await
                .unwrap(),
            Some(expected)
        );
        let denied = store
            .with_provider_login_dispatch(
                owner,
                "before-revoke",
                reserved,
                &logins::DispatchScope {
                    registry_revision: 2,
                    recipient_fingerprint: "unused".into(),
                },
                || -> anyhow::Result<()> { panic!("revoked reservation cannot dispatch") },
            )
            .await
            .unwrap_err();
        assert_eq!(denied.to_string(), "login_not_admitted");
    }

    async fn revocation_contract(store: &Store) -> Registry {
        let owner = "revocation-owner";
        let MutationResult::Applied(created) = store
            .mutate_provider_accounts(owner, "create", &create("Personal"))
            .await
            .unwrap()
        else {
            panic!("create");
        };
        let id = created.account_id;
        seed_deleted_revocation_session(store, &id).await;
        store
            .mutate_provider_accounts(
                owner,
                "default",
                &Mutation {
                    expected_revision: 1,
                    change: Change::SetDefault {
                        provider_id: "claude-code".into(),
                        id: id.clone(),
                    },
                },
            )
            .await
            .unwrap();
        let input = Mutation {
            expected_revision: 2,
            change: Change::BeginRevocation { id: id.clone() },
        };
        let reserved = reserve_before_revocation(store, &id).await;
        assert!(serde_json::from_value::<Mutation>(serde_json::to_value(&input).unwrap()).is_err());
        let (first, retry) = tokio::join!(
            store.mutate_provider_accounts(owner, "revoke", &input),
            store.mutate_provider_accounts(owner, "revoke", &input)
        );
        assert_eq!(first.unwrap(), retry.unwrap());
        let registry = store.provider_accounts(owner).await.unwrap();
        assert_eq!(registry.revision, 3);
        assert!(registry.accounts[0].revoking);
        assert!(!registry.accounts[0].enabled);
        assert!(registry.defaults.is_empty());
        assert_revoked_dispatch(store, &reserved).await;
        assert!(registry.resolve("claude-code", Some(&id)).is_err());
        let enable = Mutation {
            expected_revision: 3,
            change: Change::SetEnabled {
                id: id.clone(),
                enabled: true,
            },
        };
        assert_eq!(
            store
                .mutate_provider_accounts(owner, "enable", &enable)
                .await
                .unwrap(),
            MutationResult::Invalid("account_revoking")
        );
        let repeat = Mutation {
            expected_revision: 3,
            ..input
        };
        assert!(matches!(
            store
                .mutate_provider_accounts(owner, "revoke-again", &repeat)
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
        assert_eq!(store.provider_accounts(owner).await.unwrap(), registry);
        assert_eq!(
            store
                .claim_provider_login(
                    owner,
                    "late-login",
                    &logins::Request {
                        account_id: id,
                        machine_id: "machine".into(),
                        provider_version: "fixture".into(),
                        generation_digest: "sha256:fixture".into(),
                        method: "native".into(),
                    }
                )
                .await
                .unwrap_err()
                .to_string(),
            "account_unavailable"
        );
        let mut inconsistent = registry.clone();
        inconsistent.accounts[0].enabled = true;
        assert!(!inconsistent.accounts[0].available());
        assert!(
            inconsistent
                .resolve("claude-code", Some(&inconsistent.accounts[0].id))
                .is_err()
        );
        registry
    }

    async fn seed_deleted_revocation_session(store: &Store, account: &ProviderAccountId) {
        let owner = "revocation-owner";
        let query = "INSERT INTO users (id,username,password_algo,password_hash) VALUES ($1,$1,'fixture','fixture')";
        match &store.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query(query)
                    .bind(owner)
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query(query)
                    .bind(owner)
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
        }
        let machine = store
            .list_machines()
            .await
            .unwrap()
            .into_iter()
            .find(|machine| machine.connection_mode == "local")
            .unwrap();
        let id = "revocation-deleted-session";
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            id.into(),
            "claude-code".into(),
            "/fixture".into(),
            "fixture".into(),
            crate::core::SessionOrigin::Api,
            false,
        );
        let mut meta = hub.session_info(id).unwrap().meta;
        meta.machine_id = machine.id;
        meta.owner_user_id = Some(owner.into());
        meta.provider_account_id = account.clone();
        store.insert_session(&meta).await.unwrap();
        store.delete_session(id).await.unwrap();
    }

    #[tokio::test]
    async fn sqlite_revocation_fence_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            temp.path().join("accounts.sqlite").display()
        );
        let store = Store::connect(&url, temp.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let expected = revocation_contract(&store).await;
        revocation::grant_tests::contract(&store).await;
        let stop = stop_evidence_contract(&store).await;
        let receipt = store
            .provider_account_receipt("revocation-owner", "revoke")
            .await
            .unwrap();
        drop(store);
        let store = Store::connect(&url, temp.path().join("artifacts"))
            .await
            .unwrap();
        assert_eq!(
            store.provider_accounts("revocation-owner").await.unwrap(),
            expected
        );
        revocation::grant_tests::persisted(&store).await;
        assert_eq!(
            store
                .provider_account_stop("revocation-owner", "revoke", &stop.session_id)
                .await
                .unwrap(),
            Some(stop)
        );
        assert_eq!(
            store
                .provider_account_receipt("revocation-owner", "revoke")
                .await
                .unwrap(),
            receipt
        );
        assert!(
            store
                .provider_login_receipt("revocation-owner", "before-revoke")
                .await
                .unwrap()
                .unwrap()
                .cancel_requested
        );
        cancellation_scope_contract(&store).await;
    }

    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
    async fn postgres_revocation_fence() {
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = Store::connect(&url, temp.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        revocation_contract(&store).await;
        revocation::grant_tests::contract(&store).await;
        stop_evidence_contract(&store).await;
        cancellation_scope_contract(&store).await;
    }

    async fn cancellation_scope_contract(store: &Store) {
        let owner = "cancel-scope-owner";
        let mut claims = Vec::new();
        for (index, label) in ["First", "Second"].into_iter().enumerate() {
            let mut input = create(label);
            input.expected_revision = i64::try_from(index).unwrap();
            let MutationResult::Applied(created) = store
                .mutate_provider_accounts(owner, label, &input)
                .await
                .unwrap()
            else {
                panic!("create scoped account")
            };
            let claim = store
                .claim_provider_login(
                    owner,
                    label,
                    &logins::Request {
                        account_id: created.account_id,
                        machine_id: "machine".into(),
                        provider_version: "fixture".into(),
                        generation_digest: "sha256:fixture".into(),
                        method: "native".into(),
                    },
                )
                .await
                .unwrap();
            claims.push(claim);
        }
        let stale = store
            .mutate_provider_accounts(
                owner,
                "stale-revoke",
                &Mutation {
                    expected_revision: 1,
                    change: Change::BeginRevocation {
                        id: claims[0].receipt.input.account_id.clone(),
                    },
                },
            )
            .await
            .unwrap();
        assert!(matches!(stale, MutationResult::Conflict(_)));
        assert_eq!(
            store.provider_login_receipt(owner, "First").await.unwrap(),
            Some(claims[0].receipt.clone())
        );
        // A terminal refusal is historical evidence and must survive cancellation.
        store
            .reject_provider_login(
                owner,
                "First",
                &claims[0],
                logins::DispatchRejection::StartRejected,
            )
            .await
            .unwrap();
        let mut expected = store
            .provider_login_receipt(owner, "First")
            .await
            .unwrap()
            .unwrap();
        expected.cancel_requested = true;
        let result = store
            .mutate_provider_accounts(
                owner,
                "revoke-scoped",
                &Mutation {
                    expected_revision: 2,
                    change: Change::BeginRevocation {
                        id: claims[0].receipt.input.account_id.clone(),
                    },
                },
            )
            .await
            .unwrap();
        assert_scoped_plan(result);
        assert_eq!(
            store.provider_login_receipt(owner, "First").await.unwrap(),
            Some(expected)
        );
        assert_eq!(
            store.provider_login_receipt(owner, "Second").await.unwrap(),
            Some(claims[1].receipt.clone())
        );
    }

    async fn set_stop_fixture_key(store: &Store, machine: &str, key: &str) {
        let query = "UPDATE machines SET encryption_public_key=$2 WHERE id=$1";
        match &store.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query(query)
                    .bind(machine)
                    .bind(key)
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query(query)
                    .bind(machine)
                    .bind(key)
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
        }
    }

    async fn stop_evidence_contract(store: &Store) -> revocation::stops::Evidence {
        use revocation::stops::Evidence;
        let owner = "revocation-owner";
        let receipt = store
            .provider_account_receipt(owner, "revoke")
            .await
            .unwrap()
            .unwrap();
        let plan = receipt.revocation.as_ref().unwrap();
        let target = &plan.sessions[0];
        let mut evidence = Evidence {
            tree_exit: None,
            account_id: receipt.account_id.clone(),
            session_id: target.session_id.clone(),
            machine_id: target.machine_id.clone().unwrap(),
            recipient_fingerprint: crate::admin::hex_sha256(b"stop-fixture-key"),
        };
        set_stop_fixture_key(store, &evidence.machine_id, "stop-fixture-key").await;
        assert_stop_dispatch(store, &evidence).await;
        let mut wrong = evidence.clone();
        wrong.session_id = "other-session".into();
        assert_eq!(
            store
                .record_provider_account_stop(owner, "revoke", &wrong, || true)
                .await
                .unwrap_err()
                .to_string(),
            "stop_scope_mismatch"
        );
        wrong = evidence.clone();
        wrong.recipient_fingerprint = "wrong-key".into();
        assert_eq!(
            store
                .record_provider_account_stop(owner, "revoke", &wrong, || true)
                .await
                .unwrap_err()
                .to_string(),
            "machine_enrollment_changed"
        );
        assert_eq!(
            store
                .record_provider_account_stop(owner, "revoke", &evidence, || false)
                .await
                .unwrap_err()
                .to_string(),
            "machine_connection_changed"
        );
        assert!(
            store
                .provider_account_stop(owner, "revoke", &evidence.session_id)
                .await
                .unwrap()
                .is_none()
        );
        let (first, retry) = tokio::join!(
            store.record_provider_account_stop(owner, "revoke", &evidence, || true),
            store.record_provider_account_stop(owner, "revoke", &evidence, || true),
        );
        assert_eq!(first.unwrap(), evidence);
        assert_eq!(retry.unwrap(), evidence);
        let readiness = store
            .provider_revocation_readiness(owner, "revoke")
            .await
            .unwrap();
        assert!(!readiness.account_completed);
        assert!(
            !readiness
                .missing
                .iter()
                .any(|missing| missing.kind == "session_stop"
                    && missing.target == evidence.session_id)
        );
        assert!(
            readiness
                .missing
                .iter()
                .any(|missing| missing.kind == "session_process_tree_exit"
                    && missing.target == evidence.session_id)
        );
        assert_eq!(
            store
                .provider_account_receipt(owner, "revoke")
                .await
                .unwrap(),
            Some(receipt)
        );
        assert!(
            store
                .provider_account_stop("other-owner", "revoke", &evidence.session_id)
                .await
                .unwrap()
                .is_none()
        );
        assert_tree_upgrade(store, &mut evidence).await;
        assert_stop_history(store, &evidence, &wrong).await;
        evidence
    }

    async fn assert_tree_upgrade(store: &Store, evidence: &mut revocation::stops::Evidence) {
        let owner = "revocation-owner";
        evidence.tree_exit = Some(crate::provider_account::WorkerExitObservation {
            request: crate::provider_account::WorkerExitRequest {
                stop_command_id: None,
                account_id: evidence.account_id.clone(),
                session_id: evidence.session_id.clone(),
            },
            launch_id: uuid::Uuid::new_v4(),
        });
        assert!(
            store
                .record_provider_account_stop(owner, "revoke", evidence, || false)
                .await
                .is_err()
        );
        store
            .record_provider_account_stop(owner, "revoke", evidence, || true)
            .await
            .unwrap();
        let report = store
            .provider_revocation_readiness(owner, "revoke")
            .await
            .unwrap();
        assert!(!report.account_completed);
        assert!(
            !report
                .missing
                .iter()
                .any(|item| item.kind == "session_process_tree_exit"
                    && item.target == evidence.session_id)
        );
        let mut changed_launch = evidence.clone();
        changed_launch.tree_exit.as_mut().unwrap().launch_id = uuid::Uuid::new_v4();
        assert!(
            store
                .record_provider_account_stop(owner, "revoke", &changed_launch, || true)
                .await
                .is_err()
        );
    }

    async fn assert_stop_history(
        store: &Store,
        evidence: &revocation::stops::Evidence,
        wrong: &revocation::stops::Evidence,
    ) {
        let owner = "revocation-owner";
        set_stop_fixture_key(store, &evidence.machine_id, "replacement-key").await;
        assert_eq!(
            store
                .dispatch_provider_account_stop(
                    "revocation-owner",
                    "revoke",
                    evidence,
                    || -> anyhow::Result<()> {
                        panic!("historical evidence cannot authorize dispatch")
                    }
                )
                .await
                .unwrap_err()
                .to_string(),
            "machine_enrollment_changed"
        );
        assert_eq!(
            store
                .record_provider_account_stop(owner, "revoke", evidence, || panic!(
                    "historical read is not new admission"
                ))
                .await
                .unwrap(),
            *evidence
        );
        assert_eq!(
            store
                .record_provider_account_stop(owner, "revoke", wrong, || true)
                .await
                .unwrap_err()
                .to_string(),
            "stop_evidence_conflict"
        );
    }

    async fn assert_stop_dispatch(store: &Store, evidence: &revocation::stops::Evidence) {
        let mut wrong = evidence.clone();
        wrong.session_id = "outside-plan".into();
        assert_eq!(
            store
                .dispatch_provider_account_stop(
                    "revocation-owner",
                    "revoke",
                    &wrong,
                    || -> anyhow::Result<()> { panic!("out-of-scope dispatch") }
                )
                .await
                .unwrap_err()
                .to_string(),
            "stop_scope_mismatch"
        );
        wrong = evidence.clone();
        wrong.recipient_fingerprint = "changed".into();
        assert_eq!(
            store
                .dispatch_provider_account_stop(
                    "revocation-owner",
                    "revoke",
                    &wrong,
                    || -> anyhow::Result<()> { panic!("wrong enrollment dispatch") }
                )
                .await
                .unwrap_err()
                .to_string(),
            "machine_enrollment_changed"
        );
        assert_eq!(
            store
                .dispatch_provider_account_stop(
                    "revocation-owner",
                    "revoke",
                    evidence,
                    || -> anyhow::Result<()> { anyhow::bail!("machine_connection_changed") }
                )
                .await
                .unwrap_err()
                .to_string(),
            "machine_connection_changed"
        );
        let result = store
            .dispatch_provider_account_stop("revocation-owner", "revoke", evidence, || Ok(42))
            .await
            .unwrap();
        assert_eq!(result, 42);
        assert!(
            store
                .provider_account_stop("revocation-owner", "revoke", &evidence.session_id)
                .await
                .unwrap()
                .is_none()
        );
    }

    fn assert_scoped_plan(result: MutationResult) {
        let MutationResult::Applied(receipt) = result else {
            panic!("revocation receipt")
        };
        let plan = receipt.revocation.unwrap();
        assert_eq!(
            plan.logins,
            vec![revocation::OperationTarget {
                operation_id: "First".into(),
                machine_id: "machine".into(),
            }]
        );
        assert!(plan.grants.is_empty() && plan.sessions.is_empty());
    }

    fn bind(id: &ProviderAccountId, revision: i64, subject: &str) -> Mutation {
        Mutation {
            expected_revision: revision,
            change: Change::BindIdentity {
                id: id.clone(),
                provider_id: "claude-code".into(),
                identity: NativeIdentity {
                    authority: "native-fixture".into(),
                    subject: subject.into(),
                    tenant: Some("tenant".into()),
                },
            },
        }
    }

    #[test]
    fn native_identity_cannot_be_supplied_through_public_mutations() {
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let internal = serde_json::to_value(bind(&id, 1, "subject")).unwrap();
        assert!(serde_json::from_value::<Mutation>(internal).is_err());
        let mut public = serde_json::to_value(create("Personal")).unwrap();
        public["change"]["identity_fingerprint"] = json!("forged");
        assert!(serde_json::from_value::<Mutation>(public).is_err());
    }

    #[test]
    fn native_identity_fingerprint_is_scoped_and_unambiguous() {
        let identity = NativeIdentity {
            authority: "native".into(),
            subject: "a:b".into(),
            tenant: Some("c".into()),
        };
        let fingerprint = identity.fingerprint("claude-code").unwrap();
        assert_ne!(fingerprint, identity.fingerprint("codex").unwrap());
        for changed in [
            NativeIdentity {
                authority: "other".into(),
                ..identity.clone()
            },
            NativeIdentity {
                subject: "a".into(),
                tenant: Some("b:c".into()),
                ..identity.clone()
            },
            NativeIdentity {
                tenant: None,
                ..identity.clone()
            },
        ] {
            assert_ne!(fingerprint, changed.fingerprint("claude-code").unwrap());
        }
        for subject in ["", " subject", "subject\n", &"x".repeat(513)] {
            assert!(
                NativeIdentity {
                    subject: subject.into(),
                    ..identity.clone()
                }
                .fingerprint("claude-code")
                .is_err()
            );
        }
    }

    #[test]
    fn identity_binding_rejects_duplicates_disabled_accounts_and_wrong_providers() {
        let mut registry = Registry::default();
        let first = registry.apply(&create("First").change).unwrap();
        let second = registry.apply(&create("Second").change).unwrap();
        registry.apply(&bind(&first, 2, "subject").change).unwrap();
        let before = registry.clone();
        assert_eq!(
            registry.apply(&bind(&second, 3, "subject").change),
            Err("native_identity_already_bound")
        );
        let mut wrong_provider = bind(&second, 3, "other").change;
        if let Change::BindIdentity { provider_id, .. } = &mut wrong_provider {
            *provider_id = "codex".into();
        }
        assert_eq!(registry.apply(&wrong_provider), Err("account_unavailable"));
        assert_eq!(registry, before);
        registry
            .apply(&Change::SetEnabled {
                id: first.clone(),
                enabled: false,
            })
            .unwrap();
        let disabled = registry.clone();
        assert_eq!(
            registry.apply(&bind(&first, 4, "subject").change),
            Err("account_unavailable")
        );
        assert_eq!(registry, disabled);
        assert_eq!(
            registry.apply(&bind(&ProviderAccountId::default(), 4, "unbound").change),
            Err("account_unavailable")
        );
    }

    async fn identity_contract(store: &Store) {
        let owner = format!("identity-{}", uuid::Uuid::new_v4());
        let MutationResult::Applied(created) = store
            .mutate_provider_accounts(&owner, "create", &create("Personal"))
            .await
            .unwrap()
        else {
            panic!("create")
        };
        let id = created.account_id;
        let first = bind(&id, 1, "first");
        let second = bind(&id, 1, "second");
        let (a, b) = tokio::join!(
            store.mutate_provider_accounts(&owner, "login-a", &first),
            store.mutate_provider_accounts(&owner, "login-b", &second),
        );
        let (winner, loser, operation) = match (a.unwrap(), b.unwrap()) {
            (MutationResult::Applied(_), MutationResult::Conflict(_)) => (first, second, "login-a"),
            (MutationResult::Conflict(_), MutationResult::Applied(_)) => (second, first, "login-b"),
            outcomes => panic!("unexpected outcomes: {outcomes:?}"),
        };
        let snapshot = store.provider_accounts(&owner).await.unwrap();
        assert_eq!(snapshot.revision, 2);
        assert!(snapshot.accounts[0].identity_fingerprint.is_some());
        assert!(matches!(
            store
                .mutate_provider_accounts(&owner, operation, &winner)
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
        assert_eq!(
            store
                .mutate_provider_accounts(
                    &owner,
                    "mismatch",
                    &Mutation {
                        expected_revision: 2,
                        ..loser
                    }
                )
                .await
                .unwrap(),
            MutationResult::Invalid("native_identity_mismatch")
        );
        assert!(matches!(
            store
                .mutate_provider_accounts(
                    &owner,
                    "same-native-account",
                    &Mutation {
                        expected_revision: 2,
                        ..winner.clone()
                    }
                )
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
        assert_eq!(store.provider_accounts(&owner).await.unwrap(), snapshot);
        let rename = Mutation {
            expected_revision: 2,
            change: Change::Rename {
                id: id.clone(),
                label: "New label".into(),
            },
        };
        assert!(matches!(
            store
                .mutate_provider_accounts(&owner, "rename", &rename)
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
        assert_eq!(
            store.provider_accounts(&owner).await.unwrap().accounts[0].identity_fingerprint,
            snapshot.accounts[0].identity_fingerprint
        );
        assert_eq!(
            store
                .mutate_provider_accounts(
                    "other-owner",
                    "bind",
                    &Mutation {
                        expected_revision: 0,
                        ..winner
                    }
                )
                .await
                .unwrap(),
            MutationResult::Invalid("account_unavailable")
        );
    }

    fn create(label: &str) -> Mutation {
        Mutation {
            expected_revision: 0,
            change: Change::Create {
                provider_id: "claude-code".into(),
                label: label.into(),
            },
        }
    }

    #[allow(clippy::too_many_lines)] // One persistence, replay and ownership contract for both DBs.
    async fn contract(store: &Store) {
        identity_contract(store).await;
        let owner = format!("accounts-{}", uuid::Uuid::new_v4());
        let other = format!("{owner}-other");
        let input = create("Personal Max");
        let first = store
            .mutate_provider_accounts(&owner, "create", &input)
            .await
            .unwrap();
        let MutationResult::Applied(ref receipt) = first else {
            panic!("create failed")
        };
        let id = receipt.account_id.clone();
        assert!(!id.is_default());
        assert!(receipt.registry.defaults.is_empty());
        assert_eq!(
            store
                .mutate_provider_accounts(&owner, "create", &input)
                .await
                .unwrap(),
            first
        );
        assert_eq!(
            store
                .mutate_provider_accounts(&owner, "create", &create("Other"))
                .await
                .unwrap(),
            MutationResult::Invalid("operation_reused")
        );
        assert!(
            store
                .provider_accounts(&other)
                .await
                .unwrap()
                .accounts
                .is_empty()
        );
        assert!(
            store
                .provider_account_receipt(&other, "create")
                .await
                .unwrap()
                .is_none()
        );
        let default = Mutation {
            expected_revision: 1,
            change: Change::SetDefault {
                provider_id: "claude-code".into(),
                id: id.clone(),
            },
        };
        let rename = Mutation {
            expected_revision: 1,
            change: Change::Rename {
                id: id.clone(),
                label: "Renamed".into(),
            },
        };
        let (a, b) = tokio::join!(
            store.mutate_provider_accounts(&owner, "default", &default),
            store.mutate_provider_accounts(&owner, "rename", &rename)
        );
        assert!(matches!(
            (a.unwrap(), b.unwrap()),
            (MutationResult::Applied(_), MutationResult::Conflict(_))
                | (MutationResult::Conflict(_), MutationResult::Applied(_))
        ));
        let current = store.provider_accounts(&owner).await.unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(current.accounts.len(), 1);
        // The first receipt is immutable even after later edits.
        assert_eq!(
            store
                .provider_account_receipt(&owner, "create")
                .await
                .unwrap()
                .as_ref(),
            Some(receipt)
        );
        assert_eq!(
            store
                .mutate_provider_accounts(&owner, "create", &input)
                .await
                .unwrap(),
            first
        );
        let default = Mutation {
            expected_revision: 2,
            ..default
        };
        assert!(matches!(
            store
                .mutate_provider_accounts(&owner, "set-default", &default)
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
        let current = store.provider_accounts(&owner).await.unwrap();
        assert_eq!(current.resolve("claude-code", None).unwrap(), id);
        assert!(current.resolve("codex", Some(&id)).is_err());
        assert!(
            current
                .resolve("claude-code", Some(&ProviderAccountId::default()))
                .unwrap()
                .is_default()
        );
        let disable = Mutation {
            expected_revision: 3,
            change: Change::SetEnabled {
                id: id.clone(),
                enabled: false,
            },
        };
        assert_eq!(
            store
                .mutate_provider_accounts(&owner, "disable-default", &disable)
                .await
                .unwrap(),
            MutationResult::Invalid("account_is_default")
        );
        let reset = Mutation {
            expected_revision: 3,
            change: Change::SetDefault {
                provider_id: "claude-code".into(),
                id: ProviderAccountId::default(),
            },
        };
        store
            .mutate_provider_accounts(&owner, "reset", &reset)
            .await
            .unwrap();
        let disable = Mutation {
            expected_revision: 4,
            ..disable
        };
        assert!(matches!(
            store
                .mutate_provider_accounts(&owner, "disable", &disable)
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
        let current = store.provider_accounts(&owner).await.unwrap();
        assert!(current.resolve("claude-code", Some(&id)).is_err());
        assert!(current.resolve("claude-code", None).unwrap().is_default());
        assert_eq!(
            store.provider_accounts(&other).await.unwrap(),
            Registry::default()
        );
    }

    #[tokio::test]
    async fn sqlite_accounts_are_atomic_owner_scoped_and_persistent() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("accounts.sqlite").display()
        );
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        contract(&store).await;
        let applied = store
            .mutate_provider_accounts("persist", "create", &create("Personal Pro"))
            .await
            .unwrap();
        let MutationResult::Applied(ref created) = applied else {
            panic!("create")
        };
        let bound = store
            .mutate_provider_accounts(
                "persist",
                "native-login",
                &bind(&created.account_id, 1, "persisted-subject"),
            )
            .await
            .unwrap();
        let MutationResult::Applied(bound) = bound else {
            panic!("bind")
        };
        drop(store);
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        assert_eq!(
            store.provider_accounts("persist").await.unwrap(),
            bound.registry
        );
        assert_eq!(
            store
                .mutate_provider_accounts("persist", "create", &create("Personal Pro"))
                .await
                .unwrap(),
            applied
        );
        assert_eq!(
            store
                .provider_accounts("persist")
                .await
                .unwrap()
                .accounts
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn invalid_session_account_is_quarantined_without_default_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("sessions.sqlite").display()
        );
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let hub = crate::core::Hub::new();
        let machine_id = store
            .list_machines()
            .await
            .unwrap()
            .into_iter()
            .find(|machine| machine.connection_mode == "local")
            .unwrap()
            .id;
        for id in ["valid", "invalid"] {
            hub.create_local_session(
                id.into(),
                "claude-code".into(),
                "/work".into(),
                id.into(),
                crate::core::SessionOrigin::Api,
                false,
            );
            let mut meta = hub.session_info(id).unwrap().meta;
            meta.machine_id.clone_from(&machine_id);
            store.insert_session(&meta).await.unwrap();
        }
        let StorageBackend::Sqlite(db) = &store.backend else {
            unreachable!()
        };
        sqlx::query("UPDATE sessions SET provider_account_id='../default' WHERE id='invalid'")
            .execute(&db.pool)
            .await
            .unwrap();
        let restored = store.load_all().await.unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].meta.id, "valid");
        assert!(restored[0].meta.provider_account_id.is_default());
        // Quarantine is non-destructive and never rewrites the bad identity.
        let bad: String =
            sqlx::query_scalar("SELECT provider_account_id FROM sessions WHERE id='invalid'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(bad, "../default");
    }

    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
    async fn postgres_accounts_contract() {
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        contract(&store).await;
    }

    #[test]
    fn account_metadata_is_closed_and_contains_no_credentials_or_machine_placement() {
        let input = json!({"expected_revision":0,"change":{"type":"create","provider_id":"claude-code","label":"Pro"}});
        for field in ["owner", "machine_id", "token", "auth_generation", "id"] {
            let mut value = input.clone();
            value["change"][field] = json!("injected");
            assert!(serde_json::from_value::<Mutation>(value).is_err());
        }
        for label in ["", "  ", "secret\n", &"a".repeat(129)] {
            assert!(Registry::default().apply(&create(label).change).is_err());
        }
        let mut registry = Registry::default();
        for _ in 0..128 {
            registry
                .apply(&create("same label is not identity").change)
                .unwrap();
        }
        assert_eq!(
            registry.apply(&create("overflow").change),
            Err("account_capacity")
        );
        assert_eq!(
            registry
                .accounts
                .iter()
                .map(|a| &a.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            128
        );
    }
}
