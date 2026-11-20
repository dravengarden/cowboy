//! Usage of a named portable account (protocol 50): the Provider's signed
//! collector runs in the account's own vault replica scope, exactly the
//! credential and home a session on that account launches with, and only on
//! the account's current Service generation. It never reads the Machine's
//! Service-account credential.
//!
//! Credential refresh: a collector may rotate the account's credential (the
//! Codex collector asks its CLI to refresh an expired token once). The CLI
//! writes the rotation into the account's runtime projection, which the
//! account-scope credential watch proposes to the Service like any session's
//! refresh: the Service compare-and-swaps it into the vault and redistributes
//! it, or discards a stale loser and restores the vault generation. The
//! collector itself persists nothing, and an observation that straddles a
//! generation change is refused by the same host fence as Service collectors.
use super::*;
use crate::machine_protocol::portable_usage::{PortableUsageRequest, portable_usage_error_code};

/// Upper bound on waiting behind this account's credential writes or another
/// observation before the query is refused as busy.
const ADMISSION: Duration = Duration::from_secs(20);

pub struct PortableInvocation {
    request: PortableUsageRequest,
    connected: Arc<AtomicBool>,
}

impl PortableInvocation {
    pub fn new(request: PortableUsageRequest, connected: Arc<AtomicBool>) -> Self {
        Self { request, connected }
    }
}

impl MachinePluginStore {
    /// Collect one named portable account's usage. Errors are closed codes
    /// from [`portable_usage_error_code`]; collector output never leaves.
    pub async fn collect_portable_usage(
        &self,
        invocation: PortableInvocation,
    ) -> Result<serde_json::Value> {
        let PortableInvocation { request, connected } = invocation;
        self.portable_usage(request, &connected)
            .await
            .map_err(|error| anyhow::anyhow!(portable_usage_error_code(&format!("{error:#}"))))
    }

    async fn portable_usage(
        &self,
        request: PortableUsageRequest,
        connected: &AtomicBool,
    ) -> Result<serde_json::Value> {
        let account = portable_usage_account(&request)?;
        let scoped = self
            .for_account(account)
            .context("portable_usage_account_required")?;
        // Credential applies and wipes of this account wait for this guard
        // (account -> lifecycle order), so the replica cannot be replaced or
        // removed under the running collector.
        let _observation = tokio::time::timeout(ADMISSION, scoped.native_observation_guard())
            .await
            .map_err(|_| anyhow::anyhow!("portable_usage_busy"))?;
        ensure!(
            connected.load(Ordering::Acquire),
            "machine_connection_changed"
        );
        ensure!(
            scoped
                .latest_auth_envelope(&request.plugin_id)?
                .is_some_and(|replica| replica.auth_generation == request.auth_generation
                    && replica.action == ProviderAuthAction::Apply),
            "portable_usage_replica_pending"
        );
        // The Service host path at account scope: it verifies the exact signed
        // generation and the account replica generation under the lifecycle
        // lock, launches the collector with the account's session environment
        // (sidecars included) and refuses an observation whose generation
        // changed while it ran.
        scoped
            .invoke_executable_host(PluginHostRequest {
                plugin_id: request.plugin_id,
                plugin_version: request.plugin_version,
                generation_digest: request.generation_digest,
                auth_generation: Some(request.auth_generation),
                operation: PluginHostOperation::CollectUsage,
                payload: request.payload,
            })
            .await
            .map_err(|failure| failure.error)
    }
}

/// Only a named account of a Provider whose named credentials live in the
/// Service vault (never a native-only Provider's native home) qualifies.
fn portable_usage_account(request: &PortableUsageRequest) -> Result<&ProviderAccountId> {
    ensure!(
        !request.account_id.is_default()
            && request.auth_generation > 0
            && crate::provider_account::portable_credentials_allowed(
                &request.account_id,
                &request.plugin_id,
            )
            .is_ok(),
        "portable_usage_account_required"
    );
    Ok(&request.account_id)
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
        assert!(portable_usage_account(&request(named, "codex", 3)).is_ok());
        for refused in [
            request("default", "codex", 3),
            request(named, "codex", 0),
            request(named, "claude-code", 3),
        ] {
            assert_eq!(
                portable_usage_account(&refused).unwrap_err().to_string(),
                "portable_usage_account_required"
            );
        }
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
