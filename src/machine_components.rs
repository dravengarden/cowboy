//! Content-addressed Machine payload activation.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use sha2::{Digest as _, Sha256};

use crate::machine_protocol::{
    ArtifactFormat, ComponentInventory, ComponentState, DesiredComponent,
};

use crate::component_proof::{component_proof, component_slot};

mod cached_host;
mod host_payload;

pub(crate) use cached_host::check_portable_host_cache;

pub struct ComponentStore {
    root: PathBuf,
    publisher_key: Option<String>,
    bootstrap_acp_generation: String,
    max_cache_bytes: u64,
    max_unused_age: std::time::Duration,
}

impl ComponentStore {
    pub fn new(
        root: PathBuf,
        publisher_key_path: Option<&Path>,
        bootstrap_acp_generation: String,
    ) -> anyhow::Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let publisher_key = publisher_key_path
            .map(std::fs::read_to_string)
            .transpose()
            .context("reading component publisher key")?;
        std::fs::create_dir_all(root.join("payloads"))?;
        std::fs::create_dir_all(root.join("active"))?;
        std::fs::create_dir_all(root.join("rollback"))?;
        std::fs::create_dir_all(root.join("commands"))?;
        Ok(Self {
            root,
            publisher_key,
            bootstrap_acp_generation,
            max_cache_bytes: 10 * 1024 * 1024 * 1024,
            max_unused_age: std::time::Duration::from_secs(30 * 24 * 60 * 60),
        })
    }

    #[must_use]
    pub fn bootstrap_acp_generation(&self) -> &str {
        &self.bootstrap_acp_generation
    }

    pub async fn reconcile(&self, desired: DesiredComponent) -> anyhow::Result<ComponentInventory> {
        let floor_at_start =
            if desired.id.kind == crate::machine_protocol::ComponentKind::MachineHost {
                crate::session_deletion_admission::reader_floor::read(
                    self.root
                        .parent()
                        .context("component store has no Machine state parent")?,
                )?
            } else {
                None
            };
        self.check_session_deletion_host_selection(&desired)?;
        let publisher_key = self
            .publisher_key
            .as_deref()
            .context("component updates require --artifact-public-key")?;
        let signature = desired
            .signature
            .as_deref()
            .context("component artifact is unsigned")?;
        let url = reqwest::Url::parse(&desired.artifact_url)?;
        let loopback = matches!(url.host_str(), Some("127.0.0.1" | "::1" | "localhost"));
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            bail!("component artifact must use HTTPS");
        }
        let bytes = reqwest::Client::new()
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if digest != desired.digest.to_ascii_lowercase() {
            bail!("component digest mismatch");
        }
        let proof = component_proof(&desired);
        if !crate::machine_auth::verify(publisher_key, &proof, signature)? {
            bail!("component signature is invalid");
        }
        self.check_session_deletion_host_selection(&desired)?;
        self.check_retained_floor(floor_at_start.as_ref())?;
        let host_payload = host_payload::HostPayload::from_authenticated(&desired, &bytes)?;
        let slot = component_slot(&desired);
        let generation = self
            .root
            .join("payloads")
            .join(&slot)
            .join(&desired.version)
            .join(&digest);
        std::fs::create_dir_all(&generation)?;
        let executable = component_executable(&generation, &desired)?;
        if !executable.exists() {
            match desired.artifact_format {
                ArtifactFormat::Raw => {
                    let temporary = generation.join(".bin.partial");
                    std::fs::write(&temporary, &bytes)?;
                    set_executable(&temporary)?;
                    std::fs::rename(temporary, &executable)?;
                }
                ArtifactFormat::TarGz => extract_tar_gz(&generation, &bytes, &executable)?,
            }
        }
        if let Some(payload) = &host_payload {
            payload.verify(&generation)?;
            // Retain the authenticated envelope bytes for offline startup
            // verification. Never reconstruct archive expectations from cache.
            retain_host_artifact(&generation, &bytes)?;
        }
        std::fs::write(
            generation.join("manifest.json"),
            serde_json::to_vec_pretty(&desired)?,
        )?;
        if desired.automatic && desired.probe.is_none() {
            bail!("automatic component activation requires a health probe");
        }
        if let Some(probe) = &desired.probe {
            self.check_session_deletion_host_selection(&desired)?;
            let timeout = probe.timeout_ms.clamp(100, 120_000);
            let mut child = spawn_staged_probe(&executable, &generation, &probe.args).await?;
            let status =
                match tokio::time::timeout(std::time::Duration::from_millis(timeout), child.wait())
                    .await
                {
                    Ok(status) => status.context("waiting for staged component health probe")?,
                    Err(_) => {
                        let _ = child.kill().await;
                        bail!("staged component health probe timed out after {timeout}ms");
                    }
                };
            if !status.success() {
                bail!("staged component health probe exited with {status}");
            }
        }
        self.check_session_deletion_host_selection(&desired)?;
        self.check_retained_floor(floor_at_start.as_ref())?;
        // A signed probe may mutate its own staging directory. Verify again
        // before publishing, without repairing or executing substituted bytes.
        if let Some(payload) = &host_payload {
            payload.verify(&generation)?;
            let cached = cached_host::verify_generation(&self.root, &generation, publisher_key)?;
            if cached != desired {
                bail!("staged Machine host manifest changed during probe");
            }
            cached_host::retain_reader_floor(&self.root, &desired, publisher_key)?;
        }
        let active = self.root.join("active").join(&slot);
        let prior_generation = std::fs::read_link(&active).ok();
        let rollback_generation = prior_generation.as_ref().and_then(|target| {
            target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        if let Some(prior_generation) = prior_generation {
            replace_symlink(
                &self.root.join("rollback").join(&slot),
                &prior_generation,
                &format!(".{slot}.rollback-next"),
            )?;
        }
        let temporary_link = self.root.join("active").join(format!(".{slot}.next"));
        let _ = std::fs::remove_file(&temporary_link);
        std::os::unix::fs::symlink(&generation, &temporary_link)?;
        std::fs::rename(&temporary_link, &active)?;
        if let Some(command) = component_command(&desired) {
            replace_symlink(
                &self.root.join("commands").join(&command),
                &executable,
                &format!(".{command}.next"),
            )?;
        }
        self.prune()?;
        Ok(ComponentInventory {
            id: desired.id,
            state: ComponentState::Active,
            version: desired.version,
            generation: desired.generation,
            digest,
            rollback_generation,
            active_leases: 0,
            auth: None,
            detail: None,
            update: None,
            superseded_by: None,
        })
    }

    fn check_session_deletion_host_selection(
        &self,
        desired: &DesiredComponent,
    ) -> anyhow::Result<()> {
        desired
            .validate_session_deletion_declaration()
            .map_err(anyhow::Error::msg)?;
        if desired.id.kind == crate::machine_protocol::ComponentKind::MachineHost {
            crate::session_deletion_admission::require_empty_portable_namespace(
                self.root
                    .parent()
                    .context("component store has no Machine state parent")?,
            )?;
            cached_host::check_floor_candidate(&self.root, desired, self.publisher_key.as_deref())?;
        }
        Ok(())
    }

    fn check_retained_floor(
        &self,
        retained: Option<&crate::session_deletion_admission::reader_floor::Floor>,
    ) -> anyhow::Result<()> {
        if let Some(retained) = retained {
            let current = crate::session_deletion_admission::reader_floor::read(
                self.root
                    .parent()
                    .context("component store has no Machine state parent")?,
            )?;
            anyhow::ensure!(
                current.as_ref() == Some(retained),
                "portable reader floor changed or disappeared during reconciliation"
            );
        }
        Ok(())
    }

    pub fn active(&self) -> anyhow::Result<Vec<(DesiredComponent, PathBuf)>> {
        let mut active = Vec::new();
        for entry in std::fs::read_dir(self.root.join("active"))? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = std::fs::canonicalize(entry.path())?;
            let desired = serde_json::from_slice::<DesiredComponent>(&std::fs::read(
                path.join("manifest.json"),
            )?)?;
            let executable = component_executable(&path, &desired)?;
            active.push((desired, executable));
        }
        active.sort_by_key(|(component, _)| component_slot(component));
        Ok(active)
    }

    pub fn rollback_generation(&self, desired: &DesiredComponent) -> Option<String> {
        let target =
            std::fs::read_link(self.root.join("rollback").join(component_slot(desired))).ok()?;
        let manifest = std::fs::read(target.join("manifest.json")).ok()?;
        serde_json::from_slice::<DesiredComponent>(&manifest)
            .ok()
            .map(|component| component.generation)
    }

    #[must_use]
    pub fn command_path(&self, command: &str) -> PathBuf {
        self.root.join("commands").join(command)
    }

    fn prune(&self) -> anyhow::Result<()> {
        let mut protected = Vec::new();
        if let Some(floor) = crate::session_deletion_admission::reader_floor::read(
            self.root
                .parent()
                .context("component store has no Machine state parent")?,
        )? {
            protected.push(floor.anchor_path(&self.root));
        }
        for root in [self.root.join("active"), self.root.join("rollback")] {
            for entry in std::fs::read_dir(root)? {
                if let Ok(target) = std::fs::canonicalize(entry?.path()) {
                    protected.push(target);
                }
            }
        }
        let mut generations = generation_directories(&self.root.join("payloads"))?;
        generations.sort_by_key(|generation| generation.modified);
        let mut total = generations
            .iter()
            .map(|generation| generation.bytes)
            .sum::<u64>();
        let now = std::time::SystemTime::now();
        for generation in generations {
            if protected.iter().any(|path| path == &generation.path) {
                continue;
            }
            let expired = now
                .duration_since(generation.modified)
                .is_ok_and(|age| age > self.max_unused_age);
            if expired || total > self.max_cache_bytes {
                std::fs::remove_dir_all(&generation.path)?;
                total = total.saturating_sub(generation.bytes);
            }
        }
        Ok(())
    }
}

