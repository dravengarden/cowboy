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
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _, symlink};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use cowboy_plugin_sdk::{
    PLUGIN_RELEASE_SIGNATURE_NAMESPACE, PluginArtifactFormat, PluginComponentKind, PluginPackage,
    PluginPayload, PluginRuntimeArtifacts,
};
use cowboy_provider_sdk::{
    AgentRuntimeBinding, Architecture, OperatingSystem, PlatformRuntimeArtifacts,
    PrivateComponentKind, ProviderArtifactFormat, ProviderPackage, RefreshOwnership,
    ReleasedPrivateComponent, RuntimeContract, RuntimeSidecar, RuntimeSidecarTransport,
};
use futures::StreamExt as _;
use sha2::{Digest as _, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::legacy_provider_release::LegacyProviderRelease;
use crate::machine_auth::LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE;
use crate::machine_code_plugins::{
    CodeLaunchPlan, CodeRuntimeHost, CodeRuntimeSelection, probe_code_runtime,
};
use crate::machine_protocol::{
    DesiredPlugin, Platform, PluginHostOperation, PluginInstallationState, PluginInventory,
    PortableCredentialBundle, ProviderAuthAction, ProviderMaterializationState,
    ProviderReplicaState, SealedProviderAuth,
};
use crate::plugin_host::{PluginHostSpec, PluginUsageSidecar};
use crate::plugin_host_bundle::{MAX_HOST_BUNDLE_BYTES, PluginHostBundle};

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

#[derive(Debug)]
pub(crate) struct PluginHostInvocationFailure {
    pub started: bool,
    pub error: anyhow::Error,
}

impl From<anyhow::Error> for PluginHostInvocationFailure {
    fn from(error: anyhow::Error) -> Self {
        Self {
            started: false,
            error,
        }
    }
}

struct PreparedUsageSidecars {
    children: Vec<tokio::process::Child>,
    targets: Vec<serde_json::Value>,
}

struct ResolvedPluginHostInvocation {
    command: Vec<String>,
    environment: BTreeMap<String, String>,
    collector_sidecars: Vec<PluginUsageSidecar>,
}

pub(crate) struct ExportedAuthCandidate {
    pub provider_version: String,
    pub generation_digest: String,
    pub auth_contract_fingerprint: String,
    pub bundle: PortableCredentialBundle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAuthRefreshCandidate {
    pub provider_id: String,
    pub expected_generation: u64,
    pub provider_version: String,
    pub generation_digest: String,
    pub auth_contract_fingerprint: String,
    pub bundle: PortableCredentialBundle,
}

#[derive(Debug, Default)]
pub(crate) struct ProviderAuthRefreshObservations {
    pub candidates: Vec<ProviderAuthRefreshCandidate>,
    pub failed_provider_ids: BTreeSet<String>,
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
        ensure!(
            shared.as_bytes().iter().any(|byte| *byte != 0),
            "Service ephemeral encryption key is non-contributory"
        );
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

pub(crate) struct MachinePluginStore {
    root: PathBuf,
    auth_root: PathBuf,
    platform: Platform,
    architecture: String,
    encryption: MachineEncryptionIdentity,
    lifecycle: tokio::sync::Mutex<()>,
    telemetry_export: tokio::sync::Mutex<()>,
    code_runtimes: CodeRuntimeHost,
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

fn ensure_machine_installable_kind(package: &PluginPackage) -> Result<()> {
    ensure!(
        matches!(
            package.manifest.kind,
            cowboy_plugin_sdk::PluginKind::AgentProvider
                | cowboy_plugin_sdk::PluginKind::CodeIntelligence
                | cowboy_plugin_sdk::PluginKind::TelemetryBackend
        ),
        "Machine installer received a Controller-only Plugin kind"
    );
    Ok(())
}

impl MachinePluginStore {
    pub fn new(state_dir: &Path, platform: Platform, architecture: String) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = state_dir.join("plugins");
        let legacy_root = state_dir.join("providers");
        if legacy_root.is_dir() && !root.exists() {
            fs::rename(&legacy_root, &root).with_context(|| {
                format!(
                    "migrating legacy Provider generations into {}",
                    root.display()
                )
            })?;
        }
        let auth_root = state_dir.join("provider-auth");
        let auth_providers_root = auth_root.join("providers");
        for directory in [&root, &auth_root, &auth_providers_root] {
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
            telemetry_export: tokio::sync::Mutex::new(()),
            code_runtimes: CodeRuntimeHost::default(),
        })
    }

    #[must_use]
    pub fn encryption_public_key(&self) -> &str {
        self.encryption.public_key()
    }

    fn checked_install_package(&self, desired: &DesiredPlugin) -> Result<PluginPackage> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&desired.package_base64)
            .context("decoding Plugin package")?;
        ensure!(
            bytes.len() <= MAX_PROVIDER_PACKAGE_BYTES,
            "Plugin package exceeds 8 MiB"
        );
        let plugin_package = desired.release.validate_bytes(&bytes)?;
        ensure_machine_installable_kind(&plugin_package)?;
        if let Some(active) = self.inventory_one(&plugin_package.manifest.id)? {
            ensure!(
                active.plugin_kind == plugin_package.manifest.kind,
                "installed Plugin identity cannot change capability kind"
            );
        }
        Ok(plugin_package)
    }

    pub async fn install(&self, desired: &DesiredPlugin) -> Result<PluginInventory> {
        let _lifecycle = self.lifecycle.lock().await;
        let plugin_package = self.checked_install_package(desired)?;
        let (host_bundle, host_bundle_bytes) =
            desired_plugin_host_bundle(desired, &plugin_package)?;
        let signature_valid = crate::machine_auth::verify_namespaced(
            &desired.publisher_public_key,
            PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
            &desired.release.proof(),
            &desired.release.signature,
        )?;
        ensure!(signature_valid, "Plugin publisher signature is invalid");
        self.pin_publisher(
            &plugin_package.manifest.publisher,
            &desired.publisher_public_key,
        )?;
        if matches!(
            plugin_package.payload,
            PluginPayload::CodeIntelligence(_) | PluginPayload::TelemetryBackend(_)
        ) {
            return self
                .install_non_agent_plugin(
                    &plugin_package,
                    desired,
                    host_bundle.as_ref(),
                    host_bundle_bytes.as_deref(),
                )
                .await;
        }
        let package = plugin_package
            .agent_provider()
            .context("Machine Agent installer received a non-Agent Plugin")?
            .clone();
        let provider_release = desired.release.agent_provider_binding(&plugin_package)?;
        let payload = matching_payload(&package, &self.platform, &self.architecture)?;
        let runtime_artifacts =
            matching_runtime_artifacts(&provider_release, &self.platform, &self.architecture)?;
        let auth_envelope = self.latest_auth_envelope(&package.manifest.id)?;
        let prepared_auth = auth_envelope
            .as_ref()
            .map(|envelope| self.prepare_auth_inner(envelope, Some(&package)))
            .transpose()?;

        let plugin_root = self.plugin_root(&package.manifest.id);
        let generation_name = digest_generation_name(&desired.release.artifact_digest)?;
        let generation = plugin_root.join("generations").join(&generation_name);
        let content = generation.join("content");
        fs::create_dir_all(&content)
            .with_context(|| format!("creating Provider generation {}", generation.display()))?;
        fs::set_permissions(&generation, fs::Permissions::from_mode(0o700))?;
        atomic_write(
            &content.join("package.cowboy-provider"),
            &package.canonical_bytes()?,
            0o600,
        )?;
        atomic_write(
            &content.join("package.cowboy-plugin"),
            &plugin_package.canonical_bytes()?,
            0o600,
        )?;
        atomic_write(
            &content.join("plugin-release.json"),
            &serde_json::to_vec(&desired.release)?,
            0o600,
        )?;
        atomic_write(
            &content.join("publisher.pub"),
            desired.publisher_public_key.as_bytes(),
            0o600,
        )?;
        stage_plugin_host_bundle(&content, host_bundle.as_ref(), host_bundle_bytes.as_deref())?;

        let runtime = stage_provider_runtime(&content, runtime_artifacts).await?;
        let launch_command = runtime
            .commands
            .get(&payload.launch_command)
            .context("staged Provider runtime does not export its launch command")?;
        let launch_command = content.join(&launch_command.executable);
        ensure_within(&content, &launch_command)?;
        probe_provider_runtime(&package.manifest.runtime, &launch_command).await?;
        let activation = Self::activate(&plugin_root, &generation_name)?;
        // A sealed Service replica may predate installation. Materialize it as
        // part of activation so installation never asks for another login.
        if let (Some(envelope), Some(prepared)) = (auth_envelope.as_ref(), prepared_auth)
            && let Err(error) = self.commit_prepared_auth(envelope, Some(&package), prepared)
        {
            if let Err(rollback_error) = Self::restore_activation(&plugin_root, &activation) {
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

    async fn install_non_agent_plugin(
        &self,
        package: &PluginPackage,
        desired: &DesiredPlugin,
        host_bundle: Option<&PluginHostBundle>,
        host_bundle_bytes: Option<&[u8]>,
    ) -> Result<PluginInventory> {
        let artifacts = matching_plugin_runtime_artifacts(
            &desired.release.runtime_artifacts,
            &self.platform,
            &self.architecture,
        )?;
        if matches!(package.payload, PluginPayload::CodeIntelligence(_)) {
            ensure!(
                artifacts.components.iter().any(|component| {
                    component.kind == PluginComponentKind::CodeIntelligenceAdapter
                }),
                "code-intelligence Plugin has no adapter component"
            );
        }
        let runtime_artifacts = provider_staging_projection(artifacts);
        let plugin_root = self.plugin_root(&package.manifest.id);
        let generation_name = digest_generation_name(&desired.release.artifact_digest)?;
        let generation = plugin_root.join("generations").join(&generation_name);
        let content = generation.join("content");
        fs::create_dir_all(&content)
            .with_context(|| format!("creating Plugin generation {}", generation.display()))?;
        fs::set_permissions(&generation, fs::Permissions::from_mode(0o700))?;
        atomic_write(
            &content.join("package.cowboy-plugin"),
            &package.canonical_bytes()?,
            0o600,
        )?;
        atomic_write(
            &content.join("plugin-release.json"),
            &serde_json::to_vec(&desired.release)?,
            0o600,
        )?;
        atomic_write(
            &content.join("publisher.pub"),
            desired.publisher_public_key.as_bytes(),
            0o600,
        )?;
        stage_plugin_host_bundle(&content, host_bundle, host_bundle_bytes)?;
        stage_provider_runtime(&content, &runtime_artifacts).await?;
        if matches!(package.payload, PluginPayload::CodeIntelligence(_))
            && let Some(plan) =
                self.code_launch_plan(package, &desired.release.artifact_digest, &content)?
        {
            probe_code_runtime(&plan).await?;
        }
        let inventory = PluginInventory {
            plugin_id: package.manifest.id.clone(),
            plugin_version: package.manifest.version.clone(),
            plugin_kind: package.manifest.kind,
            generation_digest: desired.release.artifact_digest.clone(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            state: PluginInstallationState::Active,
            rollback_generation_digest: None,
            active_session_leases: 0,
            auth_generation: None,
            replica_state: ProviderReplicaState::Absent,
            materialization_state: ProviderMaterializationState::NotInstalled,
            detail: None,
        };
        atomic_write(
            &content.join("plugin-inventory.json"),
            &serde_json::to_vec(&inventory)?,
            0o600,
        )?;
        if matches!(package.payload, PluginPayload::CodeIntelligence(_)) {
            self.activate_code_generation(package, &generation_name)?;
        } else {
            Self::activate(&plugin_root, &generation_name)?;
        }
        self.inventory_one(&package.manifest.id)?
            .context("activated Plugin is missing from inventory")
    }

    /// Re-activate one already verified, retained generation. The Controller
    /// uses this only as uninstall-saga compensation when its durable session
    /// transaction fails after the Machine has removed the active link.
    pub async fn reactivate(
        &self,
        provider_id: &str,
        generation_digest: &str,
    ) -> Result<PluginInventory> {
        let _lifecycle = self.lifecycle.lock().await;
        validate_plugin_id(provider_id)?;
        let generation_name = digest_generation_name(generation_digest)?;
        let (plugin_package, _, content) =
            self.verified_plugin_generation(provider_id, generation_digest)?;
        if plugin_package.manifest.kind == cowboy_plugin_sdk::PluginKind::TelemetryBackend {
            Self::activate(&self.plugin_root(provider_id), &generation_name)?;
            return self
                .inventory_one(provider_id)?
                .context("reactivated telemetry Plugin is missing from inventory");
        }
        if plugin_package.manifest.kind == cowboy_plugin_sdk::PluginKind::CodeIntelligence {
            if let Some(plan) =
                self.code_launch_plan(&plugin_package, generation_digest, &content)?
            {
                probe_code_runtime(&plan).await?;
            }
            self.activate_code_generation(&plugin_package, &generation_name)?;
            return self
                .inventory_one(provider_id)?
                .context("reactivated Plugin is missing from inventory");
        }
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
        let plugin_root = self.plugin_root(provider_id);
        let activation = Self::activate(&plugin_root, &generation_name)?;
        if let (Some(envelope), Some(prepared)) = (auth_envelope.as_ref(), prepared_auth)
            && let Err(error) = self.commit_prepared_auth(envelope, Some(&package), prepared)
        {
            if let Err(rollback_error) = Self::restore_activation(&plugin_root, &activation) {
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
        validate_plugin_id(provider_id)?;
        let active = self
            .inventory_one(provider_id)?
            .context("Plugin is not installed")?;
        ensure!(
            active.generation_digest == expected_digest,
            "active Plugin generation changed; refresh the uninstall plan"
        );
        let plugin_root = self.plugin_root(provider_id);
        let active_link = plugin_root.join("active");
        if active_link.exists() || active_link.symlink_metadata().is_ok() {
            fs::remove_file(&active_link)
                .with_context(|| format!("removing {}", active_link.display()))?;
        }
        if active.plugin_kind != cowboy_plugin_sdk::PluginKind::AgentProvider {
            return Ok(());
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
        let runtime = self.auth_provider_root(provider_id).join("runtime");
        if runtime.exists() {
            fs::remove_dir_all(&runtime).with_context(|| {
                format!(
                    "removing Provider runtime credential projections {}",
                    runtime.display()
                )
            })?;
        }
        Ok(())
    }

    pub fn inventory(&self) -> Result<Vec<PluginInventory>> {
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
        output.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
        Ok(output)
    }

    fn activate_code_generation(&self, package: &PluginPackage, generation: &str) -> Result<()> {
        let root = self.plugin_root(&package.manifest.id);
        let marker = root.join(".code-runtime-owned-v2");
        let owned = matches!(&package.payload, PluginPayload::CodeIntelligence(contract) if contract.runtime.is_some());
        ensure!(
            owned || !marker.exists(),
            "cannot replace an owned code runtime with a legacy adapter"
        );
        let had_marker = marker.exists();
        if owned && !had_marker {
            atomic_write(&marker, b"2\n", 0o600)?;
        }
        if let Err(error) = Self::activate(&root, generation) {
            if owned && !had_marker {
                fs::remove_file(&marker)?;
            }
            return Err(error);
        }
        Ok(())
    }

    fn code_launch_plan(
        &self,
        package: &PluginPackage,
        digest: &str,
        content: &Path,
    ) -> Result<Option<CodeLaunchPlan>> {
        let PluginPayload::CodeIntelligence(contract) = &package.payload else {
            bail!("installed Plugin is not a code-intelligence engine");
        };
        let Some(runtime) = &contract.runtime else {
            return Ok(None);
        };
        let commands = runtime
            .components
            .iter()
            .map(|component| {
                Ok((
                    component.command.clone(),
                    runtime_command(&content.join("package.cowboy-plugin"), &component.command)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(Some(CodeLaunchPlan {
            plugin_id: package.manifest.id.clone(),
            generation_digest: digest.to_owned(),
            runtime: runtime.clone(),
            commands,
            home: self
                .plugin_root(&package.manifest.id)
                .join("runtime")
                .join(digest_generation_name(digest)?)
                .join("home"),
        }))
    }

    /// Resolve the selected Plugin by capability id, never a global server
    /// executable. Existing routes keep their immutable generation on uninstall.
    pub async fn code_request(
        &self,
        plugin_id: &str,
        payload: &serde_json::Value,
        legacy_socket: Option<&Path>,
    ) -> Result<serde_json::Value> {
        validate_plugin_id(plugin_id)?;
        self.code_runtimes
            .request(plugin_id, payload, || {
                let root = self.plugin_root(plugin_id);
                if let Some(generation) = read_link_name(&root.join("active")) {
                    let digest = format!("sha256:{generation}");
                    let (package, _, content) =
                        self.verified_plugin_generation(plugin_id, &digest)?;
                    if let Some(plan) = self.code_launch_plan(&package, &digest, &content)? {
                        return Ok(CodeRuntimeSelection::Installed(plan));
                    }
                }
                ensure!(
                    !root.join(".code-runtime-owned-v2").exists(),
                    "code-intelligence Plugin is not installed; legacy fallback is disabled"
                );
                Ok(CodeRuntimeSelection::Legacy(
                    legacy_socket
                        .context("code-intelligence Plugin is not installed")?
                        .to_path_buf(),
                ))
            })
            .await
    }

    async fn export_telemetry(
        &self,
        plugin_id: &str,
        plugin_version: &str,
        generation_digest: &str,
        auth_generation: Option<u64>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let _export = self
            .telemetry_export
            .try_lock()
            .map_err(|_| anyhow::anyhow!("telemetry exporter is busy"))?;
        // No Provider home, auth generation, executable or usage sidecar.
        // Verification uses the lifecycle lock; bounded network I/O does
        // not hold it or block installation/agent operations.
        let (contract, config, payload) = {
            let _lifecycle = self.lifecycle.lock().await;
            (|| -> Result<_> {
                ensure!(
                    auth_generation.is_none(),
                    "telemetry cannot use Provider credentials"
                );
                let active = self
                    .inventory_one(plugin_id)?
                    .context("telemetry Plugin is not installed")?;
                ensure!(
                    active.state == PluginInstallationState::Active
                        && active.plugin_version == plugin_version
                        && active.generation_digest == generation_digest
                        && active.plugin_kind == cowboy_plugin_sdk::PluginKind::TelemetryBackend,
                    "active telemetry Plugin generation mismatch"
                );
                let (package, _, _) =
                    self.verified_plugin_generation(plugin_id, generation_digest)?;
                let PluginPayload::TelemetryBackend(contract) = package.payload else {
                    anyhow::bail!("Plugin is not a telemetry backend");
                };
                let selection = crate::telemetry_plugin::PluginSelection {
                    plugin_id: plugin_id.to_owned(),
                    plugin_version: plugin_version.to_owned(),
                    generation_digest: generation_digest.to_owned(),
                };
                let (config, payload) = crate::telemetry_plugin::prepare(
                    &self
                        .root
                        .parent()
                        .context("Machine state directory is missing")?
                        .join("telemetry.json"),
                    &selection,
                    payload,
                )?;
                Ok((contract, config, payload))
            })()?
        };
        serde_json::to_value(crate::telemetry_plugin::export(&contract, config, payload).await)
            .map_err(anyhow::Error::from)
    }

    pub async fn invoke_host(
        &self,
        plugin_id: &str,
        plugin_version: &str,
        generation_digest: &str,
        auth_generation: Option<u64>,
        operation: PluginHostOperation,
        mut payload: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, PluginHostInvocationFailure> {
        if operation == PluginHostOperation::ExportTelemetry {
            return self
                .export_telemetry(
                    plugin_id,
                    plugin_version,
                    generation_digest,
                    auth_generation,
                    payload,
                )
                .await
                .map_err(PluginHostInvocationFailure::from);
        }
        let _lifecycle = self.lifecycle.lock().await;
        let resolved = self
            .resolve_host_invocation(
                plugin_id,
                plugin_version,
                generation_digest,
                auth_generation,
                operation,
                &mut payload,
            )
            .map_err(PluginHostInvocationFailure::from)?;
        let mut environment = resolved.environment;
        let mut prepared_sidecars = if operation == PluginHostOperation::CollectUsage {
            self.prepare_usage_sidecars(&resolved.collector_sidecars)
                .await
                .map_err(PluginHostInvocationFailure::from)?
        } else {
            PreparedUsageSidecars {
                children: Vec::new(),
                targets: Vec::new(),
            }
        };
        if !prepared_sidecars.targets.is_empty() {
            environment.insert(
                "COWBOY_PLUGIN_SIDECAR_TARGETS".to_owned(),
                serde_json::to_string(&prepared_sidecars.targets)
                    .map_err(anyhow::Error::from)
                    .map_err(PluginHostInvocationFailure::from)?,
            );
            environment.insert(
                "COWBOY_PLUGIN_SIDECAR_URLS".to_owned(),
                prepared_sidecars
                    .targets
                    .iter()
                    .filter_map(|target| target.get("url").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        let (program, command_args) = resolved
            .command
            .split_first()
            .context("Plugin host operation has no executable")
            .map_err(PluginHostInvocationFailure::from)?;
        let input = serde_json::to_vec(&payload)
            .map_err(anyhow::Error::from)
            .map_err(PluginHostInvocationFailure::from)?;
        let output = crate::plugin_process::run_plugin_command_with_environment(
            program,
            command_args,
            &input,
            &environment,
        )
        .await;
        for child in &mut prepared_sidecars.children {
            let _ = child.kill().await;
        }
        let output = output.map_err(|failure| PluginHostInvocationFailure {
            started: failure.started,
            error: failure.error,
        })?;
        if !output.status.success() {
            return Err(PluginHostInvocationFailure {
                started: true,
                error: anyhow::anyhow!(
                    "Plugin host command exited {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }
        serde_json::from_slice(&output.stdout)
            .context("parsing Plugin host command response")
            .map_err(|error| PluginHostInvocationFailure {
                started: true,
                error,
            })
    }

    fn resolve_host_invocation(
        &self,
        plugin_id: &str,
        plugin_version: &str,
        generation_digest: &str,
        auth_generation: Option<u64>,
        operation: PluginHostOperation,
        payload: &mut serde_json::Value,
    ) -> Result<ResolvedPluginHostInvocation> {
        validate_plugin_id(plugin_id)?;
        let active = self
            .inventory_one(plugin_id)?
            .context("Plugin is not active on this Machine")?;
        ensure!(
            active.state == PluginInstallationState::Active
                && active.plugin_version == plugin_version
                && active.generation_digest == generation_digest
                && active.auth_generation == auth_generation,
            "active Plugin or authentication generation changed before host invocation"
        );
        let (plugin, release, content) =
            self.verified_plugin_generation(plugin_id, generation_digest)?;
        ensure!(
            plugin.manifest.version == plugin_version,
            "stored Plugin version does not match host invocation"
        );
        let host = verified_plugin_host_bundle(&plugin, &release, &content)?
            .context("exact Plugin generation has no signed host bundle")?;
        let spec = PluginHostSpec::from_json(host.files["host.json"].as_bytes())
            .context("decoding installed Plugin host contract")?;
        let usage = spec
            .usage
            .context("installed Plugin host has no usage capability")?;
        let (command, operation_name) = match operation {
            PluginHostOperation::CollectUsage => (&usage.collector_argv, "collect"),
            PluginHostOperation::ResetUsage => (&usage.reset_argv, "consume_reset"),
            PluginHostOperation::DecorateActivity => (&usage.collector_argv, "decorate_activity"),
            PluginHostOperation::ExportTelemetry => {
                bail!("telemetry cannot use executable Plugin hosts")
            }
        };
        ensure!(!command.is_empty(), "Plugin host operation has no command");
        payload
            .as_object_mut()
            .context("Plugin host invocation payload must be an object")?
            .insert(
                "operation".to_owned(),
                serde_json::Value::String(operation_name.to_owned()),
            );
        plugin
            .agent_provider()
            .context("usage host is not attached to an Agent Provider")?;
        let launch = self.launch_context(plugin_id, generation_digest, auth_generation)?;
        let host_root = content.join("host").to_string_lossy().into_owned();
        Ok(ResolvedPluginHostInvocation {
            command: command
                .iter()
                .map(|argument| argument.replace("${PLUGIN_DIR}", &host_root))
                .collect(),
            environment: plugin_host_environment(&launch)?,
            collector_sidecars: usage.collector_sidecars,
        })
    }

    async fn prepare_usage_sidecars(
        &self,
        targets: &[PluginUsageSidecar],
    ) -> Result<PreparedUsageSidecars> {
        let inventory = self.inventory()?;
        let mut prepared = PreparedUsageSidecars {
            children: Vec::new(),
            targets: Vec::new(),
        };
        for target in targets {
            let mut candidates = Vec::new();
            for installed in &inventory {
                let Ok((plugin, release, content)) = self
                    .verified_plugin_generation(&installed.plugin_id, &installed.generation_digest)
                else {
                    continue;
                };
                let Ok(Some(host)) = verified_plugin_host_bundle(&plugin, &release, &content)
                else {
                    continue;
                };
                let Ok(spec) = PluginHostSpec::from_json(host.files["host.json"].as_bytes()) else {
                    continue;
                };
                if spec.adapter_slot.as_deref() != Some(target.adapter_slot.as_str()) {
                    continue;
                }
                let Some(package) = plugin.agent_provider() else {
                    continue;
                };
                let Some(sidecar) = package
                    .manifest
                    .runtime
                    .sidecars
                    .iter()
                    .find(|sidecar| sidecar.id == target.sidecar)
                    .cloned()
                else {
                    continue;
                };
                candidates.push((installed, package.clone(), sidecar));
            }
            ensure!(
                candidates.len() <= 1,
                "multiple active Plugin generations satisfy usage sidecar {:?}",
                target.id
            );
            let Some((installed, package, sidecar)) = candidates.pop() else {
                continue;
            };
            let launch = match self.launch_context(
                &installed.plugin_id,
                &installed.generation_digest,
                installed.auth_generation,
            ) {
                Ok(launch) => launch,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        plugin_id = installed.plugin_id,
                        lane = target.id,
                        "usage sidecar launch context is unavailable"
                    );
                    continue;
                }
            };
            match start_usage_sidecar(
                &package,
                &sidecar,
                &launch,
                &self.platform,
                &self.architecture,
            )
            .await
            {
                Ok((child, base_url)) => {
                    prepared.children.push(child);
                    prepared.targets.push(serde_json::json!({
                        "id": target.id,
                        "url": format!("{base_url}{}", target.path),
                    }));
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        plugin_id = installed.plugin_id,
                        lane = target.id,
                        "usage sidecar is unavailable"
                    );
                }
            }
        }
        Ok(prepared)
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
            Some(self.prepare_launch_auth_home(provider_id, &package, generation)?)
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

    fn prepare_launch_auth_home(
        &self,
        provider_id: &str,
        package: &ProviderPackage,
        generation: u64,
    ) -> Result<PathBuf> {
        let auth = &package.manifest.authentication;
        let envelope = self
            .latest_auth_envelope(provider_id)?
            .context("session Provider auth replica is missing")?;
        ensure!(
            envelope.auth_generation >= generation,
            "session Provider auth generation is ahead of the Machine replica"
        );
        ensure!(
            envelope.action == ProviderAuthAction::Apply,
            "session Provider auth replica has no credentials"
        );
        ensure!(
            envelope.auth_contract_fingerprint
                == package.manifest.compatibility.auth_contract_fingerprint
                && envelope.projection_schema == auth.projection_schema,
            "session Provider auth replica uses a different contract"
        );
        let plaintext = self.encryption.open(&envelope)?;
        let bundle: PortableCredentialBundle = serde_json::from_slice(&plaintext)
            .context("decoding Provider runtime credential bundle")?;
        validate_portable_bundle(auth, &bundle)?;
        let materialized = self
            .auth_provider_root(provider_id)
            .join("materialized/generations")
            .join(generation.to_string());
        if envelope.auth_generation == generation {
            // A Machine upgrade or interrupted projection may leave the
            // durable sealed replica ahead of its local projections. Repair
            // the current immutable materialization before admitting the
            // session so recovery does not depend on a later Controller retry
            // or filesystem event.
            self.materialize_bundle(package, generation, &bundle)?;
            ensure!(
                self.materialization_is_current(package, &envelope)?,
                "session Provider auth generation is missing projected credentials"
            );
        } else {
            // Established sessions retain their recorded projection identity,
            // while successful Service refreshes reconcile the writable
            // runtime copy to the newest credential bundle. Rebuild only that
            // historical runtime directory: activating its immutable
            // materialization would move `current` backwards for new sessions.
            validate_materialization_metadata(&materialized, package, generation)
                .context("validating historical Provider auth materialization")?;
            self.ensure_runtime_projection(package, generation, &bundle)?;
        }
        let runtime = self
            .auth_provider_root(provider_id)
            .join("runtime/generations")
            .join(generation.to_string());
        let migrated_entries =
            migrate_legacy_runtime_state(auth, &materialized.join("home"), &runtime.join("home"))
                .context("migrating legacy Provider runtime state")?;
        if migrated_entries > 0 {
            tracing::info!(
                provider = %provider_id,
                auth_generation = generation,
                migrated_entries,
                "migrated legacy Provider state into the writable runtime projection"
            );
        }
        validate_materialization_metadata(&runtime, package, generation)
            .context("validating Provider runtime credential projection")?;
        projected_credential_bundle(auth, &runtime, &bundle.method_id)
            .context("validating Provider runtime credentials")?;
        Ok(runtime.join("home"))
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
    ) -> Result<PluginInventoryReceipt> {
        let _lifecycle = self.lifecycle.lock().await;
        validate_plugin_id(&envelope.provider_id)?;
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
        let provider_auth_root = self.auth_provider_root(&envelope.provider_id);
        let previous = self.latest_auth_envelope(&envelope.provider_id)?;
        validate_auth_replica_transition(previous.as_ref(), envelope)?;
        let auth_generation_advanced = previous
            .as_ref()
            .is_none_or(|previous| previous.auth_generation < envelope.auth_generation);
        fs::create_dir_all(provider_auth_root.join("replicas"))?;
        fs::set_permissions(&provider_auth_root, fs::Permissions::from_mode(0o700))?;
        atomic_write(
            &provider_auth_root
                .join("replicas")
                .join(format!("{}.sealed.json", envelope.auth_generation)),
            &serde_json::to_vec(envelope)?,
            0o600,
        )?;
        atomic_write(
            &provider_auth_root.join("replica-current.json"),
            &serde_json::to_vec(envelope)?,
            0o600,
        )?;

        let package = self
            .active_package(&envelope.provider_id)?
            .map(|value| value.0);
        let materialization = self.apply_auth_inner(envelope, package.as_ref())?;
        prune_auth_replicas(
            &provider_auth_root.join("replicas"),
            envelope.auth_generation,
        )?;
        Ok(PluginInventoryReceipt {
            provider_id: envelope.provider_id.clone(),
            auth_generation: envelope.auth_generation,
            auth_generation_advanced,
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
                let provider_auth_root = self.auth_provider_root(&envelope.provider_id);
                for directory in ["materialized", "runtime"] {
                    let path = provider_auth_root.join(directory);
                    if path.exists() {
                        fs::remove_dir_all(path)?;
                    }
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

    // The sealed Service materialization is immutable. Provider processes use
    // a separate writable runtime projection so token rotation can be observed
    // without letting runtime state silently redefine a signed generation.
    fn materialize_bundle(
        &self,
        package: &ProviderPackage,
        auth_generation: u64,
        bundle: &PortableCredentialBundle,
    ) -> Result<()> {
        let auth = &package.manifest.authentication;
        validate_portable_bundle(auth, bundle)?;
        let provider_auth_root = self.auth_provider_root(&package.manifest.id);
        let materialized_root = provider_auth_root.join("materialized");
        let previous_generation = read_link_name(&materialized_root.join("current"))
            .and_then(|value| value.parse::<u64>().ok());
        let generations = materialized_root.join("generations");
        fs::create_dir_all(&generations)?;
        fs::set_permissions(&generations, fs::Permissions::from_mode(0o700))?;
        let generation = generations.join(auth_generation.to_string());
        let runtime_generation = provider_auth_root
            .join("runtime/generations")
            .join(auth_generation.to_string());
        let mut migrated_legacy_refresh = false;
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
            // Protocol-five Machines launched the Provider directly against
            // this materialization, so a successful refresh may already live
            // here when the runtime/materialization split is first deployed.
            // Preserve that complete same-generation value once, then restore
            // the materialization to the exact sealed Service bundle. The
            // protocol-six watcher will submit the migrated runtime value via
            // CAS instead of silently discarding a valid refresh on upgrade.
            if previous_generation == Some(auth_generation)
                && !runtime_generation.exists()
                && let Ok(projected) =
                    projected_credential_bundle(auth, &generation, &bundle.method_id)
                && projected != *bundle
            {
                self.ensure_runtime_projection(package, auth_generation, &projected)?;
                migrated_legacy_refresh = true;
            }
            restore_projected_bundle(auth, &generation, bundle)?;
            ensure!(
                materialization_contains_bundle(auth, &generation, bundle),
                "repaired Provider auth materialization is incomplete"
            );
        } else {
            let temporary = generations.join(format!(
                ".{}.{}.{}.partial",
                auth_generation,
                std::process::id(),
                ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let result = (|| -> Result<()> {
                fs::create_dir_all(&temporary)?;
                restore_projected_bundle(auth, &temporary, bundle)?;
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
        }
        activate_link(&materialized_root, &auth_generation.to_string())?;
        if !migrated_legacy_refresh
            && previous_generation.is_none_or(|previous| previous < auth_generation)
        {
            self.reconcile_runtime_projections(package, bundle)?;
        } else {
            self.repair_runtime_projections(package, bundle)?;
        }
        self.ensure_runtime_projection(package, auth_generation, bundle)
    }

    fn ensure_runtime_projection(
        &self,
        package: &ProviderPackage,
        auth_generation: u64,
        bundle: &PortableCredentialBundle,
    ) -> Result<()> {
        let generations = self
            .auth_provider_root(&package.manifest.id)
            .join("runtime/generations");
        fs::create_dir_all(&generations)?;
        fs::set_permissions(&generations, fs::Permissions::from_mode(0o700))?;
        let generation = generations.join(auth_generation.to_string());
        let created = !generation.exists();
        if created {
            let temporary = generations.join(format!(
                ".{}.{}.{}.partial",
                auth_generation,
                std::process::id(),
                ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let result = (|| -> Result<()> {
                fs::create_dir_all(&temporary)?;
                restore_projected_bundle(&package.manifest.authentication, &temporary, bundle)?;
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
                fs::rename(&temporary, &generation)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::remove_dir_all(&temporary);
            }
            result?;
        } else {
            validate_materialization_metadata(&generation, package, auth_generation)?;
        }

        let canonical = self
            .canonical_runtime_projection(package)?
            .context("Provider runtime projection has no credential source")?;
        if generation == canonical {
            if projected_credential_bundle(
                &package.manifest.authentication,
                &generation,
                &bundle.method_id,
            )
            .is_err()
            {
                repair_missing_projected_bundle(
                    &package.manifest.authentication,
                    &generation,
                    bundle,
                )?;
            }
        } else {
            self.link_runtime_projection_credentials(
                package,
                &generation,
                &canonical,
                bundle,
                created,
            )?;
        }
        restore_projected_environment(&package.manifest.authentication, &generation, bundle)
    }

    fn reconcile_runtime_projections(
        &self,
        package: &ProviderPackage,
        bundle: &PortableCredentialBundle,
    ) -> Result<()> {
        let Some(canonical) = self.canonical_runtime_projection(package)? else {
            return Ok(());
        };
        // Historical aliases can sort before the credential source. Restore
        // the source first so a missing file cannot prevent its own repair.
        restore_projected_bundle(&package.manifest.authentication, &canonical, bundle)?;
        for generation in self.writable_auth_projection_generations(package)? {
            let metadata = read_materialization_metadata(&generation)?;
            ensure!(
                metadata.auth_contract_fingerprint
                    == package.manifest.compatibility.auth_contract_fingerprint,
                "Provider runtime projection uses a different auth contract"
            );
            if generation != canonical {
                restore_projected_environment(
                    &package.manifest.authentication,
                    &generation,
                    bundle,
                )?;
                self.link_runtime_projection_credentials(
                    package,
                    &generation,
                    &canonical,
                    bundle,
                    true,
                )?;
            }
        }
        Ok(())
    }

    fn repair_runtime_projections(
        &self,
        package: &ProviderPackage,
        bundle: &PortableCredentialBundle,
    ) -> Result<()> {
        let Some(canonical) = self.canonical_runtime_projection(package)? else {
            return Ok(());
        };
        if projected_credential_bundle(
            &package.manifest.authentication,
            &canonical,
            &bundle.method_id,
        )
        .is_err()
        {
            repair_missing_projected_bundle(&package.manifest.authentication, &canonical, bundle)?;
        }
        for generation in self.writable_auth_projection_generations(package)? {
            let metadata = read_materialization_metadata(&generation)?;
            ensure!(
                metadata.auth_contract_fingerprint
                    == package.manifest.compatibility.auth_contract_fingerprint,
                "Provider runtime projection uses a different auth contract"
            );
            if generation != canonical {
                restore_projected_environment(
                    &package.manifest.authentication,
                    &generation,
                    bundle,
                )?;
                self.link_runtime_projection_credentials(
                    package,
                    &generation,
                    &canonical,
                    bundle,
                    false,
                )?;
            }
        }
        Ok(())
    }

    fn runtime_projection_generations(&self, provider_id: &str) -> Result<Vec<PathBuf>> {
        let root = self
            .auth_provider_root(provider_id)
            .join("runtime/generations");
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut generations = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(generation) = name.to_str().and_then(|name| name.parse::<u64>().ok()) else {
                continue;
            };
            generations.push((generation, entry.path()));
        }
        generations.sort_by_key(|(generation, _)| *generation);
        Ok(generations.into_iter().map(|(_, path)| path).collect())
    }

    fn canonical_runtime_projection(&self, package: &ProviderPackage) -> Result<Option<PathBuf>> {
        let runtime_root = self
            .auth_provider_root(&package.manifest.id)
            .join("runtime");
        let generations = self.runtime_projection_generations(&package.manifest.id)?;
        if generations.is_empty() {
            return Ok(None);
        }
        let current_link = runtime_root.join("current");
        let current = read_link_name(&current_link)
            .map(|generation| runtime_root.join("generations").join(generation))
            .filter(|current| generations.iter().any(|generation| generation == current));
        let current_is_valid = current.is_some();
        let canonical = current.unwrap_or_else(|| generations[0].clone());
        let metadata = read_materialization_metadata(&canonical)?;
        ensure!(
            metadata.auth_contract_fingerprint
                == package.manifest.compatibility.auth_contract_fingerprint,
            "Provider runtime credential source uses a different auth contract"
        );
        let name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .context("Provider runtime credential source has no generation name")?;
        if !current_is_valid {
            activate_link(&runtime_root, name)?;
        }
        Ok(Some(canonical))
    }

    fn link_runtime_projection_credentials(
        &self,
        package: &ProviderPackage,
        generation: &Path,
        canonical: &Path,
        bundle: &PortableCredentialBundle,
        replace_changed: bool,
    ) -> Result<()> {
        let auth = &package.manifest.authentication;
        let home = generation.join("home");
        let canonical_home = canonical.join("home");
        ensure!(
            generation != canonical,
            "canonical projection cannot link to itself"
        );
        let materialized_generation = self
            .auth_provider_root(&package.manifest.id)
            .join("materialized/generations")
            .join(
                read_materialization_metadata(generation)?
                    .auth_generation
                    .to_string(),
            )
            .join("home");
        for credential in &auth.credential_files {
            let destination = home.join(&credential.relative_path);
            let source = canonical_home.join(&credential.relative_path);
            ensure_within(&home, &destination)?;
            ensure_within(&canonical_home, &source)?;
            let Some(value) = bundle.values.get(&credential.bundle_key) else {
                remove_projected_credential(&destination)?;
                continue;
            };
            ensure!(source.is_file(), "canonical Provider credential is missing");
            if projected_credential_alias_matches(&destination, &source) {
                continue;
            }
            if !replace_changed
                && projection_contains_uncommitted_refresh(
                    &destination,
                    &source,
                    value,
                    &materialized_generation.join(&credential.relative_path),
                )?
            {
                continue;
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
                set_directory_chain_permissions(&home, parent)?;
            }
            replace_link(&destination, &source.to_string_lossy())?;
        }
        Ok(())
    }

    fn writable_auth_projection_generations(
        &self,
        package: &ProviderPackage,
    ) -> Result<Vec<PathBuf>> {
        self.runtime_projection_generations(&package.manifest.id)
    }

    fn inventory_one(&self, provider_id: &str) -> Result<Option<PluginInventory>> {
        let active = self.plugin_root(provider_id).join("active");
        if let Some(generation) = read_link_name(&active) {
            let path = self
                .plugin_root(provider_id)
                .join("generations")
                .join(&generation)
                .join("content/plugin-inventory.json");
            if path.is_file() {
                let mut inventory: PluginInventory = serde_json::from_slice(&fs::read(&path)?)?;
                inventory.rollback_generation_digest =
                    read_link_name(&self.plugin_root(provider_id).join("rollback"))
                        .map(|name| format!("sha256:{name}"));
                return Ok(Some(inventory));
            }
        }
        let Some((package, digest)) = self.active_package(provider_id)? else {
            return Ok(None);
        };
        let rollback = read_link_name(&self.plugin_root(provider_id).join("rollback"))
            .map(|name| format!("sha256:{name}"));
        let replica = self.latest_auth_envelope(provider_id)?;
        let auth_generation = replica.as_ref().map(|value| value.auth_generation);
        let replica_state = if replica.is_some() {
            ProviderReplicaState::Current
        } else {
            ProviderReplicaState::Absent
        };
        let materialization_state =
            replica
                .as_ref()
                .map_or(ProviderMaterializationState::NotInstalled, |envelope| {
                    if envelope.action == ProviderAuthAction::Wipe {
                        ProviderMaterializationState::NotInstalled
                    } else if self
                        .materialization_is_current(&package, envelope)
                        .unwrap_or(false)
                    {
                        ProviderMaterializationState::Current
                    } else {
                        ProviderMaterializationState::Failed
                    }
                });
        let detail = (materialization_state == ProviderMaterializationState::Failed).then(|| {
            "Provider credential projection no longer matches its Service generation; waiting for credential reconciliation."
                .to_owned()
        });
        Ok(Some(PluginInventory {
            plugin_id: package.manifest.id.clone(),
            plugin_version: package.manifest.version.clone(),
            plugin_kind: cowboy_plugin_sdk::PluginKind::AgentProvider,
            generation_digest: digest,
            contract_fingerprint: package.contract_fingerprint.clone(),
            state: PluginInstallationState::Active,
            rollback_generation_digest: rollback,
            active_session_leases: 0,
            auth_generation,
            replica_state,
            materialization_state,
            detail,
        }))
    }

    fn materialization_is_current(
        &self,
        package: &ProviderPackage,
        envelope: &SealedProviderAuth,
    ) -> Result<bool> {
        if envelope.action != ProviderAuthAction::Apply {
            return Ok(false);
        }
        let generation = self
            .auth_provider_root(&package.manifest.id)
            .join("materialized/generations")
            .join(envelope.auth_generation.to_string());
        let metadata = fs::read(generation.join("metadata.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<MaterializationMetadata>(&bytes).ok());
        if !metadata.is_some_and(|value| {
            value.auth_generation == envelope.auth_generation
                && value.auth_contract_fingerprint
                    == package.manifest.compatibility.auth_contract_fingerprint
        }) {
            return Ok(false);
        }
        let plaintext = self.encryption.open(envelope)?;
        let bundle: PortableCredentialBundle = serde_json::from_slice(&plaintext)?;
        validate_portable_bundle(&package.manifest.authentication, &bundle)?;
        Ok(materialization_contains_bundle(
            &package.manifest.authentication,
            &generation,
            &bundle,
        ))
    }

    fn active_package(&self, provider_id: &str) -> Result<Option<(ProviderPackage, String)>> {
        let active = self.plugin_root(provider_id).join("active");
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
        cowboy_provider_sdk::AgentRuntimeBinding,
        PathBuf,
    )> {
        let generation = digest_generation_name(digest)?;
        let content = self
            .plugin_root(provider_id)
            .join("generations")
            .join(generation)
            .join("content");
        if !content.join("package.cowboy-plugin").is_file() {
            return self.verified_legacy_provider_generation(provider_id, digest, &content);
        }
        let (plugin_package, plugin_release, content) =
            self.verified_plugin_generation(provider_id, digest)?;
        let path = content.join("package.cowboy-provider");
        let bytes = fs::read(&path)
            .with_context(|| format!("reading Provider generation {}", path.display()))?;
        let package = plugin_package
            .agent_provider()
            .context("stored Plugin is not an Agent Provider")?
            .clone();
        ensure!(
            package.canonical_bytes()? == bytes,
            "stored Agent Provider projection does not match Plugin payload"
        );
        let release = plugin_release.agent_provider_binding(&plugin_package)?;
        ensure!(
            package.manifest.id == provider_id,
            "stored Provider id mismatch"
        );
        Ok((package, release, path))
    }

    fn verified_legacy_provider_generation(
        &self,
        provider_id: &str,
        digest: &str,
        content: &Path,
    ) -> Result<(ProviderPackage, AgentRuntimeBinding, PathBuf)> {
        let path = content.join("package.cowboy-provider");
        let bytes = fs::read(&path)
            .with_context(|| format!("reading legacy Provider generation {}", path.display()))?;
        let release: LegacyProviderRelease = serde_json::from_slice(
            &fs::read(content.join("release.json"))
                .context("reading stored legacy Provider release")?,
        )?;
        ensure!(
            release.artifact_digest == digest,
            "stored legacy Provider release generation digest mismatch"
        );
        let publisher_key = fs::read_to_string(content.join("publisher.pub"))?;
        ensure!(
            crate::machine_auth::verify_namespaced(
                &publisher_key,
                LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                &release.proof()?,
                &release.signature,
            )?,
            "stored legacy Provider publisher signature is invalid"
        );
        let (package, binding) = release.validate_and_project(&bytes)?;
        ensure!(
            package.manifest.id == provider_id,
            "stored legacy Provider id mismatch"
        );
        let target = matching_runtime_artifacts(&binding, &self.platform, &self.architecture)?;
        let staging = target.clone();
        let metadata = read_installed_runtime(content)?;
        ensure!(
            installed_runtime_matches(content, &staging, &metadata)?,
            "retained legacy Provider runtime failed integrity verification"
        );
        Ok((package, binding, path))
    }

    fn verified_plugin_generation(
        &self,
        plugin_id: &str,
        digest: &str,
    ) -> Result<(PluginPackage, cowboy_plugin_sdk::PluginRelease, PathBuf)> {
        let generation = digest_generation_name(digest)?;
        let content = self
            .plugin_root(plugin_id)
            .join("generations")
            .join(generation)
            .join("content");
        let bytes = fs::read(content.join("package.cowboy-plugin"))
            .context("reading stored Plugin package")?;
        let release: cowboy_plugin_sdk::PluginRelease = serde_json::from_slice(
            &fs::read(content.join("plugin-release.json"))
                .context("reading stored Plugin release")?,
        )?;
        ensure!(
            release.artifact_digest == digest,
            "stored Plugin release generation digest mismatch"
        );
        let package = release.validate_bytes(&bytes)?;
        ensure!(
            package.manifest.id == plugin_id,
            "stored Plugin id mismatch"
        );
        let publisher_key = fs::read_to_string(content.join("publisher.pub"))?;
        ensure!(
            crate::machine_auth::verify_namespaced(
                &publisher_key,
                PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
                &release.proof(),
                &release.signature,
            )?,
            "stored Plugin publisher signature is invalid"
        );
        let target = matching_plugin_runtime_artifacts(
            &release.runtime_artifacts,
            &self.platform,
            &self.architecture,
        )?;
        let staging = provider_staging_projection(target);
        let metadata = read_installed_runtime(&content)?;
        ensure!(
            installed_runtime_matches(&content, &staging, &metadata)?,
            "retained Plugin runtime failed integrity verification"
        );
        Ok((package, release, content))
    }

    fn activate(plugin_root: &Path, generation_name: &str) -> Result<ProviderActivationSnapshot> {
        fs::create_dir_all(plugin_root.join("generations"))?;
        let active = plugin_root.join("active");
        let snapshot = ProviderActivationSnapshot {
            active: read_link_name(&active),
            rollback: read_link_name(&plugin_root.join("rollback")),
        };
        let result = (|| -> Result<()> {
            if let Some(previous) = snapshot.active.as_deref()
                && previous != generation_name
            {
                replace_link(
                    &plugin_root.join("rollback"),
                    &format!("generations/{previous}"),
                )?;
            }
            replace_link(&active, &format!("generations/{generation_name}"))
        })();
        if let Err(error) = result {
            if let Err(rollback_error) = Self::restore_activation(plugin_root, &snapshot) {
                bail!(
                    "Provider generation activation failed: {error:#}; restoring activation links also failed: {rollback_error:#}"
                );
            }
            return Err(error);
        }
        Ok(snapshot)
    }

    fn restore_activation(plugin_root: &Path, snapshot: &ProviderActivationSnapshot) -> Result<()> {
        restore_generation_link(&plugin_root.join("active"), snapshot.active.as_deref())?;
        restore_generation_link(&plugin_root.join("rollback"), snapshot.rollback.as_deref())
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

    fn plugin_root(&self, provider_id: &str) -> PathBuf {
        self.root.join(provider_id)
    }

    fn auth_provider_root(&self, provider_id: &str) -> PathBuf {
        self.auth_root.join("providers").join(provider_id)
    }

    #[must_use]
    pub fn auth_watch_root(&self) -> PathBuf {
        self.auth_root.join("providers")
    }

    /// Return whether a filesystem notification can affect a declared
    /// compare-and-swap credential. Runtime logs and other Provider state are
    /// deliberately excluded so ordinary agent activity never causes an auth
    /// scan.
    pub fn auth_event_is_relevant(&self, paths: &[PathBuf]) -> bool {
        let root = self.auth_watch_root();
        self.auth_credential_watch_paths()
            .is_ok_and(|credentials| auth_event_paths_intersect(&root, &credentials, paths))
    }

    /// Compare each writable Machine projection with its signed Service
    /// replica. Only a complete, contract-valid changed bundle is returned;
    /// missing or partially written credentials remain a failed
    /// materialization and are never promoted to Service authority.
    pub fn auth_refresh_observations(&self) -> ProviderAuthRefreshObservations {
        let mut observations = ProviderAuthRefreshObservations::default();
        let Ok(entries) = fs::read_dir(self.auth_watch_root()) else {
            return observations;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let provider_id = entry.file_name().to_string_lossy().into_owned();
            match self.auth_refresh_candidates_for_provider(&provider_id) {
                Ok(mut candidates) => observations.candidates.append(&mut candidates),
                Err(error) => {
                    observations.failed_provider_ids.insert(provider_id.clone());
                    tracing::warn!(
                        provider = %provider_id,
                        %error,
                        "Provider runtime credential projection cannot be reconciled"
                    );
                }
            }
        }
        observations.candidates.sort_by(|left, right| {
            left.provider_id
                .cmp(&right.provider_id)
                .then(left.expected_generation.cmp(&right.expected_generation))
                .then(left.bundle.values.cmp(&right.bundle.values))
        });
        observations
    }

    fn auth_credential_watch_paths(&self) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        let root = self.auth_watch_root();
        if !root.is_dir() {
            return Ok(paths);
        }
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let provider_id = entry.file_name().to_string_lossy().into_owned();
            let Some((package, _)) = self.active_package(&provider_id)? else {
                continue;
            };
            if package.manifest.authentication.refresh != RefreshOwnership::CompareAndSwap {
                continue;
            }
            let Some(envelope) = self.latest_auth_envelope(&provider_id)? else {
                continue;
            };
            if envelope.action != ProviderAuthAction::Apply {
                continue;
            }
            for generation in self.writable_auth_projection_generations(&package)? {
                let home = generation.join("home");
                for credential in &package.manifest.authentication.credential_files {
                    let path = home.join(&credential.relative_path);
                    ensure_within(&home, &path)?;
                    paths.push(path);
                }
                paths.push(generation.join("environment.json"));
            }
        }
        Ok(paths)
    }

    fn auth_refresh_candidates_for_provider(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProviderAuthRefreshCandidate>> {
        let Some((package, generation_digest)) = self.active_package(provider_id)? else {
            return Ok(Vec::new());
        };
        let auth = &package.manifest.authentication;
        if auth.refresh != RefreshOwnership::CompareAndSwap {
            return Ok(Vec::new());
        }
        let Some(envelope) = self.latest_auth_envelope(provider_id)? else {
            return Ok(Vec::new());
        };
        if envelope.action != ProviderAuthAction::Apply {
            return Ok(Vec::new());
        }
        ensure!(
            envelope.auth_contract_fingerprint
                == package.manifest.compatibility.auth_contract_fingerprint,
            "Provider auth replica contract does not match the active package"
        );
        let baseline: PortableCredentialBundle =
            serde_json::from_slice(&self.encryption.open(&envelope)?)
                .context("decoding signed Provider credential replica")?;
        validate_portable_bundle(auth, &baseline)?;
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for generation in self.writable_auth_projection_generations(&package)? {
            let metadata = read_materialization_metadata(&generation)?;
            ensure!(
                metadata.auth_contract_fingerprint
                    == package.manifest.compatibility.auth_contract_fingerprint,
                "Provider runtime projection uses a different auth contract"
            );
            let projected = projected_credential_bundle(auth, &generation, &baseline.method_id)?;
            if projected == baseline {
                continue;
            }
            let identity = serde_json::to_vec(&projected)?;
            if !seen.insert(identity) {
                continue;
            }
            candidates.push(ProviderAuthRefreshCandidate {
                provider_id: provider_id.to_owned(),
                expected_generation: envelope.auth_generation,
                provider_version: package.manifest.version.clone(),
                generation_digest: generation_digest.clone(),
                auth_contract_fingerprint: package
                    .manifest
                    .compatibility
                    .auth_contract_fingerprint
                    .clone(),
                bundle: projected,
            });
        }
        Ok(candidates)
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

fn auth_event_paths_intersect(
    root: &Path,
    credentials: &[PathBuf],
    changed_paths: &[PathBuf],
) -> bool {
    changed_paths.iter().any(|changed| {
        changed.starts_with(root)
            && credentials.iter().any(|credential| {
                changed == credential
                    || credential.starts_with(changed)
                    || changed.parent() == credential.parent()
            })
    })
}

fn projected_credential_bundle(
    auth: &cowboy_provider_sdk::AuthenticationContract,
    generation: &Path,
    method_id: &str,
) -> Result<PortableCredentialBundle> {
    let home = generation.join("home");
    ensure!(home.is_dir(), "Provider auth projection home is missing");
    let method = auth
        .methods
        .iter()
        .find(|method| method.id == method_id)
        .context("Provider auth replica references an unknown method")?;
    let mut values = BTreeMap::new();
    let mut total = 0_usize;
    for credential in &auth.credential_files {
        let path = home.join(&credential.relative_path);
        ensure_within(&home, &path)?;
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
                if error.kind() == std::io::ErrorKind::NotFound
                    && !method.required_bundle_keys.contains(&credential.bundle_key) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                bail!("Provider auth projection is missing a method-required credential")
            }
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        }
    }
    let environment_path = generation.join("environment.json");
    let environment: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(&environment_path).with_context(|| {
            format!(
                "reading Provider auth environment {}",
                environment_path.display()
            )
        })?)?;
    for (name, bundle_key) in &auth.environment_projection {
        let Some(value) = environment.get(name) else {
            ensure!(
                !method.required_bundle_keys.contains(bundle_key),
                "Provider auth projection is missing a method-required environment value"
            );
            continue;
        };
        ensure!(
            value.len() <= MAX_CREDENTIAL_VALUE_BYTES,
            "credential value is too large"
        );
        total = total.saturating_add(value.len());
        ensure!(
            total <= MAX_CREDENTIAL_BUNDLE_BYTES,
            "credential bundle is too large"
        );
        values.insert(
            bundle_key.clone(),
            base64::engine::general_purpose::STANDARD.encode(value.as_bytes()),
        );
    }
    let bundle = PortableCredentialBundle {
        portable_schema: auth.portable_schema.clone(),
        method_id: method_id.to_owned(),
        values,
    };
    validate_portable_bundle(auth, &bundle)?;
    Ok(bundle)
}

fn materialization_contains_bundle(
    auth: &cowboy_provider_sdk::AuthenticationContract,
    generation: &Path,
    bundle: &PortableCredentialBundle,
) -> bool {
    projected_credential_bundle(auth, generation, &bundle.method_id)
        .is_ok_and(|projected| projected == *bundle)
}

fn credential_relative_paths(auth: &cowboy_provider_sdk::AuthenticationContract) -> Vec<PathBuf> {
    auth.credential_files
        .iter()
        .map(|credential| PathBuf::from(&credential.relative_path))
        .collect()
}

fn credential_path_relation(credentials: &[PathBuf], relative: &Path) -> (bool, bool) {
    let exact = credentials.iter().any(|credential| credential == relative);
    let ancestor = credentials
        .iter()
        .any(|credential| credential.starts_with(relative));
    (exact, ancestor)
}

fn migrate_legacy_runtime_state(
    auth: &cowboy_provider_sdk::AuthenticationContract,
    legacy_home: &Path,
    runtime_home: &Path,
) -> Result<usize> {
    if !legacy_home.is_dir() {
        return Ok(0);
    }
    fs::create_dir_all(runtime_home)?;
    let credentials = credential_relative_paths(auth);
    migrate_legacy_runtime_directory(legacy_home, runtime_home, Path::new(""), &credentials)
}

fn migrate_legacy_runtime_directory(
    legacy_home: &Path,
    runtime_home: &Path,
    relative: &Path,
    credentials: &[PathBuf],
) -> Result<usize> {
    let legacy_directory = legacy_home.join(relative);
    let runtime_directory = runtime_home.join(relative);
    fs::create_dir_all(&runtime_directory)?;
    let entries = fs::read_dir(&legacy_directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
    let mut migrated = 0_usize;
    for entry in entries {
        let child = relative.join(entry.file_name());
        let (exact, ancestor) = credential_path_relation(credentials, &child);
        if exact {
            continue;
        }
        let legacy = legacy_home.join(&child);
        let runtime = runtime_home.join(&child);
        let file_type = entry.file_type()?;
        if ancestor {
            ensure!(
                file_type.is_dir(),
                "Provider credential ancestor is not a directory: {}",
                legacy.display()
            );
            migrated = migrated.saturating_add(migrate_legacy_runtime_directory(
                legacy_home,
                runtime_home,
                &child,
                credentials,
            )?);
            continue;
        }
        if legacy_state_alias_matches(&legacy, &runtime) {
            continue;
        }
        if file_type.is_dir() && runtime.is_dir() {
            migrated = migrated.saturating_add(migrate_legacy_runtime_directory(
                legacy_home,
                runtime_home,
                &child,
                credentials,
            )?);
            continue;
        }
        if runtime.symlink_metadata().is_ok() {
            // Both sides may already contain Provider-owned state from workers
            // that straddled the protocol-five/runtime-home rollout. Never
            // overwrite either copy: merge directories recursively and keep
            // conflicting leaves at their original paths.
            continue;
        }
        if let Some(parent) = runtime.parent() {
            fs::create_dir_all(parent)?;
        }
        if file_type.is_symlink() {
            let target = fs::read_link(&legacy)?;
            let target = if target.is_absolute() {
                target
            } else {
                legacy
                    .parent()
                    .context("legacy Provider symlink has no parent")?
                    .join(target)
            };
            symlink(target, &runtime)?;
            fs::remove_file(&legacy)?;
        } else {
            fs::rename(&legacy, &runtime).with_context(|| {
                format!(
                    "moving legacy Provider state {} to {}",
                    legacy.display(),
                    runtime.display()
                )
            })?;
        }
        symlink(&runtime, &legacy).with_context(|| {
            format!(
                "linking legacy Provider state {} to {}",
                legacy.display(),
                runtime.display()
            )
        })?;
        migrated = migrated.saturating_add(1);
    }
    Ok(migrated)
}

fn legacy_state_alias_matches(legacy: &Path, runtime: &Path) -> bool {
    let Ok(target) = fs::read_link(legacy) else {
        return false;
    };
    let target = if target.is_absolute() {
        target
    } else {
        legacy
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .join(target)
    };
    match (fs::canonicalize(target), fs::canonicalize(runtime)) {
        (Ok(target), Ok(runtime)) => target == runtime,
        _ => false,
    }
}

fn restore_projected_bundle(
    auth: &cowboy_provider_sdk::AuthenticationContract,
    generation: &Path,
    bundle: &PortableCredentialBundle,
) -> Result<()> {
    validate_portable_bundle(auth, bundle)?;
    let home = generation.join("home");
    fs::create_dir_all(&home)?;
    set_tree_root_permissions(generation)?;
    let mut total = 0_usize;
    for file in &auth.credential_files {
        let destination = home.join(&file.relative_path);
        ensure_within(&home, &destination)?;
        let Some(value) = bundle.values.get(&file.bundle_key) else {
            remove_projected_credential(&destination)?;
            continue;
        };
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
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
            set_directory_chain_permissions(&home, parent)?;
        }
        if fs::read(&destination).ok().as_deref() != Some(bytes.as_slice()) {
            atomic_write(&destination, &bytes, 0o600)?;
        }
    }
    restore_projected_environment(auth, generation, bundle)
}

fn restore_projected_environment(
    auth: &cowboy_provider_sdk::AuthenticationContract,
    generation: &Path,
    bundle: &PortableCredentialBundle,
) -> Result<()> {
    validate_portable_bundle(auth, bundle)?;
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
        let value = String::from_utf8(bytes).context("environment credential is not UTF-8")?;
        ensure!(!value.contains('\0'), "environment credential contains NUL");
        environment.insert(name.clone(), value);
    }
    let environment_path = generation.join("environment.json");
    let encoded_environment = serde_json::to_vec(&environment)?;
    if fs::read(&environment_path).ok().as_deref() != Some(encoded_environment.as_slice()) {
        atomic_write(&environment_path, &encoded_environment, 0o600)?;
    }
    Ok(())
}

fn remove_projected_credential(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                !metadata.file_type().is_dir(),
                "Provider credential path is a directory"
            );
            fs::remove_file(path)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

fn projected_credential_alias_matches(path: &Path, source: &Path) -> bool {
    let Ok(target) = fs::read_link(path) else {
        return false;
    };
    let target = if target.is_absolute() {
        target
    } else {
        path.parent().unwrap_or_else(|| Path::new("/")).join(target)
    };
    target == source
}

fn projection_contains_uncommitted_refresh(
    destination: &Path,
    canonical: &Path,
    sealed_value: &str,
    immutable: &Path,
) -> Result<bool> {
    let metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", destination.display()));
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(false);
    }
    ensure!(
        metadata.file_type().is_file(),
        "Provider credential path is not a file"
    );
    let projected = fs::read(destination)?;
    if fs::read(canonical).is_ok_and(|value| value == projected) {
        return Ok(false);
    }
    let sealed = base64::engine::general_purpose::STANDARD
        .decode(sealed_value)
        .context("decoding sealed Provider credential")?;
    if projected == sealed || fs::read(immutable).is_ok_and(|value| value == projected) {
        return Ok(false);
    }
    Ok(true)
}

fn repair_missing_projected_bundle(
    auth: &cowboy_provider_sdk::AuthenticationContract,
    generation: &Path,
    bundle: &PortableCredentialBundle,
) -> Result<()> {
    let home = generation.join("home");
    fs::create_dir_all(&home)?;
    set_tree_root_permissions(generation)?;
    let mut total = 0_usize;
    for file in &auth.credential_files {
        let Some(value) = bundle.values.get(&file.bundle_key) else {
            continue;
        };
        let destination = home.join(&file.relative_path);
        ensure_within(&home, &destination)?;
        if destination.is_file() {
            continue;
        }
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
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
            set_directory_chain_permissions(&home, parent)?;
        }
        atomic_write(&destination, &bytes, 0o600)?;
    }
    let environment_path = generation.join("environment.json");
    // A truncated environment file is equivalent to a missing projection.
    // Rebuild declared values from the sealed Service bundle instead of
    // leaving the Provider permanently failed after an interrupted write.
    let (mut environment, mut environment_changed): (BTreeMap<String, String>, bool) =
        match fs::read(&environment_path)
            .ok()
            .and_then(|encoded| serde_json::from_slice(&encoded).ok())
        {
            Some(environment) => (environment, false),
            None => (BTreeMap::new(), true),
        };
    for (name, bundle_key) in &auth.environment_projection {
        if environment.contains_key(name) {
            continue;
        }
        let Some(value) = bundle.values.get(bundle_key) else {
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD.decode(value)?;
        ensure!(
            bytes.len() <= MAX_CREDENTIAL_VALUE_BYTES,
            "credential value too large"
        );
        let value = String::from_utf8(bytes).context("environment credential is not UTF-8")?;
        ensure!(!value.contains('\0'), "environment credential contains NUL");
        environment.insert(name.clone(), value);
        environment_changed = true;
    }
    if environment_changed {
        atomic_write(&environment_path, &serde_json::to_vec(&environment)?, 0o600)?;
    }
    Ok(())
}

fn read_materialization_metadata(generation: &Path) -> Result<MaterializationMetadata> {
    serde_json::from_slice(
        &fs::read(generation.join("metadata.json"))
            .context("reading Provider credential projection metadata")?,
    )
    .context("decoding Provider credential projection metadata")
}

fn validate_materialization_metadata(
    generation: &Path,
    package: &ProviderPackage,
    auth_generation: u64,
) -> Result<()> {
    let metadata = read_materialization_metadata(generation)?;
    ensure!(
        metadata.auth_generation == auth_generation
            && metadata.auth_contract_fingerprint
                == package.manifest.compatibility.auth_contract_fingerprint,
        "Provider runtime projection has conflicting identity"
    );
    Ok(())
}

fn matching_plugin_runtime_artifacts<'a>(
    artifacts: &'a [PluginRuntimeArtifacts],
    platform: &Platform,
    architecture: &str,
) -> Result<&'a PluginRuntimeArtifacts> {
    let os = match platform {
        Platform::Linux => OperatingSystem::Linux,
        Platform::Macos => OperatingSystem::Macos,
    };
    let architecture = match architecture {
        "x86_64" => Architecture::X86_64,
        "aarch64" => Architecture::Aarch64,
        other => bail!("unsupported Machine architecture {other:?}"),
    };
    artifacts
        .iter()
        .find(|target| target.os == os && target.architecture == architecture)
        .context("Plugin release has no runtime artifacts for this Machine platform")
}

fn provider_staging_projection(artifacts: &PluginRuntimeArtifacts) -> PlatformRuntimeArtifacts {
    PlatformRuntimeArtifacts {
        os: artifacts.os.clone(),
        architecture: artifacts.architecture.clone(),
        components: artifacts
            .components
            .iter()
            .map(|component| ReleasedPrivateComponent {
                kind: match component.kind {
                    PluginComponentKind::AgentCli => PrivateComponentKind::ProviderCli,
                    PluginComponentKind::AgentAdapter => PrivateComponentKind::ProviderAdapter,
                    PluginComponentKind::AgentGateway => PrivateComponentKind::ProviderGateway,
                    PluginComponentKind::AcpRuntime => PrivateComponentKind::AcpRuntime,
                    PluginComponentKind::CodeIntelligenceAdapter
                    | PluginComponentKind::CodeIntelligenceServer => {
                        PrivateComponentKind::ProviderAdapter
                    }
                },
                slot: component.slot.clone(),
                dependency: component.dependency.clone(),
                version: component.version.clone(),
                command: component.command.clone(),
                artifact_url: component.artifact_url.clone(),
                artifact_digest: component.artifact_digest.clone(),
                artifact_format: match component.artifact_format {
                    PluginArtifactFormat::Raw => ProviderArtifactFormat::Raw,
                    PluginArtifactFormat::TarGz => ProviderArtifactFormat::TarGz,
                },
                entrypoint: component.entrypoint.clone(),
                probe: cowboy_provider_sdk::ProviderArtifactProbe {
                    args: component.probe.args.clone(),
                    timeout_ms: component.probe.timeout_ms,
                },
            })
            .collect(),
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

fn validate_plugin_id(provider_id: &str) -> Result<()> {
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
pub(crate) struct PluginInventoryReceipt {
    pub provider_id: String,
    pub auth_generation: u64,
    pub auth_generation_advanced: bool,
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
    release: &'a cowboy_provider_sdk::AgentRuntimeBinding,
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
        .context("Agent Plugin binding has no runtime artifacts for this Machine platform")
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
    if metadata.schema_version != 2 || metadata.commands.len() != artifacts.components.len() {
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
        let component_root =
            content
                .join("runtime")
                .join(format!("{}-{}", expected.kind.as_str(), expected.slot));
        let (expected_executable, expected_artifact) = match expected.artifact_format {
            ProviderArtifactFormat::Raw => (component_root.join("bin"), component_root.join("bin")),
            ProviderArtifactFormat::TarGz => (
                component_root.join("content").join(
                    expected
                        .entrypoint
                        .as_deref()
                        .context("runtime archive requires an entrypoint")?,
                ),
                component_root.join("artifact.tar.gz"),
            ),
        };
        if executable != expected_executable || artifact != expected_artifact {
            return Ok(false);
        }
        ensure_within(content, &executable)?;
        ensure_within(content, &artifact)?;
        if !fs::symlink_metadata(&executable).is_ok_and(|metadata| metadata.is_file())
            || !fs::symlink_metadata(&artifact).is_ok_and(|metadata| metadata.is_file())
            || digest_file(&artifact)? != expected.artifact_digest.to_ascii_lowercase()
        {
            return Ok(false);
        }
        if expected.artifact_format == ProviderArtifactFormat::TarGz
            && !archive_runtime_matches(&artifact, &component_root.join("content"))?
        {
            return Ok(false);
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

fn archive_runtime_matches(archive_path: &Path, extracted: &Path) -> Result<bool> {
    let decoder = flate2::read::GzDecoder::new(fs::File::open(archive_path)?);
    let mut archive = tar::Archive::new(decoder);
    let mut expanded_bytes = 0_u64;
    let mut entries = 0_usize;
    let mut paths = BTreeSet::new();
    let mut extracted_paths = BTreeSet::new();
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
        for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
            extracted_paths.insert(ancestor.to_path_buf());
        }
        let installed = extracted.join(&path);
        let Ok(metadata) = fs::symlink_metadata(&installed) else {
            return Ok(false);
        };
        if kind.is_file() {
            if !metadata.is_file() {
                return Ok(false);
            }
            let mut digest = Sha256::new();
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                let read = entry.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            if digest_file_with_limit(
                &installed,
                MAX_PROVIDER_RUNTIME_EXPANDED_BYTES,
                "installed runtime file",
            )? != format!("sha256:{:x}", digest.finalize())
            {
                return Ok(false);
            }
        } else if !metadata.is_dir() {
            return Ok(false);
        }
    }
    // An extra module/library is as capable of changing runtime behavior as a
    // modified entrypoint. Reject additions and links, not just changed files.
    if !fs::symlink_metadata(extracted).is_ok_and(|metadata| metadata.is_dir()) {
        return Ok(false);
    }
    let mut pending = vec![extracted.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for child in fs::read_dir(directory)? {
            let child = child?;
            let path = child.path();
            if !extracted_paths.remove(path.strip_prefix(extracted)?) {
                return Ok(false);
            }
            let kind = child.file_type()?;
            if kind.is_dir() {
                pending.push(path);
            } else if !kind.is_file() {
                return Ok(false);
            }
        }
    }
    Ok(extracted_paths.is_empty())
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
    ensure!(
        digest_file(&stored_artifact)? == artifact.artifact_digest.to_ascii_lowercase(),
        "runtime component probe changed signed artifact bytes"
    );
    if artifact.artifact_format == ProviderArtifactFormat::TarGz {
        ensure!(
            archive_runtime_matches(&stored_artifact, &component_root.join("content"))?,
            "runtime component probe changed extracted runtime bytes"
        );
    }
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

struct StagedProbeHome(PathBuf);

impl Drop for StagedProbeHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn probe_released_component(
    executable: &Path,
    artifact: &ReleasedPrivateComponent,
) -> Result<()> {
    let directory = std::env::temp_dir().join(format!(
        "cowboy-component-probe-{:032x}",
        rand::random::<u128>()
    ));
    fs::DirBuilder::new().mode(0o700).create(&directory)?;
    let home = StagedProbeHome(directory);
    let mut child = None;
    for attempt in 0..=4 {
        let mut command = tokio::process::Command::new(executable);
        command
            .args(&artifact.probe.args)
            .current_dir(&home.0)
            .env_clear()
            .env("HOME", &home.0)
            .env("XDG_CONFIG_HOME", home.0.join(".config"))
            .env("XDG_CACHE_HOME", home.0.join(".cache"))
            .env("XDG_DATA_HOME", home.0.join(".local/share"))
            .env("XDG_STATE_HOME", home.0.join(".local/state"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        for name in ["PATH", "SSL_CERT_FILE", "SSL_CERT_DIR", "NIX_SSL_CERT_FILE"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.as_std_mut().process_group(0);
        match command.spawn() {
            Ok(spawned) => {
                child = Some(spawned);
                break;
            }
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && attempt < 4 => {
                // Some Linux filesystems briefly retain the writer exclusion
                // after the atomic rename even though Cowboy closed and synced
                // its descriptor. Retry only ETXTBSY; every other exec failure
                // remains an immediate trust-gate failure.
                tokio::time::sleep(Duration::from_millis(5_u64 << attempt)).await;
            }
            Err(error) => {
                return Err(error).context("starting staged Provider component probe");
            }
        }
    }
    let mut child = child.context("starting staged Provider component probe")?;
    let _process_group = child
        .id()
        .map(crate::plugin_process::PluginProcessGroup::new);
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

fn desired_plugin_host_bundle(
    desired: &DesiredPlugin,
    package: &PluginPackage,
) -> Result<(Option<PluginHostBundle>, Option<Vec<u8>>)> {
    let bytes = desired
        .host_bundle_base64
        .as_deref()
        .map(|encoded| {
            ensure!(
                encoded.len() <= MAX_HOST_BUNDLE_BYTES.saturating_mul(4).div_ceil(3) + 4,
                "encoded Plugin host bundle is too large"
            );
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .context("decoding Plugin host bundle")
        })
        .transpose()?;
    let bundle = PluginHostBundle::from_bytes_for_package(
        bytes.as_deref(),
        &package.manifest.id,
        &package.manifest.version,
        &desired.release.package_digest,
        desired.release.host_bundle_digest.as_deref(),
    )?;
    package.validate_host_contract(bundle.as_ref().map(|bundle| &bundle.files))?;
    Ok((bundle, bytes))
}

fn verified_plugin_host_bundle(
    package: &PluginPackage,
    release: &cowboy_plugin_sdk::PluginRelease,
    content: &Path,
) -> Result<Option<PluginHostBundle>> {
    let path = content.join("plugin-hostbundle.json");
    let bytes = match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
                "stored Plugin host bundle is not a regular file"
            );
            Some(fs::read(&path).context("reading stored Plugin host bundle")?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("inspecting stored Plugin host bundle"),
    };
    let bundle = PluginHostBundle::from_bytes_for_package(
        bytes.as_deref(),
        &package.manifest.id,
        &package.manifest.version,
        &release.package_digest,
        release.host_bundle_digest.as_deref(),
    )?;
    package.validate_host_contract(bundle.as_ref().map(|bundle| &bundle.files))?;
    if let Some(bundle) = &bundle {
        ensure!(
            installed_host_files_match(&content.join("host"), bundle)?,
            "installed Plugin host files failed integrity verification"
        );
    } else {
        ensure!(
            !content.join("host").exists(),
            "unbound Plugin generation contains host files"
        );
    }
    Ok(bundle)
}

fn plugin_host_environment(context: &ProviderLaunchContext) -> Result<BTreeMap<String, String>> {
    const SAFE_AMBIENT: &[&str] = &[
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "LOGNAME",
        "NIX_SSL_CERT_FILE",
        "SHELL",
        "SSL_CERT_DIR",
        "SSL_CERT_FILE",
        "TMPDIR",
        "USER",
    ];
    let mut environment = SAFE_AMBIENT
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| ((*name).to_owned(), value))
        })
        .collect::<BTreeMap<_, _>>();
    environment.extend(context.environment.clone());
    if let Some(home) = &context.home {
        let home = home.to_string_lossy().into_owned();
        environment.insert("HOME".to_owned(), home.clone());
        environment.insert("XDG_CONFIG_HOME".to_owned(), format!("{home}/.config"));
        environment.insert("XDG_DATA_HOME".to_owned(), format!("{home}/.local/share"));
        environment.insert("XDG_CACHE_HOME".to_owned(), format!("{home}/.cache"));
    }
    let commands = context
        .environment
        .get(crate::provider_behavior::COMPONENT_COMMANDS_ENV)
        .context("exact Provider component command map is missing")?;
    let commands: BTreeMap<String, String> =
        serde_json::from_str(commands).context("decoding exact Provider component commands")?;
    let mut names = BTreeSet::new();
    for (command, executable) in commands {
        let name = plugin_component_environment_name(&command)?;
        ensure!(
            names.insert(name.clone()),
            "Provider component commands have an environment-name collision"
        );
        environment.insert(name, executable);
    }
    Ok(environment)
}

fn plugin_component_environment_name(command: &str) -> Result<String> {
    ensure!(!command.is_empty(), "Provider component command is empty");
    let normalized = command
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                byte.to_ascii_uppercase() as char
            } else {
                '_'
            }
        })
        .collect::<String>();
    ensure!(
        normalized.bytes().any(|byte| byte.is_ascii_alphanumeric()),
        "Provider component command has no environment-safe characters"
    );
    Ok(format!("COWBOY_PLUGIN_COMMAND_{normalized}"))
}

async fn start_usage_sidecar(
    package: &ProviderPackage,
    sidecar: &RuntimeSidecar,
    context: &ProviderLaunchContext,
    platform: &Platform,
    architecture: &str,
) -> Result<(tokio::process::Child, String)> {
    ensure!(
        sidecar.component.kind == PrivateComponentKind::ProviderGateway,
        "usage sidecar component is not a Provider gateway"
    );
    let payload = matching_payload(package, platform, architecture)?;
    let component = payload
        .private_components
        .iter()
        .find(|component| {
            component.kind == sidecar.component.kind && component.slot == sidecar.component.slot
        })
        .context("usage sidecar component is absent from the platform payload")?;
    let executable = runtime_command(&context.package_path, &component.command)?;
    let RuntimeSidecarTransport::LoopbackHttpV1 {
        listen_argument,
        health_path,
        timeout_ms,
    } = &sidecar.transport;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .context("allocating usage sidecar loopback port")?;
    let address = listener.local_addr()?;
    drop(listener);
    let base_url = format!("http://{address}");
    let health_url = format!("{base_url}{health_path}");

    let mut command = tokio::process::Command::new(&executable);
    command
        .args(&sidecar.arguments)
        .arg(listen_argument)
        .arg(address.to_string())
        .current_dir(
            executable
                .parent()
                .context("usage sidecar executable has no parent")?,
        )
        .env_clear()
        .envs(usage_sidecar_environment(context, sidecar)?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .with_context(|| format!("spawning usage sidecar {}", executable.display()))?;
    let deadline = Instant::now() + Duration::from_millis(*timeout_ms);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(250))
        .build()?;
    loop {
        if let Some(status) = child.try_wait().context("polling usage sidecar")? {
            bail!("usage sidecar exited before readiness: {status}");
        }
        if client
            .get(&health_url)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok((child, base_url));
        }
        if Instant::now() >= deadline {
            let _ = child.kill().await;
            bail!("usage sidecar readiness timed out");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn usage_sidecar_environment(
    context: &ProviderLaunchContext,
    sidecar: &RuntimeSidecar,
) -> Result<BTreeMap<String, String>> {
    const BASELINE_NAMES: &[&str] = &[
        "HOME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "LOGNAME",
        "NIX_SSL_CERT_FILE",
        "PATH",
        "SHELL",
        "SSL_CERT_DIR",
        "SSL_CERT_FILE",
        "TMPDIR",
        "USER",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
    ];
    let baseline = plugin_host_environment(context)?;
    let mut environment = BASELINE_NAMES
        .iter()
        .filter_map(|name| {
            baseline
                .get(*name)
                .map(|value| ((*name).to_owned(), value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    environment.extend(sidecar.environment.clone());
    for name in &sidecar.auth_environment {
        environment.insert(
            name.clone(),
            context
                .environment
                .get(name)
                .with_context(|| format!("usage sidecar auth projection {name:?} is missing"))?
                .clone(),
        );
    }
    Ok(environment)
}

fn stage_plugin_host_bundle(
    content: &Path,
    bundle: Option<&PluginHostBundle>,
    bytes: Option<&[u8]>,
) -> Result<()> {
    let host_root = content.join("host");
    let bundle_path = content.join("plugin-hostbundle.json");
    let Some(bundle) = bundle else {
        ensure!(
            bytes.is_none() && !host_root.exists() && !bundle_path.exists(),
            "unbound Plugin generation contains host files"
        );
        return Ok(());
    };
    let bytes = bytes.context("validated Plugin host bundle bytes are missing")?;
    if host_root.exists() {
        ensure!(
            installed_host_files_match(&host_root, bundle)?,
            "retained Plugin host files failed integrity verification"
        );
    } else {
        let temporary = content.join(format!(
            ".host-staging-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let stage = (|| -> Result<()> {
            fs::create_dir(&temporary)?;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700))?;
            for (relative, source) in &bundle.files {
                let destination = temporary.join(relative);
                ensure_within(&temporary, &destination)?;
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                    set_directory_chain_permissions(&temporary, parent)?;
                }
                atomic_write(&destination, source.as_bytes(), 0o600)?;
            }
            fs::rename(&temporary, &host_root)?;
            Ok(())
        })();
        if stage.is_err() {
            let _ = fs::remove_dir_all(&temporary);
        }
        stage.context("staging exact Plugin host files")?;
    }
    atomic_write(&bundle_path, bytes, 0o600)?;
    Ok(())
}

fn installed_host_files_match(root: &Path, bundle: &PluginHostBundle) -> Result<bool> {
    let mut installed = BTreeMap::new();
    collect_installed_host_files(root, root, &mut installed)?;
    Ok(installed == bundle.files)
}

fn collect_installed_host_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let metadata = entry.file_type()?;
        ensure!(
            !metadata.is_symlink(),
            "Plugin host tree contains a symlink"
        );
        if metadata.is_dir() {
            collect_installed_host_files(root, &entry.path(), files)?;
            continue;
        }
        ensure!(
            metadata.is_file(),
            "Plugin host tree contains a special file"
        );
        let relative = entry
            .path()
            .strip_prefix(root)
            .context("Plugin host file escaped its generation")?
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(entry.path())?;
        files.insert(relative, source);
    }
    Ok(())
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
    // Close the writable descriptor before the executable becomes reachable at
    // its final path. Linux rejects execve while any process still has the
    // inode open for writing (ETXTBSY); keeping the descriptor across rename
    // made a fast staged-component probe race that boundary under parallel
    // test and production I/O.
    drop(file);
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

    #[cfg(feature = "full")]
    fn telemetry_release(
        publisher: &crate::machine_auth::MachineIdentity,
        version: &str,
    ) -> DesiredPlugin {
        let mut manifest: cowboy_plugin_sdk::PluginManifest =
            serde_json::from_str(include_str!("../examples/telemetry/victoria/plugin.json"))
                .unwrap();
        let mut contract: cowboy_plugin_sdk::TelemetryBackendContract = serde_json::from_str(
            include_str!("../examples/telemetry/victoria/telemetry.json"),
        )
        .unwrap();
        manifest.version = version.to_owned();
        contract.version = version.to_owned();
        let platforms = contract.supported_platforms.clone();
        let package = PluginPackage::new(
            manifest.clone(),
            manifest.component_release.clone(),
            PluginPayload::TelemetryBackend(contract),
        )
        .unwrap();
        let bytes = package.canonical_bytes().unwrap();
        let mut release = cowboy_plugin_sdk::PluginRelease {
            release_schema: 1,
            plugin_id: manifest.id,
            plugin_version: manifest.version,
            plugin_kind: manifest.kind,
            package_digest: PluginPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: format!(
                "https://plugins.example.test/victoria/{version}/victoria.cowboy-plugin"
            ),
            publisher: manifest.publisher,
            contract_fingerprint: package.contract_fingerprint,
            component_release: manifest.component_release,
            host_bundle_digest: None,
            signature: String::new(),
            runtime_artifacts: platforms
                .iter()
                .map(|platform| PluginRuntimeArtifacts {
                    os: platform.os.clone(),
                    architecture: platform.architecture.clone(),
                    components: Vec::new(),
                })
                .collect(),
            supported_platforms: platforms,
        };
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        release.signature = publisher
            .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())
            .unwrap();
        DesiredPlugin {
            release,
            package_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            publisher_public_key: publisher.public_key().to_owned(),
            host_bundle_base64: None,
        }
    }

    #[tokio::test]
    #[cfg(feature = "full")]
    #[allow(clippy::too_many_lines)] // One signed lifecycle fixture covers policy, HTTP, rollback and tamper boundaries.
    async fn telemetry_signed_install_export_upgrade_uninstall_and_retained_integrity() {
        use std::sync::Arc;
        let root = std::env::temp_dir().join(format!(
            "cowboy-telemetry-lifecycle-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("publisher")).unwrap();
        let desired = telemetry_release(&publisher, "1.0.0");
        let machine_root = root.join("machine");
        let store =
            MachinePluginStore::new(&machine_root, Platform::Linux, "x86_64".into()).unwrap();
        let installed = store.install(&desired).await.unwrap();
        assert_eq!(
            installed.plugin_kind,
            cowboy_plugin_sdk::PluginKind::TelemetryBackend
        );
        assert_eq!(installed.auth_generation, None);
        assert!(!store.auth_provider_root("victoria").exists());
        let payload = || serde_json::json!({"logs": "{\"timestamp\":\"2026-09-08T00:00:00Z\",\"message\":\"fixture\",\"component\":\"cowboy-client\",\"platform\":\"web\"}\n", "metrics": "cowboy_client_fixture{platform=\"web\"} 1 1788825600000\n"});
        // Installation alone never authorizes network egress.
        assert!(
            store
                .invoke_host(
                    "victoria",
                    "1.0.0",
                    &installed.generation_digest,
                    None,
                    PluginHostOperation::ExportTelemetry,
                    payload()
                )
                .await
                .is_err()
        );
        let received = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let captured = Arc::clone(&received);
        let app = axum::Router::new().fallback(
            move |uri: axum::extract::OriginalUri, headers: axum::http::HeaderMap, body: String| {
                let captured = Arc::clone(&captured);
                async move {
                    let mut requests = captured.lock();
                    let logs = uri.path().ends_with("jsonline");
                    let previous_logs = requests
                        .iter()
                        .filter(|(path, _, _): &&(String, axum::http::HeaderMap, String)| {
                            path.contains("jsonline")
                        })
                        .count();
                    requests.push((uri.0.to_string(), headers, body));
                    if !logs {
                        axum::http::StatusCode::BAD_REQUEST
                    } else if previous_logs == 0 {
                        axum::http::StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        axum::http::StatusCode::NO_CONTENT
                    }
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let config = serde_json::json!({
            "plugin": {"plugin_id": "victoria", "plugin_version": "1.0.0", "generation_digest": installed.generation_digest},
            "logs": {"base_url": endpoint, "bearer_token": "fixture-only-not-a-real-token"},
            "metrics": {"base_url": endpoint},
        });
        atomic_write(
            &machine_root.join("telemetry.json"),
            &serde_json::to_vec(&config).unwrap(),
            0o600,
        )
        .unwrap();
        let receipt = store
            .invoke_host(
                "victoria",
                "1.0.0",
                &installed.generation_digest,
                None,
                PluginHostOperation::ExportTelemetry,
                payload(),
            )
            .await
            .unwrap();
        assert_eq!(
            receipt,
            serde_json::json!({"logs_delivered": true, "metrics_delivered": false})
        );
        {
            let requests = received.lock();
            assert_eq!(requests.len(), 3); // Transient 503 retried; permanent 400 not retried.
            let logs: Vec<_> = requests
                .iter()
                .filter(|(path, _, _)| path.contains("jsonline"))
                .collect();
            assert_eq!(logs.len(), 2);
            assert_eq!(logs[0].2, logs[1].2);
            assert_eq!(logs[0].1["content-type"], "application/stream+json");
            assert_eq!(
                logs[0].1["authorization"],
                "Bearer fixture-only-not-a-real-token"
            );
            assert!(logs[0].0.contains("_time_field=timestamp"));
            assert!(logs[0].0.contains("_stream_fields=component%2Cplatform"));
            let metrics = requests
                .iter()
                .find(|(path, _, _)| path.ends_with("/api/v1/import/prometheus"))
                .unwrap();
            assert_eq!(metrics.1["content-type"], "text/plain; version=0.0.4");
            assert!(metrics.2.ends_with("1788825600000\n"));
        }
        assert!(
            store
                .invoke_host(
                    "victoria",
                    "1.0.0",
                    &installed.generation_digest,
                    Some(1),
                    PluginHostOperation::ExportTelemetry,
                    payload()
                )
                .await
                .is_err()
        );
        assert!(
            store
                .invoke_host(
                    "victoria",
                    "1.0.0",
                    &installed.generation_digest,
                    None,
                    PluginHostOperation::CollectUsage,
                    serde_json::json!({})
                )
                .await
                .is_err()
        );
        let next = telemetry_release(&publisher, "1.0.1");
        let upgraded = store.install(&next).await.unwrap();
        assert_eq!(
            upgraded.rollback_generation_digest,
            Some(installed.generation_digest.clone())
        );
        assert!(
            store
                .invoke_host(
                    "victoria",
                    "1.0.0",
                    &installed.generation_digest,
                    None,
                    PluginHostOperation::ExportTelemetry,
                    payload()
                )
                .await
                .is_err()
        );
        assert!(
            store
                .invoke_host(
                    "victoria",
                    "1.0.1",
                    &upgraded.generation_digest,
                    None,
                    PluginHostOperation::ExportTelemetry,
                    payload()
                )
                .await
                .is_err()
        ); // Private policy still selects the old release.
        store
            .uninstall("victoria", &upgraded.generation_digest)
            .await
            .unwrap();
        assert!(
            store
                .invoke_host(
                    "victoria",
                    "1.0.1",
                    &upgraded.generation_digest,
                    None,
                    PluginHostOperation::ExportTelemetry,
                    payload()
                )
                .await
                .is_err()
        );
        store
            .reactivate("victoria", &installed.generation_digest)
            .await
            .unwrap();
        store
            .invoke_host(
                "victoria",
                "1.0.0",
                &installed.generation_digest,
                None,
                PluginHostOperation::ExportTelemetry,
                payload(),
            )
            .await
            .unwrap();
        let count = received.lock().len();
        let content = store
            .verified_plugin_generation("victoria", &installed.generation_digest)
            .unwrap()
            .2;
        atomic_write(&content.join("package.cowboy-plugin"), b"{}\n", 0o600).unwrap();
        assert!(
            store
                .invoke_host(
                    "victoria",
                    "1.0.0",
                    &installed.generation_digest,
                    None,
                    PluginHostOperation::ExportTelemetry,
                    payload()
                )
                .await
                .is_err()
        );
        assert_eq!(received.lock().len(), count);
        server.abort();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    fn seal_auth_for_test(
        store: &MachinePluginStore,
        package: &ProviderPackage,
        signer: &crate::machine_auth::MachineIdentity,
        auth_generation: u64,
        bundle: &PortableCredentialBundle,
    ) -> SealedProviderAuth {
        let recipient = decode_fixed::<32>(
            store.encryption.public_key(),
            "Machine encryption public key",
        )
        .unwrap();
        let ephemeral = StaticSecret::from([7_u8; 32]);
        let ephemeral_public = PublicKey::from(&ephemeral);
        let shared = ephemeral.diffie_hellman(&PublicKey::from(recipient));
        let key = derive_seal_key(shared.as_bytes());
        let nonce = [9_u8; 24];
        let mut envelope = SealedProviderAuth {
            envelope_schema: 1,
            provider_id: package.manifest.id.clone(),
            auth_generation,
            auth_contract_fingerprint: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            projection_schema: package.manifest.authentication.projection_schema.clone(),
            action: ProviderAuthAction::Apply,
            ephemeral_public_key: base64::engine::general_purpose::STANDARD
                .encode(ephemeral_public.as_bytes()),
            nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
            ciphertext: String::new(),
            service_public_key: signer.public_key().to_owned(),
            signature: String::new(),
        };
        let plaintext = serde_json::to_vec(bundle).unwrap();
        let ciphertext = XChaCha20Poly1305::new(Key::from_slice(&key))
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &provider_auth_aad(&envelope),
                },
            )
            .unwrap();
        envelope.ciphertext = base64::engine::general_purpose::STANDARD.encode(ciphertext);
        envelope.signature = signer
            .sign_namespaced(
                crate::machine_auth::PROVIDER_AUTH_SIGNATURE_NAMESPACE,
                &envelope.proof(),
            )
            .unwrap();
        envelope
    }

    // This end-to-end test intentionally retains the complete signed install,
    // runtime, auth, uninstall, and rollback story in one fixture.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn signed_provider_installs_exact_runtime_and_uninstalls_as_one_unit() {
        use cowboy_plugin_sdk::{
            PluginArtifactFormat, PluginArtifactProbe, PluginComponentKind, PluginManifest,
            PluginPackage, PluginPayload, PluginRelease, PluginRuntimeArtifacts,
            RELEASE_SCHEMA_VERSION,
        };
        use cowboy_provider_sdk::{
            PlatformRuntimeArtifacts, PlatformTarget, ProviderArtifactFormat,
            ProviderArtifactProbe, ReleasedPrivateComponent, StandardProviderSource, build_package,
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
            serde_json::from_str(include_str!("../plugins/gemini/provider.json")).unwrap();
        let package = build_package(source.compile().unwrap()).unwrap();
        let runtime_artifacts: Vec<PlatformRuntimeArtifacts> = package
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
        let manifest: PluginManifest =
            serde_json::from_str(include_str!("../plugins/gemini/plugin.json")).unwrap();
        let component_release = manifest.component_release.clone();
        let plugin_package = PluginPackage::new(
            manifest,
            component_release.clone(),
            PluginPayload::AgentProvider(Box::new(package.clone())),
        )
        .unwrap();
        let bytes = plugin_package.canonical_bytes().unwrap();
        let mut release = PluginRelease {
            release_schema: RELEASE_SCHEMA_VERSION,
            plugin_id: package.manifest.id.clone(),
            plugin_version: package.manifest.version.clone(),
            plugin_kind: cowboy_plugin_sdk::PluginKind::AgentProvider,
            package_digest: PluginPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: "https://example.invalid/gemini.cowboy-plugin".to_owned(),
            publisher: package.manifest.publisher.clone(),
            contract_fingerprint: plugin_package.contract_fingerprint.clone(),
            component_release,
            host_bundle_digest: None,
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
            runtime_artifacts: runtime_artifacts
                .into_iter()
                .map(|target| PluginRuntimeArtifacts {
                    os: target.os,
                    architecture: target.architecture,
                    components: target
                        .components
                        .into_iter()
                        .map(|component| cowboy_plugin_sdk::ReleasedPluginComponent {
                            kind: match component.kind {
                                cowboy_provider_sdk::PrivateComponentKind::ProviderCli => {
                                    PluginComponentKind::AgentCli
                                }
                                cowboy_provider_sdk::PrivateComponentKind::ProviderAdapter => {
                                    PluginComponentKind::AgentAdapter
                                }
                                cowboy_provider_sdk::PrivateComponentKind::ProviderGateway => {
                                    PluginComponentKind::AgentGateway
                                }
                                cowboy_provider_sdk::PrivateComponentKind::AcpRuntime => {
                                    PluginComponentKind::AcpRuntime
                                }
                            },
                            slot: component.slot,
                            dependency: component.dependency,
                            version: component.version,
                            command: component.command,
                            artifact_url: component.artifact_url,
                            artifact_digest: component.artifact_digest,
                            artifact_format: match component.artifact_format {
                                ProviderArtifactFormat::Raw => PluginArtifactFormat::Raw,
                                ProviderArtifactFormat::TarGz => PluginArtifactFormat::TarGz,
                            },
                            entrypoint: component.entrypoint,
                            probe: PluginArtifactProbe {
                                args: component.probe.args,
                                timeout_ms: component.probe.timeout_ms,
                            },
                        })
                        .collect(),
                })
                .collect(),
        };
        let host_bundle = PluginHostBundle {
            schema: crate::plugin_host_bundle::HOST_BUNDLE_SCHEMA.to_owned(),
            plugin_id: package.manifest.id.clone(),
            plugin_version: package.manifest.version.clone(),
            package_digest: release.package_digest.clone(),
            files: BTreeMap::from([
                (
                    "host.json".to_owned(),
                    r#"{"schema_version":1,"adapter_slot":"gemini","usage":{"account":"fixture","collector":"command","collector_argv":["@plugin-js","run","--allow-env=COWBOY_PLUGIN_COMMAND_GEMINI,HOME","--allow-run=${ENV:COWBOY_PLUGIN_COMMAND_GEMINI}","${PLUGIN_DIR}/collector/index.js"]}}"#
                        .to_owned(),
                ),
                (
                    "collector/index.js".to_owned(),
                    r#"const request = JSON.parse(await new Response(Deno.stdin.readable).text()); console.log(JSON.stringify({operation: request.operation, command: Deno.env.get("COWBOY_PLUGIN_COMMAND_GEMINI"), home: Deno.env.get("HOME")}));"#
                        .to_owned(),
                ),
            ]),
        };
        host_bundle.validate().unwrap();
        let host_bundle_bytes = serde_json::to_vec(&host_bundle).unwrap();
        release.host_bundle_digest = Some(format!(
            "sha256:{:x}",
            Sha256::digest(host_bundle_bytes.as_slice())
        ));
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
                cowboy_plugin_sdk::PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
                &release.proof(),
            )
            .unwrap();
        let desired = DesiredPlugin {
            release: release.clone(),
            package_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            publisher_public_key: publisher.public_key().to_owned(),
            host_bundle_base64: Some(
                base64::engine::general_purpose::STANDARD.encode(&host_bundle_bytes),
            ),
        };
        let store =
            MachinePluginStore::new(&root.join("machine"), Platform::Linux, "x86_64".to_owned())
                .unwrap();
        let installed = store.install(&desired).await.unwrap();
        assert_eq!(installed.generation_digest, release.artifact_digest);
        let service_signer =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("service")).unwrap();
        let bundle = PortableCredentialBundle {
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            method_id: "api-key".to_owned(),
            values: BTreeMap::from([(
                "api_key".to_owned(),
                base64::engine::general_purpose::STANDARD.encode(b"fixture-api-key"),
            )]),
        };
        let envelope = seal_auth_for_test(&store, &package, &service_signer, 1, &bundle);
        let first_receipt = store.apply_auth(&envelope).await.unwrap();
        assert!(first_receipt.auth_generation_advanced);
        let collected = store
            .invoke_host(
                "gemini",
                &package.manifest.version,
                &release.artifact_digest,
                Some(1),
                PluginHostOperation::CollectUsage,
                serde_json::json!({ "operation": "spoofed" }),
            )
            .await
            .unwrap();
        assert_eq!(collected["operation"], "collect");
        assert!(
            collected["command"].as_str().is_some_and(
                |command| command.contains("/generations/") && command.ends_with("/bin")
            )
        );
        assert!(
            collected["home"]
                .as_str()
                .is_some_and(|home| home.ends_with("/runtime/generations/1/home"))
        );
        let wrong_auth = store
            .invoke_host(
                "gemini",
                &package.manifest.version,
                &release.artifact_digest,
                Some(2),
                PluginHostOperation::CollectUsage,
                serde_json::json!({}),
            )
            .await
            .unwrap_err();
        assert!(!wrong_auth.started);
        let installed_collector = root
            .join("machine/plugins/gemini/generations")
            .join(digest_generation_name(&release.artifact_digest).unwrap())
            .join("content/host/collector/index.js");
        fs::write(&installed_collector, b"console.log('{}')").unwrap();
        let tampered = store
            .invoke_host(
                "gemini",
                &package.manifest.version,
                &release.artifact_digest,
                Some(1),
                PluginHostOperation::CollectUsage,
                serde_json::json!({}),
            )
            .await
            .unwrap_err();
        assert!(!tampered.started);
        assert!(tampered.error.to_string().contains("integrity"));
        fs::write(
            &installed_collector,
            host_bundle.files["collector/index.js"].as_bytes(),
        )
        .unwrap();
        let replayed_receipt = store.apply_auth(&envelope).await.unwrap();
        assert!(!replayed_receipt.auth_generation_advanced);
        let auth_root = root.join("machine/provider-auth/providers/gemini");
        fs::remove_dir_all(auth_root.join("materialized/generations/1")).unwrap();
        fs::remove_dir_all(auth_root.join("runtime/generations/1")).unwrap();
        let context = store
            .launch_context("gemini", &release.artifact_digest, Some(1))
            .unwrap();
        assert_eq!(context.environment["GEMINI_API_KEY"], "fixture-api-key");
        assert!(context.home.unwrap().is_dir());
        assert!(
            auth_root
                .join("materialized/generations/1/metadata.json")
                .is_file()
        );
        let legacy_session =
            auth_root.join("materialized/generations/1/home/.gemini/sessions/native-session.json");
        fs::create_dir_all(legacy_session.parent().unwrap()).unwrap();
        fs::write(&legacy_session, b"legacy-native-session").unwrap();
        let refreshed_bundle = PortableCredentialBundle {
            portable_schema: bundle.portable_schema.clone(),
            method_id: bundle.method_id.clone(),
            values: BTreeMap::from([(
                "api_key".to_owned(),
                base64::engine::general_purpose::STANDARD.encode(b"refreshed-fixture-api-key"),
            )]),
        };
        let refreshed_envelope =
            seal_auth_for_test(&store, &package, &service_signer, 2, &refreshed_bundle);
        let refreshed_receipt = store.apply_auth(&refreshed_envelope).await.unwrap();
        assert!(refreshed_receipt.auth_generation_advanced);
        assert!(!auth_root.join("replicas/1.sealed.json").exists());
        assert_eq!(
            read_link_name(&auth_root.join("materialized/current")).as_deref(),
            Some("2")
        );
        fs::remove_dir_all(auth_root.join("runtime/generations/1")).unwrap();
        let historical = store
            .launch_context("gemini", &release.artifact_digest, Some(1))
            .unwrap();
        assert_eq!(
            historical.environment["GEMINI_API_KEY"],
            "refreshed-fixture-api-key"
        );
        let historical_home = historical.home.unwrap();
        assert_eq!(
            historical_home,
            auth_root.join("runtime/generations/1/home")
        );
        assert_eq!(
            fs::read(historical_home.join(".gemini/sessions/native-session.json")).unwrap(),
            b"legacy-native-session"
        );
        assert!(
            fs::symlink_metadata(
                auth_root.join("materialized/generations/1/home/.gemini/sessions")
            )
            .unwrap()
            .file_type()
            .is_symlink()
        );
        assert_eq!(
            read_link_name(&auth_root.join("materialized/current")).as_deref(),
            Some("2")
        );
        let command = store
            .authentication_component_command(
                "gemini",
                &cowboy_provider_sdk::AuthComponent {
                    kind: cowboy_provider_sdk::PrivateComponentKind::ProviderCli,
                    slot: "gemini".to_owned(),
                },
            )
            .unwrap();
        assert!(command.starts_with(root.join("machine/plugins/gemini/generations")));
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
    #[allow(clippy::too_many_lines)]
    async fn zed_uses_the_same_signed_plugin_generation_lifecycle() {
        use cowboy_plugin_sdk::{
            CodeIntelligenceContract, PluginArtifactFormat, PluginArtifactProbe,
            PluginComponentKind, PluginManifest, PluginPackage, PluginPayload, PluginRelease,
            PluginRuntimeArtifacts, RELEASE_SCHEMA_MIN_VERSION, ReleasedPluginComponent,
        };
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let script = b"#!/bin/sh\nexit 0\n".to_vec();
        let script_digest = format!("sha256:{:x}", Sha256::digest(&script));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        script.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            stream.write_all(&script).await.unwrap();
        });
        let manifest: PluginManifest =
            serde_json::from_str(include_str!("../plugins/zed/plugin.json")).unwrap();
        let component_release = manifest.component_release.clone();
        let plugin_version = manifest.version.clone();
        let mut contract: CodeIntelligenceContract =
            serde_json::from_str(include_str!("../plugins/zed/contract.json")).unwrap();
        // Retained schema-1 releases remain readable during runtime migration.
        contract.schema_version = 1;
        contract.runtime = None;
        let package = PluginPackage::new(
            manifest,
            component_release.clone(),
            PluginPayload::CodeIntelligence(contract),
        )
        .unwrap();
        let bytes = package.canonical_bytes().unwrap();
        let mut release = PluginRelease {
            release_schema: RELEASE_SCHEMA_MIN_VERSION,
            plugin_id: "zed".to_owned(),
            plugin_version,
            plugin_kind: cowboy_plugin_sdk::PluginKind::CodeIntelligence,
            package_digest: PluginPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: "https://example.invalid/zed.cowboy-plugin".to_owned(),
            publisher: package.manifest.publisher.clone(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            component_release,
            host_bundle_digest: None,
            signature: String::new(),
            supported_platforms: vec![cowboy_provider_sdk::PlatformTarget {
                os: OperatingSystem::Linux,
                architecture: Architecture::X86_64,
            }],
            runtime_artifacts: vec![PluginRuntimeArtifacts {
                os: OperatingSystem::Linux,
                architecture: Architecture::X86_64,
                components: vec![ReleasedPluginComponent {
                    kind: PluginComponentKind::CodeIntelligenceAdapter,
                    slot: "zed".to_owned(),
                    dependency: "cowboy-zed-adapter".to_owned(),
                    version: "1.0.0".to_owned(),
                    command: "cowboy-zed-adapter".to_owned(),
                    artifact_url: format!("http://{address}/runtime"),
                    artifact_digest: script_digest,
                    artifact_format: PluginArtifactFormat::Raw,
                    entrypoint: None,
                    probe: PluginArtifactProbe {
                        args: vec!["--help".to_owned()],
                        timeout_ms: 1_000,
                    },
                }],
            }],
        };
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        let root = std::env::temp_dir().join(format!(
            "cowboy-zed-plugin-install-test-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("publisher")).unwrap();
        release.signature = publisher
            .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())
            .unwrap();
        let store =
            MachinePluginStore::new(&root.join("machine"), Platform::Linux, "x86_64".to_owned())
                .unwrap();
        let installed = store
            .install(&DesiredPlugin {
                release: release.clone(),
                package_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                publisher_public_key: publisher.public_key().to_owned(),
                host_bundle_base64: None,
            })
            .await
            .unwrap();
        assert_eq!(
            installed.plugin_kind,
            cowboy_plugin_sdk::PluginKind::CodeIntelligence
        );
        assert_eq!(installed.generation_digest, release.artifact_digest);
        assert!(root.join("machine/plugins/zed/active").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires the exact portable Zed release binaries; run just zed-plugin-conformance"]
    #[allow(clippy::too_many_lines)]
    async fn released_zed_runtime_installs_and_drains() {
        use cowboy_plugin_sdk::{
            PluginArtifactProbe, PluginManifest, PluginRelease, ReleasedPluginComponent,
        };
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let manifest: PluginManifest =
            serde_json::from_str(include_str!("../plugins/zed/plugin.json")).unwrap();
        let contract: cowboy_plugin_sdk::CodeIntelligenceContract =
            serde_json::from_str(include_str!("../plugins/zed/contract.json")).unwrap();
        let runtime = contract.runtime.clone().expect("owned Zed runtime");
        let package = PluginPackage::new(
            manifest.clone(),
            manifest.component_release.clone(),
            PluginPayload::CodeIntelligence(contract.clone()),
        )
        .unwrap();
        let bytes = package.canonical_bytes().unwrap();
        let mut binaries = Vec::new();
        for name in ["COWBOY_TEST_ZED_ADAPTER", "COWBOY_TEST_ZED_SERVER"] {
            let path =
                PathBuf::from(std::env::var_os(name).expect("explicit Zed conformance binary"));
            assert!(path.is_absolute(), "conformance binaries must be absolute");
            binaries.push(fs::read(path).unwrap());
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let components = runtime
            .components
            .iter()
            .zip(&binaries)
            .enumerate()
            .map(|(index, (component, binary))| ReleasedPluginComponent {
                kind: component.kind,
                slot: component.slot.clone(),
                dependency: component.dependency.clone(),
                version: component.version.clone(),
                command: component.command.clone(),
                artifact_url: format!("http://{address}/{index}"),
                artifact_digest: PluginPackage::artifact_digest(binary),
                artifact_format: PluginArtifactFormat::Raw,
                entrypoint: None,
                probe: PluginArtifactProbe {
                    args: vec![if index == 0 { "--help" } else { "version" }.to_owned()],
                    timeout_ms: 30_000,
                },
            })
            .collect();
        let server = tokio::spawn(async move {
            for _ in 0..binaries.len() {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 4096];
                let count = stream.read(&mut request).await.unwrap();
                let request = std::str::from_utf8(&request[..count]).unwrap();
                let index = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .trim_start_matches('/')
                    .parse::<usize>()
                    .unwrap();
                let binary = &binaries[index];
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            binary.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                stream.write_all(binary).await.unwrap();
            }
        });
        let root = std::env::temp_dir().join(format!(
            "cowboy-zed-conformance-{:032x}",
            rand::random::<u128>()
        ));
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("publisher")).unwrap();
        let mut release = PluginRelease {
            release_schema: 1,
            plugin_id: manifest.id,
            plugin_version: manifest.version,
            plugin_kind: manifest.kind,
            package_digest: PluginPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: "https://example.invalid/zed.cowboy-plugin".to_owned(),
            publisher: manifest.publisher,
            contract_fingerprint: package.contract_fingerprint.clone(),
            component_release: manifest.component_release,
            host_bundle_digest: None,
            signature: String::new(),
            supported_platforms: contract.supported_platforms,
            runtime_artifacts: vec![PluginRuntimeArtifacts {
                os: OperatingSystem::Linux,
                architecture: Architecture::X86_64,
                components,
            }],
        };
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        release.signature = publisher
            .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())
            .unwrap();
        let store =
            MachinePluginStore::new(&root.join("machine"), Platform::Linux, "x86_64".to_owned())
                .unwrap();
        let installed = store
            .install(&DesiredPlugin {
                release: release.clone(),
                package_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                publisher_public_key: publisher.public_key().to_owned(),
                host_bundle_base64: None,
            })
            .await
            .unwrap();
        assert_eq!(installed.generation_digest, release.artifact_digest);
        server.await.unwrap();
        let worktree = root.join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(worktree.join("fixture.txt"), "real isolated Zed buffer\n").unwrap();
        let open = serde_json::json!({"type": "openWorktree", "path": worktree, "trusted": true});
        assert_eq!(
            store.code_request("zed", &open, None).await.unwrap()["leases"],
            1
        );
        let open_buffer = serde_json::json!({"type": "openBuffer", "worktree": worktree, "path": "fixture.txt", "leaseId": "conformance"});
        assert_eq!(
            store.code_request("zed", &open_buffer, None).await.unwrap()["leases"],
            1
        );
        assert_eq!(store.code_runtimes.live_generation_count().await, 1);
        store
            .uninstall("zed", &release.artifact_digest)
            .await
            .unwrap();
        assert!(store.inventory().unwrap().is_empty());
        let other = serde_json::json!({"type": "openWorktree", "path": root, "trusted": true});
        assert!(
            store
                .code_request("zed", &other, Some(Path::new("/must-not-use-legacy.sock")))
                .await
                .unwrap_err()
                .to_string()
                .contains("not installed")
        );
        let close_buffer = serde_json::json!({"type": "closeBuffer", "worktree": worktree, "path": "fixture.txt", "leaseId": "conformance"});
        assert_eq!(
            store
                .code_request("zed", &close_buffer, None)
                .await
                .unwrap()["leases"],
            0
        );
        assert_eq!(store.code_runtimes.live_generation_count().await, 1);
        let close = serde_json::json!({"type": "closeWorktree", "path": worktree});
        assert_eq!(
            store.code_request("zed", &close, None).await.unwrap()["leases"],
            0
        );
        assert_eq!(store.code_runtimes.live_generation_count().await, 0);
        let restored = store
            .reactivate("zed", &release.artifact_digest)
            .await
            .unwrap();
        assert_eq!(restored.generation_digest, release.artifact_digest);
        store
            .uninstall("zed", &release.artifact_digest)
            .await
            .unwrap();
        drop(store);
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
        let library = b"export const version = 1;\n";
        header.set_size(library.len() as u64);
        header.set_cksum();
        builder
            .append_data(&mut header, "lib/runtime.js", library.as_slice())
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
        atomic_write(&executable, script, 0o700).unwrap();
        assert!(installed_runtime_matches(&root, &artifacts, &metadata).unwrap());
        atomic_write(&extracted.join("lib/runtime.js"), b"injected helper", 0o600).unwrap();
        assert!(!installed_runtime_matches(&root, &artifacts, &metadata).unwrap());
        atomic_write(&extracted.join("lib/runtime.js"), library, 0o600).unwrap();
        atomic_write(&extracted.join("lib/injected.js"), b"extra module", 0o600).unwrap();
        assert!(!installed_runtime_matches(&root, &artifacts, &metadata).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generation_names_are_content_addressed_and_path_safe() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        assert_eq!(digest_generation_name(&digest).unwrap(), "ab".repeat(32));
        assert!(digest_generation_name("sha256:../../active").is_err());
        assert!(digest_generation_name(&format!("sha512:{}", "ab".repeat(32))).is_err());
        assert!(validate_plugin_id(".").is_err());
        assert!(validate_plugin_id("..").is_err());
    }

    #[test]
    fn auth_events_include_atomic_generation_switches_but_exclude_other_state() {
        let root = PathBuf::from("/state/provider-auth/providers");
        let credential = root.join("grok/runtime/generations/3/home/.grok/auth.json");
        assert!(auth_event_paths_intersect(
            &root,
            std::slice::from_ref(&credential),
            std::slice::from_ref(&credential)
        ));
        assert!(auth_event_paths_intersect(
            &root,
            std::slice::from_ref(&credential),
            &[credential.parent().unwrap().join(".auth.json.partial")]
        ));
        assert!(auth_event_paths_intersect(
            &root,
            std::slice::from_ref(&credential),
            &[root.join("grok/runtime/generations/3")]
        ));
        assert!(!auth_event_paths_intersect(
            &root,
            std::slice::from_ref(&credential),
            &[PathBuf::from("/state/plugins/grok/runtime.log")]
        ));
    }

    #[test]
    fn legacy_provider_home_moves_runtime_state_without_copying_credentials() {
        use cowboy_provider_sdk::{StandardProviderSource, build_package};

        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../plugins/codex/provider.json")).unwrap();
        let package = build_package(source.compile().unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "cowboy-legacy-provider-home-test-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let legacy = root.join("materialized/home");
        let runtime = root.join("runtime/home");
        fs::create_dir_all(legacy.join(".codex/sessions/2026/08/31")).unwrap();
        fs::create_dir_all(runtime.join(".codex")).unwrap();
        fs::write(legacy.join(".codex/auth.json"), b"sealed-auth").unwrap();
        fs::write(runtime.join(".codex/auth.json"), b"runtime-auth").unwrap();
        fs::write(
            legacy.join(".codex/sessions/2026/08/31/rollout.jsonl"),
            b"legacy-rollout",
        )
        .unwrap();
        fs::write(legacy.join(".codex/state.sqlite"), b"legacy-state").unwrap();
        fs::write(runtime.join(".codex/state.sqlite"), b"runtime-state").unwrap();

        let migrated =
            migrate_legacy_runtime_state(&package.manifest.authentication, &legacy, &runtime)
                .unwrap();
        assert!(migrated > 0);
        assert_eq!(
            fs::read(legacy.join(".codex/auth.json")).unwrap(),
            b"sealed-auth"
        );
        assert_eq!(
            fs::read(runtime.join(".codex/auth.json")).unwrap(),
            b"runtime-auth"
        );
        assert_eq!(
            fs::read(runtime.join(".codex/sessions/2026/08/31/rollout.jsonl")).unwrap(),
            b"legacy-rollout"
        );
        assert!(
            fs::symlink_metadata(legacy.join(".codex/sessions"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(legacy.join(".codex/state.sqlite")).unwrap(),
            b"legacy-state"
        );
        assert_eq!(
            fs::read(runtime.join(".codex/state.sqlite")).unwrap(),
            b"runtime-state"
        );
        assert_eq!(
            migrate_legacy_runtime_state(&package.manifest.authentication, &legacy, &runtime,)
                .unwrap(),
            0
        );
        fs::remove_dir_all(root).unwrap();
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
    fn runtime_projection_preserves_refresh_until_service_generation_advances() {
        use cowboy_provider_sdk::{StandardProviderSource, build_package};

        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../plugins/grok/provider.json")).unwrap();
        let package = build_package(source.compile().unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "cowboy-grok-auth-repair-test-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let store = MachinePluginStore::new(&root, Platform::Linux, "x86_64".to_owned()).unwrap();
        let original = br#"{"account":{"key":"sealed-token"}}"#;
        let bundle = PortableCredentialBundle {
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            method_id: "xai-account".to_owned(),
            values: BTreeMap::from([(
                "auth_json".to_owned(),
                base64::engine::general_purpose::STANDARD.encode(original),
            )]),
        };

        store.materialize_bundle(&package, 1, &bundle).unwrap();
        assert!(store.auth_watch_root().is_dir());
        let materialized_generation = root
            .join("provider-auth/providers/grok/materialized/generations/1/home/.grok/auth.json");
        let runtime_generation = root.join("provider-auth/providers/grok/runtime/generations/1");
        let runtime_auth_path = runtime_generation.join("home/.grok/auth.json");
        assert_eq!(fs::read(&materialized_generation).unwrap(), original);
        assert_eq!(fs::read(&runtime_auth_path).unwrap(), original);
        let refreshed = br#"{"account":{"key":"runtime-refreshed-token"}}"#;
        fs::remove_dir_all(root.join("provider-auth/providers/grok/runtime")).unwrap();
        fs::write(&materialized_generation, refreshed).unwrap();
        store.materialize_bundle(&package, 1, &bundle).unwrap();
        assert_eq!(fs::read(&materialized_generation).unwrap(), original);
        assert_eq!(fs::read(&runtime_auth_path).unwrap(), refreshed);
        let projected = projected_credential_bundle(
            &package.manifest.authentication,
            &runtime_generation,
            "xai-account",
        )
        .unwrap();
        assert_eq!(
            projected.values["auth_json"],
            base64::engine::general_purpose::STANDARD.encode(refreshed)
        );
        let runtime_state = runtime_auth_path
            .parent()
            .unwrap()
            .join("active_sessions.json");
        fs::write(&runtime_state, b"[]").unwrap();
        store.materialize_bundle(&package, 1, &bundle).unwrap();
        assert_eq!(fs::read(&materialized_generation).unwrap(), original);
        assert_eq!(fs::read(&runtime_auth_path).unwrap(), refreshed);

        fs::remove_file(&runtime_auth_path).unwrap();
        assert!(
            projected_credential_bundle(
                &package.manifest.authentication,
                &runtime_generation,
                "xai-account",
            )
            .unwrap_err()
            .to_string()
            .contains("method-required credential")
        );
        store.materialize_bundle(&package, 1, &bundle).unwrap();
        assert_eq!(fs::read(&runtime_auth_path).unwrap(), original);
        assert_eq!(fs::read(&runtime_state).unwrap(), b"[]");

        let service_refreshed = br#"{"account":{"key":"service-generation-two"}}"#;
        let next_bundle = PortableCredentialBundle {
            portable_schema: bundle.portable_schema.clone(),
            method_id: bundle.method_id.clone(),
            values: BTreeMap::from([(
                "auth_json".to_owned(),
                base64::engine::general_purpose::STANDARD.encode(service_refreshed),
            )]),
        };
        store.materialize_bundle(&package, 2, &next_bundle).unwrap();
        assert_eq!(fs::read(&runtime_auth_path).unwrap(), service_refreshed);
        assert_eq!(fs::read(&runtime_state).unwrap(), b"[]");
        let second_runtime_auth =
            root.join("provider-auth/providers/grok/runtime/generations/2/home/.grok/auth.json");
        assert_eq!(fs::read(&second_runtime_auth).unwrap(), service_refreshed);
        assert!(
            fs::symlink_metadata(&second_runtime_auth)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(&second_runtime_auth).unwrap(),
            runtime_auth_path
        );
        assert_eq!(
            read_link_name(&root.join("provider-auth/providers/grok/runtime/current")).as_deref(),
            Some("1")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn noncanonical_refresh_survives_until_the_service_cas_wins() {
        use cowboy_provider_sdk::{StandardProviderSource, build_package};

        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../plugins/grok/provider.json")).unwrap();
        let package = build_package(source.compile().unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "cowboy-grok-auth-candidate-test-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let store = MachinePluginStore::new(&root, Platform::Linux, "x86_64".to_owned()).unwrap();
        let bundle_for = |credential: &[u8]| PortableCredentialBundle {
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            method_id: "xai-account".to_owned(),
            values: BTreeMap::from([(
                "auth_json".to_owned(),
                base64::engine::general_purpose::STANDARD.encode(credential),
            )]),
        };
        let sealed = br#"{"account":{"key":"sealed"}}"#;
        let service_two = br#"{"account":{"key":"service-two"}}"#;
        store
            .materialize_bundle(&package, 1, &bundle_for(sealed))
            .unwrap();
        store
            .materialize_bundle(&package, 2, &bundle_for(service_two))
            .unwrap();
        let runtime_root = root.join("provider-auth/providers/grok/runtime/generations");
        let canonical = runtime_root.join("1/home/.grok/auth.json");
        let noncanonical = runtime_root.join("2/home/.grok/auth.json");

        // A Provider may atomically replace a symlink instead of writing through
        // it. Preserve that complete refresh candidate until Service CAS resolves.
        let candidate = br#"{"account":{"key":"runtime-two-refreshed"}}"#;
        fs::remove_file(&noncanonical).unwrap();
        fs::write(&noncanonical, candidate).unwrap();
        store
            .materialize_bundle(&package, 2, &bundle_for(service_two))
            .unwrap();
        assert_eq!(fs::read(&noncanonical).unwrap(), candidate);
        assert!(
            !fs::symlink_metadata(&noncanonical)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        let service_winner = br#"{"account":{"key":"service-three"}}"#;
        store
            .materialize_bundle(&package, 3, &bundle_for(service_winner))
            .unwrap();
        assert_eq!(fs::read(&canonical).unwrap(), service_winner);
        assert_eq!(fs::read(&noncanonical).unwrap(), service_winner);
        assert!(
            fs::symlink_metadata(&noncanonical)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_canonical_credentials_recover_before_older_aliases() {
        use cowboy_provider_sdk::{StandardProviderSource, build_package};

        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../plugins/grok/provider.json")).unwrap();
        let package = build_package(source.compile().unwrap()).unwrap();
        for next_generation in [18, 19] {
            let root = std::env::temp_dir().join(format!(
                "cowboy-grok-auth-source-repair-{}-{}",
                std::process::id(),
                ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let store =
                MachinePluginStore::new(&root, Platform::Linux, "x86_64".to_owned()).unwrap();
            let bundle = PortableCredentialBundle {
                portable_schema: package.manifest.authentication.portable_schema.clone(),
                method_id: "xai-account".to_owned(),
                values: BTreeMap::from([(
                    "auth_json".to_owned(),
                    base64::engine::general_purpose::STANDARD.encode(b"service-credential"),
                )]),
            };
            // A retained older session can acquire an alias after the canonical
            // runtime has already been selected. Numeric order is not dependency order.
            for generation in [3, 1, 18] {
                store
                    .materialize_bundle(&package, generation, &bundle)
                    .unwrap();
            }
            let runtime = root.join("provider-auth/providers/grok/runtime");
            let canonical = runtime.join("generations/3/home/.grok/auth.json");
            let state = runtime.join("generations/3/home/.grok/active_sessions.json");
            fs::write(&state, b"[]").unwrap();
            fs::remove_file(&canonical).unwrap();

            // Cover both same-generation repair and a newer Service bundle.
            store
                .materialize_bundle(&package, next_generation, &bundle)
                .unwrap();
            for generation in [1, 3, 18, next_generation] {
                assert_eq!(
                    fs::read(
                        runtime.join(format!("generations/{generation}/home/.grok/auth.json"))
                    )
                    .unwrap(),
                    b"service-credential"
                );
            }
            assert_eq!(fs::read(&state).unwrap(), b"[]");
            assert_eq!(
                read_link_name(&runtime.join("current")).as_deref(),
                Some("3")
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn runtime_credential_source_uses_numeric_generation_order_during_migration() {
        use cowboy_provider_sdk::{StandardProviderSource, build_package};

        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../plugins/grok/provider.json")).unwrap();
        let package = build_package(source.compile().unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "cowboy-grok-auth-order-test-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let store = MachinePluginStore::new(&root, Platform::Linux, "x86_64".to_owned()).unwrap();
        let bundle = PortableCredentialBundle {
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            method_id: "xai-account".to_owned(),
            values: BTreeMap::from([(
                "auth_json".to_owned(),
                base64::engine::general_purpose::STANDARD.encode(b"auth"),
            )]),
        };

        store.materialize_bundle(&package, 2, &bundle).unwrap();
        store.materialize_bundle(&package, 10, &bundle).unwrap();
        let runtime_root = root.join("provider-auth/providers/grok/runtime");
        fs::remove_file(runtime_root.join("current")).unwrap();

        assert_eq!(
            store
                .canonical_runtime_projection(&package)
                .unwrap()
                .unwrap(),
            runtime_root.join("generations/2")
        );
        assert_eq!(
            read_link_name(&runtime_root.join("current")).as_deref(),
            Some("2")
        );
        fs::remove_dir_all(root).unwrap();
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
        let non_contributory = SealedProviderAuth {
            envelope_schema: 1,
            provider_id: "gemini".to_owned(),
            auth_generation: 1,
            auth_contract_fingerprint: format!("sha256:{}", "ab".repeat(32)),
            projection_schema: "gemini-auth-v1".to_owned(),
            action: ProviderAuthAction::Apply,
            ephemeral_public_key: base64::engine::general_purpose::STANDARD.encode([0_u8; 32]),
            nonce: base64::engine::general_purpose::STANDARD.encode([0_u8; 24]),
            ciphertext: base64::engine::general_purpose::STANDARD.encode([0_u8; 16]),
            service_public_key: String::new(),
            signature: String::new(),
        };
        assert!(
            first
                .open(&non_contributory)
                .unwrap_err()
                .to_string()
                .contains("non-contributory")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
