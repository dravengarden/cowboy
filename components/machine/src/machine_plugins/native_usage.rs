//! Bounded observations in the native account's home; never a portable grant.
use super::*;
use crate::provider_account::{NativeProfileReceipt, ProviderAccountId};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Weak};

mod portable;
mod sandbox;
pub use portable::{
    HostAdmission, PortableInvocation, PortableRenewalInvocation, PortableResetInvocation,
};
pub(in crate::machine_plugins) use portable::{PRIVATE_HOMES, remove_private_home};

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

enum PreparedNativeUsage {
    Sandboxed {
        active_digest: String,
    },
    DirectAntigravity {
        active_digest: String,
        proxy_command: PathBuf,
        home: PathBuf,
        network: Option<cowboy_provider_sdk::RuntimeNetworkPolicy>,
    },
    Direct {
        active_digest: String,
        resolved: Box<ResolvedPluginHostInvocation>,
        payload: serde_json::Value,
    },
}

impl MachinePluginStore {
    #[allow(clippy::too_many_lines)] // One fenced read from admission to the checked report.
    fn prepare_native_usage_blocking(
        &self,
        profile: &NativeProfileReceipt,
    ) -> Result<PreparedNativeUsage> {
        self.validate_native_usage(profile)
            .context("native_usage_profile_changed")?;
        let mut payload = serde_json::json!({"operation":"collect"});
        let (active, active_digest) = self
            .active_package(&profile.provider_id)?
            .context("native_usage_collector_unavailable")?;
        if account_sandbox::sandboxed(&active) {
            return Ok(PreparedNativeUsage::Sandboxed { active_digest });
        }
        if profile.provider_id == "antigravity" {
            let (_, package_path) = self
                .package_for_generation(&profile.provider_id, &active_digest)
                .context("native_usage_collector_unavailable")?;
            let platform = matching_payload(&active, &self.platform, &self.architecture)
                .context("native_usage_collector_unavailable")?;
            let proxy_component = platform
                .private_components
                .iter()
                .find(|component| component.command == "agy-proxy")
                .context("native_usage_collector_unavailable")?;
            let proxy_command = runtime_command(&package_path, &proxy_component.command)
                .context("native_usage_collector_unavailable")?;
            let home = self
                .for_account(&profile.account_id)
                .context("native_usage_profile_changed")?
                .native_profile_home(&active, profile.generation)
                .context("native_usage_profile_changed")?;
            return Ok(PreparedNativeUsage::DirectAntigravity {
                active_digest,
                proxy_command,
                home,
                network: active.manifest.runtime.network,
            });
        }
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
        verify_live_login(&active, profile, &resolved.environment)?;
        Ok(PreparedNativeUsage::Direct {
            active_digest,
            resolved: Box::new(resolved),
            payload,
        })
    }

