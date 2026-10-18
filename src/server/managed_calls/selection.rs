//! Evidence for the calling model's decision. No quota thresholds or silent
//! model fallback: the agent chooses a signed preset within session authority.
use crate::usage::UsageSnapshot;
use serde_json::{Value, json};

pub(super) const GUIDANCE: &str = "Choose the strongest available reviewer quota permits; prefer Astra/Fable with ample quota, consider Opus/SOL when constrained, and prefer Max where supported. Never choose a weaker model merely to save quota. You decide using task risk, account AND model windows, reset times and observation freshness. Unknown/stale quota is not ample. Shared account limits still apply after switching models. Honor explicit user/session choices. Pass --agent, --preset and --reason to cowboy review. No silent fallback; re-evaluate a failure before any retry. usedPercent is USED; resetsAt is epoch seconds. Large/unsupported quota payloads: use cowboy.usage.read via MCP.";

pub(super) fn quota(snapshot: &UsageSnapshot, accounts: &[String], now: i64) -> Value {
    let missing: Vec<_> = accounts
        .iter()
        .filter(|account| {
            !snapshot
                .providers
                .iter()
                .any(|usage| usage.provider == account.as_str())
        })
        .collect();
    let accounts: Vec<_> = snapshot.providers.iter().filter(|usage| accounts.iter().any(|account| account == usage.provider)).take(20).map(|usage| {
        let limits = usage.rate_limits.as_ref();
        let bounded = limits.filter(|value| serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 8192));
        json!({
            "account": usage.provider,
            "status": usage.status,
            "observed_at_ms": (usage.observed_at_ms > 0).then_some(usage.observed_at_ms),
            "stale": usage.observed_at_ms <= 0 || now.saturating_sub(usage.observed_at_ms) > snapshot.refresh_interval_ms.max(60_000) * 2 || usage.refresh.as_ref().is_none_or(|refresh| refresh.stale),
            "limits": bounded,
            "needs_read": limits.is_some() && bounded.is_none(),
        })
    }).collect();
    json!({"observed_now_ms":now,"accounts":accounts,"unavailable_accounts":missing})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{ProviderRefreshState, ProviderUsage};

    #[test]
    fn quota_preserves_model_windows_and_marks_stale_missing_and_oversized_evidence() {
        let mut snapshot = UsageSnapshot {
            refreshed_at_ms: 1000,
            next_refresh_at_ms: 61000,
            refresh_interval_ms: 60000,
            reset_schedules: Default::default(),
            refreshing: vec![],
            providers: vec![ProviderUsage {
                provider: "anthropic",
                status: "ok",
                source: "test",
                observed_at_ms: 1000,
                account: None,
                activity: None,
                error: None,
                refresh: Some(ProviderRefreshState {
                    last_attempt_at_ms: 1000,
                    manual_refresh_after_ms: 0,
                    next_auto_refresh_at_ms: 61000,
                    stale: false,
                }),
                rate_limits: Some(json!({"rateLimitsByLimitId": {
                    "claude-five_hour": {"primary":{"usedPercent":20,"resetsAt":42}},
                    "claude-model-Fable": {"primary":{"usedPercent":98,"resetsAt":100}}
                }})),
            }],
        };
        let accounts = vec!["anthropic".into(), "openai".into()];
        let view = quota(&snapshot, &accounts, 2000);
        assert_eq!(
            view["accounts"][0]["limits"]["rateLimitsByLimitId"]["claude-model-Fable"]["primary"]["usedPercent"],
            98
        );
        assert_eq!(view["accounts"][0]["stale"], false);
        assert_eq!(view["unavailable_accounts"][0], "openai");
        assert_eq!(
            quota(&snapshot, &accounts, 200000)["accounts"][0]["stale"],
            true
        );
        assert!(
            quota(&snapshot, &[], 2000)["accounts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        snapshot.providers[0].rate_limits = Some(json!({"large":"x".repeat(8193)}));
        let view = quota(&snapshot, &accounts, 2000);
        assert_eq!(view["accounts"][0]["needs_read"], true);
        assert!(view["accounts"][0]["limits"].is_null());
    }
}
