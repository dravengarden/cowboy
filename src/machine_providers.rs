//! Machine-owned Provider package generations and credential projections.
//!
//! The Controller selects an immutable Catalog release, but the target Machine
//! repeats every trust, compatibility, platform, and interface check before an
//! active link changes. Internal component slots are accepted only as private
//! implementation prerequisites of that signed Provider package.

#![warn(clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use cowboy_provider_sdk::{
    Architecture, OperatingSystem, PlatformRuntimeArtifacts, ProviderArtifactFormat,
    ProviderPackage, ReleasedPrivateComponent, RuntimeContract,
};
use futures::StreamExt as _;
use sha2::{Digest as _, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::machine_protocol::{
    DesiredProvider, Platform, PortableCredentialBundle, ProviderAuthAction,
    ProviderInstallationState, ProviderInventory, ProviderMaterializationState,
    ProviderReplicaState, SealedProviderAuth,
};

const MAX_PROVIDER_PACKAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES: usize = 1024 * 1024 * 1024;
const MAX_PROVIDER_RUNTIME_EXPANDED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_PROVIDER_RUNTIME_ARCHIVE_ENTRIES: usize = 100_000;
const MAX_CREDENTIAL_VALUE_BYTES: usize = 4 * 1024 * 1024;
const MAX_CREDENTIAL_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
const PROVIDER_PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const AUTH_CANDIDATE_MAX_AGE: Duration = Duration::from_mins(30);
const AUTH_SEAL_DOMAIN: &[u8] = b"cowboy-provider-auth-seal-v1\0";
static ATOMIC_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub(crate) struct ProviderLaunchContext {
    pub package_path: PathBuf,
    pub command: String,
    pub version: String,
    pub behavior: cowboy_provider_sdk::ProviderBehaviorContract,
    pub environment: BTreeMap<String, String>,
    pub remove_environment: BTreeSet<String>,
    pub remove_environment_prefixes: BTreeSet<String>,
    pub home: Option<PathBuf>,
}

pub(crate) struct ExportedAuthCandidate {
    pub provider_version: String,
    pub generation_digest: String,
    pub auth_contract_fingerprint: String,
    pub bundle: PortableCredentialBundle,
}

#[derive(Clone)]
pub(crate) struct MachineEncryptionIdentity {
    secret: StaticSecret,
    public_key: String,
}

impl MachineEncryptionIdentity {
    pub fn load_or_create(state_dir: &Path) -> Result<Self> {
        fs::create_dir_all(state_dir)
            .with_context(|| format!("creating encryption identity dir {}", state_dir.display()))?;
        fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700))?;
        let secret_path = state_dir.join("identity_x25519");
        let bytes = if secret_path.exists() {
            fs::read(&secret_path).with_context(|| format!("reading {}", secret_path.display()))?
        } else {
            let mut bytes = [0_u8; 32];
            fs::File::open("/dev/urandom")
                .context("opening OS randomness")?
                .read_exact(&mut bytes)
                .context("reading X25519 secret")?;
            atomic_write(&secret_path, &bytes, 0o600)?;
            bytes.to_vec()
        };
        ensure!(
            bytes.len() == 32,
            "Machine X25519 identity has invalid length"
        );
        fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o600))?;
        let mut secret = [0_u8; 32];
        secret.copy_from_slice(&bytes);
        let secret = StaticSecret::from(secret);
        let public = PublicKey::from(&secret);
        let public_key = base64::engine::general_purpose::STANDARD.encode(public.as_bytes());
        Ok(Self { secret, public_key })
    }

    #[must_use]
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    fn open(&self, envelope: &SealedProviderAuth) -> Result<Vec<u8>> {
        let ephemeral = decode_fixed::<32>(&envelope.ephemeral_public_key, "ephemeral public key")?;
        let nonce = decode_fixed::<24>(&envelope.nonce, "auth nonce")?;
        let ciphertext = base64::engine::general_purpose::STANDARD
            .decode(&envelope.ciphertext)
            .context("decoding sealed credential ciphertext")?;
        ensure!(
            ciphertext.len() <= MAX_CREDENTIAL_BUNDLE_BYTES + 64,
            "sealed credential bundle is too large"
        );
        let shared = self.secret.diffie_hellman(&PublicKey::from(ephemeral));
        let key = derive_seal_key(shared.as_bytes());
        XChaCha20Poly1305::new(Key::from_slice(&key))
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &provider_auth_aad(envelope),
                },
            )
            .map_err(|_| anyhow::anyhow!("sealed Provider credential authentication failed"))
    }
}

pub(crate) struct MachineProviderStore {
    root: PathBuf,
    auth_root: PathBuf,
    platform: Platform,
    architecture: String,
    encryption: MachineEncryptionIdentity,
    lifecycle: tokio::sync::Mutex<()>,
}

enum PreparedProviderAuth {
    Wipe,
    NotInstalled,
    Apply(PortableCredentialBundle),
}

#[derive(Debug)]
struct ProviderActivationSnapshot {
    active: Option<String>,
    rollback: Option<String>,
}

impl MachineProviderStore {
    pub fn new(state_dir: &Path, platform: Platform, architecture: String) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = state_dir.join("providers");
        let auth_root = state_dir.join("provider-auth");
        for directory in [&root, &auth_root] {
            fs::create_dir_all(directory)
                .with_context(|| format!("creating {}", directory.display()))?;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
        let encryption = MachineEncryptionIdentity::load_or_create(&auth_root.join("identity"))?;
        Ok(Self {
            root,
            auth_root,
            platform,
            architecture,
            encryption,
            lifecycle: tokio::sync::Mutex::new(()),
        })
    }

    #[must_use]
    pub fn encryption_public_key(&self) -> &str {
        self.encryption.public_key()
    }

    pub async fn install(&self, desired: &DesiredProvider) -> Result<ProviderInventory> {
        let _lifecycle = self.lifecycle.lock().await;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&desired.package_base64)
            .context("decoding Provider package")?;
        ensure!(
            bytes.len() <= MAX_PROVIDER_PACKAGE_BYTES,
            "Provider package exceeds 8 MiB"
        );
        let package = desired.release.validate_bytes(&bytes)?;
        let signature_valid = crate::machine_auth::verify_namespaced(
            &desired.publisher_public_key,
            crate::machine_auth::PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
            &desired.release.proof(),
            &desired.release.signature,
        )?;
        ensure!(signature_valid, "Provider publisher signature is invalid");
        self.pin_publisher(&package.manifest.publisher, &desired.publisher_public_key)?;
        let payload = matching_payload(&package, &self.platform, &self.architecture)?;
        let runtime_artifacts =
            matching_runtime_artifacts(&desired.release, &self.platform, &self.architecture)?;
        let auth_envelope = self.latest_auth_envelope(&package.manifest.id)?;
        let prepared_auth = auth_envelope
            .as_ref()
            .map(|envelope| self.prepare_auth_inner(envelope, Some(&package)))
            .transpose()?;

        let provider_root = self.provider_root(&package.manifest.id);
        let generation_name = digest_generation_name(&desired.release.artifact_digest)?;
        let generation = provider_root.join("generations").join(&generation_name);
        let content = generation.join("content");
        fs::create_dir_all(&content)
            .with_context(|| format!("creating Provider generation {}", generation.display()))?;
        fs::set_permissions(&generation, fs::Permissions::from_mode(0o700))?;
        atomic_write(&content.join("package.cowboy-provider"), &bytes, 0o600)?;
        atomic_write(
            &content.join("release.json"),
            &serde_json::to_vec(&desired.release)?,
            0o600,
        )?;
        atomic_write(
            &content.join("publisher.pub"),
            desired.publisher_public_key.as_bytes(),
            0o600,
        )?;

