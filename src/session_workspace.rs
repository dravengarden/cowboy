//! Machine-local preparation of isolated Git worktrees for Cowboy sessions.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::Read as _;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const CARGO_CACHE_TAG_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";
const MAX_CARGO_CACHE_TAG_BYTES: u64 = 8192;
const MAX_CLEANUP_DIRECTORIES: usize = 100_000;
const MAX_CLEANUP_TARGETS: usize = 128;

#[derive(Debug, Deserialize)]
pub struct PrepareWorkspaceRequest {
    pub root: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedWorkspace {
    pub path: String,
    pub source_path: String,
    pub revision: Option<String>,
    pub upstream_ref: Option<String>,
    pub isolated: bool,
    pub created: bool,
}

/// Prepare or reuse the checkout owned by one Cowboy session.
///
/// Repositories without remotes use committed HEAD. Remote-backed roots fail
/// closed when their remote default branch cannot be fetched. Non-Git roots
/// remain shared because Git cannot isolate them.
pub async fn prepare(
    request: PrepareWorkspaceRequest,
    worktree_root: &Path,
) -> Result<PreparedWorkspace> {
    validate_session_id(&request.session_id)?;
    let _session_lock = acquire_session_lock(worktree_root, &request.session_id).await?;
    let source = PathBuf::from(&request.root)
        .canonicalize()
        .with_context(|| format!("canonicalizing session workspace {:?}", request.root))?;
    if !source.is_dir() {
        bail!("session workspace is not a directory: {}", source.display());
    }

    let Some(repository) = git_maybe(&source, ["rev-parse", "--show-toplevel"]).await? else {
        return Ok(PreparedWorkspace {
            path: source.display().to_string(),
            source_path: source.display().to_string(),
            revision: None,
            upstream_ref: None,
            isolated: false,
            created: false,
        });
    };
    let repository = PathBuf::from(repository)
        .canonicalize()
        .with_context(|| format!("canonicalizing Git repository for {}", source.display()))?;
    let relative_path = source
        .strip_prefix(&repository)
        .with_context(|| {
            format!(
                "selected workspace {} is outside Git repository {}",
                source.display(),
                repository.display()
            )
        })?
        .to_path_buf();
    let destination = worktree_root.join(&request.session_id);
    let session_branch = format!("cowboy/{}", request.session_id);
    git_output(
        &repository,
        ["check-ref-format", "--branch", session_branch.as_str()],
    )
    .await
    .context("validating session branch")?;
    let session_branch_ref = format!("refs/heads/{session_branch}");

    if tokio::fs::try_exists(&destination).await? {
        return reuse_existing(
            &repository,
            &source,
            &relative_path,
            &destination,
            &session_branch,
            &session_branch_ref,
        )
        .await;
    }

    let base_ref = format!("refs/cowboy/session-bases/{}", request.session_id);
    let branch_existed = git_ref_exists(&repository, &session_branch_ref).await?;
    let (revision, head_ref, base_ref_created) = if branch_existed {
        (
            git_output(
                &repository,
                ["rev-parse", &format!("{session_branch_ref}^{{commit}}")],
            )
            .await
            .context("resolving existing session branch")?,
            None,
            false,
        )
    } else if git_output(&repository, ["remote"]).await?.is_empty() {
        (
            git_output(&repository, ["rev-parse", "--verify", "HEAD^{commit}"])
                .await
                .context("local workspace needs an initial commit before it can be isolated")?,
            None,
            false,
        )
    } else {
        let remote_head = git_output(&repository, ["ls-remote", "--symref", "origin", "HEAD"])
            .await
            .context("resolving origin default branch")?;
        let head_ref = parse_remote_head(&remote_head)?;
        let remote_branch = head_ref
            .strip_prefix("refs/heads/")
            .context("origin HEAD is not a branch")?;
        git_output(&repository, ["check-ref-format", "--branch", remote_branch])
            .await
            .context("validating origin default branch")?;
        let refspec = format!("+{head_ref}:{base_ref}");
        git_output(&repository, ["fetch", "origin", refspec.as_str()])
            .await
            .context("fetching origin default branch for isolated session")?;
        let revision_spec = format!("{base_ref}^{{commit}}");
        (
            git_output(&repository, ["rev-parse", revision_spec.as_str()])
                .await
                .context("resolving fetched session base")?,
            Some(head_ref),
            true,
        )
    };

    tokio::fs::create_dir_all(worktree_root)
        .await
        .with_context(|| format!("creating worktree root {}", worktree_root.display()))?;
    if branch_existed {
        remove_stale_destination_registration(&repository, &destination).await?;
    }
    let added = if branch_existed {
        git_output(
            &repository,
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                destination.as_os_str(),
                OsStr::new(&session_branch_ref),
            ],
        )
        .await
    } else {
        git_output(
            &repository,
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("-b"),
                OsStr::new(&session_branch),
                destination.as_os_str(),
                OsStr::new(&revision),
            ],
        )
        .await
    };
    if let Err(error) = added.with_context(|| {
        format!(
            "creating isolated session worktree {}",
            destination.display()
        )
    }) {
        // The command may have created a branch or a recoverable partial
        // worktree before failing. Preserve both; a retry can prove and reuse
        // them. Only the internal fetched-base ref is disposable here.
        if base_ref_created {
            let _ = git_command(&repository, ["update-ref", "-d", &base_ref]).await;
        }
        return Err(error);
    }
    let validated = async {
        let checkout = destination
            .canonicalize()
            .with_context(|| format!("canonicalizing new worktree {}", destination.display()))?;
        let path = checkout
            .join(&relative_path)
            .canonicalize()
            .with_context(|| {
                format!(
                    "canonicalizing selected workspace {} in new worktree {}",
                    relative_path.display(),
                    checkout.display()
                )
            })?;
        if !path.starts_with(&checkout) {
            bail!(
                "selected workspace escapes new session worktree {}",
                checkout.display()
            );
        }
        Ok::<_, anyhow::Error>(path)
    }
    .await;
    let path = match validated {
        Ok(value) => value,
        Err(error) => {
            cleanup_failed_creation(
                &repository,
                &destination,
                &base_ref,
                &session_branch_ref,
                base_ref_created,
                !branch_existed,
            )
            .await;
            return Err(error);
        }
    };
    Ok(PreparedWorkspace {
        path: path.display().to_string(),
        source_path: source.display().to_string(),
        revision: Some(revision),
        upstream_ref: head_ref,
        isolated: true,
        created: true,
    })
}

async fn acquire_session_lock(worktree_root: &Path, session_id: &str) -> Result<File> {
    let lock_path = worktree_root.join(".locks").join(session_id);
    tokio::task::spawn_blocking(move || {
        let parent = lock_path
            .parent()
            .context("session lock path has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating session lock directory {}", parent.display()))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("opening session lock {}", lock_path.display()))?;
        file.lock()
            .with_context(|| format!("locking session preparation {}", lock_path.display()))?;
        Ok::<_, anyhow::Error>(file)
    })
    .await
    .context("joining session lock acquisition")?
}

