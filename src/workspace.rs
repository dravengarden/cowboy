//! Canonical session workspace resolution.
//!
//! Cowboy persists the cwd that opened a session, but checkouts may move while
//! the native Codex thread remains valid. This module maps stale Columbus paths
//! to the registry's current stable checkout without touching Codex state.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkspace {
    pub path: PathBuf,
    pub changed: bool,
    pub project: Option<String>,
}

pub fn resolve_session_workspace(
    workspace_root: &Path,
    stored_cwd: &Path,
) -> Result<ResolvedWorkspace, String> {
    if usable_directory(stored_cwd) {
        return Ok(ResolvedWorkspace {
            path: stored_cwd.to_path_buf(),
            changed: false,
            project: None,
        });
    }

    let columbus = workspace_root.join("columbus");
    let project = infer_columbus_project(&columbus, stored_cwd).ok_or_else(|| {
        format!(
            "session workspace is unavailable: {}. Cowboy could not map it to a registered Columbus project; restore the directory or open the session after choosing a valid workspace",
            stored_cwd.display()
        )
    })?;
    let replacement = current_project_checkout(&columbus, &project).ok_or_else(|| {
        format!(
            "session workspace is unavailable: {}. Columbus project {project:?} was identified, but no usable checkout could be resolved; run `harness-cli --root {} project path {project}` and restore or clone that checkout",
            stored_cwd.display(),
            columbus.display()
        )
    })?;

    Ok(ResolvedWorkspace {
        changed: replacement != stored_cwd,
        path: replacement,
        project: Some(project),
    })
}

fn usable_directory(path: &Path) -> bool {
    path.is_dir() && std::fs::read_dir(path).is_ok()
}

fn infer_columbus_project(columbus: &Path, stale: &Path) -> Option<String> {
    if let Ok(relative) = stale.strip_prefix(columbus.join("projects"))
        && let Some(name) = relative.components().next()
    {
        return registered_project(columbus, name.as_os_str().to_string_lossy().as_ref());
    }

    let workspace_root = columbus.parent()?;
    let state_root = if std::env::var_os("HOME").as_deref() == Some(workspace_root.as_os_str()) {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root.join(".local/state"))
    } else {
        workspace_root.join(".local/state")
    };
    stale
        .strip_prefix(state_root.join("columbus/bare-migration"))
        .ok()
        .and_then(|relative| relative.components().next())
        .and_then(|name| registered_project(columbus, name.as_os_str().to_string_lossy().as_ref()))
}

/// Whether a persisted session cwd belongs to a registered Columbus project.
/// This deliberately recognizes both the stable project tree and the temporary
/// bare-migration backup tree so a migration can fence workers whose actual cwd
/// inode moved even when the stable pathname itself was recreated.
pub fn session_belongs_to_project(columbus: &Path, cwd: &Path, project: &str) -> bool {
    infer_columbus_project(columbus, cwd).as_deref() == Some(project)
}

fn registered_project(columbus: &Path, name: &str) -> Option<String> {
    columbus
        .join("project-defs")
        .join(name)
        .join("project.toml")
        .is_file()
        .then(|| name.to_owned())
}

pub fn current_project_checkout(columbus: &Path, project: &str) -> Option<PathBuf> {
    if !valid_project_name(project) {
        return None;
    }
    let root = columbus.display().to_string();
    if let Ok(output) = Command::new("harness-cli")
        .args(["--root", &root, "project", "path", project])
        .output()
        && output.status.success()
        && let Ok(path) = String::from_utf8(output.stdout)
    {
        let path = PathBuf::from(path.trim());
        if usable_directory(&path) {
            return Some(path);
        }
    }

    let project_file = columbus
        .join("project-defs")
        .join(project)
        .join("project.toml");
    let body = std::fs::read_to_string(project_file).ok()?;
    let kind = toml_string(&body, "kind");
    let base = columbus.join("projects").join(project);
    let candidate = if kind.as_deref() == Some("external") {
        base.join(toml_string(&body, "default_branch")?)
    } else {
        base
    };
    usable_directory(&candidate).then_some(candidate)
}

fn valid_project_name(project: &str) -> bool {
    !project.is_empty()
        && project
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn toml_string(body: &str, key: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        if candidate.trim() != key {
            return None;
        }
        let value = value.trim();
        value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .map(str::to_owned)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cowboy-workspace-test-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("tempdir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> (TestDir, PathBuf) {
        let root = TestDir::new();
        let columbus = root.path().join("columbus");
        let definition = columbus
            .join("project-defs")
            .join("corsair")
            .join("project.toml");
        std::fs::create_dir_all(definition.parent().expect("definition parent"))
            .expect("project definition dir");
        std::fs::write(
            definition,
            "kind = \"external\"\nrepo = \"git@example/corsair\"\ndefault_branch = \"main\"\n",
        )
        .expect("project definition");
        let checkout = columbus.join("projects/corsair/main");
        std::fs::create_dir_all(&checkout).expect("checkout");
        (root, checkout)
    }

    #[test]
    fn existing_workspace_is_unchanged() {
        let (root, checkout) = fixture();
        let resolved = resolve_session_workspace(root.path(), &checkout).expect("resolve");
        assert!(!resolved.changed);
        assert_eq!(resolved.path, checkout);
    }

    #[test]
    fn deleted_migration_workspace_maps_to_current_checkout() {
        let (root, checkout) = fixture();
        let stale = root
            .path()
            .join(".local/state/columbus/bare-migration/corsair/stamp/worktrees/stable");
        let resolved = resolve_session_workspace(root.path(), &stale).expect("resolve");
        assert!(resolved.changed);
        assert_eq!(resolved.project.as_deref(), Some("corsair"));
        assert_eq!(resolved.path, checkout);
    }

    #[test]
    fn deleted_project_workspace_maps_to_current_checkout() {
        let (root, checkout) = fixture();
        let stale = root.path().join("columbus/projects/corsair/old-branch");
        let resolved = resolve_session_workspace(root.path(), &stale).expect("resolve");
        assert_eq!(resolved.path, checkout);
    }

    #[test]
    fn unknown_deleted_workspace_fails_with_actionable_error() {
        let (root, _) = fixture();
        let stale = root.path().join("gone/unregistered");
        let error = resolve_session_workspace(root.path(), &stale).expect_err("must fail");
        assert!(error.contains("restore the directory"));
        assert!(error.contains(stale.to_string_lossy().as_ref()));
    }

    #[test]
    fn project_membership_recognizes_stable_and_migration_paths() {
        let (root, checkout) = fixture();
        let columbus = root.path().join("columbus");
        let backup = root
            .path()
            .join(".local/state/columbus/bare-migration/corsair/stamp/worktrees/stable");

        assert!(session_belongs_to_project(&columbus, &checkout, "corsair"));
        assert!(session_belongs_to_project(&columbus, &backup, "corsair"));
        assert!(!session_belongs_to_project(&columbus, &checkout, "cowboy"));
        assert!(!session_belongs_to_project(
            &columbus,
            &root.path().join("tmp/bare-migration/corsair/stable"),
            "corsair"
        ));
        assert!(current_project_checkout(&columbus, "../corsair").is_none());
    }
}
