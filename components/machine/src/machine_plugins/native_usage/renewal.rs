//! Refresh-custody renewal of a named portable account (protocol 51).
//!
//! Every Machine holds only an access-only copy of a custody-managed
//! account. A renewal restores the Service's full credential, sealed to
//! this Machine for this one operation, into a private directory outside
//! every credential watch, runs the collector's ordinary refresh-capable
//! path with the Provider CLI pointed at that file, and returns the result.
//! The directory is always removed; the run is bounded by the Plugin
//! command timeout and [`RENEWAL_MACHINE_DEADLINE_MS`]. A reply is retained
//! in memory so a lost one can be resumed without ever running again.
use std::sync::LazyLock;

use super::*;
use crate::machine_protocol::portable_usage::portable_usage_error_code;
use crate::machine_protocol::refresh_custody::{
    PortableRenewalReply, PortableRenewalRequest, RENEWAL_MACHINE_DEADLINE_MS, RefreshCustody,
};

/// The credential file was unreadable after the run: outcome unknown.
pub const RENEWAL_UNREADABLE: &str = "portable_renewal_unreadable";

/// A resumed renewal this Machine no longer retains (it never runs again).
pub const RENEWAL_NOT_RETAINED: &str = "portable_renewal_not_retained";

/// Retained replies, newest last; bounded and short-lived.
static RETAINED: LazyLock<parking_lot::Mutex<Vec<(String, Instant, PortableRenewalReply)>>> =
    LazyLock::new(Default::default);
const RETAINED_ENTRIES: usize = 16;
const RETAINED_FOR: Duration = Duration::from_mins(15);

/// Renewal IDs running now: a resume of one is busy, never unknown.
static RUNNING: LazyLock<parking_lot::Mutex<std::collections::BTreeSet<String>>> =
    LazyLock::new(Default::default);

struct Running(String);

impl Running {
    fn start(renewal_id: &str) -> Option<Self> {
        RUNNING
            .lock()
            .insert(renewal_id.to_owned())
            .then(|| Self(renewal_id.to_owned()))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.lock().remove(&self.0);
    }
}

fn retained(renewal_id: &str) -> Option<PortableRenewalReply> {
    let mut retained = RETAINED.lock();
    retained.retain(|(_, at, _)| at.elapsed() < RETAINED_FOR);
    retained
        .iter()
        .find(|(id, _, _)| id == renewal_id)
        .map(|(_, _, reply)| reply.clone())
}

fn retain(reply: &PortableRenewalReply) {
    let mut retained = RETAINED.lock();
    retained.retain(|(id, at, _)| at.elapsed() < RETAINED_FOR && *id != reply.renewal_id);
    retained.push((reply.renewal_id.clone(), Instant::now(), reply.clone()));
    let excess = retained.len().saturating_sub(RETAINED_ENTRIES);
    retained.drain(..excess);
}

pub struct RenewalInvocation {
    request: PortableRenewalRequest,
    admission: HostAdmission,
}

impl RenewalInvocation {
    pub fn new(request: PortableRenewalRequest, connected: Arc<AtomicBool>) -> Self {
        Self {
            request,
            admission: HostAdmission::new(connected),
        }
    }
}

impl MachinePluginStore {
    /// The refresh-custody declaration of an exact verified generation.
    pub(in crate::machine_plugins) fn refresh_custody(
        &self,
        plugin_id: &str,
        digest: &str,
    ) -> Result<Option<RefreshCustody>> {
        let (plugin, release, content) = self.verified_plugin_generation(plugin_id, digest)?;
        let Some(host) = verified_plugin_host_bundle(&plugin, &release, &content)? else {
            return Ok(None);
        };
        RefreshCustody::from_files(&host.files).map_err(anyhow::Error::msg)
    }

