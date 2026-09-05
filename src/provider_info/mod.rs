//! Account-provider information adapters.
//!
//! Agent runtimes are only data sources. Info cards are keyed by the account
//! provider so Codex and Claude Code sessions backed by one DeepSeek key never
//! become duplicate account cards.

use serde_json::{Value, json};

use crate::core::SessionMeta;
use crate::plugin_host::{PluginUsageSpec, UsageSessionRateLimits};
use crate::usage::ProviderUsage;

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
                catalog
                    .account_usage_provider(
                        &session.provider,
                        &session.provider_version,
                        &session.provider_generation_digest,
                    )
                    .as_deref()
                    == Some(provider.provider)
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
        if provider.status != "available"
            && let Some(empty) = empty
        {
            provider.error = Some(empty.to_owned());
        }
        if let Some(projection) = binding.and_then(|binding| binding.session_rate_limits.as_ref())
            && let Some(limits) = project_session_rate_limits(&usage.raw, projection)
        {
            if provider.rate_limits.is_none() {
                provider.rate_limits = Some(limits);
            }
            provider.status = "available";
            provider.error = None;
            if let Some(source) = source {
                provider.source = source;
            }
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

fn project_session_rate_limits(raw: &Value, projection: &UsageSessionRateLimits) -> Option<Value> {
    let limits = raw.pointer(&projection.pointer)?;
    if !projection
        .required_number_fields
        .iter()
        .all(|field| limits.get(field).is_some_and(Value::is_number))
        || !projection
            .required_string_fields
            .iter()
            .all(|field| limits.get(field).is_some_and(Value::is_string))
    {
        return None;
    }
    let mut projected = serde_json::Map::new();
    projected.insert(projection.target.clone(), limits.clone());
    Some(Value::Object(projected))
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
        UsageCollectorKind, UsageErrorKind, UsageLimitParserKind, UsageResetClaim,
        UsageSessionOverlay, UsageWidgetKind,
    };

    fn binding(account: &str, projection: Option<UsageSessionRateLimits>) -> PluginUsageSpec {
        PluginUsageSpec {
            account: account.to_owned(),
            collector: UsageCollectorKind::Session,
            reset: None,
            product: None,
            parser: UsageLimitParserKind::new("generic-buckets"),
            error: UsageErrorKind::new("raw"),
            error_auth: None,
            error_config: None,
            error_fetch: None,
            order: None,
            top_bar_windows: Vec::new(),
            widget: UsageWidgetKind::new("none"),
            widget_shape: crate::plugin_host::UsageWidgetShape::None,
            widget_window: None,
            reset_claim: UsageResetClaim::AfterSuccess,
            session_overlay: UsageSessionOverlay::default(),
            session_rate_limits: projection,
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
            collector_sidecars: Vec::new(),
            collector_argv: Vec::new(),
            reset_argv: Vec::new(),
            activity: false,
        }
    }

    #[test]
    fn session_rate_limits_follow_a_generic_json_projection() {
        let projection = UsageSessionRateLimits {
            pointer: "/metadata/limits".to_owned(),
            target: "rateLimits".to_owned(),
            required_number_fields: vec!["utilization".to_owned()],
            required_string_fields: vec!["kind".to_owned()],
        };
        let projected = project_session_rate_limits(
            &json!({"metadata":{"limits":{"utilization":23.5,"kind":"five_hour"}}}),
            &projection,
        )
        .unwrap();
        assert_eq!(projected["rateLimits"]["utilization"], 23.5);
        assert!(
            project_session_rate_limits(
                &json!({"metadata":{"limits":{"utilization":"23.5","kind":"five_hour"}}}),
                &projection,
            )
            .is_none()
        );

        let binding = binding("future-provider", Some(projection));
        assert_eq!(
            binding
                .session_rate_limits
                .as_ref()
                .map(|value| value.pointer.as_str()),
            Some("/metadata/limits")
        );
    }
}
