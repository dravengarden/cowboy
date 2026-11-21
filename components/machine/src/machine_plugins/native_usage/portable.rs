//! Read-only usage of a named portable account (protocol 50): the Provider's
//! signed collector runs in the account's own vault replica scope, exactly the
//! credential and home a session on that account launches with, and only on
//! the account's current Service generation, shared by every participating
//! sidecar. It never reads the Machine's Service-account credential.
//!
//! Credential refresh: a usage poll must never redeem a refresh token. A
//! Service CAS after the vendor redeemed one cannot repair a token a
//! concurrent session on another Machine redeemed too. So the request always
//! carries `"token_refresh": false` (set by the Controller and again here),
//! and for a Provider whose CLI rotates credentials the exact signed collector
//! must declare ([`REFRESH_FREE_DECLARATION`]) that it then reports
//! authentication required instead of refreshing; renewal stays with the
//! sessions' coordinated path. A collector without that declaration never
//! starts.
use super::*;
use crate::machine_protocol::portable_usage::{
    PortableUsageRequest, REFRESH_FREE_DECLARATION, portable_usage_error_code,
};

/// Every portable invocation must start within this bound of its receipt: the
/// Controller abandons its wait after 45 seconds.
const DEADLINE: Duration = Duration::from_secs(30);

/// The Controller connection, receipt deadline and checked sidecar set one
/// invocation was admitted under; nothing starts after either has passed.
pub struct HostAdmission {
    connected: Arc<AtomicBool>,
    deadline: Instant,
    pub(super) sidecars: Option<BTreeMap<String, String>>,
    /// Variables a refresh-custody renewal adds to the collector.
    pub(super) environment: Option<BTreeMap<String, String>>,
}

impl HostAdmission {
    pub(super) fn new(connected: Arc<AtomicBool>) -> Self {
        Self {
            connected,
            deadline: Instant::now() + DEADLINE,
            sidecars: None,
            environment: None,
        }
    }

    pub(in crate::machine_plugins) fn remaining(&self) -> Result<Duration> {
        ensure!(
            self.connected.load(Ordering::Acquire),
            "machine_connection_changed"
        );
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "portable_usage_busy");
        Ok(remaining)
    }

    /// The absolute instant after which nothing of this invocation starts.
    pub(in crate::machine_plugins) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Variables added to the collector's environment (renewals only).
    pub(in crate::machine_plugins) fn environment(&self) -> Option<&BTreeMap<String, String>> {
        self.environment.as_ref()
    }

    /// The only sidecar Plugin generations this invocation may start.
    pub(in crate::machine_plugins) fn sidecars(&self) -> Option<&BTreeMap<String, String>> {
        self.sidecars.as_ref()
    }
}

pub struct PortableInvocation {
    request: PortableUsageRequest,
    admission: HostAdmission,
}

impl PortableInvocation {
    pub fn new(request: PortableUsageRequest, connected: Arc<AtomicBool>) -> Self {
        Self {
            request,
            admission: HostAdmission::new(connected),
        }
    }
}

impl MachinePluginStore {
    /// Admit one collection: a named vault account, the account's credential
    /// guard (so no apply or wipe overlaps the collector), the exact replica
    /// generation of the collector and of every checked sidecar (one shared
    /// generation), and a refresh-free collector where the Provider CLI
    /// rotates tokens.
    pub(super) async fn admit_portable(
        &self,
        request: &PortableUsageRequest,
        admission: &HostAdmission,
    ) -> Result<(Self, tokio::sync::OwnedMutexGuard<()>)> {
        portable_account(
            &request.account_id,
            &request.plugin_id,
            request.auth_generation,
        )?;
        let scoped = self
            .for_account(&request.account_id)
            .context("portable_usage_account_required")?;
        let guard = tokio::time::timeout(admission.remaining()?, scoped.native_observation_guard())
            .await
            .map_err(|_| anyhow::anyhow!("portable_usage_busy"))?;
        admission.remaining()?;
        let participants = std::iter::once(request.plugin_id.as_str()).chain(
            request
                .sidecars
                .iter()
                .map(|sidecar| sidecar.plugin_id.as_str()),
        );
        for plugin in participants {
            ensure!(
                scoped.latest_auth_envelope(plugin)?.is_some_and(|replica| {
                    replica.auth_generation == request.auth_generation
                        && replica.action == ProviderAuthAction::Apply
                }),
                "portable_usage_replica_pending"
            );
        }
        self.require_refresh_free(&request.plugin_id, &request.generation_digest)?;
        Ok((scoped, guard))
    }

