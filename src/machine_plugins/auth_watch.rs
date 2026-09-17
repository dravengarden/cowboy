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
