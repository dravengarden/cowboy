//! Cowboy Service-owned Provider authentication vault and replica sealing.
//!
//! Authentication is intentionally not a Machine property. The Service owns
//! each credential lineage's generation and encrypted bundle. Named native
//! login grants are scoped to their enrolled Machine's encryption key while
//! subscription identity remains independent of execution placement. Machines
//! materialize replicas only after the signed authentication contract matches.

#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;
use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use cowboy_provider_sdk::{AuthenticationContract, ProviderPackage, RefreshOwnership};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::machine_auth::{MachineIdentity, PROVIDER_AUTH_SIGNATURE_NAMESPACE};
use crate::machine_protocol::{PortableCredentialBundle, ProviderAuthAction, SealedProviderAuth};
use crate::provider_account::ProviderAccountId;
use crate::provider_account::portable_contract;

const VAULT_SCHEMA: u16 = 1;
const AUTH_ENVELOPE_SCHEMA: u16 = 1;
const MAX_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
const AUTH_SEAL_DOMAIN: &[u8] = b"cowboy-provider-auth-seal-v1\0";
const VAULT_DOMAIN: &[u8] = b"cowboy-provider-auth-vault-v1\0";
static ATOMIC_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub mod grants;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAuthenticationState {
    SignedOut,
    Authenticating,
    Ready,
    Expired,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceDistributionState {
    None,
    Pending,
    Current,
    Partial,
    Failed,
    Revoking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderAuthenticationStatus {
    pub provider_id: String,
    pub account_id: ProviderAccountId,
    /// Public enrollment key, not a credential. None denotes portable state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_key: Option<String>,
    pub auth_generation: u64,
    pub authentication_state: ServiceAuthenticationState,
    pub distribution_state: ServiceDistributionState,
    pub auth_contract_fingerprint: String,
    /// Public credential scope. Providers with the same scope consume the
    /// same portable bundle while retaining independent projections.
    pub authentication_scope: String,
    pub portable_schema: String,
    pub projection_schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAuthRefreshResult {
    Stale { current_generation: u64 },
    Unchanged(Vec<ProviderAuthenticationStatus>),
    Updated(Vec<ProviderAuthenticationStatus>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredProviderAuthentication {
    vault_schema: u16,
    provider_id: String,
    #[serde(default, skip_serializing_if = "ProviderAccountId::is_default")]
    account_id: ProviderAccountId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recipient_key: Option<String>,
    /// Authenticated commitment to one DB promotion intent and exact bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    promotion_commitment: Option<String>,
    auth_generation: u64,
    authentication_state: ServiceAuthenticationState,
    distribution_state: ServiceDistributionState,
    auth_contract_fingerprint: String,
    portable_schema: String,
    projection_schema: String,
    action: ProviderAuthAction,
    nonce: String,
    ciphertext: String,
    account_label: Option<String>,
    updated_at_ms: i64,
}

#[derive(Debug, Clone)]
struct TransientAuthenticationStatus {
    request_id: String,
    status: ProviderAuthenticationStatus,
}

impl StoredProviderAuthentication {
    fn require_legacy_admission(&self) -> Result<()> {
        ensure!(
            self.promotion_commitment.is_none(),
            "Provider grant requires current journal admission"
        );
        Ok(())
    }

    fn status(&self) -> ProviderAuthenticationStatus {
        ProviderAuthenticationStatus {
            provider_id: self.provider_id.clone(),
            account_id: self.account_id.clone(),
            recipient_key: self.recipient_key.clone(),
            auth_generation: self.auth_generation,
            authentication_state: self.authentication_state,
            distribution_state: self.distribution_state,
            auth_contract_fingerprint: self.auth_contract_fingerprint.clone(),
            authentication_scope: self.portable_schema.clone(),
            portable_schema: self.portable_schema.clone(),
            projection_schema: self.projection_schema.clone(),
            account_label: self.account_label.clone(),
            updated_at_ms: self.updated_at_ms,
        }
    }
}

// Closing one descriptor does not release flock while a forked child still
// holds the same open-file description before exec. Release the logical vault
// owner's lock explicitly so immediate reopen does not depend on child timing.
struct VaultOwner(fs::File);

impl Drop for VaultOwner {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

pub struct ProviderAuthService {
    root: PathBuf,
    account_id: ProviderAccountId,
    accounts: parking_lot::Mutex<BTreeMap<ProviderAccountId, Arc<Self>>>,
    recipient_key: Option<String>,
    grants: parking_lot::Mutex<BTreeMap<String, Arc<Self>>>,
    vault_key: [u8; 32],
    signer: MachineIdentity,
    providers: RwLock<BTreeMap<String, StoredProviderAuthentication>>,
    transient: RwLock<BTreeMap<String, TransientAuthenticationStatus>>,
    // Drop scoped owners before releasing the outer vault's ownership.
    _owner: VaultOwner,
}

impl ProviderAuthService {
    pub fn open(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)?;
        // The configured data location may itself be a host-managed symlink.
        // Resolve that trusted anchor once; owned credential scopes below it
        // still reject aliases.
        let root = fs::canonicalize(data_dir)?.join("provider-auth-service");
        Self::open_root(root, ProviderAccountId::default(), None, None)
    }

    fn open_root(
        root: PathBuf,
        account_id: ProviderAccountId,
        service_signer: Option<MachineIdentity>,
        recipient_key: Option<String>,
    ) -> Result<Self> {
        crate::provider_account::prepare_private_directory(&root)?;
        crate::provider_account::prepare_private_directory(&root.join("providers"))?;
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(root.join("owner.lock"))?;
        fs2::FileExt::try_lock_exclusive(&owner)
            .context("Provider authentication vault is already owned by another instance")?;
        let owner = VaultOwner(owner);
        let vault_key = load_or_create_secret(&root.join("vault.key"))?;
        let signer = match service_signer {
            Some(signer) => signer,
            None => MachineIdentity::load_or_create(&root.join("signer"))?,
        };
        let mut providers = BTreeMap::new();
        for entry in fs::read_dir(root.join("providers"))? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let stored: StoredProviderAuthentication = serde_json::from_slice(
                &fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
            )
            .with_context(|| format!("decoding {}", path.display()))?;
            validate_stored(&stored)?;
            ensure!(
                stored.account_id == account_id,
                "Provider vault account mismatch"
            );
            ensure!(
                stored.recipient_key == recipient_key,
                "Provider vault recipient mismatch"
            );
            ensure!(
                providers
                    .insert(stored.provider_id.clone(), stored)
                    .is_none(),
                "duplicate Provider authentication state"
            );
        }
        let service = Self {
            root,
            _owner: owner,
            account_id,
            accounts: parking_lot::Mutex::new(BTreeMap::new()),
            recipient_key,
            grants: parking_lot::Mutex::new(BTreeMap::new()),
            vault_key,
            signer,
            providers: RwLock::new(providers),
            transient: RwLock::new(BTreeMap::new()),
        };
        for stored in service.providers.read().values() {
            validate_vault_plaintext(stored, &service.open_vault(stored)?)?;
        }
        service.reopen_scopes()?;
        Ok(service)
    }

    fn reopen_scopes(&self) -> Result<()> {
        if self.account_id.is_default() && self.root.join("accounts").exists() {
            for entry in fs::read_dir(self.root.join("accounts"))? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let Ok(account) =
                    ProviderAccountId::try_from(entry.file_name().to_string_lossy().into_owned())
                else {
                    continue;
                };
                if self.account(&account).is_err() {
                    // Quarantine this lineage. Selecting it still returns its
                    // error; unrelated accounts must remain available.
                    tracing::warn!(
                        account_id = account.as_str(),
                        "Provider account vault unavailable"
                    );
                }
            }
        }
        if !self.account_id.is_default()
            && self.recipient_key.is_none()
            && self.root.join("machines").exists()
        {
            for entry in fs::read_dir(self.root.join("machines"))? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(entry.file_name().to_string_lossy().as_bytes())
                else {
                    continue;
                };
                if bytes.len() != 32 {
                    continue;
                }
                let recipient = base64::engine::general_purpose::STANDARD.encode(bytes);
                if self.machine_grant(&recipient).is_err() {
                    tracing::warn!(
                        account_id = self.account_id.as_str(),
                        "Provider Machine grant unavailable"
                    );
                }
            }
        }
        Ok(())
    }

    /// Resolve one independently locked credential lineage. Admission and DB
    /// ownership must be checked by the caller before opening a named account.
    pub fn account(&self, account: &ProviderAccountId) -> Result<Arc<Self>> {
        ensure!(
            !account.is_default(),
            "default authentication uses the root vault"
        );
        ensure!(
            self.account_id.is_default(),
            "cannot nest Provider account scopes"
        );
        let mut accounts = self.accounts.lock();
        if let Some(service) = accounts.get(account) {
            return Ok(Arc::clone(service));
        }
        ensure!(accounts.len() < 128, "Provider account capacity exceeded");
        let parent = self.root.join("accounts");
        crate::provider_account::prepare_private_directory(&parent)?;
        let service = Arc::new(Self::open_root(
            parent.join(account.as_str()),
            account.clone(),
            Some(self.signer.clone()),
            None,
        )?);
        accounts.insert(account.clone(), Arc::clone(&service));
        Ok(service)
    }

    /// A subscription can own multiple independent native login grants. Only
    /// the enrolled recipient of this grant can receive its credentials; this
    /// does not constrain the account's execution placement on other Machines.
    /// The caller verifies account ownership and enrollment before opening this
    /// empty scope, then verifies native identity before committing credentials.
    pub fn machine_grant(&self, recipient_key: &str) -> Result<Arc<Self>> {
        ensure!(
            !self.account_id.is_default() && self.recipient_key.is_none(),
            "Machine grants require a named account scope"
        );
        let recipient = decode_fixed::<32>(recipient_key, "Machine encryption public key")?;
        // Reject non-contributory keys before creating persistent grant state.
        let shared = StaticSecret::from([91_u8; 32]).diffie_hellman(&PublicKey::from(recipient));
        ensure!(
            shared.as_bytes().iter().any(|byte| *byte != 0),
            "Machine encryption public key is non-contributory"
        );
        let canonical = base64::engine::general_purpose::STANDARD.encode(recipient);
        let mut grants = self.grants.lock();
        if let Some(grant) = grants.get(&canonical) {
            return Ok(Arc::clone(grant));
        }
        ensure!(
            grants.len() < 256,
            "Provider Machine grant capacity exceeded"
        );
        let parent = self.root.join("machines");
        crate::provider_account::prepare_private_directory(&parent)?;
        let grant = Arc::new(Self::open_root(
            parent.join(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(recipient)),
            self.account_id.clone(),
            Some(self.signer.clone()),
            Some(canonical.clone()),
        )?);
        grants.insert(canonical, Arc::clone(&grant));
        Ok(grant)
    }

    fn vault_schema(&self) -> u16 {
        if self.recipient_key.is_some() {
            3
        } else if self.account_id.is_default() {
            VAULT_SCHEMA
        } else {
            2
        }
    }

    /// Refresh/reconciliation is lookup-only. An untrusted observation must
    /// never create a new account or mint a Machine authorization grant.
    pub fn existing_machine_grant(
        &self,
        account: &ProviderAccountId,
        recipient_key: &str,
    ) -> Result<Arc<Self>> {
        ensure!(
            self.account_id.is_default() && !account.is_default(),
            "named account lookup requires the root vault"
        );
        let recipient = decode_fixed::<32>(recipient_key, "Machine encryption public key")?;
        let canonical = base64::engine::general_purpose::STANDARD.encode(recipient);
        let scoped = self
            .accounts
            .lock()
            .get(account)
            .cloned()
            .context("Provider account is not authorized")?;
        let grant = scoped
            .grants
            .lock()
            .get(&canonical)
            .cloned()
            .context("Provider account is not authorized on this Machine")?;
        Ok(grant)
    }

    #[must_use]
    pub fn statuses(&self) -> Vec<ProviderAuthenticationStatus> {
        let mut statuses: BTreeMap<_, _> = self
            .providers
            .read()
            .values()
            .map(|stored| (stored.provider_id.clone(), stored.status()))
            .collect();
        statuses.extend(
            self.transient
                .read()
                .iter()
                .map(|(provider_id, transient)| (provider_id.clone(), transient.status.clone())),
        );
        statuses.into_values().collect()
    }

    /// Durable generations that can actually be sealed to a Machine. A
    /// transient Service login flow is deliberately excluded.
    #[must_use]
    pub fn replica_statuses(&self) -> Vec<ProviderAuthenticationStatus> {
        self.providers
            .read()
            .values()
            .map(StoredProviderAuthentication::status)
            .collect()
    }

    pub fn machine_account_replica_statuses(
        &self,
        recipient_key: &str,
    ) -> Result<Vec<ProviderAuthenticationStatus>> {
        ensure!(
            self.account_id.is_default(),
            "account inventory requires the root vault"
        );
        let recipient = decode_fixed::<32>(recipient_key, "Machine encryption public key")?;
        let canonical = base64::engine::general_purpose::STANDARD.encode(recipient);
        let accounts: Vec<_> = self.accounts.lock().values().cloned().collect();
        let mut statuses = Vec::new();
        for account in accounts {
            let grant = account.grants.lock().get(&canonical).cloned();
            if let Some(grant) = grant {
                statuses.extend(grant.replica_statuses());
            }
        }
        Ok(statuses)
    }

    #[must_use]
    pub fn status(&self, provider_id: &str) -> Option<ProviderAuthenticationStatus> {
        self.providers
            .read()
            .get(provider_id)
            .map(StoredProviderAuthentication::status)
    }

    #[must_use]
    pub fn active_authentication_request(&self, provider_id: &str) -> Option<String> {
        self.transient
            .read()
            .get(provider_id)
            .filter(|active| {
                active.status.authentication_state == ServiceAuthenticationState::Authenticating
            })
            .map(|active| active.request_id.clone())
    }

    pub fn begin_authentication(
        &self,
        package: &ProviderPackage,
        request_id: &str,
    ) -> Result<ProviderAuthenticationStatus> {
        let provider_id = &package.manifest.id;
        let current = self.providers.read().get(provider_id).cloned();
        let mut transient = self.transient.write();
        ensure!(
            !transient.get(provider_id).is_some_and(|active| {
                active.status.authentication_state == ServiceAuthenticationState::Authenticating
            }),
            "Provider authentication is already in progress"
        );
        let status = ProviderAuthenticationStatus {
            provider_id: provider_id.clone(),
            account_id: self.account_id.clone(),
            recipient_key: self.recipient_key.clone(),
            auth_generation: current.as_ref().map_or(0, |stored| stored.auth_generation),
            authentication_state: ServiceAuthenticationState::Authenticating,
            distribution_state: current
                .as_ref()
                .map_or(ServiceDistributionState::None, |stored| {
                    stored.distribution_state
                }),
            auth_contract_fingerprint: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            authentication_scope: package.manifest.authentication.portable_schema.clone(),
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            projection_schema: package.manifest.authentication.projection_schema.clone(),
            account_label: current.and_then(|stored| stored.account_label),
            updated_at_ms: now_ms(),
        };
        transient.insert(
            provider_id.clone(),
            TransientAuthenticationStatus {
                request_id: request_id.to_owned(),
                status: status.clone(),
            },
        );
        Ok(status)
    }

    pub fn cancel_authentication(&self, provider_id: &str, request_id: &str) {
        let mut transient = self.transient.write();
        if transient
            .get(provider_id)
            .is_some_and(|active| active.request_id == request_id)
        {
            transient.remove(provider_id);
        }
    }

    pub fn fail_authentication(&self, provider_id: &str, request_id: &str) {
        let mut transient = self.transient.write();
        if let Some(active) = transient
            .get_mut(provider_id)
            .filter(|active| active.request_id == request_id)
        {
            active.status.authentication_state = ServiceAuthenticationState::Error;
            active.status.updated_at_ms = now_ms();
        }
    }

    /// Run a synchronous session-registration transaction while the exact
    /// Service authentication generation remains immutable. Credential commit
    /// and logout take the matching write lock, so neither can slip between a
    /// readiness check and durable session registration.
    pub fn with_scheduling_generation<T>(
        &self,
        provider_id: &str,
        authentication_required: bool,
        expected_generation: Option<u64>,
        operation: impl FnOnce() -> T,
    ) -> Result<T> {
        if !authentication_required {
            return Ok(operation());
        }
        let providers = self.providers.read();
        let stored = providers
            .get(provider_id)
            .context("Provider is not authenticated at Cowboy Service scope")?;
        stored.require_legacy_admission()?;
        ensure!(
            stored.authentication_state == ServiceAuthenticationState::Ready,
            "Provider authentication is not ready at Cowboy Service scope"
        );
        ensure!(
            Some(stored.auth_generation) == expected_generation,
            "Provider authentication generation changed; retry session creation"
        );
        Ok(operation())
    }

    /// Like [`Self::with_scheduling_generation`], but the registered session
    /// keeps an older `retained` generation: a native fork must run in its
    /// source's generation home, where the source thread lives. Service token
    /// refreshes advance the generation without a new sign-in, and every
    /// retained runtime home is re-projected with the newest credentials, so
    /// this is as current as resuming the source itself. The readiness check
    /// still holds the exact `current` generation while registering.
    pub fn with_retained_scheduling_generation<T>(
        &self,
        provider_id: &str,
        current: u64,
        retained: u64,
        operation: impl FnOnce() -> T,
    ) -> Result<T> {
        ensure!(
            retained <= current,
            "retained Provider authentication generation is ahead of the Service"
        );
        self.with_scheduling_generation(provider_id, true, Some(current), operation)
    }

    /// Commit a successfully authenticated portable bundle as the next Service
    /// generation. A stale executor cannot overwrite a newer generation when
    /// `expected_generation` is supplied.
    pub fn commit(
        &self,
        package: &ProviderPackage,
        bundle: &PortableCredentialBundle,
        account_label: Option<String>,
        expected_generation: Option<u64>,
    ) -> Result<ProviderAuthenticationStatus> {
        self.commit_shared(
            std::slice::from_ref(package),
            bundle,
            account_label,
            &package.manifest.id,
            expected_generation,
        )?
        .into_iter()
        .next()
        .context("Provider authentication commit produced no status")
    }

    /// Commit one portable credential bundle to every newest Provider package
    /// in a public authentication scope. Each Provider keeps its own signed
    /// contract and encrypted vault entry, while the user's secret is entered
    /// only once at Service scope.
    pub fn commit_shared(
        &self,
        packages: &[ProviderPackage],
        bundle: &PortableCredentialBundle,
        account_label: Option<String>,
        requested_provider_id: &str,
        expected_generation: Option<u64>,
    ) -> Result<Vec<ProviderAuthenticationStatus>> {
        ensure!(
            !packages.is_empty(),
            "authentication scope has no Providers"
        );
        ensure!(
            packages
                .iter()
                .any(|package| package.manifest.id == requested_provider_id),
            "authentication scope does not contain the requested Provider"
        );
        self.require_portable_packages(packages)?;
        for package in packages {
            validate_bundle(&portable_contract::contract(package), bundle)?;
            ensure!(
                self.account_id.is_default()
                    || self.recipient_key.is_some()
                    || package.manifest.authentication.refresh != RefreshOwnership::CompareAndSwap,
                "Named account native refresh credentials require a Machine-bound grant"
            );
        }
        let plaintext = serde_json::to_vec(bundle)?;
        ensure!(
            plaintext.len() <= MAX_BUNDLE_BYTES,
            "credential bundle is too large"
        );
        let mut providers = self.providers.write();
        let current = providers.get(requested_provider_id).cloned();
        if let Some(expected) = expected_generation {
            ensure!(
                current.as_ref().map_or(0, |value| value.auth_generation) == expected,
                "Provider authentication generation changed; restart authentication"
            );
        }
        let current_generation = maximum_auth_generation(packages, &providers);
        if let Some(expected) = expected_generation {
            ensure!(
                current_generation == expected,
                "shared Provider authentication generation changed; restart authentication"
            );
        }
        let generation = current_generation.saturating_add(1);
        ensure!(generation != u64::MAX, "Provider auth generation exhausted");
        let account_label = account_label
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let mut stored_values = Vec::with_capacity(packages.len());
        for package in packages {
            let provider_id = &package.manifest.id;
            let auth = &package.manifest.authentication;
            let fingerprint = portable_contract::fingerprint(package)?;
            let (nonce, ciphertext) =
                self.seal_vault(provider_id, generation, &fingerprint, &plaintext, None)?;
            stored_values.push(StoredProviderAuthentication {
                promotion_commitment: None,
                vault_schema: self.vault_schema(),
                recipient_key: self.recipient_key.clone(),
                provider_id: provider_id.clone(),
                account_id: self.account_id.clone(),
                auth_generation: generation,
                authentication_state: ServiceAuthenticationState::Ready,
                distribution_state: ServiceDistributionState::Pending,
                auth_contract_fingerprint: fingerprint,
                portable_schema: auth.portable_schema.clone(),
                projection_schema: auth.projection_schema.clone(),
                action: ProviderAuthAction::Apply,
                nonce,
                ciphertext,
                account_label: account_label.clone(),
                updated_at_ms: now_ms(),
            });
        }
        let mut statuses = Vec::with_capacity(stored_values.len());
        for stored in stored_values {
            statuses.push(self.replace_locked(&mut providers, stored)?);
        }
        let mut transient = self.transient.write();
        for package in packages {
            transient.remove(&package.manifest.id);
        }
        Ok(statuses)
    }

    /// Extend the current shared credential of one public authentication scope
    /// to scope members that have never held Service authentication, such as a
    /// Provider installed after the user entered the scope's secret.
    ///
    /// Only the default Service vault participates; named accounts stay bound
    /// to their own native identity. The source is the scope's newest
    /// generation and must be a ready credential: a newer logout or failure
    /// tombstone means there is nothing to inherit. A member with any durable
    /// state, including a tombstone, is never overwritten. Adopted entries keep
    /// the source generation, so shared refresh and logout stay one lineage.
    pub fn adopt_shared_scope(
        &self,
        packages: &[ProviderPackage],
    ) -> Result<Vec<ProviderAuthenticationStatus>> {
        if !self.account_id.is_default() || packages.len() < 2 {
            return Ok(Vec::new());
        }
        let mut providers = self.providers.write();
        if packages
            .iter()
            .all(|package| providers.contains_key(&package.manifest.id))
        {
            return Ok(Vec::new());
        }
        let generation = maximum_auth_generation(packages, &providers);
        let Some(source) = packages
            .iter()
            .filter_map(|package| providers.get(&package.manifest.id))
            .find(|stored| stored.auth_generation == generation)
            .filter(|stored| {
                generation > 0
                    && stored.action == ProviderAuthAction::Apply
                    && stored.authentication_state == ServiceAuthenticationState::Ready
                    && stored.promotion_commitment.is_none()
            })
            .cloned()
        else {
            return Ok(Vec::new());
        };
        // Every member at this generation must agree; a partially written
        // scope is not a single credential to extend.
        ensure!(
            packages
                .iter()
                .filter_map(|package| providers.get(&package.manifest.id))
                .filter(|stored| stored.auth_generation == generation)
                .all(|stored| stored.action == ProviderAuthAction::Apply
                    && stored.authentication_state == ServiceAuthenticationState::Ready),
            "shared authentication scope is inconsistent at its newest generation"
        );
        let plaintext = self.open_vault(&source)?;
        let bundle: PortableCredentialBundle =
            serde_json::from_slice(&plaintext).context("decoding shared credential bundle")?;
        let mut stored_values = Vec::new();
        for package in packages
            .iter()
            .filter(|package| !providers.contains_key(&package.manifest.id))
        {
            let provider_id = &package.manifest.id;
            let auth = &package.manifest.authentication;
            if auth.portable_schema != source.portable_schema {
                continue;
            }
            self.require_portable_packages(std::slice::from_ref(package))?;
            if let Err(error) = validate_bundle(&portable_contract::contract(package), &bundle) {
                tracing::warn!(
                    %error,
                    provider_id,
                    source_provider_id = source.provider_id,
                    "shared credential does not satisfy this Provider's contract"
                );
                continue;
            }
            let fingerprint = portable_contract::fingerprint(package)?;
            let (nonce, ciphertext) =
                self.seal_vault(provider_id, generation, &fingerprint, &plaintext, None)?;
            stored_values.push(StoredProviderAuthentication {
                promotion_commitment: None,
                vault_schema: self.vault_schema(),
                recipient_key: self.recipient_key.clone(),
                provider_id: provider_id.clone(),
                account_id: self.account_id.clone(),
                auth_generation: generation,
                authentication_state: ServiceAuthenticationState::Ready,
                distribution_state: ServiceDistributionState::Pending,
                auth_contract_fingerprint: fingerprint,
                portable_schema: auth.portable_schema.clone(),
                projection_schema: auth.projection_schema.clone(),
                action: ProviderAuthAction::Apply,
                nonce,
                ciphertext,
                account_label: source.account_label.clone(),
                updated_at_ms: now_ms(),
            });
        }
        let mut statuses = Vec::with_capacity(stored_values.len());
        for stored in stored_values {
            tracing::info!(
                provider_id = stored.provider_id,
                source_provider_id = source.provider_id,
                auth_generation = generation,
                scope = source.portable_schema,
                "Provider adopted its scope's shared Service credential"
            );
            statuses.push(self.replace_locked(&mut providers, stored)?);
        }
        Ok(statuses)
    }

    /// Atomically promote credentials refreshed inside one Machine projection.
    ///
    /// The Machine must name the exact Service generation it refreshed. Only
    /// one concurrent writer can advance that generation; a loser receives the
    /// current generation and rehydrates from the Service instead of forking a
    /// second durable credential lineage.
    pub fn compare_and_swap_refresh(
        &self,
        packages: &[ProviderPackage],
        bundle: &PortableCredentialBundle,
        requested_provider_id: &str,
        expected_generation: u64,
    ) -> Result<ProviderAuthRefreshResult> {
        self.require_portable_packages(packages)?;
        let (requested_package, plaintext) =
            validate_refresh_request(packages, bundle, requested_provider_id)?;

        let mut providers = self.providers.write();
        let current = providers
            .get(requested_provider_id)
            .cloned()
            .context("Provider is not authenticated at Cowboy Service scope")?;
        let current_generation =
            maximum_auth_generation(packages, &providers).max(current.auth_generation);
        packages
            .iter()
            .filter_map(|package| providers.get(&package.manifest.id))
            .try_for_each(StoredProviderAuthentication::require_legacy_admission)?;
        if current.auth_generation != expected_generation
            || current_generation != expected_generation
        {
            return Ok(ProviderAuthRefreshResult::Stale {
                current_generation: current_generation.max(current.auth_generation),
            });
        }
        ensure!(
            current.action == ProviderAuthAction::Apply
                && matches!(
                    current.authentication_state,
                    ServiceAuthenticationState::Ready | ServiceAuthenticationState::Expired
                ),
            "Provider authentication cannot be refreshed in its current state"
        );
        ensure!(
            current.auth_contract_fingerprint == portable_contract::fingerprint(requested_package)?
                && current.portable_schema
                    == requested_package.manifest.authentication.portable_schema
                && current.projection_schema
                    == requested_package.manifest.authentication.projection_schema,
            "Provider authentication contract changed; sign in again"
        );
        let current_bundle: PortableCredentialBundle =
            serde_json::from_slice(&self.open_vault(&current)?)
                .context("decoding current Provider credential bundle")?;
        validate_bundle(
            &portable_contract::contract(requested_package),
            &current_bundle,
        )?;
        ensure!(
            current_bundle.method_id == bundle.method_id,
            "credential refresh cannot change authentication method"
        );
        // A CLI that only rewrote metadata fields has not refreshed anything;
        // promoting it would redistribute and reload every Machine.
        if current_bundle.materially_equals(bundle) {
            let statuses = packages
                .iter()
                .filter_map(|package| providers.get(&package.manifest.id))
                .map(StoredProviderAuthentication::status)
                .collect();
            return Ok(ProviderAuthRefreshResult::Unchanged(statuses));
        }

        let generation = expected_generation
            .checked_add(1)
            .context("Provider auth generation exhausted")?;
        ensure!(generation != u64::MAX, "Provider auth generation exhausted");
        let mut stored_values = Vec::with_capacity(packages.len());
        for package in packages {
            let provider_id = &package.manifest.id;
            let auth = &package.manifest.authentication;
            let fingerprint = portable_contract::fingerprint(package)?;
            let (nonce, ciphertext) =
                self.seal_vault(provider_id, generation, &fingerprint, &plaintext, None)?;
            stored_values.push(StoredProviderAuthentication {
                promotion_commitment: None,
                vault_schema: self.vault_schema(),
                recipient_key: self.recipient_key.clone(),
                provider_id: provider_id.clone(),
                auth_generation: generation,
                authentication_state: ServiceAuthenticationState::Ready,
                distribution_state: ServiceDistributionState::Pending,
                auth_contract_fingerprint: fingerprint,
                portable_schema: auth.portable_schema.clone(),
                projection_schema: auth.projection_schema.clone(),
                action: ProviderAuthAction::Apply,
                nonce,
                ciphertext,
                account_label: current.account_label.clone(),
                account_id: self.account_id.clone(),
                updated_at_ms: now_ms(),
            });
        }
        let mut statuses = Vec::with_capacity(stored_values.len());
        for stored in stored_values {
            statuses.push(self.replace_locked(&mut providers, stored)?);
        }
        Ok(ProviderAuthRefreshResult::Updated(statuses))
    }

    /// Publish a newer wipe generation. Old encrypted vault generations are
    /// no longer addressable and every connected Machine receives this tombstone.
    pub fn logout(&self, provider_id: &str) -> Result<ProviderAuthenticationStatus> {
        self.logout_shared(&[provider_id.to_owned()])?
            .into_iter()
            .next()
            .context("Provider logout produced no status")
    }

    /// Publish one wipe generation for every Provider in a shared
    /// authentication scope.
    pub fn logout_shared(
        &self,
        provider_ids: &[String],
    ) -> Result<Vec<ProviderAuthenticationStatus>> {
        ensure!(
            !provider_ids.is_empty(),
            "authentication scope has no Providers"
        );
        let mut providers = self.providers.write();
        self.logout_locked(&mut providers, provider_ids)
    }

    /// Log out every scope member that holds durable state, choosing them
    /// under the same lock as the wipe. Selecting members before locking
    /// could miss one that adopted the shared credential in between and
    /// leave it authenticated after the user logged out.
    pub fn logout_scope(&self, candidates: &[String]) -> Result<Vec<ProviderAuthenticationStatus>> {
        let mut providers = self.providers.write();
        let present: Vec<String> = candidates
            .iter()
            .filter(|provider_id| providers.contains_key(*provider_id))
            .cloned()
            .collect();
        ensure!(
            !present.is_empty(),
            "authentication scope has no durable Provider state"
        );
        self.logout_locked(&mut providers, &present)
    }

    fn logout_locked(
        &self,
        providers: &mut BTreeMap<String, StoredProviderAuthentication>,
        provider_ids: &[String],
    ) -> Result<Vec<ProviderAuthenticationStatus>> {
        let current: Vec<_> = provider_ids
            .iter()
            .map(|provider_id| {
                providers.get(provider_id).cloned().with_context(|| {
                    format!("Provider {provider_id:?} is not authenticated at Cowboy Service scope")
                })
            })
            .collect::<Result<_>>()?;
        let generation = current
            .iter()
            .map(|stored| stored.auth_generation)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        ensure!(generation != u64::MAX, "Provider auth generation exhausted");
        let mut statuses = Vec::with_capacity(current.len());
        for current in current {
            let (nonce, ciphertext) = self.seal_vault(
                &current.provider_id,
                generation,
                &current.auth_contract_fingerprint,
                &[],
                None,
            )?;
            statuses.push(self.replace_locked(
                providers,
                StoredProviderAuthentication {
                    promotion_commitment: None,
                    vault_schema: self.vault_schema(),
                    recipient_key: self.recipient_key.clone(),
                    provider_id: current.provider_id,
                    account_id: self.account_id.clone(),
                    auth_generation: generation,
                    authentication_state: ServiceAuthenticationState::SignedOut,
                    distribution_state: ServiceDistributionState::Revoking,
                    auth_contract_fingerprint: current.auth_contract_fingerprint,
                    portable_schema: current.portable_schema,
                    projection_schema: current.projection_schema,
                    action: ProviderAuthAction::Wipe,
                    nonce,
                    ciphertext,
                    account_label: None,
                    updated_at_ms: now_ms(),
                },
            )?);
        }
        let mut transient = self.transient.write();
        for provider_id in provider_ids {
            transient.remove(provider_id);
        }
        Ok(statuses)
    }

    /// Encrypt the current Service generation to one enrolled Machine and sign
    /// every field that can affect routing or credential materialization.
    pub fn seal_for_machine(
        &self,
        provider_id: &str,
        machine_public_key: &str,
    ) -> Result<SealedProviderAuth> {
        self.seal_for_machine_with_admission(provider_id, machine_public_key, None)
    }

    fn seal_for_machine_with_admission(
        &self,
        provider_id: &str,
        machine_public_key: &str,
        admitted: Option<(&str, u64)>,
    ) -> Result<SealedProviderAuth> {
        let recipient = decode_fixed::<32>(machine_public_key, "Machine encryption public key")?;
        ensure!(
            self.recipient_key.as_ref().is_none_or(
                |key| key == &base64::engine::general_purpose::STANDARD.encode(recipient)
            ),
            "Provider grant belongs to another Machine"
        );
        let stored = self
            .providers
            .read()
            .get(provider_id)
            .cloned()
            .context("Provider is not authenticated at Cowboy Service scope")?;
        let plaintext = if stored.action == ProviderAuthAction::Apply {
            crate::provider_account::portable_credentials_allowed(&self.account_id, provider_id)
                .map_err(anyhow::Error::msg)?;
            if let Some((commitment, generation)) = admitted {
                ensure!(
                    stored.promotion_commitment.as_deref() == Some(commitment)
                        && stored.auth_generation >= generation
                        && stored.authentication_state == ServiceAuthenticationState::Ready,
                    "Provider grant changed after journal admission"
                );
            } else {
                stored.require_legacy_admission()?;
            }
            self.open_vault(&stored)?
        } else {
            ensure!(admitted.is_none(), "Provider grant was revoked");
            Vec::new()
        };

        let ephemeral = StaticSecret::from(random_bytes::<32>()?);
        let ephemeral_public = PublicKey::from(&ephemeral);
        let shared = ephemeral.diffie_hellman(&PublicKey::from(recipient));
        ensure!(
            shared.as_bytes().iter().any(|byte| *byte != 0),
            "Machine encryption public key is non-contributory"
        );
        let key = derive_seal_key(shared.as_bytes());
        let nonce = random_bytes::<24>()?;
        let mut envelope = SealedProviderAuth {
            envelope_schema: if self.account_id.is_default() {
                AUTH_ENVELOPE_SCHEMA
            } else {
                2
            },
            provider_id: stored.provider_id,
            account_id: self.account_id.clone(),
            auth_generation: stored.auth_generation,
            auth_contract_fingerprint: stored.auth_contract_fingerprint,
            projection_schema: stored.projection_schema,
            action: stored.action,
            ephemeral_public_key: base64::engine::general_purpose::STANDARD
                .encode(ephemeral_public.as_bytes()),
            nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
            ciphertext: String::new(),
            service_public_key: self.signer.public_key().to_owned(),
            signature: String::new(),
        };
        let ciphertext = XChaCha20Poly1305::new(Key::from_slice(&key))
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &provider_auth_aad(&envelope),
                },
            )
            .map_err(|_| anyhow::anyhow!("sealing Provider credentials failed"))?;
        envelope.ciphertext = base64::engine::general_purpose::STANDARD.encode(ciphertext);
        envelope.signature = self
            .signer
            .sign_namespaced(PROVIDER_AUTH_SIGNATURE_NAMESPACE, &envelope.proof())?;
        Ok(envelope)
    }

    pub fn mark_distribution(
        &self,
        provider_id: &str,
        generation: u64,
        state: ServiceDistributionState,
    ) -> Result<ProviderAuthenticationStatus> {
        let mut providers = self.providers.write();
        let mut stored = providers
            .get(provider_id)
            .cloned()
            .context("Provider auth state does not exist")?;
        ensure!(
            stored.auth_generation == generation,
            "stale Provider distribution receipt"
        );
        stored.distribution_state = state;
        stored.updated_at_ms = now_ms();
        self.replace_locked(&mut providers, stored)
    }

    /// Mark the exact Service credential generation rejected by a live Agent.
    /// A stale worker cannot expire a newer login, and repeated reports for the
    /// same generation are idempotent.
    pub fn mark_expired(
        &self,
        provider_id: &str,
        generation: u64,
    ) -> Result<Option<ProviderAuthenticationStatus>> {
        let mut providers = self.providers.write();
        let Some(mut stored) = providers.get(provider_id).cloned() else {
            return Ok(None);
        };
        if stored.auth_generation != generation {
            return Ok(None);
        }
        if stored.authentication_state == ServiceAuthenticationState::Expired {
            return Ok(Some(stored.status()));
        }
        if stored.authentication_state != ServiceAuthenticationState::Ready
            || stored.action != ProviderAuthAction::Apply
        {
            return Ok(None);
        }
        stored.authentication_state = ServiceAuthenticationState::Expired;
        stored.updated_at_ms = now_ms();
        self.replace_locked(&mut providers, stored).map(Some)
    }

    fn replace_locked(
        &self,
        providers: &mut BTreeMap<String, StoredProviderAuthentication>,
        stored: StoredProviderAuthentication,
    ) -> Result<ProviderAuthenticationStatus> {
        validate_stored(&stored)?;
        let path = self
            .root
            .join("providers")
            .join(format!("{}.json", stored.provider_id));
        atomic_write(&path, &serde_json::to_vec(&stored)?, 0o600)?;
        let status = stored.status();
        providers.insert(stored.provider_id.clone(), stored);
        Ok(status)
    }

    fn require_portable_packages(&self, packages: &[ProviderPackage]) -> Result<()> {
        for package in packages {
            crate::provider_account::portable_credentials_allowed(
                &self.account_id,
                &package.manifest.id,
            )
            .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }

    fn seal_vault(
        &self,
        provider_id: &str,
        generation: u64,
        fingerprint: &str,
        plaintext: &[u8],
        promotion_commitment: Option<&str>,
    ) -> Result<(String, String)> {
        // Wipe encrypts no credential bytes and must remain available for
        // retiring historical portable state even when new imports are denied.
        if !plaintext.is_empty() {
            crate::provider_account::portable_credentials_allowed(&self.account_id, provider_id)
                .map_err(anyhow::Error::msg)?;
        }
        let nonce = random_bytes::<24>()?;
        let ciphertext = XChaCha20Poly1305::new(Key::from_slice(&self.vault_key))
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &vault_aad(
                        provider_id,
                        generation,
                        fingerprint,
                        &self.account_id,
                        self.recipient_key.as_deref(),
                        promotion_commitment,
                    ),
                },
            )
            .map_err(|_| anyhow::anyhow!("encrypting Provider auth vault failed"))?;
        Ok((
            base64::engine::general_purpose::STANDARD.encode(nonce),
            base64::engine::general_purpose::STANDARD.encode(ciphertext),
        ))
    }

    fn open_vault(&self, stored: &StoredProviderAuthentication) -> Result<Vec<u8>> {
        crate::provider_account::portable_credentials_allowed(
            &self.account_id,
            &stored.provider_id,
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(
            stored.account_id == self.account_id,
            "Provider vault account mismatch"
        );
        ensure!(
            stored.recipient_key == self.recipient_key,
            "Provider vault recipient mismatch"
        );
        let nonce = decode_fixed::<24>(&stored.nonce, "vault nonce")?;
        let ciphertext = base64::engine::general_purpose::STANDARD
            .decode(&stored.ciphertext)
            .context("decoding Provider auth vault ciphertext")?;
        ensure!(
            ciphertext.len() <= MAX_BUNDLE_BYTES + 64,
            "vault entry is too large"
        );
        XChaCha20Poly1305::new(Key::from_slice(&self.vault_key))
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &vault_aad(
                        &stored.provider_id,
                        stored.auth_generation,
                        &stored.auth_contract_fingerprint,
                        &stored.account_id,
                        stored.recipient_key.as_deref(),
                        stored.promotion_commitment.as_deref(),
                    ),
                },
            )
            .map_err(|_| anyhow::anyhow!("Provider auth vault authentication failed"))
    }
}

