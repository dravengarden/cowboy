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

pub const AUTO_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_mins(5);
const MANUAL_REFRESH_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);
const TRANSIENT_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_mins(1);
const REFRESHABLE_PROVIDERS: [&str; 3] = ["deepseek", "openai", "xai"];

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_reset_schedule: Option<ResetSchedule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xai_reset_schedule: Option<ResetSchedule>,
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

pub const RESET_PROVIDERS: [&str; 2] = ["codex", "xai"];

#[derive(Clone)]
pub struct UsageService {
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
    pub fn new(
        codex_command: String,
        store: Option<crate::store::Store>,
        cache_path: Option<PathBuf>,
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
                providers: unavailable_providers(),
                codex_reset_schedule: None,
                xai_reset_schedule: None,
            });
        Self {
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
        let refresh_openai =
            provider_refresh_due(find_provider(&current, "openai"), attempted_at_ms, policy);
        let refresh_deepseek =
            provider_refresh_due(find_provider(&current, "deepseek"), attempted_at_ms, policy);
        let refresh_xai =
            provider_refresh_due(find_provider(&current, "xai"), attempted_at_ms, policy);
        if !refresh_openai && !refresh_deepseek && !refresh_xai {
            let reset_schedules = self.reset_schedules.lock().await.clone();
            apply_reset_schedules(&mut current, &reset_schedules);
            return current;
        }
        let (openai, deepseek, xai) = if low_peak {
            // The in-process database collector is cheap. Finish it first, then
            // ensure the two provider subprocess lifetimes never overlap.
            let deepseek = if refresh_deepseek {
                Some(collect_deepseek_usage(self.store.as_ref()).await)
            } else {
                None
            };
            let openai = if refresh_openai {
                Some(collect_openai_usage(&self.codex_command).await)
            } else {
                None
            };
            let xai = if refresh_xai {
                Some(collect_configured_xai_usage(self.grok_spec.as_ref()).await)
            } else {
                None
            };
            (openai, deepseek, xai)
        } else {
            tokio::join!(
                async {
                    if refresh_openai {
                        Some(collect_openai_usage(&self.codex_command).await)
                    } else {
                        None
                    }
                },
                async {
                    if refresh_deepseek {
                        Some(collect_deepseek_usage(self.store.as_ref()).await)
                    } else {
                        None
                    }
                },
                async {
                    if refresh_xai {
                        Some(collect_configured_xai_usage(self.grok_spec.as_ref()).await)
                    } else {
                        None
                    }
                },
            )
        };
        let completed_at_ms = now_ms();
        for attempt in [deepseek, openai, xai].into_iter().flatten() {
            reconcile_provider_attempt(&mut current, attempt, completed_at_ms);
        }
        update_snapshot_refresh_times(&mut current, completed_at_ms);
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
        if !REFRESHABLE_PROVIDERS.iter().any(|provider| {
            provider_refresh_due(
                find_provider(snapshot, provider),
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
        if matches!(provider, "anthropic" | "gemini") {
            return Ok(self.snapshot().await);
        }
        if !REFRESHABLE_PROVIDERS.contains(&provider) {
            bail!("unknown usage provider");
        }
        let _guard = self.refresh_lock.lock().await;
        let mut snapshot = self.snapshot.lock().await.clone();
        let attempted_at_ms = now_ms();
        if !provider_refresh_due(find_provider(&snapshot, provider), attempted_at_ms, policy) {
            let reset_schedules = self.reset_schedules.lock().await.clone();
            apply_reset_schedules(&mut snapshot, &reset_schedules);
            return Ok(snapshot);
        }
        let replacement = match provider {
            "openai" => collect_openai_usage(&self.codex_command).await,
            "deepseek" => collect_deepseek_usage(self.store.as_ref()).await,
            "xai" => collect_configured_xai_usage(self.grok_spec.as_ref()).await,
            _ => unreachable!("provider was validated above"),
        };
        let completed_at_ms = now_ms();
        reconcile_provider_attempt(&mut snapshot, replacement, completed_at_ms);
        update_snapshot_refresh_times(&mut snapshot, completed_at_ms);
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
        match provider {
            "codex" => {
                self.consume_nearest_codex_reset(idempotency_key, expected_credit_id)
                    .await
            }
            "xai" => self.consume_nearest_xai_reset(expected_credit_id).await,
            _ => Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::anyhow!("provider does not support usage resets"),
            }),
        }
    }

    async fn consume_nearest_codex_reset(
        &self,
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
            .refresh_provider_with_policy("openai", RefreshPolicy::Force)
            .await;
        Ok(ResetResult {
            outcome,
            credit_id: Some(credit_id),
        })
    }

    async fn consume_nearest_xai_reset(
        &self,
        expected_credit_id: Option<&str>,
    ) -> std::result::Result<ResetResult, ResetError> {
        let Some(spec) = self.grok_spec.as_ref() else {
            return Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::anyhow!("managed Grok Build CLI is not configured"),
            });
        };
        // The billing ACP request refreshes Grok's OIDC credential before the
        // account bridge reads reset availability from the same official file.
        let usage = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            crate::provider_info::collect_xai(spec),
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
            .refresh_provider_with_policy("xai", RefreshPolicy::Force)
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
    provider: &str,
    failure: UsageFailureKind,
    retained_cached_value: bool,
) -> String {
    let product = match provider {
        "openai" => "OpenAI",
        "deepseek" => "DeepSeek",
        "xai" => "xAI",
        _ => "Provider",
    };
    match failure {
        UsageFailureKind::Transient if retained_cached_value => {
            format!("{product} usage is temporarily unavailable. Showing the last update.")
        }
        UsageFailureKind::Transient => {
            format!("{product} usage is temporarily unavailable. Cowboy will retry automatically.")
        }
        UsageFailureKind::Authentication if provider == "openai" => {
            "OpenAI usage authorization expired. Sign in to Codex again.".to_owned()
        }
        UsageFailureKind::Authentication if provider == "xai" => {
            "Sign in to Grok Build in Machines, then refresh xAI usage.".to_owned()
        }
        UsageFailureKind::Authentication => {
            format!("{product} usage authorization expired. Sign in again.")
        }
        UsageFailureKind::Configuration if provider == "xai" => {
            "Grok Build usage is not configured on this Machine.".to_owned()
        }
        UsageFailureKind::Configuration => {
            format!("{product} usage is not configured on this Cowboy Service.")
        }
        UsageFailureKind::Other => format!("{product} usage could not be refreshed."),
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
        let public_error = public_usage_error(attempt.provider, failure, retain_cached_value);
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

fn update_snapshot_refresh_times(snapshot: &mut UsageSnapshot, refreshed_at_ms: i64) {
    snapshot.refreshed_at_ms = refreshed_at_ms;
    snapshot.next_refresh_at_ms = snapshot
        .providers
        .iter()
        .filter(|usage| REFRESHABLE_PROVIDERS.contains(&usage.provider))
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
    snapshot.codex_reset_schedule = schedules.get("codex").cloned();
    snapshot.xai_reset_schedule = schedules.get("xai").cloned();
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
) -> UsageSnapshot {
    crate::provider_info::overlay_session_usage(snapshot, sessions, catalog)
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn intern_usage_str(value: String) -> &'static str {
    match value.as_str() {
        "deepseek" => "deepseek",
        "openai" => "openai",
        "anthropic" => "anthropic",
        "gemini" => "gemini",
        "xai" => "xai",
        "available" => "available",
        "unavailable" => "unavailable",
        "error" => "error",
        _ => Box::leak(value.into_boxed_str()),
    }
}

fn load_cached_snapshot(path: &Path) -> Option<UsageSnapshot> {
    let bytes = std::fs::read(path).ok()?;
    let cached: CachedUsageSnapshot = serde_json::from_slice(&bytes).ok()?;
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
        codex_reset_schedule: cached.codex_reset_schedule,
        xai_reset_schedule: cached.xai_reset_schedule,
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

fn unavailable_providers() -> Vec<ProviderUsage> {
    crate::provider_info::PROVIDERS
        .map(|provider| {
            crate::provider_info::unavailable(provider, "Provider adapter", "Not refreshed yet")
        })
        .to_vec()
}

async fn collect_openai_usage(command: &str) -> ProviderUsage {
    match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::provider_info::collect_openai(command),
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            crate::provider_info::error("openai", "OpenAI Codex app-server", format!("{error:#}"))
        }
        Err(_) => crate::provider_info::error(
            "openai",
            "OpenAI Codex app-server",
            "refresh timed out".to_owned(),
        ),
    }
}

async fn collect_deepseek_usage(store: Option<&crate::store::Store>) -> ProviderUsage {
    match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::provider_info::collect_deepseek(store),
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => crate::provider_info::error(
            "deepseek",
            "DeepSeek provider adapter",
            format!("{error:#}"),
        ),
        Err(_) => crate::provider_info::error(
            "deepseek",
            "DeepSeek provider adapter",
            "refresh timed out".to_owned(),
        ),
    }
}

