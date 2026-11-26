//! Fork preparation: capture a source worktree's committed and uncommitted
//! state without writing it, then reproduce that state in a new worktree.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::process::Command;

use super::{GIT_TIMEOUT, git_command, git_output};

/// Untracked files a fork copies are bounded: a large untracked tree is build
/// output that belongs in `.gitignore`, not branch state.
const FORK_UNTRACKED_LIMIT_BYTES: u64 = 512 * 1024 * 1024;

/// The committed and uncommitted state of a fork's source worktree.
pub(super) struct ForkBase {
    worktree: PathBuf,
    pub(super) revision: String,
    pub(super) include_uncommitted: bool,
    /// `git stash create` commit of staged and unstaged tracked changes.
    stash: Option<String>,
    /// Intent-to-add paths (`git add -N`) that exist in the source worktree.
    /// `git stash create` cannot record them, so they are copied like
    /// untracked files and marked intent-to-add again in the fork.
    intent_to_add: Vec<PathBuf>,
    /// Pre-copy content-aware SHA-256 fingerprint of HEAD, index entries,
    /// modes, symlink targets, and dirty/untracked file contents.
    pre_fingerprint: Option<[u8; 32]>,
}

pub(super) async fn resolve_fork_base(
    repository: &Path,
    request: &super::PrepareWorkspaceRequest,
    branch_existed: bool,
) -> Result<Option<ForkBase>> {
    match (&request.base_worktree, branch_existed) {
        (Some(base), false) => Ok(Some(
            fork_base(
                repository,
                base,
                request.include_uncommitted.unwrap_or(true),
            )
            .await?,
        )),
        _ => Ok(None),
    }
}