    /// Run one renewal, or return the retained reply of a known one.
    pub async fn renew_portable_credential(
        &self,
        invocation: RenewalInvocation,
    ) -> Result<serde_json::Value> {
        let RenewalInvocation { request, admission } = invocation;
        // A renewal ID runs at most once on this Machine.
        if let Some(reply) = retained(&request.renewal_id) {
            return Ok(serde_json::to_value(reply)?);
        }
        let Some(_running) = Running::start(&request.renewal_id) else {
            anyhow::bail!("portable_usage_busy");
        };
        if let Some(reply) = retained(&request.renewal_id) {
            return Ok(serde_json::to_value(reply)?);
        }
        ensure!(!request.resume_only, RENEWAL_NOT_RETAINED);
        let envelope = request
            .envelope
            .as_ref()
            .context("portable_usage_authentication_required")?;
        let usage = &request.usage;
        let (scoped, _guard) = self
            .admit_portable(usage, &admission)
            .await
            .map_err(|error| anyhow::anyhow!(portable_usage_error_code(&format!("{error:#}"))))?;
        let custody = self
            .refresh_custody(&usage.plugin_id, &usage.generation_digest)
            .ok()
            .flatten()
            .context("portable_usage_collector_unavailable")?;
        let full = scoped
            .open_renewal_envelope(envelope, usage)
            .map_err(|error| {
                anyhow::anyhow!("portable_usage_authentication_required: {error:#}")
            })?;
        ensure!(
            custody.holds_refresh_material(&full),
            "portable_usage_authentication_required"
        );
        let Some((usage_value, after)) = scoped
            .run_with_full_credential(&custody, &full, usage, admission)
            .await?
        else {
            // The CLI may have renewed and left an unreadable file: the
            // outcome is unknown, never "unchanged".
            let reply = PortableRenewalReply {
                renewal_id: request.renewal_id.clone(),
                usage: serde_json::json!({"error": RENEWAL_UNREADABLE}),
                bundle: None,
            };
            retain(&reply);
            return Ok(serde_json::to_value(reply)?);
        };
        let reply = PortableRenewalReply {
            renewal_id: request.renewal_id.clone(),
            usage: usage_value,
            bundle: after,
        };
        retain(&reply);
        Ok(serde_json::to_value(reply)?)
    }

    /// Restore `full` privately, run the collector's refresh-capable path
    /// against it, and read it back. `None` when the file is unreadable after
    /// the run (unknown outcome).
    async fn run_with_full_credential(
        &self,
        custody: &RefreshCustody,
        full: &PortableCredentialBundle,
        usage: &crate::machine_protocol::portable_usage::PortableUsageRequest,
        mut admission: HostAdmission,
    ) -> Result<Option<(serde_json::Value, Option<PortableCredentialBundle>)>> {
        let (package, _) = self
            .active_package(&usage.plugin_id)?
            .context("portable_usage_collector_unavailable")?;
        let auth = &package.manifest.authentication;
        let relative = auth
            .credential_files
            .iter()
            .find(|file| file.bundle_key == custody.bundle_key)
            .map(|file| file.relative_path.clone())
            .context("portable_usage_collector_unavailable")?;
        // Private, outside every credential watch, removed on every path.
        let root =
            std::env::temp_dir().join(format!("cowboy-custody-{:032x}", rand::random::<u128>()));
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let staged = StagedProbeHome(root);
        restore_projected_bundle(auth, &staged.0, full)?;
        let credential = staged.0.join("home").join(&relative);
        let mut environment = custody.renew_environment.clone();
        environment.insert(
            custody.credential_path_env.clone(),
            credential.display().to_string(),
        );
        admission.environment = Some(environment);
        admission.sidecars = Some(
            usage
                .sidecars
                .iter()
                .map(|sidecar| (sidecar.plugin_id.clone(), sidecar.generation_digest.clone()))
                .collect(),
        );
        let mut payload = usage.payload.clone();
        if let Some(payload) = payload.as_object_mut() {
            // The ordinary refresh-capable path: the CLI renews at start.
            payload.remove("token_refresh");
        }
        let run = self.invoke_executable_host_admitted(
            PluginHostRequest {
                plugin_id: usage.plugin_id.clone(),
                plugin_version: usage.plugin_version.clone(),
                generation_digest: usage.generation_digest.clone(),
                auth_generation: Some(usage.auth_generation),
                operation: PluginHostOperation::CollectUsage,
                payload,
            },
            Some(&admission),
        );
        // Dropping the run kills its process group. Whatever happened, the
        // credential file is read back: a CLI may have renewed before failing.
        let output =
            tokio::time::timeout(Duration::from_millis(RENEWAL_MACHINE_DEADLINE_MS), run).await;
        let after = projected_credential_bundle(auth, &staged.0, &full.method_id);
        drop(staged);
        let Ok(after) = after else {
            return Ok(None);
        };
        let after = Some(after);
        let usage_value = match output {
            Ok(Ok(value)) => value,
            Ok(Err(failure)) => serde_json::json!({
                "error": portable_usage_error_code(&format!("{:#}", failure.error))
            }),
            Err(_) => serde_json::json!({"error": "portable_usage_temporarily_unavailable"}),
        };
        Ok(Some((usage_value, after.filter(|after| after != full))))
    }

