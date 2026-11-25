//! Isolated staging and multi-phase verification of signed Plugin runtime
//! components.
//!
//! No `artifact-cache` hardlink is placed inside `.runtime.*.partial` while any
//! component probe, Agent launcher probe, or Code readiness probe runs. Both
//! the staging root and each component directory are pinned open with
//! `O_NOFOLLOW | O_DIRECTORY`, every reachable component is verified against a
//! digest-checked source before each probe and again after the final probe, and
//! canonical `0o400` cache hardlinks are attached through pinned directory FDs
//! only when all probes' descendant exits are confirmed (otherwise private
//! copies are written).

use super::*;
use std::os::fd::{AsFd as _, OwnedFd};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirId {
    dev: libc::dev_t,
    ino: libc::ino_t,
}

fn open_pinned_dir(path: &Path) -> Result<(OwnedFd, DirId)> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)
        .with_context(|| format!("opening pinned directory {}", path.display()))?;
    let stat = rustix::fs::fstat(&file)?;
    ensure!(
        (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR,
        "expected directory at {}",
        path.display()
    );
    Ok((
        file.into(),
        DirId {
            dev: stat.st_dev,
            ino: stat.st_ino,
        },
    ))
}

fn verify_child_dir(
    parent_fd: std::os::fd::BorrowedFd<'_>,
    name: &str,
    expected: DirId,
) -> Result<()> {
    let stat = rustix::fs::statat(parent_fd, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        .with_context(|| format!("verifying staged directory {name}"))?;
    let is_dir = (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR;
    ensure!(
        is_dir && stat.st_dev == expected.dev && stat.st_ino == expected.ino,
        "runtime probe replaced staged directory {name}"
    );
    Ok(())
}

enum PendingArchive {
    Inline,
    Cached(artifact_cache::CachedArtifact),
    Bytes(Vec<u8>),
}

pub(super) struct StagedRuntimeComponent {
    component_name: String,
    component_root: PathBuf,
    component_dir_fd: OwnedFd,
    component_identity: DirId,
    pub(super) executable: PathBuf,
    artifact: PathBuf,
    pending_archive: PendingArchive,
}

impl StagedRuntimeComponent {
    /// Verify this component's staged files and executable permissions against
    /// a digest-checked source without relying on any `artifact.tar.gz` inside
    /// staging.
    fn verify_staged_bytes(
        &self,
        temp_dir_fd: Option<std::os::fd::BorrowedFd<'_>>,
        artifact: &ReleasedPrivateComponent,
    ) -> Result<()> {
        if let Some(parent_fd) = temp_dir_fd {
            verify_child_dir(parent_fd, &self.component_name, self.component_identity)?;
        }
        let exec_meta = self.executable.symlink_metadata().with_context(|| {
            format!("inspecting staged executable {}", self.executable.display())
        })?;
        ensure!(
            exec_meta.is_file() && (exec_meta.permissions().mode() & 0o100) != 0,
            "runtime component probe replaced or removed execute permission on staged executable"
        );
        match &self.pending_archive {
            PendingArchive::Inline => {
                ensure!(
                    digest_file(&self.executable)? == artifact.artifact_digest.to_ascii_lowercase(),
                    "runtime component probe changed signed artifact bytes"
                );
            }
            PendingArchive::Cached(cached) => {
                ensure!(
                    archive_runtime_matches_reader(
                        cached.verified_reader(&artifact.artifact_digest)?,
                        &self.component_root.join("content"),
                    )?,
                    "runtime component probe changed extracted runtime bytes"
                );
            }
            PendingArchive::Bytes(bytes) => {
                ensure!(
                    archive_runtime_matches_reader(
                        std::io::Cursor::new(bytes),
                        &self.component_root.join("content"),
                    )?,
                    "runtime component probe changed extracted runtime bytes"
                );
            }
        }
        Ok(())
    }
}

/// Result of preparing a generation's runtime before outer readiness probes
/// (Agent launcher `--version` or Code runtime readiness).
pub(super) enum PreparedRuntime {
    AlreadyInstalled { metadata: InstalledRuntimeMetadata },
    Pending(PendingRuntime),
}

pub(super) struct PendingRuntime {
    content_dir_fd: OwnedFd,
    temp_name: String,
    temp_dir_fd: OwnedFd,
    temp_identity: DirId,
    staging: artifact_cache::StagedRuntimeDirectory,
    components: Vec<(ReleasedPrivateComponent, StagedRuntimeComponent, u128)>,
    all_probes_exit_confirmed: bool,
}

impl PreparedRuntime {
    pub(super) fn is_already_installed(&self) -> bool {
        matches!(self, Self::AlreadyInstalled { .. })
    }

    /// Resolve a staged command path for an outer readiness probe (running
    /// inside `.runtime.*.partial` when newly staged, with no cache hardlinks).
    pub(super) fn command_path(&self, command: &str) -> Result<PathBuf> {
        match self {
            Self::AlreadyInstalled { .. } => {
                bail!("outer probe is skipped for an already-installed verified generation")
            }
            Self::Pending(pending) => pending
                .components
                .iter()
                .find(|(artifact, _, _)| artifact.command == command)
                .map(|(_, staged, _)| staged.executable.clone())
                .with_context(|| format!("staged runtime does not export command {command}")),
        }
    }

    /// Re-verify staging root and every staged component right before an outer
    /// readiness probe.
    pub(super) fn verify_before_outer_probe(&self) -> Result<()> {
        if let Self::Pending(pending) = self {
            verify_child_dir(
                pending.content_dir_fd.as_fd(),
                &pending.temp_name,
                pending.temp_identity,
            )?;
            for (artifact, staged, _) in &pending.components {
                staged.verify_staged_bytes(Some(pending.temp_dir_fd.as_fd()), artifact)?;
            }
        }
        Ok(())
    }

    /// Record whether an outer readiness probe confirmed every descendant exited.
    pub(super) fn record_outer_probe_exit_confirmed(&mut self, confirmed: bool) {
        if let Self::Pending(pending) = self {
            pending.all_probes_exit_confirmed &= confirmed;
        }
    }

    /// Finalize after ALL probes have completed: verify pinned staging root and
    /// component dirs, attach canonical `0o400` cache hardlinks (or private
    /// copies if descendant exit could not be confirmed) via pinned directory
    /// FDs, re-verify every component, write `metadata.json`, and atomically
    /// publish `runtime/` relative to `content_dir_fd`.
    pub(super) fn finalize(self, guard: &InstallGuard<'_>) -> Result<InstalledRuntimeMetadata> {
        match self {
            Self::AlreadyInstalled { metadata } => Ok(metadata),
            Self::Pending(mut pending) => {
                verify_child_dir(
                    pending.content_dir_fd.as_fd(),
                    &pending.temp_name,
                    pending.temp_identity,
                )?;
                let temporary = pending.staging.path().to_path_buf();
                let mut commands = BTreeMap::new();
                for (artifact, staged, probe_ms) in &pending.components {
                    guard.check()?;
                    attach_and_verify_staged_component(
                        Some(pending.temp_dir_fd.as_fd()),
                        staged,
                        artifact,
                        *probe_ms,
                        pending.all_probes_exit_confirmed,
                    )?;
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
                guard.check()?;
                verify_child_dir(
                    pending.content_dir_fd.as_fd(),
                    &pending.temp_name,
                    pending.temp_identity,
                )?;
                artifact_cache::write_bytes_at(
                    pending.temp_dir_fd.as_fd(),
                    "metadata.json",
                    &serde_json::to_vec(&metadata)?,
                )?;
                rustix::fs::renameat(
                    pending.content_dir_fd.as_fd(),
                    pending.temp_name.as_str(),
                    pending.content_dir_fd.as_fd(),
                    "runtime",
                )?;
                rustix::fs::fsync(pending.content_dir_fd.as_fd())?;
                pending.staging.commit();
                Ok(metadata)
            }
        }
    }
}

fn harden_extracted_tree(root: &Path, executable: &Path) -> Result<()> {
    fn walk(path: &Path, executable: &Path) -> Result<()> {
        let meta = path.symlink_metadata()?;
        if meta.is_dir() {
            for entry in fs::read_dir(path)? {
                walk(&entry?.path(), executable)?;
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o500))?;
        } else if meta.is_file() {
            let mode = if path == executable || (meta.permissions().mode() & 0o111) != 0 {
                0o500
            } else {
                0o400
            };
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        }
        Ok(())
    }
    walk(root, executable)
}

pub(super) async fn prepare_provider_runtime(
    content: &Path,
    artifacts: &PlatformRuntimeArtifacts,
    guard: &InstallGuard<'_>,
    cache_state: Option<&Path>,
    sealed_probe: bool,
) -> Result<PreparedRuntime> {
    guard.check()?;
    let active = content.join("runtime");
    if active.symlink_metadata().is_ok() {
        let metadata = read_installed_runtime(content)
            .context("existing Provider runtime metadata is invalid")?;
        ensure!(
            installed_runtime_matches(content, artifacts, &metadata)?,
            "existing content-addressed Provider runtime failed integrity verification"
        );
        return Ok(PreparedRuntime::AlreadyInstalled { metadata });
    }
    let (content_dir_fd, _) = open_pinned_dir(content)?;
    let temp_name = format!(
        ".runtime.{}.{}.partial",
        std::process::id(),
        ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let staging = artifact_cache::StagedRuntimeDirectory::new(content.join(&temp_name));
    let temporary = staging.path().to_path_buf();
    fs::create_dir_all(&temporary)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700))?;
    let (temp_dir_fd, temp_identity) = open_pinned_dir(&temporary)?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::new();
    let mut staged_components = Vec::with_capacity(artifacts.components.len());
    for artifact in &artifacts.components {
        guard.check()?;
        let staged = materialize_runtime_component(
            &client,
            &temporary,
            artifact,
            guard,
            cache_state,
            Some(content),
        )
        .await?;
        staged_components.push((artifact.clone(), staged));
    }
    let mut probed = Vec::with_capacity(staged_components.len());
    let mut all_probes_exit_confirmed = true;
    for (idx, (artifact, staged)) in staged_components.iter().enumerate() {
        guard.check()?;
        let probe_started = Instant::now();
        if !sealed_probe {
            // Re-verify staging root AND every reachable component before
            // executing each subsequent probe so no earlier probe can alter
            // what this probe executes or loads from any sibling.
            if idx > 0 {
                verify_child_dir(content_dir_fd.as_fd(), &temp_name, temp_identity)?;
                for (a, s) in &staged_components {
                    s.verify_staged_bytes(Some(temp_dir_fd.as_fd()), a)?;
                }
            }
            let confirmed = probe_released_component(&staged.executable, artifact).await?;
            all_probes_exit_confirmed &= confirmed;
        }
        probed.push(probe_started.elapsed().as_millis());
    }
    let components = staged_components
        .into_iter()
        .zip(probed)
        .map(|((artifact, staged), probe_ms)| (artifact, staged, probe_ms))
        .collect();
    Ok(PreparedRuntime::Pending(PendingRuntime {
        content_dir_fd,
        temp_name,
        temp_dir_fd,
        temp_identity,
        staging,
        components,
        all_probes_exit_confirmed,
    }))
}

#[cfg(test)]
pub(super) async fn stage_provider_runtime(
    content: &Path,
    artifacts: &PlatformRuntimeArtifacts,
    guard: &InstallGuard<'_>,
    cache_state: Option<&Path>,
    sealed_probe: bool,
) -> Result<InstalledRuntimeMetadata> {
    let prepared =
        prepare_provider_runtime(content, artifacts, guard, cache_state, sealed_probe).await?;
    prepared.finalize(guard)
}

#[cfg(test)]
pub(super) async fn stage_runtime_component(
    client: &reqwest::Client,
    runtime_root: &Path,
    artifact: &ReleasedPrivateComponent,
    guard: &InstallGuard<'_>,
    cache_state: Option<&Path>,
    generation_content: Option<&Path>,
    sealed_probe: bool,
) -> Result<StagedRuntimeComponent> {
    let staged = materialize_runtime_component(
        client,
        runtime_root,
        artifact,
        guard,
        cache_state,
        generation_content,
    )
    .await?;
    let probe_started = Instant::now();
    let confirmed = if sealed_probe {
        true
    } else {
        probe_released_component(&staged.executable, artifact).await?
    };
    attach_and_verify_staged_component(
        None,
        &staged,
        artifact,
        probe_started.elapsed().as_millis(),
        confirmed,
    )?;
    Ok(staged)
}

async fn materialize_runtime_component(
    client: &reqwest::Client,
    runtime_root: &Path,
    artifact: &ReleasedPrivateComponent,
    guard: &InstallGuard<'_>,
    cache_state: Option<&Path>,
    generation_content: Option<&Path>,
) -> Result<StagedRuntimeComponent> {
    guard.check()?;
    let url = reqwest::Url::parse(&artifact.artifact_url)?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "::1" | "localhost"));
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "Provider runtime artifact must use HTTPS"
    );
    let started = Instant::now();
    let cached = cache_state
        .map(|state| artifact_cache::read(state, &artifact.artifact_digest))
        .transpose()?
        .flatten();
    let (bytes, source) = if let Some(bytes) = cached {
        (bytes, "cache")
    } else if let Some(bytes) = generation_content
        .and_then(|content| artifact_cache::read_retained(content, &artifact.artifact_digest))
    {
        (bytes, "retained_generation")
    } else {
        (
            download_runtime_artifact(client, url, guard).await?,
            "download",
        )
    };
    let fetch_ms = started.elapsed().as_millis();
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    ensure!(
        digest == artifact.artifact_digest.to_ascii_lowercase(),
        "Provider runtime artifact digest mismatch"
    );
    guard.check()?;
    let unpack_started = Instant::now();
    let component_name = format!("{}-{}", artifact.kind.as_str(), artifact.slot);
    let component_root = runtime_root.join(&component_name);
    fs::create_dir_all(&component_root)?;
    let (component_dir_fd, component_identity) = open_pinned_dir(&component_root)?;

    let byte_len = bytes.len();
    let (executable, stored_artifact, pending_archive) = match artifact.artifact_format {
        ProviderArtifactFormat::Raw => {
            let executable = component_root.join("bin");
            atomic_write(&executable, &bytes, 0o700)?;
            (executable.clone(), executable, PendingArchive::Inline)
        }
        ProviderArtifactFormat::TarGz => {
            let stored_artifact = component_root.join("artifact.tar.gz");
            let archive_root = component_root.join("content");
            fs::create_dir_all(&archive_root)?;
            extract_provider_tar_gz(&archive_root, &bytes)?;
            let pending = if let Some(state) = cache_state {
                PendingArchive::Cached(artifact_cache::ensure_cached(
                    state,
                    &artifact.artifact_digest,
                    &bytes,
                )?)
            } else {
                PendingArchive::Bytes(bytes)
            };
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
            harden_extracted_tree(&archive_root, &executable)?;
            (executable, stored_artifact, pending)
        }
    };
    tracing::info!(
        command = %artifact.command,
        source,
        bytes = byte_len,
        fetch_ms,
        unpack_ms = unpack_started.elapsed().as_millis(),
        "Provider runtime component materialized"
    );
    Ok(StagedRuntimeComponent {
        component_name,
        component_root,
        component_dir_fd,
        component_identity,
        executable,
        artifact: stored_artifact,
        pending_archive,
    })
}

