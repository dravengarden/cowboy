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
    PortableRenewalRequest, PortableResetRequest, PortableUsageRequest, REFRESH_FREE_DECLARATION,
    RefreshFree, portable_usage_error_code,
};

/// Every portable invocation must start within this bound of its receipt: the
/// Controller abandons its wait after 45 seconds.
const DEADLINE: Duration = Duration::from_secs(30);

/// The Controller connection, receipt deadline and checked sidecar set one
/// invocation was admitted under; nothing starts after either has passed.
pub struct HostAdmission {
    connected: Arc<AtomicBool>,
    deadline: Instant,
    sidecars: Option<BTreeMap<String, String>>,
    /// The Controller's absolute expiry (Unix ms): rechecked immediately
    /// before the collector starts, since a monotonic deadline cannot see
    /// the wall clock the Controller set it by.
    expires_at_unix_ms: Option<i64>,
    /// Stage a private credential home of this replica generation for the
    /// collector, under the lifecycle lock after the exact release resolved;
    /// `true` when the invocation cannot run without one (a reset).
    private_home: Option<(u64, bool)>,
}

impl HostAdmission {
    fn new(connected: Arc<AtomicBool>) -> Self {
        Self {
            connected,
            deadline: Instant::now() + DEADLINE,
            sidecars: None,
            expires_at_unix_ms: None,
            private_home: None,
        }
    }

    /// An admission that stages a private home of `generation`.
    #[cfg(test)]
    pub(in crate::machine_plugins) fn with_private_home(generation: u64) -> Self {
        let mut admission = Self::new(Arc::new(AtomicBool::new(true)));
        admission.private_home = Some((generation, true));
        admission
    }

    pub(in crate::machine_plugins) fn remaining(&self) -> Result<Duration> {
        ensure!(
            self.connected.load(Ordering::Acquire),
            "machine_connection_changed"
        );
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "portable_usage_busy");
        if let Some(expires) = self.expires_at_unix_ms {
            ensure!(super::unix_ms() < expires, "native_reset_expired");
        }
        Ok(remaining)
    }

    /// Stage this invocation's private credential home, if it uses one: the
    /// caller holds the lifecycle lock and resolved the exact release, so an
    /// uninstall either finished first or sees the copy and refuses.
    ///
    /// # Errors
    /// A pre-start refusal: the replica cannot be staged, or a required home
    /// does not apply to this Provider.
    pub(in crate::machine_plugins) fn stage_private_home(
        &self,
        store: &MachinePluginStore,
        plugin_id: &str,
    ) -> std::result::Result<StagedHome, PluginHostInvocationFailure> {
        let Some((generation, required)) = self.private_home else {
            return Ok(None);
        };
        match store.private_credential_home(plugin_id, generation) {
            Ok(Some(home)) => Ok(Some(home)),
            Ok(None) if !required => Ok(None),
            Ok(None) => Err(anyhow::anyhow!("native_reset_unsupported").into()),
            Err(_) => Err(anyhow::anyhow!("portable_usage_replica_pending").into()),
        }
    }

    /// The absolute instant after which nothing of this invocation starts.
    pub(in crate::machine_plugins) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// The only sidecar Plugin generations this invocation may start.
    pub(in crate::machine_plugins) fn sidecars(&self) -> Option<&BTreeMap<String, String>> {
        self.sidecars.as_ref()
    }
}

/// Remove an invocation's private credential copy once its command ended. A
/// read fails if the copy outlives it (no observation is acknowledged while
/// the credential is left behind); a reset's outcome is reported whatever
/// happens to the copy, which is then removed at Machine start.
///
/// # Errors
/// A read whose copy could not be removed.
pub(in crate::machine_plugins) fn remove_private_home(
    private: Option<PrivateHome>,
    operation: PluginHostOperation,
) -> std::result::Result<(), PluginHostInvocationFailure> {
    match private.map(PrivateHome::remove) {
        Some(Err(error)) if operation == PluginHostOperation::ResetUsage => {
            tracing::error!(%error, "a private credential copy outlived its reset");
            Ok(())
        }
        Some(Err(_)) => Err(PluginHostInvocationFailure {
            started: true,
            error: anyhow::anyhow!("portable_usage_collector_failed"),
        }),
        _ => Ok(()),
    }
}