    #[allow(clippy::too_many_lines)] // One fenced native usage collection flow.
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
            tokio::time::timeout(invocation.remaining()?, self.lock_lifecycle_owned())
                .await
                .context("native_usage_busy")?,
        );
        invocation.remaining()?;
        let mut observation = Some(observation);
        let prepared = {
            let store = self.clone();
            let profile_owned = profile.clone();
            let owned_obs = observation.take().context("native_usage_busy")?;
            let owned_life = lifecycle.take().context("native_usage_busy")?;
            let (result, owned_obs, owned_life) = tokio::task::spawn_blocking(move || {
                let result = store.prepare_native_usage_blocking(&profile_owned);
                (result, owned_obs, owned_life)
            })
            .await
            .context("native_usage_collector_unavailable")?;
            observation = Some(owned_obs);
            lifecycle = Some(owned_life);
            result?
        };
        let (active_digest, resolved, payload) = match prepared {
            PreparedNativeUsage::Sandboxed { active_digest } => {
                drop(lifecycle.take());
                let owned_obs = observation.take().context("native_usage_busy")?;
                let usage = self
                    .collect_sandbox_usage(&invocation, &active_digest, owned_obs)
                    .await?;
                self.verify_post_usage_active(&invocation, &active_digest)
                    .await?;
                return Ok(usage);
            }
            PreparedNativeUsage::DirectAntigravity {
                active_digest,
                proxy_command,
                home,
                network,
            } => {
                drop(lifecycle.take());
                let owned_obs = observation.take().context("native_usage_busy")?;
                let usage = self
                    .collect_direct_antigravity_usage(
                        &invocation,
                        &proxy_command,
                        &home,
                        network,
                        owned_obs,
                    )
                    .await?;
                self.verify_post_usage_active(&invocation, &active_digest)
                    .await?;
                return Ok(usage);
            }
            PreparedNativeUsage::Direct {
                active_digest,
                resolved,
                payload,
            } => (active_digest, *resolved, payload),
        };
        let environment = resolved.environment;
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
                tokio::time::timeout(invocation.remaining()?, self.lock_lifecycle_owned())
                    .await
                    .context("native_usage_busy")?,
            );
        }
        invocation.remaining()?;
        {
            let store = self.clone();
            let profile_owned = profile.clone();
            let digest_check = active_digest.clone();
            let revision = resolved.installation_revision.clone();
            let owned_obs = observation.take();
            let owned_life = lifecycle.take();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let _observation = owned_obs;
                let _lifecycle = owned_life;
                store
                    .validate_native_usage(&profile_owned)
                    .context("native_usage_profile_changed")?;
                store
                    .validate_host_installation(
                        &profile_owned.provider_id,
                        &digest_check,
                        revision.as_ref(),
                    )
                    .context("native_usage_installation_changed")?;
                Ok(())
            })
            .await
            .context("native_usage_installation_changed")??;
        }
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
    fn prepare_native_reset_step_blocking(
        &self,
        profile: &NativeProfileReceipt,
        dispatch: &crate::machine_protocol::NativeResetDispatch,
        operation: PluginHostOperation,
        mut payload: serde_json::Value,
    ) -> Result<(ResolvedPluginHostInvocation, serde_json::Value)> {
        let (active, active_digest) = self.selected_release(profile, dispatch)?;
        // An account-sandbox credential never leaves its daemon's custody.
        ensure!(
            !account_sandbox::sandboxed(&active),
            "native_reset_unsupported"
        );
        let resolved = self
            .resolve_host_invocation(
                &profile.provider_id,
                &active.manifest.version,
                &active_digest,
                HostAuthentication::Native(profile),
                operation,
                &mut payload,
            )
            .context("native_usage_collector_unavailable")?;
        verify_live_login(&active, profile, &resolved.environment)?;
        Ok((resolved, payload))
    }

    #[allow(clippy::too_many_lines)] // One fenced two-step native reset flow.
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
        // The two-start reset needs the Controller's spacing (protocol 51),
        // kept here from the identity read's actual start.
        let Some(spacing) = dispatch.min_spacing_ms.filter(|spacing| *spacing >= 0) else {
            return refuse("native_reset_unsupported");
        };
        let mut observation = Some(
            tokio::time::timeout(
                invocation.remaining()?,
                self.native_observations
                    .account(&profile.account_id)
                    .lock_owned(),
            )
            .await
            .context("native_usage_busy")?,
        );
        let mut lifecycle = Some(
            tokio::time::timeout(invocation.remaining()?, self.lock_lifecycle_owned())
                .await
                .context("native_usage_busy")?,
        );
        before_expiry(&invocation, dispatch)?;
        let (checked, check) = {
            let store = self.clone();
            let profile_owned = profile.clone();
            let dispatch_owned = dispatch.clone();
            let owned_obs = observation.take().context("native_usage_busy")?;
            let owned_life = lifecycle.take().context("native_usage_busy")?;
            let (result, owned_obs, owned_life) = tokio::task::spawn_blocking(move || {
                let result = store.prepare_native_reset_step_blocking(
                    &profile_owned,
                    &dispatch_owned,
                    PluginHostOperation::CollectUsage,
                    serde_json::json!({"operation": "collect"}),
                );
                (result, owned_obs, owned_life)
            })
            .await
            .context("native_usage_collector_unavailable")?;
            observation = Some(owned_obs);
            lifecycle = Some(owned_life);
            result?
        };
        if !checked
            .environment
            .contains_key("CLAUDE_SECURESTORAGE_CONFIG_DIR")
        {
            return refuse("native_reset_unsupported");
        }
        before_expiry(&invocation, dispatch)?;
        let (report, identity_started) = run_unstarted(&checked, &check).await?;
        verify_reported_identity(&checked.environment, &report)?;
        // The spend is the reset's second paced start: wait for its slot and
        // a full spacing after the identity read's process actually spawned
        // (monotonic), within the Controller's expiry.
        let wait = spend_wait(dispatch, spacing, identity_started, unix_ms())?;
        if !wait.is_zero() {
            // Never hold the installation lifecycle across the paced wait;
            // after it, the exact selected release must still be active.
            drop(lifecycle.take());
            tokio::time::sleep(wait).await;
            let remaining =
                u64::try_from(dispatch.expires_at_ms.saturating_sub(unix_ms())).unwrap_or(0);
            lifecycle = Some(
                tokio::time::timeout(
                    Duration::from_millis(remaining),
                    self.lock_lifecycle_owned(),
                )
                .await
                .context("native_usage_busy")?,
            );
        }
        before_expiry(&invocation, dispatch)?;
        let initial_payload = serde_json::to_value(request).map_err(anyhow::Error::from)?;
        let (resolved, payload) = {
            let store = self.clone();
            let profile_owned = profile.clone();
            let dispatch_owned = dispatch.clone();
            let owned_obs = observation.take().context("native_usage_busy")?;
            let owned_life = lifecycle.take().context("native_usage_busy")?;
            let (result, owned_obs, owned_life) = tokio::task::spawn_blocking(move || {
                let result = store.prepare_native_reset_step_blocking(
                    &profile_owned,
                    &dispatch_owned,
                    PluginHostOperation::ResetUsage,
                    initial_payload,
                );
                (result, owned_obs, owned_life)
            })
            .await
            .context("native_usage_collector_unavailable")?;
            observation = Some(owned_obs);
            lifecycle = Some(owned_life);
            result?
        };
        let _observation = observation;
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
            // A spend that provably never spawned is a pre-start refusal.
            error: anyhow::anyhow!(if failure.started {
                "native_reset_outcome_unknown"
            } else {
                "native_reset_preflight_failed"
            }),
        })?;
        // Held through the spend, as Service resets do.
        drop(lifecycle);
        // The collector answers every reset, refused or not, on stdout. Only
        // the closed projection leaves the Machine; never collector text.
        decode_host_response(&output)
            .map(|reply| crate::machine_protocol::project_native_reset_reply(&reply))
            .map_err(|_| PluginHostInvocationFailure {
                started: true,
                error: anyhow::anyhow!("native_reset_outcome_unknown"),
            })
    }

    /// The login is unchanged and exactly the release the Controller selected
    /// is active; the caller holds the installation lifecycle.
    fn selected_release(
        &self,
        profile: &NativeProfileReceipt,
        dispatch: &crate::machine_protocol::NativeResetDispatch,
    ) -> Result<(ProviderPackage, String)> {
        self.validate_native_usage(profile)
            .context("native_usage_profile_changed")?;
        let (active, active_digest) = self
            .active_package(&profile.provider_id)?
            .context("native_usage_collector_unavailable")?;
        ensure!(
            active.manifest.version == dispatch.plugin_version
                && active_digest == dispatch.generation_digest,
            "native_usage_installation_changed"
        );
        Ok((active, active_digest))
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

    async fn verify_post_usage_active(
        &self,
        invocation: &Invocation,
        active_digest: &str,
    ) -> Result<()> {
        let owned_life = tokio::time::timeout(invocation.remaining()?, self.lock_lifecycle_owned())
            .await
            .context("native_usage_busy")?;
        invocation.remaining()?;
        let store = self.clone();
        let profile_owned = invocation.profile.clone();
        let digest_check = active_digest.to_owned();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let _lifecycle = owned_life;
            store
                .validate_native_usage(&profile_owned)
                .context("native_usage_profile_changed")?;
            ensure!(
                store
                    .active_package(&profile_owned.provider_id)?
                    .is_some_and(|(_, digest)| digest == digest_check),
                "native_usage_installation_changed"
            );
            Ok(())
        })
        .await
        .context("native_usage_installation_changed")??;
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

