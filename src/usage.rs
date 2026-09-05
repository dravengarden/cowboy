//! Provider usage aggregation.
//!
//! Account-level limits are deliberately separate from ACP session usage. ACP
//! reports context/cost for one session; provider collectors add plan windows,
//! reset times, credits, and account activity when an official interface exists.

#![warn(clippy::pedantic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::core::SessionMeta;
use crate::machine_protocol::{PluginHostOperation, PluginInstallationState, PluginInventory};
use crate::plugin_host::PluginUsageSpec;
use crate::plugin_process::run_plugin_command;

pub const AUTO_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_mins(5);
const MANUAL_REFRESH_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);
const TRANSIENT_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_mins(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRefreshState {
    pub last_attempt_at_ms: i64,
    pub manual_refresh_after_ms: i64,
    pub next_auto_refresh_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderUsage {
    pub provider: &'static str,
    pub status: &'static str,
    pub source: &'static str,
    pub observed_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<ProviderRefreshState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedUsageSnapshot {
    refreshed_at_ms: i64,
    next_refresh_at_ms: i64,
    refresh_interval_ms: i64,
    providers: Vec<CachedProviderUsage>,
    #[serde(default)]
    reset_schedules: BTreeMap<String, ResetSchedule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedProviderUsage {
    provider: String,
    status: String,
    source: String,
    observed_at_ms: i64,
    #[serde(default)]
    account: Option<Value>,
    #[serde(default)]
    rate_limits: Option<Value>,
    #[serde(default)]
    activity: Option<Value>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    refresh: Option<ProviderRefreshState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageSnapshot {
    pub refreshed_at_ms: i64,
    pub next_refresh_at_ms: i64,
    pub refresh_interval_ms: i64,
    pub providers: Vec<ProviderUsage>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub reset_schedules: BTreeMap<String, ResetSchedule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetSchedule {
    pub fire_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetResult {
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_id: Option<String>,
}

#[derive(Debug)]
pub struct ResetError {
    pub call_may_have_reached_provider: bool,
    pub credit_id: Option<String>,
    source: anyhow::Error,
}

impl std::fmt::Display for ResetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for ResetError {}

#[derive(Clone)]
pub struct UsageService {
    bindings: UsageBindingSource,
    machine_control: Arc<crate::machine_control::MachineControl>,
    store: Option<crate::store::Store>,
    snapshot: Arc<Mutex<UsageSnapshot>>,
    refresh_lock: Arc<Mutex<()>>,
    reset_lock: Arc<Mutex<()>>,
    reset_schedules: Arc<Mutex<BTreeMap<String, ResetSchedule>>>,
    cache_path: Option<PathBuf>,
    warming: Arc<AtomicBool>,
}

enum PluginCommandRoute {
    Bootstrap,
    Machine {
        machine_id: String,
        plugin: PluginInventory,
    },
    Unavailable(String),
}

#[derive(Clone)]
enum UsageBindingSource {
    #[cfg(test)]
    Static(Arc<Vec<PluginUsageSpec>>),
    Catalog(Arc<crate::plugin_catalog::PluginCatalog>),
}

impl UsageBindingSource {
    fn current(&self) -> Vec<PluginUsageSpec> {
        match self {
            #[cfg(test)]
            Self::Static(bindings) => bindings.as_ref().clone(),
            Self::Catalog(catalog) => catalog
                .runtime()
                .map_or_else(Vec::new, |runtime| runtime.usage_bindings()),
        }
    }
}

impl UsageService {
    #[cfg(test)]
    pub fn new(store: Option<crate::store::Store>, cache_path: Option<PathBuf>) -> Self {
        Self::with_bindings(store, cache_path, Vec::new())
    }

    #[must_use]
    #[cfg(test)]
    pub fn with_bindings(
        store: Option<crate::store::Store>,
        cache_path: Option<PathBuf>,
        bindings: Vec<PluginUsageSpec>,
    ) -> Self {
        Self::with_binding_source(
            store,
            cache_path,
            UsageBindingSource::Static(Arc::new(bindings)),
            Arc::new(crate::machine_control::MachineControl::default()),
        )
    }

    #[must_use]
    pub(crate) fn with_plugin_catalog(
        store: Option<crate::store::Store>,
        cache_path: Option<PathBuf>,
        catalog: Arc<crate::plugin_catalog::PluginCatalog>,
        machine_control: Arc<crate::machine_control::MachineControl>,
    ) -> Self {
        Self::with_binding_source(
            store,
            cache_path,
            UsageBindingSource::Catalog(catalog),
            machine_control,
        )
    }

    fn with_binding_source(
        store: Option<crate::store::Store>,
        cache_path: Option<PathBuf>,
        bindings: UsageBindingSource,
        machine_control: Arc<crate::machine_control::MachineControl>,
    ) -> Self {
        let initial_bindings = bindings.current();
        let mut snapshot = cache_path
            .as_deref()
            .and_then(load_cached_snapshot)
            .unwrap_or_else(|| UsageSnapshot {
                refreshed_at_ms: 0,
                next_refresh_at_ms: 0,
                refresh_interval_ms: i64::try_from(AUTO_REFRESH_INTERVAL.as_millis())
                    .unwrap_or(i64::MAX),
                providers: initial_bindings.iter().map(placeholder_usage).collect(),
                reset_schedules: BTreeMap::new(),
            });
        align_snapshot_bindings(&mut snapshot, &initial_bindings);
        Self {
            bindings,
            machine_control,
            store,
            snapshot: Arc::new(Mutex::new(snapshot)),
            refresh_lock: Arc::new(Mutex::new(())),
            reset_lock: Arc::new(Mutex::new(())),
            reset_schedules: Arc::new(Mutex::new(BTreeMap::new())),
            cache_path,
            warming: Arc::new(AtomicBool::new(false)),
        }
    }

    #[must_use]
    pub fn plugin_bindings(&self) -> Vec<PluginUsageSpec> {
        self.bindings.current()
    }

    fn command_route(&self, account: &str, operation: PluginHostOperation) -> PluginCommandRoute {
        #[cfg(not(test))]
        let UsageBindingSource::Catalog(catalog) = &self.bindings;
        #[cfg(test)]
        let catalog = match &self.bindings {
            UsageBindingSource::Static(_) => return PluginCommandRoute::Bootstrap,
            UsageBindingSource::Catalog(catalog) => catalog,
        };
        let Some(runtime) = catalog.runtime() else {
            return PluginCommandRoute::Unavailable(
                "Plugin host is temporarily unavailable: runtime is not active".to_owned(),
            );
        };
        let released_hosts = runtime.exact_usage_hosts(account);
        if released_hosts.is_empty() {
            return PluginCommandRoute::Bootstrap;
        }
        let hosts = released_hosts
            .iter()
            .filter(|host| {
                host.usage.as_ref().is_some_and(|usage| match operation {
                    PluginHostOperation::CollectUsage => !usage.collector_argv.is_empty(),
                    PluginHostOperation::ResetUsage => !usage.reset_argv.is_empty(),
                    PluginHostOperation::DecorateActivity => {
                        usage.activity && !usage.collector_argv.is_empty()
                    }
                })
            })
            .collect::<Vec<_>>();
        if hosts.is_empty() {
            return PluginCommandRoute::Unavailable(
                "Released Plugin host does not support this usage operation".to_owned(),
            );
        }
        let installed = self.machine_control.connected_plugin_inventory();
        let mut candidates = hosts
            .into_iter()
            .flat_map(|host| {
                installed
                    .iter()
                    .filter(move |installed| {
                        installed.plugin.state == PluginInstallationState::Active
                            && host.id == installed.plugin.plugin_id
                            && host.plugin_version.as_deref()
                                == Some(installed.plugin.plugin_version.as_str())
                            && host.artifact_digest.as_deref()
                                == Some(installed.plugin.generation_digest.as_str())
                    })
                    .map(move |installed| {
                        (
                            host.plugin_version.as_deref().unwrap_or_default(),
                            installed.machine_id.clone(),
                            installed.plugin.clone(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            semver::Version::parse(right.0)
                .ok()
                .cmp(&semver::Version::parse(left.0).ok())
                .then(left.1.cmp(&right.1))
                .then(left.2.generation_digest.cmp(&right.2.generation_digest))
        });
        candidates.into_iter().next().map_or_else(
            || {
                PluginCommandRoute::Unavailable(
                    "Plugin host is temporarily unavailable: no connected Machine has a matching exact generation"
                        .to_owned(),
                )
            },
            |(_, machine_id, plugin)| PluginCommandRoute::Machine { machine_id, plugin },
        )
    }

    /// Let the signed collector add Provider-owned, data-only projections to
    /// an activity response. A missing or incompatible transform degrades to
    /// the generic Cowboy aggregate instead of breaking the activity surface.
    pub async fn decorate_activity(&self, provider: &str, activity: Value) -> Value {
        let bindings = self.bindings.current();
        let Some(binding) = bindings.iter().find(|binding| {
            binding.account == provider && binding.activity && !binding.collector_argv.is_empty()
        }) else {
            return activity;
        };
        let request = json!({
            "operation": "decorate_activity",
            "provider": provider,
            "activity": activity,
        });
        let result = async {
            let response =
                match self.command_route(&binding.account, PluginHostOperation::DecorateActivity) {
                    PluginCommandRoute::Machine { machine_id, plugin } => self
                        .machine_control
                        .plugin_host_request(
                            &machine_id,
                            &plugin,
                            PluginHostOperation::DecorateActivity,
                            request.clone(),
                        )
                        .await
                        .map_err(|error| anyhow::anyhow!(error.detail))?,
                    PluginCommandRoute::Unavailable(detail) => bail!(detail),
                    PluginCommandRoute::Bootstrap => {
                        let (program, args) = binding
                            .collector_argv
                            .split_first()
                            .context("plugin activity transform argv is empty")?;
                        let encoded = serde_json::to_vec(&request)?;
                        let output = run_plugin_command(program, args, &encoded)
                            .await
                            .map_err(|failure| failure.error)?;
                        anyhow::ensure!(
                            output.status.success(),
                            "plugin activity transform exited {}: {}",
                            output.status,
                            String::from_utf8_lossy(&output.stderr).trim()
                        );
                        serde_json::from_slice(&output.stdout)
                            .context("parsing plugin activity transform JSON")?
                    }
                };
            let transformed: PluginActivityTransform = serde_json::from_value(response)
                .context("parsing plugin activity transform response")?;
            Ok::<_, anyhow::Error>(transformed.activity)
        }
        .await;
        match result {
            Ok(transformed) => transformed,
            Err(error) => {
                tracing::warn!(%error, provider, "plugin activity transform is unavailable");
                request.get("activity").cloned().unwrap_or(Value::Null)
            }
        }
    }

    #[must_use]
    pub fn reset_provider_ids(&self) -> Vec<String> {
        self.bindings
            .current()
            .iter()
            .filter_map(|binding| binding.reset.clone())
            .collect()
    }

    #[must_use]
    #[cfg(test)]
    pub fn exposes_activity(&self, account: &str) -> bool {
        self.bindings
            .current()
            .iter()
            .any(|binding| binding.account == account && binding.activity)
    }

    #[must_use]
    pub fn reset_claims_before_attempt(&self, reset_id: &str) -> bool {
        self.bindings
            .current()
            .iter()
            .find(|binding| binding.reset.as_deref() == Some(reset_id))
            .is_some_and(PluginUsageSpec::claims_reset_before_attempt)
    }

    async fn collect_binding(&self, binding: &PluginUsageSpec) -> ProviderUsage {
        let usage = if !binding.collector_argv.is_empty() || binding.collector.is_command() {
            collect_command(
                binding,
                self.store.as_ref(),
                self.command_route(&binding.account, PluginHostOperation::CollectUsage),
                &self.machine_control,
            )
            .await
        } else if binding.collector.is_session() {
            placeholder_usage(binding)
        } else {
            crate::provider_info::error(
                intern_usage_str(binding.account.clone()),
                intern_usage_str(binding.product_label().to_owned()),
                format!(
                    "usage collector {} has no plugin command",
                    binding.collector.as_str()
                ),
            )
        };
        bind_collected_usage(usage, binding)
    }

    pub async fn snapshot(&self) -> UsageSnapshot {
        let bindings = self.bindings.current();
        let reset_schedules = self.reset_schedules.lock().await.clone();
        let mut snapshot = self.snapshot.lock().await.clone();
        align_snapshot_bindings(&mut snapshot, &bindings);
        apply_reset_schedules(&mut snapshot, &reset_schedules);
        self.maybe_warm(&snapshot);
        snapshot
    }

    /// Coalesces concurrent manual/automatic refreshes. All API callers share
    /// the same per-provider cooldown and persisted last-good value.
    pub async fn refresh(&self) -> UsageSnapshot {
        self.refresh_with_policy(false).await
    }

    /// Automatic refreshes favor a flat memory profile over minimum wall time.
    /// Plugin collectors are separate, short-lived but potentially heavy
    /// processes; running them one after another prevents their RSS peaks
    /// from stacking inside the controller service cgroup. Explicit user
    /// refreshes keep the concurrent path above.
    pub(crate) async fn refresh_background(&self) -> UsageSnapshot {
        self.refresh_with_policy(true).await
    }

    async fn refresh_with_policy(&self, low_peak: bool) -> UsageSnapshot {
        let _guard = self.refresh_lock.lock().await;
        let bindings = self.bindings.current();
        let mut current = self.snapshot.lock().await.clone();
        align_snapshot_bindings(&mut current, &bindings);
        let policy = if low_peak {
            RefreshPolicy::Background
        } else {
            RefreshPolicy::Manual
        };
        let attempted_at_ms = now_ms();
        let due: Vec<PluginUsageSpec> = bindings
            .iter()
            .filter(|binding| binding.refreshable())
            .filter(|binding| {
                provider_refresh_due(
                    find_provider(&current, &binding.account),
                    attempted_at_ms,
                    policy,
                )
            })
            .cloned()
            .collect();
        if due.is_empty() {
            let reset_schedules = self.reset_schedules.lock().await.clone();
            apply_reset_schedules(&mut current, &reset_schedules);
            return current;
        }
        // Subprocess collectors are comparatively heavy. Background refreshes
        // run them one after another so RSS peaks do not stack. Manual refresh
        // keeps the concurrent path.
        let attempts = if low_peak {
            let mut attempts = Vec::new();
            for binding in &due {
                attempts.push(self.collect_binding(binding).await);
            }
            attempts
        } else {
            let futs = due.iter().map(|binding| self.collect_binding(binding));
            futures::future::join_all(futs).await
        };
        let completed_at_ms = now_ms();
        for attempt in attempts {
            reconcile_provider_attempt(&mut current, attempt, completed_at_ms, &bindings);
        }
        update_snapshot_refresh_times(&mut current, completed_at_ms, &bindings);
        let reset_schedules = self.reset_schedules.lock().await.clone();
        apply_reset_schedules(&mut current, &reset_schedules);
        *self.snapshot.lock().await = current.clone();
        self.persist_snapshot(&current);
        current
    }

    fn maybe_warm(&self, snapshot: &UsageSnapshot) {
        if self.cache_path.is_none() || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let current_time_ms = now_ms();
        if !self.bindings.current().iter().any(|binding| {
            binding.refreshable()
                && provider_refresh_due(
                    find_provider(snapshot, &binding.account),
                    current_time_ms,
                    RefreshPolicy::Background,
                )
        }) {
            return;
        }
        if self
            .warming
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            this.refresh_background().await;
            this.warming.store(false, Ordering::Relaxed);
        });
    }

    fn persist_snapshot(&self, snapshot: &UsageSnapshot) {
        let Some(path) = self.cache_path.as_ref() else {
            return;
        };
        save_cached_snapshot(path, snapshot);
    }

    /// Refresh one provider adapter without making unrelated cards wait on a
    /// slower account API. Session-only providers are recomputed by the HTTP
    /// response overlay and therefore keep their adapter placeholder here.
    pub async fn refresh_provider(&self, provider: &str) -> Result<UsageSnapshot> {
        self.refresh_provider_with_policy(provider, RefreshPolicy::Manual)
            .await
    }

    async fn refresh_provider_with_policy(
        &self,
        provider: &str,
        policy: RefreshPolicy,
    ) -> Result<UsageSnapshot> {
        let bindings = self.bindings.current();
        let Some(binding) = bindings
            .iter()
            .find(|binding| binding.account == provider)
            .cloned()
        else {
            bail!("unknown usage provider");
        };
        if !binding.refreshable() {
            return Ok(self.snapshot().await);
        }
        let _guard = self.refresh_lock.lock().await;
        let mut snapshot = self.snapshot.lock().await.clone();
        align_snapshot_bindings(&mut snapshot, &bindings);
        let attempted_at_ms = now_ms();
        if !provider_refresh_due(find_provider(&snapshot, provider), attempted_at_ms, policy) {
            let reset_schedules = self.reset_schedules.lock().await.clone();
            apply_reset_schedules(&mut snapshot, &reset_schedules);
            return Ok(snapshot);
        }
        let replacement = self.collect_binding(&binding).await;
        let completed_at_ms = now_ms();
        reconcile_provider_attempt(&mut snapshot, replacement, completed_at_ms, &bindings);
        update_snapshot_refresh_times(&mut snapshot, completed_at_ms, &bindings);
        let reset_schedules = self.reset_schedules.lock().await.clone();
        apply_reset_schedules(&mut snapshot, &reset_schedules);
        *self.snapshot.lock().await = snapshot.clone();
        self.persist_snapshot(&snapshot);
        Ok(snapshot)
    }

    pub async fn set_reset_schedule(&self, provider: &str, schedule: Option<ResetSchedule>) {
        let mut schedules = self.reset_schedules.lock().await;
        if let Some(schedule) = schedule {
            schedules.insert(provider.to_owned(), schedule);
        } else {
            schedules.remove(provider);
        }
    }

    /// Consume exactly the earliest-expiring available credit. Callers cannot
    /// supply an id, so stale clients and concurrent sessions cannot select a
    /// later credit. The lock serializes the final refresh/select/consume path.
    pub async fn consume_nearest_reset(
        &self,
        provider: &str,
        idempotency_key: &str,
        expected_credit_id: Option<&str>,
    ) -> std::result::Result<ResetResult, ResetError> {
        let _guard = self.reset_lock.lock().await;
        let bindings = self.bindings.current();
        let Some(binding) = bindings
            .iter()
            .find(|candidate| candidate.reset.as_deref() == Some(provider))
        else {
            return Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::anyhow!("provider does not support usage resets"),
            });
        };
        if !binding.reset_argv.is_empty() {
            return consume_plugin_reset(
                binding,
                idempotency_key,
                expected_credit_id,
                self.command_route(&binding.account, PluginHostOperation::ResetUsage),
                &self.machine_control,
            )
            .await;
        }
        Err(ResetError {
            call_may_have_reached_provider: false,
            credit_id: None,
            source: anyhow::anyhow!("provider reset has no plugin command"),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshPolicy {
    Manual,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsageFailureKind {
    Transient,
    Authentication,
    Configuration,
    Other,
}

fn duration_ms(duration: std::time::Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn find_provider<'a>(snapshot: &'a UsageSnapshot, provider: &str) -> Option<&'a ProviderUsage> {
    snapshot
        .providers
        .iter()
        .find(|candidate| candidate.provider == provider)
}

fn provider_refresh_due(
    usage: Option<&ProviderUsage>,
    current_time_ms: i64,
    policy: RefreshPolicy,
) -> bool {
    let Some(refresh) = usage.and_then(|usage| usage.refresh.as_ref()) else {
        return true;
    };
    match policy {
        RefreshPolicy::Manual => current_time_ms >= refresh.manual_refresh_after_ms,
        RefreshPolicy::Background => current_time_ms >= refresh.next_auto_refresh_at_ms,
    }
}

fn provider_has_cached_value(usage: &ProviderUsage) -> bool {
    matches!(usage.status, "available" | "exhausted")
        && (usage.account.is_some() || usage.rate_limits.is_some() || usage.activity.is_some())
}

fn contains_http_status(detail: &str, status: u16) -> bool {
    let status = status.to_string();
    [
        format!(" {status} "),
        format!("http {status}"),
        format!("status {status}"),
        format!("status={status}"),
        format!("status: {status}"),
        format!("\"status\":{status}"),
        format!("\"status\": {status}"),
        format!("error ({status} "),
    ]
    .iter()
    .any(|needle| detail.contains(needle))
}

fn classify_usage_failure(detail: &str) -> UsageFailureKind {
    let detail = detail.to_ascii_lowercase();
    if [
        "authentication required",
        "authorization required",
        "unauthorized",
        "forbidden",
        "not signed in",
        "login required",
        "sign in required",
        "refresh token was already used",
        "http 401",
        "http 403",
        "status 401",
        "status 403",
        "401 unauthorized",
        "403 forbidden",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
    {
        return UsageFailureKind::Authentication;
    }
    if [
        "not configured",
        "unsupported",
        "does not expose",
        "method not found",
        "invalid request",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
    {
        return UsageFailureKind::Configuration;
    }
    if [
        "timed out",
        "timeout",
        "temporarily unavailable",
        "service unavailable",
        "connection refused",
        "connection reset",
        "connection closed",
        "network error",
        "dns error",
        "failed to connect",
        "http 408",
        "http 409",
        "http 429",
        "status 408",
        "status 409",
        "status 429",
        "408 request timeout",
        "409 conflict",
        "429 too many requests",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
        || (500..=599).any(|status| contains_http_status(&detail, status))
    {
        return UsageFailureKind::Transient;
    }
    UsageFailureKind::Other
}

fn public_usage_error(
    bindings: &[PluginUsageSpec],
    provider: &str,
    failure: UsageFailureKind,
    retained_cached_value: bool,
) -> String {
    let binding = bindings
        .iter()
        .find(|candidate| candidate.account == provider);
    let product = binding.map_or("Provider", PluginUsageSpec::product_label);
    match failure {
        UsageFailureKind::Transient if retained_cached_value => {
            format!("{product} usage is temporarily unavailable. Showing the last update.")
        }
        UsageFailureKind::Transient => {
            format!("{product} usage is temporarily unavailable. Cowboy will retry automatically.")
        }
        UsageFailureKind::Authentication => binding
            .and_then(|candidate| candidate.error_auth.clone())
            .unwrap_or_else(|| format!("{product} usage authorization expired. Sign in again.")),
        UsageFailureKind::Configuration => binding
            .and_then(|candidate| candidate.error_config.clone())
            .unwrap_or_else(|| {
                format!("{product} usage is not configured on this Cowboy Service.")
            }),
        UsageFailureKind::Other => binding
            .and_then(|candidate| candidate.error_fetch.clone())
            .unwrap_or_else(|| format!("{product} usage could not be refreshed.")),
    }
}

fn refresh_state(
    attempted_at_ms: i64,
    next_auto_interval: std::time::Duration,
    stale: bool,
) -> ProviderRefreshState {
    ProviderRefreshState {
        last_attempt_at_ms: attempted_at_ms,
        manual_refresh_after_ms: attempted_at_ms
            .saturating_add(duration_ms(MANUAL_REFRESH_COOLDOWN)),
        next_auto_refresh_at_ms: attempted_at_ms.saturating_add(duration_ms(next_auto_interval)),
        stale,
    }
}

fn reconcile_provider_attempt(
    snapshot: &mut UsageSnapshot,
    mut attempt: ProviderUsage,
    attempted_at_ms: i64,
    bindings: &[PluginUsageSpec],
) {
    let previous = find_provider(snapshot, attempt.provider).cloned();
    let failed = !matches!(attempt.status, "available" | "exhausted");
    let replacement = if failed {
        let detail = attempt
            .error
            .take()
            .unwrap_or_else(|| "provider returned no usage data".to_owned());
        let failure = classify_usage_failure(&detail);
        let retain_cached_value = failure == UsageFailureKind::Transient
            && previous.as_ref().is_some_and(provider_has_cached_value);
        tracing::warn!(
            provider = attempt.provider,
            failure = ?failure,
            retained_cached_value = retain_cached_value,
            error = %detail,
            "provider usage refresh failed"
        );
        let public_error =
            public_usage_error(bindings, attempt.provider, failure, retain_cached_value);
        if retain_cached_value {
            let mut cached = previous.expect("cached value was checked above");
            cached.error = Some(public_error);
            cached.refresh = Some(refresh_state(
                attempted_at_ms,
                TRANSIENT_RETRY_INTERVAL,
                true,
            ));
            cached
        } else {
            attempt.error = Some(public_error);
            attempt.refresh = Some(refresh_state(attempted_at_ms, AUTO_REFRESH_INTERVAL, false));
            attempt
        }
    } else {
        attempt.error = None;
        attempt.refresh = Some(refresh_state(attempted_at_ms, AUTO_REFRESH_INTERVAL, false));
        attempt
    };

    if let Some(slot) = snapshot
        .providers
        .iter_mut()
        .find(|candidate| candidate.provider == replacement.provider)
    {
        *slot = replacement;
    } else {
        snapshot.providers.push(replacement);
    }
}

fn update_snapshot_refresh_times(
    snapshot: &mut UsageSnapshot,
    refreshed_at_ms: i64,
    bindings: &[PluginUsageSpec],
) {
    snapshot.refreshed_at_ms = refreshed_at_ms;
    snapshot.next_refresh_at_ms = snapshot
        .providers
        .iter()
        .filter(|usage| {
            bindings
                .iter()
                .any(|binding| binding.account == usage.provider && binding.refreshable())
        })
        .filter_map(|usage| usage.refresh.as_ref())
        .map(|refresh| refresh.next_auto_refresh_at_ms)
        .min()
        .unwrap_or_else(|| refreshed_at_ms.saturating_add(duration_ms(AUTO_REFRESH_INTERVAL)));
    snapshot.refresh_interval_ms = duration_ms(AUTO_REFRESH_INTERVAL);
}

fn align_snapshot_bindings(snapshot: &mut UsageSnapshot, bindings: &[PluginUsageSpec]) {
    let mut aligned = Vec::with_capacity(bindings.len());
    for binding in bindings {
        if aligned
            .iter()
            .any(|usage: &ProviderUsage| usage.provider == binding.account)
        {
            continue;
        }
        aligned.push(
            snapshot
                .providers
                .iter()
                .find(|usage| usage.provider == binding.account)
                .cloned()
                .unwrap_or_else(|| placeholder_usage(binding)),
        );
    }
    snapshot.providers = aligned;
}

fn apply_reset_schedules(
    snapshot: &mut UsageSnapshot,
    schedules: &BTreeMap<String, ResetSchedule>,
) {
    snapshot.reset_schedules.clone_from(schedules);
}

/// Overlay the newest live ACP usage per provider onto the account snapshot.
/// This is evaluated at response time so token/cost updates stay push-fresh and
/// never wait for the slower account collector.
pub fn with_session_usage(
    snapshot: UsageSnapshot,
    sessions: &[SessionMeta],
    catalog: &crate::provider_catalog::ProviderCatalog,
    bindings: &[PluginUsageSpec],
) -> UsageSnapshot {
    crate::provider_info::overlay_session_usage(snapshot, sessions, catalog, bindings)
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

pub(crate) fn intern_usage_str(value: String) -> &'static str {
    match value.as_str() {
        "available" => "available",
        "unavailable" => "unavailable",
        "exhausted" => "exhausted",
        "session-only" => "session-only",
        "error" => "error",
        _ => intern_dynamic(value),
    }
}

fn intern_dynamic(value: String) -> &'static str {
    static INTERN: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeSet<&'static str>>> =
        std::sync::OnceLock::new();
    let mut interned = INTERN
        .get_or_init(|| std::sync::Mutex::new(std::collections::BTreeSet::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(existing) = interned.get(value.as_str()).copied() {
        return existing;
    }
    let leaked: &'static str = Box::leak(value.into_boxed_str());
    interned.insert(leaked);
    leaked
}

fn load_cached_snapshot(path: &Path) -> Option<UsageSnapshot> {
    let bytes = std::fs::read(path).ok()?;
    let cached: CachedUsageSnapshot = serde_json::from_slice(&bytes).ok()?;
    let reset_schedules = cached.reset_schedules;
    Some(UsageSnapshot {
        refreshed_at_ms: cached.refreshed_at_ms,
        next_refresh_at_ms: cached.next_refresh_at_ms,
        refresh_interval_ms: cached.refresh_interval_ms,
        providers: cached
            .providers
            .into_iter()
            .map(|provider| ProviderUsage {
                provider: intern_usage_str(provider.provider),
                status: intern_usage_str(provider.status),
                source: intern_usage_str(provider.source),
                observed_at_ms: provider.observed_at_ms,
                account: provider.account,
                rate_limits: provider.rate_limits,
                activity: provider.activity,
                error: provider.error,
                refresh: provider.refresh,
            })
            .collect(),
        reset_schedules,
    })
}

fn save_cached_snapshot(path: &Path, snapshot: &UsageSnapshot) {
    let Ok(bytes) = serde_json::to_vec(snapshot) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
}

fn placeholder_usage(binding: &PluginUsageSpec) -> ProviderUsage {
    crate::provider_info::unavailable(
        intern_usage_str(binding.account.clone()),
        "Provider adapter",
        "Not refreshed yet",
    )
}

fn bind_collected_usage(mut usage: ProviderUsage, binding: &PluginUsageSpec) -> ProviderUsage {
    usage.provider = intern_usage_str(binding.account.clone());
    usage.source = intern_usage_str(binding.product_label().to_owned());
    usage
}

#[cfg(test)]
fn unavailable_providers() -> Vec<ProviderUsage> {
    crate::plugin_runtime_args::usage_accounts()
        .into_iter()
        .map(|provider| {
            crate::provider_info::unavailable(provider, "Provider adapter", "Not refreshed yet")
        })
        .collect()
}

async fn collect_command(
    binding: &PluginUsageSpec,
    store: Option<&crate::store::Store>,
    route: PluginCommandRoute,
    machine_control: &crate::machine_control::MachineControl,
) -> ProviderUsage {
    let product = intern_usage_str(binding.product_label().to_owned());
    let account = intern_usage_str(binding.account.clone());
    let attached_activity = if binding.activity {
        Some(collector_activity(store, &binding.account).await)
    } else {
        None
    };
    let request = json!({
        "operation": "collect",
        "provider": binding.account,
        "product": binding.product_label(),
        "activity": attached_activity,
    });
    let result = async {
        let response = match route {
            PluginCommandRoute::Machine { machine_id, plugin } => machine_control
                .plugin_host_request(
                    &machine_id,
                    &plugin,
                    PluginHostOperation::CollectUsage,
                    request,
                )
                .await
                .map_err(|error| anyhow::anyhow!(error.detail))?,
            PluginCommandRoute::Unavailable(detail) => bail!(detail),
            PluginCommandRoute::Bootstrap => {
                let (program, args) = binding
                    .collector_argv
                    .split_first()
                    .context("plugin collector argv is empty")?;
                let request = serde_json::to_vec(&request)?;
                let output = run_plugin_command(program, args, &request)
                    .await
                    .map_err(|failure| failure.error)?;
                anyhow::ensure!(
                    output.status.success(),
                    "plugin collector exited {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                serde_json::from_slice(&output.stdout)
                    .context("parsing plugin collector usage JSON")?
            }
        };
        let collected: CachedProviderUsage =
            serde_json::from_value(response).context("parsing plugin collector usage response")?;
        Ok(ProviderUsage {
            provider: intern_usage_str(collected.provider),
            status: intern_usage_str(collected.status),
            source: intern_usage_str(collected.source),
            observed_at_ms: collected.observed_at_ms,
            account: collected.account,
            rate_limits: collected.rate_limits,
            activity: collected.activity.or(attached_activity),
            error: collected.error,
            refresh: collected.refresh,
        })
    }
    .await;
    match result {
        Ok(value) => value,
        Err(error) => crate::provider_info::error(account, product, format!("{error:#}")),
    }
}

async fn collector_activity(store: Option<&crate::store::Store>, account: &str) -> Value {
    let Some(store) = store else {
        return json!({
            "source": "cowboy",
            "windowDays": 14,
            "retentionDays": 30,
            "availableAgents": [],
            "summary": null,
            "coverage": { "producers": [] },
            "unavailableReason": "Cowboy persistence is disabled",
        });
    };
    match store.provider_usage_summary(account, 14, 30).await {
        Ok(activity) => activity,
        Err(error) => {
            tracing::warn!(%error, provider = account, "plugin usage telemetry is unavailable");
            json!({
                "source": "cowboy",
                "windowDays": 14,
                "retentionDays": 30,
                "availableAgents": [],
                "summary": null,
                "coverage": { "producers": [] },
                "telemetryError": "Cowboy request telemetry is unavailable",
            })
        }
    }
}

async fn consume_plugin_reset(
    binding: &PluginUsageSpec,
    idempotency_key: &str,
    expected_credit_id: Option<&str>,
    route: PluginCommandRoute,
    machine_control: &crate::machine_control::MachineControl,
) -> std::result::Result<ResetResult, ResetError> {
    let request = json!({
        "operation": "consume_reset",
        "provider": binding.account,
        "idempotency_key": idempotency_key,
        "expected_credit_id": expected_credit_id,
    });
    let response = match route {
        PluginCommandRoute::Machine { machine_id, plugin } => machine_control
            .plugin_host_request(
                &machine_id,
                &plugin,
                PluginHostOperation::ResetUsage,
                request,
            )
            .await
            .map_err(|error| ResetError {
                call_may_have_reached_provider: error.started,
                credit_id: None,
                source: anyhow::anyhow!(error.detail),
            })?,
        PluginCommandRoute::Unavailable(detail) => {
            return Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::anyhow!(detail),
            });
        }
        PluginCommandRoute::Bootstrap => {
            let Some((program, args)) = binding.reset_argv.split_first() else {
                return Err(ResetError {
                    call_may_have_reached_provider: false,
                    credit_id: None,
                    source: anyhow::anyhow!("plugin reset argv is empty"),
                });
            };
            let request = serde_json::to_vec(&request).map_err(|source| ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::Error::new(source).context("serialize plugin usage reset request"),
            })?;
            let output = run_plugin_command(program, args, &request)
                .await
                .map_err(|failure| ResetError {
                    call_may_have_reached_provider: failure.started,
                    credit_id: None,
                    source: failure.error,
                })?;
            if !output.status.success() {
                return Err(ResetError {
                    call_may_have_reached_provider: true,
                    credit_id: None,
                    source: anyhow::anyhow!(
                        "plugin usage reset exited {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                });
            }
            serde_json::from_slice(&output.stdout).map_err(|source| ResetError {
                call_may_have_reached_provider: true,
                credit_id: None,
                source: anyhow::Error::new(source).context("parsing plugin usage reset result"),
            })?
        }
    };
    match serde_json::from_value::<PluginResetCommandResponse>(response) {
        Ok(PluginResetCommandResponse::Success(result)) => Ok(result),
        Ok(PluginResetCommandResponse::Failure { error }) => Err(ResetError {
            call_may_have_reached_provider: error.call_may_have_reached_provider,
            credit_id: error.credit_id,
            source: anyhow::anyhow!(error.message),
        }),
        Err(source) => Err(ResetError {
            call_may_have_reached_provider: true,
            credit_id: None,
            source: anyhow::Error::new(source).context("parsing plugin usage reset result"),
        }),
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PluginResetCommandResponse {
    Success(ResetResult),
    Failure { error: PluginResetCommandError },
}

#[derive(Debug, Deserialize)]
struct PluginResetCommandError {
    message: String,
    #[serde(default)]
    call_may_have_reached_provider: bool,
    #[serde(default)]
    credit_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PluginActivityTransform {
    activity: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_host::{UsageCollectorKind, UsageErrorKind};

    #[test]
    fn reset_claim_and_ids_follow_plugin_bindings() {
        let service = UsageService::with_bindings(
            None,
            None,
            vec![openai_usage_binding(), xai_like_binding("xai")],
        );
        assert_eq!(service.reset_provider_ids(), ["codex", "xai"]);
        assert!(!service.reset_claims_before_attempt("codex"));
        assert!(service.reset_claims_before_attempt("xai"));
        assert!(!service.reset_claims_before_attempt("missing"));
        assert!(!service.exposes_activity("xai"));
        assert!(!service.exposes_activity("deepseek"));
    }

    #[test]
    fn activity_endpoint_follows_plugin_activity_capability() {
        let service = UsageService::with_bindings(
            None,
            None,
            vec![
                openai_usage_binding(),
                PluginUsageSpec {
                    account: "custom-ds".to_owned(),
                    collector: UsageCollectorKind::Named("future-command".to_owned()),
                    reset: None,
                    product: Some("DeepSeek".to_owned()),
                    parser: crate::plugin_host::UsageLimitParserKind::new("generic-buckets"),
                    error: UsageErrorKind::new("raw"),
                    error_auth: None,
                    error_config: None,
                    error_fetch: None,
                    order: None,
                    top_bar_windows: Vec::new(),
                    widget: crate::plugin_host::UsageWidgetKind::new("deepseek-balance"),
                    widget_shape: crate::plugin_host::UsageWidgetShape::Balance,
                    widget_window: None,
                    reset_claim: crate::plugin_host::UsageResetClaim::AfterSuccess,
                    session_overlay: crate::plugin_host::UsageSessionOverlay::new("none"),
                    session_rate_limits: None,
                    empty: None,
                    available_status: Some("API".to_owned()),
                    omit_empty_limits: true,
                    limit_id_prefix: None,
                    limit_labels: Vec::new(),
                    widget_balance_label: None,
                    widget_spend_label: None,
                    activity_agents: Vec::new(),
                    activity_models: Vec::new(),
                    cache_protection: None,
                    collector_sidecars: Vec::new(),
                    collector_argv: Vec::new(),
                    reset_argv: Vec::new(),
                    activity: true,
                },
            ],
        );
        assert!(service.exposes_activity("custom-ds"));
        assert!(!service.exposes_activity("deepseek"));
        assert!(!service.exposes_activity("openai"));
    }

    #[tokio::test]
    async fn collector_argv_parses_plugin_json_stdout() {
        let mut binding = openai_usage_binding();
        binding.account = "future".to_owned();
        binding.collector = UsageCollectorKind::Command;
        binding.collector_argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "read -r _request || true; printf '%s' '{\"provider\":\"future\",\"status\":\"available\",\"source\":\"cmd\",\"observed_at_ms\":1}'"
                .to_owned(),
        ];
        let control = crate::machine_control::MachineControl::default();
        let usage = collect_command(&binding, None, PluginCommandRoute::Bootstrap, &control).await;
        assert_eq!(usage.provider, "future");
        assert_eq!(usage.status, "available");
        assert_eq!(usage.source, "cmd");
        assert_eq!(usage.observed_at_ms, 1);
    }

    #[tokio::test]
    async fn signed_collector_can_decorate_filtered_activity() {
        let mut binding = openai_usage_binding();
        binding.account = "future".to_owned();
        binding.activity = true;
        binding.collector_argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "read -r _request || true; printf '%s' '{\"activity\":{\"requests\":3,\"price\":\"signed\"}}'"
                .to_owned(),
        ];
        let service = UsageService::with_bindings(None, None, vec![binding]);
        let activity = service
            .decorate_activity("future", json!({ "requests": 3 }))
            .await;
        assert_eq!(activity["requests"], 3);
        assert_eq!(activity["price"], "signed");
    }

    #[tokio::test]
    async fn reset_argv_parses_plugin_success() {
        let mut binding = openai_usage_binding();
        binding.reset_argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "read -r _request || true; printf '%s' '{\"outcome\":\"consumed\",\"credit_id\":\"credit-1\"}'"
                .to_owned(),
        ];
        let control = crate::machine_control::MachineControl::default();
        let result = consume_plugin_reset(
            &binding,
            "attempt-1",
            Some("credit-1"),
            PluginCommandRoute::Bootstrap,
            &control,
        )
        .await
        .expect("plugin reset result");
        assert_eq!(result.outcome, "consumed");
        assert_eq!(result.credit_id.as_deref(), Some("credit-1"));
    }

    #[tokio::test]
    async fn reset_argv_preserves_plugin_failure_ambiguity() {
        let mut binding = openai_usage_binding();
        binding.reset_argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "read -r _request || true; printf '%s' '{\"error\":{\"message\":\"provider outcome is unknown\",\"call_may_have_reached_provider\":true,\"credit_id\":\"credit-2\"}}'"
                .to_owned(),
        ];
        let control = crate::machine_control::MachineControl::default();
        let error = consume_plugin_reset(
            &binding,
            "attempt-2",
            Some("credit-2"),
            PluginCommandRoute::Bootstrap,
            &control,
        )
        .await
        .expect_err("plugin reset failure");
        assert!(error.call_may_have_reached_provider);
        assert_eq!(error.credit_id.as_deref(), Some("credit-2"));
        assert_eq!(error.to_string(), "provider outcome is unknown");
    }

    fn openai_usage_binding() -> PluginUsageSpec {
        PluginUsageSpec {
            account: "openai".to_owned(),
            collector: UsageCollectorKind::Command,
            reset: Some("codex".to_owned()),
            product: Some("OpenAI".to_owned()),
            parser: crate::plugin_host::UsageLimitParserKind::new("generic-buckets"),
            error: crate::plugin_host::UsageErrorKind::new("openai-auth"),
            error_auth: Some(
                "OpenAI usage authorization expired. Sign in to Codex again.".to_owned(),
            ),
            error_config: None,
            error_fetch: None,
            order: Some(0),
            top_bar_windows: vec![300, 10_080],
            widget: crate::plugin_host::UsageWidgetKind::new("openai-weekly"),
            widget_shape: crate::plugin_host::UsageWidgetShape::Percent,
            widget_window: Some(10_080),
            reset_claim: crate::plugin_host::UsageResetClaim::AfterSuccess,
            session_overlay: crate::plugin_host::UsageSessionOverlay::new("none"),
            session_rate_limits: None,
            empty: None,
            available_status: None,
            omit_empty_limits: false,
            limit_id_prefix: None,
            limit_labels: Vec::new(),
            widget_balance_label: None,
            widget_spend_label: None,
            activity_agents: Vec::new(),
            activity_models: Vec::new(),
            cache_protection: None,
            collector_sidecars: Vec::new(),
            collector_argv: Vec::new(),
            reset_argv: Vec::new(),
            activity: false,
        }
    }

    fn successful_usage(provider: &'static str, observed_at_ms: i64) -> ProviderUsage {
        ProviderUsage {
            provider,
            status: "available",
            source: "test collector",
            observed_at_ms,
            account: Some(json!({ "plan": "test" })),
            rate_limits: Some(json!({ "remaining": 80 })),
            activity: None,
            error: None,
            refresh: None,
        }
    }

    fn failed_usage(provider: &'static str, detail: &str) -> ProviderUsage {
        ProviderUsage {
            provider,
            status: "unavailable",
            source: "test collector",
            observed_at_ms: 200,
            account: None,
            rate_limits: None,
            activity: None,
            error: Some(detail.to_owned()),
            refresh: None,
        }
    }

    #[test]
    fn unavailable_snapshot_is_explicit() {
        let providers = unavailable_providers();
        let accounts = crate::plugin_runtime_args::usage_accounts();
        assert_eq!(providers.len(), accounts.len());
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider.provider)
                .collect::<Vec<_>>(),
            accounts
        );
        assert!(
            providers
                .iter()
                .all(|p| p.status == "unavailable" && p.error.is_some())
        );
    }

    #[tokio::test]
    async fn empty_bindings_do_not_invent_account_cards() {
        let service = UsageService::with_bindings(None, None, Vec::new());
        let snapshot = service.snapshot().await;
        assert!(snapshot.providers.is_empty());
    }

    #[test]
    fn usage_failures_separate_transient_and_authentication_errors() {
        assert_eq!(
            classify_usage_failure(
                "account/rateLimits/read: 503 Service Unavailable; Unable to check usage"
            ),
            UsageFailureKind::Transient
        );
        assert_eq!(
            classify_usage_failure("account/read: 401 Unauthorized"),
            UsageFailureKind::Authentication
        );
        assert_eq!(
            classify_usage_failure("managed Grok Build CLI is not configured"),
            UsageFailureKind::Configuration
        );
        assert_eq!(
            classify_usage_failure(r#"request failed with {"status":502}"#),
            UsageFailureKind::Transient
        );
        assert_eq!(
            classify_usage_failure(
                "DeepSeek balance adapter rejected request: HTTP status server error (502 Bad Gateway)"
            ),
            UsageFailureKind::Transient
        );
        assert_eq!(
            classify_usage_failure("request id 503123 has an unknown error"),
            UsageFailureKind::Other
        );
    }

    #[test]
    fn transient_failure_retains_last_good_provider_value() {
        let mut snapshot = UsageSnapshot {
            refreshed_at_ms: 100,
            next_refresh_at_ms: 400,
            refresh_interval_ms: 300,
            providers: vec![successful_usage("openai", 100)],
            reset_schedules: BTreeMap::new(),
        };

        reconcile_provider_attempt(
            &mut snapshot,
            failed_usage("openai", "503 Service Unavailable"),
            200,
            &[openai_usage_binding()],
        );

        let usage = find_provider(&snapshot, "openai").expect("OpenAI usage");
        assert_eq!(usage.status, "available");
        assert_eq!(usage.observed_at_ms, 100);
        assert_eq!(usage.rate_limits, Some(json!({ "remaining": 80 })));
        assert_eq!(
            usage.error.as_deref(),
            Some("OpenAI usage is temporarily unavailable. Showing the last update.")
        );
        let refresh = usage.refresh.as_ref().expect("refresh metadata");
        assert!(refresh.stale);
        assert_eq!(
            refresh.manual_refresh_after_ms,
            200 + duration_ms(MANUAL_REFRESH_COOLDOWN)
        );
        assert_eq!(
            refresh.next_auto_refresh_at_ms,
            200 + duration_ms(TRANSIENT_RETRY_INTERVAL)
        );
    }

    #[test]
    fn authentication_failure_never_reuses_cached_limits() {
        let mut snapshot = UsageSnapshot {
            refreshed_at_ms: 100,
            next_refresh_at_ms: 400,
            refresh_interval_ms: 300,
            providers: vec![successful_usage("openai", 100)],
            reset_schedules: BTreeMap::new(),
        };

        reconcile_provider_attempt(
            &mut snapshot,
            failed_usage("openai", "account/read: 401 Unauthorized"),
            200,
            &[openai_usage_binding()],
        );

        let usage = find_provider(&snapshot, "openai").expect("OpenAI usage");
        assert_eq!(usage.status, "unavailable");
        assert!(usage.rate_limits.is_none());
        assert_eq!(
            usage.error.as_deref(),
            Some("OpenAI usage authorization expired. Sign in to Codex again.")
        );
        assert!(!usage.refresh.as_ref().expect("refresh metadata").stale);
    }

    #[test]
    fn intern_usage_str_does_not_hardcode_account_ids() {
        assert!(std::ptr::eq(
            intern_usage_str("available".to_owned()),
            intern_usage_str("available".to_owned())
        ));
        assert!(std::ptr::eq(
            intern_usage_str("openai".to_owned()),
            intern_usage_str("openai".to_owned())
        ));
        assert!(std::ptr::eq(
            intern_usage_str("custom-ds".to_owned()),
            intern_usage_str("custom-ds".to_owned())
        ));
    }

    #[test]
    fn collected_usage_is_rekeyed_to_the_plugin_account() {
        let usage =
            bind_collected_usage(successful_usage("xai", 1), &xai_like_binding("custom-xai"));
        assert_eq!(usage.provider, "custom-xai");
        let usage = bind_collected_usage(
            successful_usage("deepseek", 1),
            &PluginUsageSpec {
                account: "custom-ds".to_owned(),
                collector: UsageCollectorKind::Named("future-command".to_owned()),
                reset: None,
                product: Some("DeepSeek".to_owned()),
                parser: crate::plugin_host::UsageLimitParserKind::new("generic-buckets"),
                error: UsageErrorKind::new("raw"),
                error_auth: None,
                error_config: None,
                error_fetch: None,
                order: None,
                top_bar_windows: Vec::new(),
                widget: crate::plugin_host::UsageWidgetKind::new("deepseek-balance"),
                widget_shape: crate::plugin_host::UsageWidgetShape::Balance,
                widget_window: None,
                reset_claim: crate::plugin_host::UsageResetClaim::AfterSuccess,
                session_overlay: crate::plugin_host::UsageSessionOverlay::new("none"),
                session_rate_limits: None,
                empty: None,
                available_status: Some("API".to_owned()),
                omit_empty_limits: true,
                limit_id_prefix: None,
                limit_labels: Vec::new(),
                widget_balance_label: None,
                widget_spend_label: None,
                activity_agents: Vec::new(),
                activity_models: Vec::new(),
                cache_protection: None,
                collector_sidecars: Vec::new(),
                collector_argv: Vec::new(),
                reset_argv: Vec::new(),
                activity: false,
            },
        );
        assert_eq!(usage.provider, "custom-ds");
    }

    fn xai_like_binding(account: &str) -> PluginUsageSpec {
        PluginUsageSpec {
            account: account.to_owned(),
            collector: UsageCollectorKind::Command,
            reset: Some("xai".to_owned()),
            product: Some("Grok Build".to_owned()),
            parser: crate::plugin_host::UsageLimitParserKind::new("xai-credits"),
            error: UsageErrorKind::new("xai-billing"),
            error_auth: Some(
                "Sign in to Grok Build in Machines, then refresh xAI usage.".to_owned(),
            ),
            error_config: Some("Grok Build usage is not configured on this Machine.".to_owned()),
            error_fetch: Some("Grok Build could not fetch xAI usage.".to_owned()),
            order: None,
            top_bar_windows: Vec::new(),
            widget: crate::plugin_host::UsageWidgetKind::new("xai-included"),
            widget_shape: crate::plugin_host::UsageWidgetShape::Percent,
            widget_window: None,
            reset_claim: crate::plugin_host::UsageResetClaim::BeforeAttempt,
            session_overlay: crate::plugin_host::UsageSessionOverlay::new("none"),
            session_rate_limits: None,
            empty: None,
            available_status: None,
            omit_empty_limits: false,
            limit_id_prefix: None,
            limit_labels: Vec::new(),
            widget_balance_label: None,
            widget_spend_label: None,
            activity_agents: Vec::new(),
            activity_models: Vec::new(),
            cache_protection: None,
            collector_sidecars: Vec::new(),
            collector_argv: Vec::new(),
            reset_argv: Vec::new(),
            activity: false,
        }
    }

    #[test]
    fn public_usage_errors_follow_plugin_error_kind() {
        let custom = xai_like_binding("custom-xai");
        assert_eq!(
            public_usage_error(
                std::slice::from_ref(&custom),
                "custom-xai",
                UsageFailureKind::Authentication,
                false,
            ),
            "Sign in to Grok Build in Machines, then refresh xAI usage."
        );
        assert_eq!(
            public_usage_error(
                &[custom],
                "custom-xai",
                UsageFailureKind::Configuration,
                false,
            ),
            "Grok Build usage is not configured on this Machine."
        );
        assert_eq!(
            public_usage_error(
                &[openai_usage_binding()],
                "openai",
                UsageFailureKind::Authentication,
                false,
            ),
            "OpenAI usage authorization expired. Sign in to Codex again."
        );
        let generic = PluginUsageSpec {
            account: "future".to_owned(),
            collector: UsageCollectorKind::Session,
            reset: None,
            product: Some("Future".to_owned()),
            parser: crate::plugin_host::UsageLimitParserKind::new("generic-buckets"),
            error: UsageErrorKind::new("raw"),
            error_auth: None,
            error_config: None,
            error_fetch: None,
            order: None,
            top_bar_windows: Vec::new(),
            widget: crate::plugin_host::UsageWidgetKind::new("none"),
            widget_shape: crate::plugin_host::UsageWidgetShape::None,
            widget_window: None,
            reset_claim: crate::plugin_host::UsageResetClaim::AfterSuccess,
            session_overlay: crate::plugin_host::UsageSessionOverlay::new("none"),
            session_rate_limits: None,
            empty: None,
            available_status: None,
            omit_empty_limits: false,
            limit_id_prefix: None,
            limit_labels: Vec::new(),
            widget_balance_label: None,
            widget_spend_label: None,
            activity_agents: Vec::new(),
            activity_models: Vec::new(),
            cache_protection: None,
            collector_sidecars: Vec::new(),
            collector_argv: Vec::new(),
            reset_argv: Vec::new(),
            activity: false,
        };
        assert_eq!(
            public_usage_error(
                &[generic],
                "future",
                UsageFailureKind::Authentication,
                false,
            ),
            "Future usage authorization expired. Sign in again."
        );
    }

    #[test]
    fn provider_refresh_metadata_enforces_shared_manual_and_background_limits() {
        let mut usage = successful_usage("openai", 100);
        usage.refresh = Some(refresh_state(1_000, AUTO_REFRESH_INTERVAL, false));

        assert!(!provider_refresh_due(
            Some(&usage),
            1_000 + duration_ms(MANUAL_REFRESH_COOLDOWN) - 1,
            RefreshPolicy::Manual,
        ));
        assert!(provider_refresh_due(
            Some(&usage),
            1_000 + duration_ms(MANUAL_REFRESH_COOLDOWN),
            RefreshPolicy::Manual,
        ));
        assert!(!provider_refresh_due(
            Some(&usage),
            1_000 + duration_ms(AUTO_REFRESH_INTERVAL) - 1,
            RefreshPolicy::Background,
        ));
    }

    #[test]
    fn usage_snapshot_cache_round_trips_without_a_collector() {
        let path = std::env::temp_dir().join(format!(
            "cowboy-usage-cache-{}-{}.json",
            std::process::id(),
            now_ms()
        ));
        let snapshot = UsageSnapshot {
            refreshed_at_ms: 42,
            next_refresh_at_ms: 99,
            refresh_interval_ms: 1_000,
            providers: unavailable_providers(),
            reset_schedules: BTreeMap::new(),
        };
        let mut snapshot = snapshot;
        snapshot.providers[0].refresh = Some(refresh_state(42, AUTO_REFRESH_INTERVAL, false));
        save_cached_snapshot(&path, &snapshot);
        let loaded = load_cached_snapshot(&path).expect("cached snapshot");
        assert_eq!(loaded.refreshed_at_ms, 42);
        assert_eq!(loaded.providers.len(), snapshot.providers.len());
        assert_eq!(loaded.providers[0].refresh, snapshot.providers[0].refresh);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_usage_cache_without_provider_refresh_metadata_still_loads() {
        let path = std::env::temp_dir().join(format!(
            "cowboy-legacy-usage-cache-{}-{}.json",
            std::process::id(),
            now_ms()
        ));
        std::fs::write(
            &path,
            br#"{"refreshed_at_ms":42,"next_refresh_at_ms":99,"refresh_interval_ms":1000,"providers":[{"provider":"openai","status":"available","source":"test","observed_at_ms":41}]}"#,
        )
        .expect("write legacy cache");
        let loaded = load_cached_snapshot(&path).expect("legacy cached snapshot");
        assert!(loaded.providers[0].refresh.is_none());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn reset_schedules_are_isolated_by_provider() {
        let service = UsageService::new(None, None);
        service
            .set_reset_schedule("codex", Some(ResetSchedule { fire_at_ms: 100 }))
            .await;
        service
            .set_reset_schedule("xai", Some(ResetSchedule { fire_at_ms: 200 }))
            .await;
        let snapshot = service.snapshot().await;
        assert_eq!(
            snapshot
                .reset_schedules
                .get("codex")
                .map(|value| value.fire_at_ms),
            Some(100)
        );
        assert_eq!(
            snapshot
                .reset_schedules
                .get("xai")
                .map(|value| value.fire_at_ms),
            Some(200)
        );

        service.set_reset_schedule("xai", None).await;
        let snapshot = service.snapshot().await;
        assert_eq!(
            snapshot
                .reset_schedules
                .get("codex")
                .map(|value| value.fire_at_ms),
            Some(100)
        );
        assert!(!snapshot.reset_schedules.contains_key("xai"));
    }

    #[test]
    fn cached_reset_schedules_remain_plugin_keyed() {
        let mut reset_schedules = BTreeMap::new();
        reset_schedules.insert("future".to_owned(), ResetSchedule { fire_at_ms: 100 });
        let cached = CachedUsageSnapshot {
            refreshed_at_ms: 1,
            next_refresh_at_ms: 2,
            refresh_interval_ms: 3,
            providers: Vec::new(),
            reset_schedules,
        };
        assert_eq!(
            cached
                .reset_schedules
                .get("future")
                .map(|value| value.fire_at_ms),
            Some(100)
        );
    }
}
