use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::future::join_all;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::store::Store;
use crate::usage::ProviderUsage;

const DEFAULT_INFO_URL: &str = "http://127.0.0.1:43871/provider-info";
const DEFAULT_FALLBACK_INFO_URL: &str = "http://127.0.0.1:43872/provider-info";
const USAGE_DAYS: i32 = 14;

#[derive(Debug, Deserialize)]
struct AccountInfo {
    account_fingerprint: String,
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
        .map(|url| async {
            tokio::time::timeout(Duration::from_secs(4), fetch_account(&client, url))
                .await
                .context("DeepSeek balance adapter timed out")?
        });
    let mut accounts = Vec::new();
    let mut last_error = None;
    for result in join_all(requests).await {
        match result {
            Ok(account) => accounts.push(account),
            Err(error) => last_error = Some(error),
        }
    }
    let account = accounts.pop().ok_or_else(|| {
        last_error.unwrap_or_else(|| anyhow::anyhow!("no DeepSeek balance adapter configured"))
    })?;
    if accounts
        .iter()
        .any(|candidate| candidate.account_fingerprint != account.account_fingerprint)
    {
        anyhow::bail!("DeepSeek gateways use different accounts; refusing to merge their usage");
    }
    let activity = if let Some(store) = store {
        store
            .provider_usage_summary("deepseek", &account.account_fingerprint, USAGE_DAYS)
            .await
            .context("loading Cowboy-measured DeepSeek usage")?
    } else {
        json!({
            "source": "cowboy", "retentionDays": USAGE_DAYS,
            "summary": null, "coverage": { "producers": [] },
            "unavailableReason": "Cowboy persistence is disabled",
        })
    };
    Ok(ProviderUsage {
        provider: "deepseek",
        status: if account.is_available {
            "available"
        } else {
            "exhausted"
        },
        source: "DeepSeek account adapter",
        observed_at_ms: crate::usage::now_ms(),
        account: Some(json!({
            "source": "deepseek", "accountFingerprint": account.account_fingerprint,
            "isAvailable": account.is_available, "balanceInfos": account.balance_infos,
        })),
        rate_limits: None,
        activity: Some(activity),
        error: None,
    })
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
