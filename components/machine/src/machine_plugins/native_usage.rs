//! Bounded observations in the native account's home; never a portable grant.
use super::*;
use crate::provider_account::{NativeProfileReceipt, ProviderAccountId};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Weak};

mod portable;
mod sandbox;
pub use portable::PortableInvocation;

/// Native processes share a home only within one account. They must not hold
/// the global installation lock or serialize independent accounts.
#[derive(Default)]
pub(super) struct Observations(
    parking_lot::Mutex<BTreeMap<ProviderAccountId, Weak<tokio::sync::Mutex<()>>>>,
);

impl Observations {
    fn account(&self, id: &ProviderAccountId) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.0.lock();
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(id.clone(), Arc::downgrade(&lock));
        lock
    }
}

pub struct Invocation {
    profile: NativeProfileReceipt,
    connected: Arc<AtomicBool>,
    deadline: Instant,
}

impl Invocation {
    pub fn new(profile: NativeProfileReceipt, connected: Arc<AtomicBool>) -> Self {
        Self {
            profile,
            connected,
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }

    fn remaining(&self) -> Result<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        ensure!(
            self.connected.load(Ordering::Acquire) && !remaining.is_zero(),
            "native_usage_admission_expired"
        );
        Ok(remaining)
    }
}

impl MachinePluginStore {
    pub async fn collect_native_usage(&self, invocation: Invocation) -> Result<serde_json::Value> {
        let profile = &invocation.profile;
        let observation = tokio::time::timeout(
            invocation.remaining()?,
            self.native_observations
                .account(&profile.account_id)
                .lock_owned(),
        )
        .await
        .context("native_usage_busy")?;
        let mut lifecycle = Some(
            tokio::time::timeout(invocation.remaining()?, self.lifecycle.lock())
                .await
                .context("native_usage_busy")?,
        );
        invocation.remaining()?;
        self.validate_native_usage(profile)
            .context("native_usage_profile_changed")?;
        let mut payload = serde_json::json!({"operation":"collect"});
        // Use the active release; launch preparation checks its authentication
        // contract against this login before any native process starts.
        let (active, active_digest) = self
            .active_package(&profile.provider_id)?
            .context("native_usage_collector_unavailable")?;
        if account_sandbox::sandboxed(&active) {
            // The credential lives only in the daemon's account home.
            drop(lifecycle);
            // The credential is independent of the Provider release: the
            // query runs on the active generation, and the daemon admits only
            // the login's current signed-in credential generation.
            let usage = self
                .collect_sandbox_usage(&invocation, &active_digest, observation)
                .await?;
            let _lifecycle = tokio::time::timeout(invocation.remaining()?, self.lifecycle.lock())
                .await
                .context("native_usage_busy")?;
            self.validate_native_usage(profile)
                .context("native_usage_profile_changed")?;
            ensure!(
                self.active_package(&profile.provider_id)?
                    .is_some_and(|(_, digest)| digest == active_digest),
                "native_usage_installation_changed"
            );
            return Ok(usage);
        }
        let _observation = observation;
        let resolved = self
            .resolve_host_invocation(
                &profile.provider_id,
                &active.manifest.version,
                &active_digest,
                HostAuthentication::Native(profile),
                PluginHostOperation::CollectUsage,
                &mut payload,
            )
            .context("native_usage_collector_unavailable")?;
        let environment = resolved.environment;
        native_only_environment(&environment).context("native_usage_home_unsupported")?;
        invocation.remaining()?;
        let (program, arguments) = resolved
            .command
            .split_first()
            .context("native_usage_collector_unavailable")?;
        let output = crate::plugin_process::run_plugin_command_with_environment_on_started(
            program,
            arguments,
            &serde_json::to_vec(&payload)?,
            &environment,
            || {
                // Tracked installations have an incarnation fence. Keep only
                // this account's home guard while the native process runs.
                if resolved.installation_revision.is_some() {
                    drop(lifecycle.take());
                }
            },
        )
        .await
        .map_err(|error| error.error)
        .context("native_usage_collector_failed")?;
        if lifecycle.is_none() {
            lifecycle = Some(
                tokio::time::timeout(invocation.remaining()?, self.lifecycle.lock())
                    .await
                    .context("native_usage_busy")?,
            );
        }
        invocation.remaining()?;
        self.validate_native_usage(profile)
            .context("native_usage_profile_changed")?;
        self.validate_host_installation(
            &profile.provider_id,
            &active_digest,
            resolved.installation_revision.as_ref(),
        )
        .context("native_usage_installation_changed")?;
        drop(lifecycle);
        // Only known reason codes leave the Machine. Never return collector
        // stderr, filesystem paths or native credentials in the response.
        decode_host_response(&output).map_err(|error| {
            if error
                .error
                .to_string()
                .contains("Claude Code usage authentication required.")
            {
                anyhow::anyhow!("native_usage_authentication_required")
            } else {
                anyhow::anyhow!("native_usage_collector_failed")
            }
        })
    }

