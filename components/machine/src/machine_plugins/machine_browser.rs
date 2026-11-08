//! The installed Machine browser Plugin as core launches it: its verified
//! runtime plan and the account root its instances are isolated under.
use super::retirement::uninstalled_generations_of;
use super::*;

/// The Machine browser manager as the Plugin store reaches it: uninstalling
/// the browser Plugin first drains every running instance through it, and
/// generation retirement keeps every runtime generation an instance pins.
/// The manager registers itself at construction, so the store never depends
/// on the browser module.
pub trait BrowserDrain: Send + Sync {
    /// Run `retire` with every runtime generation digest (`sha256:<hex>`)
    /// a browser instance may still run from: registered (starting, running
    /// or stopping) instances, opens that resolved a runtime and have not
    /// registered yet, and units retained until their exit is confirmed. New
    /// pins wait until `retire` returns, so a generation outside the set may
    /// be removed: no start can pin it meanwhile, and a start that resolved
    /// it earlier finds it gone and refuses.
    ///
    /// # Errors
    /// Returns without calling `retire` while the pins are not known (the
    /// restart inventory has not completed, or a unit of unknown generation
    /// may still run), and returns `retire`'s error.
    fn with_generation_pins(&self, retire: &mut GenerationRetirement<'_>) -> Result<()>;

    /// Hibernate every running instance (gracefully, with its stop-time
    /// snapshot where possible) and confirm each unit's exit. The returned
    /// hold refuses new opens until dropped. `authority` is checked before
    /// the drain and again before every individual stop; once it fails the
    /// drain stops nothing more.
    ///
    /// # Errors
    /// Returns when an instance could not be confirmed stopped or the
    /// authority ended.
    fn drain<'a>(&'a self, authority: &'a DrainAuthority<'a>) -> DrainFuture<'a>;
}

/// Generation retirement run while the browser pins are held.
pub type GenerationRetirement<'a> = dyn FnMut(&BTreeSet<String>) -> Result<()> + 'a;

/// Whether the uninstall that requested a drain may still act: its execution
/// lease is connected and unexpired.
pub type DrainAuthority<'a> = dyn Fn() -> Result<()> + Send + Sync + 'a;

/// What a drain resolves to: a hold that refuses new opens while it lives.
pub type DrainFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Box<dyn std::any::Any + Send>>> + Send + 'a>,
>;

/// The registered drain, shared by every clone of the store.
pub(super) type DrainSlot =
    std::sync::Arc<parking_lot::Mutex<Option<std::sync::Weak<dyn BrowserDrain>>>>;

impl MachinePluginStore {
    /// Register the Machine browser manager that uninstall drains.
    pub fn register_browser_drain(&self, drain: std::sync::Weak<dyn BrowserDrain>) {
        *self.browser_drain.lock() = Some(drain);
    }

