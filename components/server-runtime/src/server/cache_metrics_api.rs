//! Authorized read endpoints for indexed turn-level prompt-cache metrics.
//!
//! - `GET /api/provider-accounts/{account_id}/cache-metrics`: resolves
//!   `account_id` strictly within the authenticated caller's own
//!   `provider_accounts` registry so `(owner_user_id, provider, account_id)`
//!   can never leak across owners or providers and never matches `"default"`.
//! - `GET /api/sessions/{id}/cache-metrics`: enforces `session_is_visible` and
//!   combines persisted `turn_cache_facts` (surviving transcript `Clear`) with
//!   live read-only effective vs inherited cache strategy and runtime-resolved
//!   capabilities.
//!
//! Both endpoints emit `Cache-Control: no-store` so private cookie-authenticated
//! responses are never reused across sign-out or permission changes.

use super::*;
use crate::machine_protocol::{PluginInstallationState, PluginInventory};
use crate::provider_account::ProviderAccountId;
use crate::store::turn_cache_facts::{
    AccountCacheMetrics, CacheMetricsTotals, DEFAULT_RETENTION_DAYS, SessionCacheMetrics,
};
use cowboy_server_auth::server::{AuthenticatedProductRequest, session_is_visible};

const MS_PER_HOUR: i64 = 3_600_000;
const MS_PER_DAY: i64 = 86_400_000;
const MAX_CUSTOM_SPAN_MS: i64 = DEFAULT_RETENTION_DAYS * MS_PER_DAY;
const DEFAULT_RECENT_TURN_LIMIT: usize = 30;
const MAX_RECENT_TURN_LIMIT: usize = 100;
const ANTIGRAVITY_ROLLOVER_RESTRICTION: &str =
    "Automatic context renewal is disabled for Antigravity accounts by host policy.";

#[derive(Debug, Default, Deserialize)]
pub struct AccountCacheMetricsQuery {
    pub window: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SessionCacheMetricsQuery {
    pub limit: Option<usize>,
}

/// Verified keepalive capability resolution across signed installed generations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeepaliveCapability {
    Verified(bool),
    Mixed,
    Unverified,
}

impl KeepaliveCapability {
    const fn supports(self) -> Option<bool> {
        match self {
            Self::Verified(value) => Some(value),
            Self::Mixed | Self::Unverified => None,
        }
    }

    const fn status(self) -> &'static str {
        match self {
            Self::Verified(_) => "verified",
            Self::Mixed => "mixed",
            Self::Unverified => "unverified",
        }
    }
}