async fn remove_stale_destination_registration(
    repository: &Path,
    destination: &Path,
) -> Result<()> {
    if tokio::fs::try_exists(destination).await? {
        bail!(
            "session worktree {} appeared while it was being prepared; retry to reuse it",
            destination.display()
        );
    }
    let listed = git_output(repository, ["worktree", "list", "--porcelain", "-z"])
        .await
        .context("listing registered Git worktrees")?;
    let registered = listed.split('\0').any(|field| {
        field
            .strip_prefix("worktree ")
            .is_some_and(|path| Path::new(path) == destination)
    });
    if !registered {
        return Ok(());
    }
    if tokio::fs::try_exists(destination).await? {
        bail!(
            "session worktree {} reappeared before stale registration cleanup; retry to reuse it",
            destination.display()
        );
    }
    git_output(
        repository,
        [
            OsStr::new("worktree"),
            OsStr::new("remove"),
            OsStr::new("--force"),
            destination.as_os_str(),
        ],
    )
    .await
    .with_context(|| {
        format!(
            "removing stale Git registration for missing session worktree {}",
            destination.display()
        )
    })?;
    Ok(())
}

async fn cleanup_failed_creation(
    repository: &Path,
    destination: &Path,
    base_ref: &str,
    session_branch_ref: &str,
    remove_base_ref: bool,
    remove_session_branch: bool,
) {
    let _ = git_command(
        repository,
        [
            OsStr::new("worktree"),
            OsStr::new("remove"),
            OsStr::new("--force"),
            destination.as_os_str(),
        ],
    )
    .await;
    if tokio::fs::try_exists(destination).await.unwrap_or(false) {
        let _ = tokio::fs::remove_dir_all(destination).await;
    }
    if remove_base_ref {
        let _ = git_command(repository, ["update-ref", "-d", base_ref]).await;
    }
    if remove_session_branch {
        let _ = git_command(repository, ["update-ref", "-d", session_branch_ref]).await;
    }
}

async fn reuse_existing(
    repository: &Path,
    source: &Path,
    relative_path: &Path,
    destination: &Path,
    session_branch: &str,
    session_branch_ref: &str,
) -> Result<PreparedWorkspace> {
    let metadata =
        std::fs::symlink_metadata(destination).context("reading existing session worktree")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "existing session worktree is not a real directory: {}",
            destination.display()
        );
    }
    let checkout = destination
        .canonicalize()
        .with_context(|| format!("canonicalizing existing worktree {}", destination.display()))?;
    let managed_root = destination
        .parent()
        .context("session worktree has no parent")?
        .canonicalize()?;
    if checkout.parent() != Some(managed_root.as_path()) {
        bail!(
            "existing session worktree is outside managed root: {}",
            checkout.display()
        );
    }
    let source_common = PathBuf::from(
        git_output(
            repository,
            ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await?,
    )
    .canonicalize()?;
    let destination_common = PathBuf::from(
        git_output(
            &checkout,
            ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await
        .context("existing session path is not a Git worktree")?,
    )
    .canonicalize()?;
    if source_common != destination_common {
        bail!(
            "existing session path {} belongs to another repository",
            checkout.display()
        );
    }
    let path = checkout
        .join(relative_path)
        .canonicalize()
        .context("canonicalizing selected workspace in existing worktree")?;
    if !path.starts_with(&checkout) {
        bail!(
            "selected workspace escapes existing session worktree {}",
            checkout.display()
        );
    }
    if current_branch(&checkout).await?.is_none() {
        if git_ref_exists(repository, session_branch_ref).await? {
            let detached_revision = git_output(&checkout, ["rev-parse", "HEAD^{commit}"]).await?;
            let branch_revision = git_output(
                repository,
                ["rev-parse", &format!("{session_branch_ref}^{{commit}}")],
            )
            .await?;
            if detached_revision != branch_revision {
                bail!(
                    "legacy detached session {} diverges from existing task branch {session_branch}; preserving both",
                    checkout.display()
                );
            }
            git_output(&checkout, ["switch", session_branch])
                .await
                .context("attaching legacy session worktree to its existing task branch")?;
        } else {
            git_output(&checkout, ["switch", "-c", session_branch])
                .await
                .context("anchoring legacy detached session worktree on a task branch")?;
        }
    }
    let revision = git_output(&checkout, ["rev-parse", "HEAD^{commit}"]).await?;
    Ok(PreparedWorkspace {
        path: path.display().to_string(),
        source_path: source.display().to_string(),
        revision: Some(revision),
        upstream_ref: None,
        isolated: true,
        created: false,
    })
}

/// Original directory object observed when terminal deletion was accepted.
/// Keeping the handle prevents inode reuse while asynchronous cleanup waits.
#[derive(Clone)]
pub struct CleanupWorkspace {
    worktree_root: PathBuf,
    session_id: String,
    cwd: PathBuf,
    directory: Arc<File>,
}

#[derive(Debug)]
pub struct CleanupRootChanged;

impl std::fmt::Display for CleanupRootChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("original cleanup worktree directory was replaced or removed")
    }
}

impl std::error::Error for CleanupRootChanged {}

impl CleanupWorkspace {
    fn access_root(&self, logical_root: &Path) -> PathBuf {
        directory_access_root(&self.directory, logical_root)
    }

    fn verify(&self) -> Result<()> {
        let path = self.worktree_root.join(&self.session_id);
        let current = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Err(CleanupRootChanged.into());
            }
            Err(error) => return Err(error.into()),
        };
        let original = self.directory.metadata()?;
        if !current.is_dir()
            || current.file_type().is_symlink()
            || current.dev() != original.dev()
            || current.ino() != original.ino()
        {
            return Err(CleanupRootChanged.into());
        }
        Ok(())
    }
}

fn directory_access_root(directory: &File, logical_root: &Path) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let _ = logical_root;
        PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = directory;
        logical_root.to_owned()
    }
}

#[derive(Debug)]
pub struct CleanupTargetChanged;

impl std::fmt::Display for CleanupTargetChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("observed Cargo target or its markers changed before cleanup")
    }
}

impl std::error::Error for CleanupTargetChanged {}

struct CleanupTarget {
    path: PathBuf,
    directory: File,
}

impl CleanupTarget {
    fn verify(&self, workspace: &CleanupWorkspace) -> Result<()> {
        workspace.verify()?;
        let current = match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Err(CleanupTargetChanged.into());
            }
            Err(error) => return Err(error.into()),
        };
        let original = self.directory.metadata()?;
        if !current.is_dir()
            || current.file_type().is_symlink()
            || current.dev() != original.dev()
            || current.ino() != original.ino()
        {
            return Err(CleanupTargetChanged.into());
        }
        Ok(())
    }
}

