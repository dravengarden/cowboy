use std::process::Stdio;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::provider::LaunchSpec;
use crate::usage::ProviderUsage;

use super::xai_account::AccountSnapshot;

const BILLING_METHOD: &str = "_x.ai/billing";

pub(crate) async fn collect(
    spec: &LaunchSpec,
    account_id: &'static str,
    source: &'static str,
    error_auth: Option<&str>,
    error_fetch: Option<&str>,
) -> Result<ProviderUsage> {
    let mut server = GrokRpcProcess::start(
        spec,
        source,
        error_auth.map(str::to_owned),
        error_fetch.map(str::to_owned),
    )
    .await?;
    let billing = server.request(BILLING_METHOD, json!({})).await?;
    let account = match super::xai_account::collect().await {
        Ok(account) => Some(account),
        Err(error) => {
            tracing::warn!(provider = account_id, %error, "reading xAI account metadata");
            None
        }
    };
    Ok(from_billing(billing, account.as_ref(), account_id, source))
}

fn from_billing(
    mut billing: Value,
    account: Option<&AccountSnapshot>,
    provider: &'static str,
    source: &'static str,
) -> ProviderUsage {
    let tier = billing
        .get("subscription_tier")
        .or_else(|| billing.get("subscriptionTier"))
        .and_then(Value::as_str)
        .filter(|tier| !tier.trim().is_empty());
    let plan = account
        .and_then(|account| account.plan.as_deref())
        .or(tier)
        .map(str::to_owned);
    if let Some(resets) = account.and_then(|account| account.resets.as_ref())
        && let Some(root) = billing.as_object_mut()
    {
        root.insert(
            "rateLimitResetCredits".to_owned(),
            json!({
                "availableCount": resets.len(),
                "credits": resets.iter().map(|reset| json!({
                    "id": reset.id,
                    "status": "available",
                    "title": "Usage reset",
                    "grantedAt": reset.granted_at,
                    "expiresAt": reset.expires_at,
                })).collect::<Vec<_>>(),
            }),
        );
    }
    ProviderUsage {
        provider,
        status: "available",
        source,
        observed_at_ms: crate::usage::now_ms(),
        account: plan.map(|plan| json!({ "account": { "planType": plan } })),
        rate_limits: Some(billing),
        activity: None,
        error: None,
        refresh: None,
    }
}

struct GrokRpcProcess {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    product: String,
    error_auth: Option<String>,
    error_fetch: Option<String>,
}