    /// Retire Machine browser runtime generations no instance can run any
    /// more, for every browser Plugin under the store (or only `only`). An
    /// installed Plugin keeps its active and rollback generations; an
    /// uninstalled one keeps nothing of its own, and a dangling rollback link
    /// is removed. Every generation a browser instance pins (see
    /// [`BrowserDrain::with_generation_pins`]) is kept either way. `skip`
    /// names Plugins whose uninstall may still compensate by reactivating.
    /// Without a registered browser manager nothing is retired. The caller
    /// holds the store's lifecycle lock.
    ///
    /// # Errors
    /// Returns when the pins are unknown or a removal fails.
    pub(super) fn retire_browser_generations_locked(
        &self,
        only: Option<&str>,
        skip: &BTreeSet<String>,
    ) -> Result<Vec<crate::generation_retention::Outcome>> {
        let Some(pins) = self
            .browser_drain
            .lock()
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        else {
            return Ok(Vec::new());
        };
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&self.root).context("reading Plugin store")? {
            let entry = entry?;
            let plugin_id = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type()?.is_dir()
                || only.is_some_and(|only| only != plugin_id)
                || skip.contains(&plugin_id)
                || validate_plugin_id(&plugin_id).is_err()
                || !entry.path().join("generations").is_dir()
                || self.operations.ensure_unfenced(&plugin_id).is_err()
            {
                continue;
            }
            let installed = entry.path().join("active").symlink_metadata().is_ok();
            let browser = if installed {
                self.is_browser_plugin(&plugin_id)
            } else {
                uninstalled_generations_of(
                    &entry.path().join("generations"),
                    cowboy_plugin_sdk::PluginKind::MachineBrowser,
                )
            };
            if browser {
                candidates.push((plugin_id, entry.path(), installed));
            }
        }
        let mut outcomes = Vec::new();
        if candidates.is_empty() {
            return Ok(outcomes);
        }
        pins.with_generation_pins(&mut |pinned| {
            let pinned: BTreeSet<String> = pinned
                .iter()
                .map(|digest| digest.trim_start_matches("sha256:").to_owned())
                .collect();
            for (plugin_id, root, installed) in &candidates {
                let mut keep = pinned.clone();
                if *installed {
                    keep.extend(read_link_name(&root.join("active")));
                    keep.extend(read_link_name(&root.join("rollback")));
                }
                let generations = root.join("generations");
                let mut retention = crate::generation_retention::Outcome {
                    plugin_id: plugin_id.clone(),
                    ..crate::generation_retention::Outcome::default()
                };
                self.retire_unkept(&generations, &keep, &mut retention)?;
                // A rollback link has nothing to roll back to once uninstalled.
                if !*installed
                    && read_link_name(&root.join("rollback"))
                        .is_some_and(|name| !generations.join(&name).exists())
                {
                    fs::remove_file(root.join("rollback"))?;
                }
                retention.retained.sort();
                retention.retired.sort();
                outcomes.push(retention);
            }
            Ok(())
        })?;
        Ok(outcomes)
    }

    /// Parent of every named account's private directory.
    pub fn accounts_root(&self) -> PathBuf {
        self.auth_base.join("accounts")
    }

    /// The active Machine browser runtime, verified against its signed
    /// generation, or `None` when no such Plugin is installed. Every path is
    /// resolved inside the installed generation; nothing falls back to an
    /// ambient binary.
    pub fn machine_browser_runtime(&self) -> Result<Option<MachineBrowserRuntimePlan>> {
        let mut found: Option<MachineBrowserRuntimePlan> = None;
        for installed in self.inventory()? {
            if installed.plugin_kind != cowboy_plugin_sdk::PluginKind::MachineBrowser
                || installed.state != PluginInstallationState::Active
            {
                continue;
            }
            ensure!(
                found.is_none(),
                "multiple active Machine browser Plugins are installed"
            );
            let (package, _release, content) = self
                .verified_plugin_generation(&installed.plugin_id, &installed.generation_digest)?;
            let contract = package
                .machine_browser()
                .context("installed Plugin is not a Machine browser")?
                .clone();
            let metadata = read_installed_runtime(&content)?;
            ensure!(
                metadata.commands.len() == 1,
                "Machine browser runtime must export exactly one launcher command"
            );
            let (command, installed_command) = metadata
                .commands
                .iter()
                .next()
                .context("Machine browser runtime exports no command")?;
            let launcher = content.join(&installed_command.executable);
            ensure_within(&content, &launcher)?;
            ensure!(
                fs::symlink_metadata(&launcher).is_ok_and(|metadata| metadata.is_file()),
                "Machine browser launcher is missing"
            );
            let runtime_root = strip_path_suffix(&launcher, Path::new(&contract.runtime.launcher))
                .context("Machine browser launcher is not at its contract path")?;
            ensure_within(&content, &runtime_root)?;
            for relative in [
                &contract.runtime.browser,
                &contract.runtime.display_server,
                &contract.runtime.policy.file,
            ]
            .into_iter()
            .chain(contract.runtime.helpers.iter())
            {
                let path = runtime_root.join(relative);
                ensure_within(&runtime_root, &path)?;
                ensure!(
                    fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()),
                    "Machine browser runtime file {relative:?} is missing"
                );
            }
            found = Some(MachineBrowserRuntimePlan {
                plugin_id: installed.plugin_id.clone(),
                plugin_version: installed.plugin_version.clone(),
                generation_digest: installed.generation_digest.clone(),
                command: command.clone(),
                contract,
                runtime_root,
                launcher,
            });
        }
        Ok(found)
    }
}

/// `path` without its trailing `suffix` components, when it ends with them.
fn strip_path_suffix(path: &Path, suffix: &Path) -> Option<PathBuf> {
    let mut base = path.to_path_buf();
    for component in suffix.components().rev() {
        if base.file_name()? != component.as_os_str() {
            return None;
        }
        base.pop();
    }
    Some(base)
}

/// What core needs to start a Machine browser from the active generation of
/// the installed `machine_browser` Plugin (`docs/machine-browser.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineBrowserRuntimePlan {
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
    /// The runtime's single exported command (the launcher).
    pub command: String,
    pub contract: cowboy_plugin_sdk::MachineBrowserContract,
    /// Extracted artifact root: the instance's whole userland (`usr/`, `opt/`).
    pub runtime_root: PathBuf,
    pub launcher: PathBuf,
}