fn maximum_auth_generation(
    packages: &[ProviderPackage],
    providers: &BTreeMap<String, StoredProviderAuthentication>,
) -> u64 {
    packages
        .iter()
        .filter_map(|package| providers.get(&package.manifest.id))
        .map(|stored| stored.auth_generation)
        .max()
        .unwrap_or(0)
}

fn validate_stored(stored: &StoredProviderAuthentication) -> Result<()> {
    ensure!(
        stored
            .promotion_commitment
            .as_ref()
            .is_none_or(|value| stored.vault_schema == 3
                && stored.action == ProviderAuthAction::Apply
                && value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))),
        "invalid Provider promotion commitment"
    );
    ensure!(
        (stored.vault_schema == VAULT_SCHEMA
            && stored.account_id.is_default()
            && stored.recipient_key.is_none())
            || (stored.vault_schema == 2
                && !stored.account_id.is_default()
                && stored.recipient_key.is_none())
            || (stored.vault_schema == 3
                && !stored.account_id.is_default()
                && stored.recipient_key.is_some()),
        "unsupported Provider vault schema"
    );
    ensure!(
        !stored.provider_id.is_empty()
            && stored.provider_id.len() <= 64
            && stored
                .provider_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
        "invalid Provider id in vault"
    );
    ensure!(
        stored.auth_generation > 0,
        "invalid Provider auth generation"
    );
    ensure!(
        (matches!(
            stored.authentication_state,
            ServiceAuthenticationState::Ready | ServiceAuthenticationState::Expired
        ) && stored.action == ProviderAuthAction::Apply)
            || (stored.authentication_state == ServiceAuthenticationState::SignedOut
                && stored.action == ProviderAuthAction::Wipe),
        "inconsistent Provider authentication state and action"
    );
    ensure!(
        stored.action == ProviderAuthAction::Apply || stored.account_label.is_none(),
        "a signed-out Provider cannot retain an account label"
    );
    ensure!(
        stored
            .auth_contract_fingerprint
            .strip_prefix("sha256:")
            .is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            }),
        "invalid Provider auth contract fingerprint"
    );
    for (label, value) in [
        ("portable schema", stored.portable_schema.as_str()),
        ("projection schema", stored.projection_schema.as_str()),
    ] {
        ensure!(
            !value.is_empty()
                && value.len() <= 128
                && value.bytes().all(|byte| byte.is_ascii_graphic()),
            "invalid Provider {label}"
        );
    }
    ensure!(stored.updated_at_ms > 0, "invalid Provider auth timestamp");
    decode_fixed::<24>(&stored.nonce, "vault nonce")?;
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&stored.ciphertext)
        .context("decoding vault ciphertext")?;
    ensure!(
        (16..=MAX_BUNDLE_BYTES + 64).contains(&ciphertext.len()),
        "vault entry has invalid size"
    );
    Ok(())
}

