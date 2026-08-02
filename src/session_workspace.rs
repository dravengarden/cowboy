//! Machine-local preparation of isolated Git worktrees for Cowboy sessions.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

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
/// Git-backed roots fail closed when their remote default branch cannot be
/// fetched. Non-Git roots remain shared because Git cannot isolate them.
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
    let checkout = destination
        .canonicalize()
        .with_context(|| format!("canonicalizing existing worktree {}", destination.display()))?;
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
    let path = checkout
        .join(relative_path)
        .canonicalize()
        .with_context(|| {
            format!(
                "canonicalizing selected workspace {} in existing worktree {}",
                relative_path.display(),
                checkout.display()
            )
        })?;
    Ok(PreparedWorkspace {
        path: path.display().to_string(),
        source_path: source.display().to_string(),
        revision: Some(revision),
        upstream_ref: None,
        isolated: true,
        created: false,
    })
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
