//! Untrusted, content-addressed download input. Cache import is not installation
//! authority: the installer validates the signed release before consulting it.

use super::*;
use std::io::Read as _;

fn cache_path(state: &Path, digest: &str) -> Result<PathBuf> {
    let hash = digest
        .strip_prefix("sha256:")
        .context("expected SHA-256 digest")?;
    ensure!(
        hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "expected SHA-256 digest"
    );
    Ok(state.join("artifact-cache").join(hash.to_ascii_lowercase()))
}

fn read_verified(path: &Path, digest: &str) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES as u64,
        "artifact cache input must be a bounded regular file"
    );
    let mut bytes = Vec::new();
    file.take(MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES,
        "artifact cache input exceeds limit"
    );
    ensure!(
        format!("sha256:{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(digest),
        "artifact cache digest mismatch"
    );
    Ok(bytes)
}

fn validate_directory(directory: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = directory.symlink_metadata()?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode().trailing_zeros() >= 6,
        "artifact cache directory must be private and owned by the Machine user"
    );
    Ok(())
}

pub(super) fn read(state: &Path, digest: &str) -> Result<Option<Vec<u8>>> {
    let path = cache_path(state, digest)?;
    let directory = path.parent().context("artifact cache directory")?;
    if !directory.try_exists()? {
        return Ok(None);
    }
    validate_directory(directory)?;
    match path.symlink_metadata() {
        Ok(_) => read_verified(&path, digest).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// A Plugin retains only a few generations; bound the scan regardless.
const MAX_RETAINED_GENERATIONS: usize = 64;

/// Runtime bytes a retained sibling generation of the same Plugin already
/// staged, so an unchanged component is not downloaded again for every
/// release. Like the cache this is untrusted input accepted only by exact
/// digest: a damaged, linked or unreadable generation is simply a miss.
pub(super) fn read_retained(content: &Path, digest: &str) -> Option<Vec<u8>> {
    let current = content.parent()?;
    let generations = current.parent()?;
    for entry in fs::read_dir(generations)
        .ok()?
        .flatten()
        .take(MAX_RETAINED_GENERATIONS)
    {
        let generation = entry.path();
        if generation == current || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let candidate = generation.join("content");
        let Ok(metadata) = read_installed_runtime(&candidate) else {
            continue;
        };
        for command in metadata.commands.values() {
            if !command.artifact_digest.eq_ignore_ascii_case(digest) {
                continue;
            }
            let path = candidate.join(&command.artifact);
            if ensure_within(&candidate, &path).is_ok()
                && let Ok(bytes) = read_verified(&path, digest)
            {
                return Some(bytes);
            }
        }
    }
    None
}

impl MachinePluginStore {
    /// Import opaque public bytes only; never opens identity, credentials,
    /// operation journals, generation pointers, or a Controller connection.
    pub(crate) fn cache_runtime_artifact(
        state: &Path,
        source: &Path,
        digest: &str,
    ) -> Result<usize> {
        ensure!(
            state.is_absolute() && state.is_dir(),
            "existing absolute Service state directory required"
        );
        let path = cache_path(state, digest)?;
        let bytes = read_verified(source, digest)?;
        let directory = path.parent().context("artifact cache directory")?;
        if !directory.try_exists()? {
            fs::DirBuilder::new().mode(0o700).create(directory)?;
        }
        validate_directory(directory)?;
        atomic_write(&path, &bytes, 0o600)?;
        Ok(bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[tokio::test]
    async fn verified_cache_stages_without_contacting_the_artifact_origin() {
        use cowboy_provider_sdk::{
            PrivateComponentKind, ProviderArtifactFormat, ProviderArtifactProbe,
        };
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("download");
        let bytes = b"#!/bin/sh\nexit 0\n";
        fs::write(&source, bytes).unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        MachinePluginStore::cache_runtime_artifact(root.path(), &source, &digest).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let artifact = ReleasedPrivateComponent {
            kind: PrivateComponentKind::ProviderCli,
            slot: "fixture".into(),
            dependency: "fixture".into(),
            version: "1.0.0".into(),
            command: "fixture".into(),
            artifact_url: format!("http://{}/artifact", listener.local_addr().unwrap()),
            artifact_digest: digest,
            artifact_format: ProviderArtifactFormat::Raw,
            entrypoint: None,
            probe: ProviderArtifactProbe {
                args: vec![],
                timeout_ms: 1_000,
            },
        };
        let _ = rustls::crypto::ring::default_provider().install_default();
        let staged = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            stage_runtime_component(
                &reqwest::Client::new(),
                &root.path().join("staging"),
                &artifact,
                &InstallGuard::Legacy,
                Some(root.path()),
                None,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(fs::read(staged.executable).unwrap(), bytes);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn retained_generation_supplies_only_exact_unchanged_runtime_bytes() {
        let root = tempfile::tempdir().unwrap();
        let generations = root.path().join("generations");
        let bytes = b"unchanged provider cli";
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let retained = generations.join("old/content");
        fs::create_dir_all(retained.join("runtime/provider_cli-claude")).unwrap();
        fs::write(
            retained.join("runtime/provider_cli-claude/artifact.tar.gz"),
            bytes,
        )
        .unwrap();
        let metadata = |artifact: &str| {
            serde_json::json!({"schema_version": 2, "commands": {"claude": {
                "executable": "runtime/provider_cli-claude/content/claude",
                "artifact": artifact,
                "artifact_digest": digest,
            }}})
            .to_string()
        };
        fs::write(
            retained.join("runtime/metadata.json"),
            metadata("runtime/provider_cli-claude/artifact.tar.gz"),
        )
        .unwrap();
        let current = generations.join("new/content");
        fs::create_dir_all(&current).unwrap();

        assert_eq!(read_retained(&current, &digest).unwrap(), bytes);
        let other = format!("sha256:{:x}", Sha256::digest(b"changed adapter"));
        assert!(read_retained(&current, &other).is_none());
        // The current generation is never its own source.
        assert!(read_retained(&retained, &digest).is_none());

        // Escaping metadata or tampered bytes are misses, never inputs.
        fs::write(retained.join("runtime/metadata.json"), metadata("../../x")).unwrap();
        assert!(read_retained(&current, &digest).is_none());
        fs::write(
            retained.join("runtime/metadata.json"),
            metadata("runtime/provider_cli-claude/artifact.tar.gz"),
        )
        .unwrap();
        fs::write(
            retained.join("runtime/provider_cli-claude/artifact.tar.gz"),
            b"tampered",
        )
        .unwrap();
        assert!(read_retained(&current, &digest).is_none());
    }

    #[test]
    fn opaque_import_is_not_installation_and_cache_tampering_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("download");
        fs::write(&source, b"public signed artifact bytes").unwrap();
        let digest = format!(
            "sha256:{:x}",
            Sha256::digest(b"public signed artifact bytes")
        );
        assert!(read(root.path(), &digest).unwrap().is_none());
        let uppercase = format!("sha256:{}", digest[7..].to_ascii_uppercase());
        assert!(read(root.path(), &uppercase).unwrap().is_none());
        MachinePluginStore::cache_runtime_artifact(root.path(), &source, &digest).unwrap();
        assert_eq!(
            read(root.path(), &uppercase).unwrap(),
            read(root.path(), &digest).unwrap()
        );
        assert_eq!(
            read(root.path(), &digest).unwrap().unwrap(),
            b"public signed artifact bytes"
        );
        assert!(!root.path().join("plugins").exists());
        assert!(!root.path().join("plugin-operations").exists());
        let cached = cache_path(root.path(), &digest).unwrap();
        fs::write(&cached, b"tampered").unwrap();
        assert!(read(root.path(), &digest).is_err());
        fs::remove_file(&cached).unwrap();
        symlink(&source, &cached).unwrap();
        assert!(read(root.path(), &digest).is_err());
    }

    #[test]
    fn unsafe_paths_and_wrong_digests_never_create_cache_state() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("download");
        fs::write(&source, b"public").unwrap();
        assert!(
            MachinePluginStore::cache_runtime_artifact(root.path(), &source, "sha256:../escape")
                .is_err()
        );
        assert!(
            MachinePluginStore::cache_runtime_artifact(
                root.path(),
                &source,
                &format!("sha256:{}", "0".repeat(64))
            )
            .is_err()
        );
        assert!(!root.path().join("artifact-cache").exists());
        let external = tempfile::tempdir().unwrap();
        symlink(external.path(), root.path().join("artifact-cache")).unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(b"public"));
        assert!(MachinePluginStore::cache_runtime_artifact(root.path(), &source, &digest).is_err());
        assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    }
}
