//! A bounded watch set for credential files and their creation/rotation paths.
//! Native history, plugins and caches must never become recursive auth watches.

use super::*;

#[derive(Clone, Default)]
pub(crate) struct AuthWatchPlan {
    pub directories: BTreeSet<PathBuf>,
    files: BTreeSet<PathBuf>,
    discovery: BTreeSet<PathBuf>,
}

impl AuthWatchPlan {
    pub(crate) fn discover(&mut self, directory: PathBuf) {
        self.directories.insert(directory.clone());
        self.discovery.insert(directory);
    }

    pub(crate) fn file(&mut self, root: &Path, file: PathBuf) {
        let mut parent = file.parent();
        while let Some(path) = parent.filter(|path| path.starts_with(root)) {
            self.directories.insert(path.to_path_buf());
            parent = path.parent();
        }
        self.files.insert(file);
    }

    pub fn relevant(&self, paths: &[PathBuf]) -> bool {
        paths.iter().any(|changed| {
            self.files.iter().any(|file| file.starts_with(changed))
                || self.discovery.contains(changed)
                || changed
                    .parent()
                    .is_some_and(|parent| self.discovery.contains(parent))
        })
    }
}

impl MachinePluginStore {
    pub fn auth_watch_plan(&self) -> Result<AuthWatchPlan> {
        let mut plan = AuthWatchPlan::default();
        if self.account_id.is_default() {
            plan.discover(self.auth_base.clone());
            plan.discover(self.auth_base.join("accounts"));
        }
        for scope in self.existing_auth_scopes() {
            plan.discover(scope.auth_root.clone());
            match scope.scoped_auth_watch_plan() {
                Ok(scoped) => {
                    plan.directories.extend(scoped.directories);
                    plan.files.extend(scoped.files);
                    plan.discovery.extend(scoped.discovery);
                }
                Err(error) if scope.account_id.is_default() => return Err(error),
                Err(error) => tracing::warn!(account = scope.account_id.as_str(), %error,
                    "cannot discover account credential watches"),
            }
        }
        Ok(plan)
    }

    fn scoped_auth_watch_plan(&self) -> Result<AuthWatchPlan> {
        let root = self.auth_watch_root();
        let mut plan = AuthWatchPlan::default();
        plan.discover(root.clone());
        plan.discover(self.root.clone());
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let provider = entry.file_name().to_string_lossy().into_owned();
            let provider_root = entry.path();
            plan.file(&root, provider_root.join("replica-current.json"));
            plan.file(&self.root, self.plugin_root(&provider).join("active"));
            let runtime = provider_root.join("runtime");
            plan.discover(runtime.clone());
            plan.discover(runtime.join("generations"));
            let package = match self.active_auth_package(&provider) {
                Ok(Some((package, _))) => package,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(%provider, %error, "cannot discover Provider credential watches");
                    continue;
                }
            };
            if package.manifest.authentication.refresh != RefreshOwnership::CompareAndSwap {
                continue;
            }
            for generation in self.writable_auth_projection_generations(&package)? {
                let home = generation.join("home");
                for credential in &package.manifest.authentication.credential_files {
                    let path = home.join(&credential.relative_path);
                    ensure_within(&home, &path)?;
                    plan.file(&root, path);
                }
                plan.file(&root, generation.join("environment.json"));
            }
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_watch_discovery_is_scoped_and_does_not_follow_aliases() {
        let root = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
        let first = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let second = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let paths: Vec<_> = [&store, &first, &second]
            .into_iter()
            .map(|scope| {
                let path = scope.auth_watch_root().join("grok/replica-current.json");
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, b"synthetic").unwrap();
                path
            })
            .collect();
        let outside = tempfile::tempdir().unwrap();
        let alias = store
            .auth_base
            .join("accounts")
            .join(uuid::Uuid::new_v4().to_string());
        std::os::unix::fs::symlink(outside.path(), &alias).unwrap();
        fs::create_dir_all(store.auth_base.join("accounts/not-an-account/providers")).unwrap();
        let plan = store.auth_watch_plan().unwrap();
        assert!(
            paths
                .iter()
                .all(|path| plan.relevant(std::slice::from_ref(path)))
        );
        assert!(!plan.directories.iter().any(|path| path.starts_with(&alias)));
        assert_eq!(store.existing_auth_scopes().len(), 3);
        let scoped = first.auth_watch_plan().unwrap();
        assert!(scoped.relevant(&[paths[1].clone()]));
        assert!(!scoped.relevant(&[paths[0].clone(), paths[2].clone()]));
        assert_eq!(first.existing_auth_scopes().len(), 1);
    }

    #[test]
    fn watches_only_credential_ancestors_and_tracks_atomic_replacement() {
        let root = Path::new("/state/provider-auth/providers");
        let credential =
            root.join("claude-code/runtime/generations/3/home/.claude/.credentials.json");
        let mut plan = AuthWatchPlan::default();
        plan.discover(root.join("claude-code/runtime/generations"));
        plan.file(root, credential.clone());
        assert!(plan.relevant(std::slice::from_ref(&credential)));
        assert!(plan.relevant(&[credential.parent().unwrap().to_path_buf()]));
        assert!(plan.relevant(&[root.join("claude-code/runtime/generations/4")]));
        for name in [
            "debug/session.log",
            "projects/a/session.jsonl",
            "plugins/cache/a",
            ".credentials.json.tmp",
        ] {
            let unrelated = credential.parent().unwrap().join(name);
            assert!(!plan.relevant(std::slice::from_ref(&unrelated)));
            assert!(!plan.directories.contains(&unrelated));
        }
        assert!(plan.directories.len() < 10);
    }
}