fn validate_vault_plaintext(stored: &StoredProviderAuthentication, plaintext: &[u8]) -> Result<()> {
    match stored.action {
        ProviderAuthAction::Wipe => ensure!(
            plaintext.is_empty(),
            "Provider logout tombstone contains credential material"
        ),
        ProviderAuthAction::Apply => {
            let bundle: PortableCredentialBundle =
                serde_json::from_slice(plaintext).context("decoding Provider credential bundle")?;
            ensure!(
                bundle.portable_schema == stored.portable_schema,
                "stored Provider portable schema mismatch"
            );
            ensure!(
                !bundle.method_id.is_empty() && bundle.method_id.len() <= 64,
                "invalid stored Provider authentication method"
            );
            for (key, value) in bundle.values {
                ensure!(
                    !key.is_empty() && key.len() <= 128,
                    "invalid stored Provider credential key"
                );
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .context("stored Provider credential value is not base64")?;
                ensure!(
                    decoded.len() <= MAX_BUNDLE_BYTES,
                    "stored Provider credential value is too large"
                );
            }
        }
    }
    Ok(())
}

fn validate_refresh_request<'a>(
    packages: &'a [ProviderPackage],
    bundle: &PortableCredentialBundle,
    requested_provider_id: &str,
) -> Result<(&'a ProviderPackage, Vec<u8>)> {
    ensure!(
        !packages.is_empty(),
        "authentication scope has no Providers"
    );
    let requested_package = packages
        .iter()
        .find(|package| package.manifest.id == requested_provider_id)
        .context("authentication scope does not contain the requested Provider")?;
    for package in packages {
        ensure!(
            package.manifest.authentication.refresh == RefreshOwnership::CompareAndSwap,
            "Provider does not allow Machine credential refresh"
        );
        validate_bundle(&portable_contract::contract(package), bundle)?;
    }
    let plaintext = serde_json::to_vec(bundle)?;
    ensure!(
        plaintext.len() <= MAX_BUNDLE_BYTES,
        "credential bundle is too large"
    );
    Ok((requested_package, plaintext))
}

