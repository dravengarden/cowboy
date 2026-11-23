//! Explicit offline recovery under the existing journal owner lock. This is
//! host maintenance, never startup replay or a remote command. Original failed
//! evidence is archived before effects; a new host invocation is the authority.
//! Recovery only rolls an uncertain uninstall forward to its intended effect;
//! it never claims that an uncertain step did nothing.

use super::*;
use std::os::unix::fs::MetadataExt as _;

#[cfg(test)]
mod tests;

fn private_directory(path: &Path, uid: u32) -> Result<()> {
    let metadata = path.symlink_metadata()?;
    ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode().trailing_zeros() >= 6,
        "maintenance requires private, host-owned directories"
    );
    Ok(())
}

fn absent(path: &Path) -> Result<()> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => bail!("installation is not absent; maintenance refused"),
    }
}

fn package_directory(path: &Path, uid: u32) -> Result<()> {
    // The ordinary installer may create this public package directory at 0755.
    let metadata = path.symlink_metadata()?;
    ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o022 == 0,
        "unsafe Plugin package directory"
    );
    Ok(())
}

fn sync_removal(plugin: &Path, projection: Option<&Path>) -> Result<()> {
    absent(&plugin.join("active"))?;
    if let Some(projection) = projection {
        for name in ["materialized", "runtime"] {
            absent(&projection.join(name))?;
        }
    }
    // The failed forward step may never have flushed its active-link removal.
    // Persist both deletion boundaries before either completion journal.
    fs::File::open(plugin)?.sync_all()?;
    if let Some(projection) = projection {
        fs::File::open(projection)?.sync_all()?;
    }
    Ok(())
}

/// The active link of an uncertain uninstall: absent, or still the exact
/// generation it was removing (the pending intent fences every install).
fn active_is_original(plugin: &Path, step: &UninstallStep) -> Result<bool> {
    match super::active_digest(plugin)? {
        None => Ok(false),
        Some(digest) => {
            ensure!(
                digest == step.generation_digest,
                "active installation changed; maintenance refused"
            );
            Ok(true)
        }
    }
}

/// The original generation when it is still the active link, which only a
/// pending removal may unlink: a stable tombstone is written only after the
/// unlink was flushed, so a present link beside one is contradictory.
fn observed_active(plugin: &Path, step: &UninstallStep, pending: bool) -> Result<Option<String>> {
    let active = active_is_original(plugin, step)?;
    ensure!(
        !active || pending,
        "the active link contradicts the completed removal tombstone; maintenance refused"
    );
    Ok(active.then(|| step.generation_digest.clone()))
}

