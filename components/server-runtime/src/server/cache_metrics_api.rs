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

use super::*;
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

/// Runtime-resolved prompt-cache telemetry & mitigation capabilities.
///
/// Always derived from live runtime configuration, installed provider behavior,
/// host policy, and observed counters — never from advisory `models/` files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheCapabilitiesView {
    pub telemetry_level: String,
    pub observed_cache_read: bool,
    pub observed_cache_write: bool,
    pub supports_idle_rollover: bool,
    pub supports_gateway_keepalive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollover_restriction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RolloverStrategyView {
    pub supported: bool,
    pub enabled: bool,
    pub inherited: bool,
    pub session_override: Option<bool>,
    pub idle_minutes: Option<u64>,
    pub min_context_tokens: u64,
    pub due: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restriction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeepaliveStrategyView {
    pub supported: bool,
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

fn provider_supports_gateway_keepalive(
    provider: &str,
    behavior: Option<&cowboy_provider_sdk::ProviderBehaviorContract>,
) -> bool {
    let configuration = behavior.map_or_else(
        || crate::provider::legacy_behavior(provider).configuration,
        |value| value.configuration.clone(),
    );
    crate::managed_config::cache_protection(&configuration, &serde_json::json!({})).is_some()
}

fn resolve_capabilities(
    provider: &str,
    behavior: Option<&cowboy_provider_sdk::ProviderBehaviorContract>,
    totals: &CacheMetricsTotals,
) -> CacheCapabilitiesView {
    let named_only = crate::provider_account::named_only(provider);
    CacheCapabilitiesView {
        telemetry_level: "turn_end".to_owned(),
        observed_cache_read: totals.observed_cache_turns > 0,
        observed_cache_write: totals.observed_cache_write_turns > 0,
        supports_idle_rollover: !named_only,
        supports_gateway_keepalive: provider_supports_gateway_keepalive(provider, behavior),
        rollover_restriction: named_only.then(|| ANTIGRAVITY_ROLLOVER_RESTRICTION.to_owned()),
    }
}

fn effective_strategy_mode(rollover_enabled: bool, keepalive_enabled: bool) -> &'static str {
    match (rollover_enabled, keepalive_enabled) {
        (true, true) => "rollover_and_keepalive",
        (true, false) => "rollover",
        (false, true) => "keepalive",
        (false, false) => "none",
    }
}

async fn load_authorized_account_cache_metrics(
    store: &Store,
    owner_user_id: &str,
    account_id: &str,
    query: &AccountCacheMetricsQuery,
    now_ms: i64,
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
    let capabilities = resolve_capabilities(&account.provider_id, None, &metrics.totals);
    Ok(AccountCacheMetricsResponse {
        account_id: account.id.as_str().to_owned(),
        account_label: account.label.clone(),
        metrics,
        capabilities,
    })
}

async fn load_authorized_session_cache_metrics(
    hub: &Hub,
    store: &Store,
    principal: &ProductPrincipal,
    session_id: &str,
    query: &SessionCacheMetricsQuery,
    rollover_policy: crate::context_rollover::Policy,
    due: bool,
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
            &meta.provider,
            meta.provider_account_id.as_str(),
            limit,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, session = %session_id, "session cache metrics query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        })?;
    let capabilities = resolve_capabilities(
        &meta.provider,
        meta.provider_behavior.as_ref(),
        &metrics.totals,
    );

    let configuration = meta.provider_behavior.as_ref().map_or_else(
        || crate::provider::legacy_behavior(&meta.provider).configuration,
        |behavior| behavior.configuration.clone(),
    );
    let preferences = hub
        .config_preferences(&meta.id)
        .unwrap_or_else(|| serde_json::json!({}));
    let keepalive_enabled = crate::managed_config::cache_protection(&configuration, &preferences);

    let strategy = SessionCacheStrategyView {
        effective_mode: effective_strategy_mode(
            rollover_policy.enabled,
            keepalive_enabled == Some(true),
        )
        .to_owned(),
        rollover: RolloverStrategyView {
            supported: capabilities.supports_idle_rollover,
            enabled: rollover_policy.enabled,
            inherited: rollover_policy.session_override.is_none(),
            session_override: rollover_policy.session_override,
            idle_minutes: rollover_policy.idle_minutes,
            min_context_tokens: rollover_policy.min_context_tokens,
            due,
            restriction: capabilities.rollover_restriction.clone(),
        },
        gateway_keepalive: KeepaliveStrategyView {
            supported: keepalive_enabled.is_some(),
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
    let now_ms = chrono::Utc::now().timestamp_millis();
    match load_authorized_account_cache_metrics(
        store,
        &auth.principal.user_id,
        &account_id,
        &query,
        now_ms,
    )
    .await
    {
        Ok(payload) => Json(payload).into_response(),
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
    let rollover_policy = super::session_context::rollover::effective_policy(&state, &meta)
        .await
        .unwrap_or_else(|| crate::context_rollover::policy(None, None, &meta.provider));
    let idle_ms = state
        .hub
        .session_idle_for(&id)
        .map(|idle| u64::try_from(idle.as_millis()).unwrap_or(u64::MAX));
    let due = !meta.context_usage_stale()
        && idle_ms.is_some_and(|idle| {
            rollover_policy.due(std::time::Duration::from_millis(idle), meta.context_used)
        })
        && super::session_context::rollover::is_ordinary_session(&state, &meta);

    match load_authorized_session_cache_metrics(
        &state.hub,
        store,
        &auth.principal,
        &id,
        &query,
        rollover_policy,
        due,
    )
    .await
    {
        Ok(payload) => Json(payload).into_response(),
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
    fn route_classification_protects_account_and_session_cache_metrics() {
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
    fn resolve_capabilities_enforces_antigravity_boundary_and_runtime_keepalive() {
        let empty = CacheMetricsTotals::default();
        let agy = resolve_capabilities("antigravity", None, &empty);
        assert!(!agy.supports_idle_rollover);
        assert!(!agy.supports_gateway_keepalive);
        assert!(agy.rollover_restriction.is_some());
        assert!(!agy.observed_cache_read);

        let claude = resolve_capabilities(
            "claude-code",
            None,
            &CacheMetricsTotals {
                observed_cache_turns: 3,
                observed_cache_write_turns: 3,
                ..Default::default()
            },
        );
        assert!(claude.supports_idle_rollover);
        assert!(!claude.supports_gateway_keepalive);
        assert!(claude.observed_cache_read);
        assert!(claude.observed_cache_write);
        assert!(claude.rollover_restriction.is_none());

        assert_eq!(effective_strategy_mode(false, false), "none");
        assert_eq!(effective_strategy_mode(true, false), "rollover");
        assert_eq!(effective_strategy_mode(false, true), "keepalive");
        assert_eq!(
            effective_strategy_mode(true, true),
            "rollover_and_keepalive"
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn account_and_session_cache_metrics_enforce_owner_and_visibility_isolation() {
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

        // Cross-owner account ID must return 404.
        let cross =
            load_authorized_account_cache_metrics(&store, owner_a, acct_b.as_str(), &query, now)
                .await;
        assert_eq!(cross.unwrap_err().0, StatusCode::NOT_FOUND);

        // Literal "default" must never resolve on /api/provider-accounts/{account_id}/cache-metrics.
        let default_err =
            load_authorized_account_cache_metrics(&store, owner_a, "default", &query, now).await;
        assert_eq!(default_err.unwrap_err().0, StatusCode::NOT_FOUND);

        // Owned account returns exact isolated metrics and runtime capabilities.
        let ok =
            load_authorized_account_cache_metrics(&store, owner_a, acct_a.as_str(), &query, now)
                .await
                .expect("owned account metrics");
        assert_eq!(ok.account_id, acct_a.as_str());
        assert_eq!(ok.metrics.provider, "claude-code");
        assert_eq!(ok.metrics.totals.total_turns, 1);
        assert_eq!(ok.metrics.totals.cached_read_tokens, Some(90_000));
        assert_eq!(ok.metrics.totals.cache_write_tokens, Some(9_000));
        assert_eq!(ok.metrics.totals.other_uncached_input_tokens, Some(1_000));
        assert!(ok.capabilities.supports_idle_rollover);

        // Session visibility isolation: non-owner principal for Bob cannot see Alice's session.
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
        let rollover_policy = crate::context_rollover::policy(None, None, "claude-code");
        let denied = load_authorized_session_cache_metrics(
            &hub,
            &store,
            &principal_b,
            "sess-a",
            &SessionCacheMetricsQuery::default(),
            rollover_policy.clone(),
            false,
        )
        .await;
        assert_eq!(denied.unwrap_err().0, StatusCode::NOT_FOUND);

        let allowed = load_authorized_session_cache_metrics(
            &hub,
            &store,
            &principal_a,
            "sess-a",
            &SessionCacheMetricsQuery::default(),
            rollover_policy,
            false,
        )
        .await
        .expect("visible session metrics");
        assert_eq!(allowed.metrics.totals.total_turns, 1);
        assert_eq!(allowed.strategy.effective_mode, "rollover");
        assert!(allowed.strategy.rollover.inherited);
        assert_eq!(allowed.strategy.rollover.idle_minutes, Some(60));
    }
}
