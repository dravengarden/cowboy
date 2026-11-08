//! The installed Machine browser Plugin as core launches it: its verified
//! runtime plan and the account root its instances are isolated under.
use super::*;

/// The Machine browser manager as the Plugin store reaches it: uninstalling
/// the browser Plugin first drains every running instance through it. The
/// manager registers itself at construction, so the store never depends on
/// the browser module.
pub trait BrowserDrain: Send + Sync {
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
