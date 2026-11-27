//! Retirement of Plugin generations nothing can launch any more, shared by
//! the per-Plugin pass and the sweep of uninstalled Plugins.

use super::*;
use std::os::fd::AsFd as _;

pub(super) fn is_retainable_non_browser_kind(kind: cowboy_plugin_sdk::PluginKind) -> bool {
    matches!(
        kind,
        cowboy_plugin_sdk::PluginKind::AgentProvider
            | cowboy_plugin_sdk::PluginKind::CodeIntelligence
            | cowboy_plugin_sdk::PluginKind::MemoryBackend
            | cowboy_plugin_sdk::PluginKind::CallProfile
    )
}

/// Strictly validate the entire tombstone directory name: either
/// `.retired-<64 lower hex>` or `.retired-<64 lower hex>-<digits>-<digits>`.
pub(super) fn is_retired_tombstone_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(RETIRED_GENERATION_PREFIX) else {
        return false;
    };
    if is_generation_name(rest) {
        return true;
    }
    let Some((digest_part, suffix)) = rest.split_once('-') else {
        return false;
    };
    if !is_generation_name(digest_part) {
        return false;
    }
    let Some((pid, seq)) = suffix.split_once('-') else {
        return false;
    };
    !pid.is_empty()
        && !seq.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && seq.bytes().all(|b| b.is_ascii_digit())
}

