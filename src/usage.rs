//! Provider usage aggregation.
//!
//! Account-level limits are deliberately separate from ACP session usage. ACP
//! reports context/cost for one session; provider collectors add plan windows,
//! reset times, credits, and account activity when an official interface exists.

#![warn(clippy::pedantic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::core::SessionMeta;
use crate::plugin_host::{PluginUsageSpec, UsageCollectorKind};

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
    #[serde(default)]
    codex_reset_schedule: Option<ResetSchedule>,
    #[serde(default)]
    xai_reset_schedule: Option<ResetSchedule>,
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

#[derive(Debug, Clone, Serialize)]
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
    bindings: Vec<PluginUsageSpec>,
    codex_command: String,
    grok_spec: Option<crate::provider::LaunchSpec>,
    store: Option<crate::store::Store>,
    snapshot: Arc<Mutex<UsageSnapshot>>,
    refresh_lock: Arc<Mutex<()>>,
    reset_lock: Arc<Mutex<()>>,
    reset_schedules: Arc<Mutex<BTreeMap<String, ResetSchedule>>>,
    cache_path: Option<PathBuf>,
    warming: Arc<AtomicBool>,
}

impl UsageService {
    #[cfg(test)]
    pub fn new(
        codex_command: String,
        store: Option<crate::store::Store>,
        cache_path: Option<PathBuf>,
    ) -> Self {
        Self::with_bindings(codex_command, store, cache_path, Vec::new())
    }

    #[must_use]
    pub fn with_bindings(
        codex_command: String,
        store: Option<crate::store::Store>,
        cache_path: Option<PathBuf>,
        bindings: Vec<PluginUsageSpec>,
    ) -> Self {
        // Never let an account-card refresh cold-install a provider through
        // npx. Production Machine configuration supplies the managed command;
        // local development simply reports Grok billing as unavailable.
        let grok_spec = std::env::var("COWBOY_ACP_GROK_CMD")
            .ok()
            .filter(|command| !command.trim().is_empty())
            .and_then(|_| crate::provider::lookup("grok"));
        let snapshot = cache_path
            .as_deref()
            .and_then(load_cached_snapshot)
            .unwrap_or_else(|| UsageSnapshot {
                refreshed_at_ms: 0,
                next_refresh_at_ms: 0,
                refresh_interval_ms: i64::try_from(AUTO_REFRESH_INTERVAL.as_millis())
                    .unwrap_or(i64::MAX),
                providers: bindings.iter().map(placeholder_usage).collect(),
                reset_schedules: BTreeMap::new(),
            });
        Self {
            bindings,
            codex_command,
            grok_spec,
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
    pub fn plugin_bindings(&self) -> &[PluginUsageSpec] {
        &self.bindings
    }

    #[must_use]
    pub fn reset_provider_ids(&self) -> Vec<String> {
        self.bindings
            .iter()
            .filter_map(|binding| binding.reset.clone())
            .collect()
    }

    #[must_use]
    pub fn exposes_activity(&self, account: &str) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.account == account && binding.activity)
    }

    #[must_use]
    pub fn reset_claims_before_attempt(&self, reset_id: &str) -> bool {
        self.bindings
            .iter()
            .find(|binding| binding.reset.as_deref() == Some(reset_id))
            .is_some_and(PluginUsageSpec::claims_reset_before_attempt)
    }

    async fn collect_binding(&self, binding: &PluginUsageSpec) -> ProviderUsage {
        let usage = if !binding.collector_argv.is_empty() {
            collect_command(binding).await
        } else {
            match binding.collector {
                UsageCollectorKind::OpenaiAppserver => {
                    collect_openai_usage(&self.codex_command, binding).await
                }
                UsageCollectorKind::DeepseekStore => {
                    collect_deepseek_usage(self.store.as_ref(), binding).await
                }
                UsageCollectorKind::XaiBilling => {
                    collect_configured_xai_usage(self.grok_spec.as_ref(), binding).await
                }
                UsageCollectorKind::Command => collect_command(binding).await,
                UsageCollectorKind::Session | UsageCollectorKind::Unknown => {
                    placeholder_usage(binding)
                }
            }
        };
        bind_collected_usage(usage, binding)
    }