fn retain_host_artifact(generation: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    // A cached predictable partial-file link must not redirect this new write.
    let temporary = generation.join(format!(".artifact-{:032x}.partial", rand::random::<u128>()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = file
        .write_all(bytes)
        .and_then(|()| std::fs::rename(&temporary, generation.join("artifact")));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.context("retaining authenticated Machine host artifact")
}

async fn spawn_staged_probe(
    executable: &Path,
    generation: &Path,
    args: &[String],
) -> anyhow::Result<tokio::process::Child> {
    for attempt in 0_u64..3 {
        let result = tokio::process::Command::new(executable)
            .args(args)
            .current_dir(generation)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();
        match result {
            Ok(child) => return Ok(child),
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 2 => {
                // Some filesystems briefly retain the write-side executable
                // lease after atomic staging. Retry only this exact kernel
                // condition; every other spawn failure remains immediate.
                tokio::time::sleep(std::time::Duration::from_millis(10 * (attempt + 1))).await;
            }
            Err(error) => {
                return Err(error).context("starting staged component health probe");
            }
        }
    }
    unreachable!("the bounded component probe loop always returns")
}

fn component_command(desired: &DesiredComponent) -> Option<String> {
    use crate::machine_protocol::ComponentKind;
    match desired.id.kind {
        ComponentKind::MachineHost => Some("cowboy-machine".to_owned()),
        ComponentKind::AcpRuntime => Some("cowboy-acp-worker".to_owned()),
        ComponentKind::CodeAdapter => Some("cowboy-code-adapter".to_owned()),
        ComponentKind::ZedAdapter => Some("cowboy-zed-adapter".to_owned()),
        ComponentKind::ZedServer => Some("cowboy-zed-server".to_owned()),
        ComponentKind::ProviderCli => {
            (!desired.id.slot.is_empty()).then(|| desired.id.slot.clone())
        }
        ComponentKind::ProviderAdapter => {
            let slot = desired.id.slot.as_str();
            if slot.is_empty() {
                None
            } else if let Some(command) = crate::plugin_runtime_args::adapter_entrypoint(slot) {
                Some(command.to_owned())
            } else {
                Some(format!("cowboy-acp-{slot}"))
            }
        }
        ComponentKind::ManagedNode => Some("node".to_owned()),
    }
}

fn replace_symlink(link: &Path, target: &Path, temporary_name: &str) -> std::io::Result<()> {
    let temporary = link
        .parent()
        .expect("component link has parent")
        .join(temporary_name);
    let _ = std::fs::remove_file(&temporary);
    std::os::unix::fs::symlink(target, &temporary)?;
    std::fs::rename(temporary, link)
}

struct GenerationDirectory {
    path: PathBuf,
    bytes: u64,
    modified: std::time::SystemTime,
}

fn generation_directories(root: &Path) -> std::io::Result<Vec<GenerationDirectory>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for slot in std::fs::read_dir(root)? {
        for version in std::fs::read_dir(slot?.path())? {
            for digest in std::fs::read_dir(version?.path())? {
                let path = digest?.path();
                if path.is_dir() {
                    let metadata = std::fs::metadata(&path)?;
                    out.push(GenerationDirectory {
                        bytes: directory_bytes(&path)?,
                        modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                        path,
                    });
                }
            }
        }
    }
    Ok(out)
}

fn directory_bytes(path: &Path) -> std::io::Result<u64> {
    let mut bytes = 0_u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            bytes = bytes.saturating_add(directory_bytes(&entry.path())?);
        } else {
            bytes = bytes.saturating_add(metadata.len());
        }
    }
    Ok(bytes)
}

fn component_executable(generation: &Path, desired: &DesiredComponent) -> anyhow::Result<PathBuf> {
    match desired.artifact_format {
        ArtifactFormat::Raw => Ok(generation.join("bin")),
        ArtifactFormat::TarGz => {
            let entrypoint = desired
                .entrypoint
                .as_deref()
                .context("archive component requires entrypoint")?;
            let relative = Path::new(entrypoint);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                bail!("component entrypoint must be a safe relative path");
            }
            Ok(generation.join("content").join(relative))
        }
    }
}

