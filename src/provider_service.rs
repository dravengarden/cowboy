//! Cowboy Service-owned Provider authentication vault and replica sealing.
//!
//! Authentication is intentionally not a Machine property. The Service owns
//! one monotonically increasing generation per Provider, encrypts the portable
//! bundle at rest, and seals that generation independently to every enrolled
//! Machine. Machines may materialize a replica only after the signed Provider
//! authentication contract matches exactly.

#![warn(clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;
use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use cowboy_provider_sdk::{AuthenticationContract, ProviderPackage};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::machine_auth::{MachineIdentity, PROVIDER_AUTH_SIGNATURE_NAMESPACE};
use crate::machine_protocol::{PortableCredentialBundle, ProviderAuthAction, SealedProviderAuth};

const VAULT_SCHEMA: u16 = 1;
const AUTH_ENVELOPE_SCHEMA: u16 = 1;
const MAX_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
const AUTH_SEAL_DOMAIN: &[u8] = b"cowboy-provider-auth-seal-v1\0";
const VAULT_DOMAIN: &[u8] = b"cowboy-provider-auth-vault-v1\0";
static ATOMIC_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServiceAuthenticationState {
    SignedOut,
    Authenticating,
    Ready,
    Expired,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServiceDistributionState {
    None,
    Pending,
    Current,
    Partial,
    Failed,
    Revoking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ProviderAuthenticationStatus {
    pub provider_id: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredProviderAuthentication {
    vault_schema: u16,
    provider_id: String,
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
    fn status(&self) -> ProviderAuthenticationStatus {
        ProviderAuthenticationStatus {
            provider_id: self.provider_id.clone(),
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

pub(crate) struct ProviderAuthService {
    root: PathBuf,
    vault_key: [u8; 32],
    signer: MachineIdentity,
    providers: RwLock<BTreeMap<String, StoredProviderAuthentication>>,
    transient: RwLock<BTreeMap<String, TransientAuthenticationStatus>>,
}

impl ProviderAuthService {
    pub(crate) fn open(data_dir: &Path) -> Result<Self> {
        let root = data_dir.join("provider-auth-service");
        fs::create_dir_all(root.join("providers"))
            .with_context(|| format!("creating Provider auth vault {}", root.display()))?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(root.join("providers"), fs::Permissions::from_mode(0o700))?;
        let vault_key = load_or_create_secret(&root.join("vault.key"))?;
        let signer = MachineIdentity::load_or_create(&root.join("signer"))?;
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
                providers
                    .insert(stored.provider_id.clone(), stored)
                    .is_none(),
                "duplicate Provider authentication state"
            );
        }
        let service = Self {
            root,
            vault_key,
            signer,
            providers: RwLock::new(providers),
            transient: RwLock::new(BTreeMap::new()),
        };
        for stored in service.providers.read().values() {
            validate_vault_plaintext(stored, &service.open_vault(stored)?)?;
        }
        Ok(service)
    }

    #[must_use]
    pub(crate) fn statuses(&self) -> Vec<ProviderAuthenticationStatus> {
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
    pub(crate) fn replica_statuses(&self) -> Vec<ProviderAuthenticationStatus> {
        self.providers
            .read()
            .values()
            .map(StoredProviderAuthentication::status)
            .collect()
    }

    #[must_use]
    pub(crate) fn status(&self, provider_id: &str) -> Option<ProviderAuthenticationStatus> {
        self.providers
            .read()
            .get(provider_id)
            .map(StoredProviderAuthentication::status)
    }

    pub(crate) fn begin_authentication(
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

    pub(crate) fn cancel_authentication(&self, provider_id: &str, request_id: &str) {
        let mut transient = self.transient.write();
        if transient
            .get(provider_id)
            .is_some_and(|active| active.request_id == request_id)
        {
            transient.remove(provider_id);
        }
    }

    pub(crate) fn fail_authentication(&self, provider_id: &str, request_id: &str) {
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
    pub(crate) fn with_scheduling_generation<T>(
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

    /// Commit a successfully authenticated portable bundle as the next Service
    /// generation. A stale executor cannot overwrite a newer generation when
    /// `expected_generation` is supplied.
    pub(crate) fn commit(
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
    pub(crate) fn commit_shared(
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
        for package in packages {
            validate_bundle(&package.manifest.authentication, bundle)?;
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
        let current_generation = packages
            .iter()
            .filter_map(|package| providers.get(&package.manifest.id))
            .map(|stored| stored.auth_generation)
            .max()
            .unwrap_or(0);
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
            let (nonce, ciphertext) = self.seal_vault(
                provider_id,
                generation,
                &package.manifest.compatibility.auth_contract_fingerprint,
                &plaintext,
            )?;
            stored_values.push(StoredProviderAuthentication {
                vault_schema: VAULT_SCHEMA,
                provider_id: provider_id.clone(),
                auth_generation: generation,
                authentication_state: ServiceAuthenticationState::Ready,
                distribution_state: ServiceDistributionState::Pending,
                auth_contract_fingerprint: package
                    .manifest
                    .compatibility
                    .auth_contract_fingerprint
                    .clone(),
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

    /// Publish a newer wipe generation. Old encrypted vault generations are
    /// no longer addressable and every connected Machine receives this tombstone.
    pub(crate) fn logout(&self, provider_id: &str) -> Result<ProviderAuthenticationStatus> {
        self.logout_shared(&[provider_id.to_owned()])?
            .into_iter()
            .next()
            .context("Provider logout produced no status")
    }

    /// Publish one wipe generation for every Provider in a shared
    /// authentication scope.
    pub(crate) fn logout_shared(
        &self,
        provider_ids: &[String],
    ) -> Result<Vec<ProviderAuthenticationStatus>> {
        ensure!(
            !provider_ids.is_empty(),
            "authentication scope has no Providers"
        );
        let mut providers = self.providers.write();
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
            )?;
            statuses.push(self.replace_locked(
                &mut providers,
                StoredProviderAuthentication {
                    vault_schema: VAULT_SCHEMA,
                    provider_id: current.provider_id,
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
    pub(crate) fn seal_for_machine(
        &self,
        provider_id: &str,
        machine_public_key: &str,
    ) -> Result<SealedProviderAuth> {
        let recipient = decode_fixed::<32>(machine_public_key, "Machine encryption public key")?;
        let stored = self
            .providers
            .read()
            .get(provider_id)
            .cloned()
            .context("Provider is not authenticated at Cowboy Service scope")?;
        let plaintext = if stored.action == ProviderAuthAction::Apply {
            self.open_vault(&stored)?
        } else {
            Vec::new()
        };

        let ephemeral = StaticSecret::from(random_bytes::<32>()?);
        let ephemeral_public = PublicKey::from(&ephemeral);
        let shared = ephemeral.diffie_hellman(&PublicKey::from(recipient));
        let key = derive_seal_key(shared.as_bytes());
        let nonce = random_bytes::<24>()?;
        let mut envelope = SealedProviderAuth {
            envelope_schema: AUTH_ENVELOPE_SCHEMA,
            provider_id: stored.provider_id,
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

    pub(crate) fn mark_distribution(
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

    fn seal_vault(
        &self,
        provider_id: &str,
        generation: u64,
        fingerprint: &str,
        plaintext: &[u8],
    ) -> Result<(String, String)> {
        let nonce = random_bytes::<24>()?;
        let ciphertext = XChaCha20Poly1305::new(Key::from_slice(&self.vault_key))
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &vault_aad(provider_id, generation, fingerprint),
                },
            )
            .map_err(|_| anyhow::anyhow!("encrypting Provider auth vault failed"))?;
        Ok((
            base64::engine::general_purpose::STANDARD.encode(nonce),
            base64::engine::general_purpose::STANDARD.encode(ciphertext),
        ))
    }

    fn open_vault(&self, stored: &StoredProviderAuthentication) -> Result<Vec<u8>> {
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
                    ),
                },
            )
            .map_err(|_| anyhow::anyhow!("Provider auth vault authentication failed"))
    }
}

fn validate_stored(stored: &StoredProviderAuthentication) -> Result<()> {
    ensure!(
        stored.vault_schema == VAULT_SCHEMA,
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
        (stored.authentication_state == ServiceAuthenticationState::Ready
            && stored.action == ProviderAuthAction::Apply)
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
    let action = match envelope.action {
        ProviderAuthAction::Apply => "apply",
        ProviderAuthAction::Wipe => "wipe",
    };
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n",
        envelope.envelope_schema,
        envelope.provider_id,
        envelope.auth_generation,
        envelope.auth_contract_fingerprint,
        envelope.projection_schema,
        action
    )
    .into_bytes()
}

fn vault_aad(provider_id: &str, generation: u64, fingerprint: &str) -> Vec<u8> {
    let mut output = VAULT_DOMAIN.to_vec();
    output.extend_from_slice(provider_id.as_bytes());
    output.push(b'\n');
    output.extend_from_slice(generation.to_string().as_bytes());
    output.push(b'\n');
    output.extend_from_slice(fingerprint.as_bytes());
    output.push(b'\n');
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
    Ok(())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(id: &str) -> ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource = serde_json::from_str(match id {
            "gemini" => include_str!("../providers/gemini/provider.json"),
            "claude-deepseek" => include_str!("../providers/claude-deepseek/provider.json"),
            "codex-deepseek" => include_str!("../providers/codex-deepseek/provider.json"),
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
            ["settings_json", "oauth_creds_json"]
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
                    "code-assist"
                }
                .to_owned(),
            values,
        }
    }

    #[test]
    fn transient_login_state_is_service_wide_but_not_replicated() {
        let (root, service) = service();
        let package = package("gemini");
        let authenticating = service
            .begin_authentication(&package, "first-login")
            .unwrap();
        assert_eq!(authenticating.auth_generation, 0);
        assert_eq!(
            authenticating.authentication_state,
            ServiceAuthenticationState::Authenticating
        );
        assert!(service.status("gemini").is_none());
        assert!(service.replica_statuses().is_empty());
        assert_eq!(service.statuses(), vec![authenticating]);
        assert!(
            service
                .begin_authentication(&package, "duplicate-login")
                .unwrap_err()
                .to_string()
                .contains("already in progress")
        );
        service.fail_authentication("gemini", "first-login");
        assert_eq!(
            service.statuses()[0].authentication_state,
            ServiceAuthenticationState::Error
        );
        service.cancel_authentication("gemini", "wrong-login");
        assert_eq!(service.statuses().len(), 1);
        service.cancel_authentication("gemini", "first-login");
        assert!(service.statuses().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn service_generation_is_durable_cas_and_sealed_per_machine() {
        let (root, service) = service();
        let package = package("gemini");
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
                .with_scheduling_generation("gemini", true, Some(1), || 7)
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
                "gemini",
                &base64::engine::general_purpose::STANDARD.encode(first_machine_public.as_bytes()),
            )
            .unwrap();
        let second_machine_secret = StaticSecret::from(random_bytes::<32>().unwrap());
        let second_machine_public = PublicKey::from(&second_machine_secret);
        let second_envelope = service
            .seal_for_machine(
                "gemini",
                &base64::engine::general_purpose::STANDARD.encode(second_machine_public.as_bytes()),
            )
            .unwrap();
        assert_eq!(first_envelope.auth_generation, 1);
        assert_eq!(second_envelope.auth_generation, 1);
        assert_eq!(first_envelope.action, ProviderAuthAction::Apply);
        assert_ne!(first_envelope.ciphertext, second_envelope.ciphertext);
        assert!(!first_envelope.ciphertext.contains("settings_json"));

        let logged_out = service.logout("gemini").unwrap();
        assert_eq!(logged_out.auth_generation, 2);
        assert_eq!(
            logged_out.authentication_state,
            ServiceAuthenticationState::SignedOut
        );
        assert!(
            service
                .with_scheduling_generation("gemini", true, Some(1), || ())
                .unwrap_err()
                .to_string()
                .contains("not ready")
        );
        assert!(
            service
                .mark_distribution("gemini", 1, ServiceDistributionState::Current)
                .unwrap_err()
                .to_string()
                .contains("stale Provider distribution receipt")
        );
        let wipe = service
            .seal_for_machine(
                "gemini",
                &base64::engine::general_purpose::STANDARD.encode(first_machine_public.as_bytes()),
            )
            .unwrap();
        assert_eq!(wipe.auth_generation, 2);
        assert_eq!(wipe.action, ProviderAuthAction::Wipe);

        let reopened = ProviderAuthService::open(&root).unwrap();
        let reopened = reopened.status("gemini").unwrap();
        assert_eq!(reopened.auth_generation, 2);
        assert_eq!(
            reopened.authentication_state,
            ServiceAuthenticationState::SignedOut
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_authentication_scope_commits_one_bundle_to_each_projection() {
        let (root, service) = service();
        let claude = package("claude-deepseek");
        let codex = package("codex-deepseek");
        let bundle = bundle(&claude);
        let statuses = service
            .commit_shared(
                &[claude.clone(), codex.clone()],
                &bundle,
                None,
                "claude-deepseek",
                None,
            )
            .unwrap();
        assert_eq!(statuses.len(), 2);
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
        assert_eq!(first.auth_generation, second.auth_generation);
        assert_eq!(first.action, ProviderAuthAction::Apply);
        assert_eq!(second.action, ProviderAuthAction::Apply);
        assert_ne!(first.ciphertext, second.ciphertext);

        let provider_ids = vec!["claude-deepseek".to_owned(), "codex-deepseek".to_owned()];
        let logged_out = service.logout_shared(&provider_ids).unwrap();
        assert_eq!(logged_out.len(), 2);
        assert!(logged_out.iter().all(|status| {
            status.auth_generation == 2
                && status.authentication_state == ServiceAuthenticationState::SignedOut
        }));
        fs::remove_dir_all(root).unwrap();
    }
}