/// Capture the original directory before waiting for the stopped worker to exit.
pub fn capture_cleanup_workspace(
    worktree_root: &Path,
    session_id: &str,
    cwd: &Path,
) -> Result<CleanupWorkspace> {
    validate_session_id(session_id)?;
    let worktree_root = std::path::absolute(worktree_root)?;
    let cwd = std::path::absolute(cwd)?;
    validated_cleanup_root(&worktree_root, session_id, &cwd)?;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(worktree_root.join(session_id))?;
    let workspace = CleanupWorkspace {
        worktree_root,
        session_id: session_id.to_owned(),
        cwd,
        directory: Arc::new(directory),
    };
    workspace.verify()?;
    Ok(workspace)
}

/// Clear Cargo build directory contents from a permanently stopped session worktree.
///
/// The Git worktree and every source file remain intact. A directory is eligible
/// only when it is named `target`, carries both Cargo cache markers, and stays
/// within the exact Machine-owned worktree for `session_id`. Keep the directory
/// itself: its pathname cannot be atomically verified and unlinked.
pub async fn cleanup_build_artifacts(workspace: &CleanupWorkspace) -> Result<Vec<PathBuf>> {
    let workspace = workspace.clone();
    tokio::task::spawn_blocking(move || cleanup_build_artifacts_sync(&workspace))
        .await
        .context("joining session build-artifact cleanup")?
}

fn validated_cleanup_root(worktree_root: &Path, session_id: &str, cwd: &Path) -> Result<PathBuf> {
    let managed_root = worktree_root
        .canonicalize()
        .with_context(|| format!("canonicalizing worktree root {}", worktree_root.display()))?;
    let session_path = worktree_root.join(session_id);
    let metadata = std::fs::symlink_metadata(&session_path)
        .with_context(|| format!("reading session worktree {}", session_path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "session worktree is not a real directory: {}",
            session_path.display()
        );
    }
    let session_root = session_path
        .canonicalize()
        .with_context(|| format!("canonicalizing session worktree {}", session_path.display()))?;
    if session_root.parent() != Some(managed_root.as_path()) {
        bail!(
            "session worktree {} is outside managed root {}",
            session_root.display(),
            managed_root.display()
        );
    }
    let canonical_cwd = cwd
        .canonicalize()
        .with_context(|| format!("canonicalizing stopped session cwd {}", cwd.display()))?;
    if !canonical_cwd.starts_with(&session_root) {
        bail!(
            "stopped session cwd {} is outside its worktree {}",
            canonical_cwd.display(),
            session_root.display()
        );
    }
    Ok(session_root)
}

fn cleanup_build_artifacts_sync(workspace: &CleanupWorkspace) -> Result<Vec<PathBuf>> {
    workspace.verify()?;
    let session_root = validated_cleanup_root(
        &workspace.worktree_root,
        &workspace.session_id,
        &workspace.cwd,
    )?;
    workspace.verify()?;

    let access_root = workspace.access_root(&session_root);
    let mut candidates = Vec::new();
    let mut pending = vec![PathBuf::new()];
    let mut visited = 0_usize;
    while let Some(directory) = pending.pop() {
        workspace.verify()?;
        visited = visited.saturating_add(1);
        if visited > MAX_CLEANUP_DIRECTORIES {
            bail!("session worktree cleanup exceeded {MAX_CLEANUP_DIRECTORIES} directories");
        }
        let Some(handle) = open_cleanup_scan_directory(workspace, &access_root, &directory)? else {
            continue;
        };
        let directory_path = access_root.join(&directory);
        let scan_root = directory_access_root(&handle, &directory_path);
        for entry in std::fs::read_dir(&scan_root)
            .with_context(|| format!("reading worktree directory {}", directory_path.display()))?
        {
            let entry = entry
                .with_context(|| format!("reading worktree entry below {}", directory.display()))?;
            let file_type = entry.file_type().with_context(|| {
                format!("reading worktree entry type {}", entry.path().display())
            })?;
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name();
            let relative_path = directory.join(&name);
            let path = access_root.join(&relative_path);
            if name == OsStr::new("target")
                && let Some(directory) =
                    open_cleanup_scan_directory(workspace, &access_root, &relative_path)?
                && cargo_target_markers_match(&directory)?
            {
                if candidates.len() >= MAX_CLEANUP_TARGETS {
                    bail!(
                        "session cleanup exceeded {MAX_CLEANUP_TARGETS} observed Cargo targets; preserving artifacts"
                    );
                }
                candidates.push(CleanupTarget { path, directory });
                continue;
            }
            if matches!(name.to_str(), Some(".git" | "node_modules" | "vendor")) {
                continue;
            }
            pending.push(relative_path);
        }
    }

    remove_cleanup_targets(workspace, &session_root, &access_root, candidates, &|_| {
        Ok(())
    })
}

/// Linux must resolve every scan component from the retained Session root.
/// Unsupported kernels fail rather than falling back to pathname traversal.
fn open_cleanup_scan_directory(
    workspace: &CleanupWorkspace,
    access_root: &Path,
    relative: &Path,
) -> Result<Option<File>> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        let _ = access_root;
        let relative = if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        };
        match openat2(
            workspace.directory.as_ref(),
            relative,
            OFlags::RDONLY
                | OFlags::DIRECTORY
                | OFlags::NOFOLLOW
                | OFlags::NONBLOCK
                | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
        ) {
            Ok(directory) => Ok(Some(File::from(directory))),
            Err(error)
                if matches!(
                    error,
                    rustix::io::Errno::LOOP
                        | rustix::io::Errno::XDEV
                        | rustix::io::Errno::NOENT
                        | rustix::io::Errno::NOTDIR
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error).context("opening bounded cleanup scan directory"),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = workspace;
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(access_root.join(relative))
        {
            Ok(directory) => Ok(Some(directory)),
            Err(error) => Err(error).context("opening cleanup scan directory"),
        }
    }
}

fn remove_cleanup_targets(
    workspace: &CleanupWorkspace,
    session_root: &Path,
    access_root: &Path,
    mut candidates: Vec<CleanupTarget>,
    before_child_remove: &impl Fn(&Path) -> Result<()>,
) -> Result<Vec<PathBuf>> {
    candidates.sort_by_key(|target| std::cmp::Reverse(target.path.components().count()));
    let mut removed = Vec::with_capacity(candidates.len());
    for target in candidates {
        target.verify(workspace)?;
        if !cargo_target_markers_match(&target.directory)? {
            return Err(CleanupTargetChanged.into());
        }
        let canonical_target = target
            .path
            .canonicalize()
            .with_context(|| format!("canonicalizing Cargo target {}", target.path.display()))?;
        if !canonical_target.starts_with(session_root)
            || canonical_target.file_name() != Some(OsStr::new("target"))
        {
            bail!(
                "refusing Cargo target outside session worktree: {}",
                canonical_target.display()
            );
        }
        let contents_root = directory_access_root(&target.directory, &target.path);
        for entry in std::fs::read_dir(&contents_root)? {
            let entry = entry?;
            target.verify(workspace)?;
            before_child_remove(&entry.path())?;
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())?;
            } else {
                std::fs::remove_file(entry.path())?;
            }
        }
        target.verify(workspace)?;
        // Retain the empty directory. A pathname unlink after identity checking
        // could delete an independently substituted empty directory. Contents
        // above remain attached to the original target on Linux.
        removed.push(session_root.join(target.path.strip_prefix(access_root)?));
    }
    Ok(removed)
}

