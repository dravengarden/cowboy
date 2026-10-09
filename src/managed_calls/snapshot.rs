//! Target-owned immutable input for managed child rounds.
//!
//! A round captures the parent's `HEAD`, its index and its working tree
//! (tracked plus non-ignored untracked files, plus explicitly named context
//! files) as two synthetic commits in a private repository whose object store
//! borrows the parent's objects through `alternates`. The parent worktree and
//! its repository metadata are only read: every new object, index and ref is
//! written to the child's own repository. The capture is performed twice and
//! refused when the two passes differ, so a reviewer never reads a mix of two
//! states. The child's workspace path is stable across rounds; a continued
//! conversation is refreshed in place only while its worker is stopped.

use std::ffi::OsStr;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

use super::protocol::{ChildPrepared, ChildRound};
#[cfg(test)]
use super::round::read_round;
use super::round::{MAX_ROUND_BYTES, RoundMarker, read_private};
use crate::execution_environment::ManagedChildV1;

const GIT_TIMEOUT: Duration = Duration::from_secs(120);

static LOCKS: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

struct ChildLock(String);

impl ChildLock {
    fn acquire(child: &str) -> Option<Self> {
        LOCKS
            .lock()
            .insert(child.to_owned())
            .then(|| Self(child.to_owned()))
    }
}

impl Drop for ChildLock {
    fn drop(&mut self) {
        LOCKS.lock().remove(&self.0);
    }
}

fn private_directory(path: &Path) -> std::io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let partial = path.with_extension("partial");
    let _ = std::fs::remove_file(&partial);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&partial)?;
    std::io::Write::write_all(&mut &file, bytes)?;
    file.sync_all()?;
    std::fs::rename(&partial, path)?;
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

struct Git {
    home: Option<std::ffi::OsString>,
    path: Option<std::ffi::OsString>,
}

impl Git {
    fn new() -> Self {
        Self {
            home: std::env::var_os("HOME"),
            path: std::env::var_os("PATH"),
        }
    }

    /// Hooks, fsmonitor daemons, pagers and credential prompts are disabled
    /// for every invocation. Only explicit environment is passed through.
    fn command(&self, envs: &[(&str, &OsStr)]) -> Command {
        let mut command = Command::new("git");
        command.env_clear();
        if let Some(path) = &self.path {
            command.env("PATH", path);
        }
        if let Some(home) = &self.home {
            command.env("HOME", home);
        }
        command
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Cowboy")
            .env("GIT_AUTHOR_EMAIL", "cowboy@localhost")
            .env("GIT_COMMITTER_NAME", "Cowboy")
            .env("GIT_COMMITTER_EMAIL", "cowboy@localhost")
            .env("GIT_AUTHOR_DATE", "@0 +0000")
            .env("GIT_COMMITTER_DATE", "@0 +0000")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.pager=cat",
                "-c",
                "advice.detachedHead=false",
            ])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        for (name, value) in envs {
            command.env(name, value);
        }
        command
    }

    async fn run(
        &self,
        envs: &[(&str, &OsStr)],
        args: &[&OsStr],
        code: &'static str,
    ) -> Result<String, &'static str> {
        let mut command = self.command(envs);
        command.args(args);
        let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
            .await
            .map_err(|_| "snapshot_timeout")?
            .map_err(|_| "git_unavailable")?;
        if !output.status.success() {
            return Err(code);
        }
        String::from_utf8(output.stdout)
            .map(|text| text.trim_end_matches('\n').to_owned())
            .map_err(|_| code)
    }
}

fn os(value: &str) -> &OsStr {
    OsStr::new(value)
}

fn object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The source must be an existing, canonical top-level Git work tree inside
/// one of this Machine's worktree roots or registered workspaces.
fn source_allowed(
    source: &Path,
    roots: &crate::session_workspace::WorktreeRoots,
    workspaces: &[PathBuf],
) -> bool {
    std::fs::canonicalize(source).is_ok_and(|canonical| canonical == source)
        && (roots
            .all()
            .any(|root| std::fs::canonicalize(root).is_ok_and(|root| source.starts_with(root)))
            || workspaces.iter().any(|workspace| {
                std::fs::canonicalize(workspace).is_ok_and(|root| source.starts_with(root))
            }))
}

async fn common_git_dir(git: &Git, path: &Path) -> Option<PathBuf> {
    let common = git
        .run(
            &[],
            &[
                os("-C"),
                path.as_os_str(),
                os("rev-parse"),
                os("--path-format=absolute"),
                os("--git-common-dir"),
            ],
            "not_a_repository",
        )
        .await
        .ok()?;
    std::fs::canonicalize(common).ok()
}

