//! The Controller HTTP route table, grouped by domain.
//!
//! Sub-state route tables live in `cowboy-server-accounts`, `cowboy-server-code`,
//! and `cowboy-server-plugins` so their Axum handler monomorphization runs off
//! the critical path; `cowboy-server-routes` compiles the remaining `AppState`
//! routes once (`test = false`) in parallel with `cowboy-server-agent-api`.

use cowboy::{machine_lanes as root_machine_lanes, otlp, plugin_process};
use cowboy_server_runtime::server::*;

/// Health, metrics, telemetry, diagnostics, preferences and notification routes.
pub fn platform_routes(state: &AppState) -> Router<Arc<AppState>> {
    let telemetry = Router::new()
        .merge(telemetry_binding::resolution::surface::routes())
        .merge(telemetry_binding::recovery::surface::routes())
        .merge(telemetry_binding::surface::routes())
        .with_state(state.telemetry_api_state());
    Router::new()
        .merge(cardea_authorization::routes())
        .merge(peer_notices::routes())
        .merge(telemetry)
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/api/metrics", get(api_metrics))
        .route("/api/sync/dataset", get(sync_dataset::get_dataset))
        .route("/api/preferences/{scope}", get(preferences::read))
        .route("/api/superagent/activity", get(superagent::activity))
        .route(
            "/api/preferences",
            put(preferences::update).layer(DefaultBodyLimit::max(1024 * 1024)),
        )
        .route(
            "/api/observability/batches",
            post(api_observability_batch).layer(DefaultBodyLimit::max(256 * 1024)),
        )
        .route(
            "/api/observability/incidents",
            get(api_observability_incidents),
        )
        .route(
            "/api/telemetry/v1/{signal}",
            post(api_otlp).layer(DefaultBodyLimit::max(otlp::MAX_BYTES)),
        )
        .route("/api/logs", get(api_diagnostic_logs))
        .route("/api/logs/{id}", get(api_diagnostic_log_detail))
        .route("/metrics", get(prometheus_metrics))
        .route("/api/web-push/config", get(api_web_push_config))
        .route(
            "/api/notifications/presence",
            put(notifications::renew_presence).delete(notifications::release_presence),
        )
        .route(
            "/api/web-push/subscription",
            put(notifications::subscribe).delete(notifications::unsubscribe),
        )
}

/// Provider account, usage, Plugin and Provider authentication routes.
pub fn provider_routes(state: &AppState) -> Router<Arc<AppState>> {
    Router::new()
        .merge(
            cowboy_server_accounts::server::routes::accounts_routes()
                .with_state(Arc::clone(&state.accounts)),
        )
        .route(
            "/api/provider-accounts/{account_id}/cache-metrics",
            get(cache_metrics_api::account_cache_metrics),
        )
        .route("/api/usage", get(api_usage).post(api_usage_refresh))
        .route("/api/usage/executors", get(api_usage_executors))
        .route(
            "/api/usage/{provider}/executor",
            axum::routing::put(api_usage_executor_update),
        )
        .route("/api/usage/{provider}/activity", get(api_usage_activity))
        .route("/api/usage/{provider}", post(api_usage_provider_refresh))
        .route("/api/usage/logs", get(api_usage_logs))
        .route("/api/usage/{provider}/reset", post(api_provider_reset))
        .route(
            "/api/usage/{provider}/reset/schedule",
            put(api_provider_reset_schedule).delete(api_provider_reset_cancel),
        )
        .route("/api/providers", get(api_providers))
        .route("/api/plugins", get(api_plugins))
        .route(
            "/api/plugins/catalog/refresh",
            post(api_plugin_catalog_refresh),
        )
        .route(
            "/api/plugins/{id}/call",
            post(api_plugin_call).layer(DefaultBodyLimit::max(
                plugin_process::MAX_PLUGIN_COMMAND_INPUT_BYTES,
            )),
        )
        .route(
            "/api/providers/catalog/refresh",
            post(api_provider_catalog_refresh),
        )
        .route(
            "/api/providers/{id}/auth",
            post(api_provider_auth_commit).delete(api_provider_auth_logout),
        )
        .route(
            "/api/providers/{id}/auth/start",
            post(api_provider_auth_start),
        )
        .route(
            "/api/providers/{id}/auth/{request_id}",
            get(api_provider_auth_events)
                .post(api_provider_auth_submit)
                .delete(api_provider_auth_cancel),
        )
        .route(
            "/api/plugins/{id}/auth",
            post(api_provider_auth_commit).delete(api_provider_auth_logout),
        )
        .route(
            "/api/plugins/{id}/auth/start",
            post(api_provider_auth_start),
        )
        .route(
            "/api/plugins/{id}/auth/{request_id}",
            get(api_provider_auth_events)
                .post(api_provider_auth_submit)
                .delete(api_provider_auth_cancel),
        )
        .route(
            "/plugin-artifacts/{digest}/{name}",
            get(plugin_release_artifact),
        )
        // Published URL compatibility for releases created before the Plugin
        // Catalog cutover. Delete after those immutable URLs age out.
        .route(
            "/provider-artifacts/{digest}/{name}",
            get(plugin_release_artifact),
        )
}

