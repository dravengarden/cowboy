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
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<Option<Box<dyn std::any::Any + Send>>> {
        let store = self.clone();
        let pid = plugin_id.to_owned();
        let is_browser =
            Self::run_with_lifecycle(lifecycle, "browser plugin check task failed", move || {
                store.is_browser_plugin_checked(&pid)
            })
            .await?;
        if !is_browser {
            return Ok(None);
        }
        let Some(drain) = self
            .browser_drain
            .lock()
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        else {
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
        lifecycle: &mut Option<LifecycleGuard>,
    ) -> Result<Option<Box<dyn std::any::Any + Send>>> {
        let store = self.clone();
        let pid = plugin_id.to_owned();
        if let Some(active) = Self::run_with_lifecycle(
            lifecycle,
            "uninstall inventory check task failed",
            move || store.inventory_one(&pid),
        )
        .await?
        {
            ensure!(
                active.generation_digest == expected_digest,
                "active Plugin generation changed; refresh the uninstall plan"
            );
        }
        self.drain_browsers(plugin_id, &|| Ok(()), lifecycle).await
    }

    /// Strictly classify whether an active Plugin is a Machine browser from its
    /// signed descriptor; unreadable or inconsistent active slots fail closed.
    pub(super) fn is_browser_plugin_checked(&self, plugin_id: &str) -> Result<bool> {
        validate_plugin_id(plugin_id)?;
        let active = self.plugin_root(plugin_id).join("active");
        let Some(generation) = read_link_name(&active) else {
            ensure!(
                active.symlink_metadata().is_err(),
                "invalid active link for Plugin {plugin_id}"
            );
            return Ok(false);
        };
        let gen_dir = self
            .plugin_root(plugin_id)
            .join("generations")
            .join(&generation);
        let kind = retirement::classify_generation_kind(plugin_id, &generation, &gen_dir)
            .with_context(|| format!("cannot classify active generation for Plugin {plugin_id}"))?;
        Ok(kind == cowboy_plugin_sdk::PluginKind::MachineBrowser)
    }

    pub(super) fn is_browser_plugin(&self, plugin_id: &str) -> bool {
        self.is_browser_plugin_checked(plugin_id).unwrap_or(false)
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
            // Durable before any credential cleanup, so an active link that
            // is still present after a crash proves no effect happened.
            fs::File::open(&plugin_root)?.sync_all()?;
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
        // Private credential copies: this scope's, then every named account's
        // (the guard proved them empty). The uninstall completes only once
        // all are gone.
        remove_projection::remove_below_provider(
            &self.auth_root,
            provider_id,
            native_usage::PRIVATE_HOMES,
        )
        .context("removing private credential copies")?;
        let named = (native_usage::PRIVATE_HOMES, Some(provider_id));
        remove_projection::remove_below_named_providers(&self.auth_base, &|_| true, named, &|| {
            Ok(())
        })
        .context("removing named private credential copies")?;
        remove_projection::ensure_absent_below_named_providers(&self.auth_base, &|_| true, named)
    }

    /// Refuse while a named account holds `provider_id` credentials, or while
    /// any credential tree the uninstall cleans is malformed (see
    /// [`named_credential_refusal`]).
    pub(super) fn ensure_no_named_auth_materializations(&self, provider_id: &str) -> Result<()> {
        named_credential_refusal(&self.auth_base, provider_id)
    }
}

/// Login candidate homes below a Provider's credential directory.
const CANDIDATES: &str = "candidates";

/// The uninstall's refusal check over a Machine's `provider-auth` base:
/// refuse while a named account holds a `provider_id` projection (materialized,
/// runtime, a private copy) or an unrevoked native profile, or while a
/// credential tree the uninstall cleans is malformed. It uses the cleanup's own
/// no-follow descriptor traversal and reads only, so a journaled uninstall can
/// refuse definitely before its intent, and offline completion runs the same
/// check. Named credentials are never an uninstall's to delete, and neither
/// is an outstanding login candidate home (default scope or named).
pub(super) fn named_credential_refusal(auth: &Path, provider_id: &str) -> Result<()> {
    remove_projection::ensure_provider_traversable(auth, provider_id)?;
    // An outstanding login candidate home may hold plaintext until its own
    // finalization or discard, which needs the installed Provider.
    remove_projection::ensure_empty_below_provider(auth, provider_id, CANDIDATES)?;
    for (leaf, empty_directory) in [
        ("materialized", false),
        ("runtime", false),
        (native_usage::PRIVATE_HOMES, true),
        (CANDIDATES, true),
    ] {
        remove_projection::ensure_clear_below_named_providers(
            auth,
            provider_id,
            leaf,
            empty_directory,
        )?;
    }
    let accounts = auth.join("accounts");
    if !accounts.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(accounts)? {
        let entry = entry?;
        // The descriptor walks above classify accounts by name; an
        // undecodable one is a malformed tree, never skipped.
        ensure!(
            entry.file_type()?.is_dir() && entry.file_name().to_str().is_some(),
            "invalid Provider account directory"
        );
        native_profile::ensure_revoked(
            &entry
                .path()
                .join("providers")
                .join(provider_id)
                .join("native"),
            &entry.file_name().to_string_lossy(),
            provider_id,
        )?;
    }
    Ok(())
}