    pub async fn snapshot(&self) -> UsageSnapshot {
        let reset_schedules = self.reset_schedules.lock().await.clone();
        let mut snapshot = self.snapshot.lock().await.clone();
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
    /// Codex and Grok collectors are separate, short-lived but comparatively
    /// heavy processes; running them one after another prevents their RSS peaks
    /// from stacking inside the controller service cgroup. Explicit user
    /// refreshes keep the concurrent path above.
    pub(crate) async fn refresh_background(&self) -> UsageSnapshot {
        self.refresh_with_policy(true).await
    }

    async fn refresh_with_policy(&self, low_peak: bool) -> UsageSnapshot {
        let _guard = self.refresh_lock.lock().await;
        let mut current = self.snapshot.lock().await.clone();
        let policy = if low_peak {
            RefreshPolicy::Background
        } else {
            RefreshPolicy::Manual
        };
        let attempted_at_ms = now_ms();
        let due: Vec<PluginUsageSpec> = self
            .bindings
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
            reconcile_provider_attempt(&mut current, attempt, completed_at_ms, &self.bindings);
        }
        update_snapshot_refresh_times(&mut current, completed_at_ms, &self.bindings);
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
        if !self.bindings.iter().any(|binding| {
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
        let Some(binding) = self
            .bindings
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
        let attempted_at_ms = now_ms();
        if !provider_refresh_due(find_provider(&snapshot, provider), attempted_at_ms, policy) {
            let reset_schedules = self.reset_schedules.lock().await.clone();
            apply_reset_schedules(&mut snapshot, &reset_schedules);
            return Ok(snapshot);
        }
        let replacement = self.collect_binding(&binding).await;
        let completed_at_ms = now_ms();
        reconcile_provider_attempt(&mut snapshot, replacement, completed_at_ms, &self.bindings);
        update_snapshot_refresh_times(&mut snapshot, completed_at_ms, &self.bindings);
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
        let Some(binding) = self
            .bindings
            .iter()
            .find(|candidate| candidate.reset.as_deref() == Some(provider))
        else {
            return Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::anyhow!("provider does not support usage resets"),
            });
        };
        let account = binding.account.clone();
        match binding.collector {
            UsageCollectorKind::OpenaiAppserver => {
                self.consume_nearest_codex_reset(&account, idempotency_key, expected_credit_id)
                    .await
            }
            UsageCollectorKind::XaiBilling => {
                self.consume_nearest_xai_reset(&account, expected_credit_id)
                    .await
            }
            UsageCollectorKind::DeepseekStore
            | UsageCollectorKind::Session
            | UsageCollectorKind::Command
            | UsageCollectorKind::Unknown => Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::anyhow!("provider does not support usage resets"),
            }),
        }
    }

    async fn consume_nearest_codex_reset(
        &self,
        account: &str,
        idempotency_key: &str,
        expected_credit_id: Option<&str>,
    ) -> std::result::Result<ResetResult, ResetError> {
        let usage = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            collect_codex(&self.codex_command),
        )
        .await
        .context("refresh before Codex reset timed out")
        .and_then(|result| result)
        .map_err(|source| ResetError {
            call_may_have_reached_provider: false,
            credit_id: None,
            source,
        })?;
        let credit_id = nearest_available_credit_id(usage.rate_limits.as_ref())
            .context("no available Codex reset credit")
            .map_err(|source| ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source,
            })?;
        if expected_credit_id.is_some_and(|expected| expected != credit_id) {
            return Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: Some(credit_id),
                source: anyhow::anyhow!(
                    "nearest Codex reset credit changed; refresh and confirm again"
                ),
            });
        }
        let mut server = JsonRpcProcess::start(&self.codex_command)
            .await
            .map_err(|source| ResetError {
                call_may_have_reached_provider: false,
                credit_id: Some(credit_id.clone()),
                source,
            })?;
        let response = server
            .request(
                "account/rateLimitResetCredit/consume",
                json!({ "creditId": credit_id, "idempotencyKey": idempotency_key }),
            )
            .await
            .map_err(|source| ResetError {
                // Once the consume frame is written, a missing/error response is
                // ambiguous. Never retry automatically: the provider may have
                // committed the credit before the transport failed.
                call_may_have_reached_provider: true,
                credit_id: Some(credit_id.clone()),
                source,
            })?;
        let outcome = response
            .get("outcome")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let _ = self
            .refresh_provider_with_policy(account, RefreshPolicy::Force)
            .await;
        Ok(ResetResult {
            outcome,
            credit_id: Some(credit_id),
        })
    }

    async fn consume_nearest_xai_reset(
        &self,
        account: &str,
        expected_credit_id: Option<&str>,
    ) -> std::result::Result<ResetResult, ResetError> {
        let binding = self
            .bindings
            .iter()
            .find(|binding| binding.account == account);
        let Some(spec) = self.grok_spec.as_ref() else {
            return Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::anyhow!(
                    "{}",
                    binding
                        .and_then(|binding| binding.error_config.as_deref())
                        .unwrap_or("usage is not configured")
                ),
            });
        };
        let account_id = intern_usage_str(account.to_owned());
        let product = intern_usage_str(
            binding
                .map(PluginUsageSpec::product_label)
                .unwrap_or("Provider")
                .to_owned(),
        );
        // The billing ACP request refreshes Grok's OIDC credential before the
        // account bridge reads reset availability from the same official file.
        let usage = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            crate::provider_info::collect_xai(
                spec,
                account_id,
                product,
                binding.and_then(|binding| binding.error_auth.as_deref()),
                binding.and_then(|binding| binding.error_fetch.as_deref()),
            ),
        )
        .await
        .context("refresh before xAI reset timed out")
        .and_then(|result| result)
        .map_err(|source| ResetError {
            call_may_have_reached_provider: false,
            credit_id: None,
            source,
        })?;
        let credit_id = nearest_available_credit_id(usage.rate_limits.as_ref())
            .context("no available xAI reset")
            .map_err(|source| ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source,
            })?;
        if expected_credit_id.is_some_and(|expected| expected != credit_id) {
            return Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: Some(credit_id),
                source: anyhow::anyhow!("nearest xAI reset changed; refresh and confirm again"),
            });
        }
        let remaining = crate::provider_info::redeem_xai_reset(&credit_id)
            .await
            .map_err(|source| ResetError {
                // RedeemReset has no provider idempotency key. Once the HTTP
                // request is sent, never retry automatically after an error.
                call_may_have_reached_provider: true,
                credit_id: Some(credit_id.clone()),
                source,
            })?;
        let outcome = format!(
            "consumed; {remaining} reset{} remaining",
            if remaining == 1 { "" } else { "s" }
        );
        let _ = self
            .refresh_provider_with_policy(account, RefreshPolicy::Force)
            .await;
        Ok(ResetResult {
            outcome,
            credit_id: Some(credit_id),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshPolicy {
    Manual,
    Background,
    Force,
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
        RefreshPolicy::Force => true,
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

fn apply_reset_schedules(
    snapshot: &mut UsageSnapshot,
    schedules: &BTreeMap<String, ResetSchedule>,
) {
    snapshot.reset_schedules.clone_from(schedules);
}

fn cached_reset_schedules(cached: &CachedUsageSnapshot) -> BTreeMap<String, ResetSchedule> {
    let mut schedules = cached.reset_schedules.clone();
    insert_legacy_reset_schedule(
        &mut schedules,
        "openai-appserver",
        cached.codex_reset_schedule.as_ref(),
    );
    insert_legacy_reset_schedule(
        &mut schedules,
        "xai-billing",
        cached.xai_reset_schedule.as_ref(),
    );
    schedules
}

fn insert_legacy_reset_schedule(
    schedules: &mut BTreeMap<String, ResetSchedule>,
    collector: &str,
    schedule: Option<&ResetSchedule>,
) {
    let Some(schedule) = schedule else {
        return;
    };
    let key = crate::plugin_runtime_args::usage_reset_id_for_collector(collector)
        .unwrap_or(collector)
        .to_owned();
    schedules.entry(key).or_insert_with(|| schedule.clone());
}

fn nearest_available_credit_id(rate_limits: Option<&Value>) -> Option<String> {
    let credits = rate_limits?
        .get("rateLimitResetCredits")?
        .get("credits")?
        .as_array()?;
    credits
        .iter()
        .filter(|credit| credit.get("status").and_then(Value::as_str) == Some("available"))
        .filter_map(|credit| {
            let id = credit.get("id")?.as_str()?.to_owned();
            let expires = credit
                .get("expiresAt")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX);
            let granted = credit
                .get("grantedAt")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX);
            Some(((expires, granted, id.clone()), id))
        })
        .min_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, id)| id)
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
    let reset_schedules = cached_reset_schedules(&cached);
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

