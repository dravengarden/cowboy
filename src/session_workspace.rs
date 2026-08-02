//! Machine-local preparation of isolated Git worktrees for Cowboy sessions.

use std::ffi::OsStr;
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

    if tokio::fs::try_exists(&destination).await? {
        return reuse_existing(&repository, &source, &relative_path, &destination).await;
    }

    let remote_head = git_output(&repository, ["ls-remote", "--symref", "origin", "HEAD"])
        .await
        .context("resolving origin default branch")?;
    let head_ref = parse_remote_head(&remote_head)?;
    let branch = head_ref
        .strip_prefix("refs/heads/")
        .context("origin HEAD is not a branch")?;
    git_output(&repository, ["check-ref-format", "--branch", branch])
        .await
        .context("validating origin default branch")?;
    let base_ref = format!("refs/cowboy/session-bases/{}", request.session_id);
    let refspec = format!("+{head_ref}:{base_ref}");
    git_output(&repository, ["fetch", "origin", refspec.as_str()])
        .await
        .context("fetching origin default branch for isolated session")?;
    let revision_spec = format!("{base_ref}^{{commit}}");
    let revision = git_output(&repository, ["rev-parse", revision_spec.as_str()])
        .await
        .context("resolving fetched session base")?;

    tokio::fs::create_dir_all(worktree_root)
        .await
        .with_context(|| format!("creating worktree root {}", worktree_root.display()))?;
    git_output(
        &repository,
        [
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("--detach"),
            destination.as_os_str(),
            OsStr::new(&revision),
        ],
    )
    .await
    .with_context(|| {
        format!(
            "creating isolated session worktree {}",
            destination.display()
        )
    })?;
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
    Ok(PreparedWorkspace {
        path: path.display().to_string(),
        source_path: source.display().to_string(),
        revision: Some(revision),
        upstream_ref: Some(head_ref),
        isolated: true,
        created: true,
    })
}

async fn reuse_existing(
    repository: &Path,
    source: &Path,
    relative_path: &Path,
    destination: &Path,
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