/// When the spend may start: the Controller's paced slot and, when it sent the
/// spacing, a full spacing after the identity read actually started. A spend
/// that could then not start before the expiry is refused unsent.
/// How long the spend waits from `now_ms`: until the Controller's absolute
/// slot and a full `spacing` after the identity read spawned (monotonic).
/// Refused (unsent) if it could then not start before the dispatch expiry.
fn spend_wait(
    dispatch: &crate::machine_protocol::NativeResetDispatch,
    spacing: i64,
    identity_started: std::time::Instant,
    now_ms: i64,
) -> Result<Duration> {
    let millis = |value: i64| Duration::from_millis(u64::try_from(value).unwrap_or(0));
    let wait = millis(dispatch.spend_not_before_ms.saturating_sub(now_ms))
        .max(millis(spacing).saturating_sub(identity_started.elapsed()));
    let wait_ms = i64::try_from(wait.as_millis()).unwrap_or(i64::MAX);
    ensure!(
        now_ms.saturating_add(wait_ms) < dispatch.expires_at_ms,
        "native_reset_expired"
    );
    Ok(wait)
}

/// Refuse to start anything once the invocation or the Controller's dispatch
/// expiry has passed.
fn before_expiry(
    invocation: &Invocation,
    dispatch: &crate::machine_protocol::NativeResetDispatch,
) -> Result<()> {
    ensure!(unix_ms() < dispatch.expires_at_ms, "native_reset_expired");
    // The invocation's connection must still be current; its relative
    // deadline does not bound a paced wait the Controller granted.
    ensure!(
        invocation.connected.load(Ordering::Acquire),
        "native_usage_admission_expired"
    );
    Ok(())
}

