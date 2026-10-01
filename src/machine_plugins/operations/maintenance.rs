//! Explicit offline recovery under the existing journal owner lock. This is
//! host maintenance, never startup replay or a remote command. Original failed
//! evidence is archived before effects; a new host invocation is the authority.

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

fn sync_removal(plugin: &Path, projection: &Path) -> Result<()> {
    absent(&plugin.join("active"))?;
    for name in ["materialized", "runtime"] {
        absent(&projection.join(name))?;
    }
    // The failed forward step may never have flushed its active-link removal.
    // Persist both deletion boundaries before either completion journal.
    fs::File::open(plugin)?.sync_all()?;
    fs::File::open(projection)?.sync_all()?;
    Ok(())
}

pub(crate) fn complete_absent_uninstall(
    state_dir: &Path,
    service: &str,
    machine: &str,
    operation: &str,
    confirmation: Option<&str>,
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
        matches!(
            original.outcome,
            StepOutcome::Unknown {
                reason: StepUncertainty::EffectFailure
            }
        ),
        "maintenance only accepts a recorded failed uninstall effect"
    );
    let step = &original.step;
    let request_digest = step.request_digest()?;
    journal.install_attempts.ensure_unfenced(&step.plugin_id)?;
    journal.installations.maintenance_removal(step, false)?;
    let plugins = state_dir.join("plugins");
    let plugin = plugins.join(&step.plugin_id);
    let auth = state_dir.join("provider-auth");
    let providers = auth.join("providers");
    let projection = providers.join(&step.plugin_id);
    for path in [&plugins, &auth, &providers, &projection] {
        private_directory(path, uid)?;
    }
    package_directory(&plugin, uid)?;
    absent(&plugin.join("active"))?;
    let mut report = serde_json::json!({
        "schema": 1, "operation_id": operation, "service_id": service,
        "machine_id": machine, "plugin_id": step.plugin_id,
        "request_digest": request_digest, "applied": false,
        "requires_controller_reconciliation": true,
    });
    let Some(confirmation) = confirmation else {
        return Ok(report);
    };
    ensure!(
        confirmation == request_digest,
        "uninstall confirmation digest changed"
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
    for name in ["materialized", "runtime"] {
        checkpoint()?;
        super::super::remove_projection::remove_checked(&projection.join(name), &checkpoint)?;
    }
    sync_removal(&plugin, &projection)?;
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
