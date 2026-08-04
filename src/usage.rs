//! Provider usage aggregation.
//!
//! Account-level limits are deliberately separate from ACP session usage. ACP
//! reports context/cost for one session; provider collectors add plan windows,
//! reset times, credits, and account activity when an official interface exists.

#![warn(clippy::pedantic)]

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
    pub codex_reset_schedule: Option<CodexResetSchedule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexResetSchedule {
    pub fire_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexResetResult {
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_id: Option<String>,
}

#[derive(Debug)]
pub struct CodexResetError {
    pub call_may_have_reached_provider: bool,
    pub credit_id: Option<String>,
    source: anyhow::Error,
}

impl std::fmt::Display for CodexResetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for CodexResetError {}

#[derive(Clone)]
pub struct UsageService {
    codex_command: String,
    snapshot: Arc<Mutex<UsageSnapshot>>,
    refresh_lock: Arc<Mutex<()>>,
    reset_lock: Arc<Mutex<()>>,
    reset_schedule: Arc<Mutex<Option<CodexResetSchedule>>>,
}

impl UsageService {
    pub fn new(codex_command: String) -> Self {
        Self {
            codex_command,
            snapshot: Arc::new(Mutex::new(UsageSnapshot {
                refreshed_at_ms: 0,
                next_refresh_at_ms: 0,
                refresh_interval_ms: i64::try_from(AUTO_REFRESH_INTERVAL.as_millis())
                    .unwrap_or(i64::MAX),
                providers: unavailable_providers(),
                codex_reset_schedule: None,
            })),
            refresh_lock: Arc::new(Mutex::new(())),
            reset_lock: Arc::new(Mutex::new(())),
            reset_schedule: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn snapshot(&self) -> UsageSnapshot {
        let mut snapshot = self.snapshot.lock().await.clone();
        let reset_schedule = self.reset_schedule.lock().await;
        snapshot.codex_reset_schedule.clone_from(&reset_schedule);
        snapshot
    }

    /// Coalesces concurrent manual/automatic refreshes. A failed provider keeps
    /// an explicit error row; it never makes the whole endpoint fail.
    pub async fn refresh(&self) -> UsageSnapshot {
        let _guard = self.refresh_lock.lock().await;
        let current = self.snapshot.lock().await.clone();
        if current.refreshed_at_ms > 0 && now_ms().saturating_sub(current.refreshed_at_ms) < 3_000 {
            return current;
        }
        let codex = match tokio::time::timeout(
            std::time::Duration::from_secs(12),
            collect_codex(&self.codex_command),
        )
        .await
        {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => provider_error("codex", "codex-app-server", error.to_string()),
            Err(_) => provider_error("codex", "codex-app-server", "refresh timed out".into()),
        };
        let refreshed_at_ms = now_ms();
        let next = UsageSnapshot {
            refreshed_at_ms,
            next_refresh_at_ms: refreshed_at_ms.saturating_add(
                i64::try_from(AUTO_REFRESH_INTERVAL.as_millis()).unwrap_or(i64::MAX),
            ),
            refresh_interval_ms: i64::try_from(AUTO_REFRESH_INTERVAL.as_millis())
                .unwrap_or(i64::MAX),
            providers: vec![
                codex,
                unavailable("claude-code", "ACP", "Waiting for ACP rate-limit data"),
                unavailable(
                    "claude-deepseek",
                    "ACP",
                    "Waiting for DeepSeek session usage",
                ),
                unavailable(
                    "gemini",
                    "ACP",
                    "Provider does not expose account limits over ACP",
                ),
            ],
            codex_reset_schedule: self.reset_schedule.lock().await.clone(),
        };
        *self.snapshot.lock().await = next.clone();
        next
    }

    pub async fn set_reset_schedule(&self, schedule: Option<CodexResetSchedule>) {
        *self.reset_schedule.lock().await = schedule;
    }

    /// Consume exactly the earliest-expiring available credit. Callers cannot
    /// supply an id, so stale clients and concurrent sessions cannot select a
    /// later credit. The lock serializes the final refresh/select/consume path.
    pub async fn consume_nearest_reset(
        &self,
        idempotency_key: &str,
        expected_credit_id: Option<&str>,
    ) -> std::result::Result<CodexResetResult, CodexResetError> {
        let _guard = self.reset_lock.lock().await;
        let usage = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            collect_codex(&self.codex_command),
        )
        .await
        .context("refresh before Codex reset timed out")
        .and_then(|result| result)
        .map_err(|source| CodexResetError {
            call_may_have_reached_provider: false,
            credit_id: None,
            source,
        })?;
        let credit_id = nearest_available_credit_id(usage.rate_limits.as_ref())
            .context("no available Codex reset credit")
            .map_err(|source| CodexResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source,
            })?;
        if expected_credit_id.is_some_and(|expected| expected != credit_id) {
            return Err(CodexResetError {
                call_may_have_reached_provider: false,
                credit_id: Some(credit_id),
                source: anyhow::anyhow!(
                    "nearest Codex reset credit changed; refresh and confirm again"
                ),
            });
        }
        let mut server = JsonRpcProcess::start(&self.codex_command)
            .await
            .map_err(|source| CodexResetError {
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
            .map_err(|source| CodexResetError {
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
        Ok(CodexResetResult {
            outcome,
            credit_id: Some(credit_id),
        })
    }

    async fn refresh_force(&self) -> UsageSnapshot {
        self.snapshot.lock().await.refreshed_at_ms = 0;
        self.refresh().await
    }
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
pub fn with_session_usage(mut snapshot: UsageSnapshot, sessions: &[SessionMeta]) -> UsageSnapshot {
    for provider in &mut snapshot.providers {
        let Some((_, usage)) = sessions
            .iter()
            .filter(|session| session.provider == provider.provider)
            .filter_map(|session| {
                session
                    .usage
                    .as_ref()
                    .map(|usage| (usage.observed_at_ms, usage))
            })
            .max_by_key(|(observed_at, _)| *observed_at)
        else {
            continue;
        };
        let provider_limits = (provider.provider == "claude-code")
            .then(|| claude_account_limits(&usage.raw))
            .flatten();
        let has_account_limits = provider_limits.is_some();
        if provider.rate_limits.is_none() {
            provider.rate_limits = provider_limits;
        }
        provider.activity = Some(json!({ "session": usage.raw }));
        provider.observed_at_ms = usage.observed_at_ms;
        if has_account_limits {
            provider.status = "available";
            provider.source = "Claude Agent SDK via ACP";
            provider.error = None;
        } else if provider.status != "available" {
            provider.status = "session-only";
            provider.error =
                Some("Account quota is not exposed; showing ACP session activity".into());
        }
    }
    snapshot
}

fn claude_account_limits(raw: &Value) -> Option<Value> {
    let limits = raw.pointer("/_meta/_claude~1rateLimit")?;
    limits.get("utilization").and_then(Value::as_f64)?;
    limits.get("rateLimitType").and_then(Value::as_str)?;
    Some(json!({ "rateLimits": limits }))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn unavailable_providers() -> Vec<ProviderUsage> {
    vec![
        unavailable("codex", "codex-app-server", "Not refreshed yet"),
        unavailable("claude-code", "ACP", "Waiting for ACP rate-limit data"),
        unavailable(
            "claude-deepseek",
            "ACP",
            "Waiting for DeepSeek session usage",
        ),
        unavailable(
            "gemini",
            "ACP",
            "Provider does not expose account limits over ACP",
        ),
    ]
}

fn unavailable(provider: &'static str, source: &'static str, message: &str) -> ProviderUsage {
    ProviderUsage {
        provider,
        status: "unavailable",
        source,
        observed_at_ms: now_ms(),
        account: None,
        rate_limits: None,
        activity: None,
        error: Some(message.to_owned()),
    }
}

fn provider_error(provider: &'static str, source: &'static str, message: String) -> ProviderUsage {
    ProviderUsage {
        error: Some(message),
        ..unavailable(provider, source, "")
    }
}

async fn collect_codex(command: &str) -> Result<ProviderUsage> {
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
        assert_eq!(providers.len(), 4);
        assert!(
            providers
                .iter()
                .any(|provider| provider.provider == "claude-deepseek")
        );
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

    #[test]
    fn claude_rate_limit_event_extracts_account_limits() {
        let limits = claude_account_limits(&json!({
            "sessionUpdate": "usage_update",
            "_meta": { "_claude/rateLimit": {
                "status": "allowed",
                "rateLimitType": "five_hour",
                "utilization": 23.5,
                "resetsAt": 100
            }}
        }))
        .unwrap();
        assert_eq!(limits["rateLimits"]["utilization"], 23.5);
        assert_eq!(limits["rateLimits"]["rateLimitType"], "five_hour");
    }
}
