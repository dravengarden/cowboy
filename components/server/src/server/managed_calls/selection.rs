//! Evidence for the calling model's decision. No quota thresholds or silent
//! model fallback: the agent chooses a signed preset within session authority.
use crate::usage::UsageSnapshot;
use serde_json::{Value, json};

pub(super) const GUIDANCE: &str = "For Review/Design Review, use providers[].review_candidates in order: session preferences override Settings defaults. Only Opus, Fable, Astra and SOL participate. Choose the first preferred family with fresh sufficient account AND model quota; prefer Max within that family. Do not skip a preferred family merely because another is stronger. Unknown/stale quota is not ample; inspect windows and reset times before choosing. Explicit user choice takes precedence within the allowed list. Pass --agent, --preset and --reason; explain any skipped preference. No silent fallback after a failed call: diagnose before retrying. Analysis/image calls are unaffected. usedPercent is USED; resetsAt is epoch seconds. Large quota payloads: use cowboy.usage.read.";

pub(super) fn family(model: &str) -> Option<&'static str> {
    if model == "opus" || model.starts_with("claude-opus-") {
        Some("opus")
    } else if model == "fable" || model.starts_with("claude-fable-") {
        Some("fable")
    } else if model.starts_with("gpt-") && model.ends_with("-astra") {
        Some("astra")
    } else if model.starts_with("gpt-") && model.ends_with("-sol") {
        Some("sol")
    } else {
        None
    }
}

pub(super) fn review_preset(
    choices: &[Value],
    explicit: Option<&str>,
) -> Result<String, &'static str> {
    if let Some(id) = explicit {
        return choices
            .iter()
            .any(|choice| choice["id"].as_str() == Some(id))
            .then(|| id.to_owned())
            .ok_or("review_model_not_allowed");
    }
    choices
        .first()
        .and_then(|choice| choice["id"].as_str())
        .map(str::to_owned)
        .ok_or("preset_unavailable")
}

pub(super) fn candidates(
    presets: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
) -> Vec<Value> {
    let order = if provider == "codex" {
        &preference.gpt
    } else {
        &preference.claude
    };
    let mut choices: Vec<_> = presets
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|preset| {
            let family = family(preset["values"]["model"].as_str()?)?;
            let rank = order.iter().position(|v| v == family)?;
            let effort = preset["values"]["reasoning_effort"]
                .as_str()
                .or(preset["values"]["effort"].as_str())
                .unwrap_or("default");
            let effort_rank = match effort {
                "max" => 0,
                "xhigh" => 1,
                "high" => 2,
                "default" => 3,
                "medium" => 4,
                _ => 5,
            };
            Some((rank, effort_rank, preset.clone()))
        })
        .collect();
    choices.sort_by_key(|(rank, effort, _)| (*rank, *effort));
    choices.into_iter().map(|(_, _, preset)| preset).collect()
}

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
    fn review_order_precedes_strength_and_excludes_weak_or_removed_models() {
        let presets = json!([
            {"id":"fable-max","values":{"model":"claude-fable-5-1[1m]","effort":"max"}},
            {"id":"sonnet-high","values":{"model":"sonnet","effort":"high"}},
            {"id":"opus-default","values":{"model":"opus","effort":"default"}},
            {"id":"opus-high","values":{"model":"opus","effort":"high"}}
        ]);
        let mut preference = super::super::super::agent_tools::ReviewPreferences::default();
        let ids = |p: &super::super::super::agent_tools::ReviewPreferences| {
            candidates(&presets, "claude-code", p)
                .iter()
                .map(|v| v["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&preference), ["opus-high", "opus-default", "fable-max"]);
        let choices = candidates(&presets, "claude-code", &preference);
        assert_eq!(review_preset(&choices, None).unwrap(), "opus-high");
        assert_eq!(
            review_preset(&choices, Some("fable-max")).unwrap(),
            "fable-max"
        );
        assert_eq!(
            review_preset(&choices, Some("sonnet-high")),
            Err("review_model_not_allowed")
        );
        assert_eq!(review_preset(&[], None), Err("preset_unavailable"));
        preference.claude = vec!["fable".into(), "opus".into()];
        assert_eq!(ids(&preference)[0], "fable-max");
        preference.claude = vec!["opus".into()];
        assert!(!ids(&preference).iter().any(|v| v.contains("fable")));
        assert_eq!(family("gpt-6.1-sol"), Some("sol"));
        assert_eq!(family("gpt-6-astra"), Some("astra"));
        assert_eq!(family("gpt-6-luna"), None);
    }

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