/// A staged private credential home and the variables pointing into it.
type StagedHome = Option<(PrivateHome, BTreeMap<String, String>)>;

/// One immediate banked reset of a named portable account; see
/// [`MachinePluginStore::consume_portable_reset`].
pub struct PortableResetInvocation {
    request: PortableResetRequest,
    connected: Arc<AtomicBool>,
}

impl PortableResetInvocation {
    pub fn new(request: PortableResetRequest, connected: Arc<AtomicBool>) -> Self {
        Self { request, connected }
    }
}

/// Directory, beside an account's replicas, of private credential copies.
pub(in crate::machine_plugins) const PRIVATE_HOMES: &str = "private-homes";

/// A private credential copy; [`Self::remove`] reports whether it is gone.
pub(in crate::machine_plugins) struct PrivateHome(Option<PathBuf>);

impl PrivateHome {
    pub(in crate::machine_plugins) fn path(&self) -> &Path {
        self.0.as_deref().unwrap_or(Path::new("/nonexistent"))
    }

    /// Remove the copy.
    ///
    /// # Errors
    /// If it cannot be removed (it is then retried at Machine start).
    pub(in crate::machine_plugins) fn remove(mut self) -> Result<()> {
        match self.0.take() {
            Some(path) => fs::remove_dir_all(&path)
                .or_else(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })
                .with_context(|| format!("removing private credential copy {}", path.display())),
            None => Ok(()),
        }
    }
}

impl Drop for PrivateHome {
    fn drop(&mut self) {
        if let Some(path) = self.0.take()
            && let Err(error) = fs::remove_dir_all(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::error!(path = %path.display(), %error,
                "a private credential copy could not be removed; it is removed at Machine start");
        }
    }
}

/// One idle renewal of a named portable account's sign-in; see
/// [`MachinePluginStore::renew_portable_credential`].
pub struct PortableRenewalInvocation {
    request: PortableRenewalRequest,
    connected: Arc<AtomicBool>,
}

