use std::process::Stdio;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::provider::LaunchSpec;
use crate::usage::ProviderUsage;

const BILLING_METHOD: &str = "_x.ai/billing";
pub(crate) const SOURCE: &str = "Grok Build ACP _x.ai/billing";

pub(crate) async fn collect(spec: &LaunchSpec) -> Result<ProviderUsage> {
    let mut server = GrokRpcProcess::start(spec).await?;
    let billing = server.request(BILLING_METHOD, json!({})).await?;
    Ok(from_billing(billing))
}

fn from_billing(billing: Value) -> ProviderUsage {
    let tier = billing
        .get("subscriptionTier")
        .and_then(Value::as_str)
        .filter(|tier| !tier.trim().is_empty());
    ProviderUsage {
        provider: "xai",
        status: "available",
        source: SOURCE,
        observed_at_ms: crate::usage::now_ms(),
        account: tier.map(|tier| json!({ "account": { "planType": tier } })),
        rate_limits: Some(billing),
        activity: None,
        error: None,
    }
}

struct GrokRpcProcess {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl GrokRpcProcess {
    async fn start(spec: &LaunchSpec) -> Result<Self> {
        let mut command = Command::new(&spec.command);
        command.args(&spec.args);
        for (key, _) in std::env::vars_os() {
            if key
                .to_str()
                .is_some_and(|name| spec.removes_inherited_env(name))
            {
                command.env_remove(key);
            }
        }
        command.envs(&spec.env);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("start Grok usage collector: {}", spec.command))?;
        let stdin = child.stdin.take().context("Grok collector stdin")?;
        let stdout = child.stdout.take().context("Grok collector stdout")?;
        let mut process = Self {
            child,
            stdin,
            lines: BufReader::new(stdout).lines(),
            next_id: 1,
        };
        process
            .request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "fs": { "readTextFile": false, "writeTextFile": false },
                        "terminal": false
                    },
                    "clientInfo": {
                        "name": "cowboy-usage",
                        "title": "Cowboy",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;
        Ok(process)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let request = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        self.stdin.write_all(request.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await.context("flush Grok ACP request")?;
        loop {
            let line = self
                .lines
                .next_line()
                .await
                .context("read Grok ACP")?
                .context("Grok ACP closed")?;
            let message: Value = serde_json::from_str(&line)
                .with_context(|| format!("parse Grok ACP message: {line}"))?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                if error.get("code").and_then(Value::as_i64) == Some(-32601)
                    && method == BILLING_METHOD
                {
                    bail!("installed Grok Build does not expose subscription billing over ACP yet");
                }
                bail!("{method}: {error}");
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

impl Drop for GrokRpcProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::{BILLING_METHOD, from_billing};
    use serde_json::json;

    #[test]
    fn billing_uses_groks_namespaced_acp_method() {
        assert_eq!(BILLING_METHOD, "_x.ai/billing");
    }

    #[test]
    fn billing_keeps_the_official_shape_and_surfaces_the_tier() {
        let usage = from_billing(json!({
            "config": {
                "creditUsagePercent": 37.5,
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "end": "2026-08-17T00:00:00Z"
                }
            },
            "subscriptionTier": "SuperGrok Heavy"
        }));
        assert_eq!(usage.provider, "xai");
        assert_eq!(usage.status, "available");
        assert_eq!(
            usage
                .account
                .as_ref()
                .and_then(|value| value.pointer("/account/planType")),
            Some(&json!("SuperGrok Heavy"))
        );
        assert_eq!(
            usage
                .rate_limits
                .as_ref()
                .and_then(|value| value.pointer("/config/creditUsagePercent")),
            Some(&json!(37.5))
        );
    }
}