    /// Spend one banked usage reset the owner confirmed or scheduled, through
    /// the same signed collector, CLI identity, account home guard and deadline
    /// as [`Self::collect_native_usage`]. The collector re-reads availability
    /// and refuses a changed grant before it posts. Unlike a read, the reset
    /// keeps the installation lifecycle until it ends, as Service resets do.
    /// `started` on a failure means the collector may have reached the
    /// Provider; such an outcome is unknown and never retried automatically.
    pub async fn consume_native_reset(
        &self,
        invocation: Invocation,
        request: &crate::machine_protocol::NativeResetRequest,
    ) -> std::result::Result<serde_json::Value, PluginHostInvocationFailure> {
        let refuse = |code: &'static str| Err(anyhow::anyhow!(code).into());
        let profile = &invocation.profile;
        if !request.valid() {
            return refuse("native_reset_request_invalid");
        }
        let _observation = tokio::time::timeout(
            invocation.remaining()?,
            self.native_observations
                .account(&profile.account_id)
                .lock_owned(),
        )
        .await
        .context("native_usage_busy")?;
        let _lifecycle = tokio::time::timeout(invocation.remaining()?, self.lifecycle.lock())
            .await
            .context("native_usage_busy")?;
        self.validate_native_usage(profile)
            .context("native_usage_profile_changed")?;
        let (active, active_digest) = self
            .active_package(&profile.provider_id)?
            .context("native_usage_collector_unavailable")?;
        // An account-sandbox credential never leaves its daemon's custody.
        if account_sandbox::sandboxed(&active) {
            return refuse("native_reset_unsupported");
        }
        let mut payload = serde_json::to_value(request).map_err(anyhow::Error::from)?;
        let resolved = self
            .resolve_host_invocation(
                &profile.provider_id,
                &active.manifest.version,
                &active_digest,
                HostAuthentication::Native(profile),
                PluginHostOperation::ResetUsage,
                &mut payload,
            )
            .context("native_usage_collector_unavailable")?;
        native_only_environment(&resolved.environment).context("native_usage_home_unsupported")?;
        if !resolved
            .environment
            .contains_key("CLAUDE_SECURESTORAGE_CONFIG_DIR")
        {
            return refuse("native_reset_unsupported");
        }
        invocation.remaining()?;
        let (program, arguments) = resolved
            .command
            .split_first()
            .context("native_usage_collector_unavailable")?;
        let output = crate::plugin_process::run_plugin_command_with_environment_on_started(
            program,
            arguments,
            &serde_json::to_vec(&payload).map_err(anyhow::Error::from)?,
            &resolved.environment,
            || {},
        )
        .await
        .map_err(|failure| PluginHostInvocationFailure {
            started: failure.started,
            error: anyhow::anyhow!(if failure.started {
                "native_reset_outcome_unknown"
            } else {
                "native_usage_collector_failed"
            }),
        })?;
        // The collector answers every reset, refused or not, on stdout. Only
        // its closed fields leave the Machine; never stderr or credentials.
        decode_host_response(&output).map_err(|_| PluginHostInvocationFailure {
            started: true,
            error: anyhow::anyhow!("native_reset_outcome_unknown"),
        })
    }

    pub(super) fn validate_native_usage(&self, profile: &NativeProfileReceipt) -> Result<()> {
        // A native profile receipt exists only for a Provider whose signed
        // login recipe verified the identity; the receipt is the capability.
        ensure!(
            !profile.account_id.is_default(),
            "named native account required"
        );
        ensure!(
            self.observe_native_profile(
                &profile.account_id,
                &profile.provider_id,
                profile.generation,
                &profile.request_id
            )? == *profile,
            "native usage profile changed"
        );
        Ok(())
    }

    /// Serialize native logout against bounded usage processes sharing its home.
    pub async fn native_observation_guard(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.native_observations
            .account(&self.account_id)
            .lock_owned()
            .await
    }
}