/// Machine registry, enrollment, connection, Plugin installation and project routes.
pub fn machine_routes(state: &AppState) -> Router<Arc<AppState>> {
    Router::new()
        .merge(
            cowboy_server_plugins::server::routes::routes().with_state(Arc::clone(&state.plugins)),
        )
        .route("/api/project-policies", get(projects::policies))
        .route("/api/project-placements", get(projects::placements))
        .route(
            "/api/machines/{id}/projects",
            get(projects::list).post(projects::request),
        )
        .route(
            "/api/machines/{id}/project-policy",
            put(projects::update_policy),
        )
        .route("/api/machines", get(api_machines))
        .route(
            "/api/machines/enrollment",
            post(api_machine_create_enrollment).delete(api_machine_cancel_enrollment),
        )
        .route("/api/machines/{id}/events", get(api_machine_events))
        .route(
            "/api/machines/{id}/deployment-health",
            get(api_machine_deployment_health),
        )
        .route(
            "/api/machines/{id}/durable-state",
            get(api_machine_durable_state),
        )
        .route("/api/machines/{id}/plugins", get(api_machine_plugins))
        .route(
            "/api/machines/{id}/providers/{provider_id}/uninstall-plan",
            post(api_machine_plugin_uninstall_plan),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/uninstall-plan",
            post(api_machine_plugin_uninstall_plan),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/lifecycle-history",
            get(plugin_history::get_history),
        )
        .route("/api/machines/{id}/refresh", post(api_machine_refresh))
        .route(
            "/api/machines/{id}/components/reconcile",
            post(api_machine_reconcile),
        )
        .route(
            "/api/machines/{id}/components/reconcile-one",
            post(api_machine_reconcile_one),
        )
        .route(
            "/api/machines/{id}/components/update-npm",
            post(api_machine_update_npm),
        )
        .route("/api/machines/{id}/revoke", post(api_machine_revoke))
        .route("/api/machine/service", get(api_machine_service))
        .route("/api/machine/enroll", post(api_machine_enroll))
        .route("/api/machine/connect", any(machine_ws_upgrade))
        .route(root_machine_lanes::LANE_PATH, any(machine_lanes::upgrade))
        .merge(
            cowboy_server_accounts::server::routes::machine_stream_routes()
                .with_state(Arc::clone(&state.accounts)),
        )
        .route(
            "/api/machines/{id}/maintenance",
            put(session_maintenance::configure),
        )
}

/// Linked context, its handoff to a new agent, and forks.
fn with_session_context_routes(
    router: Router<Arc<AppState>>,
    _state: &Arc<AppState>,
) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/sessions/{id}/context",
            get(session_context::get).put(session_context::set),
        )
        .route(
            "/api/sessions/{id}/context/send",
            post(session_context::send).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route(
            "/api/sessions/{id}/context/cancel",
            post(session_context::cancel),
        )
        .route("/api/sessions/{id}/fork", post(session_context::fork))
        .route(
            "/api/sessions/{id}/lineage",
            get(session_context::lineage_get),
        )
        .route(
            "/api/sessions/{id}/lineage/{hop_id}/history",
            get(session_context::lineage_history),
        )
        .route(
            "/api/sessions/{id}/lineage/{hop_id}/question-pages",
            get(session_context::lineage_question_pages),
        )
        .route(
            "/api/sessions/{id}/lineage/{hop_id}/question-pages/{page_id}",
            get(session_context::lineage_question_page),
        )
        .route(
            "/api/sessions/{id}/continue-here",
            post(session_context::continue_here),
        )
        .route(
            "/api/sessions/{id}/duplicate/options",
            post(session_context::duplicate_options),
        )
        .route(
            "/api/sessions/{id}/duplicate",
            post(session_context::duplicate),
        )
}

