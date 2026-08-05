use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::future::join_all;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::usage::ProviderUsage;

const DEFAULT_INFO_URL: &str = "http://127.0.0.1:8088/provider-info";
const DEFAULT_FALLBACK_INFO_URL: &str = "http://127.0.0.1:8089/provider-info";
const DEFAULT_LOGS_URL: &str = "http://127.0.0.1:6302/select/logsql/query";
const TELEMETRY_DAYS: i64 = 14;

#[derive(Debug, Deserialize)]
struct AccountInfo {
    account_fingerprint: String,
    is_available: bool,
    balance_infos: Vec<Value>,
}

#[derive(Default)]
struct Totals {
    requests: u64,
    errors: u64,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_hit: u64,
    cache_miss: u64,
}

impl Totals {
    fn add(&mut self, fields: &BTreeMap<&str, &str>) {
        self.requests = self.requests.saturating_add(1);
        let status = parse_u64(fields.get("status").copied());
        if status >= 400 {
            self.errors = self.errors.saturating_add(1);
        }
        self.input = self
            .input
            .saturating_add(parse_u64(fields.get("input_tokens").copied()));
        self.output = self
            .output
            .saturating_add(parse_u64(fields.get("output_tokens").copied()));
        self.reasoning = self
            .reasoning
            .saturating_add(parse_u64(fields.get("reasoning_tokens").copied()));
        self.cache_hit = self
            .cache_hit
            .saturating_add(parse_u64(fields.get("cache_hit_tokens").copied()));
        self.cache_miss = self
            .cache_miss
            .saturating_add(parse_u64(fields.get("cache_miss_tokens").copied()));
    }

    fn json(&self) -> Value {
        json!({
            "requests": self.requests, "errors": self.errors,
            "inputTokens": self.input, "outputTokens": self.output,
            "reasoningTokens": self.reasoning,
            "cacheHitTokens": self.cache_hit, "cacheMissTokens": self.cache_miss,
        })
    }
}

pub(crate) async fn collect() -> Result<ProviderUsage> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building DeepSeek info client")?;
    let info_urls = std::env::var("COWBOY_PROVIDER_INFO_DEEPSEEK_URLS")
        .unwrap_or_else(|_| format!("{DEFAULT_INFO_URL},{DEFAULT_FALLBACK_INFO_URL}"));
    let account_requests = info_urls
        .split(',')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(|info_url| async {
            tokio::time::timeout(Duration::from_secs(4), fetch_account(&client, info_url))
                .await
                .context("DeepSeek balance adapter timed out")?
        });
    let mut accounts = Vec::new();
    let mut last_error = None;
    for result in join_all(account_requests).await {
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

    let logs_url = std::env::var("COWBOY_PROVIDER_INFO_LOGS_URL")
        .unwrap_or_else(|_| DEFAULT_LOGS_URL.to_owned());
    let query = format!(
        "_time:{TELEMETRY_DAYS}d cowboy_provider_usage | sort by (_time asc) | limit 100000"
    );
    let mut logs_url = reqwest::Url::parse(&logs_url).context("parsing DeepSeek telemetry URL")?;
    logs_url.query_pairs_mut().append_pair("query", &query);
    let logs = match tokio::time::timeout(Duration::from_secs(7), async {
        client
            .get(logs_url)
            .send()
            .await
            .context("querying DeepSeek usage telemetry")?
            .error_for_status()
            .context("DeepSeek telemetry query failed")?
            .text()
            .await
            .context("reading DeepSeek usage telemetry")
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(anyhow::anyhow!(
            "DeepSeek telemetry query timed out: {error}"
        )),
    };

    let mut total = Totals::default();
    let mut daily = BTreeMap::<String, Totals>::new();
    let mut by_agent = BTreeMap::<String, Totals>::new();
    let mut hosts = BTreeSet::<String>::new();
    let telemetry_error = logs.as_ref().err().map(ToString::to_string);
    for line in logs.as_deref().unwrap_or_default().lines().take(100_000) {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let unit = record.get("_SYSTEMD_UNIT").and_then(Value::as_str);
        if !matches!(
            unit,
            Some("codex-deepseek.service" | "claude-deepseek.service")
        ) {
            continue;
        }
        let Some(message) = record.get("_msg").and_then(Value::as_str) else {
            continue;
        };
        let fields = log_fields(message);
        if fields.get("account").copied() != Some(account.account_fingerprint.as_str()) {
            continue;
        }
        total.add(&fields);
        if let Some(day) = record
            .get("_time")
            .and_then(Value::as_str)
            .and_then(|time| time.get(..10))
        {
            daily.entry(day.to_owned()).or_default().add(&fields);
        }
        if let Some(agent) = fields.get("agent") {
            by_agent
                .entry((*agent).to_owned())
                .or_default()
                .add(&fields);
        }
        if let Some(host) = record.get("host").and_then(Value::as_str) {
            hosts.insert(host.to_owned());
        }
    }
    let daily = daily
        .into_iter()
        .map(|(day, totals)| json!({"day":day,"totals":totals.json()}))
        .collect::<Vec<_>>();
    let by_agent = by_agent
        .into_iter()
        .map(|(agent, totals)| (agent, totals.json()))
        .collect::<serde_json::Map<_, _>>();
    let observed_at_ms = crate::usage::now_ms();
    Ok(ProviderUsage {
        provider: "deepseek",
        status: if account.is_available {
            "available"
        } else {
            "exhausted"
        },
        source: "DeepSeek API + 14d gateway telemetry",
        observed_at_ms,
        account: Some(
            json!({"isAvailable":account.is_available,"balanceInfos":account.balance_infos}),
        ),
        rate_limits: None,
        activity: Some(json!({
            "summary": total.json(), "daily": daily, "byAgent": by_agent,
            "hosts": hosts, "retentionDays": TELEMETRY_DAYS,
            "telemetryError": telemetry_error,
        })),
        error: None,
    })
}

async fn fetch_account(client: &reqwest::Client, info_url: &str) -> Result<AccountInfo> {
    client
        .get(info_url)
        .send()
        .await
        .context("querying DeepSeek balance adapter")?
        .error_for_status()
        .context("DeepSeek balance adapter rejected request")?
        .json::<AccountInfo>()
        .await
        .context("decoding DeepSeek balance adapter")
}

fn log_fields(message: &str) -> BTreeMap<&str, &str> {
    message
        .split_ascii_whitespace()
        .filter_map(|part| part.split_once('='))
        .collect()
}

fn parse_u64(value: Option<&str>) -> u64 {
    value
        .and_then(|value| value.trim_matches('"').parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{Totals, log_fields};

    #[test]
    fn parses_only_stable_usage_fields() {
        let fields = log_fields(
            "2026 INFO cowboy_provider_usage schema=1 account=abc agent=claude status=200 input_tokens=10 output_tokens=3 cache_hit_tokens=8 cache_miss_tokens=2",
        );
        let mut totals = Totals::default();
        totals.add(&fields);
        assert_eq!(totals.requests, 1);
        assert_eq!(totals.input, 10);
        assert_eq!(totals.output, 3);
        assert_eq!(totals.cache_hit, 8);
        assert_eq!(totals.cache_miss, 2);
    }
}