pub(super) fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(i64::MAX, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// Run a read-only collector invocation that must not have reached anything
/// yet from the caller's point of view: any failure is "not started".
async fn run_unstarted(
    resolved: &ResolvedPluginHostInvocation,
    payload: &serde_json::Value,
) -> Result<(serde_json::Value, std::time::Instant)> {
    let (program, arguments) = resolved
        .command
        .split_first()
        .context("native_usage_collector_unavailable")?;
    // Captured when the process actually spawned; the spacing runs from it.
    let mut started = None;
    let output = crate::plugin_process::run_plugin_command_with_environment_on_started(
        program,
        arguments,
        &serde_json::to_vec(payload)?,
        &resolved.environment,
        || started = Some(std::time::Instant::now()),
    )
    .await
    .map_err(|_| anyhow::anyhow!("native_reset_preflight_failed"))?;
    let started = started.context("native_reset_preflight_failed")?;
    // An authentication failure keeps its category so the Service backs off.
    decode_host_response(&output)
        .map_err(|failure| {
            if failure
                .error
                .to_string()
                .contains("Claude Code usage authentication required.")
            {
                anyhow::anyhow!("native_usage_authentication_required")
            } else {
                // The identity read failed before any spend: unsent.
                anyhow::anyhow!("native_reset_preflight_failed")
            }
        })
        .map(|report| (report, started))
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
    // Streamed with bounded memory: the config grows with projects and MCP.
    let path = |leaf: &str| vec!["oauthAccount".to_owned(), leaf.to_owned()];
    let (email_path, organization_path) = (path("emailAddress"), path("organizationName"));
    let login =
        native_identity::read_live_fields(home, ".claude.json", &[&email_path, &organization_path])
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
    let live = native_identity::read_live(home, recipe, &profile.account_id)
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
            spend_not_before_ms: 0,
            min_spacing_ms: Some(0),
        };
        let unspaced = crate::machine_protocol::NativeResetDispatch {
            min_spacing_ms: None,
            ..dispatch.clone()
        };
        for (request, dispatch, code) in [
            (invalid, &dispatch, "native_reset_request_invalid"),
            // Without an enforceable spacing (protocol 51) nothing starts.
            (request.clone(), &unspaced, "native_reset_unsupported"),
            // No verified login of this generation exists in its home.
            (request, &dispatch, "native_usage_profile_changed"),
        ] {
            let invocation = Invocation::new(profile.clone(), Arc::new(AtomicBool::new(true)));
            let failure = store
                .consume_native_reset(invocation, &request, dispatch)
                .await
                .unwrap_err();
            assert!(!failure.started, "{code}");
            assert_eq!(failure.error.to_string(), code);
        }
    }

    /// The spend keeps a full spacing from the identity read's actual start
    /// and is refused (unsent) if it could then not start before expiry.
    #[test]
    fn the_spend_keeps_the_spacing_from_the_actual_identity_read() {
        let dispatch =
            |expires_at_ms, spend_not_before_ms| crate::machine_protocol::NativeResetDispatch {
                plugin_version: "1".into(),
                generation_digest: "digest".into(),
                expires_at_ms,
                spend_not_before_ms,
                min_spacing_ms: Some(60_000),
            };
        let now = 1_000_000;
        let started = std::time::Instant::now();
        // An identity read that spawned late moves the spend past the slot.
        let wait = spend_wait(&dispatch(i64::MAX, now), 60_000, started, now).unwrap();
        assert!(wait > Duration::from_secs(59) && wait <= Duration::from_mins(1));
        // A later absolute slot still wins.
        assert_eq!(
            spend_wait(&dispatch(i64::MAX, now + 90_000), 60_000, started, now).unwrap(),
            Duration::from_secs(90)
        );
        // Spacing already elapsed since the spawn: no wait.
        let earlier = started.checked_sub(Duration::from_secs(61)).unwrap();
        assert_eq!(
            spend_wait(&dispatch(i64::MAX, now), 60_000, earlier, now).unwrap(),
            Duration::ZERO
        );
        assert_eq!(
            spend_wait(&dispatch(now + 30_000, now), 60_000, started, now)
                .unwrap_err()
                .to_string(),
            "native_reset_expired"
        );
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
            spend_not_before_ms: 0,
            min_spacing_ms: None,
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
