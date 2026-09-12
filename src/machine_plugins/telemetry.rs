//! One connection-owned telemetry call. This is admission of bounded external
//! emission, not a durable binding, a replay grant or an undo implementation.

use super::*;
use std::os::unix::fs::MetadataExt as _;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const ADMISSION_BUDGET: Duration = Duration::from_secs(15);

pub(crate) mod managed;

pub(crate) struct PluginHostRequest {
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
    pub auth_generation: Option<u64>,
    pub operation: PluginHostOperation,
    pub payload: serde_json::Value,
}

// Only the connection owner constructs this. No Clone/Deserialize or caller-
// supplied deadline; a caller cannot substitute another request after capture.
pub(crate) struct PluginHostInvocation {
    request: PluginHostRequest,
    telemetry: Option<TelemetryExecutionLease>,
}

impl PluginHostInvocation {
    pub(in crate::machine_plugins) fn new(
        request: PluginHostRequest,
        connected: Arc<AtomicBool>,
    ) -> Self {
        let telemetry = matches!(
            request.operation,
            PluginHostOperation::ExportTelemetry | PluginHostOperation::ExportOtlp
        )
        .then(|| TelemetryExecutionLease {
            connected,
            deadline: Instant::now() + ADMISSION_BUDGET,
            retired: AtomicBool::new(false),
        });
        Self { request, telemetry }
    }

    pub(super) fn into_parts(self) -> (PluginHostRequest, Option<TelemetryExecutionLease>) {
        (self.request, self.telemetry)
    }
}

pub(super) struct TelemetryExecutionLease {
    connected: Arc<AtomicBool>,
    deadline: Instant,
    retired: AtomicBool,
}

impl TelemetryExecutionLease {
    fn remaining(&self) -> Result<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if !self.connected.load(Ordering::Acquire) || remaining.is_zero() {
            self.retire();
        }
        ensure!(
            !self.retired.load(Ordering::Acquire),
            "telemetry execution lease ended"
        );
        Ok(remaining)
    }

    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }

    async fn lock<'a>(
        &self,
        store: &'a MachinePluginStore,
    ) -> Result<tokio::sync::MutexGuard<'a, ()>> {
        let guard = tokio::time::timeout(self.remaining()?, store.lifecycle.lock())
            .await
            .map_err(|_| {
                self.retire();
                anyhow::anyhow!("telemetry admission budget ended")
            })?;
        self.remaining()?;
        Ok(guard)
    }
}

// In legacy/untracked installations this is a local filesystem observation,
// not a durable installation generation. Tracked inventory also checks its
// monotonic installation revision. Neither observation is serialized as a grant.
#[derive(PartialEq, Eq)]
struct ActivationObservation {
    device: u64,
    inode: u64,
    changed: (i64, i64),
}

