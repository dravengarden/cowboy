use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::store::Store;
use crate::usage::ProviderUsage;

const DEFAULT_INFO_URL: &str = "http://127.0.0.1:61137/provider-info";
const DEFAULT_FALLBACK_INFO_URL: &str = "http://127.0.0.1:61138/provider-info";
const USAGE_DAYS: i32 = 14;
const USAGE_RETENTION_DAYS: i32 = 30;

#[derive(Debug, Deserialize)]
struct AccountInfo {
    #[serde(default)]
    agent: String,
    account_fingerprint: String,
    is_available: bool,
    balance_infos: Vec<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountView {
    account_fingerprint: String,
    agents: Vec<String>,
    is_available: bool,
    balance_infos: Vec<Value>,
}

pub(crate) async fn collect(store: Option<&Store>) -> Result<ProviderUsage> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building DeepSeek info client")?;
    let info_urls = std::env::var("COWBOY_PROVIDER_INFO_DEEPSEEK_URLS")
        .unwrap_or_else(|_| format!("{DEFAULT_INFO_URL},{DEFAULT_FALLBACK_INFO_URL}"));
    let requests = info_urls
        .split(',')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .enumerate()
        .map(|(index, url)| {
            let client = &client;
            async move {
                let fallback_agent = if index == 0 { "codex" } else { "claude" };
                let mut account =
                    tokio::time::timeout(Duration::from_secs(4), fetch_account(client, url))
                        .await
                        .with_context(|| {
                            format!("{fallback_agent} DeepSeek balance adapter timed out")
                        })?
                        .with_context(|| {
                            format!("{fallback_agent} DeepSeek balance adapter failed")
                        })?;
                if account.agent.is_empty() {
                    account.agent = fallback_agent.to_owned();
                }
                if !matches!(account.agent.as_str(), "codex" | "claude") {
                    anyhow::bail!("DeepSeek balance adapter returned an unknown agent lane");
                }
                Ok::<_, anyhow::Error>(account)
            }
        });
    let mut accounts = Vec::new();
    let mut adapter_errors = Vec::new();
    let mut adapter_failure_details = Vec::new();
    for result in join_all(requests).await {
        match result {
            Ok(account) => accounts.push(account),
            Err(error) => {
                let public_error = error.to_string();
                let detail = format!("{error:#}");
                tracing::warn!(provider = "deepseek", error = %detail, "balance adapter failed");
                adapter_errors.push(public_error);
                adapter_failure_details.push(detail);
            }
        }
    }
    if accounts.is_empty() {
        anyhow::bail!(
            "{}",
            adapter_failure_details
                .pop()
                .unwrap_or_else(|| "no DeepSeek balance adapter configured".to_owned())
        );
    }
    let account_views = group_accounts(accounts);
    let available = account_views.iter().any(|account| account.is_available);
    let mut activity = if let Some(store) = store {
        match store
            .provider_usage_summary("deepseek", USAGE_DAYS, USAGE_RETENTION_DAYS)
            .await
        {
            Ok(activity) => activity,
            Err(error) => {
                tracing::warn!(%error, "gateway-measured DeepSeek usage is unavailable");
                json!({
                    "source": "cowboy", "windowDays": USAGE_DAYS,
                    "retentionDays": USAGE_RETENTION_DAYS, "availableAgents": [],
                    "summary": null, "coverage": { "producers": [] },
                    "telemetryError": "Cowboy request telemetry is unavailable",
                })
            }
        }
    } else {
        json!({
            "source": "cowboy", "windowDays": USAGE_DAYS,
            "retentionDays": USAGE_RETENTION_DAYS, "availableAgents": [],
            "summary": null, "coverage": { "producers": [] },
            "unavailableReason": "Cowboy persistence is disabled",
        })
    };
    super::deepseek_pricing::decorate_activity(&mut activity);
    let mut account = json!({
        "source": "deepseek",
        "accounts": &account_views,
        "adapterErrors": adapter_errors,
    });
    // Keep the additive v1 shape while both cached and current Web bundles may
    // coexist. It is unambiguous only when both isolated lanes use one account.
    if let [single] = account_views.as_slice()
        && let Some(object) = account.as_object_mut()
    {
        object.insert(
            "accountFingerprint".to_owned(),
            single.account_fingerprint.clone().into(),
        );
        object.insert("isAvailable".to_owned(), single.is_available.into());
        object.insert(
            "balanceInfos".to_owned(),
            single.balance_infos.clone().into(),
        );
    }
    Ok(ProviderUsage {
        provider: "deepseek",
        status: if available { "available" } else { "exhausted" },
        source: "DeepSeek account adapter",
        observed_at_ms: crate::usage::now_ms(),
        account: Some(account),
        rate_limits: None,
        activity: Some(activity),
        error: None,
        refresh: None,
    })
}

fn group_accounts(accounts: Vec<AccountInfo>) -> Vec<AccountView> {
    let mut grouped = std::collections::BTreeMap::<String, AccountView>::new();
    for account in accounts {
        let entry = grouped
            .entry(account.account_fingerprint.clone())
            .or_insert_with(|| AccountView {
                account_fingerprint: account.account_fingerprint,
                agents: Vec::new(),
                is_available: account.is_available,
                balance_infos: account.balance_infos,
            });
        entry.is_available |= account.is_available;
        if !entry.agents.contains(&account.agent) {
            entry.agents.push(account.agent);
            entry.agents.sort();
        }
    }
    grouped.into_values().collect()
}

async fn fetch_account(client: &reqwest::Client, url: &str) -> Result<AccountInfo> {
    client
        .get(url)
        .send()
        .await
        .context("querying DeepSeek balance adapter")?
        .error_for_status()
        .context("DeepSeek balance adapter rejected request")?
        .json()
        .await
        .context("decoding DeepSeek balance adapter")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(agent: &str, fingerprint: &str, total: &str) -> AccountInfo {
        AccountInfo {
            agent: agent.to_owned(),
            account_fingerprint: fingerprint.to_owned(),
            is_available: true,
            balance_infos: vec![json!({ "currency": "CNY", "total_balance": total })],
        }
    }

    #[test]
    fn shared_credentials_collapse_without_merging_agent_lanes() {
        let grouped = group_accounts(vec![
            account("codex", "0123456789abcdef", "10.00"),
            account("claude", "0123456789abcdef", "10.00"),
        ]);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].agents, ["claude", "codex"]);
    }

    #[test]
    fn isolated_credentials_keep_independent_balances() {
        let grouped = group_accounts(vec![
            account("codex", "0123456789abcdef", "10.00"),
            account("claude", "fedcba9876543210", "20.00"),
        ]);
        assert_eq!(grouped.len(), 2);
        assert_ne!(
            grouped[0].account_fingerprint,
            grouped[1].account_fingerprint
        );
    }
}