/// Session, execution, draft, managed-call, history and WebSocket routes.
pub fn session_routes(state: &Arc<AppState>) -> Router<Arc<AppState>> {
    with_session_context_routes(Router::new(), state)
        .route("/api/drafts", get(draft_documents::list))
        .route(
            "/api/drafts/mutations",
            post(draft_documents::mutate).layer(DefaultBodyLimit::max(12 * 1024 * 1024)),
        )
        .route("/api/drafts/{id}", get(draft_documents::read))
        .route("/api/drafts/{id}/history", get(draft_documents::history))
        .route(
            "/api/drafts/{id}/history/index",
            get(draft_documents::history_index),
        )
        .route(
            "/api/drafts/{id}/history/{revision}",
            get(draft_documents::history_snapshot),
        )
        .route(
            "/api/sessions/{id}/draft-copies/{cmid}",
            delete(draft_documents::undo_copy).layer(DefaultBodyLimit::max(12 * 1024 * 1024)),
        )
        .route(
            "/api/sessions",
            post(api_new_session).layer(middleware::from_fn_with_state(
                Arc::clone(&state.session_create_replays),
                session_create_replay::replay,
            )),
        )
        .route("/api/sessions/{id}/calls", get(managed_calls::list))
        .route(
            "/api/sessions/{id}/calls/dismiss",
            post(managed_calls::dismiss),
        )
        .route(
            "/api/sessions/{id}/calls/approval",
            post(managed_calls::decide_approval),
        )
        .merge(session_tool_routes(state))
        .route("/api/agent-tools", get(agent_tools::list))
        .route(
            "/api/folders",
            get(folder_defaults::list).post(folder_defaults::create),
        )
        .route(
            "/api/folders/{id}/defaults",
            get(folder_defaults::get_defaults).patch(folder_defaults::update),
        )
        .route(
            "/api/agent-tools/{agent}",
            put(agent_tools::configure_agent),
        )
        .route(
            "/api/sessions/{id}/calls/{call_id}",
            get(managed_calls::inspect),
        )
        .route(
            "/api/sessions/{id}/calls/{call_id}/cancel",
            post(managed_calls::cancel),
        )
        .route(
            "/api/execution-sessions",
            post(execution::sessions::create).layer(middleware::from_fn_with_state(
                Arc::clone(&state.session_create_replays),
                session_create_replay::replay,
            )),
        )
        .route(
            "/api/execution-environments",
            get(execution::sessions::availability),
        )
        .route("/api/sessions/{id}/info", get(api_session_info))
        .route(
            "/api/sessions/{id}/reload",
            get(api_session_reload_plan)
                .post(api_session_reload)
                .put(session_provider_updates::configure),
        )
        .route("/api/sessions/{id}/hibernate", post(api_session_hibernate))
        .route(
            "/api/session-maintenance",
            get(session_maintenance::inventory),
        )
        .route(
            "/api/sessions/{id}/cache-protection",
            get(api_session_cache_protection),
        )
        .route(
            "/api/sessions/{id}/cache-metrics",
            get(cache_metrics_api::session_cache_metrics),
        )
        .route(
            "/api/sessions/{id}/context-rollover",
            get(session_context::rollover::get).put(session_context::rollover::set),
        )
        .route("/api/sessions/{id}/question-pages", get(api_question_pages))
        .route(
            "/api/sessions/{id}/question-pages/{page_id}",
            get(api_question_page),
        )
        .route("/api/sessions/{id}/bootstrap", get(api_session_bootstrap))
        .route(
            "/api/sessions/{id}/subagents/lookup",
            post(background_tasks::lookup),
        )
        .route(
            "/api/sessions/{id}/subagents/{child}/transcript",
            get(background_tasks::transcript),
        )
        .route(
            "/api/sessions/{id}/tasks/acknowledge",
            post(notifications::acknowledge),
        )
        .route("/api/sessions/{id}/prompt", post(api_session_prompt))
        .route("/api/sessions/{id}/config-revert", get(config_revert::get))
        .route(
            "/api/sessions/{id}/config-revert/restore",
            post(config_revert::restore),
        )
        .route(
            "/api/sessions/{id}/delivery/{operation}",
            get(send_delivery::receipt),
        )
        .route(
            "/api/sessions/{id}/delivery-options",
            get(send_delivery::options),
        )
        .route(
            "/api/sessions/{id}/delivery-policy",
            put(send_delivery::set_policy),
        )
        .route(
            "/api/sessions/{id}/delivery-hold/release",
            post(send_delivery::release),
        )
        .route("/api/history/{id}", get(api_history))
        .route("/api/artifacts/{name}", get(api_artifact))
        .route("/ws", any(ws_upgrade))
}

/// Code browsing, intelligence, buffer and workspace routes.
pub fn code_routes(state: &Arc<AppState>) -> Router<Arc<AppState>> {
    Router::<Arc<AppState>>::new()
        .merge(code_buffers::routes(code_buffers::Context {
            service_id: state.service_id.clone(),
            hub: state.hub.clone(),
            machine_control: Arc::clone(&state.machine_control),
            code_buffers: Arc::clone(&state.code_buffers),
            code_navigation_admission: state.code_navigation_admission,
            shutdown: state.shutdown.clone(),
            product_auth_enabled: state.product_auth_enabled,
            store: state.store.clone(),
            device_access: Arc::clone(&state.device_access),
            product_authentication: Arc::clone(&state.product_authentication),
        }))
        .merge(cowboy_server_code::server::routes::routes().with_state(Arc::clone(&state.code)))
}

fn session_tool_routes(state: &AppState) -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/sessions/{id}/tools",
            get(agent_tools::session).put(agent_tools::configure_session),
        )
        .merge(
            cowboy_server_accounts::server::routes::session_acknowledge_routes()
                .with_state(Arc::clone(&state.accounts)),
        )
        .merge(mcp_bindings::routes())
}
