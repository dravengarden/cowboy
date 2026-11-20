//! Bounded observations in the native account's home; never a portable grant.
use super::*;
use crate::provider_account::{NativeProfileReceipt, ProviderAccountId};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Weak};

mod portable;
mod sandbox;
pub use portable::{HostAdmission, PortableInvocation};

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
    #[allow(clippy::too_many_lines)] // One fenced read from admission to the checked report.
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
        verify_live_login(&active, profile, &environment)?;
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
        // The account the collector read from the credential must be the
        // login's; otherwise nothing is published.
        decode_host_response(&output)
            .map_err(|error| {
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
            .and_then(|usage| verify_reported_identity(&environment, &usage).map(|()| usage))
    }

    /// Spend one banked usage reset the owner confirmed, through the same
    /// signed collector, CLI identity, account home guard and deadline as
    /// [`Self::collect_native_usage`], on exactly the release the Controller
    /// selected (checked under the lifecycle lock, held until the reset ends
    /// as Service resets do). In the same invocation, immediately before the
    /// spend, the collector's own report must name the login's account. No
    /// collector starts after the Controller's dispatch expiry. `started` on a
    /// failure means the collector may have reached the Provider; such an
    /// outcome is unknown and never retried. Only the closed reply leaves.
    pub async fn consume_native_reset(
        &self,
        invocation: Invocation,
        request: &crate::machine_protocol::NativeResetRequest,
        dispatch: &crate::machine_protocol::NativeResetDispatch,
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
        if active.manifest.version != dispatch.plugin_version
            || active_digest != dispatch.generation_digest
        {
            return refuse("native_usage_installation_changed");
        }
        // An account-sandbox credential never leaves its daemon's custody.
        if account_sandbox::sandboxed(&active) {
            return refuse("native_reset_unsupported");
        }
        let resolve = |operation, payload: &mut serde_json::Value| {
            self.resolve_host_invocation(
                &profile.provider_id,
                &active.manifest.version,
                &active_digest,
                HostAuthentication::Native(profile),
                operation,
                payload,
            )
            .context("native_usage_collector_unavailable")
        };
        let mut check = serde_json::json!({"operation": "collect"});
        let checked = resolve(PluginHostOperation::CollectUsage, &mut check)?;
        verify_live_login(&active, profile, &checked.environment)?;
        if !checked
            .environment
            .contains_key("CLAUDE_SECURESTORAGE_CONFIG_DIR")
        {
            return refuse("native_reset_unsupported");
        }
        before_expiry(&invocation, dispatch)?;
        let report = run_unstarted(&checked, &check).await?;
        verify_reported_identity(&checked.environment, &report)?;
        let mut payload = serde_json::to_value(request).map_err(anyhow::Error::from)?;
        let resolved = resolve(PluginHostOperation::ResetUsage, &mut payload)?;
        verify_live_login(&active, profile, &resolved.environment)?;
        before_expiry(&invocation, dispatch)?;
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
        // the closed projection leaves the Machine; never collector text.
        decode_host_response(&output)
            .map(|reply| crate::machine_protocol::project_native_reset_reply(&reply))
            .map_err(|_| PluginHostInvocationFailure {
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

/// Refuse to start anything once the invocation or the Controller's dispatch
/// expiry has passed.
fn before_expiry(
    invocation: &Invocation,
    dispatch: &crate::machine_protocol::NativeResetDispatch,
) -> Result<()> {
    invocation.remaining()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(i64::MAX, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        });
    ensure!(now < dispatch.expires_at_ms, "native_reset_expired");
    Ok(())
}

/// Run a read-only collector invocation that must not have reached anything
/// yet from the caller's point of view: any failure is "not started".
async fn run_unstarted(
    resolved: &ResolvedPluginHostInvocation,
    payload: &serde_json::Value,
) -> Result<serde_json::Value> {
    let (program, arguments) = resolved
        .command
        .split_first()
        .context("native_usage_collector_unavailable")?;
    let output = crate::plugin_process::run_plugin_command_with_environment_on_started(
        program,
        arguments,
        &serde_json::to_vec(payload)?,
        &resolved.environment,
        || {},
    )
    .await
    .map_err(|_| anyhow::anyhow!("native_usage_collector_failed"))?;
    decode_host_response(&output).map_err(|_| anyhow::anyhow!("native_usage_profile_changed"))
}

fn text(value: &serde_json::Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|v| !v.is_empty())
}

/// The account the collector reports from the credential (its auth status:
/// email and organization) must be the account the native home's login
/// metadata names, whose stable IDs [`verify_live_login`] matched to the
/// receipt. A report without an email is not evidence and is refused.
fn verify_reported_identity(
    environment: &BTreeMap<String, String>,
    report: &serde_json::Value,
) -> Result<()> {
    // Only a collector that reads the credential itself (Claude's
    // enrichment and reset) reports through this check.
    if !environment.contains_key("CLAUDE_SECURESTORAGE_CONFIG_DIR") {
        return Ok(());
    }
    let home = Path::new(
        environment
            .get("HOME")
            .context("native_usage_home_unsupported")?,
    );
    let login: serde_json::Value = serde_json::from_slice(
        &native_identity::read_login_file(home, ".claude.json", 256 * 1024)
            .map_err(|_| anyhow::anyhow!("native_usage_profile_changed"))?,
    )
    .map_err(|_| anyhow::anyhow!("native_usage_profile_changed"))?;
    let reported = &report["account"]["account"];
    let email = text(&login["oauthAccount"]["emailAddress"]);
    ensure!(
        email.is_some()
            && text(&reported["email"]).map(str::to_ascii_lowercase)
                == email.map(str::to_ascii_lowercase),
        "native_usage_profile_changed"
    );
    if let (Some(reported), Some(expected)) = (
        text(&reported["organization"]),
        text(&login["oauthAccount"]["organizationName"]),
    ) {
        ensure!(reported == expected, "native_usage_profile_changed");
    }
    Ok(())
}

/// The native home must still hold exactly the receipt's login before a
/// collector reads its credential or spends a reset: every declared credential
/// file present there is a private regular file, checked from descriptor
/// metadata only (never read into the Machine), and the login's signed
/// identity recipe reads exactly the receipt's identity from the live home's
/// login metadata (another login in the same home is refused).
fn verify_live_login(
    active: &ProviderPackage,
    profile: &NativeProfileReceipt,
    environment: &BTreeMap<String, String>,
) -> Result<()> {
    native_only_environment(environment).context("native_usage_home_unsupported")?;
    let home = Path::new(
        environment
            .get("HOME")
            .context("native_usage_home_unsupported")?,
    );
    let auth = &active.manifest.authentication;
    for file in &auth.credential_files {
        native_identity::credential_file_safe(home, &file.relative_path)
            .map_err(|_| anyhow::anyhow!("native_usage_home_unsupported"))?;
    }
    let recipe = auth
        .methods
        .iter()
        .find(|method| method.id == profile.method)
        .and_then(|method| method.native_identity.as_ref())
        .context("native_usage_profile_changed")?;
    let live = native_identity::read(home, recipe, &profile.account_id)
        .map_err(|_| anyhow::anyhow!("native_usage_profile_changed"))?;
    ensure!(live == profile.identity, "native_usage_profile_changed");
    Ok(())
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

    #[test]
    fn a_linked_credential_or_another_login_in_the_home_is_refused() {
        let source: cowboy_provider_sdk::StandardProviderSource = serde_json::from_str(
            include_str!("../../../../plugins/claude-code/provider.json"),
        )
        .unwrap();
        let package = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
        let profile: NativeProfileReceipt = serde_json::from_value(serde_json::json!({
            "account_id":"11111111-1111-4111-8111-111111111111",
            "provider_id":"claude-code","request_id":"login","generation":1,
            "method":"claude-subscription","provider_version":"1","generation_digest":"digest",
            "auth_contract_fingerprint":"contract",
            "identity":{"authority":"claude-ai","subject":"otter","tenant":"org-a"}
        }))
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(home.join(".claude")).unwrap();
        let login = |subject: &str| {
            fs::write(
                home.join(".claude.json"),
                serde_json::to_vec(&serde_json::json!({"oauthAccount":
                    {"accountUuid": subject, "organizationUuid": "org-a"}}))
                .unwrap(),
            )
            .unwrap();
        };
        let environment =
            BTreeMap::from([("HOME".to_owned(), home.to_string_lossy().into_owned())]);
        login("otter");
        fs::write(home.join(".claude/.credentials.json"), b"{}").unwrap();
        verify_live_login(&package, &profile, &environment).unwrap();
        // Another login in the same home.
        login("lynx");
        assert_eq!(
            verify_live_login(&package, &profile, &environment)
                .unwrap_err()
                .to_string(),
            "native_usage_profile_changed"
        );
        login("otter");
        // The collector's report must name the login's account.
        let claude = BTreeMap::from([
            ("HOME".to_owned(), home.to_string_lossy().into_owned()),
            (
                "CLAUDE_SECURESTORAGE_CONFIG_DIR".to_owned(),
                home.join(".claude").to_string_lossy().into_owned(),
            ),
        ]);
        fs::write(
            home.join(".claude.json"),
            serde_json::to_vec(&serde_json::json!({"oauthAccount": {"accountUuid": "otter",
                "organizationUuid": "org-a", "emailAddress": "Otter@example.com",
                "organizationName": "Otter Org"}}))
            .unwrap(),
        )
        .unwrap();
        let report = |email: Option<&str>| serde_json::json!({"account": {"account": {"email": email, "organization": "Otter Org"}}});
        verify_reported_identity(&claude, &report(Some("otter@example.com"))).unwrap();
        for wrong in [Some("lynx@example.com"), None] {
            assert_eq!(
                verify_reported_identity(&claude, &report(wrong))
                    .unwrap_err()
                    .to_string(),
                "native_usage_profile_changed"
            );
        }
        // A credential linked to another subscription's file.
        let other = dir.path().join("other-credentials.json");
        fs::write(&other, b"{}").unwrap();
        fs::remove_file(home.join(".claude/.credentials.json")).unwrap();
        std::os::unix::fs::symlink(&other, home.join(".claude/.credentials.json")).unwrap();
        assert_eq!(
            verify_live_login(&package, &profile, &environment)
                .unwrap_err()
                .to_string(),
            "native_usage_home_unsupported"
        );
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
        let dispatch = crate::machine_protocol::NativeResetDispatch {
            plugin_version: "1".into(),
            generation_digest: "digest".into(),
            expires_at_ms: i64::MAX,
        };
        for (request, code) in [
            (invalid, "native_reset_request_invalid"),
            // No verified login of this generation exists in its home.
            (request, "native_usage_profile_changed"),
        ] {
            let invocation = Invocation::new(profile.clone(), Arc::new(AtomicBool::new(true)));
            let failure = store
                .consume_native_reset(invocation, &request, &dispatch)
                .await
                .unwrap_err();
            assert!(!failure.started, "{code}");
            assert_eq!(failure.error.to_string(), code);
        }
    }

    #[test]
    fn nothing_starts_after_the_controller_dispatch_expiry() {
        let profile: NativeProfileReceipt = serde_json::from_value(serde_json::json!({
            "account_id":"11111111-1111-4111-8111-111111111111",
            "provider_id":"claude-code","request_id":"login","generation":1,
            "method":"native","provider_version":"1","generation_digest":"digest",
            "auth_contract_fingerprint":"contract","identity":{"authority":"claude","subject":"otter"}
        }))
        .unwrap();
        let invocation = Invocation::new(profile, Arc::new(AtomicBool::new(true)));
        let dispatch = |expires_at_ms| crate::machine_protocol::NativeResetDispatch {
            plugin_version: "1".into(),
            generation_digest: "digest".into(),
            expires_at_ms,
        };
        before_expiry(&invocation, &dispatch(i64::MAX)).unwrap();
        assert_eq!(
            before_expiry(&invocation, &dispatch(1))
                .unwrap_err()
                .to_string(),
            "native_reset_expired"
        );
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