#[cfg(test)]
fn observe_cargo_target_directory(path: &Path) -> Result<Option<File>> {
    let Ok(directory) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    else {
        return Ok(None);
    };
    if cargo_target_markers_match(&directory)? {
        Ok(Some(directory))
    } else {
        Ok(None)
    }
}

fn cargo_target_markers_match(directory: &File) -> Result<bool> {
    if open_regular_cargo_marker(directory, ".rustc_info.json").is_none() {
        return Ok(false);
    }
    let Some(file) = open_regular_cargo_marker(directory, "CACHEDIR.TAG") else {
        return Ok(false);
    };
    if file.metadata()?.len() > MAX_CARGO_CACHE_TAG_BYTES {
        return Ok(false);
    }
    let mut bytes = Vec::new();
    if file
        .take(MAX_CARGO_CACHE_TAG_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_CARGO_CACHE_TAG_BYTES
    {
        return Ok(false);
    }
    let Ok(tag) = std::str::from_utf8(&bytes) else {
        return Ok(false);
    };
    Ok(tag.lines().any(|line| line == CARGO_CACHE_TAG_SIGNATURE))
}

fn open_regular_cargo_marker(directory: &File, name: &str) -> Option<File> {
    use rustix::fs::{Mode, OFlags, openat};
    let file = File::from(
        openat(
            directory,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .ok()?,
    );
    file.metadata().ok()?.is_file().then_some(file)
}

fn validate_session_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        bail!("invalid session id {value:?}");
    }
    Ok(())
}

fn parse_remote_head(output: &str) -> Result<String> {
    output
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix("ref: ")?;
            let (reference, target) = value.split_once('\t')?;
            (target == "HEAD" && reference.starts_with("refs/heads/")).then(|| reference.to_owned())
        })
        .context("origin did not advertise a default branch")
}

