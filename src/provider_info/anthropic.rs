use serde_json::{Value, json};

use crate::usage::ProviderUsage;

pub(crate) fn overlay(provider: &mut ProviderUsage, raw: &Value) {
    let Some(limits) = account_limits(raw) else {
        if provider.status != "available" {
            provider.error = Some(
                "Anthropic account quota is not exposed; showing Claude session activity".into(),
            );
        }
        return;
    };
    if provider.rate_limits.is_none() {
        provider.rate_limits = Some(limits);
    }
    provider.status = "available";
    provider.source = "Claude Agent SDK via ACP";
    provider.error = None;
}

fn account_limits(raw: &Value) -> Option<Value> {
    let limits = raw.pointer("/_meta/_claude~1rateLimit")?;
    limits.get("utilization").and_then(Value::as_f64)?;
    limits.get("rateLimitType").and_then(Value::as_str)?;
    Some(json!({ "rateLimits": limits }))
}

#[cfg(test)]
mod tests {
    use super::account_limits;
    use serde_json::json;

    #[test]
    fn extracts_claude_account_limit_metadata() {
        let limits = account_limits(&json!({"_meta":{"_claude/rateLimit":{
            "utilization":23.5,"rateLimitType":"five_hour","resetsAt":100
        }}}))
        .unwrap();
        assert_eq!(limits["rateLimits"]["utilization"], 23.5);
    }
}
