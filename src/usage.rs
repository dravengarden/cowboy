//! Provider usage aggregation.
//!
//! Account-level limits are deliberately separate from ACP session usage. ACP
//! reports context/cost for one session; provider collectors add plan windows,
//! reset times, credits, and account activity when an official interface exists.

use std::process::Stdio;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context as _, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::core::SessionMeta;

pub const AUTO_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

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
}

#[derive(Clone)]
pub struct UsageService {
    codex_command: String,
    snapshot: Arc<Mutex<UsageSnapshot>>,
    refresh_lock: Arc<Mutex<()>>,
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
            })),
            refresh_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn snapshot(&self) -> UsageSnapshot {
        self.snapshot.lock().await.clone()
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
                    "gemini",
                    "ACP",
                    "Provider does not expose account limits over ACP",
                ),
            ],
        };
        *self.snapshot.lock().await = next.clone();
        next
    }
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
        let provider_limits = usage
            .raw
            .pointer("/_meta/_claude~1rateLimit")
            .cloned()
            .map(|rate_limits| json!({ "rateLimits": rate_limits }));
        if provider.rate_limits.is_none() {
            provider.rate_limits = provider_limits;
        }
        provider.activity = Some(json!({ "session": usage.raw }));
        provider.observed_at_ms = usage.observed_at_ms;
        if provider.status != "available" {
            provider.status = "session-only";
            provider.error = Some("Account limits unavailable; showing ACP session usage".into());
        }
    }
    snapshot
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
        assert_eq!(providers.len(), 3);
        assert!(providers
            .iter()
            .all(|p| p.status == "unavailable" && p.error.is_some()));
    }
}
