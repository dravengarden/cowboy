//! Bounded observations in the native account's home; never a portable grant.
use super::*;
use crate::provider_account::{NativeProfileReceipt, ProviderAccountId};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Weak};

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

pub(crate) struct Invocation {
    profile: NativeProfileReceipt,
    connected: Arc<AtomicBool>,
    deadline: Instant,
}

impl Invocation {
    pub(crate) fn new(profile: NativeProfileReceipt, connected: Arc<AtomicBool>) -> Self {
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
    pub(crate) async fn collect_native_usage(
        &self,
        invocation: Invocation,
    ) -> Result<serde_json::Value> {
        let profile = &invocation.profile;
        let _observation = tokio::time::timeout(
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
        let resolved = self
            .resolve_host_invocation(
                &profile.provider_id,
                &profile.provider_version,
                &profile.generation_digest,
                HostAuthentication::Native(profile),
                PluginHostOperation::CollectUsage,
                &mut payload,
            )
            .context("native_usage_collector_unavailable")?;
        let mut environment = resolved.environment;
        native_only_environment(&mut environment).context("native_usage_home_unsupported")?;
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
            &profile.generation_digest,
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

    pub(super) fn validate_native_usage(&self, profile: &NativeProfileReceipt) -> Result<()> {
        ensure!(
            !profile.account_id.is_default() && profile.provider_id == "claude-code",
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
    pub(crate) async fn native_observation_guard(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.native_observations
            .account(&self.account_id)
            .lock_owned()
            .await
    }
}

fn native_only_environment(environment: &mut BTreeMap<String, String>) -> Result<()> {
    let home = environment
        .get("HOME")
        .context("native usage home missing")?;
    // The signed Claude collector's optional credits enrichment reads this
    // variable. Omit it so only native auth-status/get_usage accesses credentials.
    // Native Claude must resolve the exact same .claude home, never a fallback.
    ensure!(
        environment
            .get("CLAUDE_SECURESTORAGE_CONFIG_DIR")
            .is_some_and(|directory| Path::new(directory) == Path::new(home).join(".claude")),
        "native usage credential layout unsupported"
    );
    environment.remove("CLAUDE_SECURESTORAGE_CONFIG_DIR");
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
        assert!(native_only_environment(&mut environment).is_err());
        environment.insert(
            "CLAUDE_SECURESTORAGE_CONFIG_DIR".into(),
            "/accounts/otter/home/.claude".into(),
        );
        native_only_environment(&mut environment).unwrap();
        assert_eq!(environment["HOME"], "/accounts/otter/home");
        assert!(!environment.contains_key("CLAUDE_SECURESTORAGE_CONFIG_DIR"));
        assert!(native_only_environment(&mut environment).is_err());
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
