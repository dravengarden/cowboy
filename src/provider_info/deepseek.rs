use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::store::Store;
use crate::usage::ProviderUsage;

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

pub(crate) async fn collect(
    store: Option<&Store>,
    account: &str,
    product: &str,
) -> Result<ProviderUsage> {
    let account_id = crate::usage::intern_usage_str(account.to_owned());
    let product_id = crate::usage::intern_usage_str(product.to_owned());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building provider info client")?;
    let info_urls = std::env::var(crate::plugin_runtime_args::provider_info_urls_env(account))
        .unwrap_or_else(|_| crate::plugin_runtime_args::loopback_info_urls());
    let lanes = crate::plugin_runtime_args::usage_activity_agent_ids(account);
    let product_name = product.to_owned();
    let requests = info_urls
        .split(',')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .enumerate()
        .map(|(index, url)| {
            let client = &client;
            let fallback_agent = lanes.get(index).copied().unwrap_or(account).to_owned();
            let allowed: Vec<&str> = lanes.to_vec();
            let product = product_name.clone();
            async move {
                let mut account =
                    tokio::time::timeout(Duration::from_secs(4), fetch_account(client, url))
                        .await
                        .with_context(|| {
                            format!("{fallback_agent} {product} balance adapter timed out")
                        })?
                        .with_context(|| {
                            format!("{fallback_agent} {product} balance adapter failed")
                        })?;
                if account.agent.is_empty() {
                    account.agent = fallback_agent;
                }
                if !allowed.is_empty() && !allowed.iter().any(|lane| *lane == account.agent) {
                    anyhow::bail!("{product} balance adapter returned an unknown agent lane");
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
                tracing::warn!(provider = account, error = %detail, "balance adapter failed");
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
                .unwrap_or_else(|| format!("no {product} balance adapter configured"))
        );
    }
    let account_views = group_accounts(accounts);
    let available = account_views.iter().any(|account| account.is_available);
    let mut activity = if let Some(store) = store {
        match store
            .provider_usage_summary(account, USAGE_DAYS, USAGE_RETENTION_DAYS)
            .await
        {
            Ok(activity) => activity,
            Err(error) => {
                tracing::warn!(%error, product, "gateway-measured usage is unavailable");
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
        "source": account_id,
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
        provider: account_id,
        status: if available { "available" } else { "exhausted" },
        source: product_id,
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
        .context("querying balance adapter")?
        .error_for_status()
        .context("balance adapter rejected request")?
        .json()
        .await
        .context("decoding balance adapter")
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