        let runtime = stage_provider_runtime(&content, runtime_artifacts).await?;
        let launch_command = runtime
            .commands
            .get(&payload.launch_command)
            .context("staged Provider runtime does not export its launch command")?;
        let launch_command = content.join(&launch_command.executable);
        ensure_within(&content, &launch_command)?;
        probe_provider_runtime(&package.manifest.runtime, &launch_command).await?;
        let activation = Self::activate(&provider_root, &generation_name)?;
        // A sealed Service replica may predate installation. Materialize it as
        // part of activation so installation never asks for another login.
        if let (Some(envelope), Some(prepared)) = (auth_envelope.as_ref(), prepared_auth)
            && let Err(error) = self.commit_prepared_auth(envelope, Some(&package), prepared)
        {
            if let Err(rollback_error) = Self::restore_activation(&provider_root, &activation) {
                bail!(
                    "Provider authentication activation failed: {error:#}; restoring the previous Provider generation also failed: {rollback_error:#}"
                );
            }
            return Err(error.context(
                "Provider authentication activation failed; previous generation restored",
            ));
        }
        self.inventory_one(&package.manifest.id)?
            .context("activated Provider is missing from inventory")
    }

    /// Re-activate one already verified, retained generation. The Controller
    /// uses this only as uninstall-saga compensation when its durable session
    /// transaction fails after the Machine has removed the active link.
    pub async fn reactivate(
        &self,
        provider_id: &str,
        generation_digest: &str,
    ) -> Result<ProviderInventory> {
        let _lifecycle = self.lifecycle.lock().await;
        validate_provider_id(provider_id)?;
        let generation_name = digest_generation_name(generation_digest)?;
        let (package, release, package_path) =
            self.verified_generation(provider_id, generation_digest)?;
        let payload = matching_payload(&package, &self.platform, &self.architecture)?;
        let runtime_artifacts =
            matching_runtime_artifacts(&release, &self.platform, &self.architecture)?;
        let content = package_path
            .parent()
            .context("Provider package has no generation content directory")?;
        let runtime_metadata = read_installed_runtime(content)?;
        ensure!(
            installed_runtime_matches(content, runtime_artifacts, &runtime_metadata)?,
            "retained Provider runtime failed integrity verification"
        );
        let launch_command = runtime_command(&package_path, &payload.launch_command)?;
        probe_provider_runtime(&package.manifest.runtime, &launch_command).await?;
        let auth_envelope = self.latest_auth_envelope(provider_id)?;
        let prepared_auth = auth_envelope
            .as_ref()
            .map(|envelope| self.prepare_auth_inner(envelope, Some(&package)))
            .transpose()?;
        let provider_root = self.provider_root(provider_id);
        let activation = Self::activate(&provider_root, &generation_name)?;
        if let (Some(envelope), Some(prepared)) = (auth_envelope.as_ref(), prepared_auth)
            && let Err(error) = self.commit_prepared_auth(envelope, Some(&package), prepared)
        {
            if let Err(rollback_error) = Self::restore_activation(&provider_root, &activation) {
                bail!(
                    "Provider authentication reactivation failed: {error:#}; restoring the previous activation also failed: {rollback_error:#}"
                );
            }
            return Err(error.context("Provider reactivation failed; previous activation restored"));
        }
        self.inventory_one(provider_id)?
            .context("reactivated Provider is missing from inventory")
    }

    pub async fn uninstall(&self, provider_id: &str, expected_digest: &str) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        validate_provider_id(provider_id)?;
        let active = self
            .active_package(provider_id)?
            .context("Provider is not installed")?;
        ensure!(
            active.1 == expected_digest,
            "active Provider generation changed; refresh the uninstall plan"
        );
        let provider_root = self.provider_root(provider_id);
        let active_link = provider_root.join("active");
        if active_link.exists() || active_link.symlink_metadata().is_ok() {
            fs::remove_file(&active_link)
                .with_context(|| format!("removing {}", active_link.display()))?;
        }
        let materialized = self.auth_provider_root(provider_id).join("materialized");
        if materialized.exists() {
            fs::remove_dir_all(&materialized).with_context(|| {
                format!(
                    "removing Provider credential projection {}",
                    materialized.display()
                )
            })?;
        }
        Ok(())
    }

    pub fn inventory(&self) -> Result<Vec<ProviderInventory>> {
        let mut output = Vec::new();
        if !self.root.exists() {
            return Ok(output);
        }
        for entry in fs::read_dir(&self.root).context("reading Provider store")? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let provider_id = entry.file_name().to_string_lossy().into_owned();
            if let Some(inventory) = self.inventory_one(&provider_id)? {
                output.push(inventory);
            }
        }
        output.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        Ok(output)
    }

    pub fn launch_context(
        &self,
        provider_id: &str,
        generation_digest: &str,
        auth_generation: Option<u64>,
    ) -> Result<ProviderLaunchContext> {
        let (package, package_path) =
            self.package_for_generation(provider_id, generation_digest)?;
        let payload = matching_payload(&package, &self.platform, &self.architecture)?;
        let auth = &package.manifest.authentication;
        let home = if auth.required {
            let generation = auth_generation.context("session has no Provider auth generation")?;
            let materialized = self
                .auth_provider_root(provider_id)
                .join("materialized/generations")
                .join(generation.to_string());
            let metadata: MaterializationMetadata = serde_json::from_slice(
                &fs::read(materialized.join("metadata.json")).context("reading auth projection")?,
            )?;
            ensure!(
                metadata.auth_generation == generation,
                "session Provider auth generation is not materialized"
            );
            ensure!(
                metadata.auth_contract_fingerprint
                    == package.manifest.compatibility.auth_contract_fingerprint,
                "session Provider auth generation uses a different contract"
            );
            Some(materialized.join("home"))
        } else {
            None
        };
        let mut environment = BTreeMap::new();
        if let Some(current) = home.as_ref().and_then(|home| home.parent()) {
            let env_path = current.join("environment.json");
            if env_path.is_file() {
                let projected: BTreeMap<String, String> =
                    serde_json::from_slice(&fs::read(&env_path)?)?;
                environment.extend(projected);
            }
        }
        let mut component_commands = BTreeMap::new();
        let mut component_directories = BTreeSet::new();
        for component in &payload.private_components {
            let executable = runtime_command(&package_path, &component.command)?;
            let parent = executable
                .parent()
                .context("installed Provider component has no parent directory")?;
            component_directories.insert(parent.to_path_buf());
            component_commands.insert(component.command.clone(), executable.display().to_string());
        }
        environment.insert(
            crate::provider_behavior::COMPONENT_COMMANDS_ENV.to_owned(),
            serde_json::to_string(&component_commands)?,
        );
        let mut provider_path = component_directories.into_iter().collect::<Vec<_>>();
        if let Some(inherited) = std::env::var_os("PATH") {
            provider_path.extend(std::env::split_paths(&inherited));
        }
        environment.insert(
            "PATH".to_owned(),
            std::env::join_paths(provider_path)
                .context("building Provider component PATH")?
                .to_string_lossy()
                .into_owned(),
        );
        let command = runtime_command(&package_path, &payload.launch_command)?;
        Ok(ProviderLaunchContext {
            package_path,
            command: command.display().to_string(),
            version: package.manifest.version.clone(),
            behavior: package.manifest.runtime.behavior.clone(),
            environment,
            remove_environment: package.manifest.runtime.remove_environment.clone(),
            remove_environment_prefixes: package
                .manifest
                .runtime
                .remove_environment_prefixes
                .clone(),
            home,
        })
    }

    /// Export credentials created by a temporary login executor into the
    /// Provider-declared portable bundle. The caller sends this once to the
    /// Cowboy Service; the Machine is never the source of auth generation truth.
    pub fn authentication_method(
        &self,
        provider_id: &str,
        method_id: &str,
    ) -> Result<cowboy_provider_sdk::AuthMethod> {
        let (package, _) = self
            .active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        package
            .manifest
            .authentication
            .methods
            .into_iter()
            .find(|method| method.id == method_id)
            .context("authentication method is not declared by the active Provider")
    }

    pub fn authentication_component_command(
        &self,
        provider_id: &str,
        component: &cowboy_provider_sdk::AuthComponent,
    ) -> Result<PathBuf> {
        let (package, digest) = self
            .active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        let payload = matching_payload(&package, &self.platform, &self.architecture)?;
        let requirement = payload
            .private_components
            .iter()
            .find(|candidate| candidate.kind == component.kind && candidate.slot == component.slot)
            .context("authentication component is not exported by the active Provider")?;
        let (_, package_path) = self.package_for_generation(provider_id, &digest)?;
        runtime_command(&package_path, &requirement.command)
    }

    pub fn export_auth_candidate(
        &self,
        provider_id: &str,
        method_id: &str,
        home: &Path,
    ) -> Result<ExportedAuthCandidate> {
        let (package, generation_digest) = self
            .active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        let auth = &package.manifest.authentication;
        let method = auth
            .methods
            .iter()
            .find(|method| method.id == method_id)
            .context("authentication method is not declared by the active Provider")?;
        let provider_version = package.manifest.version.clone();
        let auth_contract_fingerprint = package
            .manifest
            .compatibility
            .auth_contract_fingerprint
            .clone();
        let portable_schema = auth.portable_schema.clone();
        let candidate_root = self.auth_candidate_root(provider_id);
        ensure_within(&candidate_root, home)?;
        let mut values = BTreeMap::new();
        let mut total = 0_usize;
        for credential in &auth.credential_files {
            if !method.required_bundle_keys.contains(&credential.bundle_key) {
                continue;
            }
            let path = home.join(&credential.relative_path);
            ensure_within(home, &path)?;
            match fs::read(&path) {
                Ok(bytes) => {
                    ensure!(
                        bytes.len() <= MAX_CREDENTIAL_VALUE_BYTES,
                        "credential value is too large"
                    );
                    total = total.saturating_add(bytes.len());
                    ensure!(
                        total <= MAX_CREDENTIAL_BUNDLE_BYTES,
                        "credential bundle is too large"
                    );
                    values.insert(
                        credential.bundle_key.clone(),
                        base64::engine::general_purpose::STANDARD.encode(bytes),
                    );
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && !credential.required => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    bail!(
                        "login completed without required credential {}",
                        credential.bundle_key
                    )
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("reading {}", path.display()));
                }
            }
        }
        for (name, bundle_key) in &auth.environment_projection {
            if !method.required_bundle_keys.contains(bundle_key) {
                continue;
            }
            if let Ok(value) = std::env::var(name) {
                values.insert(
                    bundle_key.clone(),
                    base64::engine::general_purpose::STANDARD.encode(value.as_bytes()),
                );
            }
        }
        Ok(ExportedAuthCandidate {
            provider_version,
            generation_digest,
            auth_contract_fingerprint,
            bundle: PortableCredentialBundle {
                portable_schema,
                method_id: method_id.to_owned(),
                values,
            },
        })
        .and_then(|candidate| {
            validate_portable_bundle(auth, &candidate.bundle)?;
            Ok(candidate)
        })
    }

    pub fn auth_candidate_from_secret(
        &self,
        provider_id: &str,
        method_id: &str,
        secret: &str,
    ) -> Result<ExportedAuthCandidate> {
        let secret = secret.trim();
        ensure!(!secret.is_empty(), "authentication secret is empty");
        ensure!(
            secret.len() <= MAX_CREDENTIAL_VALUE_BYTES && !secret.contains(['\0', '\r', '\n']),
            "authentication secret is invalid"
        );
        let (package, generation_digest) = self
            .active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        let method = package
            .manifest
            .authentication
            .methods
            .iter()
            .find(|method| method.id == method_id)
            .context("authentication method is not declared by the active Provider")?;
        let cowboy_provider_sdk::AuthExecutor::SecretInputV1 { bundle_key, .. } = &method.executor
        else {
            bail!("authentication method does not accept a secret")
        };
        let bundle = PortableCredentialBundle {
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            method_id: method_id.to_owned(),
            values: BTreeMap::from([(
                bundle_key.clone(),
                base64::engine::general_purpose::STANDARD.encode(secret.as_bytes()),
            )]),
        };
        validate_portable_bundle(&package.manifest.authentication, &bundle)?;
        Ok(ExportedAuthCandidate {
            provider_version: package.manifest.version.clone(),
            generation_digest,
            auth_contract_fingerprint: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            bundle,
        })
    }

    pub fn prepare_auth_candidate_home(
        &self,
        provider_id: &str,
        request_id: &str,
    ) -> Result<PathBuf> {
        self.active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        validate_auth_candidate_id(request_id)?;
        let root = self.auth_candidate_root(provider_id);
        fs::create_dir_all(&root)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        self.prune_auth_candidates(provider_id, AUTH_CANDIDATE_MAX_AGE)?;
        let candidate = root.join(request_id);
        ensure!(
            !candidate.exists(),
            "authentication candidate already exists"
        );
        let home = candidate.join("home");
        fs::create_dir_all(&home)?;
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700))?;
        Ok(home)
    }

    pub fn discard_auth_candidate(&self, provider_id: &str, request_id: &str) -> Result<()> {
        validate_auth_candidate_id(request_id)?;
        let candidate = self.auth_candidate_root(provider_id).join(request_id);
        ensure_within(&self.auth_candidate_root(provider_id), &candidate)?;
        match fs::remove_dir_all(&candidate) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("removing {}", candidate.display())),
        }
    }

    /// Remove only the isolated temporary login home after the Service has
    /// durably accepted and redistributed its credential generation.
    pub fn finalize_auth_candidate(
        &self,
        provider_id: &str,
        method_id: &str,
        request_id: &str,
    ) -> Result<()> {
        let (package, _) = self
            .active_package(provider_id)?
            .context("Provider is no longer installed")?;
        let method = package
            .manifest
            .authentication
            .methods
            .iter()
            .find(|method| method.id == method_id)
            .context("authentication method is not declared by the active Provider")?;
        if !matches!(
            method.executor,
            cowboy_provider_sdk::AuthExecutor::CommandV1 { .. }
        ) {
            return Ok(());
        }
        self.discard_auth_candidate(provider_id, request_id)
    }

    pub async fn apply_auth(
        &self,
        envelope: &SealedProviderAuth,
    ) -> Result<ProviderInventoryReceipt> {
        let _lifecycle = self.lifecycle.lock().await;
        validate_provider_id(&envelope.provider_id)?;
        ensure!(
            envelope.envelope_schema == 1,
            "unsupported auth envelope schema"
        );
        let signature_valid = crate::machine_auth::verify_namespaced(
            &envelope.service_public_key,
            crate::machine_auth::PROVIDER_AUTH_SIGNATURE_NAMESPACE,
            &envelope.proof(),
            &envelope.signature,
        )?;
        ensure!(
            signature_valid,
            "Service auth envelope signature is invalid"
        );
        self.pin_service_key(&envelope.service_public_key)?;
        let provider_root = self.auth_provider_root(&envelope.provider_id);
        let previous = self.latest_auth_envelope(&envelope.provider_id)?;
        validate_auth_replica_transition(previous.as_ref(), envelope)?;
        fs::create_dir_all(provider_root.join("replicas"))?;
        fs::set_permissions(&provider_root, fs::Permissions::from_mode(0o700))?;
        atomic_write(
            &provider_root
                .join("replicas")
                .join(format!("{}.sealed.json", envelope.auth_generation)),
            &serde_json::to_vec(envelope)?,
            0o600,
        )?;
        atomic_write(
            &provider_root.join("replica-current.json"),
            &serde_json::to_vec(envelope)?,
            0o600,
        )?;

        let package = self
            .active_package(&envelope.provider_id)?
            .map(|value| value.0);
        let materialization = self.apply_auth_inner(envelope, package.as_ref())?;
        prune_auth_replicas(&provider_root.join("replicas"), envelope.auth_generation)?;
        Ok(ProviderInventoryReceipt {
            provider_id: envelope.provider_id.clone(),
            auth_generation: envelope.auth_generation,
            replica_state: ProviderReplicaState::Current,
            materialization_state: materialization,
        })
    }

    fn apply_auth_inner(
        &self,
        envelope: &SealedProviderAuth,
        package: Option<&ProviderPackage>,
    ) -> Result<ProviderMaterializationState> {
        let prepared = self.prepare_auth_inner(envelope, package)?;
        self.commit_prepared_auth(envelope, package, prepared)
    }

    fn prepare_auth_inner(
        &self,
        envelope: &SealedProviderAuth,
        package: Option<&ProviderPackage>,
    ) -> Result<PreparedProviderAuth> {
        if envelope.action == ProviderAuthAction::Wipe {
            return Ok(PreparedProviderAuth::Wipe);
        }
        let Some(package) = package else {
            return Ok(PreparedProviderAuth::NotInstalled);
        };
        ensure!(
            package.manifest.id == envelope.provider_id,
            "Provider authentication envelope targets a different Provider"
        );
        ensure!(
            package.manifest.compatibility.auth_contract_fingerprint
                == envelope.auth_contract_fingerprint,
            "installed Provider authentication contract does not match the Service generation"
        );
        ensure!(
            package.manifest.authentication.projection_schema == envelope.projection_schema,
            "installed Provider projection schema does not match the Service generation"
        );
        let plaintext = self.encryption.open(envelope)?;
        let bundle: PortableCredentialBundle =
            serde_json::from_slice(&plaintext).context("decoding portable credential bundle")?;
        validate_portable_bundle(&package.manifest.authentication, &bundle)?;
        Ok(PreparedProviderAuth::Apply(bundle))
    }

    fn commit_prepared_auth(
        &self,
        envelope: &SealedProviderAuth,
        package: Option<&ProviderPackage>,
        prepared: PreparedProviderAuth,
    ) -> Result<ProviderMaterializationState> {
        match prepared {
            PreparedProviderAuth::Wipe => {
                let materialized = self
                    .auth_provider_root(&envelope.provider_id)
                    .join("materialized");
                if materialized.exists() {
                    fs::remove_dir_all(materialized)?;
                }
                Ok(ProviderMaterializationState::NotInstalled)
            }
            PreparedProviderAuth::NotInstalled => Ok(ProviderMaterializationState::NotInstalled),
            PreparedProviderAuth::Apply(bundle) => {
                let package = package.context("Provider disappeared before auth activation")?;
                self.materialize_bundle(package, envelope.auth_generation, &bundle)?;
                Ok(ProviderMaterializationState::Current)
            }
        }
    }

    // Materialization is one atomic projection transaction across files,
    // environment, generation metadata, and activation links.
    #[allow(clippy::too_many_lines)]
    fn materialize_bundle(
        &self,
        package: &ProviderPackage,
        auth_generation: u64,
        bundle: &PortableCredentialBundle,
    ) -> Result<()> {
        let auth = &package.manifest.authentication;
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
        let provider_root = self.auth_provider_root(&package.manifest.id);
        let generations = provider_root.join("materialized/generations");
        fs::create_dir_all(&generations)?;
        fs::set_permissions(&generations, fs::Permissions::from_mode(0o700))?;
        let generation = generations.join(auth_generation.to_string());
        if generation.exists() {
            let metadata: MaterializationMetadata = serde_json::from_slice(
                &fs::read(generation.join("metadata.json"))
                    .context("reading existing Provider auth materialization")?,
            )?;
            ensure!(
                metadata.auth_generation == auth_generation
                    && metadata.auth_contract_fingerprint
                        == package.manifest.compatibility.auth_contract_fingerprint,
                "existing Provider auth materialization has conflicting identity"
            );
            ensure!(
                generation.join("home").is_dir() && generation.join("environment.json").is_file(),
                "existing Provider auth materialization is incomplete"
            );
            for file in auth.credential_files.iter().filter(|file| file.required) {
                ensure!(
                    generation.join("home").join(&file.relative_path).is_file(),
                    "existing Provider auth materialization is missing required value {}",
                    file.bundle_key
                );
            }
            return activate_link(
                &provider_root.join("materialized"),
                &auth_generation.to_string(),
            );
        }
        let temporary = generations.join(format!(
            ".{}.{}.{}.partial",
            auth_generation,
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| -> Result<()> {
            let home = temporary.join("home");
            fs::create_dir_all(&home)?;
            set_tree_root_permissions(&temporary)?;
            let mut total = 0_usize;
            for file in &auth.credential_files {
                let value = bundle.values.get(&file.bundle_key);
                ensure!(
                    !file.required || value.is_some(),
                    "portable credential bundle is missing required value {}",
                    file.bundle_key
                );
                let Some(value) = value else { continue };
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .with_context(|| format!("decoding credential value {}", file.bundle_key))?;
                ensure!(
                    bytes.len() <= MAX_CREDENTIAL_VALUE_BYTES,
                    "credential value too large"
                );
                total = total.saturating_add(bytes.len());
                ensure!(
                    total <= MAX_CREDENTIAL_BUNDLE_BYTES,
                    "credential bundle too large"
                );
                let destination = home.join(&file.relative_path);
                ensure_within(&home, &destination)?;
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                    set_directory_chain_permissions(&home, parent)?;
                }
                atomic_write(&destination, &bytes, 0o600)?;
            }
            let mut environment = BTreeMap::new();
            for (name, bundle_key) in &auth.environment_projection {
                let Some(value) = bundle.values.get(bundle_key) else {
                    continue;
                };
                let bytes = base64::engine::general_purpose::STANDARD.decode(value)?;
                ensure!(
                    bytes.len() <= MAX_CREDENTIAL_VALUE_BYTES,
                    "credential value too large"
                );
                let value =
                    String::from_utf8(bytes).context("environment credential is not UTF-8")?;
                ensure!(!value.contains('\0'), "environment credential contains NUL");
                environment.insert(name.clone(), value);
            }
            atomic_write(
                &temporary.join("environment.json"),
                &serde_json::to_vec(&environment)?,
                0o600,
            )?;
            let metadata = MaterializationMetadata {
                auth_generation,
                auth_contract_fingerprint: package
                    .manifest
                    .compatibility
                    .auth_contract_fingerprint
                    .clone(),
            };
            atomic_write(
                &temporary.join("metadata.json"),
                &serde_json::to_vec(&metadata)?,
                0o600,
            )?;
            fs::rename(&temporary, &generation).with_context(|| {
                format!(
                    "activating Provider auth generation {}",
                    generation.display()
                )
            })?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&temporary);
        }
        result?;
        activate_link(
            &provider_root.join("materialized"),
            &auth_generation.to_string(),
        )
    }

    fn inventory_one(&self, provider_id: &str) -> Result<Option<ProviderInventory>> {
        let Some((package, digest)) = self.active_package(provider_id)? else {
            return Ok(None);
        };
        let rollback = read_link_name(&self.provider_root(provider_id).join("rollback"))
            .map(|name| format!("sha256:{name}"));
        let replica = self.latest_auth_envelope(provider_id)?;
        let auth_generation = replica.as_ref().map(|value| value.auth_generation);
        let replica_state = if replica.is_some() {
            ProviderReplicaState::Current
        } else {
            ProviderReplicaState::Absent
        };
        let materialization_state =
            auth_generation.map_or(ProviderMaterializationState::NotInstalled, |generation| {
                let current = self
                    .auth_provider_root(provider_id)
                    .join("materialized/current");
                let metadata = fs::read(current.join("metadata.json"))
                    .ok()
                    .and_then(|bytes| {
                        serde_json::from_slice::<MaterializationMetadata>(&bytes).ok()
                    });
                if metadata.is_some_and(|value| value.auth_generation == generation) {
                    ProviderMaterializationState::Current
                } else {
                    ProviderMaterializationState::Failed
                }
            });
        Ok(Some(ProviderInventory {
            provider_id: package.manifest.id.clone(),
            provider_version: package.manifest.version.clone(),
            generation_digest: digest,
            contract_fingerprint: package.contract_fingerprint.clone(),
            state: ProviderInstallationState::Active,
            rollback_generation_digest: rollback,
            active_session_leases: 0,
            auth_generation,
            replica_state,
            materialization_state,
            detail: None,
        }))
    }

    fn active_package(&self, provider_id: &str) -> Result<Option<(ProviderPackage, String)>> {
        let active = self.provider_root(provider_id).join("active");
        let Some(generation) = read_link_name(&active) else {
            return Ok(None);
        };
        let digest = format!("sha256:{generation}");
        self.package_for_generation(provider_id, &digest)
            .map(|(package, _)| Some((package, digest)))
    }

    fn package_for_generation(
        &self,
        provider_id: &str,
        digest: &str,
    ) -> Result<(ProviderPackage, PathBuf)> {
        let (package, _, path) = self.verified_generation(provider_id, digest)?;
        Ok((package, path))
    }

    fn verified_generation(
        &self,
        provider_id: &str,
        digest: &str,
    ) -> Result<(
        ProviderPackage,
        cowboy_provider_sdk::ProviderRelease,
        PathBuf,
    )> {
        let generation = digest_generation_name(digest)?;
        let path = self
            .provider_root(provider_id)
            .join("generations")
            .join(generation)
            .join("content/package.cowboy-provider");
        let bytes = fs::read(&path)
            .with_context(|| format!("reading Provider generation {}", path.display()))?;
        let release: cowboy_provider_sdk::ProviderRelease = serde_json::from_slice(
            &fs::read(path.with_file_name("release.json"))
                .context("reading stored Provider release")?,
        )?;
        ensure!(
            release.artifact_digest == digest,
            "stored Provider release generation digest mismatch"
        );
        let package = release.validate_bytes(&bytes)?;
        ensure!(
            package.manifest.id == provider_id,
            "stored Provider id mismatch"
        );
        let publisher_key = fs::read_to_string(path.with_file_name("publisher.pub"))?;
        ensure!(
            crate::machine_auth::verify_namespaced(
                &publisher_key,
                crate::machine_auth::PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                &release.proof(),
                &release.signature,
            )?,
            "stored Provider publisher signature is invalid"
        );
        Ok((package, release, path))
    }

    fn activate(provider_root: &Path, generation_name: &str) -> Result<ProviderActivationSnapshot> {
        fs::create_dir_all(provider_root.join("generations"))?;
        let active = provider_root.join("active");
        let snapshot = ProviderActivationSnapshot {
            active: read_link_name(&active),
            rollback: read_link_name(&provider_root.join("rollback")),
        };
        let result = (|| -> Result<()> {
            if let Some(previous) = snapshot.active.as_deref()
                && previous != generation_name
            {
                replace_link(
                    &provider_root.join("rollback"),
                    &format!("generations/{previous}"),
                )?;
            }
            replace_link(&active, &format!("generations/{generation_name}"))
        })();
        if let Err(error) = result {
            if let Err(rollback_error) = Self::restore_activation(provider_root, &snapshot) {
                bail!(
                    "Provider generation activation failed: {error:#}; restoring activation links also failed: {rollback_error:#}"
                );
            }
            return Err(error);
        }
        Ok(snapshot)
    }

    fn restore_activation(
        provider_root: &Path,
        snapshot: &ProviderActivationSnapshot,
    ) -> Result<()> {
        restore_generation_link(&provider_root.join("active"), snapshot.active.as_deref())?;
        restore_generation_link(
            &provider_root.join("rollback"),
            snapshot.rollback.as_deref(),
        )
    }

    fn pin_publisher(&self, publisher: &str, public_key: &str) -> Result<()> {
        let trust = self.root.join("trust");
        fs::create_dir_all(&trust)?;
        fs::set_permissions(&trust, fs::Permissions::from_mode(0o700))?;
        pin_key(&trust.join(format!("{publisher}.pub")), public_key)
    }

    fn pin_service_key(&self, public_key: &str) -> Result<()> {
        pin_key(&self.auth_root.join("service.pub"), public_key)
    }

    fn latest_auth_envelope(&self, provider_id: &str) -> Result<Option<SealedProviderAuth>> {
        let path = self
            .auth_provider_root(provider_id)
            .join("replica-current.json");
        if !path.is_file() {
            return Ok(None);
        }
        serde_json::from_slice(&fs::read(&path)?)
            .context("decoding current Provider auth replica")
            .map(Some)
    }

    fn provider_root(&self, provider_id: &str) -> PathBuf {
        self.root.join(provider_id)
    }

    fn auth_provider_root(&self, provider_id: &str) -> PathBuf {
        self.auth_root.join("providers").join(provider_id)
    }

    fn auth_candidate_root(&self, provider_id: &str) -> PathBuf {
        self.auth_provider_root(provider_id).join("candidates")
    }

    fn prune_auth_candidates(&self, provider_id: &str, maximum_age: Duration) -> Result<()> {
        let root = self.auth_candidate_root(provider_id);
        let now = SystemTime::now();
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.file_type().is_dir()
                || now
                    .duration_since(metadata.modified().unwrap_or(now))
                    .unwrap_or_default()
                    < maximum_age
            {
                continue;
            }
            fs::remove_dir_all(entry.path()).with_context(|| {
                format!("removing expired auth candidate {}", entry.path().display())
            })?;
        }
        Ok(())
    }
}

