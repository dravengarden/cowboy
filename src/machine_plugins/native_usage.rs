//! Bounded observations in the native account's home; never a portable grant.
use super::*;
use crate::provider_account::NativeProfileReceipt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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
            "native usage connection or admission expired"
        );
        Ok(remaining)
    }
}

impl MachinePluginStore {
    pub(crate) async fn collect_native_usage(
        &self,
        invocation: Invocation,
    ) -> Result<serde_json::Value> {
        let _lifecycle =
            tokio::time::timeout(invocation.remaining()?, self.lifecycle.lock()).await?;
        invocation.remaining()?;
        let profile = &invocation.profile;
        self.validate_native_usage(profile)?;
        let mut payload = serde_json::json!({"operation":"collect"});
        let resolved = self.resolve_host_invocation(
            &profile.provider_id,
            &profile.provider_version,
            &profile.generation_digest,
            HostAuthentication::Native(profile),
            PluginHostOperation::CollectUsage,
            &mut payload,
        )?;
        let mut environment = resolved.environment;
        native_only_environment(&mut environment)?;
        invocation.remaining()?;
        let (program, arguments) = resolved
            .command
            .split_first()
            .context("native usage collector missing")?;
        let output = crate::plugin_process::run_plugin_command_with_environment_on_started(
            program,
            arguments,
            &serde_json::to_vec(&payload)?,
            &environment,
            || {},
        )
        .await
        .map_err(|error| error.error)?;
        invocation.remaining()?;
        self.validate_native_usage(profile)?;
        self.validate_host_installation(
            &profile.provider_id,
            &profile.generation_digest,
            resolved.installation_revision.as_ref(),
        )?;
        decode_host_response(&output).map_err(|error| error.error)
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
    pub(crate) async fn native_observation_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.lifecycle.lock().await
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
