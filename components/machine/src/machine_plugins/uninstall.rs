//! Uninstalling a Plugin's active installation, and the Machine browser drain
//! that must complete before a browser Plugin's generation is unlinked.

use super::*;

impl MachinePluginStore {
    /// Drain every running Machine browser instance before `plugin_id`'s
    /// active generation is removed, when it is the Machine browser Plugin.
    /// The returned hold refuses new browser opens until it is dropped (after
    /// the uninstall finished or failed). `authority` is re-checked before
    /// every individual stop.
    ///
    /// # Errors
    /// Returns when a running instance could not be confirmed stopped or the
    /// authority ended; the uninstall must not proceed.
    pub(super) async fn drain_browsers(
        &self,
        plugin_id: &str,
        authority: &machine_browser::DrainAuthority<'_>,
    ) -> Result<Option<Box<dyn std::any::Any + Send>>> {
        if !self.is_browser_plugin(plugin_id) {
            return Ok(None);
        }
        let Some(drain) = self
            .browser_drain
            .lock()
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        else {
            // No browser manager runs in this process: nothing to drain.
            return Ok(None);
        };
        drain
            .drain(authority)
            .await
            .map(Some)
            .context("draining Machine browser instances before uninstall")
    }

    /// The local (Machine-owner) uninstall's drain: its own authority holds
    /// throughout, but the exact target is checked before any browser stops.
    ///
    /// # Errors
    /// Returns when the active generation changed or the drain failed.
    pub(super) async fn drain_local(
        &self,
        plugin_id: &str,
        expected_digest: &str,
    ) -> Result<Option<Box<dyn std::any::Any + Send>>> {
        if let Some(active) = self.inventory_one(plugin_id)? {
            ensure!(
                active.generation_digest == expected_digest,
                "active Plugin generation changed; refresh the uninstall plan"
            );
        }
        self.drain_browsers(plugin_id, &|| Ok(())).await
    }

    /// Whether `plugin_id`'s active installation is a Machine browser.
    pub(super) fn is_browser_plugin(&self, plugin_id: &str) -> bool {
        self.inventory_one(plugin_id)
            .ok()
            .flatten()
            .is_some_and(|installed| {
                installed.plugin_kind == cowboy_plugin_sdk::PluginKind::MachineBrowser
            })
    }

    pub(super) fn uninstall_inner(
        &self,
        provider_id: &str,
        expected_digest: &str,
        before_mutation: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        validate_plugin_id(provider_id)?;
        let active = self
            .inventory_one(provider_id)?
            .context("Plugin is not installed")?;
        ensure!(
            active.generation_digest == expected_digest,
            "active Plugin generation changed; refresh the uninstall plan"
        );
        // Until account-aware lifecycle receipts are available, refuse before
        // changing the active installation instead of leaving another account's
        // plaintext credentials behind a successful uninstall receipt.
        self.ensure_no_named_auth_materializations(provider_id)?;
        let plugin_root = self.plugin_root(provider_id);
        let active_link = plugin_root.join("active");
        let remove_active = active_link.exists() || active_link.symlink_metadata().is_ok();
        // Inventory may re-verify retained bytes/auth materialization. Even this
        // last validation and filesystem lookup consume the original lease.
        before_mutation()?;
        if remove_active {
            fs::remove_file(&active_link)
                .with_context(|| format!("removing {}", active_link.display()))?;
        }
        if active.plugin_kind != cowboy_plugin_sdk::PluginKind::AgentProvider {
            return Ok(());
        }
        let materialized = self.auth_provider_root(provider_id).join("materialized");
        remove_projection::remove(&materialized).with_context(|| {
            format!(
                "removing Provider credential projection {}",
                materialized.display()
            )
        })?;
        let runtime = self.auth_provider_root(provider_id).join("runtime");
        remove_projection::remove(&runtime).with_context(|| {
            format!(
                "removing Provider runtime credential projections {}",
                runtime.display()
            )
        })?;
        Ok(())
    }

    pub(super) fn ensure_no_named_auth_materializations(&self, provider_id: &str) -> Result<()> {
        let accounts = self.auth_base.join("accounts");
        if !accounts.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(accounts)? {
            let entry = entry?;
            ensure!(
                entry.file_type()?.is_dir(),
                "invalid Provider account directory"
            );
            let provider = entry.path().join("providers").join(provider_id);
            ensure!(
                !provider.join("materialized").try_exists()?
                    && !provider.join("runtime").try_exists()?,
                "Revoke named Provider account projections before uninstalling this Plugin"
            );
            native_profile::ensure_revoked(
                &provider.join("native"),
                &entry.file_name().to_string_lossy(),
                provider_id,
            )?;
        }
        Ok(())
    }
}