impl ActivationObservation {
    fn read(store: &MachinePluginStore, id: &str) -> Result<Self> {
        let metadata = fs::symlink_metadata(store.plugin_root(id).join("active"))?;
        ensure!(metadata.is_symlink(), "telemetry activation is not a link");
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

impl MachinePluginStore {
    pub(super) async fn export_telemetry(
        &self,
        request: PluginHostRequest,
        lease: TelemetryExecutionLease,
    ) -> std::result::Result<serde_json::Value, PluginHostInvocationFailure> {
        let _export = self
            .telemetry_export
            .try_lock()
            .map_err(|_| anyhow::anyhow!("telemetry exporter is busy"))?;
        let PluginHostRequest {
            plugin_id,
            plugin_version,
            generation_digest,
            auth_generation,
            operation,
            payload,
        } = request;
        validate_plugin_id(&plugin_id)?;
        ensure_no_auth(auth_generation)?;
        let policy_path = self
            .root
            .parent()
            .context("Machine state directory is missing")?
            .join("telemetry.json");
        let selection = crate::telemetry_plugin::PluginSelection {
            plugin_id: plugin_id.clone(),
            plugin_version,
            generation_digest,
        };
        let (contract, policy, payload, active, activation) = {
            let _lifecycle = lease.lock(self).await?;
            self.operations.telemetry_bindings.ensure_legacy_allowed()?;
            self.operations.ensure_unfenced(&plugin_id)?;
            let active = self.telemetry_inventory(&selection)?;
            let activation = ActivationObservation::read(self, &plugin_id)?;
            let contract = self.telemetry_contract(&selection, &active)?;
            let (policy, payload) = crate::telemetry_plugin::prepare(
                &policy_path,
                &selection,
                payload,
                operation == PluginHostOperation::ExportOtlp,
            )?;
            lease.remaining()?;
            (contract, policy, payload, active, activation)
        };
        let admitted = AtomicBool::new(false);
        let admit = || async {
            let result = async {
                let _lifecycle = lease.lock(self).await?;
                self.operations.telemetry_bindings.ensure_legacy_allowed()?;
                self.operations.ensure_unfenced(&plugin_id)?;
                let current = self.telemetry_inventory(&selection)?;
                ensure!(
                    current.installation_revision == active.installation_revision
                        && current.contract_fingerprint == active.contract_fingerprint
                        && ActivationObservation::read(self, &plugin_id)? == activation,
                    "telemetry installation changed"
                );
                // Revalidate retained signed bytes and the original private
                // policy at EVERY attempt. Never inherit a replacement token,
                // endpoint or a same-release reinstallation on a retry.
                self.telemetry_contract(&selection, &current)?;
                ensure!(policy.unchanged(&policy_path), "telemetry policy changed");
                lease.remaining()?;
                admitted.store(true, Ordering::Release);
                Ok::<(), anyhow::Error>(())
            }
            .await;
            if result.is_err() {
                lease.retire();
            }
            result.is_ok()
        };
        // The lock ends at attempt admission. Already-admitted HTTP may finish
        // after disconnect/uninstall; network I/O never owns the lifecycle lock.
        let result = crate::telemetry_plugin::export(&contract, &policy, payload, &admit).await;
        if lease.retired.load(Ordering::Acquire) && !admitted.load(Ordering::Acquire) {
            return Err(anyhow::anyhow!("telemetry admission ended before emission").into());
        }
        serde_json::to_value(result).map_err(|error| PluginHostInvocationFailure {
            started: true,
            error: error.into(),
        })
    }

    pub(super) fn telemetry_inventory(
        &self,
        selection: &crate::telemetry_plugin::PluginSelection,
    ) -> Result<PluginInventory> {
        let active = self
            .inventory_one(&selection.plugin_id)?
            .context("telemetry Plugin is not installed")?;
        ensure!(
            active.state == PluginInstallationState::Active
                && active.plugin_version == selection.plugin_version
                && active.generation_digest == selection.generation_digest
                && active.auth_generation.is_none()
                && active.plugin_kind == cowboy_plugin_sdk::PluginKind::TelemetryBackend,
            "active telemetry Plugin generation mismatch"
        );
        Ok(active)
    }

    pub(super) fn telemetry_contract(
        &self,
        selection: &crate::telemetry_plugin::PluginSelection,
        inventory: &PluginInventory,
    ) -> Result<cowboy_plugin_sdk::TelemetryBackendContract> {
        let (package, _, _) =
            self.verified_plugin_generation(&selection.plugin_id, &selection.generation_digest)?;
        ensure!(
            package.manifest.version == selection.plugin_version
                && package.contract_fingerprint == inventory.contract_fingerprint,
            "telemetry inventory does not match signed release"
        );
        let PluginPayload::TelemetryBackend(contract) = package.payload else {
            bail!("Plugin is not a telemetry backend");
        };
        Ok(contract)
    }
}

fn ensure_no_auth(generation: Option<u64>) -> Result<()> {
    ensure!(
        generation.is_none(),
        "telemetry cannot use Provider credentials"
    );
    Ok(())
}

#[cfg(all(test, feature = "full"))]
mod tests;
