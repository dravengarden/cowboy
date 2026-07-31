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
    max_cache_bytes: u64,
    max_unused_age: std::time::Duration,
}

impl ComponentStore {
    pub fn new(root: PathBuf, publisher_key_path: Option<&Path>) -> anyhow::Result<Self> {
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
            max_cache_bytes: 10 * 1024 * 1024 * 1024,
            max_unused_age: std::time::Duration::from_secs(30 * 24 * 60 * 60),
        })
    }

    pub async fn reconcile(&self, desired: DesiredComponent) -> anyhow::Result<ComponentInventory> {
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
        })
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
        ComponentKind::ProviderAdapter => match desired.id.slot.as_str() {
            "codex" => Some("codex-acp".to_owned()),
            "claude" | "claude-code" => Some("claude-agent-acp".to_owned()),
            "gemini" => Some("gemini-acp".to_owned()),
            "" => None,
            slot => Some(format!("cowboy-acp-{slot}")),
        },
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
    format!(
        "cowboy-component-v2\n{}\n{}\n{}\n{}\n{}\n{}\n",
        component_slot(desired),
        desired.version,
        desired.generation,
        desired.digest,
        format,
        desired.entrypoint.as_deref().unwrap_or("")
    )
    .into_bytes()
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

    use sha2::{Digest as _, Sha256};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;
    use crate::machine_auth::MachineIdentity;
    use crate::machine_protocol::{ComponentId, ComponentKind};

    static TEST_ID: AtomicU64 = AtomicU64::new(1);

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
        let store = ComponentStore::new(root.join("store"), Some(&public_key)).expect("store");

        let first = signed_component(&identity, serve_once(b"first").await, b"first", "v1");
        let activated = store.reconcile(first).await.expect("activate first");
        assert_eq!(activated.rollback_generation, None);
        let second = signed_component(&identity, serve_once(b"second").await, b"second", "v2");
        let activated = store.reconcile(second).await.expect("activate second");
        assert!(activated.rollback_generation.is_some());
        let active =
            std::fs::read_link(root.join("store/active/provider_cli-codex")).expect("active link");
        assert_eq!(
            std::fs::read(active.join("bin")).expect("active bytes"),
            b"second"
        );

        std::fs::remove_dir_all(root).expect("cleanup");
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
        let store = ComponentStore::new(root.join("store"), Some(&public_key)).unwrap();
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
            automatic: true,
        };
        desired.signature = Some(
            identity
                .sign(&component_proof(&desired))
                .expect("signature"),
        );
        desired
    }

    async fn serve_once(body: &'static [u8]) -> String {
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
            stream.write_all(body).await.expect("body");
        });
        format!("http://{address}/artifact")
    }
}