/// Read the source worktree without writing it. `git stash create` stores a
/// dangling commit and leaves the worktree, index and stash list unchanged.
pub(super) async fn fork_base(
    repository: &Path,
    worktree: &Path,
    include_uncommitted: bool,
) -> Result<ForkBase> {
    let worktree = worktree
        .canonicalize()
        .with_context(|| format!("fork source worktree {} is unavailable", worktree.display()))?;
    let common = |path: PathBuf| async move {
        let dir = git_output(
            &path,
            ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await?;
        PathBuf::from(dir)
            .canonicalize()
            .context("canonicalizing Git common directory")
    };
    if common(worktree.clone()).await? != common(repository.to_path_buf()).await? {
        bail!(
            "fork source worktree {} belongs to another repository",
            worktree.display()
        );
    }
    let revision = git_output(&worktree, ["rev-parse", "--verify", "HEAD^{commit}"])
        .await
        .context("resolving fork source HEAD")?;
    if !include_uncommitted {
        return Ok(ForkBase {
            worktree,
            revision,
            include_uncommitted: false,
            stash: None,
            intent_to_add: Vec::new(),
            pre_fingerprint: None,
        });
    }
    let pre_fingerprint = worktree_content_fingerprint(&worktree, &revision).await?;
    let intent_to_add = intent_to_add_paths(&worktree).await?;
    let stash = if intent_to_add.is_empty() {
        git_output(&worktree, ["stash", "create"]).await
    } else {
        stash_without(&worktree, &intent_to_add).await
    }
    .context("capturing fork source changes")?;
    Ok(ForkBase {
        worktree,
        revision,
        include_uncommitted: true,
        stash: (!stash.is_empty()).then_some(stash),
        intent_to_add,
        pre_fingerprint: Some(pre_fingerprint),
    })
}

/// Paths recorded with `git add -N` whose file is present. Only such an entry
/// can be "added" between the index and the worktree; a deleted one does not
/// prevent `git stash create`.
async fn intent_to_add_paths(worktree: &Path) -> Result<Vec<PathBuf>> {
    let listed = git_command(
        worktree,
        [
            "diff",
            "--name-only",
            "--diff-filter=A",
            "--no-renames",
            "-z",
        ],
    )
    .await?;
    if !listed.status.success() {
        bail!(
            "listing fork source intent-to-add files: {}",
            String::from_utf8_lossy(&listed.stderr).trim()
        );
    }
    Ok(nul_separated(&listed.stdout))
}

/// `git stash create` against a private copy of the source index with the
/// intent-to-add entries removed. The source index itself is never written.
async fn stash_without(worktree: &Path, intent_to_add: &[PathBuf]) -> Result<String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let index = PathBuf::from(
        git_output(
            worktree,
            ["rev-parse", "--path-format=absolute", "--git-path", "index"],
        )
        .await?,
    );
    let scratch = TemporaryIndex(std::env::temp_dir().join(format!(
        "cowboy-fork-index-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    tokio::fs::copy(&index, &scratch.0)
        .await
        .with_context(|| format!("copying fork source index {}", index.display()))?;
    let mut remove = vec![
        OsStr::new("--literal-pathspecs"),
        OsStr::new("rm"),
        OsStr::new("--cached"),
        OsStr::new("--quiet"),
        OsStr::new("--"),
    ];
    remove.extend(intent_to_add.iter().map(|path| path.as_os_str()));
    git_with_index(worktree, &scratch.0, remove).await?;
    git_with_index(worktree, &scratch.0, ["stash", "create"]).await
}

/// Removes a private index copy however its stash attempt ends.
struct TemporaryIndex(PathBuf);

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn git_with_index<I, S>(worktree: &Path, index: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(worktree)
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .with_context(|| format!("git timed out in {}", worktree.display()))?
        .with_context(|| format!("starting git in {}", worktree.display()))?;
    if !output.status.success() {
        bail!(
            "git failed in {}: {}",
            worktree.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn nul_separated(bytes: &[u8]) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(OsStr::from_bytes(path)))
        .collect()
}

/// Parse `-z` porcelain v1 paths (including rename/copy entries) for hashing.
fn parse_porcelain_paths(bytes: &[u8]) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;
    let mut paths = Vec::new();
    let mut parts = bytes.split(|b| *b == 0);
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let xy = &entry[..2];
        let path = PathBuf::from(OsStr::from_bytes(&entry[3..]));
        paths.push(path);
        // Renames/copies in porcelain v1 `-z` carry the original path next.
        if (xy.contains(&b'R') || xy.contains(&b'C'))
            && let Some(orig) = parts.next()
            && !orig.is_empty()
        {
            paths.push(PathBuf::from(OsStr::from_bytes(orig)));
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Content-aware SHA-256 fingerprint of `HEAD`, staged index entries, file
/// modes, symlink targets, and full bytes of every dirty/untracked file.
async fn worktree_content_fingerprint(worktree: &Path, head: &str) -> Result<[u8; 32]> {
    let status = git_command(worktree, ["status", "--porcelain=v1", "-z", "-uall"]).await?;
    if !status.status.success() {
        bail!(
            "reading fork source status: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        );
    }
    let stage = git_command(worktree, ["ls-files", "--stage", "-z"]).await?;
    if !stage.status.success() {
        bail!(
            "reading fork source index: {}",
            String::from_utf8_lossy(&stage.stderr).trim()
        );
    }
    let root = worktree.to_path_buf();
    let head = head.to_owned();
    let status_bytes = status.stdout;
    let stage_bytes = stage.stdout;
    tokio::task::spawn_blocking(move || -> Result<[u8; 32]> {
        use sha2::{Digest as _, Sha256};
        use std::io::Read as _;
        use std::os::unix::ffi::OsStrExt as _;
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
        let mut hasher = Sha256::new();
        hasher.update(b"HEAD\0");
        hasher.update(head.as_bytes());
        hasher.update(b"\0STAGE\0");
        hasher.update((stage_bytes.len() as u64).to_le_bytes());
        hasher.update(&stage_bytes);
        hasher.update(b"\0STATUS\0");
        hasher.update((status_bytes.len() as u64).to_le_bytes());
        hasher.update(&status_bytes);
        let mut buf = [0u8; 16_384];
        let mut total_bytes = 0_u64;
        for rel in parse_porcelain_paths(&status_bytes) {
            hasher.update(b"\0FILE\0");
            hasher.update(rel.as_os_str().as_bytes());
            hasher.update(b"\0");
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                bail!("unexpected path in fork source status: {}", rel.display());
            }
            // Confine traversal identically to copy_files before opening anything.
            real_ancestors(&root, &rel, false)?;
            let full = root.join(&rel);
            match std::fs::symlink_metadata(&full) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    hasher.update(b"MISSING");
                }
                Err(err) => {
                    return Err(err).with_context(|| format!("fingerprinting {}", full.display()));
                }
                Ok(meta) => {
                    hasher.update(meta.mode().to_le_bytes());
                    let ft = meta.file_type();
                    if ft.is_symlink() {
                        hasher.update(b"LINK\0");
                        let target = std::fs::read_link(&full)?;
                        hasher.update(target.as_os_str().as_bytes());
                    } else if ft.is_file() {
                        total_bytes = total_bytes.saturating_add(meta.len());
                        if total_bytes > FORK_UNTRACKED_LIMIT_BYTES {
                            bail!(
                                "the source worktree has more than {} MiB of uncommitted files; ignore build output or commit it before forking",
                                FORK_UNTRACKED_LIMIT_BYTES / 1024 / 1024
                            );
                        }
                        hasher.update(b"REG\0");
                        hasher.update(meta.len().to_le_bytes());
                        let mut f = std::fs::OpenOptions::new()
                            .read(true)
                            .custom_flags(libc::O_NOFOLLOW)
                            .open(&full)?;
                        let mut read_this = 0_u64;
                        loop {
                            let n = f.read(&mut buf)?;
                            if n == 0 {
                                break;
                            }
                            read_this = read_this.saturating_add(n as u64);
                            if total_bytes
                                .saturating_add(read_this.saturating_sub(meta.len()))
                                > FORK_UNTRACKED_LIMIT_BYTES
                            {
                                bail!(
                                    "source file {} grew past the fork copy budget during fingerprinting",
                                    rel.display()
                                );
                            }
                            hasher.update(&buf[..n]);
                        }
                    } else if ft.is_dir() {
                        hasher.update(b"DIR");
                    } else {
                        hasher.update(b"OTHER");
                    }
                }
            }
        }
        Ok(hasher.finalize().into())
    })
    .await
    .context("joining fork source fingerprint")?
}

pub(super) async fn copy_fork_state(base: &ForkBase, destination: &Path) -> Result<()> {
    if !base.include_uncommitted {
        return Ok(());
    }
    if let Some(stash) = &base.stash {
        git_output(destination, ["stash", "apply", "--index", stash.as_str()])
            .await
            .context("applying fork source changes")?;
    }
    let listed = git_command(
        &base.worktree,
        ["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await?;
    if !listed.status.success() {
        bail!(
            "listing fork source untracked files: {}",
            String::from_utf8_lossy(&listed.stderr).trim()
        );
    }
    let mut paths = nul_separated(&listed.stdout);
    paths.extend(base.intent_to_add.iter().cloned());
    let source = base.worktree.clone();
    let target = destination.to_path_buf();
    tokio::task::spawn_blocking(move || copy_files(&source, &target, &paths))
        .await
        .context("joining fork file copy")??;
    if !base.intent_to_add.is_empty() {
        let mut add = vec![
            OsStr::new("--literal-pathspecs"),
            OsStr::new("add"),
            OsStr::new("--intent-to-add"),
            OsStr::new("--"),
        ];
        add.extend(base.intent_to_add.iter().map(|path| path.as_os_str()));
        git_output(destination, add)
            .await
            .context("marking fork intent-to-add files")?;
    }
    if let Some(pre) = &base.pre_fingerprint {
        let post_head = git_output(&base.worktree, ["rev-parse", "--verify", "HEAD^{commit}"])
            .await
            .context("verifying fork source HEAD stability")?;
        let post_fp = worktree_content_fingerprint(&base.worktree, &post_head).await?;
        if post_head != base.revision || post_fp != *pre {
            bail!(
                "source worktree changed while copying uncommitted files; retry once the source is idle or fork from committed HEAD"
            );
        }
    }
    Ok(())
}

/// Copy source-relative files that Git does not reproduce into `destination`.
fn copy_files(source: &Path, destination: &Path, paths: &[PathBuf]) -> Result<()> {
    let mut copied = 0_u64;
    for relative in paths {
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            bail!("unexpected untracked path {}", relative.display());
        }
        let from = source.join(relative);
        let to = destination.join(relative);
        // Neither tree may route a path through a symlinked directory: a
        // copy must stay inside the destination and read only the source.
        real_ancestors(source, relative, false)?;
        real_ancestors(destination, relative, true)?;
        let metadata = std::fs::symlink_metadata(&from)
            .with_context(|| format!("reading {}", from.display()))?;
        if metadata.file_type().is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(&from)?, &to)
                .with_context(|| format!("copying symlink {}", relative.display()))?;
        } else if metadata.is_file() {
            copied += metadata.len();
            if copied > FORK_UNTRACKED_LIMIT_BYTES {
                bail!(
                    "the source worktree has more than {} MiB of untracked files; ignore build output or commit it before forking",
                    FORK_UNTRACKED_LIMIT_BYTES / 1024 / 1024
                );
            }
            copy_no_follow(&from, &to, metadata.permissions())
                .with_context(|| format!("copying {}", relative.display()))?;
        }
    }
    Ok(())
}

/// Require every directory above `relative` in `root` to be a real directory,
/// creating missing ones one level at a time when `create` is set.
pub(super) fn real_ancestors(root: &Path, relative: &Path, create: bool) -> Result<()> {
    let mut current = root.to_path_buf();
    let Some(parent) = relative.parent() else {
        return Ok(());
    };
    for part in parent.components() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => bail!("{} is not a real directory", current.display()),
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current)
                    .with_context(|| format!("creating {}", current.display()))?;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", current.display()));
            }
        }
    }
    Ok(())
}

