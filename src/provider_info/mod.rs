//! Account-provider information adapters.
//!
//! Agent runtimes are only data sources. Info cards are keyed by the account
//! provider so Codex and Claude Code sessions backed by one DeepSeek key never
//! become duplicate account cards.

mod anthropic;
mod deepseek;
mod deepseek_pricing;
mod gemini;
mod openai;
mod xai;
mod xai_account;

use serde_json::{Value, json};

use crate::core::SessionMeta;
use crate::plugin_host::{PluginUsageSpec, UsageSessionOverlay};
use crate::usage::ProviderUsage;

pub(crate) use deepseek::collect as collect_deepseek;
pub(crate) use deepseek_pricing::decorate_activity as decorate_deepseek_activity;
pub(crate) use openai::collect as collect_openai;
pub(crate) use xai::collect as collect_xai;
pub(crate) use xai_account::redeem_reset as redeem_xai_reset;

pub(crate) fn overlay_session_usage(
    mut snapshot: crate::usage::UsageSnapshot,
    sessions: &[SessionMeta],
    catalog: &crate::provider_catalog::ProviderCatalog,
    bindings: &[PluginUsageSpec],
) -> crate::usage::UsageSnapshot {
    for provider in &mut snapshot.providers {
        let Some((_, session, usage)) = sessions
            .iter()
            .filter(|session| {
                catalog.account_usage_provider(
                    &session.provider,
                    &session.provider_version,
                    &session.provider_generation_digest,
                ) == Some(provider.provider)
            })
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
        let binding = bindings
            .iter()
            .find(|binding| binding.account == provider.provider);
        let empty = binding.and_then(|binding| binding.empty.as_deref());
        let source = binding
            .map(|binding| crate::usage::intern_usage_str(binding.product_label().to_owned()));
        match session_overlay(bindings, provider.provider) {
            UsageSessionOverlay::AnthropicRateLimit => {
                anthropic::overlay(provider, &usage.raw, empty, source);
            }
            UsageSessionOverlay::GeminiSessionOnly => {
                gemini::overlay(provider, empty);
                if let Some(source) = source {
                    provider.source = source;
                }
            }
            UsageSessionOverlay::None | UsageSessionOverlay::Unknown => {}
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

fn session_overlay(bindings: &[PluginUsageSpec], account: &str) -> UsageSessionOverlay {
    bindings
        .iter()
        .find(|binding| binding.account == account)
        .map_or(UsageSessionOverlay::None, |binding| binding.session_overlay)
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
        refresh: None,
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
    use super::*;
    use crate::plugin_host::{
        UsageCollectorKind, UsageErrorKind, UsageLimitParserKind, UsageResetClaim, UsageWidgetKind,
    };

    fn binding(account: &str, overlay: UsageSessionOverlay) -> PluginUsageSpec {
        PluginUsageSpec {
            account: account.to_owned(),
            collector: UsageCollectorKind::Session,
            reset: None,
            product: None,
            parser: UsageLimitParserKind::GenericBuckets,
            error: UsageErrorKind::Raw,
            error_auth: None,
            error_config: None,
            error_fetch: None,
            order: None,
            top_bar_windows: Vec::new(),
            widget: UsageWidgetKind::None,
            widget_shape: crate::plugin_host::UsageWidgetShape::None,
            widget_window: None,
            reset_claim: UsageResetClaim::AfterSuccess,
            session_overlay: overlay,
            empty: None,
            available_status: None,
            omit_empty_limits: false,
            limit_id_prefix: None,
            limit_labels: Vec::new(),
            widget_balance_label: None,
            widget_spend_label: None,
            activity_agents: Vec::new(),
            activity_models: Vec::new(),
            cache_protection: None,
            collector_argv: Vec::new(),
            activity: false,
        }
    }

    #[test]
    fn session_overlay_follows_plugin_binding_not_account_id() {
        let bindings = [binding(
            "custom-claude",
            UsageSessionOverlay::AnthropicRateLimit,
        )];
        assert_eq!(
            session_overlay(&bindings, "custom-claude"),
            UsageSessionOverlay::AnthropicRateLimit
        );
        assert_eq!(
            session_overlay(&bindings, "anthropic"),
            UsageSessionOverlay::None
        );
        assert_eq!(
            session_overlay(
                &[binding("gemini", UsageSessionOverlay::GeminiSessionOnly)],
                "gemini"
            ),
            UsageSessionOverlay::GeminiSessionOnly
        );
    }
}