fn validate_auth_candidate_id(request_id: &str) -> Result<()> {
    ensure!(
        !request_id.is_empty()
            && request_id.len() <= 160
            && request_id
                .bytes()
                .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') }),
        "invalid authentication candidate id"
    );
    Ok(())
}

fn validate_provider_id(provider_id: &str) -> Result<()> {
    ensure!(
        !provider_id.is_empty()
            && provider_id.len() <= 128
            && provider_id != "."
            && provider_id != ".."
            && provider_id.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b'.')
            }),
        "invalid Provider id"
    );
    Ok(())
}

fn validate_auth_replica_transition(
    previous: Option<&SealedProviderAuth>,
    incoming: &SealedProviderAuth,
) -> Result<()> {
    let Some(previous) = previous else {
        return Ok(());
    };
    ensure!(
        incoming.auth_generation >= previous.auth_generation,
        "stale Service auth generation"
    );
    if incoming.auth_generation == previous.auth_generation {
        ensure!(
            incoming.provider_id == previous.provider_id
                && incoming.auth_contract_fingerprint == previous.auth_contract_fingerprint
                && incoming.projection_schema == previous.projection_schema
                && incoming.action == previous.action,
            "conflicting Service auth envelope for the current generation"
        );
    }
    Ok(())
}

fn prune_auth_replicas(root: &Path, current_generation: u64) -> Result<()> {
    let current_name = format!("{current_generation}.sealed.json");
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file()
            || path.file_name().and_then(|value| value.to_str()) == Some(current_name.as_str())
        {
            continue;
        }
        fs::remove_file(&path)
            .with_context(|| format!("removing retired auth replica {}", path.display()))?;
    }
    Ok(())
}