fn validate_bundle(auth: &AuthenticationContract, bundle: &PortableCredentialBundle) -> Result<()> {
    ensure!(
        bundle.portable_schema == auth.portable_schema,
        "portable schema mismatch"
    );
    let method = auth
        .methods
        .iter()
        .find(|method| method.id == bundle.method_id)
        .context("portable bundle references an unknown authentication method")?;
    let allowed: BTreeSet<_> = auth
        .credential_files
        .iter()
        .map(|file| file.bundle_key.as_str())
        .chain(auth.environment_projection.values().map(String::as_str))
        .collect();
    ensure!(
        bundle
            .values
            .keys()
            .all(|key| allowed.contains(key.as_str())),
        "portable credential bundle contains undeclared values"
    );
    for file in &auth.credential_files {
        ensure!(
            !file.required || bundle.values.contains_key(&file.bundle_key),
            "portable credential bundle is missing {}",
            file.bundle_key
        );
    }
    ensure!(
        method
            .required_bundle_keys
            .iter()
            .all(|key| bundle.values.contains_key(key)),
        "portable credential bundle is missing a method-required value"
    );
    for value in bundle.values.values() {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(value)
            .context("portable credential value is not base64")?;
        ensure!(
            decoded.len() <= MAX_BUNDLE_BYTES,
            "credential value is too large"
        );
    }
    Ok(())
}

