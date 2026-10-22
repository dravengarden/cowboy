//! Cowboy becomes the project owner on the first successful edit. Host roots
//! are bootstrap input only; removed Matrix aliases cannot reappear on reload.
use super::{WorkspaceConfig, WorkspaceSnapshot, load_workspace_snapshot};
use crate::machine_protocol::{
    MachineWorkspace,
    projects::{Discovery, Registry, Request},
};
use anyhow::{Context as _, ensure};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeSet, VecDeque},
    path::Path,
};

impl WorkspaceConfig {
    fn managed(&self) -> anyhow::Result<Option<Registry>> {
        let Some(path) = &self.managed_path else {
            return Ok(None);
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let registry: Registry =
            serde_json::from_slice(&bytes).context("reading Cowboy projects")?;
        ensure!(
            registry.schema == 1 && registry.managed && !registry.revision.is_empty(),
            "invalid project registry"
        );
        ensure!(registry.projects.len() <= 512, "too many projects");
        let mut ids = BTreeSet::new();
        for project in &registry.projects {
            validate(project)?;
            ensure!(ids.insert(&project.id), "duplicate project ID");
        }
        Ok(Some(registry))
    }

    pub(super) fn load_snapshot(&self) -> anyhow::Result<WorkspaceSnapshot> {
        if let Some(registry) = self.managed()? {
            return Ok(WorkspaceSnapshot {
                owner: crate::machine_protocol::projects::Owner::Cowboy,
                revision: Some(registry.revision),
                workspaces: registry.projects,
            });
        }
        load_workspace_snapshot(&self.path, &self.fallback)
    }

    pub(super) fn project_request(&self, request: Request) -> anyhow::Result<serde_json::Value> {
        if let Request::Discover { root } = request {
            return Ok(serde_json::to_value(discover(&root)?)?);
        }
        // One lock covers disk CAS, retirement and publication, including the
        // bootstrap file. A timed-out edit must be observed before a new CAS.
        let mut identities = self.identities.lock();
        let mut registry = if let Some(registry) = self.managed()? {
            registry
        } else {
            let snapshot = load_workspace_snapshot(&self.path, &self.fallback)?;
            let bytes = serde_json::to_vec(&snapshot.workspaces)?;
            Registry {
                schema: 1,
                revision: format!("bootstrap-{:x}", Sha256::digest(bytes)),
                managed: false,
                projects: snapshot.workspaces,
            }
        };
        match request {
            Request::List => return Ok(serde_json::to_value(registry)?),
            Request::Adopt { expected_revision } => {
                ensure!(
                    expected_revision == registry.revision,
                    "Projects changed; reload before saving"
                );
            }
            Request::Discover { .. } => unreachable!(),
            Request::Upsert {
                expected_revision,
                mut project,
            } => {
                ensure!(
                    expected_revision == registry.revision,
                    "Projects changed; reload before saving"
                );
                validate(&project)?;
                let canonical = Path::new(&project.canonical_path)
                    .canonicalize()
                    .context("project directory is unavailable")?;
                ensure!(canonical.is_dir(), "project must be a directory");
                project.canonical_path = canonical
                    .to_str()
                    .context("project path must be UTF-8")?
                    .into();
                ensure!(
                    !registry
                        .projects
                        .iter()
                        .any(|p| p.id != project.id && p.canonical_path == project.canonical_path),
                    "this directory is already registered"
                );
                if let Some(existing) = registry.projects.iter_mut().find(|p| p.id == project.id) {
                    ensure!(
                        existing.canonical_path == project.canonical_path,
                        "an existing project cannot be moved; register a new project ID"
                    );
                    *existing = project;
                } else {
                    ensure!(registry.projects.len() < 512, "too many projects");
                    registry.projects.push(project);
                }
            }
            Request::Remove {
                expected_revision,
                id,
            } => {
                ensure!(
                    expected_revision == registry.revision,
                    "Projects changed; reload before saving"
                );
                ensure!(
                    registry.projects.iter().any(|p| p.id == id),
                    "unknown project"
                );
                registry.projects.retain(|p| p.id != id);
            }
        }
        ensure!(registry.projects.len() <= 512, "too many projects");
        for project in &registry.projects {
            validate(project)?;
        }
        registry.managed = true;
        registry.revision = format!("projects-{:032x}", rand::random::<u128>());
        registry.projects.sort_by(|a, b| a.id.cmp(&b.id));
        crate::owned_json::write(
            self.managed_path
                .as_deref()
                .context("project management is unavailable")?,
            &registry,
        )?;
        let snapshot = WorkspaceSnapshot {
            owner: crate::machine_protocol::projects::Owner::Cowboy,
            revision: Some(registry.revision.clone()),
            workspaces: registry.projects.clone(),
        };
        self.updates.send_modify(|current| {
            identities.retain_configuration(&current.workspaces, &snapshot.workspaces);
            *current = snapshot;
        });
        Ok(serde_json::to_value(registry)?)
    }
}

fn validate(project: &MachineWorkspace) -> anyhow::Result<()> {
    ensure!(
        !project.id.is_empty()
            && project.id.len() <= 128
            && project
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
        "invalid project ID"
    );
    ensure!(
        !project.display_name.trim().is_empty()
            && project.display_name.len() <= 256
            && !project.display_name.chars().any(char::is_control),
        "invalid project name"
    );
    let path = Path::new(&project.canonical_path);
    ensure!(
        path.is_absolute()
            && project.canonical_path.len() <= 4096
            && !project.canonical_path.chars().any(char::is_control)
            && !path.components().any(|p| matches!(
                p,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )),
        "project path must be an absolute normalized directory"
    );
    Ok(())
}

fn discover(root: &str) -> anyhow::Result<Discovery> {
    ensure!(
        Path::new(root).is_absolute(),
        "discovery needs an absolute directory"
    );
    let root = Path::new(root).canonicalize()?;
    ensure!(
        root.parent().is_some() && root.is_dir(),
        "choose a project directory, not the filesystem root"
    );
    let mut queue = VecDeque::from([(root.clone(), 0)]);
    let mut paths = Vec::new();
    let mut visited = 0;
    let mut truncated = false;
    while let Some((path, depth)) = queue.pop_front() {
        visited += 1;
        if visited > 1024 || paths.len() >= 200 {
            truncated = true;
            break;
        }
        if depth == 0
            || path.join(".git").exists()
            || path.join("project-defs/registry.toml").is_file()
        {
            paths.push(path.display().to_string());
        }
        if depth == 3 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.')
                || matches!(&*name, "node_modules" | "target" | "vendor" | "result")
            {
                continue;
            }
            if queue.len() + visited >= 1024 {
                truncated = true;
                break;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    paths.sort();
    Ok(Discovery {
        root: root.display().to_string(),
        paths,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn list(config: &WorkspaceConfig) -> Registry {
        serde_json::from_value(config.project_request(Request::List).unwrap()).unwrap()
    }

    #[test]
    fn managed_projects_cas_persists_and_never_resurrects_bootstrap_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let args = vec![format!("legacy={}", dir.path().display())];
        let managed = dir.path().join("projects.json");
        let config = WorkspaceConfig::new(
            dir.path().join("bootstrap"),
            args.clone(),
            Some(managed.clone()),
        )
        .unwrap();
        let old = list(&config);
        config
            .project_request(Request::Remove {
                expected_revision: old.revision.clone(),
                id: "legacy".into(),
            })
            .unwrap();
        assert!(
            config
                .project_request(Request::Remove {
                    expected_revision: old.revision,
                    id: "legacy".into()
                })
                .is_err()
        );
        config.reload().unwrap();
        assert!(config.snapshot().workspaces.is_empty());
        std::fs::write(
            dir.path().join("bootstrap"),
            b"retired bootstrap is no longer authoritative",
        )
        .unwrap();
        let restart =
            WorkspaceConfig::new(dir.path().join("bootstrap"), args, Some(managed)).unwrap();
        assert!(list(&restart).projects.is_empty());
        assert!(list(&restart).managed);
    }

    #[test]
    fn registration_validates_local_path_and_does_not_retarget_identity() {
        let dir = tempfile::tempdir().unwrap();
        let config = WorkspaceConfig::new(
            dir.path().join("bootstrap"),
            vec![],
            Some(dir.path().join("projects.json")),
        )
        .unwrap();
        let project = MachineWorkspace {
            id: "stable".into(),
            display_name: "hawk/anything".into(),
            canonical_path: dir.path().display().to_string(),
        };
        config
            .project_request(Request::Upsert {
                expected_revision: list(&config).revision,
                project: project.clone(),
            })
            .unwrap();
        let another = tempfile::tempdir().unwrap();
        let moved = MachineWorkspace {
            canonical_path: another.path().display().to_string(),
            ..project
        };
        assert!(
            config
                .project_request(Request::Upsert {
                    expected_revision: list(&config).revision,
                    project: moved
                })
                .is_err()
        );
        assert_eq!(
            list(&config).projects[0].canonical_path,
            dir.path().display().to_string()
        );
    }

    #[test]
    fn discovery_is_bounded_and_does_not_follow_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(outside.path().join(".git")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("outside")).unwrap();
        std::fs::create_dir_all(root.path().join("projects/repo/.git")).unwrap();
        let found = discover(root.path().to_str().unwrap()).unwrap();
        assert_eq!(found.paths.len(), 2);
        assert!(!found.paths.iter().any(|p| p.contains("outside")));
    }
}