    /// Verify and open the renewal's sealed full credential: the Service's
    /// signature, this account and Provider, the exact vault generation.
    fn open_renewal_envelope(
        &self,
        envelope: &SealedProviderAuth,
        usage: &crate::machine_protocol::portable_usage::PortableUsageRequest,
    ) -> Result<PortableCredentialBundle> {
        ensure!(
            envelope.account_id == self.account_id
                && envelope.provider_id == usage.plugin_id
                && envelope.auth_generation == usage.auth_generation
                && envelope.action == ProviderAuthAction::Apply
                && envelope.envelope_schema == 2,
            "renewal envelope does not match the request"
        );
        ensure!(
            crate::machine_auth::verify_namespaced(
                &envelope.service_public_key,
                crate::machine_auth::PROVIDER_AUTH_SIGNATURE_NAMESPACE,
                &envelope.proof(),
                &envelope.signature,
            )?,
            "Service auth envelope signature is invalid"
        );
        let replica = self
            .latest_auth_envelope(&usage.plugin_id)?
            .context("account replica is missing")?;
        ensure!(
            replica.service_public_key == envelope.service_public_key,
            "renewal envelope is signed by another Service"
        );
        let package = self.active_package(&usage.plugin_id)?.map(|value| value.0);
        match self.prepare_auth_inner(envelope, package.as_ref())? {
            PreparedProviderAuth::Apply(bundle) => Ok(bundle),
            _ => anyhow::bail!("renewal envelope carries no credential"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(id: &str) -> PortableRenewalReply {
        PortableRenewalReply {
            renewal_id: id.into(),
            usage: serde_json::json!({}),
            bundle: None,
        }
    }

    fn renewal(id: &str, resume_only: bool) -> PortableRenewalRequest {
        serde_json::from_value(serde_json::json!({
            "renewal_id": id,
            "resume_only": resume_only,
            "usage": {
                "account_id": "11111111-1111-4111-8111-111111111111", "plugin_id": "grok",
                "plugin_version": "3.2.17", "generation_digest": "sha256:x",
                "auth_generation": 3, "payload": {},
            },
        }))
        .unwrap()
    }

    async fn run(
        store: &MachinePluginStore,
        request: PortableRenewalRequest,
    ) -> Result<serde_json::Value> {
        store
            .renew_portable_credential(RenewalInvocation::new(
                request,
                Arc::new(AtomicBool::new(true)),
            ))
            .await
    }

    /// A known renewal ID returns its retained reply and never runs again;
    /// an unknown resume never runs; a running one is busy; a fresh one
    /// without the account's current replica refuses before starting.
    #[tokio::test]
    async fn a_renewal_runs_at_most_once_per_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        assert_eq!(
            run(&store, renewal("renewal-unknown", true))
                .await
                .unwrap_err()
                .to_string(),
            RENEWAL_NOT_RETAINED
        );
        let error = run(&store, renewal("renewal-fresh", false))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "portable_usage_authentication_required");
        let kept = reply("renewal-kept");
        retain(&kept);
        let replayed = run(&store, renewal("renewal-kept", false)).await.unwrap();
        assert_eq!(
            serde_json::from_value::<PortableRenewalReply>(replayed).unwrap(),
            kept
        );
        let _running = Running::start("renewal-running").unwrap();
        assert_eq!(
            run(&store, renewal("renewal-running", false))
                .await
                .unwrap_err()
                .to_string(),
            "portable_usage_busy"
        );
        // Retention is bounded; the oldest replies are dropped first.
        for index in 0..RETAINED_ENTRIES + 4 {
            retain(&reply(&format!("renewal-fill-{index}")));
        }
        assert!(RETAINED.lock().len() <= RETAINED_ENTRIES);
        assert_eq!(retained("renewal-kept"), None);
    }
}