/// A source outside the Machine's roots is accepted only as a work tree of a
/// repository the Machine registered: its common Git directory must be the
/// same as one registered workspace's.
async fn source_permitted(
    git: &Git,
    source: &Path,
    roots: &crate::session_workspace::WorktreeRoots,
    workspaces: &[PathBuf],
) -> bool {
    if source_allowed(source, roots, workspaces) {
        return true;
    }
    if std::fs::canonicalize(source).ok().as_deref() != Some(source) {
        return false;
    }
    let Some(common) = common_git_dir(git, source).await else {
        return false;
    };
    for workspace in workspaces {
        if common_git_dir(git, workspace).await.as_ref() == Some(&common) {
            return true;
        }
    }
    false
}

struct Capture {
    head: String,
    index_tree: String,
    worktree_tree: String,
}

/// One capture pass into `repo`, reading `source` through a copied index.
async fn capture(
    git: &Git,
    source: &Path,
    repo: &Path,
    index_path: &Path,
    scratch: &Path,
    files: &[String],
) -> Result<Capture, &'static str> {
    let source_os = source.as_os_str();
    let head = git
        .run(
            &[],
            &[
                os("-C"),
                source_os,
                os("rev-parse"),
                os("--verify"),
                os("HEAD^{commit}"),
            ],
            "unborn_head",
        )
        .await?;
    let staged = scratch.join("staged.index");
    let working = scratch.join("working.index");
    for copy in [&staged, &working] {
        let _ = std::fs::remove_file(copy);
        std::fs::copy(index_path, copy).map_err(|_| "index_unavailable")?;
    }
    let repo_os = repo.as_os_str();
    let index_tree = git
        .run(
            &[
                ("GIT_DIR", repo_os),
                ("GIT_WORK_TREE", source_os),
                ("GIT_INDEX_FILE", staged.as_os_str()),
            ],
            &[os("write-tree")],
            "index_unsupported",
        )
        .await?;
    let working_env = [
        ("GIT_DIR", repo_os),
        ("GIT_WORK_TREE", source_os),
        ("GIT_INDEX_FILE", working.as_os_str()),
    ];
    git.run(
        &working_env,
        &[os("add"), os("--all"), os("--"), os(".")],
        "worktree_unreadable",
    )
    .await?;
    if !files.is_empty() {
        let mut args = vec![os("add"), os("--force"), os("--")];
        args.extend(files.iter().map(|file| os(file)));
        git.run(&working_env, &args, "context_unreadable").await?;
    }
    let worktree_tree = git
        .run(&working_env, &[os("write-tree")], "worktree_unreadable")
        .await?;
    if ![&head, &index_tree, &worktree_tree]
        .into_iter()
        .all(|id| object_id(id))
    {
        return Err("snapshot_invalid");
    }
    Ok(Capture {
        head,
        index_tree,
        worktree_tree,
    })
}

/// Refuse inputs this snapshot cannot represent faithfully.
async fn refuse_unsupported(
    git: &Git,
    source: &Path,
    repo: &Path,
    tree: &str,
) -> Result<(), &'static str> {
    let source_os = source.as_os_str();
    // A clean/smudge filter (Git LFS, git-crypt) means repository bytes differ
    // from what a reviewer should read; do not guess.
    let filters = git
        .run(
            &[],
            &[
                os("-C"),
                source_os,
                os("config"),
                os("--get-regexp"),
                os("^filter\\."),
            ],
            "filter_check_failed",
        )
        .await;
    if filters.is_ok_and(|text| !text.is_empty()) {
        return Err("filter_unsupported");
    }
    let listing = git
        .run(
            &[("GIT_DIR", repo.as_os_str())],
            &[os("ls-tree"), os("-r"), os("--full-tree"), os(tree)],
            "snapshot_invalid",
        )
        .await?;
    for line in listing.lines() {
        let Some((meta, path)) = line.split_once('\t') else {
            return Err("snapshot_invalid");
        };
        let mut fields = meta.split(' ');
        let mode = fields.next().unwrap_or_default();
        let object = fields.nth(1).unwrap_or_default();
        match mode {
            "160000" => return Err("submodule_unsupported"),
            "120000" => {
                let target = git
                    .run(
                        &[("GIT_DIR", repo.as_os_str())],
                        &[os("cat-file"), os("blob"), os(object)],
                        "snapshot_invalid",
                    )
                    .await?;
                if !symlink_contained(path, &target) {
                    return Err("symlink_unsupported");
                }
            }
            _ => {}
        }
        if path.ends_with(".gitattributes") {
            let attributes = git
                .run(
                    &[("GIT_DIR", repo.as_os_str())],
                    &[os("cat-file"), os("blob"), os(object)],
                    "snapshot_invalid",
                )
                .await?;
            if attributes.contains("filter=") {
                return Err("filter_unsupported");
            }
        }
    }
    Ok(())
}

