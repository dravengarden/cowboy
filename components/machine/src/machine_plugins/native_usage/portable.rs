//! Usage and banked resets of a named portable account (protocol 50): the
//! Provider's signed collector runs in the account's own vault replica scope,
//! exactly the credential and home a session on that account launches with,
//! and only on the account's current Service generation (every participating
//! scope member's, sidecars included). It never reads the Machine's
//! Service-account credential.
//!
//! Credential refresh: a usage poll or reset must never redeem a refresh
//! token. A Service CAS after the vendor redeemed one cannot repair a token a
//! concurrent session on another Machine redeemed too. So the request always
//! carries `"token_refresh": false`, and for a Provider whose CLI rotates
//! credentials the exact signed collector must declare
//! ([`REFRESH_FREE_DECLARATION`]) that it then reports authentication required
//! instead of refreshing; renewal stays with the sessions' coordinated path.
//! A collector without that declaration is refused before it starts.
use super::*;
use crate::machine_protocol::portable_usage::{
    PortableResetRequest, PortableUsageRequest, REFRESH_FREE_DECLARATION,
    portable_usage_error_code, project_reset_reply,
};

/// Every portable invocation must start within this bound of its receipt: the
/// Controller abandons its wait after 45 seconds.
const DEADLINE: Duration = Duration::from_secs(30);

/// The Controller connection and receipt deadline one invocation was admitted
/// under; nothing starts after either has passed.
pub struct HostAdmission {
    connected: Arc<AtomicBool>,
    deadline: Instant,
}

