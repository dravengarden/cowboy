//! Operator-owned source/storage path wiring. No project names or disk policy.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};

pub(super) const ENVIRONMENT_NAMES: [&str; 4] = [
    "CARGO_BUILD_BUILD_DIR",
    "GOCACHE",
    "BUN_INSTALL_CACHE_DIR",
    "TMPDIR",
];
const MARKER: &str = ".cowboy-storage";
const MARKER_CONTENT: &[u8] = b"cowboy-workspace-storage-v1\n";

#[derive(Clone, Debug)]
pub struct WorkspaceStorage {
    source_alias: PathBuf,
    source: PathBuf,
    root: PathBuf,
    worktrees: PathBuf,
    device: u64,
    inode: u64,
    owner: u32,
    // Pin the admitted inode so deletion/recreation cannot reuse its identity.
    _root_handle: Arc<fs::File>,
}

impl FromStr for WorkspaceStorage {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let (source, root) = value
            .split_once('=')
            .context("workspace storage must be /absolute/source=/absolute/storage")?;
        ensure!(
            Path::new(source).is_absolute() && Path::new(root).is_absolute(),
            "workspace storage source and destination must be absolute paths"
        );
        let source_alias = PathBuf::from(source);
        let source = source_alias
            .canonicalize()
            .context("resolving workspace storage source")?;
        let root = Path::new(root)
            .canonicalize()
            .context("workspace storage root must already exist")?;
        ensure!(
            source.is_dir() && root.is_dir(),
            "workspace storage paths must be directories"
        );
        ensure!(
            !root.starts_with(&source),
            "workspace storage must be outside the source checkout"
        );
        let handle = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&root)?;
        let metadata = handle.metadata()?;
        let route = Self {
            source_alias,
            source,
            worktrees: root.join("worktrees/sessions"),
            root,
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
            _root_handle: Arc::new(handle),
        };
        route.validate_root()?;
        Ok(route)
    }
}

impl WorkspaceStorage {
    pub(super) fn worktrees(&self) -> &Path {
        &self.worktrees
    }

    pub(super) fn validate_root(&self) -> Result<()> {
        let metadata = fs::symlink_metadata(&self.root)
            .with_context(|| format!("workspace storage unavailable: {}", self.root.display()))?;
        ensure!(
            metadata.is_dir() && metadata.dev() == self.device && metadata.ino() == self.inode,
            "workspace storage root changed: {}",
            self.root.display()
        );
        // Provision this on the intended mounted filesystem. An empty,
        // unmounted mountpoint is never sufficient startup admission.
        let mut marker = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.root.join(MARKER))
            .context("workspace storage needs a host-provisioned .cowboy-storage marker")?;
        let metadata = marker.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == self.owner
                && metadata.len() == MARKER_CONTENT.len() as u64,
            "invalid workspace storage marker"
        );
        let mut content = [0_u8; MARKER_CONTENT.len()];
        marker.read_exact(&mut content)?;
        ensure!(
            content == MARKER_CONTENT,
            "invalid workspace storage marker version"
        );
        Ok(())
    }

    /// Reject symlink children and replacement mounts rather than writing
    /// caches through them or falling back onto the Machine's default disk.
    fn directory(&self, relative: &str) -> Result<PathBuf> {
        self.validate_root()?;
        let mut path = self.root.clone();
        for part in relative.split('/') {
            path.push(part);
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error).context("creating workspace storage directory"),
            }
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                metadata.is_dir() && metadata.dev() == self.device && metadata.uid() == self.owner,
                "workspace storage child must be a same-device owned directory: {}",
                path.display()
            );
        }
        Ok(path)
    }

    pub(super) fn prepare_worktrees(&self) -> Result<()> {
        self.directory("worktrees/sessions").map(|_| ())
    }

    pub(super) fn environment(&self) -> Result<BTreeMap<String, String>> {
        Ok(BTreeMap::from([
            // Keep intermediates inside the checkout's marked target so the
            // existing bounded deletion cleanup also owns their lifecycle.
            (
                "CARGO_BUILD_BUILD_DIR".into(),
                "{workspace-root}/target".into(),
            ),
            (
                "GOCACHE".into(),
                self.directory("cache/go")?.display().to_string(),
            ),
            (
                "BUN_INSTALL_CACHE_DIR".into(),
                self.directory("cache/bun")?.display().to_string(),
            ),
            (
                "TMPDIR".into(),
                self.directory("tmp")?.display().to_string(),
            ),
        ]))
    }
}