/// Runtime-resolved prompt-cache telemetry & mitigation capabilities.
///
/// Always derived from installed signed plugin manifests, session behavior,
/// host policy, and observed counters — never from advisory `models/` files or
/// bundled fallback defaults when installed evidence is absent or mixed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheCapabilitiesView {
    pub telemetry_level: String,
    pub observed_cache_read: bool,
    pub observed_cache_write: bool,
    pub supports_idle_rollover: bool,
    pub supports_gateway_keepalive: Option<bool>,
    pub keepalive_capability_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollover_restriction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RolloverStrategyView {
    pub availability: String,
    pub supported: bool,
    pub enabled: Option<bool>,
    pub inherited: Option<bool>,
    pub session_override: Option<bool>,
    pub idle_minutes: Option<u64>,
    pub min_context_tokens: Option<u64>,
    pub due: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restriction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeepaliveStrategyView {
    pub supported: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCacheStrategyView {
    pub effective_mode: String,
    pub rollover: RolloverStrategyView,
    pub gateway_keepalive: KeepaliveStrategyView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountCacheMetricsResponse {
    pub account_id: String,
    pub account_label: String,
    #[serde(flatten)]
    pub metrics: AccountCacheMetrics,
    pub capabilities: CacheCapabilitiesView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCacheMetricsResponse {
    #[serde(flatten)]
    pub metrics: SessionCacheMetrics,
    pub capabilities: CacheCapabilitiesView,
    pub strategy: SessionCacheStrategyView,
}

fn no_store_json<T: Serialize>(payload: T) -> Response {
    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store")],
        Json(payload),
    )
        .into_response()
}

fn parse_window(
    query: &AccountCacheMetricsQuery,
    now_ms: i64,
) -> Result<(String, i64, i64), &'static str> {
    if query.from_ms.is_some() || query.to_ms.is_some() {
        let to_ms = query.to_ms.unwrap_or(now_ms);
        let from_ms = query
            .from_ms
            .ok_or("from_ms is required when to_ms is provided")?;
        if from_ms < 0 || to_ms < 0 || from_ms > to_ms {
            return Err("invalid time range: require 0 <= from_ms <= to_ms");
        }
        if to_ms.saturating_sub(from_ms) > MAX_CUSTOM_SPAN_MS {
            return Err("requested time range exceeds 90-day retention");
        }
        return Ok(("custom".to_owned(), from_ms, to_ms));
    }
    let label = query.window.as_deref().unwrap_or("7d").trim();
    let span_ms = match label {
        "24h" => 24 * MS_PER_HOUR,
        "7d" => 7 * MS_PER_DAY,
        "30d" => 30 * MS_PER_DAY,
        _ => return Err("unsupported window: expected 24h, 7d, or 30d"),
    };
    Ok((label.to_owned(), now_ms.saturating_sub(span_ms), now_ms))
}

fn combine_keepalive_observations(observations: &[Option<bool>]) -> KeepaliveCapability {
    if observations.is_empty() {
        return KeepaliveCapability::Unverified;
    }
    let mut saw_true = false;
    let mut saw_false = false;
    let mut saw_unknown = false;
    for obs in observations {
        match obs {
            Some(true) => saw_true = true,
            Some(false) => saw_false = true,
            None => saw_unknown = true,
        }
    }
    if (saw_unknown && (saw_true || saw_false)) || (saw_true && saw_false) {
        KeepaliveCapability::Mixed
    } else if saw_true {
        KeepaliveCapability::Verified(true)
    } else if saw_false {
        KeepaliveCapability::Verified(false)
    } else {
        KeepaliveCapability::Unverified
    }
}

async fn resolve_account_keepalive_capability(
    state: &AppState,
    store: &Store,
    provider_id: &str,
) -> KeepaliveCapability {
    let Ok(machines) = store.list_machines().await else {
        return KeepaliveCapability::Unverified;
    };
    let mut observations = Vec::new();
    for machine in machines.iter().filter(|m| !m.revoked) {
        let Some(raw_plugins) = machine.inventory.get("plugins") else {
            continue;
        };
        let Ok(plugins) = serde_json::from_value::<Vec<PluginInventory>>(raw_plugins.clone())
        else {
            continue;
        };
        for plugin in plugins
            .into_iter()
            .filter(|p| p.plugin_id == provider_id && p.state == PluginInstallationState::Active)
        {
            let obs = state
                .provider_catalog
                .package(
                    provider_id,
                    &plugin.plugin_version,
                    &plugin.generation_digest,
                )
                .map(|pkg| {
                    crate::managed_config::cache_protection(
                        &pkg.manifest.runtime.behavior.configuration,
                        &serde_json::json!({}),
                    )
                    .is_some()
                });
            observations.push(obs);
        }
    }
    combine_keepalive_observations(&observations)
}

fn resolve_session_configuration(
    provider_catalog: &crate::provider_catalog::ProviderCatalog,
    meta: &crate::core::SessionMeta,
) -> Option<cowboy_provider_sdk::ConfigurationBehavior> {
    if let Some(behavior) = meta.provider_behavior.as_ref() {
        return Some(behavior.configuration.clone());
    }
    provider_catalog
        .package(
            &meta.provider,
            &meta.provider_version,
            &meta.provider_generation_digest,
        )
        .map(|pkg| pkg.manifest.runtime.behavior.configuration)
}

fn resolve_capabilities(
    provider: &str,
    keepalive: KeepaliveCapability,
    totals: &CacheMetricsTotals,
) -> CacheCapabilitiesView {
    let named_only = crate::provider_account::named_only(provider);
    CacheCapabilitiesView {
        telemetry_level: "turn_end".to_owned(),
        observed_cache_read: totals.observed_cache_turns > 0,
        observed_cache_write: totals.observed_cache_write_turns > 0,
        supports_idle_rollover: !named_only,
        supports_gateway_keepalive: keepalive.supports(),
        keepalive_capability_status: keepalive.status().to_owned(),
        rollover_restriction: named_only.then(|| ANTIGRAVITY_ROLLOVER_RESTRICTION.to_owned()),
    }
}

fn effective_strategy_mode(
    rollover_enabled: Option<bool>,
    keepalive_enabled: Option<bool>,
) -> &'static str {
    match (rollover_enabled, keepalive_enabled) {
        (Some(true), Some(true)) => "rollover_and_keepalive",
        (Some(true), _) => "rollover",
        (Some(false), Some(true)) => "keepalive",
        (Some(false), _) => "none",
        (None, Some(true)) => "keepalive_rollover_unknown",
        (None, _) => "unavailable",
    }
}

async fn load_authorized_account_cache_metrics(
    store: &Store,
    owner_user_id: &str,
    account_id: &str,
    query: &AccountCacheMetricsQuery,
    now_ms: i64,
    keepalive: KeepaliveCapability,
) -> Result<AccountCacheMetricsResponse, (StatusCode, &'static str)> {
    let Ok(parsed_id) = account_id.parse::<ProviderAccountId>() else {
        return Err((StatusCode::NOT_FOUND, "unknown provider account"));
    };
    if parsed_id.is_default() {
        return Err((StatusCode::NOT_FOUND, "unknown provider account"));
    }
    let registry = store
        .provider_accounts(owner_user_id)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "failed reading provider accounts for cache metrics");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        })?;
    let Some(account) = registry.accounts.iter().find(|entry| entry.id == parsed_id) else {
        return Err((StatusCode::NOT_FOUND, "unknown provider account"));
    };
    let (window, from_ms, to_ms) =
        parse_window(query, now_ms).map_err(|msg| (StatusCode::BAD_REQUEST, msg))?;
    let metrics = store
        .account_cache_metrics(
            owner_user_id,
            &account.provider_id,
            account.id.as_str(),
            from_ms,
            to_ms,
            &window,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, account = %account.id.as_str(), "account cache metrics query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        })?;
    let capabilities = resolve_capabilities(&account.provider_id, keepalive, &metrics.totals);
    Ok(AccountCacheMetricsResponse {
        account_id: account.id.as_str().to_owned(),
        account_label: account.label.clone(),
        metrics,
        capabilities,
    })
}

