//! Account-provider information adapters.
//!
//! Agent runtimes are only data sources. Info cards are keyed by the account
//! provider so Codex and Claude Code sessions backed by one DeepSeek key never
//! become duplicate account cards.

use serde_json::{Value, json};

use crate::core::SessionMeta;
use crate::plugin_host::{PluginUsageSpec, UsageSessionRateLimits};
use crate::usage::ProviderUsage;

pub fn overlay_session_usage(
    snapshot: crate::usage::UsageSnapshot,
    sessions: &[SessionMeta],
    catalog: &crate::provider_catalog::ProviderCatalog,
    bindings: &[PluginUsageSpec],
) -> crate::usage::UsageSnapshot {
    overlay_service_sessions(snapshot, sessions, bindings, |session| {
        catalog.account_usage_provider(
            &session.provider,
            &session.provider_version,
            &session.provider_generation_digest,
        )
    })
}

fn overlay_service_sessions(
    mut snapshot: crate::usage::UsageSnapshot,
    sessions: &[SessionMeta],
    bindings: &[PluginUsageSpec],
    provider_for: impl Fn(&SessionMeta) -> Option<String>,
) -> crate::usage::UsageSnapshot {
    for provider in &mut snapshot.providers {
        let Some((_, session, usage)) = sessions
            .iter()
            // The Service collector owns the legacy/default credential. A
            // named account report must never fill its quota or activity.
            .filter(|session| session.provider_account_id.is_default())
            .filter(|session| provider_for(session).as_deref() == Some(provider.provider))
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
        overlay_provider_session(provider, &session.provider, usage, binding);
    }
    snapshot
}