pub(super) fn validate_routes<'a>(
    routes: &[WorkspaceStorage],
    roots: impl Iterator<Item = &'a Path>,
) -> Result<()> {
    let roots: Vec<_> = roots
        .map(|root| root.canonicalize().unwrap_or_else(|_| root.to_path_buf()))
        .collect();
    for (index, route) in routes.iter().enumerate() {
        ensure!(
            !roots
                .iter()
                .any(|root| route.root.starts_with(root) || root.starts_with(&route.root)),
            "workspace storage overlaps a default worktree root"
        );
        for previous in &routes[..index] {
            ensure!(
                route.source != previous.source,
                "duplicate workspace storage source"
            );
            ensure!(
                !route.root.starts_with(&previous.root) && !previous.root.starts_with(&route.root),
                "workspace storage destinations must not overlap"
            );
            ensure!(
                !route.root.starts_with(&previous.source)
                    && !previous.root.starts_with(&route.source)
                    && !route.source.starts_with(&previous.root)
                    && !previous.source.starts_with(&route.root),
                "workspace storage must not overlap another route's source"
            );
        }
    }
    Ok(())
}

pub(super) fn select<'a>(
    routes: &'a [WorkspaceStorage],
    path: &Path,
) -> Result<Option<&'a WorkspaceStorage>> {
    if routes.is_empty() {
        return Ok(None);
    }
    // Unavailable unrelated remote cwd paths do not need local storage. A
    // configured alias, however, must still resolve to its admitted source.
    let canonical = path.canonicalize().ok();
    let route = routes
        .iter()
        .filter(|route| {
            path.starts_with(&route.source_alias)
                || path.starts_with(&route.source)
                || path.starts_with(&route.root)
                || canonical.as_ref().is_some_and(|path| {
                    path.starts_with(&route.source) || path.starts_with(&route.root)
                })
        })
        .max_by_key(|route| route.source.components().count());
    if let Some(route) = route {
        route.validate_root()?;
        let canonical = canonical.context("configured workspace path is unavailable")?;
        ensure!(
            canonical.starts_with(&route.source) || canonical.starts_with(&route.root),
            "configured workspace source changed"
        );
    }
    Ok(route)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_workspace::WorktreeRoots;
    use std::os::unix::fs::symlink;

    fn route(base: &Path, name: &str) -> WorkspaceStorage {
        let source = base.join(format!("source-{name}"));
        let storage = base.join(format!("storage-{name}"));
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&storage).unwrap();
        fs::write(storage.join(MARKER), MARKER_CONTENT).unwrap();
        format!("{}={}", source.display(), storage.display())
            .parse()
            .unwrap()
    }

    #[test]
    fn routes_aliases_and_preserves_existing_sessions_without_prefix_collisions() {
        let temp = tempfile::tempdir().unwrap();
        let route = route(temp.path(), "one");
        let alias = temp.path().join("alias");
        symlink(&route.source, &alias).unwrap();
        let primary = temp.path().join("primary");
        let legacy = temp.path().join("legacy");
        fs::create_dir_all(primary.join("sess-primary")).unwrap();
        fs::create_dir_all(legacy.join("sess-legacy")).unwrap();
        let roots = WorktreeRoots::moved(primary.clone(), legacy.clone())
            .with_storage(vec![route.clone()])
            .unwrap();
        assert_eq!(
            roots.for_source("sess-new", &alias).unwrap(),
            route.worktrees
        );
        assert_eq!(roots.for_source("sess-primary", &alias).unwrap(), primary);
        assert_eq!(roots.for_source("sess-legacy", &alias).unwrap(), legacy);
        let sibling = temp.path().join("source-one-unrelated");
        fs::create_dir(&sibling).unwrap();
        assert_eq!(
            roots.for_source("sess-unrelated", &sibling).unwrap(),
            primary
        );
        fs::create_dir(route.worktrees.join("sess-moved")).unwrap();
        symlink(
            route.worktrees.join("sess-moved"),
            legacy.join("sess-moved"),
        )
        .unwrap();
        assert_eq!(roots.for_session("sess-moved"), route.worktrees);
        assert!(roots.all().any(|root| root == route.worktrees));
        assert!(roots.for_source("../escape", &alias).is_err());
    }

    #[test]
    fn caches_share_project_data_but_cargo_stays_in_its_workspace_for_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let route = route(temp.path(), "one");
        let roots = WorktreeRoots::single(temp.path().join("primary"))
            .with_storage(vec![route.clone()])
            .unwrap();
        let environment = roots.storage_environment(&route.source).unwrap();
        assert_eq!(environment.len(), ENVIRONMENT_NAMES.len());
        for (name, value) in &environment {
            assert!(crate::execution_target_environment::allowed(name));
            if name != "CARGO_BUILD_BUILD_DIR" {
                assert!(Path::new(value).starts_with(&route.root));
            }
        }
        assert_eq!(
            environment["CARGO_BUILD_BUILD_DIR"],
            "{workspace-root}/target"
        );
        fs::create_dir(route.worktrees.join("sess-new")).unwrap();
        assert_eq!(
            roots
                .storage_environment(&route.worktrees.join("sess-new"))
                .unwrap(),
            environment
        );
        assert!(roots.storage_environment(temp.path()).unwrap().is_empty());
        assert!(
            WorktreeRoots::single(temp.path().join("default"))
                .storage_environment(Path::new("/unavailable/remote/cwd"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn missing_replaced_storage_and_escaping_children_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let route = route(temp.path(), "one");
        let roots = WorktreeRoots::single(temp.path().join("primary"))
            .with_storage(vec![route.clone()])
            .unwrap();
        let retired = temp.path().join("retired");
        fs::rename(&route.root, &retired).unwrap();
        assert!(roots.for_source("sess-new", &route.source).is_err());
        assert!(!route.root.exists());
        fs::create_dir(&route.root).unwrap();
        assert!(roots.storage_environment(&route.source).is_err());
        fs::remove_dir(&route.root).unwrap();
        fs::rename(&retired, &route.root).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, route.root.join("cache")).unwrap();
        assert!(roots.storage_environment(&route.source).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }

    #[test]
    fn mappings_reject_relative_missing_duplicate_and_overlapping_roots() {
        assert!("relative=/absolute".parse::<WorkspaceStorage>().is_err());
        assert!("/absolute=relative".parse::<WorkspaceStorage>().is_err());
        assert!("missing-delimiter".parse::<WorkspaceStorage>().is_err());
        let temp = tempfile::tempdir().unwrap();
        let route = route(temp.path(), "one");
        let roots = WorktreeRoots::single(temp.path().join("primary"));
        assert!(
            roots
                .clone()
                .with_storage(vec![route.clone(), route.clone()])
                .is_err()
        );
        assert!(
            WorktreeRoots::single(route.root.clone())
                .with_storage(vec![route.clone()])
                .is_err()
        );
        assert!(
            format!(
                "{}={}/missing",
                route.source.display(),
                route.root.display()
            )
            .parse::<WorkspaceStorage>()
            .is_err()
        );
        assert!(
            format!("{}={}", route.source.display(), route.source.display())
                .parse::<WorkspaceStorage>()
                .is_err()
        );
        let nested_root = route.root.join("nested");
        fs::create_dir(&nested_root).unwrap();
        fs::write(nested_root.join(MARKER), MARKER_CONTENT).unwrap();
        let source_two = temp.path().join("source-two");
        fs::create_dir(&source_two).unwrap();
        let nested = format!("{}={}", source_two.display(), nested_root.display())
            .parse()
            .unwrap();
        assert!(roots.with_storage(vec![route, nested]).is_err());
    }

    #[test]
    fn empty_mountpoint_and_cross_route_source_storage_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let first = route(temp.path(), "one");
        let second = route(temp.path(), "two");
        fs::remove_file(first.root.join(MARKER)).unwrap();
        assert!(
            format!("{}={}", first.source.display(), first.root.display())
                .parse::<WorkspaceStorage>()
                .is_err()
        );
        fs::write(first.root.join(MARKER), MARKER_CONTENT).unwrap();
        let nested = first.source.join("another-project-storage");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join(MARKER), MARKER_CONTENT).unwrap();
        let second = format!("{}={}", second.source.display(), nested.display())
            .parse()
            .unwrap();
        assert!(
            WorktreeRoots::single(temp.path().join("primary"))
                .with_storage(vec![first, second])
                .is_err()
        );
    }

    #[test]
    fn unavailable_route_defers_cleanup_without_claiming_the_root_changed() {
        let temp = tempfile::tempdir().unwrap();
        let route = route(temp.path(), "one");
        let roots = WorktreeRoots::single(temp.path().join("primary"))
            .with_storage(vec![route.clone()])
            .unwrap();
        fs::create_dir(route.worktrees.join("sess-deleted")).unwrap();
        fs::rename(&route.root, temp.path().join("offline")).unwrap();
        let error = roots.for_cleanup("sess-deleted").unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::session_workspace::CleanupRootChanged>()
                .is_none()
        );
        fs::rename(temp.path().join("offline"), &route.root).unwrap();
        assert_eq!(roots.for_cleanup("sess-deleted").unwrap(), route.worktrees);
    }

    #[tokio::test]
    async fn routed_git_worktree_can_be_reused_and_captured_for_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let route = route(temp.path(), "git");
        for args in [
            vec!["init", "-q"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "initial",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&route.source)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let roots = WorktreeRoots::single(temp.path().join("primary"))
            .with_storage(vec![route.clone()])
            .unwrap();
        for created in [true, false] {
            let result = crate::session_workspace::prepare(
                crate::session_workspace::PrepareWorkspaceRequest {
                    root: route.source.display().to_string(),
                    session_id: "sess-route".into(),
                    ..Default::default()
                },
                roots.for_source("sess-route", &route.source).unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(result.created, created);
            assert!(result.isolated);
            assert_eq!(Path::new(&result.path), route.worktrees.join("sess-route"));
            let cleanup = crate::session_workspace::capture_cleanup_workspace(
                roots.for_session("sess-route"),
                "sess-route",
                Path::new(&result.path),
            )
            .unwrap();
            crate::session_workspace::resume_cleanup_workspace(
                roots.for_session("sess-route"),
                "sess-route",
                &cleanup.root_identity().unwrap(),
            )
            .unwrap();
        }
        let workspace = route.worktrees.join("sess-route");
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"storage-cleanup-test\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
        assert!(
            std::process::Command::new("cargo")
                .args(["build", "--offline", "--quiet"])
                .current_dir(&workspace)
                .env_remove("CARGO_TARGET_DIR")
                .env_remove("CARGO_BUILD_TARGET_DIR")
                .envs(roots.storage_environment(&workspace).unwrap())
                .status()
                .unwrap()
                .success()
        );
        assert!(workspace.join("target/debug/deps").is_dir());
        assert!(
            fs::read_dir(workspace.join("target/debug/deps"))
                .unwrap()
                .count()
                > 0
        );
        let cleanup = crate::session_workspace::capture_cleanup_workspace(
            roots.for_cleanup("sess-route").unwrap(),
            "sess-route",
            &workspace,
        )
        .unwrap();
        crate::session_workspace::cleanup_build_artifacts(&cleanup)
            .await
            .unwrap();
        // Cleanup intentionally retains empty directory inodes.
        assert_eq!(
            fs::read_dir(workspace.join("target/debug/deps"))
                .unwrap()
                .count(),
            0
        );
        assert!(!workspace.join("target/debug/storage-cleanup-test").exists());
        assert!(workspace.join("src/main.rs").is_file());
        assert!(!route.root.join("cache/cargo").exists());
    }
}