fn validate_portable_bundle(
    auth: &cowboy_provider_sdk::AuthenticationContract,
    bundle: &PortableCredentialBundle,
) -> Result<()> {
    ensure!(
        bundle.portable_schema == auth.portable_schema,
        "portable credential schema mismatch"
    );
    let method = auth
        .methods
        .iter()
        .find(|method| method.id == bundle.method_id)
        .context("portable credential bundle references an unknown authentication method")?;
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
    ensure!(
        method
            .required_bundle_keys
            .iter()
            .all(|key| bundle.values.contains_key(key)),
        "portable credential bundle is missing a method-required value"
    );
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderInventoryReceipt {
    pub provider_id: String,
    pub auth_generation: u64,
    pub replica_state: ProviderReplicaState,
    pub materialization_state: ProviderMaterializationState,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct MaterializationMetadata {
    auth_generation: u64,
    auth_contract_fingerprint: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledRuntimeMetadata {
    schema_version: u16,
    commands: BTreeMap<String, InstalledRuntimeCommand>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledRuntimeCommand {
    executable: String,
    artifact: String,
    artifact_digest: String,
}

struct StagedRuntimeComponent {
    executable: PathBuf,
    artifact: PathBuf,
}

pub(crate) fn provider_auth_aad(envelope: &SealedProviderAuth) -> Vec<u8> {
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

pub(crate) fn derive_seal_key(shared: &[u8; 32]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(AUTH_SEAL_DOMAIN);
    digest.update(shared);
    digest.finalize().into()
}

fn matching_payload<'a>(
    package: &'a ProviderPackage,
    platform: &Platform,
    architecture: &str,
) -> Result<&'a cowboy_provider_sdk::PlatformPayload> {
    let os = match platform {
        Platform::Linux => OperatingSystem::Linux,
        Platform::Macos => OperatingSystem::Macos,
    };
    let architecture = match architecture {
        "x86_64" => Architecture::X86_64,
        "aarch64" => Architecture::Aarch64,
        other => bail!("unsupported Machine architecture {other:?}"),
    };
    package
        .manifest
        .runtime
        .platforms
        .iter()
        .find(|payload| payload.os == os && payload.architecture == architecture)
        .context("Provider has no payload for this Machine platform")
}

fn matching_runtime_artifacts<'a>(
    release: &'a cowboy_provider_sdk::ProviderRelease,
    platform: &Platform,
    architecture: &str,
) -> Result<&'a PlatformRuntimeArtifacts> {
    let os = match platform {
        Platform::Linux => OperatingSystem::Linux,
        Platform::Macos => OperatingSystem::Macos,
    };
    let architecture = match architecture {
        "x86_64" => Architecture::X86_64,
        "aarch64" => Architecture::Aarch64,
        other => bail!("unsupported Machine architecture {other:?}"),
    };
    release
        .runtime_artifacts
        .iter()
        .find(|artifacts| artifacts.os == os && artifacts.architecture == architecture)
        .context("Provider release has no runtime artifacts for this Machine platform")
}

async fn stage_provider_runtime(
    content: &Path,
    artifacts: &PlatformRuntimeArtifacts,
) -> Result<InstalledRuntimeMetadata> {
    let active = content.join("runtime");
    if active.exists() {
        let metadata = read_installed_runtime(content)
            .context("existing Provider runtime metadata is invalid")?;
        ensure!(
            installed_runtime_matches(content, artifacts, &metadata)?,
            "existing content-addressed Provider runtime failed integrity verification"
        );
        return Ok(metadata);
    }
    let temporary = content.join(format!(
        ".runtime.{}.{}.partial",
        std::process::id(),
        ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = async {
        fs::create_dir_all(&temporary)?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700))?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::new();
        let mut commands = BTreeMap::new();
        for artifact in &artifacts.components {
            let staged = stage_runtime_component(&client, &temporary, artifact).await?;
            let executable = staged
                .executable
                .strip_prefix(&temporary)
                .context("staged Provider runtime escaped its staging directory")?;
            let stored_artifact = staged
                .artifact
                .strip_prefix(&temporary)
                .context("staged Provider artifact escaped its staging directory")?;
            let installed = InstalledRuntimeCommand {
                executable: Path::new("runtime")
                    .join(executable)
                    .to_string_lossy()
                    .into_owned(),
                artifact: Path::new("runtime")
                    .join(stored_artifact)
                    .to_string_lossy()
                    .into_owned(),
                artifact_digest: artifact.artifact_digest.to_ascii_lowercase(),
            };
            ensure!(
                commands
                    .insert(artifact.command.clone(), installed)
                    .is_none(),
                "duplicate staged Provider command"
            );
        }
        let metadata = InstalledRuntimeMetadata {
            schema_version: 2,
            commands,
        };
        atomic_write(
            &temporary.join("metadata.json"),
            &serde_json::to_vec(&metadata)?,
            0o600,
        )?;
        fs::rename(&temporary, &active)?;
        Ok(metadata)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

fn installed_runtime_matches(
    content: &Path,
    artifacts: &PlatformRuntimeArtifacts,
    metadata: &InstalledRuntimeMetadata,
) -> Result<bool> {
    if metadata.commands.len() != artifacts.components.len() {
        return Ok(false);
    }
    for expected in &artifacts.components {
        let Some(installed) = metadata.commands.get(&expected.command) else {
            return Ok(false);
        };
        if installed.artifact_digest != expected.artifact_digest.to_ascii_lowercase() {
            return Ok(false);
        }
        let executable = content.join(&installed.executable);
        let artifact = content.join(&installed.artifact);
        ensure_within(content, &executable)?;
        ensure_within(content, &artifact)?;
        if !executable.is_file()
            || !artifact.is_file()
            || digest_file(&artifact)? != expected.artifact_digest.to_ascii_lowercase()
        {
            return Ok(false);
        }
        if expected.artifact_format == ProviderArtifactFormat::TarGz {
            let entrypoint = expected
                .entrypoint
                .as_deref()
                .context("released Provider archive has no entrypoint")?;
            let expected_executable = archive_entry_digest(&artifact, entrypoint)?;
            if digest_file_with_limit(
                &executable,
                MAX_PROVIDER_RUNTIME_EXPANDED_BYTES,
                "installed Provider runtime entrypoint",
            )? != expected_executable
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn digest_file(path: &Path) -> Result<String> {
    digest_file_with_limit(
        path,
        MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES as u64,
        "stored Provider runtime artifact",
    )
}

fn digest_file_with_limit(path: &Path, limit: u64, label: &str) -> Result<String> {
    let metadata = fs::metadata(path)?;
    ensure!(metadata.len() <= limit, "{label} exceeds its size limit");
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn archive_entry_digest(archive_path: &Path, entrypoint: &str) -> Result<String> {
    let decoder = flate2::read::GzDecoder::new(fs::File::open(archive_path)?);
    let mut archive = tar::Archive::new(decoder);
    let expected = Path::new(entrypoint);
    let mut found = None;
    let mut expanded_bytes = 0_u64;
    let mut entries = 0_usize;
    let mut paths = BTreeSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        ensure!(
            path.components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
            "Provider runtime archive contains an unsafe path"
        );
        ensure!(
            paths.insert(path.clone()),
            "Provider runtime archive contains a duplicate path"
        );
        entries = entries.saturating_add(1);
        expanded_bytes = expanded_bytes.saturating_add(entry.header().size()?);
        ensure!(
            entries <= MAX_PROVIDER_RUNTIME_ARCHIVE_ENTRIES
                && expanded_bytes <= MAX_PROVIDER_RUNTIME_EXPANDED_BYTES,
            "Provider runtime archive exceeds extraction limits"
        );
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "Provider runtime archive contains an unsupported entry type"
        );
        if path == expected {
            ensure!(kind.is_file(), "Provider runtime entrypoint is not a file");
            let mut digest = Sha256::new();
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                let read = entry.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            found = Some(format!("sha256:{:x}", digest.finalize()));
        }
    }
    found.context("Provider runtime archive entrypoint is missing")
}

async fn stage_runtime_component(
    client: &reqwest::Client,
    runtime_root: &Path,
    artifact: &ReleasedPrivateComponent,
) -> Result<StagedRuntimeComponent> {
    let url = reqwest::Url::parse(&artifact.artifact_url)?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "::1" | "localhost"));
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "Provider runtime artifact must use HTTPS"
    );
    let response = client.get(url).send().await?.error_for_status()?;
    if let Some(length) = response.content_length() {
        ensure!(
            length <= MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES as u64,
            "Provider runtime artifact exceeds 1 GiB"
        );
    }
    let capacity = usize::try_from(
        response
            .content_length()
            .unwrap_or_default()
            .min(MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES as u64),
    )
    .context("Provider runtime artifact capacity does not fit this platform")?;
    let mut bytes = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("downloading Provider runtime artifact")?;
        let next_size = bytes
            .len()
            .checked_add(chunk.len())
            .context("Provider runtime artifact size overflow")?;
        ensure!(
            next_size <= MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES,
            "Provider runtime artifact exceeds 1 GiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    ensure!(
        digest == artifact.artifact_digest.to_ascii_lowercase(),
        "Provider runtime artifact digest mismatch"
    );
    let component_root = runtime_root.join(format!("{}-{}", artifact.kind.as_str(), artifact.slot));
    fs::create_dir_all(&component_root)?;
    let (executable, stored_artifact) = match artifact.artifact_format {
        ProviderArtifactFormat::Raw => {
            let executable = component_root.join("bin");
            atomic_write(&executable, &bytes, 0o700)?;
            (executable.clone(), executable)
        }
        ProviderArtifactFormat::TarGz => {
            let stored_artifact = component_root.join("artifact.tar.gz");
            atomic_write(&stored_artifact, &bytes, 0o600)?;
            let archive_root = component_root.join("content");
            fs::create_dir_all(&archive_root)?;
            extract_provider_tar_gz(&archive_root, &bytes)?;
            let entrypoint = artifact
                .entrypoint
                .as_deref()
                .context("Provider runtime archive has no entrypoint")?;
            let executable = archive_root.join(entrypoint);
            ensure_within(&archive_root, &executable)?;
            ensure!(
                executable.is_file(),
                "Provider runtime entrypoint is missing"
            );
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
            (executable, stored_artifact)
        }
    };
    probe_released_component(&executable, artifact).await?;
    Ok(StagedRuntimeComponent {
        executable,
        artifact: stored_artifact,
    })
}

fn extract_provider_tar_gz(destination: &Path, bytes: &[u8]) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(false);
    let mut expanded_bytes = 0_u64;
    let mut entries = 0_usize;
    let mut paths = BTreeSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        let path = entry.path()?.into_owned();
        ensure!(
            path.components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
            "Provider runtime archive contains an unsafe path"
        );
        ensure!(
            paths.insert(path),
            "Provider runtime archive contains a duplicate path"
        );
        entries = entries.saturating_add(1);
        expanded_bytes = expanded_bytes.saturating_add(entry.header().size()?);
        ensure!(
            entries <= MAX_PROVIDER_RUNTIME_ARCHIVE_ENTRIES
                && expanded_bytes <= MAX_PROVIDER_RUNTIME_EXPANDED_BYTES,
            "Provider runtime archive exceeds extraction limits"
        );
        ensure!(
            kind.is_file() || kind.is_dir(),
            "Provider runtime archive contains an unsupported entry type"
        );
        ensure!(
            entry.unpack_in(destination)?,
            "Provider runtime archive contains an unsafe path"
        );
    }
    Ok(())
}

