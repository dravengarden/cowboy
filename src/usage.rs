//! Provider usage aggregation.
//!
//! Account-level limits are deliberately separate from ACP session usage. ACP
//! reports context/cost for one session; provider collectors add plan windows,
//! reset times, credits, and account activity when an official interface exists.

#![warn(clippy::pedantic)]

use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::core::SessionMeta;

pub const AUTO_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_mins(5);

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
}

impl UsageService {
    pub fn new(codex_command: String, store: Option<crate::store::Store>) -> Self {
        // Never let an account-card refresh cold-install a provider through
        // npx. Production Machine configuration supplies the managed command;
        // local development simply reports Grok billing as unavailable.
        let grok_spec = std::env::var("COWBOY_ACP_GROK_CMD")
            .ok()
            .filter(|command| !command.trim().is_empty())
            .and_then(|_| crate::provider::lookup("grok"));
        Self {
            codex_command,
            grok_spec,
            store,
            snapshot: Arc::new(Mutex::new(UsageSnapshot {
                refreshed_at_ms: 0,
                next_refresh_at_ms: 0,
                refresh_interval_ms: i64::try_from(AUTO_REFRESH_INTERVAL.as_millis())
                    .unwrap_or(i64::MAX),
                providers: unavailable_providers(),
                codex_reset_schedule: None,
                xai_reset_schedule: None,
            })),
            refresh_lock: Arc::new(Mutex::new(())),
            reset_lock: Arc::new(Mutex::new(())),
            reset_schedules: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub async fn snapshot(&self) -> UsageSnapshot {
        let reset_schedules = self.reset_schedules.lock().await.clone();
        let mut snapshot = self.snapshot.lock().await.clone();
        apply_reset_schedules(&mut snapshot, &reset_schedules);
        snapshot
    }

    /// Coalesces concurrent manual/automatic refreshes. A failed provider keeps
    /// an explicit error row; it never makes the whole endpoint fail.
    pub async fn refresh(&self) -> UsageSnapshot {
        let _guard = self.refresh_lock.lock().await;
        let mut current = self.snapshot.lock().await.clone();
        if current.refreshed_at_ms > 0 && now_ms().saturating_sub(current.refreshed_at_ms) < 3_000 {
            let reset_schedules = self.reset_schedules.lock().await.clone();
            apply_reset_schedules(&mut current, &reset_schedules);
            return current;
        }
        let grok_spec = self.grok_spec.clone();
        let xai = async move {
            let Some(spec) = grok_spec else {
                return crate::provider_info::unavailable(
                    "xai",
                    crate::provider_info::XAI_SOURCE,
                    "Managed Grok Build CLI is not configured",
                );
            };
            collect_xai_usage(&spec).await
        };
        let (openai, deepseek, xai) = tokio::join!(
            tokio::time::timeout(
                std::time::Duration::from_secs(12),
                crate::provider_info::collect_openai(&self.codex_command),
            ),
            tokio::time::timeout(
                std::time::Duration::from_secs(12),
                crate::provider_info::collect_deepseek(self.store.as_ref()),
            ),
            xai,
        );
        let openai = match openai {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                crate::provider_info::error("openai", "OpenAI Codex app-server", error.to_string())
            }
            Err(_) => crate::provider_info::error(
                "openai",
                "OpenAI Codex app-server",
                "refresh timed out".into(),
            ),
        };
        let deepseek = match deepseek {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => crate::provider_info::error(
                "deepseek",
                "DeepSeek provider adapter",
                error.to_string(),
            ),
            Err(_) => crate::provider_info::error(
                "deepseek",
                "DeepSeek provider adapter",
                "refresh timed out".into(),
            ),
        };
        let refreshed_at_ms = now_ms();
        let reset_schedules = self.reset_schedules.lock().await.clone();
        let mut next = UsageSnapshot {
            refreshed_at_ms,
            next_refresh_at_ms: refreshed_at_ms.saturating_add(
                i64::try_from(AUTO_REFRESH_INTERVAL.as_millis()).unwrap_or(i64::MAX),
            ),
            refresh_interval_ms: i64::try_from(AUTO_REFRESH_INTERVAL.as_millis())
                .unwrap_or(i64::MAX),
            providers: vec![
                deepseek,
                openai,
                crate::provider_info::unavailable(
                    "anthropic",
                    "Anthropic Agent SDK via ACP",
                    "Waiting for Anthropic rate-limit data",
                ),
                crate::provider_info::unavailable(
                    "gemini",
                    "Gemini ACP",
                    "Provider does not expose account limits over ACP",
                ),
                xai,
            ],
            codex_reset_schedule: None,
            xai_reset_schedule: None,
        };
        apply_reset_schedules(&mut next, &reset_schedules);
        *self.snapshot.lock().await = next.clone();
        next
    }

    /// Refresh one provider adapter without making unrelated cards wait on a
    /// slower account API. Session-only providers are recomputed by the HTTP
    /// response overlay and therefore keep their adapter placeholder here.
    pub async fn refresh_provider(&self, provider: &str) -> Result<UsageSnapshot> {
        let _guard = self.refresh_lock.lock().await;
        let replacement = match provider {
            "openai" => tokio::time::timeout(
                std::time::Duration::from_secs(12),
                crate::provider_info::collect_openai(&self.codex_command),
            )
            .await
            .context("OpenAI provider refresh timed out")??,
            "deepseek" => tokio::time::timeout(
                std::time::Duration::from_secs(12),
                crate::provider_info::collect_deepseek(self.store.as_ref()),
            )
            .await
            .context("DeepSeek provider refresh timed out")??,
            "xai" => {
                let Some(spec) = self.grok_spec.as_ref() else {
                    return Err(anyhow::anyhow!("managed Grok Build CLI is not configured"));
                };
                collect_xai_usage(spec).await
            }
            "anthropic" | "gemini" => {
                return Ok(self.snapshot().await);
            }
            _ => bail!("unknown usage provider"),
        };
        let mut snapshot = self.snapshot.lock().await;
        let Some(slot) = snapshot
            .providers
            .iter_mut()
            .find(|candidate| candidate.provider == provider)
        else {
            bail!("usage provider is not configured");
        };
        *slot = replacement;
        snapshot.refreshed_at_ms = now_ms();
        snapshot.next_refresh_at_ms = snapshot
            .refreshed_at_ms
            .saturating_add(i64::try_from(AUTO_REFRESH_INTERVAL.as_millis()).unwrap_or(i64::MAX));
        drop(snapshot);
        Ok(self.snapshot().await)
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
        let _ = self.refresh_force().await;
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
        let _ = self.refresh_force().await;
        Ok(ResetResult {
            outcome,
            credit_id: Some(credit_id),
        })
    }

    async fn refresh_force(&self) -> UsageSnapshot {
        self.snapshot.lock().await.refreshed_at_ms = 0;
        self.refresh().await
    }
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

fn unavailable_providers() -> Vec<ProviderUsage> {
    crate::provider_info::PROVIDERS
        .map(|provider| {
            crate::provider_info::unavailable(provider, "Provider adapter", "Not refreshed yet")
        })
        .to_vec()
}

async fn collect_xai_usage(spec: &crate::provider::LaunchSpec) -> ProviderUsage {
    match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::provider_info::collect_xai(spec),
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            crate::provider_info::error("xai", crate::provider_info::XAI_SOURCE, error.to_string())
        }
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

    #[tokio::test]
    async fn reset_schedules_are_isolated_by_provider() {
        let service = UsageService::new("codex".to_owned(), None);
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