fn extract_tar_gz(generation: &Path, bytes: &[u8], executable: &Path) -> anyhow::Result<()> {
    let temporary = generation.join(".content.partial");
    let _ = std::fs::remove_dir_all(&temporary);
    std::fs::create_dir_all(&temporary)?;
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(false);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            bail!("component archive links are not allowed");
        }
        if !entry.unpack_in(&temporary)? {
            bail!("component archive contains an unsafe path");
        }
    }
    let relative = executable.strip_prefix(generation.join("content"))?;
    let staged_executable = temporary.join(relative);
    if !staged_executable.is_file() {
        bail!("component archive entrypoint is missing");
    }
    set_executable(&staged_executable)?;
    std::fs::rename(&temporary, generation.join("content"))?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;
    use crate::machine_auth::MachineIdentity;
    use crate::machine_protocol::{ComponentId, ComponentKind};

    static TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn host_archive(executable: &[u8], companion: &[u8]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for (path, bytes) in [("bin/host", executable), ("lib/companion", companion)] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, bytes).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap()
    }

    fn portable_host(
        identity: &MachineIdentity,
        url: String,
        bytes: &[u8],
        version: &str,
        archive: bool,
        reader: bool,
    ) -> DesiredComponent {
        let mut desired = signed_component(identity, url, bytes, version);
        desired.id.kind = ComponentKind::MachineHost;
        desired.id.slot.clear();
        if archive {
            desired.artifact_format = ArtifactFormat::TarGz;
            desired.entrypoint = Some("bin/host".into());
        }
        if reader {
            desired.session_deletion_journal =
                Some(crate::machine_protocol::SessionDeletionReader {
                    reader_schema: 1,
                    writer_schema: 0,
                });
        }
        desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
        desired
    }

    #[tokio::test]
    async fn reader_floor_survives_updates_and_pruning_and_refuses_signed_downgrades() {
        use crate::session_deletion_admission::reader_floor::{self, NAME};
        for archive in [false, true] {
            let state = tempfile::tempdir_in("/tmp").unwrap();
            let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
            let key = state.path().join("publisher.pub");
            std::fs::write(&key, identity.public_key()).unwrap();
            let mut store = ComponentStore::new(
                state.path().join("components"),
                Some(&key),
                "fixture".into(),
            )
            .unwrap();
            let bytes = if archive {
                host_archive(b"#!/bin/sh\nexit 0\n", b"signed companion")
            } else {
                b"#!/bin/sh\nexit 0\n".to_vec()
            };
            let legacy = portable_host(
                &identity,
                serve_once(&bytes).await,
                &bytes,
                "legacy",
                archive,
                false,
            );
            store.reconcile(legacy.clone()).await.unwrap();
            assert!(!state.path().join(NAME).exists());
            let legacy_generation = store
                .root
                .join("active/machine_host")
                .canonicalize()
                .unwrap();
            let anchor = portable_host(
                &identity,
                serve_once(&bytes).await,
                &bytes,
                "anchor",
                archive,
                true,
            );
            store.reconcile(anchor.clone()).await.unwrap();
            let retained = std::fs::read(state.path().join(NAME)).unwrap();
            let floor = reader_floor::read(state.path()).unwrap().unwrap();
            assert_eq!(floor.anchor_generation, anchor.generation);
            check_portable_host_cache(state.path(), Some(&key)).unwrap();
            for version in ["second", "third"] {
                store
                    .reconcile(portable_host(
                        &identity,
                        serve_once(&bytes).await,
                        &bytes,
                        version,
                        archive,
                        true,
                    ))
                    .await
                    .unwrap();
                assert_eq!(std::fs::read(state.path().join(NAME)).unwrap(), retained);
                check_portable_host_cache(state.path(), Some(&key)).unwrap();
            }
            let active = std::fs::read_link(store.root.join("active/machine_host")).unwrap();
            let command = std::fs::read_link(store.command_path("cowboy-machine")).unwrap();
            let mut downgrade = legacy.clone();
            downgrade.artifact_url = "http://127.0.0.1:1/must-not-fetch".into();
            assert!(
                store
                    .reconcile(downgrade)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("portable reader floor")
            );
            let mut replacement = anchor.clone();
            replacement.automatic = false;
            replacement.signature = Some(identity.sign(&component_proof(&replacement)).unwrap());
            assert!(
                store
                    .reconcile(replacement)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("accepted anchor proof")
            );
            replace_symlink(
                &store.root.join("active/machine_host"),
                &legacy_generation,
                ".test-active",
            )
            .unwrap();
            replace_symlink(
                &store.command_path("cowboy-machine"),
                &component_executable(&legacy_generation, &legacy).unwrap(),
                ".test-command",
            )
            .unwrap();
            assert!(
                check_portable_host_cache(state.path(), Some(&key))
                    .unwrap_err()
                    .to_string()
                    .contains("undeclared")
            );
            replace_symlink(
                &store.root.join("active/machine_host"),
                &active,
                ".test-active",
            )
            .unwrap();
            replace_symlink(
                &store.command_path("cowboy-machine"),
                &command,
                ".test-command",
            )
            .unwrap();
            store.max_cache_bytes = 0;
            store.max_unused_age = std::time::Duration::ZERO;
            store.prune().unwrap();
            assert!(!legacy_generation.exists());
            assert!(floor.anchor_path(&store.root).exists());
            check_portable_host_cache(state.path(), Some(&key)).unwrap();
            for path in [
                store.root.join("active/machine_host"),
                store.command_path("cowboy-machine"),
            ] {
                std::fs::remove_file(path).unwrap();
            }
            assert!(
                check_portable_host_cache(state.path(), Some(&key))
                    .unwrap_err()
                    .to_string()
                    .contains("bootstrap fallback")
            );
            assert_eq!(std::fs::read(state.path().join(NAME)).unwrap(), retained);
            assert!(reader_floor::require_absent_for_install(state.path()).is_err());
        }
    }

    #[tokio::test]
    async fn floor_commit_precedes_pointer_publication_and_survives_publication_failure() {
        use crate::session_deletion_admission::reader_floor;
        let state = tempfile::tempdir_in("/tmp").unwrap();
        let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
        let key = state.path().join("publisher.pub");
        std::fs::write(&key, identity.public_key()).unwrap();
        let store = ComponentStore::new(
            state.path().join("components"),
            Some(&key),
            "fixture".into(),
        )
        .unwrap();
        std::fs::create_dir(store.root.join("active/machine_host")).unwrap();
        let bytes = b"#!/bin/sh\nexit 0\n";
        let candidate = portable_host(
            &identity,
            serve_once(bytes).await,
            bytes,
            "anchor",
            false,
            true,
        );
        assert!(store.reconcile(candidate).await.is_err());
        let floor = reader_floor::read(state.path()).unwrap().unwrap();
        assert!(floor.anchor_path(&store.root).exists());
        cached_host::authenticate_floor(&store.root, &floor, identity.public_key()).unwrap();
        assert!(!store.command_path("cowboy-machine").exists());
        assert!(check_portable_host_cache(state.path(), Some(&key)).is_err());
    }

    #[tokio::test]
    async fn anchor_corruption_or_probe_floor_removal_refuses_without_pointer_changes() {
        use crate::session_deletion_admission::reader_floor::{self, NAME};
        for remove in [false, true] {
            let state = tempfile::tempdir_in("/tmp").unwrap();
            let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
            let key = state.path().join("publisher.pub");
            std::fs::write(&key, identity.public_key()).unwrap();
            let store = ComponentStore::new(
                state.path().join("components"),
                Some(&key),
                "fixture".into(),
            )
            .unwrap();
            let bytes = b"#!/bin/sh\nexit 0\n";
            store
                .reconcile(portable_host(
                    &identity,
                    serve_once(bytes).await,
                    bytes,
                    "anchor",
                    false,
                    true,
                ))
                .await
                .unwrap();
            let floor = reader_floor::read(state.path()).unwrap().unwrap();
            let active = std::fs::read_link(store.root.join("active/machine_host")).unwrap();
            let command = std::fs::read_link(store.command_path("cowboy-machine")).unwrap();
            if remove {
                let script = format!("#!/bin/sh\nrm '{}'\n", state.path().join(NAME).display());
                assert!(
                    store
                        .reconcile(portable_host(
                            &identity,
                            serve_once(script.as_bytes()).await,
                            script.as_bytes(),
                            "candidate",
                            false,
                            true
                        ))
                        .await
                        .unwrap_err()
                        .to_string()
                        .contains("floor changed or disappeared")
                );
                assert!(
                    !state.path().join(NAME).exists(),
                    "signed probe effects were repaired"
                );
            } else {
                std::fs::write(
                    floor.anchor_path(&store.root).join("artifact"),
                    "corrupt proof",
                )
                .unwrap();
                assert!(check_portable_host_cache(state.path(), Some(&key)).is_err());
                let candidate = portable_host(
                    &identity,
                    "http://127.0.0.1:1/must-not-fetch".into(),
                    bytes,
                    "candidate",
                    false,
                    true,
                );
                assert!(store.reconcile(candidate).await.is_err());
                assert_eq!(
                    std::fs::read(floor.anchor_path(&store.root).join("artifact")).unwrap(),
                    b"corrupt proof"
                );
            }
            assert_eq!(
                std::fs::read_link(store.root.join("active/machine_host")).unwrap(),
                active
            );
            assert_eq!(
                std::fs::read_link(store.command_path("cowboy-machine")).unwrap(),
                command
            );
        }
    }

    #[tokio::test]
    async fn substituted_cached_hosts_refuse_before_probe_or_pointer_changes() {
        for case in [
            "raw-bytes",
            "raw-link",
            "archive-bytes",
            "archive-extra",
            "archive-link",
        ] {
            let state = tempfile::tempdir_in("/tmp").unwrap();
            let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
            let public_key = state.path().join("publisher.pub");
            std::fs::write(&public_key, identity.public_key()).unwrap();
            let store = ComponentStore::new(
                state.path().join("components"),
                Some(&public_key),
                "fixture".into(),
            )
            .unwrap();
            let marker = state.path().join("probe-ran");
            let script = format!("#!/bin/sh\ntouch '{}'\n", marker.display()).into_bytes();
            let archive = case.starts_with("archive");
            let bytes = if archive {
                host_archive(&script, b"signed companion")
            } else {
                script.clone()
            };
            let mut desired =
                signed_component(&identity, serve_once(&bytes).await, &bytes, "candidate");
            desired.id.kind = ComponentKind::MachineHost;
            desired.id.slot.clear();
            desired.session_deletion_journal =
                Some(crate::machine_protocol::SessionDeletionReader {
                    reader_schema: 1,
                    writer_schema: 0,
                });
            if archive {
                desired.artifact_format = ArtifactFormat::TarGz;
                desired.entrypoint = Some("bin/host".into());
            }
            desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
            let generation = store
                .root
                .join("payloads/machine_host/candidate")
                .join(&desired.digest);
            std::fs::create_dir_all(&generation).unwrap();
            let executable = component_executable(&generation, &desired).unwrap();
            if archive {
                extract_tar_gz(&generation, &bytes, &executable).unwrap();
            } else {
                std::fs::write(&executable, &bytes).unwrap();
                set_executable(&executable).unwrap();
            }
            match case {
                "raw-bytes" => std::fs::write(&executable, &script[..script.len() - 1]).unwrap(),
                "raw-link" => {
                    let target = state.path().join("outside-host");
                    std::fs::write(&target, &bytes).unwrap();
                    set_executable(&target).unwrap();
                    std::fs::remove_file(&executable).unwrap();
                    std::os::unix::fs::symlink(target, &executable).unwrap();
                }
                "archive-bytes" => std::fs::write(
                    generation.join("content/lib/companion"),
                    "changed companion",
                )
                .unwrap(),
                "archive-extra" => {
                    std::fs::write(generation.join("content/unsigned"), "extra").unwrap()
                }
                "archive-link" => {
                    let companion = generation.join("content/lib/companion");
                    std::fs::remove_file(&companion).unwrap();
                    std::os::unix::fs::symlink(&executable, companion).unwrap();
                }
                _ => unreachable!(),
            }
            std::fs::write(generation.join("manifest.json"), b"retained manifest").unwrap();
            for (directory, slot) in [
                ("active", "machine_host"),
                ("rollback", "machine_host"),
                ("commands", "cowboy-machine"),
            ] {
                std::os::unix::fs::symlink(
                    "/retained-target",
                    store.root.join(directory).join(slot),
                )
                .unwrap();
            }
            let error = store.reconcile(desired).await.unwrap_err();
            assert!(
                error.to_string().contains("staged Machine host"),
                "{case}: {error:#}"
            );
            assert!(!marker.exists(), "{case}: substituted probe executed");
            assert_eq!(
                std::fs::read(generation.join("manifest.json")).unwrap(),
                b"retained manifest"
            );
            for (directory, slot) in [
                ("active", "machine_host"),
                ("rollback", "machine_host"),
                ("commands", "cowboy-machine"),
            ] {
                assert_eq!(
                    std::fs::read_link(store.root.join(directory).join(slot)).unwrap(),
                    Path::new("/retained-target")
                );
            }
        }
    }

    #[tokio::test]
    async fn signed_host_probe_cannot_publish_modified_payloads() {
        for archive in [false, true] {
            let state = tempfile::tempdir_in("/tmp").unwrap();
            let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
            let public_key = state.path().join("publisher.pub");
            std::fs::write(&public_key, identity.public_key()).unwrap();
            let store = ComponentStore::new(
                state.path().join("components"),
                Some(&public_key),
                "fixture".into(),
            )
            .unwrap();
            let marker = state.path().join("probe-ran");
            let mutation = if archive {
                "printf 'changed' > content/lib/companion"
            } else {
                "printf '\\n# changed\\n' >> bin"
            };
            let script =
                format!("#!/bin/sh\ntouch '{}'\n{mutation}\n", marker.display()).into_bytes();
            let bytes = if archive {
                host_archive(&script, b"signed companion")
            } else {
                script
            };
            let mut desired =
                signed_component(&identity, serve_once(&bytes).await, &bytes, "candidate");
            desired.id.kind = ComponentKind::MachineHost;
            desired.id.slot.clear();
            if archive {
                desired.artifact_format = ArtifactFormat::TarGz;
                desired.entrypoint = Some("bin/host".into());
            }
            desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
            let error = store.reconcile(desired).await.unwrap_err();
            assert!(
                error.to_string().contains("staged Machine host"),
                "{error:#}"
            );
            assert!(marker.exists());
            for directory in ["active", "rollback", "commands"] {
                assert_eq!(
                    std::fs::read_dir(store.root.join(directory))
                        .unwrap()
                        .count(),
                    0
                );
            }
            // Probe effects remain; this is publication refusal, not rollback.
        }
    }

    #[tokio::test]
    async fn unchanged_signed_hosts_activate_on_first_stage_and_cache_reuse() {
        for archive in [false, true] {
            let state = tempfile::tempdir_in("/tmp").unwrap();
            let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
            let public_key = state.path().join("publisher.pub");
            std::fs::write(&public_key, identity.public_key()).unwrap();
            let store = ComponentStore::new(
                state.path().join("components"),
                Some(&public_key),
                "fixture".into(),
            )
            .unwrap();
            let script = b"#!/bin/sh\nexit 0\n";
            let bytes = if archive {
                host_archive(script, b"signed companion")
            } else {
                script.to_vec()
            };
            for _ in 0..2 {
                let mut desired =
                    signed_component(&identity, serve_once(&bytes).await, &bytes, "healthy");
                desired.id.kind = ComponentKind::MachineHost;
                desired.id.slot.clear();
                if archive {
                    desired.artifact_format = ArtifactFormat::TarGz;
                    desired.entrypoint = Some("bin/host".into());
                }
                desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
                store.reconcile(desired).await.unwrap();
                assert_eq!(
                    std::fs::read(store.command_path("cowboy-machine")).unwrap(),
                    script
                );
            }
        }
    }

    #[tokio::test]
    async fn cached_host_authentication_is_offline_read_only_and_refuses_substitution() {
        for archive in [false, true] {
            for case in [
                "bytes",
                "manifest",
                "artifact",
                "legacy",
                "proof-link",
                "command",
                "dangling",
                "missing-active",
                "outside",
                "non-executable",
                "payload-link",
            ] {
                let state = tempfile::tempdir_in("/tmp").unwrap();
                let identity =
                    MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
                let key = state.path().join("publisher.pub");
                std::fs::write(&key, identity.public_key()).unwrap();
                let store = ComponentStore::new(
                    state.path().join("components"),
                    Some(&key),
                    "fixture".into(),
                )
                .unwrap();
                let marker = state.path().join("executed");
                let script = format!("#!/bin/sh\ntouch '{}'\n", marker.display());
                let bytes = if archive {
                    host_archive(script.as_bytes(), b"signed companion")
                } else {
                    script.as_bytes().to_vec()
                };
                let mut desired =
                    signed_component(&identity, serve_once(&bytes).await, &bytes, "healthy");
                desired.id.kind = ComponentKind::MachineHost;
                desired.id.slot.clear();
                if archive {
                    desired.artifact_format = ArtifactFormat::TarGz;
                    desired.entrypoint = Some("bin/host".into());
                }
                desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
                store.reconcile(desired.clone()).await.unwrap();
                std::fs::remove_file(&marker).unwrap();
                let generation =
                    std::fs::canonicalize(store.root.join("active/machine_host")).unwrap();
                let executable = component_executable(&generation, &desired).unwrap();
                check_portable_host_cache(state.path(), Some(&key)).unwrap();
                assert!(!marker.exists(), "authentication executed the cached probe");
                assert!(check_portable_host_cache(state.path(), None).is_err());
                let other_key =
                    MachineIdentity::load_or_create(&state.path().join("other-signer")).unwrap();
                let wrong = state.path().join("wrong.pub");
                std::fs::write(&wrong, other_key.public_key()).unwrap();
                assert!(check_portable_host_cache(state.path(), Some(&wrong)).is_err());
                match case {
                    "bytes" => {
                        let path = if archive {
                            generation.join("content/lib/companion")
                        } else {
                            executable.clone()
                        };
                        std::fs::write(path, "substituted").unwrap();
                    }
                    "manifest" => {
                        desired.generation = "unsigned-generation".into();
                        std::fs::write(
                            generation.join("manifest.json"),
                            serde_json::to_vec(&desired).unwrap(),
                        )
                        .unwrap();
                    }
                    "artifact" => {
                        std::fs::write(generation.join("artifact"), "substituted artifact").unwrap()
                    }
                    "legacy" => std::fs::remove_file(generation.join("artifact")).unwrap(),
                    "proof-link" => {
                        let artifact = generation.join("artifact");
                        let outside = state.path().join("outside-artifact");
                        std::fs::rename(&artifact, &outside).unwrap();
                        std::os::unix::fs::symlink(outside, artifact).unwrap();
                    }
                    "command" | "dangling" => {
                        let target = state.path().join("substitute");
                        if case == "command" {
                            std::fs::write(&target, script.as_bytes()).unwrap();
                            set_executable(&target).unwrap();
                        }
                        std::fs::remove_file(store.command_path("cowboy-machine")).unwrap();
                        std::os::unix::fs::symlink(target, store.command_path("cowboy-machine"))
                            .unwrap();
                    }
                    "missing-active" => {
                        std::fs::remove_file(store.root.join("active/machine_host")).unwrap()
                    }
                    "outside" => {
                        let outside = state.path().join("outside-generation");
                        std::fs::rename(&generation, &outside).unwrap();
                        std::fs::remove_file(store.root.join("active/machine_host")).unwrap();
                        std::os::unix::fs::symlink(outside, store.root.join("active/machine_host"))
                            .unwrap();
                    }
                    "non-executable" => {
                        use std::os::unix::fs::PermissionsExt as _;
                        std::fs::set_permissions(
                            &executable,
                            std::fs::Permissions::from_mode(0o644),
                        )
                        .unwrap();
                    }
                    "payload-link" => {
                        let outside = state.path().join("outside-executable");
                        std::fs::rename(&executable, &outside).unwrap();
                        std::os::unix::fs::symlink(outside, &executable).unwrap();
                    }
                    _ => unreachable!(),
                }
                let active = std::fs::read_link(store.root.join("active/machine_host")).ok();
                let command = std::fs::read_link(store.command_path("cowboy-machine")).unwrap();
                assert!(
                    check_portable_host_cache(state.path(), Some(&key)).is_err(),
                    "{archive}/{case}"
                );
                assert_eq!(
                    std::fs::read_link(store.root.join("active/machine_host")).ok(),
                    active
                );
                assert_eq!(
                    std::fs::read_link(store.command_path("cowboy-machine")).unwrap(),
                    command
                );
                assert!(
                    !marker.exists(),
                    "{archive}/{case}: cached code ran during refusal"
                );
            }
        }
    }

    #[test]
    fn retaining_host_proof_does_not_follow_cached_destination_or_partial_links() {
        let state = tempfile::tempdir().unwrap();
        let generation = state.path().join("generation");
        std::fs::create_dir(&generation).unwrap();
        let outside = state.path().join("outside");
        std::fs::write(&outside, b"retained outside bytes").unwrap();
        for name in ["artifact", ".artifact.partial"] {
            std::os::unix::fs::symlink(&outside, generation.join(name)).unwrap();
        }
        retain_host_artifact(&generation, b"authenticated proof").unwrap();
        assert_eq!(std::fs::read(&outside).unwrap(), b"retained outside bytes");
        assert!(
            std::fs::symlink_metadata(generation.join("artifact"))
                .unwrap()
                .is_file()
        );
        assert_eq!(
            std::fs::read(generation.join("artifact")).unwrap(),
            b"authenticated proof"
        );
        assert_eq!(
            std::fs::read_link(generation.join(".artifact.partial")).unwrap(),
            outside
        );
        assert_eq!(std::fs::read_dir(&generation).unwrap().count(), 2);
    }

    #[test]
    fn absent_cached_host_selection_needs_no_key_or_state_creation() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("absent");
        check_portable_host_cache(&state, None).unwrap();
        assert!(!state.exists());
        std::fs::create_dir_all(state.join("components/active")).unwrap();
        check_portable_host_cache(&state, None).unwrap();
        assert!(!state.join("components/commands").exists());
        std::fs::remove_dir(state.join("components/active")).unwrap();
        std::os::unix::fs::symlink(root.path(), state.join("components/active")).unwrap();
        assert!(check_portable_host_cache(&state, None).is_err());
    }

    #[tokio::test]
    #[ignore = "requires an independently built immutable Machine host release"]
    async fn immutable_portable_launcher_authenticates_cached_hosts_before_exec() {
        let release = PathBuf::from(
            std::env::var("COWBOY_TEST_PORTABLE_HOST_RELEASE").expect("immutable release required"),
        );
        let native = release.join("bin/cowboy-machine");
        assert!(native.is_file());
        for archive in [false, true] {
            for case in [
                "healthy",
                "bytes",
                "manifest",
                "artifact",
                "legacy",
                "pointer",
                "committed",
                "floor-healthy",
                "floor-absent",
                "floor-legacy",
                "floor-corrupt",
                "floor-anchor",
            ] {
                let state = tempfile::tempdir_in("/tmp").unwrap();
                let identity =
                    MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
                let key = state.path().join("publisher.pub");
                std::fs::write(&key, identity.public_key()).unwrap();
                let store = ComponentStore::new(
                    state.path().join("components"),
                    Some(&key),
                    "fixture".into(),
                )
                .unwrap();
                let selected = state.path().join("selected");
                let fallback = state.path().join("fallback");
                let script = format!(
                    "#!/bin/sh\n[ \"${{1-}}\" = --probe ] && exit 0\ntouch '{}'\n",
                    selected.display()
                );
                let bytes = if archive {
                    host_archive(script.as_bytes(), b"signed companion")
                } else {
                    script.as_bytes().to_vec()
                };
                let mut desired =
                    signed_component(&identity, serve_once(&bytes).await, &bytes, "healthy");
                desired.id.kind = ComponentKind::MachineHost;
                desired.id.slot.clear();
                desired.probe.as_mut().unwrap().args = vec!["--probe".into()];
                if archive {
                    desired.artifact_format = ArtifactFormat::TarGz;
                    desired.entrypoint = Some("bin/host".into());
                }
                desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
                store.reconcile(desired.clone()).await.unwrap();
                let generation = store
                    .root
                    .join("active/machine_host")
                    .canonicalize()
                    .unwrap();
                let executable = component_executable(&generation, &desired).unwrap();
                if case.starts_with("floor-") {
                    let mut reader = desired.clone();
                    reader.version = "reader-anchor".into();
                    reader.generation = "reader-anchor".into();
                    reader.artifact_url = serve_once(&bytes).await;
                    reader.session_deletion_journal =
                        Some(crate::machine_protocol::SessionDeletionReader {
                            reader_schema: 1,
                            writer_schema: 0,
                        });
                    reader.signature = Some(identity.sign(&component_proof(&reader)).unwrap());
                    store.reconcile(reader.clone()).await.unwrap();
                    match case {
                        "floor-healthy" => {}
                        "floor-absent" => {
                            std::fs::remove_file(store.root.join("active/machine_host")).unwrap();
                            std::fs::remove_file(store.command_path("cowboy-machine")).unwrap();
                            // The preceding independent guard ignores this floor;
                            // installing that bootstrap is refused by the new installer.
                            let old = PathBuf::from(
                                std::env::var("COWBOY_TEST_PORTABLE_OLD_HOST_RELEASE")
                                    .expect("old guard required"),
                            );
                            let old_output =
                                tokio::process::Command::new(old.join("bin/cowboy-machine"))
                                    .args(["--check-portable-session-deletion", "--state-dir"])
                                    .arg(state.path())
                                    .output()
                                    .await
                                    .unwrap();
                            assert!(
                                old_output.status.success(),
                                "old guard negative control did not reach bootstrap selection"
                            );
                        }
                        "floor-legacy" => {
                            replace_symlink(
                                &store.root.join("active/machine_host"),
                                &generation,
                                ".test-active",
                            )
                            .unwrap();
                            replace_symlink(
                                &store.command_path("cowboy-machine"),
                                &executable,
                                ".test-command",
                            )
                            .unwrap();
                        }
                        "floor-corrupt" => std::fs::write(
                            state
                                .path()
                                .join(crate::session_deletion_admission::reader_floor::NAME),
                            b"{}",
                        )
                        .unwrap(),
                        "floor-anchor" => {
                            let floor =
                                crate::session_deletion_admission::reader_floor::read(state.path())
                                    .unwrap()
                                    .unwrap();
                            reader.version = "reader-second".into();
                            reader.generation = "reader-second".into();
                            reader.artifact_url = serve_once(&bytes).await;
                            reader.signature =
                                Some(identity.sign(&component_proof(&reader)).unwrap());
                            store.reconcile(reader).await.unwrap();
                            std::fs::write(
                                floor.anchor_path(&store.root).join("artifact"),
                                b"substituted proof",
                            )
                            .unwrap();
                        }
                        _ => unreachable!(),
                    }
                }
                std::fs::create_dir(state.path().join("bootstrap")).unwrap();
                let bootstrap = state.path().join("bootstrap/cowboy-machine");
                // The real immutable bootstrap runs the diagnostic. The shim's
                // ordinary-start marker detects any unintended fallback.
                std::fs::write(&bootstrap, format!("#!/bin/sh\nif [ \"${{1-}}\" = --check-portable-session-deletion ]; then exec '{}' \"$@\"; fi\ntouch '{}'\n", native.display(), fallback.display())).unwrap();
                set_executable(&bootstrap).unwrap();
                match case {
                    "healthy" | "floor-healthy" | "floor-absent" | "floor-legacy"
                    | "floor-corrupt" | "floor-anchor" => {}
                    "bytes" => {
                        let target = if archive {
                            generation.join("content/lib/companion")
                        } else {
                            executable.clone()
                        };
                        std::fs::write(target, b"substitute").unwrap();
                    }
                    "manifest" => {
                        desired.generation = "unsigned".into();
                        std::fs::write(
                            generation.join("manifest.json"),
                            serde_json::to_vec(&desired).unwrap(),
                        )
                        .unwrap();
                    }
                    "artifact" => {
                        std::fs::write(generation.join("artifact"), b"substitute").unwrap()
                    }
                    "legacy" => std::fs::remove_file(generation.join("artifact")).unwrap(),
                    "pointer" => {
                        std::fs::remove_file(store.command_path("cowboy-machine")).unwrap();
                        std::os::unix::fs::symlink(
                            &bootstrap,
                            store.command_path("cowboy-machine"),
                        )
                        .unwrap();
                    }
                    "committed" => {
                        std::fs::create_dir(state.path().join("session-deletions")).unwrap();
                        std::fs::write(
                            state.path().join("session-deletions/deletions.json"),
                            b"{}",
                        )
                        .unwrap();
                    }
                    _ => unreachable!(),
                }
                let launcher = state.path().join("launcher");
                std::fs::write(
                    &launcher,
                    crate::machine_install::portable_cache_launcher_fixture(state.path(), &key),
                )
                .unwrap();
                let output = tokio::process::Command::new("/bin/sh")
                    .arg(&launcher)
                    .output()
                    .await
                    .unwrap();
                assert_eq!(
                    output.status.success(),
                    matches!(case, "healthy" | "floor-healthy"),
                    "{archive}/{case}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    selected.exists(),
                    matches!(case, "healthy" | "floor-healthy"),
                    "{archive}/{case}: selected code executed"
                );
                assert!(!fallback.exists(), "{archive}/{case}: fallback executed");
                assert_eq!(
                    state.path().join("run").exists(),
                    matches!(case, "healthy" | "floor-healthy"),
                    "refusal created runtime directory"
                );
            }
        }
    }

    #[tokio::test]
    async fn signed_probe_cannot_publish_modified_manifest_or_retained_proof() {
        for target in ["manifest.json", "artifact"] {
            let state = tempfile::tempdir_in("/tmp").unwrap();
            let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
            let key = state.path().join("publisher.pub");
            std::fs::write(&key, identity.public_key()).unwrap();
            let store = ComponentStore::new(
                state.path().join("components"),
                Some(&key),
                "fixture".into(),
            )
            .unwrap();
            let script = format!("#!/bin/sh\nprintf 'substituted' > {target}\n");
            let mut desired = signed_component(
                &identity,
                serve_once(script.as_bytes()).await,
                script.as_bytes(),
                "candidate",
            );
            desired.id.kind = ComponentKind::MachineHost;
            desired.id.slot.clear();
            desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
            assert!(store.reconcile(desired).await.is_err());
            for directory in ["active", "rollback", "commands"] {
                assert_eq!(
                    std::fs::read_dir(store.root.join(directory))
                        .unwrap()
                        .count(),
                    0
                );
            }
        }
    }

    #[test]
    fn reader_claim_has_a_distinct_signature_domain_and_preserves_legacy_bytes() {
        let root = tempfile::tempdir().unwrap();
        let identity = MachineIdentity::load_or_create(root.path()).unwrap();
        let mut desired = signed_component(
            &identity,
            "https://example.invalid/host".into(),
            b"host",
            "v1",
        );
        desired.id.kind = ComponentKind::MachineHost;
        desired.id.slot.clear();
        desired.generation = "g1".into();
        desired.digest = "a".repeat(64);
        desired.probe = None;
        desired.automatic = false;
        let legacy = format!(
            "cowboy-component-v3\n12:machine_host\n2:v1\n2:g1\n64:{}\n3:raw\n0:\n0:\n5:false\n",
            "a".repeat(64)
        );
        assert_eq!(component_proof(&desired), legacy.as_bytes());
        let old_signature = identity.sign(&component_proof(&desired)).unwrap();
        desired.session_deletion_journal = Some(crate::machine_protocol::SessionDeletionReader {
            reader_schema: 1,
            writer_schema: 0,
        });
        let expected = legacy.replacen("cowboy-component-v3", "cowboy-component-v4", 1)
            + "37:{\"reader_schema\":1,\"writer_schema\":0}\n";
        assert_eq!(component_proof(&desired), expected.as_bytes());
        assert!(
            !crate::machine_auth::verify(
                identity.public_key(),
                &component_proof(&desired),
                &old_signature
            )
            .unwrap()
        );
        let signature = identity.sign(&component_proof(&desired)).unwrap();
        assert!(
            crate::machine_auth::verify(
                identity.public_key(),
                &component_proof(&desired),
                &signature
            )
            .unwrap()
        );
        for mutation in ["strip", "reader", "writer"] {
            let mut changed = desired.clone();
            match mutation {
                "strip" => changed.session_deletion_journal = None,
                "reader" => {
                    changed
                        .session_deletion_journal
                        .as_mut()
                        .unwrap()
                        .reader_schema = 2
                }
                "writer" => {
                    changed
                        .session_deletion_journal
                        .as_mut()
                        .unwrap()
                        .writer_schema = 1
                }
                _ => unreachable!(),
            }
            assert!(
                !crate::machine_auth::verify(
                    identity.public_key(),
                    &component_proof(&changed),
                    &signature
                )
                .unwrap(),
                "{mutation}"
            );
        }
    }

    #[tokio::test]
    async fn invalid_reader_claim_refuses_before_fetch_or_staging() {
        let state = tempfile::tempdir().unwrap();
        let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
        let store =
            ComponentStore::new(state.path().join("components"), None, "fixture".into()).unwrap();
        for (kind, reader_schema, writer_schema) in [
            (ComponentKind::ProviderCli, 1, 0),
            (ComponentKind::MachineHost, 2, 0),
            (ComponentKind::MachineHost, 1, 1),
        ] {
            let mut desired = signed_component(
                &identity,
                "http://127.0.0.1:1/never-fetched".into(),
                b"host",
                "v1",
            );
            desired.id.kind = kind;
            desired.id.slot.clear();
            desired.session_deletion_journal =
                Some(crate::machine_protocol::SessionDeletionReader {
                    reader_schema,
                    writer_schema,
                });
            assert!(
                store
                    .reconcile(desired)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("Session deletion declaration")
            );
        }
        for name in ["active", "rollback", "commands", "payloads"] {
            assert_eq!(std::fs::read_dir(store.root.join(name)).unwrap().count(), 0);
        }
    }

    #[tokio::test]
    async fn signed_reader_claim_does_not_admit_committed_portable_state() {
        let state = tempfile::tempdir().unwrap();
        let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
        let store =
            ComponentStore::new(state.path().join("components"), None, "fixture".into()).unwrap();
        let journal = state.path().join("session-deletions/deletions.json");
        std::fs::create_dir(journal.parent().unwrap()).unwrap();
        std::fs::write(&journal, "retained evidence").unwrap();
        let mut desired = signed_component(
            &identity,
            "http://127.0.0.1:1/never-fetched".into(),
            b"host",
            "v1",
        );
        desired.id.kind = ComponentKind::MachineHost;
        desired.id.slot.clear();
        desired.session_deletion_journal = Some(crate::machine_protocol::SessionDeletionReader {
            reader_schema: 1,
            writer_schema: 0,
        });
        desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
        assert!(
            store
                .reconcile(desired)
                .await
                .unwrap_err()
                .to_string()
                .contains("portable Session deletion reader admission")
        );
        assert_eq!(std::fs::read(&journal).unwrap(), b"retained evidence");
        for name in ["active", "rollback", "commands", "payloads"] {
            assert_eq!(std::fs::read_dir(store.root.join(name)).unwrap().count(), 0);
        }
    }

    #[tokio::test]
    async fn terminal_journal_refuses_host_fetch_without_mutating_component_links() {
        let state = tempfile::tempdir().unwrap();
        let store =
            ComponentStore::new(state.path().join("components"), None, "fixture".into()).unwrap();
        std::fs::create_dir(state.path().join("session-deletions")).unwrap();
        std::fs::write(state.path().join("session-deletions/deletions.json"), "{}").unwrap();
        let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
        let mut desired = signed_component(
            &identity,
            "http://127.0.0.1:1/never-fetched".into(),
            b"host",
            "v1",
        );
        desired.id.kind = ComponentKind::MachineHost;
        desired.id.slot.clear();
        let error = store.reconcile(desired).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("portable Session deletion reader admission")
        );
        for name in ["active", "rollback", "commands", "payloads"] {
            assert_eq!(std::fs::read_dir(store.root.join(name)).unwrap().count(), 0);
        }
    }

    #[tokio::test]
    async fn host_probe_creating_a_terminal_journal_cannot_publish_its_candidate() {
        let state = tempfile::tempdir_in("/tmp").unwrap();
        let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
        let public_key = state.path().join("publisher.pub");
        std::fs::write(&public_key, identity.public_key()).unwrap();
        let store = ComponentStore::new(
            state.path().join("components"),
            Some(&public_key),
            "fixture".into(),
        )
        .unwrap();
        let journal = state.path().join("session-deletions");
        std::fs::create_dir(&journal).unwrap();
        let bytes = format!(
            "#!/bin/sh\nprintf '{{}}' > '{}/deletions.json'\n",
            journal.display()
        )
        .into_bytes();
        let mut desired = signed_component(&identity, serve_once(&bytes).await, &bytes, "v1");
        desired.id.kind = ComponentKind::MachineHost;
        desired.id.slot.clear();
        desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
        let error = store.reconcile(desired).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("portable Session deletion reader admission")
        );
        assert_eq!(
            std::fs::read(journal.join("deletions.json")).unwrap(),
            b"{}"
        );
        for name in ["active", "rollback", "commands"] {
            assert_eq!(std::fs::read_dir(store.root.join(name)).unwrap().count(), 0);
        }
        // Verified staging may remain; refusal is not rollback of probe effects.
        assert_eq!(
            std::fs::read_dir(store.root.join("payloads"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn provider_cli_and_adapter_commands_never_collide() {
        let component = |kind, slot: &str| DesiredComponent {
            id: ComponentId {
                kind,
                slot: slot.to_owned(),
            },
            version: "v1".to_owned(),
            generation: "v1".to_owned(),
            artifact_url: "https://example.invalid/component".to_owned(),
            digest: "digest".to_owned(),
            artifact_format: ArtifactFormat::Raw,
            entrypoint: None,
            signature: None,
            session_deletion_journal: None,
            probe: None,
            automatic: true,
        };
        assert_eq!(
            component_command(&component(ComponentKind::ProviderCli, "codex")).as_deref(),
            Some("codex")
        );
        assert_eq!(
            component_command(&component(ComponentKind::ProviderAdapter, "codex")).as_deref(),
            Some("codex-acp")
        );
        assert_eq!(
            component_command(&component(ComponentKind::ProviderAdapter, "claude")).as_deref(),
            Some("claude-agent-acp")
        );
        assert_eq!(
            component_command(&component(ComponentKind::ProviderAdapter, "claude-code")).as_deref(),
            Some("claude-agent-acp")
        );
        assert_eq!(
            component_command(&component(ComponentKind::ProviderAdapter, "gemini")).as_deref(),
            Some("gemini")
        );
        assert_eq!(
            component_command(&component(ComponentKind::ProviderAdapter, "grok")).as_deref(),
            Some("grok")
        );
        assert_eq!(
            component_command(&component(ComponentKind::ProviderAdapter, "future")).as_deref(),
            Some("cowboy-acp-future")
        );
    }

    #[tokio::test]
    async fn signed_payload_activates_by_content_and_remembers_rollback() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-component-test-{}-{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let identity_dir = root.join("signer");
        let identity = MachineIdentity::load_or_create(&identity_dir).expect("signer");
        let public_key = root.join("publisher.pub");
        std::fs::write(&public_key, identity.public_key()).expect("public key");
        let store = ComponentStore::new(
            root.join("store"),
            Some(&public_key),
            "worker-bootstrap".to_owned(),
        )
        .expect("store");

        // Terminal state restricts Machine host selection, not unrelated payloads.
        std::fs::create_dir(root.join("session-deletions")).unwrap();
        std::fs::write(root.join("session-deletions/deletions.json"), "{}").unwrap();

        const FIRST: &[u8] = b"#!/bin/sh\nexit 0\n# first\n";
        const SECOND: &[u8] = b"#!/bin/sh\nexit 0\n# second\n";
        let first = signed_component(&identity, serve_once(FIRST).await, FIRST, "v1");
        let activated = store.reconcile(first).await.expect("activate first");
        assert_eq!(activated.rollback_generation, None);
        let second = signed_component(&identity, serve_once(SECOND).await, SECOND, "v2");
        let activated = store.reconcile(second).await.expect("activate second");
        assert!(activated.rollback_generation.is_some());
        let active =
            std::fs::read_link(root.join("store/active/provider_cli-codex")).expect("active link");
        assert_eq!(
            std::fs::read(active.join("bin")).expect("active bytes"),
            SECOND
        );

        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn failed_health_probe_never_replaces_the_active_generation() {
        const HEALTHY: &[u8] = b"#!/bin/sh\nexit 0\n";
        const BROKEN: &[u8] = b"#!/bin/sh\nexit 23\n";
        let root = std::env::temp_dir().join(format!(
            "cowboy-component-probe-test-{}-{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let identity = MachineIdentity::load_or_create(&root.join("signer")).unwrap();
        let public_key = root.join("publisher.pub");
        std::fs::write(&public_key, identity.public_key()).unwrap();
        let store = ComponentStore::new(
            root.join("store"),
            Some(&public_key),
            "worker-bootstrap".to_owned(),
        )
        .unwrap();

        store
            .reconcile(signed_component(
                &identity,
                serve_once(HEALTHY).await,
                HEALTHY,
                "healthy",
            ))
            .await
            .unwrap();
        let error = store
            .reconcile(signed_component(
                &identity,
                serve_once(BROKEN).await,
                BROKEN,
                "broken",
            ))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("health probe exited"));
        let active = store.command_path("codex").canonicalize().unwrap();
        assert!(active.to_string_lossy().contains("healthy"));
        assert_eq!(std::fs::read(active).unwrap(), HEALTHY);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn signed_archive_activates_only_its_safe_entrypoint() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-component-archive-test-{}-{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let identity = MachineIdentity::load_or_create(&root.join("signer")).unwrap();
        let public_key = root.join("publisher.pub");
        std::fs::write(&public_key, identity.public_key()).unwrap();
        let store = ComponentStore::new(
            root.join("store"),
            Some(&public_key),
            "worker-bootstrap".to_owned(),
        )
        .unwrap();
        let mut archive_bytes = Vec::new();
        {
            let encoder =
                flate2::write::GzEncoder::new(&mut archive_bytes, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let bytes = b"#!/bin/sh\nexit 0\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, "bin/provider", &bytes[..])
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }
        let mut desired = signed_component(
            &identity,
            serve_once(Box::leak(archive_bytes.clone().into_boxed_slice())).await,
            &archive_bytes,
            "archive-v1",
        );
        desired.artifact_format = ArtifactFormat::TarGz;
        desired.entrypoint = Some("bin/provider".to_owned());
        desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
        store.reconcile(desired).await.unwrap();
        let command = store.command_path("codex").canonicalize().unwrap();
        assert!(command.ends_with("content/bin/provider"));
        assert!(command.is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn signed_component(
        identity: &MachineIdentity,
        artifact_url: String,
        bytes: &[u8],
        version: &str,
    ) -> DesiredComponent {
        let mut desired = DesiredComponent {
            id: ComponentId {
                kind: ComponentKind::ProviderCli,
                slot: "codex".to_owned(),
            },
            version: version.to_owned(),
            generation: version.to_owned(),
            artifact_url,
            digest: format!("{:x}", Sha256::digest(bytes)),
            artifact_format: ArtifactFormat::Raw,
            entrypoint: None,
            signature: None,
            session_deletion_journal: None,
            probe: Some(crate::machine_protocol::ComponentProbe {
                args: Vec::new(),
                timeout_ms: 2_000,
            }),
            automatic: true,
        };
        desired.signature = Some(
            identity
                .sign(&component_proof(&desired))
                .expect("signature"),
        );
        desired
    }

    async fn serve_once(body: &[u8]) -> String {
        let body = body.to_vec();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.expect("header");
            stream.write_all(&body).await.expect("body");
        });
        format!("http://{address}/artifact")
    }
}