/// Parse explicit `plugin_kind` from `plugin-inventory.json` bound to
/// `(plugin_id, expected_digest)`, refusing records that omit `plugin_kind`
/// (which would otherwise silently default to `AgentProvider`) or disagree on
/// `plugin_id` / `generation_digest`.
fn explicit_inventory_kind(
    inventory_path: &Path,
    plugin_id: &str,
    expected_digest: &str,
) -> Option<cowboy_plugin_sdk::PluginKind> {
    let bytes = fs::read(inventory_path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let obj = value.as_object()?;
    if obj.get("plugin_id")?.as_str()? != plugin_id {
        return None;
    }
    if !obj
        .get("generation_digest")?
        .as_str()?
        .eq_ignore_ascii_case(expected_digest)
    {
        return None;
    }
    let kind_value = obj.get("plugin_kind")?;
    serde_json::from_value::<cowboy_plugin_sdk::PluginKind>(kind_value.clone()).ok()
}

/// Classify a single generation directory from its verified signed descriptor
/// bound to `(plugin_id, gen_name)`, never letting an unsigned or defaulted
/// `plugin-inventory.json` override the signed package or send a generation to
/// the wrong sweeper. Returns `None` if records conflict or cannot be verified.
pub(super) fn classify_generation_kind(
    plugin_id: &str,
    gen_name: &str,
    generation_dir: &Path,
) -> Option<cowboy_plugin_sdk::PluginKind> {
    let content = generation_dir.join("content");
    let expected_digest = format!("sha256:{gen_name}");
    let inventory_path = content.join("plugin-inventory.json");
    let has_inventory = inventory_path.symlink_metadata().is_ok();

    let plugin_pkg_path = content.join("package.cowboy-plugin");
    if plugin_pkg_path.symlink_metadata().is_ok() {
        let pkg_bytes = fs::read(&plugin_pkg_path).ok()?;
        let release: cowboy_plugin_sdk::PluginRelease =
            serde_json::from_slice(&fs::read(content.join("plugin-release.json")).ok()?).ok()?;
        if !release
            .artifact_digest
            .eq_ignore_ascii_case(&expected_digest)
        {
            return None;
        }
        let package = release.validate_bytes(&pkg_bytes).ok()?;
        if package.manifest.id != plugin_id {
            return None;
        }
        let publisher_key = fs::read_to_string(content.join("publisher.pub")).ok()?;
        if !crate::machine_auth::verify_namespaced(
            &publisher_key,
            PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
            &release.proof(),
            &release.signature,
        )
        .ok()?
        {
            return None;
        }
        if has_inventory
            && explicit_inventory_kind(&inventory_path, plugin_id, &expected_digest)
                != Some(package.manifest.kind)
        {
            return None;
        }
        return Some(package.manifest.kind);
    }

    let legacy_pkg_path = content.join("package.cowboy-provider");
    if legacy_pkg_path.symlink_metadata().is_ok() {
        let pkg_bytes = fs::read(&legacy_pkg_path).ok()?;
        let release: LegacyProviderRelease =
            serde_json::from_slice(&fs::read(content.join("release.json")).ok()?).ok()?;
        if !release
            .artifact_digest
            .eq_ignore_ascii_case(&expected_digest)
        {
            return None;
        }
        if !format!("sha256:{:x}", Sha256::digest(&pkg_bytes))
            .eq_ignore_ascii_case(&expected_digest)
        {
            return None;
        }
        let package: ProviderPackage = serde_json::from_slice(&pkg_bytes).ok()?;
        if package.manifest.id != plugin_id {
            return None;
        }
        let publisher_key = fs::read_to_string(content.join("publisher.pub")).ok()?;
        if !crate::machine_auth::verify_namespaced(
            &publisher_key,
            LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
            &release.proof().ok()?,
            &release.signature,
        )
        .ok()?
        {
            return None;
        }
        if has_inventory
            && explicit_inventory_kind(&inventory_path, plugin_id, &expected_digest)
                != Some(cowboy_plugin_sdk::PluginKind::AgentProvider)
        {
            return None;
        }
        return Some(cowboy_plugin_sdk::PluginKind::AgentProvider);
    }

    if has_inventory {
        return explicit_inventory_kind(&inventory_path, plugin_id, &expected_digest);
    }
    None
}

/// Atomically rename `generations/name` to a fresh non-colliding tombstone
/// inside `generations` using `RENAME_NOREPLACE` (Linux) or existence check
/// plus retry on `EEXIST`/`ENOTEMPTY`.
fn rename_to_unique_tombstone(generations: &Path, name: &str) -> Result<PathBuf> {
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(generations)
        .with_context(|| format!("opening {}", generations.display()))?;
    for attempt in 0..32_u32 {
        let candidate = if attempt == 0 {
            format!("{RETIRED_GENERATION_PREFIX}{name}")
        } else {
            format!(
                "{RETIRED_GENERATION_PREFIX}{name}-{}-{}",
                std::process::id(),
                ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            )
        };
        #[cfg(target_os = "linux")]
        let outcome = rustix::fs::renameat_with(
            dir.as_fd(),
            name,
            dir.as_fd(),
            candidate.as_str(),
            rustix::fs::RenameFlags::NOREPLACE,
        );
        #[cfg(not(target_os = "linux"))]
        let outcome = if generations.join(&candidate).symlink_metadata().is_ok() {
            Err(rustix::io::Errno::EXIST)
        } else {
            rustix::fs::renameat(dir.as_fd(), name, dir.as_fd(), candidate.as_str())
        };
        match outcome {
            Ok(()) => return Ok(generations.join(candidate)),
            Err(rustix::io::Errno::EXIST | rustix::io::Errno::NOTEMPTY) => {}
            Err(error) => {
                return Err(error).with_context(|| format!("retiring generation {name}"));
            }
        }
    }
    bail!("could not allocate unique tombstone name for generation {name}")
}

impl MachinePluginStore {
    /// Phase 1 (fast, safe to run under pin locks): collect any existing
    /// validated `.retired-*` tombstones and rename every unkept generation
    /// under `generations` whose per-generation verified kind matches
    /// `allow_kind` to a unique tombstone path, syncing `generations`.
    pub(super) fn tombstone_unkept_matching(
        &self,
        plugin_id: &str,
        generations: &Path,
        keep: &BTreeSet<String>,
        allow_kind: impl Fn(cowboy_plugin_sdk::PluginKind) -> bool,
        retention: &mut crate::generation_retention::Outcome,
    ) -> Result<Vec<PathBuf>> {
        let mut tombstones = Vec::new();
        for entry in fs::read_dir(generations)
            .with_context(|| format!("reading {}", generations.display()))?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(RETIRED_GENERATION_PREFIX) {
                ensure!(
                    is_retired_tombstone_name(&name),
                    "invalid retired generation"
                );
                tombstones.push(entry.path());
            } else if !is_generation_name(&name) || !entry.file_type()?.is_dir() {
            } else if keep.contains(&name)
                || !classify_generation_kind(plugin_id, &name, &entry.path())
                    .is_some_and(&allow_kind)
            {
                retention.retained.push(format!("sha256:{name}"));
            } else {
                let tombstone = rename_to_unique_tombstone(generations, &name)?;
                retention.retired.push(format!("sha256:{name}"));
                tombstones.push(tombstone);
            }
        }
        if !retention.retired.is_empty() {
            fs::File::open(generations)?.sync_all()?;
            // Proof keys are "<content root>\n<artifact set>".
            self.runtime_proofs.lock().retain(|key, _| {
                key.split_once('\n')
                    .is_none_or(|(content, _)| Path::new(content).exists())
            });
        }
        Ok(tombstones)
    }

    /// Finish any already-validated `.retired-*` tombstones in `generations`,
    /// regardless of whether the slot is active or currently holds an excluded
    /// Plugin kind (e.g. `TelemetryBackend`).
    fn sweep_existing_tombstones(generations: &Path) -> Result<()> {
        let mut tombstones = Vec::new();
        for entry in fs::read_dir(generations)
            .with_context(|| format!("reading {}", generations.display()))?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_retired_tombstone_name(&name) {
                tombstones.push(entry.path());
            }
        }
        if tombstones.is_empty() {
            return Ok(());
        }
        let mut ignored = crate::generation_retention::Outcome::default();
        Self::remove_tombstones(&tombstones, &mut ignored)?;
        fs::File::open(generations)?.sync_all()?;
        Ok(())
    }

    /// Phase 2 (slow recursive disk removal, run outside Code/browser pin locks).
    pub(super) fn remove_tombstones(
        tombstones: &[PathBuf],
        retention: &mut crate::generation_retention::Outcome,
    ) -> Result<()> {
        for tombstone in tombstones {
            retention.freed_bytes = retention.freed_bytes.saturating_add(tree_bytes(tombstone));
            remove_projection::remove(tombstone)
                .with_context(|| format!("removing {}", tombstone.display()))?;
        }
        Ok(())
    }

    /// Sweep abandoned `.runtime.*` partial staging directories (safe because
    /// the caller holds `self.lifecycle`) and retire aged unreferenced blobs
    /// from `artifact-cache`.
    pub(super) fn retire_artifact_cache_locked(&self) {
        if let Err(error) = artifact_cache::sweep_abandoned_runtime_staging(&self.root) {
            tracing::warn!(%error, "sweeping abandoned runtime staging failed");
        }
        if let (Some(config), Some(state)) = (self.device_config.get(), self.root.parent()) {
            let unreferenced_after =
                config.get(&crate::config::schema::DEVICE_ARTIFACT_CACHE_UNREFERENCED_AFTER);
            match artifact_cache::retire_unreferenced(state, &self.root, unreferenced_after) {
                Ok(retired) if retired.files > 0 => tracing::info!(
                    files = retired.files,
                    freed_bytes = retired.bytes,
                    "retired unreferenced runtime artifact cache entries"
                ),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "artifact cache retention failed"),
            }
        }
    }

    /// Retire generations of Plugins that are no longer installed, and finish
    /// any stranded `.retired-*` tombstones across all unfenced slots (even if
    /// reinstalled as an excluded kind). Also runs Machine browser retirement.
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
        let root = self.root.clone();
        let entries = tokio::task::spawn_blocking(move || read_store_plugin_dirs(&root))
            .await
            .context("uninstalled retention scan task failed")??;
        for (plugin_id, root) in entries {
            let generations = root.join("generations");
            if skip.contains(&plugin_id)
                || validate_plugin_id(&plugin_id).is_err()
                || !generations.is_dir()
                || self.operations.ensure_unfenced(&plugin_id).is_err()
            {
                continue;
            }
            if root.join("active").symlink_metadata().is_ok() {
                // Slot is active (possibly an excluded kind like Telemetry);
                // finish any stranded tombstones without touching live gens.
                let gens = generations.clone();
                tokio::task::spawn_blocking(move || Self::sweep_existing_tombstones(&gens))
                    .await
                    .context("tombstone sweep task failed")??;
                continue;
            }
            let mut keep: BTreeSet<String> = referenced
                .get(&plugin_id)
                .into_iter()
                .flatten()
                .map(|digest| digest.trim_start_matches("sha256:").to_owned())
                .collect();
            let mut retention = crate::generation_retention::Outcome {
                plugin_id: plugin_id.clone(),
                ..crate::generation_retention::Outcome::default()
            };
            let tombstones = self
                .code_runtimes
                .with_generation_pins(&plugin_id, |code_pins| {
                    let _profiles = self.auth_candidates.lock();
                    keep.extend(self.retained_native_generations(&plugin_id)?);
                    keep.extend(
                        code_pins
                            .iter()
                            .map(|digest| digest.trim_start_matches("sha256:").to_owned()),
                    );
                    self.tombstone_unkept_matching(
                        &plugin_id,
                        &generations,
                        &keep,
                        is_retainable_non_browser_kind,
                        &mut retention,
                    )
                })
                .await?;
            let root_for_remove = root.clone();
            let gens_for_remove = generations.clone();
            let mut retention = tokio::task::spawn_blocking(
                move || -> Result<crate::generation_retention::Outcome> {
                    Self::remove_tombstones(&tombstones, &mut retention)?;
                    if read_link_name(&root_for_remove.join("rollback"))
                        .is_some_and(|name| !gens_for_remove.join(&name).exists())
                    {
                        fs::remove_file(root_for_remove.join("rollback"))?;
                    }
                    Ok(retention)
                },
            )
            .await
            .context("uninstalled retention removal task failed")??;
            if !retention.retired.is_empty() {
                retention.retained.sort();
                retention.retired.sort();
                outcomes.push(retention);
            }
        }
        let store = self.clone();
        let skip_owned = skip.clone();
        let browser_outcomes = tokio::task::spawn_blocking(move || {
            store.retire_browser_and_cache_blocking(&skip_owned)
        })
        .await
        .context("browser and cache retention task failed")?;
        outcomes.extend(browser_outcomes);
        Ok(outcomes)
    }

    fn retire_browser_and_cache_blocking(
        &self,
        skip: &BTreeSet<String>,
    ) -> Vec<crate::generation_retention::Outcome> {
        let browsers = match self.retire_browser_generations_locked(None, skip) {
            Ok(browsers) => browsers
                .into_iter()
                .filter(|retention| !retention.retired.is_empty())
                .collect(),
            Err(error) => {
                tracing::warn!(%error, "Machine browser generation retention skipped");
                Vec::new()
            }
        };
        self.retire_artifact_cache_locked();
        browsers
    }

    pub(super) fn retire_auth_generations_blocking(
        &self,
        provider_id: &str,
        referenced: BTreeSet<u64>,
        grace: Duration,
    ) -> Result<AuthRetention> {
        let mut outcome = AuthRetention {
            provider_id: provider_id.to_owned(),
            ..AuthRetention::default()
        };
        let root = self.auth_provider_root(provider_id);
        let runtime = root.join("runtime/generations");
        let materialized = root.join("materialized/generations");
        let Some(runtime_current) = read_link_name(&root.join("runtime/current"))
            .and_then(|name| name.parse::<u64>().ok())
            .filter(|generation| runtime.join(generation.to_string()).is_dir())
        else {
            return Ok(outcome);
        };
        let mut keep = referenced;
        keep.insert(runtime_current);
        keep.extend(
            read_link_name(&root.join("materialized/current"))
                .and_then(|name| name.parse::<u64>().ok()),
        );
        if let Some(envelope) = self.latest_auth_envelope(provider_id)? {
            keep.insert(envelope.auth_generation);
        }
        let now = SystemTime::now();
        let mut candidates = BTreeSet::new();
        for directory in [&runtime, &materialized] {
            if !directory.is_dir() {
                continue;
            }
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some(interrupted) = name.strip_prefix(RETIRED_GENERATION_PREFIX) {
                    ensure!(
                        interrupted.parse::<u64>().is_ok(),
                        "invalid retired credential generation"
                    );
                    remove_projection::remove(&entry.path())?;
                    continue;
                }
                let Ok(generation) = name.parse::<u64>() else {
                    continue;
                };
                if generation == 0 || generation.to_string() != name || !entry.file_type()?.is_dir()
                {
                    continue;
                }
                let metadata = entry.metadata()?;
                let changed_at = SystemTime::UNIX_EPOCH
                    + Duration::from_secs(
                        u64::try_from(std::os::unix::fs::MetadataExt::ctime(&metadata))
                            .unwrap_or_default(),
                    );
                let changed = metadata.modified().unwrap_or(now).max(changed_at);
                if now.duration_since(changed).unwrap_or_default() < grace {
                    keep.insert(generation);
                }
                candidates.insert(generation);
            }
        }
        // The two newest generations cover a launch admitted on the previous
        // current generation while a refresh advanced the Machine after the
        // Controller's snapshot.
        keep.extend(candidates.iter().rev().take(2).copied());
        keep.extend(live_auth_homes(&runtime));
        for generation in candidates {
            if keep.contains(&generation) {
                outcome.retained.push(generation);
                continue;
            }
            // Runtime first: a materialization without its runtime copy is
            // still a valid immutable source, never the reverse.
            for directory in [&runtime, &materialized] {
                let path = directory.join(generation.to_string());
                if !path.exists() {
                    continue;
                }
                let tombstone = directory.join(format!("{RETIRED_GENERATION_PREFIX}{generation}"));
                fs::rename(&path, &tombstone).with_context(|| {
                    format!("retiring credential generation {}", path.display())
                })?;
                outcome.freed_bytes = outcome.freed_bytes.saturating_add(tree_bytes(&tombstone));
                remove_projection::remove(&tombstone)
                    .with_context(|| format!("removing {}", tombstone.display()))?;
            }
            outcome.retired.push(generation);
        }
        for directory in [&runtime, &materialized] {
            if !outcome.retired.is_empty() && directory.is_dir() {
                fs::File::open(directory)?.sync_all()?;
            }
        }
        Ok(outcome)
    }
}