async fn collect_command(binding: &PluginUsageSpec) -> ProviderUsage {
    let product = intern_usage_str(binding.product_label().to_owned());
    let account = intern_usage_str(binding.account.clone());
    let Some((program, args)) = binding.collector_argv.split_first() else {
        return crate::provider_info::unavailable(
            account,
            product,
            "plugin collector argv is empty",
        );
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    match tokio::time::timeout(std::time::Duration::from_secs(12), async {
        let output = command.output().await?;
        anyhow::ensure!(
            output.status.success(),
            "plugin collector exited {}",
            output.status
        );
        let collected: CachedProviderUsage = serde_json::from_slice(&output.stdout)
            .context("parsing plugin collector usage JSON")?;
        Ok(ProviderUsage {
            provider: intern_usage_str(collected.provider),
            status: intern_usage_str(collected.status),
            source: intern_usage_str(collected.source),
            observed_at_ms: collected.observed_at_ms,
            account: collected.account,
            rate_limits: collected.rate_limits,
            activity: collected.activity,
            error: collected.error,
            refresh: collected.refresh,
        })
    })
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => crate::provider_info::error(account, product, format!("{error:#}")),
        Err(_) => crate::provider_info::error(account, product, "refresh timed out".to_owned()),
    }
}

fn collector_identity(binding: &PluginUsageSpec) -> (&'static str, &'static str) {
    (
        intern_usage_str(binding.account.clone()),
        intern_usage_str(binding.product_label().to_owned()),
    )
}