/// A relative link that stays inside the snapshot after lexical resolution.
fn symlink_contained(path: &str, target: &str) -> bool {
    if target.is_empty() || target.starts_with('/') || target.contains('\0') {
        return false;
    }
    let mut depth: Vec<&str> = path.split('/').collect();
    depth.pop();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if depth.pop().is_none() {
                    return false;
                }
            }
            part => depth.push(part),
        }
    }
    true
}

fn input_revision(capture: &Capture, files: &[String]) -> String {
    super::canonical_digest(&serde_json::json!({
        "schema": 1,
        "head": capture.head,
        "index_tree": capture.index_tree,
        "worktree_tree": capture.worktree_tree,
        "files": files,
    }))
}

/// Prepare one round. Repeating the same call id observes the stored receipt
/// instead of capturing again, so a lost reply cannot change the input.
pub async fn prepare(
    managed: &Path,
    machine_id: &str,
    round: &ChildRound,
    roots: &crate::session_workspace::WorktreeRoots,
    workspaces: &[PathBuf],
) -> Result<ChildPrepared, &'static str> {
    if !round.validate() {
        return Err("invalid_request");
    }
    let source = PathBuf::from(&round.source_cwd);
    let git = Git::new();
    if !source_permitted(&git, &source, roots, workspaces).await {
        return Err("source_unavailable");
    }
    private_directory(managed).map_err(|_| "preparation_failed")?;
    let managed = std::fs::canonicalize(managed).map_err(|_| "preparation_failed")?;
    let directory = managed.join(&round.child_session_id);
    let _lock = ChildLock::acquire(&directory.display().to_string()).ok_or("child_busy")?;
    private_directory(&directory).map_err(|_| "preparation_failed")?;
    let workspace = directory.join("workspace");
    let identity = ManagedChildV1 {
        schema: 1,
        phase: "managed_child".into(),
        session_id: round.child_session_id.clone(),
        parent_session_id: round.parent_session_id.clone(),
        machine_id: machine_id.to_owned(),
        workspace_id: round.workspace_id.clone(),
        cwd: workspace.display().to_string(),
        profile: round.profile,
    };
    identity.validate().map_err(|_| "invalid_request")?;
    let marker = directory.join("snapshot.json");
    match read_private(&marker, 16 * 1024) {
        Some(bytes) => {
            if serde_json::from_slice::<ManagedChildV1>(&bytes)
                .ok()
                .as_ref()
                != Some(&identity)
            {
                return Err("identity_mismatch");
            }
        }
        None if marker.exists() => return Err("identity_mismatch"),
        None => {
            let bytes = serde_json::to_vec(&identity).map_err(|_| "preparation_failed")?;
            write_private(&marker, &bytes).map_err(|_| "preparation_failed")?;
        }
    }
    let round_path = directory.join("round.json");
    if let Some(existing) = read_private(&round_path, MAX_ROUND_BYTES)
        .and_then(|bytes| serde_json::from_slice::<RoundMarker>(&bytes).ok())
        && existing.call_id == round.call_id
        && workspace.is_dir()
    {
        if existing.output_schema != round.output_schema {
            return Err("identity_mismatch");
        }
        return Ok(ChildPrepared {
            cwd: workspace.display().to_string(),
            input_revision: existing.input_revision,
            head: existing.head,
            index_tree: existing.index_tree,
            worktree_tree: existing.worktree_tree,
        });
    }
    let top = git
        .run(
            &[],
            &[
                os("-C"),
                source.as_os_str(),
                os("rev-parse"),
                os("--show-toplevel"),
            ],
            "not_a_repository",
        )
        .await?;
    if Path::new(&top) != source {
        return Err("subdirectory_unsupported");
    }
    let resolve = |name: &'static str| {
        let git = &git;
        let source = source.clone();
        async move {
            let path = git
                .run(
                    &[],
                    &[
                        os("-C"),
                        source.as_os_str(),
                        os("rev-parse"),
                        os("--path-format=absolute"),
                        os("--git-path"),
                        os(name),
                    ],
                    "not_a_repository",
                )
                .await?;
            Ok::<_, &'static str>(PathBuf::from(path))
        }
    };
    let index_path = resolve("index").await?;
    let objects = resolve("objects").await?;
    let exclude = resolve("info/exclude").await?;
    let repo = directory.join("repo.git");
    if !repo.join("HEAD").is_file() {
        let _ = std::fs::remove_dir_all(&repo);
        git.run(
            &[],
            &[
                os("init"),
                os("--quiet"),
                os("--bare"),
                os("--template="),
                repo.as_os_str(),
            ],
            "preparation_failed",
        )
        .await?;
    }
    // Repository internals inherit the private child directory's protection.
    std::fs::create_dir_all(repo.join("info")).map_err(|_| "preparation_failed")?;
    std::fs::create_dir_all(repo.join("objects").join("info")).map_err(|_| "preparation_failed")?;
    write_private(
        &repo.join("objects").join("info").join("alternates"),
        format!("{}\n", objects.display()).as_bytes(),
    )
    .map_err(|_| "preparation_failed")?;
    let excludes = std::fs::read(&exclude).unwrap_or_default();
    write_private(&repo.join("info").join("exclude"), &excludes)
        .map_err(|_| "preparation_failed")?;
    let scratch = directory.join("scratch");
    let _ = std::fs::remove_dir_all(&scratch);
    private_directory(&scratch).map_err(|_| "preparation_failed")?;
    for file in &round.files {
        let path = source.join(file);
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| "context_unreadable")?;
        if !metadata.is_file()
            || std::fs::canonicalize(&path)
                .map_or(true, |canonical| !canonical.starts_with(&source))
        {
            return Err("context_unsupported");
        }
    }
    let first = capture(&git, &source, &repo, &index_path, &scratch, &round.files).await?;
    let second = capture(&git, &source, &repo, &index_path, &scratch, &round.files).await?;
    let _ = std::fs::remove_dir_all(&scratch);
    if first.head != second.head
        || first.index_tree != second.index_tree
        || first.worktree_tree != second.worktree_tree
    {
        return Err("input_changed");
    }
    refuse_unsupported(&git, &source, &repo, &first.worktree_tree).await?;
    let repo_env = [("GIT_DIR", repo.as_os_str())];
    let staged = git
        .run(
            &repo_env,
            &[
                os("commit-tree"),
                os(&first.index_tree),
                os("-p"),
                os(&first.head),
                os("-m"),
                os("Cowboy snapshot: staged changes"),
            ],
            "preparation_failed",
        )
        .await?;
    let working = git
        .run(
            &repo_env,
            &[
                os("commit-tree"),
                os(&first.worktree_tree),
                os("-p"),
                os(&staged),
                os("-m"),
                os("Cowboy snapshot: working tree changes"),
            ],
            "preparation_failed",
        )
        .await?;
    // Mirror the parent's branch and remote-tracking names so a reviewer can
    // diff against its base exactly as in the parent checkout.
    let refs = git
        .run(
            &[],
            &[
                os("-C"),
                source.as_os_str(),
                os("for-each-ref"),
                os("--format=update %(refname) %(objectname)"),
                os("refs/heads"),
                os("refs/remotes"),
                os("refs/tags"),
            ],
            "preparation_failed",
        )
        .await?;
    let mut update = git.command(&repo_env);
    update
        .args(["update-ref", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = update.spawn().map_err(|_| "git_unavailable")?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt as _;
        let mut input = refs
            .lines()
            .filter(|line| !line.contains("refs/heads/cowboy-snapshot"))
            .collect::<Vec<_>>()
            .join("\n");
        input.push_str(&format!("\nupdate refs/heads/cowboy-snapshot {working}\n"));
        stdin
            .write_all(input.as_bytes())
            .await
            .map_err(|_| "preparation_failed")?;
    }
    let status = tokio::time::timeout(GIT_TIMEOUT, child.wait())
        .await
        .map_err(|_| "snapshot_timeout")?
        .map_err(|_| "git_unavailable")?;
    if !status.success() {
        return Err("preparation_failed");
    }
    if workspace.join(".git").exists() {
        // The workspace is a linked worktree of the child repository; refresh
        // it in place so its path (the native session cwd) never changes.
        git.run(
            &[],
            &[
                os("-C"),
                workspace.as_os_str(),
                os("checkout"),
                os("--quiet"),
                os("--force"),
                os("-B"),
                os("cowboy-review"),
                os(&working),
            ],
            "checkout_failed",
        )
        .await?;
        git.run(
            &[],
            &[os("-C"), workspace.as_os_str(), os("clean"), os("-ffdxq")],
            "checkout_failed",
        )
        .await?;
    } else {
        let _ = std::fs::remove_dir_all(&workspace);
        git.run(
            &repo_env,
            &[
                os("worktree"),
                os("add"),
                os("--quiet"),
                os("--force"),
                os("-B"),
                os("cowboy-review"),
                workspace.as_os_str(),
                os(&working),
            ],
            "checkout_failed",
        )
        .await?;
    }
    let tree = git
        .run(
            &[],
            &[
                os("-C"),
                workspace.as_os_str(),
                os("rev-parse"),
                os("HEAD^{tree}"),
            ],
            "checkout_failed",
        )
        .await?;
    if tree != first.worktree_tree {
        return Err("checkout_failed");
    }
    let revision = input_revision(&first, &round.files);
    let marker = RoundMarker {
        schema: 1,
        call_id: round.call_id.clone(),
        input_revision: revision.clone(),
        head: first.head.clone(),
        index_tree: first.index_tree.clone(),
        worktree_tree: first.worktree_tree.clone(),
        output_schema: round.output_schema.clone(),
    };
    let bytes = serde_json::to_vec(&marker).map_err(|_| "preparation_failed")?;
    if bytes.len() as u64 > MAX_ROUND_BYTES {
        return Err("invalid_request");
    }
    write_private(&round_path, &bytes).map_err(|_| "preparation_failed")?;
    Ok(ChildPrepared {
        cwd: workspace.display().to_string(),
        input_revision: revision,
        head: first.head,
        index_tree: first.index_tree,
        worktree_tree: first.worktree_tree,
    })
}