impl GrokRpcProcess {
    async fn start(
        spec: &LaunchSpec,
        product: &str,
        error_auth: Option<String>,
        error_fetch: Option<String>,
    ) -> Result<Self> {
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
            product: product.to_owned(),
            error_auth,
            error_fetch,
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
            let message: Value = serde_json::from_str(&line).context("parse Grok ACP response")?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                bail!(
                    "{}",
                    rpc_error_message(
                        method,
                        error,
                        &self.product,
                        self.error_auth.as_deref(),
                        self.error_fetch.as_deref(),
                    )
                );
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

fn rpc_error_message(
    method: &str,
    error: &Value,
    product: &str,
    error_auth: Option<&str>,
    error_fetch: Option<&str>,
) -> String {
    let code = error.get("code").and_then(Value::as_i64);
    let detail = error
        .get("data")
        .and_then(Value::as_str)
        .or_else(|| error.get("message").and_then(Value::as_str))
        .map(str::trim)
        .filter(|detail| !detail.is_empty());

    if method == BILLING_METHOD && code == Some(-32601) {
        return format!("installed {product} does not expose subscription billing over ACP yet");
    }
    if code == Some(-32000)
        && detail.is_some_and(|detail| {
            detail
                .to_ascii_lowercase()
                .contains("authentication required")
        })
    {
        return error_auth
            .unwrap_or("Sign in again to refresh usage.")
            .to_owned();
    }

    let operation = if method == BILLING_METHOD {
        error_fetch
            .unwrap_or("usage could not be fetched")
            .to_owned()
    } else {
        format!("{product} ACP request failed")
    };
    if let Some(detail) = detail {
        return format!("{operation}: {detail}");
    }
    code.map_or_else(
        || operation.to_owned(),
        |code| format!("{operation} (ACP error {code})"),
    )
}

impl Drop for GrokRpcProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::{BILLING_METHOD, from_billing, rpc_error_message};
    use crate::provider_info::xai_account::{AccountSnapshot, ResetCredit};
    use crate::usage::ProviderUsage;
    use serde_json::{Value, json};

    const AUTH: &str = "Sign in to Grok Build in Machines, then refresh xAI usage.";
    const FETCH: &str = "Grok Build could not fetch xAI usage";

    fn billing(value: Value, account: Option<&AccountSnapshot>) -> ProviderUsage {
        from_billing(value, account, "xai", "xAI")
    }

    fn rpc(error: &Value) -> String {
        rpc_error_message(BILLING_METHOD, error, "xAI", Some(AUTH), Some(FETCH))
    }

    #[test]
    fn billing_uses_groks_namespaced_acp_method() {
        assert_eq!(BILLING_METHOD, "_x.ai/billing");
    }

    #[test]
    fn billing_keeps_the_official_shape_and_surfaces_the_tier() {
        let usage = billing(
            json!({
                "config": {
                    "creditUsagePercent": 37.5,
                    "currentPeriod": {
                        "type": "USAGE_PERIOD_TYPE_WEEKLY",
                        "end": "2026-08-17T00:00:00Z"
                    }
                },
                "subscription_tier": "SuperGrok Heavy"
            }),
            None,
        );
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

    #[test]
    fn billing_accepts_the_legacy_camel_case_subscription_tier() {
        let usage = billing(json!({ "subscriptionTier": "SuperGrok" }), None);
        assert_eq!(
            usage
                .account
                .as_ref()
                .and_then(|value| value.pointer("/account/planType")),
            Some(&json!("SuperGrok"))
        );
    }

    #[test]
    fn authoritative_subscription_and_reset_fix_the_free_tier() {
        let account = AccountSnapshot {
            plan: Some("SuperGrok".to_owned()),
            resets: Some(vec![ResetCredit {
                id: "reset-a".to_owned(),
                granted_at: Some(100),
                expires_at: Some(200),
            }]),
        };
        let usage = billing(
            json!({
                "config": { "isUnifiedBillingUser": true },
                "subscription_tier": "Free"
            }),
            Some(&account),
        );
        assert_eq!(
            usage
                .account
                .as_ref()
                .and_then(|value| value.pointer("/account/planType")),
            Some(&json!("SuperGrok"))
        );
        assert_eq!(
            usage
                .rate_limits
                .as_ref()
                .and_then(|value| value.pointer("/rateLimitResetCredits/availableCount")),
            Some(&json!(1))
        );
        assert_eq!(
            usage
                .rate_limits
                .as_ref()
                .and_then(|value| value.pointer("/rateLimitResetCredits/credits/0/expiresAt")),
            Some(&json!(200))
        );
    }

    #[test]
    fn billing_authentication_error_becomes_an_actionable_sign_in_message() {
        let error = json!({
            "code": -32000,
            "message": "Authentication required",
            "data": "Authentication required to fetch billing data"
        });
        let message = rpc(&error);
        assert_eq!(message, AUTH);
        assert!(!message.contains('{'));
        assert!(!message.contains(BILLING_METHOD));
    }

    #[test]
    fn billing_errors_keep_text_details_without_exposing_rpc_json() {
        let message = rpc(&json!({
            "code": -32001,
            "message": "Billing is temporarily unavailable"
        }));
        assert_eq!(
            message,
            "Grok Build could not fetch xAI usage: Billing is temporarily unavailable"
        );
        assert!(!message.contains("-32001"));
    }

    #[test]
    fn missing_billing_method_keeps_the_upgrade_guidance() {
        assert_eq!(
            rpc(&json!({ "code": -32601, "message": "Method not found" })),
            "installed xAI does not expose subscription billing over ACP yet"
        );
    }
}