fn load_or_create_secret(path: &Path) -> Result<[u8; 32]> {
    let bytes = if path.is_file() {
        fs::read(path).with_context(|| format!("reading {}", path.display()))?
    } else {
        let value = random_bytes::<32>()?;
        atomic_write(path, &value, 0o600)?;
        value.to_vec()
    };
    ensure!(bytes.len() == 32, "Provider vault key has invalid length");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    let mut key = [0_u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut output = [0_u8; N];
    fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut output)
        .context("reading OS randomness")?;
    Ok(output)
}

fn decode_fixed<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .with_context(|| format!("decoding {label}"))?;
    ensure!(bytes.len() == N, "{label} has invalid length");
    let mut output = [0_u8; N];
    output.copy_from_slice(&bytes);
    Ok(output)
}

fn derive_seal_key(shared: &[u8; 32]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(AUTH_SEAL_DOMAIN);
    digest.update(shared);
    digest.finalize().into()
}

fn provider_auth_aad(envelope: &SealedProviderAuth) -> Vec<u8> {
    envelope.authentication_aad()
}

fn vault_aad(
    provider_id: &str,
    generation: u64,
    fingerprint: &str,
    account: &ProviderAccountId,
    recipient: Option<&str>,
    promotion_commitment: Option<&str>,
) -> Vec<u8> {
    let mut output = VAULT_DOMAIN.to_vec();
    output.extend_from_slice(provider_id.as_bytes());
    output.push(b'\n');
    output.extend_from_slice(generation.to_string().as_bytes());
    output.push(b'\n');
    output.extend_from_slice(fingerprint.as_bytes());
    output.push(b'\n');
    if !account.is_default() {
        output.extend_from_slice(b"account:");
        output.extend_from_slice(account.as_str().as_bytes());
        output.push(b'\n');
    }
    if let Some(recipient) = recipient {
        output.extend_from_slice(b"recipient:");
        output.extend_from_slice(recipient.as_bytes());
        output.push(b'\n');
    }
    if let Some(commitment) = promotion_commitment {
        output.extend_from_slice(b"promotion:");
        output.extend_from_slice(commitment.as_bytes());
        output.push(b'\n');
    }
    output
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().context("atomic write path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.{}.partial",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("value"),
        std::process::id(),
        ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temporary)
        .with_context(|| format!("creating {}", temporary.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path).with_context(|| format!("activating {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    // A commit receipt must survive loss of power, not just process restart.
    // sync_all on the temporary file alone does not persist the directory entry.
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    mod grant_commits;
    use super::*;

    #[test]
    fn named_claude_credentials_cannot_enter_the_portable_vault() {
        let (root, service) = service();
        let account = service
            .account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let error = account
            .seal_vault("claude-code", 1, "contract", b"synthetic-secret", None)
            .unwrap_err();
        assert_eq!(error.to_string(), "native_owned_credentials_required");
        assert!(!account.root.join("providers/claude-code.json").exists());
        assert!(
            account
                .seal_vault("claude-deepseek", 1, "contract", b"synthetic-api-key", None)
                .is_ok()
        );
        assert!(
            service
                .seal_vault("claude-code", 1, "contract", b"synthetic-legacy", None)
                .is_ok()
        );
        let package = package("claude-deepseek");
        account
            .commit(&package, &bundle(&package), None, None)
            .unwrap();
        // Simulate retained metadata from the prototype, without creating or
        // decrypting any Claude credential. Cleanup must work from metadata.
        let mut retained = account
            .providers
            .read()
            .get("claude-deepseek")
            .unwrap()
            .clone();
        retained.provider_id = "claude-code".into();
        assert!(account.open_vault(&retained).is_err());
        account
            .providers
            .write()
            .insert("claude-code".into(), retained);
        account.logout("claude-code").unwrap();
        let key = PublicKey::from(&StaticSecret::from([42; 32]));
        let envelope = account
            .seal_for_machine(
                "claude-code",
                &base64::engine::general_purpose::STANDARD.encode(key.as_bytes()),
            )
            .unwrap();
        assert_eq!(envelope.action, ProviderAuthAction::Wipe);
        drop(account);
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn account_and_machine_vaults_reject_directory_aliases() {
        use std::os::unix::fs::symlink;

        let (root, service) = service();
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let parent = service.root.join("accounts");
        fs::create_dir(&parent).unwrap();
        let target = root.join("unrelated");
        fs::create_dir(&target).unwrap();
        symlink(&target, parent.join(id.as_str())).unwrap();
        assert!(service.account(&id).is_err());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        fs::remove_file(parent.join(id.as_str())).unwrap();
        let account = service.account(&id).unwrap();
        let recipient = PublicKey::from(&StaticSecret::from([42; 32]));
        let key = base64::engine::general_purpose::STANDARD.encode(recipient.as_bytes());
        let grants = account.root.join("machines");
        fs::create_dir(&grants).unwrap();
        let name = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(recipient.as_bytes());
        symlink(&target, grants.join(&name)).unwrap();
        assert!(account.machine_grant(&key).is_err());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        fs::remove_file(grants.join(&name)).unwrap();
        let grant = account.machine_grant(&key).unwrap();
        assert_eq!(grant.recipient_key.as_deref(), Some(key.as_str()));
    }

    #[test]
    fn configured_data_location_may_be_a_symlink_but_vault_root_may_not() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        fs::create_dir(&data).unwrap();
        let alias = root.path().join("configured-data");
        symlink(&data, &alias).unwrap();
        let service = ProviderAuthService::open(&alias).unwrap();
        assert_eq!(
            service.root,
            fs::canonicalize(&data)
                .unwrap()
                .join("provider-auth-service")
        );
        drop(service);
        let relocated = data.join("relocated");
        fs::rename(data.join("provider-auth-service"), &relocated).unwrap();
        symlink(&relocated, data.join("provider-auth-service")).unwrap();
        assert!(ProviderAuthService::open(&alias).is_err());
    }

    #[test]
    fn refresh_grant_lookup_never_creates_or_crosses_authorization() {
        let (root, service) = service();
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let other: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let key = |byte| {
            base64::engine::general_purpose::STANDARD
                .encode(PublicKey::from(&StaticSecret::from([byte; 32])).as_bytes())
        };
        let first_key = key(7);
        let second_key = key(8);
        assert!(service.existing_machine_grant(&id, &first_key).is_err());
        assert!(service.accounts.lock().is_empty());
        let account = service.account(&id).unwrap();
        assert!(service.existing_machine_grant(&id, &first_key).is_err());
        assert!(account.grants.lock().is_empty());
        let grant = account.machine_grant(&first_key).unwrap();
        assert!(Arc::ptr_eq(
            &grant,
            &service.existing_machine_grant(&id, &first_key).unwrap()
        ));
        assert!(service.existing_machine_grant(&id, &second_key).is_err());
        assert!(service.existing_machine_grant(&other, &first_key).is_err());
        assert_eq!(account.grants.lock().len(), 1);
        assert_eq!(service.accounts.lock().len(), 1);
        assert!(!service.root.join("accounts").join(other.as_str()).exists());
        drop(grant);
        drop(account);
        drop(service);
        let service = ProviderAuthService::open(&root).unwrap();
        assert!(service.existing_machine_grant(&id, &first_key).is_ok());
        assert!(service.existing_machine_grant(&id, &second_key).is_err());
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_named_vault_does_not_disable_default_authentication() {
        let (root, service) = service();
        let package = package("claude-deepseek");
        service
            .commit(&package, &bundle(&package), None, Some(0))
            .unwrap();
        let id =
            ProviderAccountId::try_from("11111111-1111-4111-8111-111111111111".to_owned()).unwrap();
        let account = service.account(&id).unwrap();
        let broken = account.root.join("providers/claude-deepseek.json");
        account
            .commit(&package, &bundle(&package), None, Some(0))
            .unwrap();
        drop(account);
        drop(service);
        fs::write(broken, b"invalid vault fixture").unwrap();
        let service = ProviderAuthService::open(&root).unwrap();
        assert!(service.account(&id).is_err());
        assert_eq!(
            service
                .status("claude-deepseek")
                .unwrap()
                .authentication_state,
            ServiceAuthenticationState::Ready
        );
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One grant fixture covers refresh, revoke, recipient isolation and restart.
    fn one_subscription_has_independent_machine_grants() {
        let (root, service) = service();
        let id =
            ProviderAccountId::try_from("11111111-1111-4111-8111-111111111111".to_owned()).unwrap();
        let account = service.account(&id).unwrap();
        let key = |byte| {
            base64::engine::general_purpose::STANDARD
                .encode(PublicKey::from(&StaticSecret::from([byte; 32])).as_bytes())
        };
        let first_key = key(7);
        let second_key = key(8);
        let first = account.machine_grant(&first_key).unwrap();
        let second = account.machine_grant(&second_key).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &account.machine_grant(&first_key).unwrap()
        ));
        assert!(first.machine_grant(&second_key).is_err());
        assert!(service.machine_grant(&first_key).is_err());
        let package = package("grok");
        let bundle = bundle(&package);
        // A named native login must never enter a portable, multi-writer vault.
        assert!(account.commit(&package, &bundle, None, Some(0)).is_err());
        first.commit(&package, &bundle, None, Some(0)).unwrap();
        let mut second_bundle = bundle.clone();
        second_bundle.values.insert(
            "auth_json".into(),
            base64::engine::general_purpose::STANDARD.encode(br#"{"token":"second-native-login"}"#),
        );
        second
            .commit(&package, &second_bundle, None, Some(0))
            .unwrap();
        let stored = first.providers.read()["grok"].clone();
        assert_eq!(stored.vault_schema, 3);
        assert!(second.open_vault(&stored).is_err());
        let mut swapped = stored;
        swapped.recipient_key = Some(second_key.clone());
        assert!(second.open_vault(&swapped).is_err());
        assert!(first.seal_for_machine("grok", &first_key).is_ok());
        assert!(second.seal_for_machine("grok", &second_key).is_ok());
        assert!(first.seal_for_machine("grok", &second_key).is_err());
        assert!(second.seal_for_machine("grok", &first_key).is_err());
        assert!(account.status("grok").is_none());
        let mut refreshed = bundle;
        refreshed.values.insert(
            "auth_json".into(),
            base64::engine::general_purpose::STANDARD.encode(br#"{"token":"first-refreshed"}"#),
        );
        assert!(matches!(
            first
                .compare_and_swap_refresh(std::slice::from_ref(&package), &refreshed, "grok", 1)
                .unwrap(),
            ProviderAuthRefreshResult::Updated(_)
        ));
        let second_stored = second.providers.read()["grok"].clone();
        let first_inventory = service
            .machine_account_replica_statuses(&first_key)
            .unwrap();
        let second_inventory = service
            .machine_account_replica_statuses(&second_key)
            .unwrap();
        assert_eq!(first_inventory.len(), 1);
        assert_eq!(first_inventory[0].auth_generation, 2);
        assert_eq!(first_inventory[0].account_id, id);
        assert_eq!(second_inventory.len(), 1);
        assert_eq!(second_inventory[0].auth_generation, 1);
        assert!(
            service
                .machine_account_replica_statuses(&key(9))
                .unwrap()
                .is_empty()
        );
        let unchanged: PortableCredentialBundle =
            serde_json::from_slice(&second.open_vault(&second_stored).unwrap()).unwrap();
        assert_eq!(unchanged, second_bundle);
        first.logout("grok").unwrap();
        assert_eq!(first.status("grok").unwrap().auth_generation, 3);
        assert_eq!(second.status("grok").unwrap().auth_generation, 1);
        assert_eq!(
            second.status("grok").unwrap().authentication_state,
            ServiceAuthenticationState::Ready
        );
        drop(first);
        drop(second);
        drop(account);
        drop(service);
        let reopened = ProviderAuthService::open(&root).unwrap();
        let account = reopened.account(&id).unwrap();
        assert_eq!(
            account
                .machine_grant(&first_key)
                .unwrap()
                .status("grok")
                .unwrap()
                .authentication_state,
            ServiceAuthenticationState::SignedOut
        );
        assert_eq!(
            account
                .machine_grant(&second_key)
                .unwrap()
                .status("grok")
                .unwrap()
                .authentication_state,
            ServiceAuthenticationState::Ready
        );
        assert!(
            account
                .machine_grant(&first_key)
                .unwrap()
                .seal_for_machine("grok", &second_key)
                .is_err()
        );
        drop(account);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[allow(clippy::used_underscore_binding)] // Inspect the RAII-only descriptor to model fork/dup.
    fn vault_owner_releases_lock_even_when_a_child_retains_the_descriptor() {
        let (root, service) = service();
        // dup, like fork before exec, retains the same open-file description.
        // This makes the reopen race deterministic without spawning an agent
        // or depending on process scheduler timing.
        let inherited = service._owner.0.try_clone().unwrap();
        assert!(ProviderAuthService::open(&root).is_err());
        drop(service);
        let reopened = ProviderAuthService::open(&root).unwrap();
        assert!(ProviderAuthService::open(&root).is_err());
        drop(inherited);
        assert!(ProviderAuthService::open(&root).is_err());
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn account_vaults_have_independent_credentials_generations_and_owners() {
        let (root, service) = service();
        let service = Arc::new(service);
        let first_id =
            ProviderAccountId::try_from("11111111-1111-4111-8111-111111111111".to_owned()).unwrap();
        let second_id =
            ProviderAccountId::try_from("22222222-2222-4222-8222-222222222222".to_owned()).unwrap();
        let first = service.account(&first_id).unwrap();
        let second = service.account(&second_id).unwrap();
        assert_eq!(first.signer.public_key(), service.signer.public_key());
        assert_eq!(second.signer.public_key(), service.signer.public_key());
        assert!(Arc::ptr_eq(&first, &service.account(&first_id).unwrap()));
        assert!(service.account(&ProviderAccountId::default()).is_err());
        assert!(ProviderAuthService::open(&root).is_err());
        let package = package("claude-deepseek");
        let first_bundle = bundle(&package);
        let mut second_bundle = first_bundle.clone();
        second_bundle.values.insert(
            "api_key".into(),
            base64::engine::general_purpose::STANDARD.encode(b"second-secret"),
        );
        first
            .commit(&package, &first_bundle, Some("Personal".into()), Some(0))
            .unwrap();
        second
            .commit(&package, &second_bundle, Some("Personal".into()), Some(0))
            .unwrap();
        assert!(service.status("claude-deepseek").is_none());
        let first_stored = first.providers.read()["claude-deepseek"].clone();
        assert!(second.open_vault(&first_stored).is_err());
        let mut relabelled = first_stored.clone();
        relabelled.account_id = second_id.clone();
        assert!(second.open_vault(&relabelled).is_err());
        first.logout("claude-deepseek").unwrap();
        assert_eq!(first.status("claude-deepseek").unwrap().auth_generation, 2);
        let second_status = second.status("claude-deepseek").unwrap();
        assert_eq!(second_status.auth_generation, 1);
        assert_eq!(second_status.account_id, second_id);
        assert_eq!(
            second_status.authentication_state,
            ServiceAuthenticationState::Ready
        );
        assert!(
            second
                .with_scheduling_generation("claude-deepseek", true, Some(1), || ())
                .is_ok()
        );
        drop(first);
        drop(second);
        drop(service);
        let reopened = Arc::new(ProviderAuthService::open(&root).unwrap());
        assert_eq!(
            reopened
                .account(&first_id)
                .unwrap()
                .status("claude-deepseek")
                .unwrap()
                .authentication_state,
            ServiceAuthenticationState::SignedOut
        );
        assert_eq!(
            reopened
                .account(&second_id)
                .unwrap()
                .status("claude-deepseek")
                .unwrap()
                .auth_generation,
            1
        );
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    fn package(id: &str) -> ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource = serde_json::from_str(match id {
            "grok" => include_str!("../plugins/grok/provider.json"),
            "claude-deepseek" => include_str!("../plugins/claude-deepseek/provider.json"),
            "codex-deepseek" => include_str!("../plugins/codex-deepseek/provider.json"),
            _ => panic!("unsupported fixture"),
        })
        .unwrap();
        cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
    }

    fn service() -> (PathBuf, ProviderAuthService) {
        let root = std::env::temp_dir().join(format!(
            "cowboy-provider-service-test-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let service = ProviderAuthService::open(&root).unwrap();
        (root, service)
    }

    fn bundle(package: &ProviderPackage) -> PortableCredentialBundle {
        let values = if package.manifest.authentication.portable_schema == "deepseek-api-key-v1" {
            [("api_key", b"deepseek-secret".as_slice())]
                .into_iter()
                .map(|(key, value)| {
                    (
                        key.to_owned(),
                        base64::engine::general_purpose::STANDARD.encode(value),
                    )
                })
                .collect()
        } else {
            ["auth_json"]
                .map(|key| {
                    (
                        key.to_owned(),
                        base64::engine::general_purpose::STANDARD.encode(b"{}"),
                    )
                })
                .into_iter()
                .collect()
        };
        PortableCredentialBundle {
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            method_id:
                if package.manifest.authentication.portable_schema == "deepseek-api-key-v1" {
                    "api-key"
                } else {
                    "xai-account"
                }
                .to_owned(),
            values,
        }
    }

    #[test]
    fn transient_login_state_is_service_wide_but_not_replicated() {
        let (root, service) = service();
        let package = package("grok");
        let authenticating = service
            .begin_authentication(&package, "first-login")
            .unwrap();
        assert_eq!(authenticating.auth_generation, 0);
        assert_eq!(
            authenticating.authentication_state,
            ServiceAuthenticationState::Authenticating
        );
        assert!(service.status("grok").is_none());
        assert!(service.replica_statuses().is_empty());
        assert_eq!(service.statuses(), vec![authenticating]);
        assert_eq!(
            service.active_authentication_request("grok").as_deref(),
            Some("first-login")
        );
        assert!(
            service
                .begin_authentication(&package, "duplicate-login")
                .unwrap_err()
                .to_string()
                .contains("already in progress")
        );
        service.fail_authentication("grok", "first-login");
        assert_eq!(
            service.statuses()[0].authentication_state,
            ServiceAuthenticationState::Error
        );
        assert!(service.active_authentication_request("grok").is_none());
        service.cancel_authentication("grok", "wrong-login");
        assert_eq!(service.statuses().len(), 1);
        service.cancel_authentication("grok", "first-login");
        assert!(service.statuses().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_native_login_has_one_owner_and_stale_cancel_cannot_remove_it() {
        let (root, service) = service();
        let package = package("grok");
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = ["login-a", "login-b"]
                .into_iter()
                .map(|request| {
                    let service = &service;
                    let package = &package;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        (
                            request,
                            service.begin_authentication(package, request).is_ok(),
                        )
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|(_, accepted)| *accepted).count(), 1);
        let winner = results.iter().find(|(_, accepted)| *accepted).unwrap().0;
        let loser = results.iter().find(|(_, accepted)| !*accepted).unwrap().0;
        service.cancel_authentication("grok", loser);
        assert_eq!(
            service.active_authentication_request("grok").as_deref(),
            Some(winner)
        );
        service.cancel_authentication("grok", winner);
        assert!(service.active_authentication_request("grok").is_none());
        service
            .begin_authentication(&package, "next-login")
            .unwrap();
        service.cancel_authentication("grok", winner);
        assert_eq!(
            service.active_authentication_request("grok").as_deref(),
            Some("next-login")
        );
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_scheduling_generation_holds_the_current_one() {
        let (root, service) = service();
        let package = package("grok");
        let bundle = bundle(&package);
        service.commit(&package, &bundle, None, None).unwrap();
        let refreshed = service.commit(&package, &bundle, None, Some(1)).unwrap();
        assert_eq!(refreshed.auth_generation, 2);
        assert_eq!(
            service
                .with_retained_scheduling_generation("grok", 2, 1, || 5)
                .unwrap(),
            5
        );
        // Never ahead of the Service, and the current generation is exact.
        assert!(
            service
                .with_retained_scheduling_generation("grok", 2, 3, || ())
                .is_err()
        );
        assert!(
            service
                .with_retained_scheduling_generation("grok", 3, 1, || ())
                .is_err()
        );
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn service_generation_is_durable_cas_and_sealed_per_machine() {
        let (root, service) = service();
        let package = package("grok");
        let bundle = bundle(&package);
        let status = service
            .commit(&package, &bundle, Some("account".to_owned()), None)
            .unwrap();
        assert_eq!(status.auth_generation, 1);
        assert_eq!(
            status.authentication_state,
            ServiceAuthenticationState::Ready
        );
        assert_eq!(
            service
                .with_scheduling_generation("grok", true, Some(1), || 7)
                .unwrap(),
            7
        );
        assert_eq!(
            service
                .with_scheduling_generation("provider-without-auth", false, None, || 9)
                .unwrap(),
            9
        );
        assert!(
            service
                .commit(&package, &bundle, None, Some(0))
                .unwrap_err()
                .to_string()
                .contains("generation changed")
        );

        let first_machine_secret = StaticSecret::from(random_bytes::<32>().unwrap());
        let first_machine_public = PublicKey::from(&first_machine_secret);
        let first_envelope = service
            .seal_for_machine(
                "grok",
                &base64::engine::general_purpose::STANDARD.encode(first_machine_public.as_bytes()),
            )
            .unwrap();
        let second_machine_secret = StaticSecret::from(random_bytes::<32>().unwrap());
        let second_machine_public = PublicKey::from(&second_machine_secret);
        let second_envelope = service
            .seal_for_machine(
                "grok",
                &base64::engine::general_purpose::STANDARD.encode(second_machine_public.as_bytes()),
            )
            .unwrap();
        assert_eq!(first_envelope.auth_generation, 1);
        assert_eq!(second_envelope.auth_generation, 1);
        assert_eq!(first_envelope.action, ProviderAuthAction::Apply);
        assert_ne!(first_envelope.ciphertext, second_envelope.ciphertext);
        assert!(!first_envelope.ciphertext.contains("auth_json"));

        let non_contributory = base64::engine::general_purpose::STANDARD.encode([0_u8; 32]);
        assert!(
            service
                .seal_for_machine("grok", &non_contributory)
                .unwrap_err()
                .to_string()
                .contains("non-contributory")
        );

        let logged_out = service.logout("grok").unwrap();
        assert_eq!(logged_out.auth_generation, 2);
        assert_eq!(
            logged_out.authentication_state,
            ServiceAuthenticationState::SignedOut
        );
        assert!(
            service
                .with_scheduling_generation("grok", true, Some(1), || ())
                .unwrap_err()
                .to_string()
                .contains("not ready")
        );
        assert!(
            service
                .mark_distribution("grok", 1, ServiceDistributionState::Current)
                .unwrap_err()
                .to_string()
                .contains("stale Provider distribution receipt")
        );
        let wipe = service
            .seal_for_machine(
                "grok",
                &base64::engine::general_purpose::STANDARD.encode(first_machine_public.as_bytes()),
            )
            .unwrap();
        assert_eq!(wipe.auth_generation, 2);
        assert_eq!(wipe.action, ProviderAuthAction::Wipe);

        drop(service);
        let reopened = ProviderAuthService::open(&root).unwrap();
        let reopened = reopened.status("grok").unwrap();
        assert_eq!(reopened.auth_generation, 2);
        assert_eq!(
            reopened.authentication_state,
            ServiceAuthenticationState::SignedOut
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejected_generation_expires_durably_without_overwriting_a_new_login() {
        let (root, service) = service();
        let package = package("grok");
        let bundle = bundle(&package);
        service
            .commit(&package, &bundle, Some("account".to_owned()), None)
            .unwrap();

        let expired = service
            .mark_expired("grok", 1)
            .unwrap()
            .expect("matching ready generation");
        assert_eq!(
            expired.authentication_state,
            ServiceAuthenticationState::Expired
        );
        assert!(
            service
                .with_scheduling_generation("grok", true, Some(1), || ())
                .unwrap_err()
                .to_string()
                .contains("not ready")
        );
        drop(service);
        let service = ProviderAuthService::open(&root).unwrap();
        assert_eq!(
            service.status("grok").unwrap().authentication_state,
            ServiceAuthenticationState::Expired
        );

        let refreshed = service
            .commit(&package, &bundle, Some("account".to_owned()), Some(1))
            .unwrap();
        assert_eq!(refreshed.auth_generation, 2);
        assert!(service.mark_expired("grok", 1).unwrap().is_none());
        assert_eq!(
            service.status("grok").unwrap().authentication_state,
            ServiceAuthenticationState::Ready
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_login_addition_preserves_the_legacy_refresh_contract() {
        let (root, service) = service();
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(include_str!("../plugins/claude-code/provider.json")).unwrap();
        let next = cowboy_provider_sdk::build_package(source.clone().compile().unwrap()).unwrap();
        let mut previous = source;
        previous
            .authentication
            .methods
            .retain(|method| method.native_identity.is_none());
        let previous = cowboy_provider_sdk::build_package(previous.compile().unwrap()).unwrap();
        let original = PortableCredentialBundle {
            portable_schema: previous.manifest.authentication.portable_schema.clone(),
            method_id: "claude-account".into(),
            values: BTreeMap::from([(
                "credentials_json".into(),
                base64::engine::general_purpose::STANDARD.encode(b"synthetic-original"),
            )]),
        };
        let status = service.commit(&previous, &original, None, None).unwrap();
        let expected = status.auth_contract_fingerprint;
        assert_ne!(
            expected,
            next.manifest.compatibility.auth_contract_fingerprint
        );
        let mut rotated = original;
        rotated.values.insert(
            "credentials_json".into(),
            base64::engine::general_purpose::STANDARD.encode(b"synthetic-rotated"),
        );
        let ProviderAuthRefreshResult::Updated(statuses) = service
            .compare_and_swap_refresh(std::slice::from_ref(&next), &rotated, "claude-code", 1)
            .unwrap()
        else {
            panic!("refresh was not committed");
        };
        assert_eq!(statuses[0].auth_contract_fingerprint, expected);
        assert_eq!(statuses[0].auth_generation, 2);
        // A fresh legacy login on the new release must remain readable by
        // retained old workers and the accepted rollback release as well.
        let refreshed = service.commit(&next, &rotated, None, Some(2)).unwrap();
        assert_eq!(refreshed.auth_contract_fingerprint, expected);
        drop(service);
        let reopened = ProviderAuthService::open(&root).unwrap();
        assert_eq!(
            reopened
                .status("claude-code")
                .unwrap()
                .auth_contract_fingerprint,
            expected
        );
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_refresh_is_a_single_service_generation_cas() {
        let (root, service) = service();
        let package = package("grok");
        let initial = bundle(&package);
        service
            .commit(&package, &initial, Some("account".to_owned()), None)
            .unwrap();

        let mut winner = initial.clone();
        winner.values.insert(
            "auth_json".to_owned(),
            base64::engine::general_purpose::STANDARD.encode(br#"{"token":"winner"}"#),
        );
        let updated = service
            .compare_and_swap_refresh(std::slice::from_ref(&package), &winner, "grok", 1)
            .unwrap();
        let ProviderAuthRefreshResult::Updated(statuses) = updated else {
            panic!("first valid refresh did not advance the Service generation");
        };
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].auth_generation, 2);
        assert_eq!(
            statuses[0].authentication_state,
            ServiceAuthenticationState::Ready
        );
        assert_eq!(
            statuses[0].distribution_state,
            ServiceDistributionState::Pending
        );

        let mut loser = initial;
        loser.values.insert(
            "auth_json".to_owned(),
            base64::engine::general_purpose::STANDARD.encode(br#"{"token":"loser"}"#),
        );
        assert_eq!(
            service
                .compare_and_swap_refresh(std::slice::from_ref(&package), &loser, "grok", 1,)
                .unwrap(),
            ProviderAuthRefreshResult::Stale {
                current_generation: 2
            }
        );

        let stored = service.providers.read().get("grok").cloned().unwrap();
        let stored_bundle: PortableCredentialBundle =
            serde_json::from_slice(&service.open_vault(&stored).unwrap()).unwrap();
        assert_eq!(stored_bundle, winner);
        let ProviderAuthRefreshResult::Unchanged(statuses) = service
            .compare_and_swap_refresh(&[package], &winner, "grok", 2)
            .unwrap()
        else {
            panic!("replaying the winning credential should be idempotent");
        };
        assert_eq!(statuses[0].auth_generation, 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn valid_runtime_refresh_can_rescue_the_exact_expired_generation() {
        let (root, service) = service();
        let package = package("grok");
        let initial = bundle(&package);
        service
            .commit(&package, &initial, Some("account".to_owned()), None)
            .unwrap();
        service.mark_expired("grok", 1).unwrap().unwrap();
        let mut refreshed = initial;
        refreshed.values.insert(
            "auth_json".to_owned(),
            base64::engine::general_purpose::STANDARD.encode(br#"{"token":"recovered"}"#),
        );

        let ProviderAuthRefreshResult::Updated(statuses) = service
            .compare_and_swap_refresh(&[package], &refreshed, "grok", 1)
            .unwrap()
        else {
            panic!("the exact expired generation was not rescued");
        };
        assert_eq!(statuses[0].auth_generation, 2);
        assert_eq!(
            statuses[0].authentication_state,
            ServiceAuthenticationState::Ready
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_authentication_scope_commits_one_bundle_to_each_projection() {
        let (root, service) = service();
        let claude = package("claude-deepseek");
        let codex = package("codex-deepseek");
        let mut harness = codex.clone();
        harness.manifest.id = "deepseek-harness".to_owned();
        harness.manifest.authentication.projection_schema =
            "deepseek-harness-gateway-v1".to_owned();
        let bundle = bundle(&claude);
        let statuses = service
            .commit_shared(
                &[claude.clone(), codex.clone(), harness.clone()],
                &bundle,
                None,
                "claude-deepseek",
                None,
            )
            .unwrap();
        assert_eq!(statuses.len(), 3);
        assert!(statuses.iter().all(|status| {
            status.auth_generation == 1
                && status.authentication_state == ServiceAuthenticationState::Ready
                && status.authentication_scope == "deepseek-api-key-v1"
        }));
        let first_machine_secret = StaticSecret::from(random_bytes::<32>().unwrap());
        let first_machine_public = PublicKey::from(&first_machine_secret);
        let first = service
            .seal_for_machine(
                "claude-deepseek",
                &base64::engine::general_purpose::STANDARD.encode(first_machine_public.as_bytes()),
            )
            .unwrap();
        let second = service
            .seal_for_machine(
                "codex-deepseek",
                &base64::engine::general_purpose::STANDARD.encode(first_machine_public.as_bytes()),
            )
            .unwrap();
        let third = service
            .seal_for_machine(
                "deepseek-harness",
                &base64::engine::general_purpose::STANDARD.encode(first_machine_public.as_bytes()),
            )
            .unwrap();
        assert_eq!(first.auth_generation, second.auth_generation);
        assert_eq!(second.auth_generation, third.auth_generation);
        assert_eq!(first.action, ProviderAuthAction::Apply);
        assert_eq!(second.action, ProviderAuthAction::Apply);
        assert_eq!(third.action, ProviderAuthAction::Apply);
        assert_ne!(first.ciphertext, second.ciphertext);
        assert_ne!(second.ciphertext, third.ciphertext);

        let provider_ids = vec![
            "claude-deepseek".to_owned(),
            "codex-deepseek".to_owned(),
            "deepseek-harness".to_owned(),
        ];
        let logged_out = service.logout_shared(&provider_ids).unwrap();
        assert_eq!(logged_out.len(), 3);
        assert!(logged_out.iter().all(|status| {
            status.auth_generation == 2
                && status.authentication_state == ServiceAuthenticationState::SignedOut
        }));
        fs::remove_dir_all(root).unwrap();
    }

    fn harness(codex: &ProviderPackage) -> ProviderPackage {
        let mut harness = codex.clone();
        harness.manifest.id = "deepseek-harness".to_owned();
        harness.manifest.authentication.projection_schema =
            "deepseek-harness-gateway-v1".to_owned();
        harness
    }

    #[test]
    fn later_scope_member_adopts_the_ready_shared_credential_once() {
        let (root, service) = service();
        let claude = package("claude-deepseek");
        let codex = package("codex-deepseek");
        let harness = harness(&codex);
        let bundle = bundle(&claude);
        service
            .commit_shared(
                &[claude.clone(), codex.clone()],
                &bundle,
                Some("Mole".into()),
                "claude-deepseek",
                None,
            )
            .unwrap();
        // A refresh advances the shared lineage before the new member exists.
        service
            .commit_shared(
                &[claude.clone(), codex.clone()],
                &bundle,
                Some("Mole".into()),
                "codex-deepseek",
                Some(1),
            )
            .unwrap();
        let scope = [claude.clone(), codex.clone(), harness.clone()];

        let adopted = service.adopt_shared_scope(&scope).unwrap();
        assert_eq!(adopted.len(), 1);
        let status = &adopted[0];
        assert_eq!(status.provider_id, "deepseek-harness");
        assert_eq!(status.auth_generation, 2);
        assert_eq!(
            status.authentication_state,
            ServiceAuthenticationState::Ready
        );
        assert_eq!(status.distribution_state, ServiceDistributionState::Pending);
        assert_eq!(status.projection_schema, "deepseek-harness-gateway-v1");
        assert_eq!(status.account_label.as_deref(), Some("Mole"));
        let machine_secret = StaticSecret::from(random_bytes::<32>().unwrap());
        let machine = base64::engine::general_purpose::STANDARD
            .encode(PublicKey::from(&machine_secret).as_bytes());
        let sealed = service
            .seal_for_machine("deepseek-harness", &machine)
            .unwrap();
        assert_eq!(sealed.action, ProviderAuthAction::Apply);
        assert_eq!(sealed.auth_generation, 2);
        assert!(service.adopt_shared_scope(&scope).unwrap().is_empty());

        // The adopted member joins the one shared lineage for logout.
        let ids: Vec<_> = scope
            .iter()
            .map(|package| package.manifest.id.clone())
            .collect();
        let logged_out = service.logout_shared(&ids).unwrap();
        assert!(logged_out.iter().all(|status| status.auth_generation == 3));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scope_adoption_never_revives_a_logout_or_overwrites_member_state() {
        let (other_root, empty) = service();
        let (root, service) = service();
        let claude = package("claude-deepseek");
        let codex = package("codex-deepseek");
        let harness = harness(&codex);
        let bundle = bundle(&claude);
        service
            .commit_shared(
                &[claude.clone(), codex.clone()],
                &bundle,
                None,
                "claude-deepseek",
                None,
            )
            .unwrap();
        service
            .logout_shared(&["claude-deepseek".to_owned(), "codex-deepseek".to_owned()])
            .unwrap();
        let scope = [claude.clone(), codex.clone(), harness.clone()];
        assert!(service.adopt_shared_scope(&scope).unwrap().is_empty());
        assert!(service.status("deepseek-harness").is_none());

        // A member logged out on its own keeps its tombstone.
        service
            .commit_shared(
                &[claude.clone(), codex.clone(), harness.clone()],
                &bundle,
                None,
                "claude-deepseek",
                Some(2),
            )
            .unwrap();
        service.logout("deepseek-harness").unwrap();
        let before = service.status("deepseek-harness").unwrap();
        assert!(service.adopt_shared_scope(&scope).unwrap().is_empty());
        assert_eq!(service.status("deepseek-harness").unwrap(), before);

        // Nothing to adopt from an unauthenticated scope or a lone Provider.
        assert!(empty.adopt_shared_scope(&scope).unwrap().is_empty());
        assert!(service.adopt_shared_scope(&[harness]).unwrap().is_empty());
        fs::remove_dir_all(other_root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scope_logout_selects_members_under_its_own_lock() {
        let (root, service) = service();
        let claude = package("claude-deepseek");
        let codex = package("codex-deepseek");
        let harness = harness(&codex);
        let bundle = bundle(&claude);
        service
            .commit_shared(
                &[claude.clone(), codex.clone()],
                &bundle,
                None,
                "claude-deepseek",
                None,
            )
            .unwrap();
        // A caller that listed members before this adoption still wipes it.
        service
            .adopt_shared_scope(&[claude.clone(), codex.clone(), harness.clone()])
            .unwrap();
        let ids = vec![
            "claude-deepseek".to_owned(),
            "codex-deepseek".to_owned(),
            "deepseek-harness".to_owned(),
            "grok".to_owned(),
        ];
        let logged_out = service.logout_scope(&ids).unwrap();
        assert_eq!(logged_out.len(), 3);
        assert!(logged_out.iter().all(|status| {
            status.auth_generation == 2
                && status.authentication_state == ServiceAuthenticationState::SignedOut
        }));
        assert!(service.logout_scope(&["grok".to_owned()]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