impl PortableRenewalInvocation {
    pub fn new(request: PortableRenewalRequest, connected: Arc<AtomicBool>) -> Self {
        Self { request, connected }
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
    async fn admit_portable(
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
        let host = verified_plugin_host_bundle(&plugin, &release, &content)
            .ok()
            .flatten();
        let declared = RefreshFree::from_declaration(
            host.as_ref()
                .and_then(|host| host.files.get(REFRESH_FREE_DECLARATION))
                .map(String::as_str),
        )
        .usage;
        ensure!(declared, "portable_usage_refresh_unsupported");
        Ok(())
    }

    /// For a Provider whose CLI rotates credentials, a private home holding
    /// exactly the account replica's sealed credential at `generation`: a
    /// live session can rewrite its writable runtime home (even with another
    /// sign-in) without advancing the sealed generation, so neither a read
    /// nor a reset ever runs from it. `None` keeps the session environment
    /// (a non-rotating key, such as a `DeepSeek` API key).
    pub(in crate::machine_plugins) fn private_credential_home(
        &self,
        plugin_id: &str,
        generation: u64,
    ) -> Result<Option<(PrivateHome, BTreeMap<String, String>)>> {
        let (package, _) = self
            .active_package(plugin_id)?
            .context("portable_usage_collector_unavailable")?;
        let auth = &package.manifest.authentication;
        if auth.refresh != cowboy_provider_sdk::RefreshOwnership::CompareAndSwap
            || auth.credential_files.is_empty()
        {
            return Ok(None);
        }
        let envelope = self
            .latest_auth_envelope(plugin_id)?
            .filter(|replica| {
                replica.auth_generation == generation && replica.action == ProviderAuthAction::Apply
            })
            .context("portable_usage_replica_pending")?;
        let PreparedProviderAuth::Apply(bundle) =
            self.prepare_auth_inner(&envelope, Some(&package))?
        else {
            anyhow::bail!("portable_usage_replica_pending");
        };
        // Account-owned (removed with the account's credentials and at
        // Machine start), never the shared system temporary directory. Each
        // level is created parent-first and a symlink is refused.
        let parent = self.prepare_provider_root(plugin_id)?.join(PRIVATE_HOMES);
        let root = parent.join(format!("{:032x}", rand::random::<u128>()));
        crate::provider_account::prepare_private_directory(&parent)?;
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let private = PrivateHome(Some(root));
        restore_projected_bundle(auth, private.path(), &bundle)?;
        let home = private.path().join("home");
        let environment = BTreeMap::from([
            ("HOME".to_owned(), home.display().to_string()),
            (
                "XDG_CONFIG_HOME".to_owned(),
                home.join(".config").display().to_string(),
            ),
            (
                "XDG_DATA_HOME".to_owned(),
                home.join(".local/share").display().to_string(),
            ),
            (
                "XDG_CACHE_HOME".to_owned(),
                home.join(".cache").display().to_string(),
            ),
        ]);
        Ok(Some((private, environment)))
    }

    /// The account's real, private credential directory of `plugin_id`,
    /// created parent-first; a symlink at either level is refused, so nothing
    /// decrypted is ever written through a linked directory.
    ///
    /// # Errors
    /// If a level is a symlink or not a directory, or cannot be created.
    pub(in crate::machine_plugins) fn prepare_provider_root(
        &self,
        plugin_id: &str,
    ) -> Result<PathBuf> {
        let provider = self.auth_provider_root(plugin_id);
        for directory in [self.auth_watch_root(), provider.clone()] {
            crate::provider_account::prepare_private_directory(&directory)
                .context("Provider credential directory must be a real private directory")?;
        }
        Ok(provider)
    }

    /// Remove every named account's private credential copies left by an
    /// interrupted run. The Machine runs this before it accepts work. Only
    /// real named-account provider directories are traversed, from opened
    /// directory descriptors; a symlink at any level is never followed.
    ///
    /// # Errors
    /// If a directory cannot be enumerated or any copy cannot be removed.
    pub fn remove_private_homes(&self) -> Result<()> {
        let scoped = (!self.account_id.is_default()).then(|| self.account_id.as_str());
        remove_projection::remove_below_named_providers(
            &self.auth_base,
            &|name| {
                scoped.map_or_else(
                    || {
                        crate::provider_account::ProviderAccountId::try_from(name.to_owned())
                            .is_ok_and(|id| !id.is_default())
                    },
                    |only| only == name,
                )
            },
            (PRIVATE_HOMES, None),
            &|| Ok(()),
        )
        .context("removing private credential copies")
    }

    /// The refresh-free declaration of an exact verified generation.
    fn refresh_free(&self, plugin_id: &str, digest: &str) -> RefreshFree {
        let Ok((plugin, release, content)) = self.verified_plugin_generation(plugin_id, digest)
        else {
            return RefreshFree::default();
        };
        let host = verified_plugin_host_bundle(&plugin, &release, &content)
            .ok()
            .flatten();
        RefreshFree::from_declaration(
            host.as_ref()
                .and_then(|host| host.files.get(REFRESH_FREE_DECLARATION))
                .map(String::as_str),
        )
    }

    /// Spend one owner-confirmed banked reset of a named portable account:
    /// the exact selected release's collector runs `consume_reset` with
    /// `"token_refresh": false` under the admission of its usage (account
    /// scope, current replica generation, account credential guard), and the
    /// release must declare that this path never redeems a refresh token.
    /// Nothing starts after `expires_at_ms`. Only the closed reset reply
    /// leaves the Machine; a failure after start is unknown, never retried.
    pub async fn consume_portable_reset(
        &self,
        invocation: PortableResetInvocation,
    ) -> std::result::Result<serde_json::Value, PluginHostInvocationFailure> {
        let PortableResetInvocation { request, connected } = invocation;
        let refuse = |code: &'static str| PluginHostInvocationFailure {
            started: false,
            error: anyhow::anyhow!(code),
        };
        let remaining = request.expires_at_ms.saturating_sub(super::unix_ms());
        if !request.reset.valid() || remaining <= 0 {
            return Err(refuse(if remaining <= 0 {
                "native_reset_expired"
            } else {
                "native_reset_request_invalid"
            }));
        }
        let mut admission = HostAdmission::new(connected);
        admission.deadline = admission
            .deadline
            .min(Instant::now() + Duration::from_millis(u64::try_from(remaining).unwrap_or(0)));
        admission.expires_at_unix_ms = Some(request.expires_at_ms);
        let usage = PortableUsageRequest {
            account_id: request.account_id.clone(),
            plugin_id: request.plugin_id.clone(),
            plugin_version: request.plugin_version.clone(),
            generation_digest: request.generation_digest.clone(),
            auth_generation: request.auth_generation,
            payload: serde_json::Value::Null,
            sidecars: Vec::new(),
        };
        let (scoped, _guard) = self
            .admit_portable(&usage, &admission)
            .await
            .map_err(|error| PluginHostInvocationFailure {
                started: false,
                error: anyhow::anyhow!(pre_spend_code(&format!("{error:#}"))),
            })?;
        if !self
            .refresh_free(&request.plugin_id, &request.generation_digest)
            .reset
        {
            return Err(refuse("native_reset_unsupported"));
        }
        admission.sidecars = Some(BTreeMap::new());
        admission.private_home = Some((request.auth_generation, true));
        let mut payload = serde_json::to_value(&request.reset).map_err(anyhow::Error::from)?;
        if let Some(object) = payload.as_object_mut() {
            object.insert("token_refresh".into(), serde_json::Value::Bool(false));
        }
        // The Service host path resolves exactly the selected release under
        // the lifecycle lock and keeps it through the reset.
        scoped
            .invoke_executable_host_admitted(
                PluginHostRequest {
                    plugin_id: request.plugin_id,
                    plugin_version: request.plugin_version,
                    generation_digest: request.generation_digest,
                    auth_generation: Some(request.auth_generation),
                    operation: PluginHostOperation::ResetUsage,
                    payload,
                },
                Some(&admission),
            )
            .await
            .map(|reply| crate::machine_protocol::project_native_reset_reply(&reply))
            .map_err(|failure| PluginHostInvocationFailure {
                started: failure.started,
                error: anyhow::anyhow!(if failure.started {
                    "native_reset_outcome_unknown"
                } else {
                    pre_spend_code(&format!("{:#}", failure.error))
                }),
            })
    }