fn native_only_environment(environment: &BTreeMap<String, String>) -> Result<()> {
    let home = environment
        .get("HOME")
        .context("native usage home missing")?
        .clone();
    // The signed Claude collector reads this account's own credential file,
    // read-only, for its reset-credit, prepaid-balance and plan-tier
    // enrichment and for an owner-confirmed reset, after the native CLI had
    // its chance to refresh the token. Native Claude must resolve the exact
    // same .claude home, never another account's or a shared one: any other
    // layout refuses the whole query. Other Providers' collectors read their
    // native home through HOME alone.
    if let Some(directory) = environment.get("CLAUDE_SECURESTORAGE_CONFIG_DIR") {
        ensure!(
            Path::new(directory) == Path::new(&home).join(".claude"),
            "native usage credential layout unsupported"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_home_guard_blocks_its_account_but_not_other_accounts_or_installations() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let first: ProviderAccountId = "11111111-1111-4111-8111-111111111111".parse().unwrap();
        let second: ProviderAccountId = "22222222-2222-4222-8222-222222222222".parse().unwrap();
        let first_store = store.for_account(&first).unwrap();
        let _logout = first_store.native_observation_guard().await;
        assert!(
            store
                .native_observations
                .account(&first)
                .try_lock_owned()
                .is_err()
        );
        assert!(
            store
                .for_account(&second)
                .unwrap()
                .native_observations
                .account(&second)
                .try_lock_owned()
                .is_ok()
        );
        assert!(store.lifecycle.try_lock().is_ok());
    }

    #[test]
    fn native_usage_cannot_use_a_shared_or_missing_credential_home() {
        let mut environment = BTreeMap::from([
            ("HOME".into(), "/accounts/otter/home".into()),
            (
                "CLAUDE_SECURESTORAGE_CONFIG_DIR".into(),
                "/accounts/lynx/.claude".into(),
            ),
        ]);
        assert!(native_only_environment(&environment).is_err());
        environment.insert(
            "CLAUDE_SECURESTORAGE_CONFIG_DIR".into(),
            "/accounts/otter/home/.claude".into(),
        );
        // The account's own credential directory is admitted, and stays in
        // the collector environment for its read-only enrichment.
        native_only_environment(&environment).unwrap();
        // A relative or nested spelling is not the account's .claude home.
        for other in [
            "accounts/otter/home/.claude",
            "/accounts/otter/home/.claude/x",
        ] {
            let mut spelled = environment.clone();
            spelled.insert("CLAUDE_SECURESTORAGE_CONFIG_DIR".into(), other.into());
            assert!(native_only_environment(&spelled).is_err(), "{other}");
        }
        // Providers without a separate credential directory read HOME alone.
        environment.remove("CLAUDE_SECURESTORAGE_CONFIG_DIR");
        native_only_environment(&environment).unwrap();
        environment.remove("HOME");
        assert!(native_only_environment(&environment).is_err());
    }

    #[tokio::test]
    async fn a_reset_that_cannot_be_admitted_never_starts_the_collector() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let profile: NativeProfileReceipt = serde_json::from_value(serde_json::json!({
            "account_id":"11111111-1111-4111-8111-111111111111",
            "provider_id":"claude-code","request_id":"login","generation":1,
            "method":"native","provider_version":"1","generation_digest":"digest",
            "auth_contract_fingerprint":"contract","identity":{"authority":"claude","subject":"otter"}
        }))
        .unwrap();
        let request = crate::machine_protocol::NativeResetRequest {
            provider: "anthropic".into(),
            idempotency_key: "cowboy-reset-1-2-3".into(),
            expected_credit_id: Some("grant:1".into()),
        };
        let invalid = crate::machine_protocol::NativeResetRequest {
            idempotency_key: "not a key".into(),
            ..request.clone()
        };
        for (request, code) in [
            (invalid, "native_reset_request_invalid"),
            // No verified login of this generation exists in its home.
            (request, "native_usage_profile_changed"),
        ] {
            let invocation = Invocation::new(profile.clone(), Arc::new(AtomicBool::new(true)));
            let failure = store
                .consume_native_reset(invocation, &request)
                .await
                .unwrap_err();
            assert!(!failure.started, "{code}");
            assert_eq!(failure.error.to_string(), code);
        }
    }

    #[tokio::test]
    async fn stale_connection_cannot_wait_for_a_new_admission() {
        let profile = serde_json::from_value(serde_json::json!({
            "account_id":"11111111-1111-4111-8111-111111111111",
            "provider_id":"claude-code","request_id":"login","generation":1,
            "method":"native","provider_version":"1","generation_digest":"digest",
            "auth_contract_fingerprint":"contract","identity":{"authority":"claude","subject":"otter"}
        })).unwrap();
        let connected = Arc::new(AtomicBool::new(true));
        let invocation = Invocation::new(profile, Arc::clone(&connected));
        assert!(invocation.remaining().is_ok());
        connected.store(false, Ordering::Release);
        assert!(invocation.remaining().is_err());
    }
}
