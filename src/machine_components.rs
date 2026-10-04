//! Content-addressed Machine payload activation.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use sha2::{Digest as _, Sha256};

use crate::machine_protocol::{
    ArtifactFormat, ComponentInventory, ComponentState, DesiredComponent,
};

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

fn component_slot(desired: &DesiredComponent) -> String {
    let kind = serde_json::to_value(&desired.id.kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "component".to_owned());
    if desired.id.slot.is_empty() {
        kind
    } else {
        format!("{kind}-{}", desired.id.slot.replace('/', "_"))
    }
}

fn component_proof(desired: &DesiredComponent) -> Vec<u8> {
    let format = match desired.artifact_format {
        ArtifactFormat::Raw => "raw",
        ArtifactFormat::TarGz => "tar_gz",
    };
    let probe = desired
        .probe
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .expect("component probe serializes")
        .unwrap_or_default();
    let mut fields = vec![
        component_slot(desired),
        desired.version.clone(),
        desired.generation.clone(),
        desired.digest.clone(),
        format.to_owned(),
        desired.entrypoint.clone().unwrap_or_default(),
        probe,
        desired.automatic.to_string(),
    ];
    let mut proof = if let Some(reader) = &desired.session_deletion_journal {
        fields.push(serde_json::to_string(reader).expect("reader declaration serializes"));
        b"cowboy-component-v4\n".to_vec()
    } else {
        // Preserve every byte of existing signatures when no claim is present.
        b"cowboy-component-v3\n".to_vec()
    };
    for field in fields {
        proof.extend_from_slice(field.len().to_string().as_bytes());
        proof.push(b':');
        proof.extend_from_slice(field.as_bytes());
        proof.push(b'\n');
    }
    proof
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