async fn probe_released_component(
    executable: &Path,
    artifact: &ReleasedPrivateComponent,
) -> Result<()> {
    let mut child = tokio::process::Command::new(executable)
        .args(&artifact.probe.args)
        .current_dir(
            executable
                .parent()
                .context("Provider executable has no parent")?,
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("starting staged Provider component probe")?;
    let status = if let Ok(status) = tokio::time::timeout(
        Duration::from_millis(artifact.probe.timeout_ms),
        child.wait(),
    )
    .await
    {
        status.context("waiting for staged Provider component probe")?
    } else {
        let _ = child.kill().await;
        bail!("staged Provider component probe timed out");
    };
    ensure!(
        status.success(),
        "staged Provider component probe exited {status}"
    );
    Ok(())
}

fn runtime_command(package_path: &Path, command: &str) -> Result<PathBuf> {
    let content = package_path
        .parent()
        .context("Provider package has no generation content directory")?;
    let metadata = read_installed_runtime(content)?;
    ensure!(
        metadata.schema_version == 2,
        "unsupported installed Provider runtime metadata"
    );
    let installed = metadata
        .commands
        .get(command)
        .with_context(|| format!("installed Provider runtime does not export {command:?}"))?;
    let executable = content.join(&installed.executable);
    ensure_within(content, &executable)?;
    ensure!(
        executable.is_file(),
        "installed Provider command is missing"
    );
    Ok(executable)
}

fn read_installed_runtime(content: &Path) -> Result<InstalledRuntimeMetadata> {
    let metadata_path = content.join("runtime/metadata.json");
    let metadata: InstalledRuntimeMetadata = serde_json::from_slice(
        &fs::read(&metadata_path)
            .with_context(|| format!("reading {}", metadata_path.display()))?,
    )?;
    ensure!(
        metadata.schema_version == 2,
        "unsupported installed Provider runtime metadata"
    );
    Ok(metadata)
}

async fn probe_provider_runtime(runtime: &RuntimeContract, command: &Path) -> Result<()> {
    ensure!(
        runtime.protocol == "agent-client-protocol-1.3",
        "unsupported Provider driver protocol"
    );
    let output = tokio::time::timeout(
        PROVIDER_PROBE_TIMEOUT,
        tokio::process::Command::new(command)
            .arg("--version")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Provider staged probe timed out")?
    .with_context(|| format!("starting Provider entrypoint {}", command.display()))?;
    ensure!(
        output.status.success(),
        "Provider staged probe exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

fn digest_generation_name(digest: &str) -> Result<String> {
    let value = digest
        .strip_prefix("sha256:")
        .context("Provider digest must use sha256")?;
    ensure!(
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Provider digest is malformed"
    );
    Ok(value.to_ascii_lowercase())
}

fn pin_key(path: &Path, value: &str) -> Result<()> {
    let normalized = crate::machine_auth::validate_public_key(value)?;
    if path.exists() {
        let current = crate::machine_auth::validate_public_key(&fs::read_to_string(path)?)?;
        ensure!(
            current == normalized,
            "trusted signing key rotation requires re-enrollment"
        );
        return Ok(());
    }
    atomic_write(path, normalized.as_bytes(), 0o600)
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

fn replace_link(path: &Path, target: &str) -> Result<()> {
    let parent = path.parent().context("link has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.next",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("link"),
        std::process::id()
    ));
    if temporary.symlink_metadata().is_ok() {
        fs::remove_file(&temporary)?;
    }
    symlink(target, &temporary)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn restore_generation_link(path: &Path, generation: Option<&str>) -> Result<()> {
    if let Some(generation) = generation {
        return replace_link(path, &format!("generations/{generation}"));
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

fn activate_link(materialized_root: &Path, generation: &str) -> Result<()> {
    replace_link(
        &materialized_root.join("current"),
        &format!("generations/{generation}"),
    )
}

fn read_link_name(path: &Path) -> Option<String> {
    let target = fs::read_link(path).ok()?;
    target.file_name()?.to_str().map(str::to_owned)
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

fn ensure_within(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .context("credential path escapes projection root")?;
    ensure!(
        relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "credential path is unsafe"
    );
    Ok(())
}

fn set_tree_root_permissions(path: &Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    fs::set_permissions(path.join("home"), fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn set_directory_chain_permissions(root: &Path, leaf: &Path) -> Result<()> {
    let mut current = leaf;
    loop {
        fs::set_permissions(current, fs::Permissions::from_mode(0o700))?;
        if current == root {
            break;
        }
        current = current
            .parent()
            .context("credential directory escaped root")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // This end-to-end test intentionally retains the complete signed install,
    // runtime, auth, uninstall, and rollback story in one fixture.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn signed_provider_installs_exact_runtime_and_uninstalls_as_one_unit() {
        use cowboy_provider_sdk::{
            PlatformRuntimeArtifacts, PlatformTarget, ProviderArtifactFormat,
            ProviderArtifactProbe, ProviderRelease, RELEASE_SCHEMA_VERSION,
            ReleasedPrivateComponent, StandardProviderSource, build_package,
        };
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let script = b"#!/bin/sh\nexit 0\n".to_vec();
        let script_digest = format!("sha256:{:x}", Sha256::digest(&script));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let served_script = script.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4_096];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        served_script.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            stream.write_all(&served_script).await.unwrap();
        });

        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../providers/gemini/provider.json")).unwrap();
        let package = build_package(source.compile().unwrap()).unwrap();
        let bytes = package.canonical_bytes().unwrap();
        let runtime_artifacts = package
            .manifest
            .runtime
            .platforms
            .iter()
            .map(|payload| PlatformRuntimeArtifacts {
                os: payload.os.clone(),
                architecture: payload.architecture.clone(),
                components: payload
                    .private_components
                    .iter()
                    .map(|requirement| ReleasedPrivateComponent {
                        kind: requirement.kind.clone(),
                        slot: requirement.slot.clone(),
                        dependency: requirement.dependency.clone(),
                        version: package.manifest.runtime.dependencies[0].version.clone(),
                        command: requirement.command.clone(),
                        artifact_url: format!("http://{address}/runtime"),
                        artifact_digest: script_digest.clone(),
                        artifact_format: ProviderArtifactFormat::Raw,
                        entrypoint: None,
                        probe: ProviderArtifactProbe {
                            args: vec!["--version".to_owned()],
                            timeout_ms: 1_000,
                        },
                    })
                    .collect(),
            })
            .collect();
        let mut release = ProviderRelease {
            release_schema: RELEASE_SCHEMA_VERSION,
            provider_id: package.manifest.id.clone(),
            provider_version: package.manifest.version.clone(),
            package_digest: ProviderPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: "https://example.invalid/gemini.cowboy-provider".to_owned(),
            publisher: package.manifest.publisher.clone(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            signature: String::new(),
            supported_platforms: package
                .manifest
                .runtime
                .platforms
                .iter()
                .map(|payload| PlatformTarget {
                    os: payload.os.clone(),
                    architecture: payload.architecture.clone(),
                })
                .collect(),
            runtime_artifacts,
        };
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        let root = std::env::temp_dir().join(format!(
            "cowboy-provider-install-test-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("publisher")).unwrap();
        release.signature = publisher
            .sign_namespaced(
                crate::machine_auth::PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                &release.proof(),
            )
            .unwrap();
        let desired = DesiredProvider {
            release: release.clone(),
            package_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            publisher_public_key: publisher.public_key().to_owned(),
        };
        let store =
            MachineProviderStore::new(&root.join("machine"), Platform::Linux, "x86_64".to_owned())
                .unwrap();
        let installed = store.install(&desired).await.unwrap();
        assert_eq!(installed.generation_digest, release.artifact_digest);
        let command = store
            .authentication_component_command(
                "gemini",
                &cowboy_provider_sdk::AuthComponent {
                    kind: cowboy_provider_sdk::PrivateComponentKind::ProviderCli,
                    slot: "gemini".to_owned(),
                },
            )
            .unwrap();
        assert!(command.starts_with(root.join("machine/providers/gemini/generations")));
        assert!(command.is_file());
        store.install(&desired).await.unwrap();
        atomic_write(&command, b"#!/bin/sh\nexit 0\n# tampered\n", 0o700).unwrap();
        let error = store.install(&desired).await.unwrap_err();
        assert!(error.to_string().contains("integrity verification"));
        atomic_write(&command, &script, 0o700).unwrap();
        store
            .uninstall("gemini", &release.artifact_digest)
            .await
            .unwrap();
        assert!(store.inventory().unwrap().is_empty());
        atomic_write(&command, b"#!/bin/sh\nexit 0\n# tampered again\n", 0o700).unwrap();
        let error = store
            .reactivate("gemini", &release.artifact_digest)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("integrity verification"));
        assert!(store.inventory().unwrap().is_empty());
        atomic_write(&command, &script, 0o700).unwrap();
        let restored = store
            .reactivate("gemini", &release.artifact_digest)
            .await
            .unwrap();
        assert_eq!(restored.generation_digest, release.artifact_digest);
        store
            .uninstall("gemini", &release.artifact_digest)
            .await
            .unwrap();
        assert!(store.inventory().unwrap().is_empty());
        server.await.unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn failed_runtime_staging_removes_partial_generation() {
        use cowboy_provider_sdk::{
            Architecture, OperatingSystem, PlatformRuntimeArtifacts, PrivateComponentKind,
            ProviderArtifactFormat, ProviderArtifactProbe, ReleasedPrivateComponent,
        };

        let root = std::env::temp_dir().join(format!(
            "cowboy-provider-runtime-failure-test-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let artifacts = PlatformRuntimeArtifacts {
            os: OperatingSystem::Linux,
            architecture: Architecture::X86_64,
            components: vec![ReleasedPrivateComponent {
                kind: PrivateComponentKind::ProviderCli,
                slot: "fixture".to_owned(),
                dependency: "fixture".to_owned(),
                version: "1.0.0".to_owned(),
                command: "fixture".to_owned(),
                artifact_url: "file:///tmp/not-a-provider-runtime".to_owned(),
                artifact_digest: format!("sha256:{}", "00".repeat(32)),
                artifact_format: ProviderArtifactFormat::Raw,
                entrypoint: None,
                probe: ProviderArtifactProbe {
                    args: Vec::new(),
                    timeout_ms: 1_000,
                },
            }],
        };

        let error = stage_provider_runtime(&root, &artifacts).await.unwrap_err();
        assert!(error.to_string().contains("must use HTTPS"));
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".runtime.")
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_runtime_revalidation_binds_the_extracted_entrypoint() {
        use cowboy_provider_sdk::{
            Architecture, OperatingSystem, PlatformRuntimeArtifacts, PrivateComponentKind,
            ProviderArtifactProbe,
        };
        use flate2::Compression;
        use flate2::write::GzEncoder;

        let script = b"#!/bin/sh\nexit 0\n";
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o700);
        header.set_size(script.len() as u64);
        header.set_cksum();
        builder
            .append_data(&mut header, "bin/fixture", script.as_slice())
            .unwrap();
        builder.finish().unwrap();
        let archive_bytes = builder.into_inner().unwrap().finish().unwrap();
        let archive_digest = format!("sha256:{:x}", Sha256::digest(&archive_bytes));

        let root = std::env::temp_dir().join(format!(
            "cowboy-provider-runtime-archive-test-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let component = root.join("runtime/provider_cli-fixture");
        let archive = component.join("artifact.tar.gz");
        let extracted = component.join("content");
        fs::create_dir_all(&extracted).unwrap();
        atomic_write(&archive, &archive_bytes, 0o600).unwrap();
        extract_provider_tar_gz(&extracted, &archive_bytes).unwrap();
        let executable = extracted.join("bin/fixture");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();

        let artifacts = PlatformRuntimeArtifacts {
            os: OperatingSystem::Linux,
            architecture: Architecture::X86_64,
            components: vec![ReleasedPrivateComponent {
                kind: PrivateComponentKind::ProviderCli,
                slot: "fixture".to_owned(),
                dependency: "fixture".to_owned(),
                version: "1.0.0".to_owned(),
                command: "fixture".to_owned(),
                artifact_url: "https://example.invalid/fixture.tar.gz".to_owned(),
                artifact_digest: archive_digest.clone(),
                artifact_format: ProviderArtifactFormat::TarGz,
                entrypoint: Some("bin/fixture".to_owned()),
                probe: ProviderArtifactProbe {
                    args: Vec::new(),
                    timeout_ms: 1_000,
                },
            }],
        };
        let metadata = InstalledRuntimeMetadata {
            schema_version: 2,
            commands: BTreeMap::from([(
                "fixture".to_owned(),
                InstalledRuntimeCommand {
                    executable: "runtime/provider_cli-fixture/content/bin/fixture".to_owned(),
                    artifact: "runtime/provider_cli-fixture/artifact.tar.gz".to_owned(),
                    artifact_digest: archive_digest,
                },
            )]),
        };
        assert!(installed_runtime_matches(&root, &artifacts, &metadata).unwrap());
        atomic_write(&executable, b"#!/bin/sh\nexit 1\n", 0o700).unwrap();
        assert!(!installed_runtime_matches(&root, &artifacts, &metadata).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generation_names_are_content_addressed_and_path_safe() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        assert_eq!(digest_generation_name(&digest).unwrap(), "ab".repeat(32));
        assert!(digest_generation_name("sha256:../../active").is_err());
        assert!(digest_generation_name(&format!("sha512:{}", "ab".repeat(32))).is_err());
        assert!(validate_provider_id(".").is_err());
        assert!(validate_provider_id("..").is_err());
    }

    #[test]
    fn auth_replicas_never_move_backwards_or_change_meaning() {
        let envelope = |generation, action| SealedProviderAuth {
            envelope_schema: 1,
            provider_id: "gemini".to_owned(),
            auth_generation: generation,
            auth_contract_fingerprint: format!("sha256:{}", "ab".repeat(32)),
            projection_schema: "gemini-auth-v1".to_owned(),
            action,
            ephemeral_public_key: String::new(),
            nonce: String::new(),
            ciphertext: String::new(),
            service_public_key: String::new(),
            signature: String::new(),
        };
        let current = envelope(2, ProviderAuthAction::Apply);
        assert!(
            validate_auth_replica_transition(
                Some(&current),
                &envelope(1, ProviderAuthAction::Apply)
            )
            .is_err()
        );
        assert!(
            validate_auth_replica_transition(
                Some(&current),
                &envelope(2, ProviderAuthAction::Wipe)
            )
            .is_err()
        );
        assert!(
            validate_auth_replica_transition(
                Some(&current),
                &envelope(3, ProviderAuthAction::Wipe)
            )
            .is_ok()
        );
    }

    #[test]
    fn x25519_identity_is_durable_and_private() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-provider-encryption-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let first = MachineEncryptionIdentity::load_or_create(&root).unwrap();
        let second = MachineEncryptionIdentity::load_or_create(&root).unwrap();
        assert_eq!(first.public_key(), second.public_key());
        assert_eq!(
            fs::metadata(root.join("identity_x25519"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }
}