/// Copy one regular file without following a symlink at either end.
fn copy_no_follow(from: &Path, to: &Path, permissions: std::fs::Permissions) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut input = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(from)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(to)?;
    std::io::copy(&mut input, &mut output)?;
    output.set_permissions(permissions)?;
    Ok(())
}

#[cfg(test)]
mod fingerprint_tests {
    use super::*;

    fn git(repo: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn content_fingerprint_detects_in_place_rewrite_of_dirty_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q"]);
        git(repo, &["config", "user.email", "test@example.com"]);
        git(repo, &["config", "user.name", "Test"]);
        std::fs::write(repo.join("a.txt"), b"v1\n").unwrap();
        git(repo, &["add", "a.txt"]);
        git(repo, &["commit", "-q", "-m", "init"]);
        let head = String::from_utf8(
            std::process::Command::new("git")
                .current_dir(repo)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();

        // Modify tracked file once and record fingerprint.
        std::fs::write(repo.join("a.txt"), b"dirty-alpha\n").unwrap();
        let fp1 = worktree_content_fingerprint(repo, &head).await.unwrap();

        // Rewrite the exact same dirty file with different content of the SAME length.
        std::fs::write(repo.join("a.txt"), b"dirty-bravo\n").unwrap();
        let fp2 = worktree_content_fingerprint(repo, &head).await.unwrap();
        assert_ne!(
            fp1, fp2,
            "rewriting an already-dirty tracked file must change the content fingerprint"
        );

        // Add untracked file and rewrite it too.
        std::fs::write(repo.join("u.txt"), b"untracked-1").unwrap();
        let fp3 = worktree_content_fingerprint(repo, &head).await.unwrap();
        assert_ne!(fp2, fp3);
        std::fs::write(repo.join("u.txt"), b"untracked-2").unwrap();
        let fp4 = worktree_content_fingerprint(repo, &head).await.unwrap();
        assert_ne!(fp3, fp4);
    }

    #[tokio::test]
    async fn content_fingerprint_stays_confined_when_symlink_points_outside() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), b"top-secret").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q"]);
        git(repo, &["config", "user.email", "test@example.com"]);
        git(repo, &["config", "user.name", "Test"]);
        std::fs::write(repo.join("a.txt"), b"ok\n").unwrap();
        git(repo, &["add", "a.txt"]);
        git(repo, &["commit", "-q", "-m", "init"]);
        let head = String::from_utf8(
            std::process::Command::new("git")
                .current_dir(repo)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();

        // Untracked symlink to outside file must hash link target, never read outside contents.
        std::os::unix::fs::symlink(outside.path().join("secret.txt"), repo.join("link.txt"))
            .unwrap();
        let fp1 = worktree_content_fingerprint(repo, &head).await.unwrap();
        std::fs::write(outside.path().join("secret.txt"), b"changed-outside").unwrap();
        let fp2 = worktree_content_fingerprint(repo, &head).await.unwrap();
        assert_eq!(
            fp1, fp2,
            "mutating outside symlink target must not affect confined worktree fingerprint"
        );
    }
}