#[allow(clippy::too_many_arguments)]
async fn load_authorized_session_cache_metrics(
    hub: &Hub,
    store: &Store,
    principal: &ProductPrincipal,
    session_id: &str,
    query: &SessionCacheMetricsQuery,
    rollover_policy: Option<crate::context_rollover::Policy>,
    due: bool,
    configuration: Option<&cowboy_provider_sdk::ConfigurationBehavior>,
) -> Result<SessionCacheMetricsResponse, (StatusCode, &'static str)> {
    if !session_is_visible(hub, principal, session_id) {
        return Err((StatusCode::NOT_FOUND, "unknown session"));
    }
    let Some(meta) = hub.session_info(session_id).map(|info| info.meta) else {
        return Err((StatusCode::NOT_FOUND, "unknown session"));
    };
    let limit = query
        .limit
        .unwrap_or(DEFAULT_RECENT_TURN_LIMIT)
        .clamp(1, MAX_RECENT_TURN_LIMIT);
    let metrics = store
        .session_cache_metrics(
            session_id,
            meta.owner_user_id.as_deref(),
            &meta.provider,
            meta.provider_account_id.as_str(),
            limit,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, session = %session_id, "session cache metrics query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        })?;

    let preferences = hub
        .config_preferences(&meta.id)
        .unwrap_or_else(|| serde_json::json!({}));
    let (keepalive_cap, keepalive_enabled) = match configuration {
        Some(cfg) => {
            let enabled = crate::managed_config::cache_protection(cfg, &preferences);
            (KeepaliveCapability::Verified(enabled.is_some()), enabled)
        }
        None => (KeepaliveCapability::Unverified, None),
    };
    let capabilities = resolve_capabilities(&meta.provider, keepalive_cap, &metrics.totals);

    let (rollover_view, rollover_enabled) = match rollover_policy {
        Some(policy) => (
            RolloverStrategyView {
                availability: "available".to_owned(),
                supported: capabilities.supports_idle_rollover,
                enabled: Some(policy.enabled),
                inherited: Some(policy.session_override.is_none()),
                session_override: policy.session_override,
                idle_minutes: policy.idle_minutes,
                min_context_tokens: Some(policy.min_context_tokens),
                due,
                restriction: capabilities.rollover_restriction.clone(),
            },
            Some(policy.enabled),
        ),
        None => (
            RolloverStrategyView {
                availability: "unavailable".to_owned(),
                supported: capabilities.supports_idle_rollover,
                enabled: None,
                inherited: None,
                session_override: None,
                idle_minutes: None,
                min_context_tokens: None,
                due: false,
                restriction: capabilities.rollover_restriction.clone(),
            },
            None,
        ),
    };

    let strategy = SessionCacheStrategyView {
        effective_mode: effective_strategy_mode(rollover_enabled, keepalive_enabled).to_owned(),
        rollover: rollover_view,
        gateway_keepalive: KeepaliveStrategyView {
            supported: keepalive_cap.supports(),
            enabled: keepalive_enabled,
        },
    };

    Ok(SessionCacheMetricsResponse {
        metrics,
        capabilities,
        strategy,
    })
}

/// `GET /api/provider-accounts/{account_id}/cache-metrics`
pub async fn account_cache_metrics(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account_id): Path<String>,
    Query(query): Query<AccountCacheMetricsQuery>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "store unavailable").into_response();
    };
    let Ok(parsed_id) = account_id.parse::<ProviderAccountId>() else {
        return (StatusCode::NOT_FOUND, "unknown provider account").into_response();
    };
    if parsed_id.is_default() {
        return (StatusCode::NOT_FOUND, "unknown provider account").into_response();
    }
    let provider_id = match store.provider_accounts(&auth.principal.user_id).await {
        Ok(registry) => match registry.accounts.iter().find(|entry| entry.id == parsed_id) {
            Some(account) => account.provider_id.clone(),
            None => return (StatusCode::NOT_FOUND, "unknown provider account").into_response(),
        },
        Err(error) => {
            tracing::warn!(%error, "failed reading provider accounts for cache metrics");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };
    let keepalive = resolve_account_keepalive_capability(&state, store, &provider_id).await;
    let now_ms = chrono::Utc::now().timestamp_millis();
    match load_authorized_account_cache_metrics(
        store,
        &auth.principal.user_id,
        &account_id,
        &query,
        now_ms,
        keepalive,
    )
    .await
    {
        Ok(payload) => no_store_json(payload),
        Err(error) => error.into_response(),
    }
}