    /// Renew one named portable account's sign-in while no session of it
    /// runs (the Controller admits it only then). The exact selected release
    /// must declare idle renewal; its collector runs `renew` in the account's
    /// own session home (the writable runtime projection a session launches
    /// with, never a private copy) under the account's credential guard and
    /// the lifecycle lock, so the Provider CLI refreshes exactly as a
    /// session's startup would, and with the same file lock a concurrent
    /// session on this Machine shares. A refreshed credential leaves only
    /// through the ordinary refresh observation and the Service
    /// compare-and-swap, which validates its identity; this reply carries only
    /// `renewed` (the session home now holds a credential the vault does not)
    /// or `unchanged`. Nothing starts after `expires_at_ms`.
    pub async fn renew_portable_credential(
        &self,
        invocation: PortableRenewalInvocation,
    ) -> std::result::Result<serde_json::Value, PluginHostInvocationFailure> {
        let PortableRenewalInvocation { request, connected } = invocation;
        let refuse = |code: &'static str| PluginHostInvocationFailure {
            started: false,
            error: anyhow::anyhow!(code),
        };
        let remaining = request.expires_at_ms.saturating_sub(super::unix_ms());
        if remaining <= 0 {
            return Err(refuse("portable_usage_busy"));
        }
        let mut admission = HostAdmission::new(connected);
        admission.deadline = admission
            .deadline
            .min(Instant::now() + Duration::from_millis(u64::try_from(remaining).unwrap_or(0)));
        admission.expires_at_unix_ms = Some(request.expires_at_ms);
        let usage = PortableUsageRequest {
            account_id: request.account_id.clone(),
            plugin_id: request.plugin_id.clone(),
            plugin_version: request.plugin_version.clone(),
            generation_digest: request.generation_digest.clone(),
            auth_generation: request.auth_generation,
            payload: serde_json::Value::Null,
            sidecars: Vec::new(),
        };
        let (scoped, _guard) = self
            .admit_portable(&usage, &admission)
            .await
            .map_err(|error| refuse(portable_usage_error_code(&format!("{error:#}"))))?;
        if !self
            .refresh_free(&request.plugin_id, &request.generation_digest)
            .renewal
        {
            return Err(refuse("portable_renewal_unsupported"));
        }
        // The session homes must hold exactly the admitted sealed credential:
        // a home with an uncommitted refresh (or another sign-in) is the
        // ordinary refresh observation's to reconcile, never this renewal's
        // to spend, and only then is a later candidate this run's.
        let (account, plugin) = (request.account_id.clone(), request.plugin_id.clone());
        if scoped
            .pending_refresh(account.clone(), plugin.clone())
            .await
            .map_or(true, |(candidate, unreadable)| candidate || unreadable)
        {
            return Err(refuse("portable_usage_replica_pending"));
        }
        // No sidecar and no private home: the session environment.
        admission.sidecars = Some(BTreeMap::new());
        let ran = scoped
            .invoke_executable_host_admitted(
                PluginHostRequest {
                    plugin_id: request.plugin_id.clone(),
                    plugin_version: request.plugin_version,
                    generation_digest: request.generation_digest,
                    auth_generation: Some(request.auth_generation),
                    operation: PluginHostOperation::RenewCredential,
                    payload: serde_json::json!({"token_refresh": "renew"}),
                },
                Some(&admission),
            )
            .await
            .map_err(|failure| PluginHostInvocationFailure {
                started: failure.started,
                error: anyhow::anyhow!(portable_usage_error_code(&format!("{:#}", failure.error))),
            });
        if let Err(failure) = &ran
            && !failure.started
        {
            return ran;
        }
        // Whatever the collector reported once it started, the session home
        // decides: the CLI may have renewed before a later read failed, and
        // that credential reaches the vault through the refresh observation
        // either way. No candidate existed before the run.
        let (renewed, unreadable) =
            scoped
                .pending_refresh(account, plugin)
                .await
                .map_err(|error| PluginHostInvocationFailure {
                    started: true,
                    error,
                })?;
        match ran {
            // The rotation and a throttled billing read are reported apart, so
            // the Controller still paces the Provider.
            Err(failure) if renewed && failure.error.to_string() == "portable_usage_throttled" => {
                Ok(serde_json::json!({"outcome": "renewed", "billing": "portable_usage_throttled"}))
            }
            _ if renewed => Ok(serde_json::json!({"outcome": "renewed"})),
            // A home the CLI may have rewritten cannot be read: indeterminate.
            _ if unreadable => Err(PluginHostInvocationFailure {
                started: true,
                error: anyhow::anyhow!("portable_usage_collector_failed"),
            }),
            Ok(reply) if reply["renewal"] == "attempted" => {
                Ok(serde_json::json!({"outcome": "unchanged"}))
            }
            Ok(_) => Err(PluginHostInvocationFailure {
                started: true,
                error: anyhow::anyhow!("portable_usage_collector_failed"),
            }),
            Err(failure) => Err(failure),
        }
    }

    /// Whether a writable session home of `account`'s `plugin` holds a
    /// credential its sealed replica does not (a refresh candidate), and
    /// whether some home could not be read at all.
    async fn pending_refresh(
        &self,
        account: ProviderAccountId,
        plugin: String,
    ) -> Result<(bool, bool)> {
        let observer = self.clone();
        tokio::task::spawn_blocking(move || {
            // The observer is best effort (an unreadable directory reads as
            // empty); this check is strict: the account's credential root and
            // the Provider's session homes must enumerate, and one must exist.
            let enumerable = fs::read_dir(observer.auth_watch_root()).is_ok()
                && observer
                    .runtime_projection_generations(&plugin)
                    .is_ok_and(|homes| !homes.is_empty());
            let observations = observer.auth_refresh_observations();
            (
                observations.candidates.iter().any(|candidate| {
                    candidate.account_id == account && candidate.provider_id == plugin
                }),
                !enumerable
                    || observations
                        .failed_provider_ids
                        .iter()
                        .any(|(failed, provider)| *failed == account && *provider == plugin),
            )
        })
        .await
        .map_err(anyhow::Error::from)
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
        admission.private_home = Some((request.auth_generation, false));
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

/// The exact code of a reset that never started its spend: a native reset
/// code (the expiry rechecked before spawn included), a portable admission
/// code, or `native_reset_preflight_failed` for any other pre-spawn failure.
fn pre_spend_code(detail: &str) -> &'static str {
    match crate::machine_protocol::native_usage_error_code(detail) {
        "native_usage_unavailable" => match portable_usage_error_code(detail) {
            "portable_usage_collector_failed" | "portable_usage_temporarily_unavailable" => {
                "native_reset_preflight_failed"
            }
            code => code,
        },
        code => code,
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
            expires_at_unix_ms: None,
            private_home: None,
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

    /// A renewal that expired, is not a vault account or lacks the account's
    /// current replica never starts the collector.
    #[tokio::test]
    async fn a_portable_renewal_that_cannot_be_admitted_never_starts() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let renewal = |plugin: &str, expires_at_ms: i64| -> PortableRenewalRequest {
            serde_json::from_value(serde_json::json!({
                "account_id": "11111111-1111-4111-8111-111111111111", "plugin_id": plugin,
                "plugin_version": "3.2.36", "generation_digest": "sha256:x",
                "auth_generation": 3, "expires_at_ms": expires_at_ms,
            }))
            .unwrap()
        };
        let later = super::super::unix_ms() + 60_000;
        for (request, code) in [
            (renewal("grok", 1), "portable_usage_busy"),
            (
                renewal("claude-code", later),
                "portable_usage_account_required",
            ),
            (renewal("grok", later), "portable_usage_replica_pending"),
        ] {
            let invocation =
                PortableRenewalInvocation::new(request, Arc::new(AtomicBool::new(true)));
            let failure = store
                .renew_portable_credential(invocation)
                .await
                .unwrap_err();
            assert!(!failure.started, "{code}");
            assert_eq!(failure.error.to_string(), code);
        }
    }

    /// A reset that is invalid, expired, not a vault account or without the
    /// account's current replica never starts the collector.
    #[tokio::test]
    async fn a_portable_reset_that_cannot_be_admitted_never_starts() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let reset = |key: &str, expires_at_ms: i64| -> PortableResetRequest {
            serde_json::from_value(serde_json::json!({
                "account_id": "11111111-1111-4111-8111-111111111111", "plugin_id": "codex",
                "plugin_version": "3.6.20", "generation_digest": "sha256:x", "auth_generation": 3,
                "reset": {"provider": "openai", "idempotency_key": key,
                    "expected_credit_id": "credit-1"},
                "expires_at_ms": expires_at_ms,
            }))
            .unwrap()
        };
        let later = super::super::unix_ms() + 60_000;
        let mut claude = reset("cowboy-reset-1", later);
        claude.plugin_id = "claude-code".into();
        for (request, code) in [
            (reset("not a key", later), "native_reset_request_invalid"),
            (reset("cowboy-reset-1", 1), "native_reset_expired"),
            (claude, "portable_usage_account_required"),
            (
                reset("cowboy-reset-1", later),
                "portable_usage_replica_pending",
            ),
        ] {
            let invocation = PortableResetInvocation::new(request, Arc::new(AtomicBool::new(true)));
            let failure = store.consume_portable_reset(invocation).await.unwrap_err();
            assert!(!failure.started, "{code}");
            assert_eq!(failure.error.to_string(), code);
        }
    }
}