async fn collect_openai_usage(command: &str, binding: &PluginUsageSpec) -> ProviderUsage {
    let (account, product) = collector_identity(binding);
    match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::provider_info::collect_openai(command),
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => crate::provider_info::error(account, product, format!("{error:#}")),
        Err(_) => crate::provider_info::error(account, product, "refresh timed out".to_owned()),
    }
}

async fn collect_deepseek_usage(
    store: Option<&crate::store::Store>,
    binding: &PluginUsageSpec,
) -> ProviderUsage {
    let (account, product) = collector_identity(binding);
    match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::provider_info::collect_deepseek(store, &binding.account, binding.product_label()),
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => crate::provider_info::error(account, product, format!("{error:#}")),
        Err(_) => crate::provider_info::error(account, product, "refresh timed out".to_owned()),
    }
}

async fn collect_configured_xai_usage(
    spec: Option<&crate::provider::LaunchSpec>,
    binding: &PluginUsageSpec,
) -> ProviderUsage {
    let (account, product) = collector_identity(binding);
    let Some(spec) = spec else {
        return crate::provider_info::unavailable(
            account,
            product,
            binding
                .error_config
                .as_deref()
                .unwrap_or("usage is not configured"),
        );
    };
    collect_xai_usage(spec, account, product, binding).await
}

async fn collect_xai_usage(
    spec: &crate::provider::LaunchSpec,
    account: &'static str,
    product: &'static str,
    binding: &PluginUsageSpec,
) -> ProviderUsage {
    match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::provider_info::collect_xai(
            spec,
            account,
            product,
            binding.error_auth.as_deref(),
            binding.error_fetch.as_deref(),
        ),
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => crate::provider_info::error(account, product, format!("{error:#}")),
        Err(_) => crate::provider_info::error(account, product, "refresh timed out".to_owned()),
    }
}

pub(crate) async fn collect_codex(command: &str) -> Result<ProviderUsage> {
    let mut server = JsonRpcProcess::start(command).await?;
    let account = server
        .request("account/read", json!({ "refreshToken": false }))
        .await?;
    let rate_limits = server.request("account/rateLimits/read", json!({})).await?;
    if !has_supported_rate_limit_shape(&rate_limits) {
        tracing::warn!(
            provider = "codex",
            source = "codex-app-server",
            "usage collector received an unknown rate-limit schema; exposing an empty summary"
        );
    }
    // Usage activity is newer than rateLimits and may be unavailable for API-key
    // or Bedrock auth. Keep limits useful even when this optional call fails.
    let activity = server.request("account/usage/read", json!({})).await.ok();
    Ok(ProviderUsage {
        provider: "codex",
        status: "available",
        source: "codex-app-server",
        observed_at_ms: now_ms(),
        account: Some(account),
        rate_limits: Some(rate_limits),
        activity,
        error: None,
        refresh: None,
    })
}

fn has_supported_rate_limit_shape(value: &Value) -> bool {
    value.get("rateLimits").is_some_and(Value::is_object)
        || value
            .get("rateLimitsByLimitId")
            .is_some_and(Value::is_object)
}

