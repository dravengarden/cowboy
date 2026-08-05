//! Account-provider information adapters.
//!
//! Agent runtimes are only data sources. Info cards are keyed by the account
//! provider so Codex and Claude Code sessions backed by one DeepSeek key never
//! become duplicate account cards.

mod anthropic;
mod deepseek;
mod gemini;
mod openai;

use serde_json::{Value, json};

use crate::core::SessionMeta;
use crate::usage::ProviderUsage;

pub(crate) use deepseek::collect as collect_deepseek;
pub(crate) use openai::collect as collect_openai;

pub(crate) const PROVIDERS: [&str; 4] = ["openai", "anthropic", "deepseek", "gemini"];

pub(crate) fn account_provider(agent: &str) -> Option<&'static str> {
    match agent {
        "codex" => Some("openai"),
        "claude" | "claude-code" => Some("anthropic"),
        "codex-deepseek" | "claude-deepseek" => Some("deepseek"),
        "gemini" => Some("gemini"),
        _ => None,
    }
}

pub(crate) fn overlay_session_usage(
    mut snapshot: crate::usage::UsageSnapshot,
    sessions: &[SessionMeta],
) -> crate::usage::UsageSnapshot {
    for provider in &mut snapshot.providers {
        let Some((_, session, usage)) = sessions
            .iter()
            .filter(|session| account_provider(&session.provider) == Some(provider.provider))
            .filter_map(|session| {
                session
                    .usage
                    .as_ref()
                    .map(|usage| (usage.observed_at_ms, session, usage))
            })
            .max_by_key(|(observed_at, _, _)| *observed_at)
        else {
            continue;
        };
        if provider.provider == "anthropic" {
            anthropic::overlay(provider, &usage.raw);
        } else if provider.provider == "gemini" {
            gemini::overlay(provider);
        }
        let latest = json!({ "agent": session.provider, "session": usage.raw });
        match provider.activity.as_mut().and_then(Value::as_object_mut) {
            Some(activity) => {
                activity.insert("latest_session".to_owned(), latest);
            }
            None => provider.activity = Some(json!({ "latest_session": latest })),
        }
        provider.observed_at_ms = provider.observed_at_ms.max(usage.observed_at_ms);
        if provider.status != "available" && provider.status != "exhausted" {
            provider.status = "session-only";
        }
    }
    snapshot
}

pub(crate) fn unavailable(
    provider: &'static str,
    source: &'static str,
    message: &str,
) -> ProviderUsage {
    ProviderUsage {
        provider,
        status: "unavailable",
        source,
        observed_at_ms: crate::usage::now_ms(),
        account: None,
        rate_limits: None,
        activity: None,
        error: Some(message.to_owned()),
    }
}

pub(crate) fn error(
    provider: &'static str,
    source: &'static str,
    message: String,
) -> ProviderUsage {
    ProviderUsage {
        error: Some(message),
        ..unavailable(provider, source, "")
    }
}

#[cfg(test)]
mod tests {
    use super::account_provider;

    #[test]
    fn agent_shells_map_to_one_account_provider() {
        assert_eq!(account_provider("codex"), Some("openai"));
        assert_eq!(account_provider("claude-code"), Some("anthropic"));
        assert_eq!(account_provider("codex-deepseek"), Some("deepseek"));
        assert_eq!(account_provider("claude-deepseek"), Some("deepseek"));
        assert_eq!(account_provider("gemini"), Some("gemini"));
    }
}