async fn git_ref_exists(repository: &Path, reference: &str) -> Result<bool> {
    let output = git_command(repository, ["show-ref", "--verify", "--quiet", reference]).await?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => bail!(
            "checking Git ref {reference:?} failed in {}: {}",
            repository.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

async fn current_branch(repository: &Path) -> Result<Option<String>> {
    let output = git_command(repository, ["symbolic-ref", "--quiet", "--short", "HEAD"]).await?;
    match output.status.code() {
        Some(0) => Ok(Some(String::from_utf8(output.stdout)?.trim().to_owned())),
        Some(1) => Ok(None),
        _ => bail!(
            "checking current Git branch failed in {}: {}",
            repository.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

async fn git_maybe<I, S>(repository: &Path, args: I) -> Result<Option<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_command(repository, args).await?;
    if output.status.success() {
        Ok(Some(String::from_utf8(output.stdout)?.trim().to_owned()))
    } else if String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
        Ok(None)
    } else {
        bail!(
            "git repository probe failed in {}: {}",
            repository.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

async fn git_output<I, S>(repository: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_command(repository, args).await?;
    if !output.status.success() {
        bail!(
            "git failed in {}: {}",
            repository.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

async fn git_command<I, S>(repository: &Path, args: I) -> Result<std::process::Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repository)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .with_context(|| format!("git timed out in {}", repository.display()))?
        .with_context(|| format!("starting git in {}", repository.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cowboy-session-workspace-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git(path: &Path, args: &[&str]) {
        let status = StdCommand::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "git {args:?} failed in {}",
            path.display()
        );
    }

    #[test]
    fn parses_symbolic_remote_head() {
        assert_eq!(
            parse_remote_head("ref: refs/heads/main\tHEAD\nabc\tHEAD\n").unwrap(),
            "refs/heads/main"
        );
        assert!(parse_remote_head("abc\tHEAD\n").is_err());
    }

    #[test]
    fn session_id_cannot_escape_machine_state() {
        assert!(validate_session_id("sess-123").is_ok());
        assert!(validate_session_id("../stable").is_err());
        assert!(validate_session_id("sess_123").is_err());
    }

    fn write_cargo_target(path: &Path) {
        std::fs::create_dir_all(path.join("debug/deps")).unwrap();
        std::fs::write(path.join(".rustc_info.json"), "{}\n").unwrap();
        std::fs::write(
            path.join("CACHEDIR.TAG"),
            format!("{CARGO_CACHE_TAG_SIGNATURE}\n# cargo cache\n"),
        )
        .unwrap();
        std::fs::write(path.join("debug/deps/libtest.rlib"), "generated\n").unwrap();
    }

    #[tokio::test]
    async fn cleanup_clears_only_marked_targets_inside_exact_session_worktree() {
        let temp = TestDir::new();
        let managed = temp.0.join("managed");
        let session = managed.join("sess-clean");
        let selected = session.join("project/subdir");
        std::fs::create_dir_all(&selected).unwrap();
        write_cargo_target(&session.join("target"));
        write_cargo_target(&session.join("native/replay/target"));
        std::fs::create_dir_all(session.join("examples/target")).unwrap();
        std::fs::write(session.join("examples/target/keep.txt"), "source\n").unwrap();

        let outside = temp.0.join("outside/target");
        write_cargo_target(&outside);
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.parent().unwrap(), session.join("linked-outside"))
            .unwrap();

        let observed = capture_cleanup_workspace(&managed, "sess-clean", &selected).unwrap();
        let removed = cleanup_build_artifacts(&observed).await.unwrap();

        assert_eq!(removed.len(), 2);
        for path in [session.join("target"), session.join("native/replay/target")] {
            assert!(path.is_dir());
            assert_eq!(std::fs::read_dir(path).unwrap().count(), 0);
        }
        assert!(session.join("examples/target/keep.txt").is_file());
        assert!(outside.join("debug/deps/libtest.rlib").is_file());
    }

    #[tokio::test]
    async fn cleanup_retains_directory_identity_and_allows_later_cargo_rebuild() {
        let temp = TestDir::new();
        let managed = temp.0.join("managed");
        let session = managed.join("sess-retained-target");
        let target = session.join("target");
        write_cargo_target(&target);
        let original = File::open(&target).unwrap();
        let observation =
            capture_cleanup_workspace(&managed, "sess-retained-target", &session).unwrap();
        for _ in 0..2 {
            assert_eq!(
                cleanup_build_artifacts(&observation).await.unwrap(),
                vec![target.clone()]
            );
            let retained = std::fs::symlink_metadata(&target).unwrap();
            assert!(retained.is_dir());
            assert_eq!(retained.dev(), original.metadata().unwrap().dev());
            assert_eq!(retained.ino(), original.metadata().unwrap().ino());
            assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
            assert!(
                cleanup_build_artifacts(&observation)
                    .await
                    .unwrap()
                    .is_empty()
            );
            write_cargo_target(&target);
        }
    }

    #[tokio::test]
    async fn cleanup_refuses_replaced_directory_and_preserves_both_objects() {
        let temp = TestDir::new();
        let managed = temp.0.join("managed");
        let session = managed.join("sess-replaced");
        let original = managed.join("retained-original");
        write_cargo_target(&session.join("target"));
        let observation = capture_cleanup_workspace(&managed, "sess-replaced", &session).unwrap();
        std::fs::rename(&session, &original).unwrap();
        write_cargo_target(&session.join("target"));
        std::fs::write(
            session.join("target/debug/deps/libtest.rlib"),
            "replacement",
        )
        .unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(
            std::fs::read_to_string(
                observation
                    .access_root(&session)
                    .join("target/debug/deps/libtest.rlib")
            )
            .unwrap(),
            "generated\n"
        );
        let error = cleanup_build_artifacts(&observation).await.unwrap_err();
        assert!(error.downcast_ref::<CleanupRootChanged>().is_some());
        assert!(session.join("target/debug/deps/libtest.rlib").is_file());
        assert!(original.join("target/debug/deps/libtest.rlib").is_file());

        std::fs::remove_dir_all(&session).unwrap();
        let error = cleanup_build_artifacts(&observation).await.unwrap_err();
        assert!(error.downcast_ref::<CleanupRootChanged>().is_some());
        std::fs::write(&session, "replacement file").unwrap();
        let error = cleanup_build_artifacts(&observation).await.unwrap_err();
        assert!(error.downcast_ref::<CleanupRootChanged>().is_some());
        std::fs::remove_file(&session).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&original, &session).unwrap();
            let error = cleanup_build_artifacts(&observation).await.unwrap_err();
            assert!(error.downcast_ref::<CleanupRootChanged>().is_some());
            assert!(original.join("target/debug/deps/libtest.rlib").is_file());
        }
    }

    #[tokio::test]
    async fn cleanup_target_budget_preserves_every_candidate_before_any_removal() {
        let temp = TestDir::new();
        let managed = temp.0.join("managed");
        let session = managed.join("sess-budget");
        for index in 0..=MAX_CLEANUP_TARGETS {
            write_cargo_target(&session.join(format!("project-{index}/target")));
        }
        let workspace = capture_cleanup_workspace(&managed, "sess-budget", &session).unwrap();
        let error = cleanup_build_artifacts(&workspace).await.unwrap_err();
        assert!(error.to_string().contains("observed Cargo targets"));
        for index in 0..=MAX_CLEANUP_TARGETS {
            assert!(
                session
                    .join(format!("project-{index}/target/debug/deps/libtest.rlib"))
                    .is_file()
            );
        }
        std::fs::remove_dir_all(session.join(format!("project-{MAX_CLEANUP_TARGETS}"))).unwrap();
        let removed = cleanup_build_artifacts(&workspace).await.unwrap();
        assert_eq!(removed.len(), MAX_CLEANUP_TARGETS);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn cleanup_scan_refuses_linked_ancestors_and_escaping_paths() {
        let temp = TestDir::new();
        let managed = temp.0.join("managed");
        let session = managed.join("sess-scan");
        write_cargo_target(&session.join("project/target"));
        let outside = temp.0.join("outside");
        write_cargo_target(&outside.join("target"));
        let workspace = capture_cleanup_workspace(&managed, "sess-scan", &session).unwrap();
        let access = workspace.access_root(&session);
        std::fs::rename(session.join("project"), session.join("original-project")).unwrap();
        std::os::unix::fs::symlink(&outside, session.join("project")).unwrap();
        for path in ["project", "project/target", "../outside", "/tmp"] {
            assert!(
                open_cleanup_scan_directory(&workspace, &access, Path::new(path))
                    .unwrap()
                    .is_none(),
                "{path}"
            );
        }
        let cleared = cleanup_build_artifacts_sync(&workspace).unwrap();
        assert_eq!(cleared, vec![session.join("original-project/target")]);
        assert!(outside.join("target/debug/deps/libtest.rlib").is_file());
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "requires explicitly isolated private mount namespace"]
    fn cleanup_scan_refuses_same_device_bind_mounts() {
        assert_eq!(
            std::env::var("COWBOY_TEST_CLEANUP_MOUNT_NAMESPACE").as_deref(),
            Ok("1")
        );
        let temp = TestDir::new();
        let managed = temp.0.join("managed");
        let session = managed.join("sess-mount-scan");
        write_cargo_target(&session.join("local/target"));
        let outside = temp.0.join("outside");
        write_cargo_target(&outside.join("target"));
        struct Mounts(Vec<PathBuf>);
        impl Drop for Mounts {
            fn drop(&mut self) {
                for path in self.0.iter().rev() {
                    assert!(
                        std::process::Command::new("umount")
                            .arg(path)
                            .status()
                            .unwrap()
                            .success()
                    );
                }
            }
        }
        let mut mounts = Mounts(Vec::new());
        for (source, destination) in [
            (outside.clone(), session.join("mounted-project")),
            (outside.join("target"), session.join("direct/target")),
        ] {
            std::fs::create_dir_all(&destination).unwrap();
            assert!(
                std::process::Command::new("mount")
                    .arg("--bind")
                    .arg(source)
                    .arg(&destination)
                    .status()
                    .unwrap()
                    .success()
            );
            mounts.0.push(destination);
        }
        let workspace = capture_cleanup_workspace(&managed, "sess-mount-scan", &session).unwrap();
        let access = workspace.access_root(&session);
        for relative in ["mounted-project", "mounted-project/target", "direct/target"] {
            assert_eq!(
                std::fs::metadata(session.join(relative)).unwrap().dev(),
                workspace.directory.metadata().unwrap().dev()
            );
            assert!(
                open_cleanup_scan_directory(&workspace, &access, Path::new(relative))
                    .unwrap()
                    .is_none()
            );
        }
        assert_eq!(
            cleanup_build_artifacts_sync(&workspace).unwrap(),
            vec![session.join("local/target")]
        );
        assert!(outside.join("target/debug/deps/libtest.rlib").is_file());
        assert_eq!(
            std::fs::read_dir(session.join("local/target"))
                .unwrap()
                .count(),
            0
        );
    }

    fn observed_cleanup_target(
        temp: &TestDir,
    ) -> (CleanupWorkspace, PathBuf, PathBuf, CleanupTarget) {
        let managed = temp.0.join("managed");
        let session = managed.join("sess-observed");
        let target = session.join("project/target");
        write_cargo_target(&target);
        let workspace = capture_cleanup_workspace(&managed, "sess-observed", &session).unwrap();
        let access_root = workspace.access_root(&session);
        let path = access_root.join("project/target");
        let directory = observe_cargo_target_directory(&path).unwrap().unwrap();
        (
            workspace,
            session,
            access_root,
            CleanupTarget { path, directory },
        )
    }

    #[test]
    fn observed_target_refuses_replacements_and_marker_withdrawal() {
        for case in [
            "new-marked",
            "new-unmarked",
            "link",
            "missing",
            "parent",
            "marker",
        ] {
            let temp = TestDir::new();
            let (workspace, session, access_root, observation) = observed_cleanup_target(&temp);
            let target = session.join("project/target");
            let original = if case == "parent" {
                std::fs::rename(session.join("project"), session.join("original-project")).unwrap();
                write_cargo_target(&target);
                session.join("original-project/target")
            } else if case == "marker" {
                std::fs::remove_file(target.join("CACHEDIR.TAG")).unwrap();
                target.clone()
            } else {
                let original = session.join("original-target");
                std::fs::rename(&target, &original).unwrap();
                match case {
                    "new-marked" => write_cargo_target(&target),
                    "new-unmarked" => {
                        std::fs::create_dir(&target).unwrap();
                        std::fs::write(target.join("source.txt"), "preserve").unwrap();
                    }
                    "link" => std::os::unix::fs::symlink(&original, &target).unwrap(),
                    "missing" => {}
                    _ => unreachable!(),
                }
                original
            };
            let error = remove_cleanup_targets(
                &workspace,
                &session,
                &access_root,
                vec![observation],
                &|_| Ok(()),
            )
            .unwrap_err();
            assert!(
                error.downcast_ref::<CleanupTargetChanged>().is_some(),
                "{case}: {error}"
            );
            assert!(original.join("debug/deps/libtest.rlib").is_file(), "{case}");
            if matches!(case, "new-marked" | "parent") {
                assert!(target.join("debug/deps/libtest.rlib").is_file(), "{case}");
            }
            if case == "new-unmarked" {
                assert!(target.join("source.txt").is_file());
            }
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn target_rename_after_child_check_never_redirects_contents_into_replacement() {
        let temp = TestDir::new();
        let (workspace, session, access_root, observation) = observed_cleanup_target(&temp);
        let target = session.join("project/target");
        let original = session.join("original-target");
        let replaced = std::sync::atomic::AtomicBool::new(false);
        let error = remove_cleanup_targets(
            &workspace,
            &session,
            &access_root,
            vec![observation],
            &|_| {
                if !replaced.swap(true, Ordering::SeqCst) {
                    std::fs::rename(&target, &original)?;
                    write_cargo_target(&target);
                    std::fs::write(target.join("source.txt"), "replacement source")?;
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.downcast_ref::<CleanupTargetChanged>().is_some());
        assert!(replaced.load(Ordering::SeqCst));
        assert!(target.join("debug/deps/libtest.rlib").is_file());
        assert_eq!(
            std::fs::read_to_string(target.join("source.txt")).unwrap(),
            "replacement source"
        );
    }

    #[tokio::test]
    async fn cleanup_preserves_targets_with_linked_or_oversized_markers() {
        for case in [
            "linked-info",
            "linked-tag",
            "oversized-tag",
            "invalid-tag",
            "tag-directory",
        ] {
            let temp = TestDir::new();
            let managed = temp.0.join("managed");
            let session = managed.join("sess-marker");
            let target = session.join("target");
            write_cargo_target(&target);
            match case {
                "linked-info" | "linked-tag" => {
                    let name = if case == "linked-info" {
                        ".rustc_info.json"
                    } else {
                        "CACHEDIR.TAG"
                    };
                    let external = temp.0.join("external-marker");
                    std::fs::rename(target.join(name), &external).unwrap();
                    std::os::unix::fs::symlink(&external, target.join(name)).unwrap();
                }
                "oversized-tag" => {
                    let mut content = format!("{CARGO_CACHE_TAG_SIGNATURE}\n").into_bytes();
                    content.resize(
                        usize::try_from(MAX_CARGO_CACHE_TAG_BYTES + 1).unwrap(),
                        b' ',
                    );
                    std::fs::write(target.join("CACHEDIR.TAG"), content).unwrap();
                }
                "invalid-tag" => std::fs::write(target.join("CACHEDIR.TAG"), [0xff]).unwrap(),
                "tag-directory" => {
                    std::fs::remove_file(target.join("CACHEDIR.TAG")).unwrap();
                    std::fs::create_dir(target.join("CACHEDIR.TAG")).unwrap();
                }
                _ => unreachable!(),
            }
            let observation = capture_cleanup_workspace(&managed, "sess-marker", &session).unwrap();
            assert!(
                cleanup_build_artifacts(&observation)
                    .await
                    .unwrap()
                    .is_empty(),
                "{case}"
            );
            assert!(target.join("debug/deps/libtest.rlib").is_file(), "{case}");
        }
    }

    #[test]
    fn cleanup_fifo_markers_finish_without_a_writer_and_preserve_artifacts() {
        for name in [".rustc_info.json", "CACHEDIR.TAG"] {
            let temp = TestDir::new();
            let managed = temp.0.join("managed");
            let session = managed.join("sess-fifo");
            let target = session.join("target");
            write_cargo_target(&target);
            std::fs::remove_file(target.join(name)).unwrap();
            rustix::fs::mkfifoat(
                rustix::fs::CWD,
                target.join(name),
                rustix::fs::Mode::from_raw_mode(0o600),
            )
            .unwrap();
            let observation = capture_cleanup_workspace(&managed, "sess-fifo", &session).unwrap();
            let (sender, receiver) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                sender
                    .send(cleanup_build_artifacts_sync(&observation))
                    .unwrap();
            });
            assert!(
                receiver
                    .recv_timeout(Duration::from_secs(2))
                    .expect("marker probing must not wait for a FIFO writer")
                    .unwrap()
                    .is_empty()
            );
            thread.join().unwrap();
            assert!(target.join("debug/deps/libtest.rlib").is_file());
        }
    }

    #[test]
    fn bounded_cache_tag_accepts_exact_size_limit() {
        let temp = TestDir::new();
        let target = temp.0.join("target");
        write_cargo_target(&target);
        let mut content = format!("{CARGO_CACHE_TAG_SIGNATURE}\n").into_bytes();
        content.resize(usize::try_from(MAX_CARGO_CACHE_TAG_BYTES).unwrap(), b' ');
        std::fs::write(target.join("CACHEDIR.TAG"), content).unwrap();
        assert!(observe_cargo_target_directory(&target).unwrap().is_some());
    }

    #[tokio::test]
    async fn cleanup_rejects_shared_or_mismatched_workspaces() {
        let temp = TestDir::new();
        let managed = temp.0.join("managed");
        let session = managed.join("sess-clean");
        let outside = temp.0.join("outside");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        write_cargo_target(&session.join("target"));

        assert!(capture_cleanup_workspace(&managed, "sess-clean", &outside).is_err());
        assert!(session.join("target").is_dir());
        assert!(capture_cleanup_workspace(&managed, "../escape", &session).is_err());
    }

    #[tokio::test]
    async fn prepares_fresh_remote_worktree_without_touching_dirty_source() {
        let temp = TestDir::new();
        let remote = temp.0.join("remote.git");
        let source = temp.0.join("source");
        let managed = temp.0.join("managed");
        git(&temp.0, &["init", "--bare", remote.to_str().unwrap()]);
        git(
            &temp.0,
            &["clone", remote.to_str().unwrap(), source.to_str().unwrap()],
        );
        git(&source, &["config", "user.name", "Cowboy Test"]);
        git(&source, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(source.join("value.txt"), "remote\n").unwrap();
        git(&source, &["add", "value.txt"]);
        git(&source, &["commit", "-m", "initial"]);
        git(&source, &["push", "-u", "origin", "HEAD:main"]);
        git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        std::fs::write(source.join("value.txt"), "unfinished\n").unwrap();

        let prepared = prepare(
            PrepareWorkspaceRequest {
                root: source.display().to_string(),
                session_id: "sess-1".to_owned(),
            },
            &managed,
        )
        .await
        .unwrap();
        assert!(prepared.isolated);
        assert!(prepared.created);
        assert_ne!(prepared.path, source.display().to_string());
        assert_eq!(
            git_output(
                Path::new(&prepared.path),
                ["symbolic-ref", "--short", "HEAD"]
            )
            .await
            .unwrap(),
            "cowboy/sess-1"
        );
        assert_eq!(
            git_output(
                &source,
                ["rev-parse", "refs/cowboy/session-bases/sess-1^{commit}"],
            )
            .await
            .unwrap(),
            prepared.revision.clone().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&prepared.path).join("value.txt")).unwrap(),
            "remote\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("value.txt")).unwrap(),
            "unfinished\n"
        );

        let unpublished = source.join("unpublished-subdir");
        std::fs::create_dir_all(&unpublished).unwrap();
        std::fs::write(unpublished.join("value.txt"), "not remote\n").unwrap();
        let failed = prepare(
            PrepareWorkspaceRequest {
                root: unpublished.display().to_string(),
                session_id: "sess-missing-subdir".to_owned(),
            },
            &managed,
        )
        .await;
        assert!(failed.is_err());
        assert!(!managed.join("sess-missing-subdir").exists());
        assert!(
            !git_command(
                &source,
                [
                    "show-ref",
                    "--verify",
                    "refs/cowboy/session-bases/sess-missing-subdir"
                ],
            )
            .await
            .unwrap()
            .status
            .success()
        );
        assert!(
            !git_command(
                &source,
                [
                    "show-ref",
                    "--verify",
                    "refs/heads/cowboy/sess-missing-subdir"
                ],
            )
            .await
            .unwrap()
            .status
            .success()
        );
        assert!(
            !git_output(&source, ["worktree", "list", "--porcelain"])
                .await
                .unwrap()
                .contains("sess-missing-subdir")
        );

        let selected = source.join("nested");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::write(selected.join("value.txt"), "nested\n").unwrap();
        git(&source, &["add", "nested/value.txt"]);
        git(&source, &["commit", "-m", "nested"]);
        git(&source, &["push", "origin", "HEAD:main"]);
        let nested = prepare(
            PrepareWorkspaceRequest {
                root: selected.display().to_string(),
                session_id: "sess-nested".to_owned(),
            },
            &managed,
        )
        .await
        .unwrap();
        assert!(nested.isolated);
        assert_eq!(
            std::fs::read_to_string(Path::new(&nested.path).join("value.txt")).unwrap(),
            "nested\n"
        );
        assert_eq!(Path::new(&nested.path).file_name(), selected.file_name());

        std::fs::write(Path::new(&prepared.path).join("local.txt"), "keep\n").unwrap();
        let reused = prepare(
            PrepareWorkspaceRequest {
                root: source.display().to_string(),
                session_id: "sess-1".to_owned(),
            },
            &managed,
        )
        .await
        .unwrap();
        assert!(!reused.created);
        assert_eq!(reused.path, prepared.path);
        assert!(Path::new(&reused.path).join("local.txt").is_file());

        let recoverable = prepare(
            PrepareWorkspaceRequest {
                root: source.display().to_string(),
                session_id: "sess-recoverable".to_owned(),
            },
            &managed,
        )
        .await
        .unwrap();
        std::fs::write(
            Path::new(&recoverable.path).join("unpublished.txt"),
            "keep this commit\n",
        )
        .unwrap();
        git(Path::new(&recoverable.path), &["add", "unpublished.txt"]);
        git(
            Path::new(&recoverable.path),
            &["commit", "-m", "unpublished session work"],
        );
        let unpublished_revision = git_output(Path::new(&recoverable.path), ["rev-parse", "HEAD"])
            .await
            .unwrap();
        std::fs::remove_dir_all(managed.join("sess-recoverable")).unwrap();
        let restore_request = || PrepareWorkspaceRequest {
            root: source.display().to_string(),
            session_id: "sess-recoverable".to_owned(),
        };
        let (restored, concurrent) = tokio::join!(
            prepare(restore_request(), &managed),
            prepare(restore_request(), &managed)
        );
        let restored = restored.unwrap();
        let concurrent = concurrent.unwrap();
        assert_eq!(restored.path, concurrent.path);
        assert_ne!(restored.created, concurrent.created);
        assert_eq!(
            restored.revision.as_deref(),
            Some(unpublished_revision.as_str())
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&restored.path).join("unpublished.txt")).unwrap(),
            "keep this commit\n"
        );

        let legacy = managed.join("sess-legacy");
        git(
            &source,
            &[
                "worktree",
                "add",
                "--detach",
                legacy.to_str().unwrap(),
                "HEAD",
            ],
        );
        std::fs::write(legacy.join("legacy-dirty.txt"), "preserve me\n").unwrap();
        let migrated = prepare(
            PrepareWorkspaceRequest {
                root: source.display().to_string(),
                session_id: "sess-legacy".to_owned(),
            },
            &managed,
        )
        .await
        .unwrap();
        assert!(!migrated.created);
        assert_eq!(
            git_output(
                Path::new(&migrated.path),
                ["symbolic-ref", "--short", "HEAD"]
            )
            .await
            .unwrap(),
            "cowboy/sess-legacy"
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&migrated.path).join("legacy-dirty.txt")).unwrap(),
            "preserve me\n"
        );

        let divergent = managed.join("sess-divergent");
        git(
            &source,
            &[
                "worktree",
                "add",
                "--detach",
                divergent.to_str().unwrap(),
                "HEAD",
            ],
        );
        let detached_revision = git_output(&divergent, ["rev-parse", "HEAD"]).await.unwrap();
        git(
            &source,
            &["update-ref", "refs/heads/cowboy/sess-divergent", "HEAD~1"],
        );
        let divergent_result = prepare(
            PrepareWorkspaceRequest {
                root: source.display().to_string(),
                session_id: "sess-divergent".to_owned(),
            },
            &managed,
        )
        .await;
        assert!(divergent_result.is_err());
        assert!(current_branch(&divergent).await.unwrap().is_none());
        assert_eq!(
            git_output(&divergent, ["rev-parse", "HEAD"]).await.unwrap(),
            detached_revision
        );
    }

    #[tokio::test]
    async fn isolates_local_commits_and_preserves_dirty_source_and_session_edits() {
        let temp = TestDir::new();
        let source = temp.0.join("source");
        let managed = temp.0.join("managed");
        git(&temp.0, &["init", source.to_str().unwrap()]);
        git(&source, &["config", "user.name", "Cowboy Test"]);
        git(&source, &["config", "user.email", "test@example.invalid"]);
        let request = || PrepareWorkspaceRequest {
            root: source.display().to_string(),
            session_id: "sess-local".to_owned(),
        };
        assert!(
            prepare(request(), &managed)
                .await
                .unwrap_err()
                .to_string()
                .contains("initial commit")
        );
        std::fs::write(source.join("value.txt"), "committed").unwrap();
        git(&source, &["add", "value.txt"]);
        git(&source, &["commit", "-m", "initial"]);
        std::fs::write(source.join("value.txt"), "unfinished").unwrap();
        let prepared = prepare(request(), &managed).await.unwrap();
        assert!(prepared.isolated && prepared.created);
        assert_eq!(prepared.upstream_ref, None);
        assert_eq!(
            prepared.revision,
            Some(git_output(&source, ["rev-parse", "HEAD"]).await.unwrap())
        );
        let value = Path::new(&prepared.path).join("value.txt");
        assert_eq!(std::fs::read_to_string(&value).unwrap(), "committed");
        assert_eq!(
            std::fs::read_to_string(source.join("value.txt")).unwrap(),
            "unfinished"
        );
        std::fs::write(&value, "session edit").unwrap();
        assert!(!prepare(request(), &managed).await.unwrap().created);
        assert_eq!(std::fs::read_to_string(&value).unwrap(), "session edit");

        // A configured but unavailable remote must never fall back to local HEAD.
        git(
            &source,
            &["remote", "add", "origin", "/nonexistent-cowboy-test-remote"],
        );
        let remote_request = || PrepareWorkspaceRequest {
            root: source.display().to_string(),
            session_id: "sess-remote".to_owned(),
        };
        assert!(prepare(remote_request(), &managed).await.is_err());
        git(&source, &["remote", "rename", "origin", "upstream"]);
        assert!(prepare(remote_request(), &managed).await.is_err());
        assert!(!managed.join("sess-remote").exists());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn refuses_linked_checkout_and_escaping_selection_without_mutating_existing_work() {
        let temp = TestDir::new();
        let source = temp.0.join("source");
        let managed = temp.0.join("managed");
        let outside = temp.0.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), "outside").unwrap();
        git(&temp.0, &["init", source.to_str().unwrap()]);
        git(&source, &["config", "user.name", "Cowboy Test"]);
        git(&source, &["config", "user.email", "test@example.invalid"]);
        std::os::unix::fs::symlink(&outside, source.join("nested")).unwrap();
        git(&source, &["add", "nested"]);
        git(&source, &["commit", "-m", "linked subdirectory"]);
        // The selected source subdirectory is a dirty real directory, while
        // the committed version to be isolated links outside the checkout.
        std::fs::remove_file(source.join("nested")).unwrap();
        std::fs::create_dir(source.join("nested")).unwrap();
        let request = |root: &Path, id: &str| PrepareWorkspaceRequest {
            root: root.display().to_string(),
            session_id: id.to_owned(),
        };
        let error = prepare(request(&source.join("nested"), "sess-new"), &managed)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("escapes new session worktree"));
        assert!(!managed.join("sess-new").exists());
        assert_eq!(
            std::fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "outside"
        );

        std::os::unix::fs::symlink(&source, managed.join("sess-link")).unwrap();
        let error = prepare(request(&source, "sess-link"), &managed)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not a real directory"));
        assert!(
            std::fs::symlink_metadata(managed.join("sess-link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );

        let existing = managed.join("sess-existing");
        git(
            &source,
            &[
                "worktree",
                "add",
                "--detach",
                existing.to_str().unwrap(),
                "HEAD",
            ],
        );
        std::fs::write(existing.join("dirty.txt"), "preserve").unwrap();
        let error = prepare(request(&source.join("nested"), "sess-existing"), &managed)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("escapes existing session worktree")
        );
        assert!(current_branch(&existing).await.unwrap().is_none());
        assert!(
            !git_ref_exists(&source, "refs/heads/cowboy/sess-existing")
                .await
                .unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(existing.join("dirty.txt")).unwrap(),
            "preserve"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "outside"
        );
        // Internal links remain compatible; only an escape is refused.
        std::fs::remove_file(existing.join("nested")).unwrap();
        std::fs::create_dir(existing.join("internal")).unwrap();
        std::os::unix::fs::symlink("internal", existing.join("nested")).unwrap();
        let reused = prepare(request(&source.join("nested"), "sess-existing"), &managed)
            .await
            .unwrap();
        assert_eq!(Path::new(&reused.path), existing.join("internal"));
        assert!(!reused.created);
    }

    #[tokio::test]
    async fn leaves_non_git_workspace_shared() {
        let temp = TestDir::new();
        let prepared = prepare(
            PrepareWorkspaceRequest {
                root: temp.0.display().to_string(),
                session_id: "sess-2".to_owned(),
            },
            &temp.0.join("managed"),
        )
        .await
        .unwrap();
        assert!(!prepared.isolated);
        assert_eq!(prepared.path, temp.0.display().to_string());
    }

    #[tokio::test]
    async fn corrupted_git_workspace_fails_closed() {
        let temp = TestDir::new();
        std::fs::write(temp.0.join(".git"), "not a gitdir\n").unwrap();
        let result = prepare(
            PrepareWorkspaceRequest {
                root: temp.0.display().to_string(),
                session_id: "sess-3".to_owned(),
            },
            &temp.0.join("managed"),
        )
        .await;
        assert!(result.is_err());
    }
}