fn overlay_provider_session(
    provider: &mut ProviderUsage,
    agent: &str,
    usage: &crate::agent_model::SessionUsage,
    binding: Option<&PluginUsageSpec>,
) {
    // A live context update does not refresh an account quota observation or
    // clear its refresh error. Keep session-only Providers as a fallback.
    if provider.rate_limits.is_none() {
        // An attempted account collection's error (for example an expired
        // login) is more actionable than the generic empty-state copy. Only
        // a never-collected placeholder takes the empty text.
        let collection_failed = provider.refresh.is_some() && provider.error.is_some();
        if provider.status != "available"
            && !collection_failed
            && let Some(empty) = binding.and_then(|binding| binding.empty.as_deref())
        {
            provider.error = Some(empty.to_owned());
        }
        if let Some(projection) = binding.and_then(|binding| binding.session_rate_limits.as_ref())
            && let Some(limits) = project_session_rate_limits(&usage.raw, projection)
        {
            provider.rate_limits = Some(limits);
            provider.status = "available";
            provider.error = None;
            if let Some(binding) = binding {
                provider.source =
                    crate::usage::intern_usage_str(binding.product_label().to_owned());
            }
        }
        provider.observed_at_ms = provider.observed_at_ms.max(usage.observed_at_ms);
        if provider.status != "available" && provider.status != "exhausted" {
            provider.status = "session-only";
        }
    }
    let latest = json!({ "agent": agent, "session": usage.raw });
    match provider.activity.as_mut().and_then(Value::as_object_mut) {
        Some(activity) => {
            activity.insert("latest_session".to_owned(), latest);
        }
        None => provider.activity = Some(json!({ "latest_session": latest })),
    }
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

pub fn unavailable(provider: &'static str, source: &'static str, message: &str) -> ProviderUsage {
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

pub fn error(provider: &'static str, source: &'static str, message: String) -> ProviderUsage {
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

    #[test]
    fn live_session_preserves_cached_account_quota_freshness_and_error() {
        let binding = binding(
            "future",
            Some(UsageSessionRateLimits {
                pointer: "/limits".to_owned(),
                target: "rateLimits".to_owned(),
                required_number_fields: vec!["utilization".to_owned()],
                required_string_fields: Vec::new(),
            }),
        );
        let usage = crate::agent_model::SessionUsage {
            used: 30,
            size: 100,
            observed_at_ms: 900,
            raw: json!({"limits": {"utilization": 90}}),
        };
        for limits in [json!({"remaining": 50}), json!({"rateLimitsByLimitId": {}})] {
            let mut provider = unavailable("future", "Account", "Cached update");
            provider.status = "available";
            provider.observed_at_ms = 100;
            provider.rate_limits = Some(limits.clone());
            provider.refresh = Some(crate::usage::ProviderRefreshState {
                last_attempt_at_ms: 800,
                manual_refresh_after_ms: 830,
                next_auto_refresh_at_ms: 860,
                stale: true,
            });
            overlay_provider_session(&mut provider, "agent", &usage, Some(&binding));
            assert_eq!(provider.observed_at_ms, 100);
            assert_eq!(provider.rate_limits, Some(limits));
            assert_eq!(provider.error.as_deref(), Some("Cached update"));
            assert!(provider.refresh.unwrap().stale);
            assert_eq!(
                provider.activity.unwrap()["latest_session"]["session"],
                usage.raw
            );
        }
        let mut fallback = unavailable("future", "Account", "No data");
        fallback.observed_at_ms = 0;
        overlay_provider_session(&mut fallback, "agent", &usage, Some(&binding));
        assert_eq!(fallback.status, "available");
        assert_eq!(fallback.observed_at_ms, 900);
        assert!(fallback.error.is_none());
        assert_eq!(
            fallback.rate_limits.unwrap()["rateLimits"]["utilization"],
            90
        );
    }

    #[test]
    fn live_session_keeps_a_failed_collection_error_over_empty_copy() {
        let mut binding = binding("future", None);
        binding.empty = Some("Account quota is not exposed for this session.".to_owned());
        let usage = crate::agent_model::SessionUsage {
            used: 30,
            size: 100,
            observed_at_ms: 900,
            raw: json!({}),
        };
        let mut failed = unavailable("future", "Account", "Sign in again.");
        failed.refresh = Some(crate::usage::ProviderRefreshState {
            last_attempt_at_ms: 800,
            manual_refresh_after_ms: 830,
            next_auto_refresh_at_ms: 860,
            stale: false,
        });
        overlay_provider_session(&mut failed, "agent", &usage, Some(&binding));
        assert_eq!(failed.status, "session-only");
        assert_eq!(failed.error.as_deref(), Some("Sign in again."));

        let mut placeholder = unavailable("future", "Account", "Not refreshed yet");
        overlay_provider_session(&mut placeholder, "agent", &usage, Some(&binding));
        assert_eq!(
            placeholder.error.as_deref(),
            Some("Account quota is not exposed for this session.")
        );
    }

    #[test]
    fn named_session_cannot_fill_default_account_quota_or_activity() {
        let projection = binding(
            "future",
            Some(UsageSessionRateLimits {
                pointer: "/limits".into(),
                target: "rateLimits".into(),
                required_number_fields: vec!["utilization".into()],
                required_string_fields: vec![],
            }),
        );
        let default = crate::core::Hub::prepare_session(crate::core::SessionRegistration {
            id: "default-session".into(),
            provider: "future-agent".into(),
            provider_account_id: Default::default(),
            provider_version: "1.0.0".into(),
            provider_generation_digest: "digest".into(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "machine".into(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/fixture".into(),
            title: String::new(),
            origin: crate::core::SessionOrigin::Web,
            system: false,
            owner_user_id: Some("owner".into()),
            owner_username: None,
        });
        let mut default = default;
        default.usage = Some(crate::agent_model::SessionUsage {
            used: 1,
            size: 100,
            observed_at_ms: 10,
            raw: json!({"limits":{"utilization":20}}),
        });
        let mut named = default.clone();
        named.provider_account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        named.usage.as_mut().unwrap().observed_at_ms = 20;
        named.usage.as_mut().unwrap().raw = json!({"limits":{"utilization":95}});
        let snapshot = crate::usage::UsageSnapshot {
            refreshed_at_ms: 0,
            next_refresh_at_ms: 0,
            refresh_interval_ms: 0,
            providers: vec![unavailable("future", "Fixture", "No data")],
            reset_schedules: Default::default(),
            refreshing: vec![],
        };
        let result = overlay_service_sessions(
            snapshot.clone(),
            &[default, named.clone()],
            std::slice::from_ref(&projection),
            |_| Some("future".into()),
        );
        assert_eq!(
            result.providers[0].rate_limits.as_ref().unwrap()["rateLimits"]["utilization"],
            20
        );
        assert_eq!(
            result.providers[0].activity.as_ref().unwrap()["latest_session"]["session"]["limits"]["utilization"],
            20
        );
        let result =
            overlay_service_sessions(snapshot, &[named], &[projection], |_| Some("future".into()));
        assert!(result.providers[0].rate_limits.is_none());
        assert!(result.providers[0].activity.is_none());
    }
}