async fn collect_configured_xai_usage(spec: Option<&crate::provider::LaunchSpec>) -> ProviderUsage {
    let Some(spec) = spec else {
        return crate::provider_info::unavailable(
            "xai",
            crate::provider_info::XAI_SOURCE,
            "Managed Grok Build CLI is not configured",
        );
    };
    collect_xai_usage(spec).await
}

async fn collect_xai_usage(spec: &crate::provider::LaunchSpec) -> ProviderUsage {
    match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::provider_info::collect_xai(spec),
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => crate::provider_info::error(
            "xai",
            crate::provider_info::XAI_SOURCE,
            format!("{error:#}"),
        ),
        Err(_) => crate::provider_info::error(
            "xai",
            crate::provider_info::XAI_SOURCE,
            "refresh timed out".to_owned(),
        ),
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
        assert_eq!(providers.len(), 5);
        assert_eq!(providers[0].provider, "deepseek");
        assert_eq!(providers[1].provider, "openai");
        assert!(
            providers
                .iter()
                .all(|p| p.status == "unavailable" && p.error.is_some())
        );
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
            codex_reset_schedule: None,
            xai_reset_schedule: None,
        };

        reconcile_provider_attempt(
            &mut snapshot,
            failed_usage("openai", "503 Service Unavailable"),
            200,
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
            codex_reset_schedule: None,
            xai_reset_schedule: None,
        };

        reconcile_provider_attempt(
            &mut snapshot,
            failed_usage("openai", "account/read: 401 Unauthorized"),
            200,
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
            codex_reset_schedule: None,
            xai_reset_schedule: None,
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
            snapshot.codex_reset_schedule.map(|value| value.fire_at_ms),
            Some(100)
        );
        assert_eq!(
            snapshot.xai_reset_schedule.map(|value| value.fire_at_ms),
            Some(200)
        );

        service.set_reset_schedule("xai", None).await;
        let snapshot = service.snapshot().await;
        assert_eq!(
            snapshot.codex_reset_schedule.map(|value| value.fire_at_ms),
            Some(100)
        );
        assert!(snapshot.xai_reset_schedule.is_none());
    }
}
