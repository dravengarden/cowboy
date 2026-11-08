//! Retirement of Plugin generations nothing can launch any more, shared by
//! the per-Plugin pass and the sweep of uninstalled Plugins.

use super::*;

impl MachinePluginStore {
    /// Tombstone and remove every generation under `generations` not in
    /// `keep`, finishing tombstones an interrupted pass left behind.
    pub(super) fn retire_unkept(
        &self,
        generations: &Path,
        keep: &BTreeSet<String>,
        retention: &mut crate::generation_retention::Outcome,
    ) -> Result<()> {
        for entry in fs::read_dir(generations)
            .with_context(|| format!("reading {}", generations.display()))?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let tombstone = if let Some(interrupted) = name.strip_prefix(RETIRED_GENERATION_PREFIX)
            {
                ensure!(
                    is_generation_name(interrupted),
                    "invalid retired generation"
                );
                entry.path()
            } else if !is_generation_name(&name) || !entry.file_type()?.is_dir() {
                continue;
            } else if keep.contains(&name) {
                retention.retained.push(format!("sha256:{name}"));
                continue;
            } else {
                let tombstone = generations.join(format!("{RETIRED_GENERATION_PREFIX}{name}"));
                fs::rename(entry.path(), &tombstone)
                    .with_context(|| format!("retiring generation {name}"))?;
                retention.retired.push(format!("sha256:{name}"));
                tombstone
            };
            retention.freed_bytes = retention.freed_bytes.saturating_add(tree_bytes(&tombstone));
            remove_projection::remove(&tombstone)
                .with_context(|| format!("removing {}", tombstone.display()))?;
        }
        if !retention.retired.is_empty() {
            fs::File::open(generations)?.sync_all()?;
            // Proof keys are "<content root>\n<artifact set>".
            self.runtime_proofs.lock().retain(|key, _| {
                key.split_once('\n')
                    .is_none_or(|(content, _)| Path::new(content).exists())
            });
        }
        Ok(())
    }

    /// Retire generations of Agent Provider Plugins that are no longer
    /// installed: their active link is gone, so only sessions still pinning a
    /// generation (`referenced`, including soft-deleted sessions until purge)
    /// keep it. `skip` names Plugins whose uninstall saga may still compensate
    /// by reactivating the removed generation; they are left untouched.
    ///
    /// The same periodic pass retires Machine browser runtime generations,
    /// installed or not, that no browser instance pins (the Controller holds
    /// no references for them); unknown pins skip only that part.
    pub async fn retire_uninstalled_generations(
        &self,
        referenced: &BTreeMap<String, BTreeSet<String>>,
        skip: &BTreeSet<String>,
    ) -> Result<Vec<crate::generation_retention::Outcome>> {
        let _lifecycle = self.lifecycle.lock().await;
        let mut outcomes = Vec::new();
        if !self.root.exists() {
            return Ok(outcomes);
        }
        for entry in fs::read_dir(&self.root).context("reading Provider store")? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let plugin_id = entry.file_name().to_string_lossy().into_owned();
            let root = entry.path();
            let generations = root.join("generations");
            if skip.contains(&plugin_id)
                || validate_plugin_id(&plugin_id).is_err()
                || root.join("active").symlink_metadata().is_ok()
                || !generations.is_dir()
                // Code runtimes own leases independent of sessions.
                || root.join(".code-runtime-owned-v2").exists()
                || !uninstalled_generations_of(
                    &generations,
                    cowboy_plugin_sdk::PluginKind::AgentProvider,
                )
                || self.operations.ensure_unfenced(&plugin_id).is_err()
            {
                continue;
            }
            let mut keep: BTreeSet<String> = referenced
                .get(&plugin_id)
                .into_iter()
                .flatten()
                .map(|digest| digest.trim_start_matches("sha256:").to_owned())
                .collect();
            let _profiles = self.auth_candidates.lock();
            keep.extend(self.retained_native_generations(&plugin_id)?);
            let mut retention = crate::generation_retention::Outcome {
                plugin_id: plugin_id.clone(),
                ..crate::generation_retention::Outcome::default()
            };
            self.retire_unkept(&generations, &keep, &mut retention)?;
            // A rollback link has nothing to roll back to once uninstalled.
            if read_link_name(&root.join("rollback"))
                .is_some_and(|name| !generations.join(&name).exists())
            {
                fs::remove_file(root.join("rollback"))?;
            }
            if !retention.retired.is_empty() {
                retention.retained.sort();
                retention.retired.sort();
                outcomes.push(retention);
            }
        }
        match self.retire_browser_generations_locked(None, skip) {
            Ok(browsers) => outcomes.extend(
                browsers
                    .into_iter()
                    .filter(|retention| !retention.retired.is_empty()),
            ),
            Err(error) => {
                tracing::warn!(%error, "Machine browser generation retention skipped");
            }
        }
        Ok(outcomes)
    }
}

/// Whether an uninstalled Plugin's generations are recorded installations
/// of `kind`. Unknown or other kinds are never retired by inference.
pub(super) fn uninstalled_generations_of(
    generations: &Path,
    kind: cowboy_plugin_sdk::PluginKind,
) -> bool {
    let Ok(entries) = fs::read_dir(generations) else {
        return false;
    };
    let mut recorded = false;
    for entry in entries.flatten() {
        let path = entry.path().join("content/plugin-inventory.json");
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        match serde_json::from_slice::<PluginInventory>(&bytes) {
            Ok(inventory) if inventory.plugin_kind == kind => {
                recorded = true;
            }
            _ => return false,
        }
    }
    recorded
}

#[cfg(test)]
mod tests;