    /// A Provider whose CLI rotates credentials (`compare_and_swap`) needs a
    /// collector that declares it never redeems a refresh token on request.
    fn require_refresh_free(&self, plugin_id: &str, digest: &str) -> Result<()> {
        let (plugin, release, content) = self
            .verified_plugin_generation(plugin_id, digest)
            .context("portable_usage_collector_unavailable")?;
        let package = plugin
            .agent_provider()
            .context("portable_usage_collector_unavailable")?;
        if package.manifest.authentication.refresh
            != cowboy_provider_sdk::RefreshOwnership::CompareAndSwap
        {
            return Ok(());
        }
        let declared = verified_plugin_host_bundle(&plugin, &release, &content)
            .ok()
            .flatten()
            .and_then(|host| host.files.get(REFRESH_FREE_DECLARATION).cloned())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|value| value["token_refresh"] == "request");
        ensure!(declared, "portable_usage_refresh_unsupported");
        Ok(())
    }

    /// Collect one named portable account's usage. Errors are closed codes
    /// from [`portable_usage_error_code`]; collector output never leaves.
    pub async fn collect_portable_usage(
        &self,
        invocation: PortableInvocation,
    ) -> Result<serde_json::Value> {
        let PortableInvocation {
            mut request,
            mut admission,
        } = invocation;
        let (scoped, _guard) = self
            .admit_portable(&request, &admission)
            .await
            .map_err(|error| anyhow::anyhow!(portable_usage_error_code(&format!("{error:#}"))))?;
        admission.sidecars = Some(
            request
                .sidecars
                .iter()
                .map(|sidecar| (sidecar.plugin_id.clone(), sidecar.generation_digest.clone()))
                .collect(),
        );
        if let Some(payload) = request.payload.as_object_mut() {
            payload.insert("token_refresh".into(), serde_json::Value::Bool(false));
        }
        // The Service host path at account scope: it verifies the exact signed
        // generation and the account replica generation under the lifecycle
        // lock, launches the collector with the account's session environment
        // (only the checked sidecars) and refuses an observation whose
        // generation changed while it ran.
        scoped
            .invoke_executable_host_admitted(
                PluginHostRequest {
                    plugin_id: request.plugin_id,
                    plugin_version: request.plugin_version,
                    generation_digest: request.generation_digest,
                    auth_generation: Some(request.auth_generation),
                    operation: PluginHostOperation::CollectUsage,
                    payload: request.payload,
                },
                Some(&admission),
            )
            .await
            .map_err(|failure| {
                anyhow::anyhow!(portable_usage_error_code(&format!("{:#}", failure.error)))
            })
    }
}

/// Only a named account of a Provider whose named credentials live in the
/// Service vault (never a native-only Provider's native home) qualifies.
fn portable_account(account: &ProviderAccountId, plugin: &str, generation: u64) -> Result<()> {
    ensure!(
        !account.is_default()
            && generation > 0
            && crate::provider_account::portable_credentials_allowed(account, plugin).is_ok(),
        "portable_usage_account_required"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(account: &str, plugin: &str, generation: u64) -> PortableUsageRequest {
        serde_json::from_value(serde_json::json!({
            "account_id": account, "plugin_id": plugin, "plugin_version": "1.0.0",
            "generation_digest": "sha256:x", "auth_generation": generation,
            "payload": {},
        }))
        .unwrap()
    }

    #[test]
    fn only_named_vault_accounts_are_admitted() {
        let named = "11111111-1111-4111-8111-111111111111";
        let admit = |request: &PortableUsageRequest| {
            portable_account(
                &request.account_id,
                &request.plugin_id,
                request.auth_generation,
            )
        };
        assert!(admit(&request(named, "codex", 3)).is_ok());
        for refused in [
            request("default", "codex", 3),
            request(named, "codex", 0),
            request(named, "claude-code", 3),
        ] {
            assert_eq!(
                admit(&refused).unwrap_err().to_string(),
                "portable_usage_account_required"
            );
        }
    }

    #[test]
    fn an_abandoned_or_disconnected_invocation_never_starts() {
        let connected = Arc::new(AtomicBool::new(true));
        let admission = HostAdmission::new(Arc::clone(&connected));
        assert!(admission.remaining().is_ok());
        let expired = HostAdmission {
            connected: Arc::clone(&connected),
            deadline: Instant::now(),
            sidecars: None,
            environment: None,
        };
        assert_eq!(
            expired.remaining().unwrap_err().to_string(),
            "portable_usage_busy"
        );
        connected.store(false, Ordering::Release);
        assert_eq!(
            admission.remaining().unwrap_err().to_string(),
            "machine_connection_changed"
        );
    }

    #[tokio::test]
    async fn every_participating_replica_must_be_the_requested_generation() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let mut sidecar = request("11111111-1111-4111-8111-111111111111", "claude-deepseek", 3);
        sidecar
            .sidecars
            .push(crate::machine_protocol::portable_usage::PortableSidecar {
                plugin_id: "codex-deepseek".into(),
                generation_digest: "sha256:y".into(),
            });
        let invocation = PortableInvocation::new(sidecar, Arc::new(AtomicBool::new(true)));
        let error = store.collect_portable_usage(invocation).await.unwrap_err();
        assert_eq!(error.to_string(), "portable_usage_replica_pending");
    }

    #[tokio::test]
    async fn a_machine_without_the_current_account_generation_refuses_before_starting() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let connected = Arc::new(AtomicBool::new(true));
        let invocation = PortableInvocation::new(
            request("11111111-1111-4111-8111-111111111111", "codex", 3),
            Arc::clone(&connected),
        );
        let error = store.collect_portable_usage(invocation).await.unwrap_err();
        assert_eq!(error.to_string(), "portable_usage_replica_pending");
        connected.store(false, Ordering::Release);
        let invocation = PortableInvocation::new(
            request("11111111-1111-4111-8111-111111111111", "codex", 3),
            connected,
        );
        let error = store.collect_portable_usage(invocation).await.unwrap_err();
        assert_eq!(error.to_string(), "machine_connection_changed");
    }

    #[tokio::test]
    async fn a_busy_account_does_not_block_another_account() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let busy: ProviderAccountId = "11111111-1111-4111-8111-111111111111".parse().unwrap();
        let _held = store
            .for_account(&busy)
            .unwrap()
            .native_observation_guard()
            .await;
        let other = PortableInvocation::new(
            request("22222222-2222-4222-8222-222222222222", "codex", 3),
            Arc::new(AtomicBool::new(true)),
        );
        // The other account reaches its own replica check at once.
        let error =
            tokio::time::timeout(Duration::from_secs(2), store.collect_portable_usage(other))
                .await
                .expect("independent accounts never wait for each other")
                .unwrap_err();
        assert_eq!(error.to_string(), "portable_usage_replica_pending");
    }
}