struct JsonRpcProcess {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl JsonRpcProcess {
    async fn start(command: &str) -> Result<Self> {
        let mut child = Command::new(command)
            .args([
                "app-server",
                "--stdio",
                "-c",
                "features.memories=false",
                "-c",
                "analytics.enabled=false",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("start Codex usage collector: {command}"))?;
        let stdin = child.stdin.take().context("collector stdin")?;
        let stdout = child.stdout.take().context("collector stdout")?;
        let mut out = Self {
            child,
            stdin,
            lines: BufReader::new(stdout).lines(),
            next_id: 1,
        };
        out.request(
            "initialize",
            json!({
                "clientInfo": { "name": "cowboy-usage", "title": "Cowboy", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true }
            }),
        ).await?;
        out.notify("initialized", json!({})).await?;
        Ok(out)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.write(&json!({ "id": id, "method": method, "params": params }))
            .await?;
        loop {
            let line = self
                .lines
                .next_line()
                .await
                .context("read app-server")?
                .context("app-server closed")?;
            let message: Value = serde_json::from_str(&line)
                .with_context(|| format!("parse app-server message: {line}"))?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                bail!("{method}: {error}");
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.write(&json!({ "method": method, "params": params }))
            .await
    }

    async fn write(&mut self, value: &Value) -> Result<()> {
        self.stdin
            .write_all(serde_json::to_string(value)?.as_bytes())
            .await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await.context("flush app-server")
    }
}

impl Drop for JsonRpcProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_host::UsageErrorKind;

    #[test]
    fn reset_claim_and_ids_follow_plugin_bindings() {
        let service = UsageService::with_bindings(
            "codex".to_owned(),
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
    fn activity_endpoint_follows_deepseek_store_collector() {
        let service = UsageService::with_bindings(
            "codex".to_owned(),
            None,
            None,
            vec![
                openai_usage_binding(),
                PluginUsageSpec {
                    account: "custom-ds".to_owned(),
                    collector: UsageCollectorKind::DeepseekStore,
                    reset: None,
                    product: Some("DeepSeek".to_owned()),
                    parser: crate::plugin_host::UsageLimitParserKind::GenericBuckets,
                    error: UsageErrorKind::Raw,
                    error_auth: None,
                    error_config: None,
                    error_fetch: None,
                    order: None,
                    top_bar_windows: Vec::new(),
                    widget: crate::plugin_host::UsageWidgetKind::DeepseekBalance,
                    widget_shape: crate::plugin_host::UsageWidgetShape::Balance,
                    widget_window: None,
                    reset_claim: crate::plugin_host::UsageResetClaim::AfterSuccess,
                    session_overlay: crate::plugin_host::UsageSessionOverlay::None,
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
                    collector_argv: Vec::new(),
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
        binding.collector = UsageCollectorKind::OpenaiAppserver;
        binding.collector_argv = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "printf '%s' '{\"provider\":\"future\",\"status\":\"available\",\"source\":\"cmd\",\"observed_at_ms\":1}'"
                .to_owned(),
        ];
        let usage = collect_command(&binding).await;
        assert_eq!(usage.provider, "future");
        assert_eq!(usage.status, "available");
        assert_eq!(usage.source, "cmd");
        assert_eq!(usage.observed_at_ms, 1);
    }

    fn openai_usage_binding() -> PluginUsageSpec {
        PluginUsageSpec {
            account: "openai".to_owned(),
            collector: UsageCollectorKind::OpenaiAppserver,
            reset: Some("codex".to_owned()),
            product: Some("OpenAI".to_owned()),
            parser: crate::plugin_host::UsageLimitParserKind::GenericBuckets,
            error: crate::plugin_host::UsageErrorKind::OpenaiAuth,
            error_auth: Some(
                "OpenAI usage authorization expired. Sign in to Codex again.".to_owned(),
            ),
            error_config: None,
            error_fetch: None,
            order: Some(0),
            top_bar_windows: vec![300, 10_080],
            widget: crate::plugin_host::UsageWidgetKind::OpenaiWeekly,
            widget_shape: crate::plugin_host::UsageWidgetShape::Percent,
            widget_window: Some(10_080),
            reset_claim: crate::plugin_host::UsageResetClaim::AfterSuccess,
            session_overlay: crate::plugin_host::UsageSessionOverlay::None,
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
            collector_argv: Vec::new(),
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
        let service = UsageService::with_bindings("codex".to_owned(), None, None, Vec::new());
        let snapshot = service.snapshot().await;
        assert!(snapshot.providers.is_empty());
    }

    #[test]
    fn unknown_rate_limit_schema_degrades_without_panicking() {
        assert!(!has_supported_rate_limit_shape(&json!({ "future": [] })));
        assert!(has_supported_rate_limit_shape(&json!({ "rateLimits": {} })));
        assert!(has_supported_rate_limit_shape(
            &json!({ "rateLimitsByLimitId": {} })
        ));
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
                collector: UsageCollectorKind::DeepseekStore,
                reset: None,
                product: Some("DeepSeek".to_owned()),
                parser: crate::plugin_host::UsageLimitParserKind::GenericBuckets,
                error: UsageErrorKind::Raw,
                error_auth: None,
                error_config: None,
                error_fetch: None,
                order: None,
                top_bar_windows: Vec::new(),
                widget: crate::plugin_host::UsageWidgetKind::DeepseekBalance,
                widget_shape: crate::plugin_host::UsageWidgetShape::Balance,
                widget_window: None,
                reset_claim: crate::plugin_host::UsageResetClaim::AfterSuccess,
                session_overlay: crate::plugin_host::UsageSessionOverlay::None,
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
                collector_argv: Vec::new(),
                activity: false,
            },
        );
        assert_eq!(usage.provider, "custom-ds");
    }

    fn xai_like_binding(account: &str) -> PluginUsageSpec {
        PluginUsageSpec {
            account: account.to_owned(),
            collector: UsageCollectorKind::XaiBilling,
            reset: Some("xai".to_owned()),
            product: Some("Grok Build".to_owned()),
            parser: crate::plugin_host::UsageLimitParserKind::XaiCredits,
            error: UsageErrorKind::XaiBilling,
            error_auth: Some(
                "Sign in to Grok Build in Machines, then refresh xAI usage.".to_owned(),
            ),
            error_config: Some("Grok Build usage is not configured on this Machine.".to_owned()),
            error_fetch: Some("Grok Build could not fetch xAI usage.".to_owned()),
            order: None,
            top_bar_windows: Vec::new(),
            widget: crate::plugin_host::UsageWidgetKind::XaiIncluded,
            widget_shape: crate::plugin_host::UsageWidgetShape::Percent,
            widget_window: None,
            reset_claim: crate::plugin_host::UsageResetClaim::BeforeAttempt,
            session_overlay: crate::plugin_host::UsageSessionOverlay::None,
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
            collector_argv: Vec::new(),
            activity: false,
        }
    }

    #[test]
    fn public_usage_errors_follow_plugin_error_kind() {
        let custom = xai_like_binding("custom-xai");
        assert_eq!(
            public_usage_error(
                &[custom.clone()],
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
            parser: crate::plugin_host::UsageLimitParserKind::GenericBuckets,
            error: UsageErrorKind::Raw,
            error_auth: None,
            error_config: None,
            error_fetch: None,
            order: None,
            top_bar_windows: Vec::new(),
            widget: crate::plugin_host::UsageWidgetKind::None,
            widget_shape: crate::plugin_host::UsageWidgetShape::None,
            widget_window: None,
            reset_claim: crate::plugin_host::UsageResetClaim::AfterSuccess,
            session_overlay: crate::plugin_host::UsageSessionOverlay::None,
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
            collector_argv: Vec::new(),
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
        assert!(provider_refresh_due(
            Some(&usage),
            1_000,
            RefreshPolicy::Force,
        ));
    }

    #[test]
    fn reset_selector_only_chooses_nearest_available_credit() {
        let limits = json!({ "rateLimitResetCredits": { "credits": [
            { "id": "later", "status": "available", "expiresAt": 300 },
            { "id": "used", "status": "redeemed", "expiresAt": 50 },
            { "id": "nearest", "status": "available", "expiresAt": 100 }
        ]}});
        assert_eq!(
            nearest_available_credit_id(Some(&limits)).as_deref(),
            Some("nearest")
        );
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
        let service = UsageService::new("codex".to_owned(), None, None);
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
    fn legacy_named_reset_schedules_import_into_the_plugin_keyed_map() {
        let cached = CachedUsageSnapshot {
            refreshed_at_ms: 1,
            next_refresh_at_ms: 2,
            refresh_interval_ms: 3,
            providers: Vec::new(),
            reset_schedules: BTreeMap::new(),
            codex_reset_schedule: Some(ResetSchedule { fire_at_ms: 100 }),
            xai_reset_schedule: Some(ResetSchedule { fire_at_ms: 200 }),
        };
        let schedules = cached_reset_schedules(&cached);
        assert_eq!(
            schedules.get("codex").map(|value| value.fire_at_ms),
            Some(100)
        );
        assert_eq!(
            schedules.get("xai").map(|value| value.fire_at_ms),
            Some(200)
        );
    }
}