/// `GET /api/sessions/{id}/cache-metrics`
pub async fn session_cache_metrics(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
    Query(query): Query<SessionCacheMetricsQuery>,
) -> Response {
    if !session_is_visible(&state.hub, &auth.principal, &id) {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let Some(meta) = state.hub.session_info(&id).map(|info| info.meta) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "store unavailable").into_response();
    };
    // Preserve `None` on preference read failure instead of fabricating an
    // enabled/default policy that disagrees with runtime dispatch.
    let rollover_policy = super::session_context::rollover::effective_policy(&state, &meta).await;
    let idle_ms = state
        .hub
        .session_idle_for(&id)
        .map(|idle| u64::try_from(idle.as_millis()).unwrap_or(u64::MAX));
    let due = rollover_policy.as_ref().is_some_and(|policy| {
        !meta.context_usage_stale()
            && idle_ms.is_some_and(|idle| {
                policy.due(std::time::Duration::from_millis(idle), meta.context_used)
            })
            && super::session_context::rollover::is_ordinary_session(&state, &meta)
    });
    let configuration = resolve_session_configuration(&state.provider_catalog, &meta);

    match load_authorized_session_cache_metrics(
        &state.hub,
        store,
        &auth.principal,
        &id,
        &query,
        rollover_policy,
        due,
        configuration.as_ref(),
    )
    .await
    {
        Ok(payload) => no_store_json(payload),
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::AdminRole;
    use crate::agent_model::TurnUsage;
    use crate::core::SessionRegistration;
    use crate::store::ProductUser;
    use crate::store::provider_accounts::{Change, Mutation, MutationResult};
    use cowboy_server_auth::server::{RouteAuth, classify_route};

    fn test_user(id: &str, username: &str) -> ProductUser {
        ProductUser {
            id: id.to_owned(),
            username: username.to_owned(),
            password_algo: "argon2id".to_owned(),
            password_hash: "hash".to_owned(),
            created_at_ms: 1_000,
            updated_at_ms: 1_000,
            disabled_at_ms: None,
        }
    }

    #[test]
    fn route_classification_and_no_store_header_protect_private_cache_metrics() {
        assert_eq!(
            classify_route(
                &Method::GET,
                "/api/provider-accounts/11111111-1111-1111-1111-111111111111/cache-metrics"
            ),
            RouteAuth::ProductOperator
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/sessions/sess-1/cache-metrics"),
            RouteAuth::ProductSessionSee
        );
        let response = no_store_json(serde_json::json!({"ok": true}));
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
    }

    #[test]
    fn parse_window_validates_presets_and_bounded_custom_ranges() {
        let now = 1_700_000_000_000_i64;
        let (label, from, to) = parse_window(&AccountCacheMetricsQuery::default(), now).unwrap();
        assert_eq!(label, "7d");
        assert_eq!(to - from, 7 * MS_PER_DAY);

        let (label, from, to) = parse_window(
            &AccountCacheMetricsQuery {
                window: Some("24h".into()),
                ..Default::default()
            },
            now,
        )
        .unwrap();
        assert_eq!(label, "24h");
        assert_eq!(to - from, 24 * MS_PER_HOUR);

        assert!(
            parse_window(
                &AccountCacheMetricsQuery {
                    window: Some("999d".into()),
                    ..Default::default()
                },
                now,
            )
            .is_err()
        );
        assert!(
            parse_window(
                &AccountCacheMetricsQuery {
                    window: None,
                    from_ms: Some(now - (DEFAULT_RETENTION_DAYS + 1) * MS_PER_DAY),
                    to_ms: Some(now),
                },
                now,
            )
            .is_err()
        );
        assert!(
            parse_window(
                &AccountCacheMetricsQuery {
                    window: None,
                    from_ms: Some(now + 100),
                    to_ms: Some(now),
                },
                now,
            )
            .is_err()
        );
    }

    #[test]
    fn resolve_capabilities_and_keepalive_combination_represent_verified_mixed_and_unverified() {
        assert_eq!(
            combine_keepalive_observations(&[]),
            KeepaliveCapability::Unverified
        );
        assert_eq!(
            combine_keepalive_observations(&[Some(true), Some(true)]),
            KeepaliveCapability::Verified(true)
        );
        assert_eq!(
            combine_keepalive_observations(&[Some(true), Some(false)]),
            KeepaliveCapability::Mixed
        );
        assert_eq!(
            combine_keepalive_observations(&[Some(false), None]),
            KeepaliveCapability::Mixed
        );

        let empty = CacheMetricsTotals::default();
        let agy = resolve_capabilities("antigravity", KeepaliveCapability::Verified(false), &empty);
        assert!(!agy.supports_idle_rollover);
        assert_eq!(agy.supports_gateway_keepalive, Some(false));
        assert_eq!(agy.keepalive_capability_status, "verified");
        assert!(agy.rollover_restriction.is_some());

        let unverified =
            resolve_capabilities("claude-deepseek", KeepaliveCapability::Unverified, &empty);
        assert_eq!(unverified.supports_gateway_keepalive, None);
        assert_eq!(unverified.keepalive_capability_status, "unverified");

        assert_eq!(effective_strategy_mode(None, None), "unavailable");
        assert_eq!(
            effective_strategy_mode(None, Some(true)),
            "keepalive_rollover_unknown"
        );
        assert_eq!(effective_strategy_mode(Some(false), Some(false)), "none");
        assert_eq!(effective_strategy_mode(Some(true), Some(false)), "rollover");
        assert_eq!(
            effective_strategy_mode(Some(false), Some(true)),
            "keepalive"
        );
        assert_eq!(
            effective_strategy_mode(Some(true), Some(true)),
            "rollover_and_keepalive"
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn account_and_session_cache_metrics_enforce_owner_visibility_and_unavailable_policy() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();

        let owner_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let owner_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        store
            .insert_user(&test_user(owner_a, "alice"))
            .await
            .unwrap();
        store.insert_user(&test_user(owner_b, "bob")).await.unwrap();

        let MutationResult::Applied(reg_a) = store
            .mutate_provider_accounts(
                owner_a,
                "create-acct-a",
                &Mutation {
                    expected_revision: 0,
                    change: Change::Create {
                        provider_id: "claude-code".into(),
                        label: "Alice Claude".into(),
                    },
                },
            )
            .await
            .unwrap()
        else {
            panic!("expected owner_a account creation");
        };
        let acct_a = reg_a.registry.accounts[0].id.clone();

        let MutationResult::Applied(reg_b) = store
            .mutate_provider_accounts(
                owner_b,
                "create-acct-b",
                &Mutation {
                    expected_revision: 0,
                    change: Change::Create {
                        provider_id: "antigravity".into(),
                        label: "Bob AGY".into(),
                    },
                },
            )
            .await
            .unwrap()
        else {
            panic!("expected owner_b account creation");
        };
        let acct_b = reg_b.registry.accounts[0].id.clone();

        let hub = Hub::new();
        hub.create_session(SessionRegistration {
            id: "sess-a".into(),
            provider: "claude-code".into(),
            provider_account_id: acct_a.clone(),
            provider_version: "1.0.0".into(),
            provider_generation_digest: "gen-a".into(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".into(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp/a".into(),
            title: "Alice session".into(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: Some(owner_a.into()),
            owner_username: Some("alice".into()),
        });
        let meta_a = hub.session_info("sess-a").unwrap().meta;
        store.insert_session(&meta_a).await.unwrap();
        store
            .upsert_event_batch(
                &[Envelope {
                    session_id: "sess-a".into(),
                    seq: 1,
                    event: Event::TurnEnd {
                        stop_reason: "EndTurn".into(),
                        usage: Some(TurnUsage {
                            total_tokens: 105_000,
                            input_tokens: 100_000,
                            output_tokens: 5_000,
                            cached_input_tokens: Some(90_000),
                            uncached_input_tokens: Some(10_000),
                            cache_write_tokens: Some(9_000),
                            reasoning_tokens: None,
                        }),
                    },
                    cmid: None,
                }],
                &HashMap::from([("sess-a".into(), 2)]),
            )
            .await
            .unwrap();

        let now = chrono::Utc::now().timestamp_millis() + 60_000;
        let query = AccountCacheMetricsQuery {
            window: Some("24h".into()),
            ..Default::default()
        };

        let cross = load_authorized_account_cache_metrics(
            &store,
            owner_a,
            acct_b.as_str(),
            &query,
            now,
            KeepaliveCapability::Verified(false),
        )
        .await;
        assert_eq!(cross.unwrap_err().0, StatusCode::NOT_FOUND);

        let default_err = load_authorized_account_cache_metrics(
            &store,
            owner_a,
            "default",
            &query,
            now,
            KeepaliveCapability::Unverified,
        )
        .await;
        assert_eq!(default_err.unwrap_err().0, StatusCode::NOT_FOUND);

        let ok = load_authorized_account_cache_metrics(
            &store,
            owner_a,
            acct_a.as_str(),
            &query,
            now,
            KeepaliveCapability::Verified(false),
        )
        .await
        .expect("owned account metrics");
        assert_eq!(ok.account_id, acct_a.as_str());
        assert_eq!(ok.metrics.totals.cached_read_tokens, Some(90_000));
        assert_eq!(ok.capabilities.supports_gateway_keepalive, Some(false));

        let principal_a = ProductPrincipal {
            user_id: owner_a.into(),
            username: "alice".into(),
            role: AdminRole::Operator,
        };
        let principal_b = ProductPrincipal {
            user_id: owner_b.into(),
            username: "bob".into(),
            role: AdminRole::Operator,
        };

        let denied = load_authorized_session_cache_metrics(
            &hub,
            &store,
            &principal_b,
            "sess-a",
            &SessionCacheMetricsQuery::default(),
            None,
            false,
            None,
        )
        .await;
        assert_eq!(denied.unwrap_err().0, StatusCode::NOT_FOUND);

        // When preference read fails (None), strategy MUST report unavailable and never enabled/due.
        let unavailable = load_authorized_session_cache_metrics(
            &hub,
            &store,
            &principal_a,
            "sess-a",
            &SessionCacheMetricsQuery::default(),
            None,
            false,
            None,
        )
        .await
        .expect("session metrics with unavailable rollover policy");
        assert_eq!(unavailable.strategy.effective_mode, "unavailable");
        assert_eq!(unavailable.strategy.rollover.availability, "unavailable");
        assert_eq!(unavailable.strategy.rollover.enabled, None);
        assert!(!unavailable.strategy.rollover.due);
        assert_eq!(
            unavailable.capabilities.keepalive_capability_status,
            "unverified"
        );
    }
}