/// Whether `path` is present (a symlink counts); errors other than absence
/// propagate.
fn present(path: &Path) -> Result<bool> {
    match path.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Finish one uncertain uninstall whose installation slot still holds this
/// request's pending removal (or its matching tombstone after an interrupted
/// pass): run the live uninstall's named-credential refusal check, remove and
/// flush the active link if it is still the original generation, remove the
/// default-scope projections, verify and flush their absence, then complete
/// the tombstone and record the step `applied`. Preview without confirmation.
pub fn complete_absent_uninstall(
    state_dir: &Path,
    service: &str,
    machine: &str,
    operation: &str,
    confirmation: Option<(&str, Option<&str>)>,
) -> Result<serde_json::Value> {
    let started = Instant::now();
    let checkpoint = || -> Result<()> {
        ensure!(
            started.elapsed() < Duration::from_secs(60),
            "maintenance execution budget expired"
        );
        Ok(())
    };
    let uid = rustix::process::geteuid().as_raw();
    private_directory(state_dir, uid)?;
    // Fails while the resident Machine owns the journal. It is never safe to
    // edit its files from a second process with a stale in-memory authority.
    let journal = Journal::open(state_dir)?;
    let original = journal
        .state
        .lock()
        .receipts
        .values()
        .find(|receipt| {
            receipt.step.service_id == service
                && receipt.step.machine_id == machine
                && receipt.step.operation_id == operation
        })
        .cloned()
        .context("exact uninstall receipt not found")?;
    ensure!(
        matches!(original.outcome, StepOutcome::Unknown { .. }),
        "maintenance only accepts an uncertain uninstall receipt"
    );
    let step = &original.step;
    let request_digest = step.request_digest()?;
    journal.install_attempts.ensure_unfenced(&step.plugin_id)?;
    let pending = journal.installations.maintenance_removal(step, false)?;
    let plugins = state_dir.join("plugins");
    let plugin = plugins.join(&step.plugin_id);
    let auth = state_dir.join("provider-auth");
    let providers = auth.join("providers");
    let projection = providers.join(&step.plugin_id);
    private_directory(&plugins, uid)?;
    // A Plugin without credentials (not an Agent Provider) has no projection.
    for path in [&auth, &providers, &projection] {
        if present(path)? {
            private_directory(path, uid)?;
        }
    }
    let projection = present(&projection)?.then_some(projection);
    package_directory(&plugin, uid)?;
    let active_generation = observed_active(&plugin, step, pending)?;
    let active = active_generation.is_some();
    // The live uninstall's own refusal check: named account credentials are
    // account revocation's to remove, never this maintenance's.
    super::super::uninstall::named_credential_refusal(&auth, &step.plugin_id)?;
    let mut report = serde_json::json!({
        "schema": 1, "operation_id": operation, "service_id": service,
        "machine_id": machine, "plugin_id": step.plugin_id,
        "request_digest": request_digest, "applied": false,
        "removes_active_link": active, "active_generation": active_generation,
        "requires_controller_reconciliation": true,
    });
    let Some((confirmation, remove_active)) = confirmation else {
        return Ok(report);
    };
    ensure!(
        confirmation == request_digest,
        "uninstall confirmation digest changed"
    );
    // Deactivating is confirmed separately and only for the exact generation
    // this run observed as the active link.
    ensure!(
        remove_active == active_generation.as_deref(),
        "active link removal must be confirmed separately with exactly the reported active_generation, and only when one is reported"
    );
    checkpoint()?;
    let audit_root = state_dir.join("plugin-maintenance");
    match fs::DirBuilder::new().mode(0o700).create(&audit_root) {
        Ok(()) => fs::File::open(state_dir)?.sync_all()?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    private_directory(&audit_root, uid)?;
    let audit_id = format!("{:032x}", rand::random::<u128>());
    let audit_path = audit_root.join(format!("{audit_id}.json"));
    let mut audit = serde_json::json!({
        "schema": 1, "action": "complete_absent_uninstall", "uid": uid,
        "audit_id": audit_id, "original_receipt": original,
        "started_at_ms": chrono::Utc::now().timestamp_millis(), "completed": false,
    });
    atomic_write(&audit_path, &serde_json::to_vec(&audit)?, 0o600)?;
    fs::File::open(&audit_root)?.sync_all()?;
    let removal = (plugin.as_path(), active);
    roll_forward(removal, projection.as_deref(), &auth, step, &checkpoint)?;
    checkpoint()?;
    // A failure after either write remains recoverable with a new explicit
    // host invocation. The original step is never re-executed.
    journal.installations.maintenance_removal(step, true)?;
    let mut completed = original.clone();
    completed.outcome = StepOutcome::Applied {};
    checkpoint()?;
    journal.persist(&step.key()?, &completed)?;
    audit["completed"] = true.into();
    audit["completed_at_ms"] = chrono::Utc::now().timestamp_millis().into();
    atomic_write(&audit_path, &serde_json::to_vec(&audit)?, 0o600)?;
    fs::File::open(&audit_root)?.sync_all()?;
    report["applied"] = true.into();
    report["audit_id"] = audit_id.into();
    Ok(report)
}

/// Roll the original step forward exactly as the live uninstall does: its
/// refusal check, then the active link (flushed before any credential
/// cleanup), the default scope's projections and empty named private-copy
/// directories, then the refusal check and flushed absence again.
fn roll_forward(
    (plugin, remove_active): (&Path, bool),
    projection: Option<&Path>,
    auth: &Path,
    step: &UninstallStep,
    checkpoint: &dyn Fn() -> Result<()>,
) -> Result<()> {
    checkpoint()?;
    super::super::uninstall::named_credential_refusal(auth, &step.plugin_id)?;
    if active_is_original(plugin, step)? {
        ensure!(
            remove_active,
            "the active link appeared after it was confirmed absent"
        );
        // After both validations: never start removal past the budget.
        checkpoint()?;
        fs::remove_file(plugin.join("active"))?;
    }
    // Also when the link was already absent: the failed forward step may have
    // unlinked it without flushing, and no credential deletion may persist
    // ahead of that unlink.
    fs::File::open(plugin)?.sync_all()?;
    if let Some(projection) = projection {
        for name in [
            "materialized",
            "runtime",
            super::super::native_usage::PRIVATE_HOMES,
        ] {
            checkpoint()?;
            super::super::remove_projection::remove_checked(&projection.join(name), &checkpoint)?;
        }
    }
    // Only empty private-copy directories remain (the refusal check above).
    remove_named_private_homes(auth, &step.plugin_id, checkpoint)?;
    super::super::uninstall::named_credential_refusal(auth, &step.plugin_id)?;
    sync_removal(plugin, projection)
}

/// Every named account's private credential copies of `plugin_id`, under the
/// maintenance deadline; completion is written only once none remain.
fn remove_named_private_homes(
    auth: &Path,
    plugin_id: &str,
    checkpoint: &dyn Fn() -> Result<()>,
) -> Result<()> {
    let named = (super::super::native_usage::PRIVATE_HOMES, Some(plugin_id));
    let account = |name: &str| {
        crate::provider_account::ProviderAccountId::try_from(name.to_owned())
            .is_ok_and(|id| !id.is_default())
    };
    super::super::remove_projection::remove_below_named_providers(
        auth, &account, named, checkpoint,
    )?;
    super::super::remove_projection::ensure_absent_below_named_providers(auth, &account, named)
}