fn read_store_plugin_dirs(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(root).context("reading Provider store")? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        found.push((
            entry.file_name().to_string_lossy().into_owned(),
            entry.path(),
        ));
    }
    Ok(found)
}

/// Reclaimable allocated disk bytes under `path` (POSIX `st_blocks * 512`),
/// including directory blocks and without following symlinks.
pub(super) fn tree_bytes(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    fn collect(path: &Path, seen: &mut std::collections::HashMap<(u64, u64), (u64, u64, u64)>) {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return;
        };
        let allocated = metadata.blocks().saturating_mul(512);
        if metadata.is_dir() {
            seen.entry((metadata.dev(), metadata.ino()))
                .or_insert((1, allocated, 1));
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    collect(&entry.path(), seen);
                }
            }
            return;
        }
        let entry = seen.entry((metadata.dev(), metadata.ino())).or_insert((
            metadata.nlink(),
            allocated,
            0,
        ));
        entry.2 = entry.2.saturating_add(1);
    }
    let mut seen = std::collections::HashMap::new();
    collect(path, &mut seen);
    seen.into_values()
        .filter(|(nlink, _, count)| count >= nlink)
        .map(|(_, allocated, _)| allocated)
        .fold(0, u64::saturating_add)
}

#[cfg(test)]
mod tests;