impl HostAdmission {
    fn new(connected: Arc<AtomicBool>) -> Self {
        Self {
            connected,
            deadline: Instant::now() + DEADLINE,
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

/// One banked reset of a named portable account; see
/// [`MachinePluginStore::consume_portable_reset`].
pub struct PortableResetInvocation {
    request: PortableResetRequest,
    admission: HostAdmission,
}

impl PortableResetInvocation {
    pub fn new(request: PortableResetRequest, connected: Arc<AtomicBool>) -> Self {
        Self {
            request,
            admission: HostAdmission::new(connected),
        }
    }
}

/// What one portable invocation must hold before its collector may start.
struct Scope<'a> {
    account: &'a ProviderAccountId,
    plugin_id: &'a str,
    generation_digest: &'a str,
    auth_generation: u64,
    sibling_generations: &'a BTreeMap<String, u64>,
}

impl MachinePluginStore {
    /// Admit one invocation: a named vault account, the account's credential
    /// guard (so no apply or wipe overlaps the collector), the exact replica
    /// generation of the collector and of every participating scope member,
    /// and a refresh-free collector where the Provider CLI rotates tokens.
    async fn admit_portable(
        &self,
        scope: &Scope<'_>,
        admission: &HostAdmission,
    ) -> Result<(Self, tokio::sync::OwnedMutexGuard<()>)> {
        portable_account(scope.account, scope.plugin_id, scope.auth_generation)?;
        let scoped = self
            .for_account(scope.account)
            .context("portable_usage_account_required")?;
        let guard = tokio::time::timeout(admission.remaining()?, scoped.native_observation_guard())
            .await
            .map_err(|_| anyhow::anyhow!("portable_usage_busy"))?;
        admission.remaining()?;
        let expected = std::iter::once((scope.plugin_id, scope.auth_generation)).chain(
            scope
                .sibling_generations
                .iter()
                .map(|(plugin, generation)| (plugin.as_str(), *generation)),
        );
        for (plugin, generation) in expected {
            ensure!(
                scoped.latest_auth_envelope(plugin)?.is_some_and(|replica| {
                    replica.auth_generation == generation
                        && replica.action == ProviderAuthAction::Apply
                }),
                "portable_usage_replica_pending"
            );
        }
        self.require_refresh_free(scope.plugin_id, scope.generation_digest)?;
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

    /// Spend one banked reset the owner confirmed or scheduled: the account's
    /// signed collector runs `consume_reset` under exactly the admission of
    /// [`Self::collect_portable_usage`]. Like Service resets it keeps the
    /// installation lifecycle until it ends. A failure after the collector
    /// started is `portable_reset_outcome_unknown` with `started` kept, so the
    /// Controller never retries it; the collector's own refusals return as
    /// its projected reply.
    pub async fn consume_portable_reset(
        &self,
        invocation: PortableResetInvocation,
    ) -> std::result::Result<serde_json::Value, PluginHostInvocationFailure> {
        let PortableResetInvocation { request, admission } = invocation;
        let refuse = |code: &'static str| PluginHostInvocationFailure {
            started: false,
            error: anyhow::anyhow!(code),
        };
        if !request.reset.valid() {
            return Err(refuse("portable_reset_request_invalid"));
        }
        let scope = Scope {
            account: &request.account_id,
            plugin_id: &request.plugin_id,
            generation_digest: &request.generation_digest,
            auth_generation: request.auth_generation,
            sibling_generations: &request.scope_generations,
        };
        let (scoped, _guard) = self
            .admit_portable(&scope, &admission)
            .await
            .map_err(|error| refuse(portable_usage_error_code(&format!("{error:#}"))))?;
        let mut payload = serde_json::to_value(&request.reset).map_err(anyhow::Error::from)?;
        payload["token_refresh"] = serde_json::Value::Bool(false);
        scoped
            .invoke_executable_host_admitted(
                PluginHostRequest {
                    plugin_id: request.plugin_id.clone(),
                    plugin_version: request.plugin_version.clone(),
                    generation_digest: request.generation_digest.clone(),
                    auth_generation: Some(request.auth_generation),
                    operation: PluginHostOperation::ResetUsage,
                    payload,
                },
                Some(&admission),
            )
            .await
            // Only the closed reset reply leaves the Machine, never an
            // upstream RPC error body; its outcome certainty is kept.
            .map(|reply| project_reset_reply(&reply))
            .map_err(|failure| PluginHostInvocationFailure {
                started: failure.started,
                error: anyhow::anyhow!(if failure.started {
                    "portable_reset_outcome_unknown"
                } else {
                    portable_usage_error_code(&format!("{:#}", failure.error))
                }),
            })
    }

    /// Collect one named portable account's usage. Errors are closed codes
    /// from [`portable_usage_error_code`]; collector output never leaves.
    pub async fn collect_portable_usage(
        &self,
        invocation: PortableInvocation,
    ) -> Result<serde_json::Value> {
        let PortableInvocation {
            mut request,
            admission,
        } = invocation;
        let scope = Scope {
            account: &request.account_id,
            plugin_id: &request.plugin_id,
            generation_digest: &request.generation_digest,
            auth_generation: request.auth_generation,
            sibling_generations: &request.scope_generations,
        };
        let (scoped, _guard) = self
            .admit_portable(&scope, &admission)
            .await
            .map_err(|error| anyhow::anyhow!(portable_usage_error_code(&format!("{error:#}"))))?;
        if let Some(payload) = request.payload.as_object_mut() {
            payload.insert("token_refresh".into(), serde_json::Value::Bool(false));
        }
        // The Service host path at account scope: it verifies the exact signed
        // generation and the account replica generation under the lifecycle
        // lock, launches the collector with the account's session environment
        // (sidecars included) and refuses an observation whose generation
        // changed while it ran.
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
        sidecar.scope_generations.insert("codex-deepseek".into(), 3);
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

    #[tokio::test]
    async fn a_reset_that_cannot_be_admitted_never_starts_the_collector() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let reset = |key: &str| -> PortableResetRequest {
            serde_json::from_value(serde_json::json!({
                "account_id": "11111111-1111-4111-8111-111111111111", "plugin_id": "codex",
                "plugin_version": "1.0.0", "generation_digest": "sha256:x", "auth_generation": 3,
                "reset": {"provider": "openai", "idempotency_key": key,
                    "expected_credit_id": "credit-1"},
            }))
            .unwrap()
        };
        let mut claude = reset("cowboy-reset-1");
        claude.plugin_id = "claude-code".into();
        for (request, code) in [
            (reset("not a key"), "portable_reset_request_invalid"),
            (claude, "portable_usage_account_required"),
            // This Machine holds no replica of the account's generation.
            (reset("cowboy-reset-1"), "portable_usage_replica_pending"),
        ] {
            let invocation = PortableResetInvocation::new(request, Arc::new(AtomicBool::new(true)));
            let failure = store.consume_portable_reset(invocation).await.unwrap_err();
            assert!(!failure.started, "{code}");
            assert_eq!(failure.error.to_string(), code);
        }
    }
}