fn attach_and_verify_staged_component(
    temp_dir_fd: Option<std::os::fd::BorrowedFd<'_>>,
    staged: &StagedRuntimeComponent,
    artifact: &ReleasedPrivateComponent,
    probe_ms: u128,
    allow_shared_hardlink: bool,
) -> Result<()> {
    if let Some(parent_fd) = temp_dir_fd {
        verify_child_dir(parent_fd, &staged.component_name, staged.component_identity)?;
    }
    match &staged.pending_archive {
        PendingArchive::Inline => {}
        PendingArchive::Cached(cached) => {
            if allow_shared_hardlink {
                artifact_cache::link_verified_at(
                    cached,
                    &artifact.artifact_digest,
                    staged.component_dir_fd.as_fd(),
                    "artifact.tar.gz",
                )?;
            } else {
                artifact_cache::copy_verified_at(
                    cached,
                    &artifact.artifact_digest,
                    staged.component_dir_fd.as_fd(),
                    "artifact.tar.gz",
                )?;
            }
        }
        PendingArchive::Bytes(bytes) => {
            artifact_cache::write_bytes_at(
                staged.component_dir_fd.as_fd(),
                "artifact.tar.gz",
                bytes,
            )?;
        }
    }
    let verify_started = Instant::now();
    let exec_meta = staged.executable.symlink_metadata().with_context(|| {
        format!(
            "inspecting staged executable {}",
            staged.executable.display()
        )
    })?;
    ensure!(
        exec_meta.is_file() && (exec_meta.permissions().mode() & 0o100) != 0,
        "runtime component probe replaced or removed execute permission on staged executable"
    );
    let art_meta = staged
        .artifact
        .symlink_metadata()
        .with_context(|| format!("inspecting staged artifact {}", staged.artifact.display()))?;
    ensure!(
        art_meta.is_file(),
        "staged runtime artifact must be a regular file"
    );
    ensure!(
        digest_file(&staged.artifact)? == artifact.artifact_digest.to_ascii_lowercase(),
        "runtime component probe changed signed artifact bytes"
    );
    if artifact.artifact_format == ProviderArtifactFormat::TarGz {
        ensure!(
            archive_runtime_matches(&staged.artifact, &staged.component_root.join("content"))?,
            "runtime component probe changed extracted runtime bytes"
        );
    }
    tracing::info!(
        command = %artifact.command,
        probe_ms,
        verify_ms = verify_started.elapsed().as_millis(),
        "Provider runtime component verified"
    );
    Ok(())
}