/// Remove one child's owned directory. A missing directory is already closed.
pub fn close(managed: &Path, child: &str) -> std::io::Result<()> {
    if !super::valid_id(child) {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let directory = managed.join(child);
    let _lock = ChildLock::acquire(&directory.display().to_string())
        .ok_or(std::io::ErrorKind::WouldBlock)?;
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(directory),
        Ok(_) => Err(std::io::ErrorKind::InvalidData.into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn git(path: &Path, args: &[&str]) {
        let status = StdCommand::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!("cw-snap-{}", rand::random::<u64>()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        std::fs::canonicalize(path).unwrap()
    }

    fn round(source: &Path, call: &str, files: Vec<String>) -> ChildRound {
        ChildRound {
            child_session_id: "child-1".into(),
            parent_session_id: "parent-1".into(),
            call_id: call.into(),
            workspace_id: "project".into(),
            source_cwd: source.display().to_string(),
            files,
            output_schema: Some(serde_json::json!({"type":"object"})),
            profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
        }
    }

    #[tokio::test]
    async fn captures_head_index_worktree_and_context_without_touching_the_source() {
        let base = root();
        let source = base.join("source");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "-q", "-b", "main"]);
        std::fs::write(source.join(".gitignore"), "ignored/\n").unwrap();
        std::fs::write(source.join("a.txt"), "committed\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-qm", "base"]);
        std::fs::write(source.join("a.txt"), "staged\n").unwrap();
        git(&source, &["add", "a.txt"]);
        std::fs::write(source.join("a.txt"), "working\n").unwrap();
        std::fs::write(source.join("new.txt"), "untracked\n").unwrap();
        std::fs::create_dir(source.join("ignored")).unwrap();
        std::fs::write(source.join("ignored/review.md"), "explicit\n").unwrap();
        std::fs::write(source.join("ignored/secret"), "never\n").unwrap();
        let index_before = std::fs::read(source.join(".git/index")).unwrap();
        let roots = crate::session_workspace::WorktreeRoots::single(base.clone());
        let managed = base.join("managed");
        let prepared = prepare(
            &managed,
            "hawk",
            &round(&source, "call-1", vec!["ignored/review.md".into()]),
            &roots,
            &[],
        )
        .await
        .unwrap();
        let workspace = PathBuf::from(&prepared.cwd);
        assert_eq!(
            std::fs::read_to_string(workspace.join("a.txt")).unwrap(),
            "working\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("new.txt")).unwrap(),
            "untracked\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("ignored/review.md")).unwrap(),
            "explicit\n"
        );
        assert!(!workspace.join("ignored/secret").exists());
        // The parent's index and refs are unchanged; staged content is a commit.
        assert_eq!(
            std::fs::read(source.join(".git/index")).unwrap(),
            index_before
        );
        let staged = StdCommand::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["show", "HEAD~1:a.txt"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&staged.stdout), "staged\n");
        let base_ref = StdCommand::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["rev-parse", "main"])
            .output()
            .unwrap();
        assert!(base_ref.status.success());
        let marker = read_round(&workspace).unwrap();
        assert_eq!(marker.call_id, "call-1");
        assert_eq!(marker.input_revision, prepared.input_revision);
        // A repeated call id observes the original receipt even after edits.
        std::fs::write(source.join("a.txt"), "later\n").unwrap();
        let repeated = prepare(
            &managed,
            "hawk",
            &round(&source, "call-1", vec!["ignored/review.md".into()]),
            &roots,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(repeated, prepared);
        // The next round refreshes the same workspace path in place.
        let next = prepare(
            &managed,
            "hawk",
            &round(&source, "call-2", vec![]),
            &roots,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(next.cwd, prepared.cwd);
        assert_ne!(next.input_revision, prepared.input_revision);
        assert_eq!(
            std::fs::read_to_string(workspace.join("a.txt")).unwrap(),
            "later\n"
        );
        assert!(!workspace.join("ignored/review.md").exists());
        // A different parent cannot adopt this child directory.
        let mut foreign = round(&source, "call-3", vec![]);
        foreign.parent_session_id = "parent-2".into();
        assert_eq!(
            prepare(&managed, "hawk", &foreign, &roots, &[])
                .await
                .unwrap_err(),
            "identity_mismatch"
        );
        close(&managed, "child-1").unwrap();
        assert!(!managed.join("child-1").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn refuses_sources_outside_roots_escaping_links_and_filters() {
        let base = root();
        let source = base.join("source");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "-q", "-b", "main"]);
        std::fs::write(source.join("a"), "a").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-qm", "base"]);
        let managed = base.join("managed");
        let elsewhere =
            crate::session_workspace::WorktreeRoots::single(PathBuf::from("/nonexistent"));
        assert_eq!(
            prepare(
                &managed,
                "hawk",
                &round(&source, "c", vec![]),
                &elsewhere,
                &[]
            )
            .await
            .unwrap_err(),
            "source_unavailable"
        );
        // A linked work tree elsewhere belongs to the registered repository.
        let linked = base.join("elsewhere").join("task");
        git(
            &source,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "task",
                linked.to_str().unwrap(),
            ],
        );
        let linked = std::fs::canonicalize(linked).unwrap();
        let mut outside = round(&source, "linked", vec![]);
        outside.child_session_id = "child-linked".into();
        outside.source_cwd = linked.display().to_string();
        assert!(
            prepare(
                &managed,
                "hawk",
                &outside,
                &elsewhere,
                std::slice::from_ref(&source)
            )
            .await
            .is_ok()
        );
        let unrelated = base.join("unrelated");
        std::fs::create_dir(&unrelated).unwrap();
        git(&unrelated, &["init", "-q"]);
        outside.source_cwd = std::fs::canonicalize(&unrelated)
            .unwrap()
            .display()
            .to_string();
        outside.call_id = "unrelated".into();
        assert_eq!(
            prepare(
                &managed,
                "hawk",
                &outside,
                &elsewhere,
                std::slice::from_ref(&source)
            )
            .await
            .unwrap_err(),
            "source_unavailable"
        );
        let roots = crate::session_workspace::WorktreeRoots::single(base.clone());
        std::os::unix::fs::symlink("/etc/passwd", source.join("escape")).unwrap();
        assert_eq!(
            prepare(&managed, "hawk", &round(&source, "c", vec![]), &roots, &[])
                .await
                .unwrap_err(),
            "symlink_unsupported"
        );
        std::fs::remove_file(source.join("escape")).unwrap();
        std::fs::write(source.join(".gitattributes"), "*.bin filter=lfs\n").unwrap();
        assert_eq!(
            prepare(&managed, "hawk", &round(&source, "c", vec![]), &roots, &[])
                .await
                .unwrap_err(),
            "filter_unsupported"
        );
        assert!(symlink_contained("a/b", "../c"));
        assert!(!symlink_contained("a", "../c"));
        let _ = std::fs::remove_dir_all(base);
    }
}
