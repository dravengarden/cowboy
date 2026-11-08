//! The Controller HTTP route table, grouped by domain. `serve_axum` merges
//! these builders and adds the SPA fallback and middleware layers.

use super::*;

/// Health, metrics, telemetry, diagnostics, preferences and notification routes.
pub(super) fn platform_routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(cardea_authorization::routes())
        .merge(telemetry_binding::resolution::surface::routes())
        .merge(telemetry_binding::recovery::surface::routes())
        .merge(telemetry_binding::surface::routes())
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
            post(api_otlp).layer(DefaultBodyLimit::max(crate::otlp::MAX_BYTES)),
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
pub(super) fn provider_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/provider-accounts",
            get(provider_accounts::read)
                .put(provider_accounts::update)
                .layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/usage",
            get(provider_accounts::usage::list),
        )
        .route(
            "/api/provider-accounts/{account_id}/usage",
            get(provider_accounts::usage::read).post(provider_accounts::usage::refresh),
        )
        .route(
            "/api/provider-accounts/{account_id}/revocation",
            get(provider_accounts::lifecycle::http_status)
                .post(provider_accounts::lifecycle::http_start)
                .layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/revocation/reconcile",
            post(provider_accounts::lifecycle::http_resume).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/login-operations/{operation}",
            get(provider_accounts::login::http_receipt)
                .delete(provider_accounts::login::http_cancel),
        )
        .route(
            "/api/provider-accounts/{account_id}/logins",
            get(provider_accounts::login::http_logins),
        )
        .route(
            "/api/provider-accounts/{account_id}/logins/{request}",
            get(provider_accounts::login::http_continue)
                .delete(provider_accounts::login::http_continue_cancel),
        )
        .route(
            "/api/provider-accounts/{account_id}/login",
            get(provider_accounts::login::options)
                .post(provider_accounts::login::start)
                .layer(DefaultBodyLimit::max(16 * 1024)),
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
                crate::plugin_process::MAX_PLUGIN_COMMAND_INPUT_BYTES,
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
pub(super) fn machine_routes() -> Router<Arc<AppState>> {
    Router::new()
    .route("/api/project-policies", get(projects::policies))
    .route("/api/project-placements", get(projects::placements))
    .route("/api/machines/{id}/projects", get(projects::list).post(projects::request))
    .route("/api/machines/{id}/project-policy", put(projects::update_policy))
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
        "/api/machines/{id}/providers/{provider_id}",
        post(api_machine_plugin_install),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}",
        post(api_machine_plugin_install),
    )
    .route(
        "/api/machines/{id}/providers/{provider_id}/uninstall-plan",
        post(api_machine_plugin_uninstall_plan),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/uninstall-plan",
        post(api_machine_plugin_uninstall_plan),
    )
    .route(
        "/api/machines/{id}/providers/{provider_id}/uninstall",
        post(api_machine_plugin_uninstall),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/uninstall",
        post(api_machine_plugin_uninstall),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/operations",
        get(api_machine_plugin_operations),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/installation-operations",
        get(plugin_install::api_machine_plugin_install_operations),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/lifecycle-history",
        get(plugin_history::get_history),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/machine-receipt",
        get(api_machine_plugin_operation_receipt),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/recovery-assessment",
        get(api_machine_plugin_recovery_assessment),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/resolution-plan",
        post(plugin_uninstall::resolution::api_resolution_plan),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/resolve",
        post(plugin_uninstall::resolution::api_resolve),
    )
    .route(
        "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/resolution",
        get(plugin_uninstall::resolution::api_resolution_receipt),
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
    .route(crate::machine_lanes::LANE_PATH, any(machine_lanes::upgrade))
    .route("/api/machines/{id}/maintenance", put(session_maintenance::configure))
}

/// Draft documents, including copies a session made of them.
fn draft_routes() -> Router<Arc<AppState>> {
    Router::new()
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
}

/// Session, execution, draft, managed-call, history and WebSocket routes.
pub(super) fn session_routes(state: &Arc<AppState>) -> Router<Arc<AppState>> {
    draft_routes()
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
        .route(
            "/api/sessions/{id}/tools",
            get(agent_tools::session).put(agent_tools::configure_session),
        )
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
        .route(
            "/api/sessions/reconcile-project",
            post(api_reconcile_project_sessions),
        )
        .route("/api/sessions/{id}/info", get(api_session_info))
        .route(
            "/api/sessions/{id}/reload",
            get(api_session_reload_plan)
                .post(api_session_reload)
                .put(session_provider_updates::configure),
        )
        .route("/api/sessions/{id}/hibernate", post(api_session_hibernate))
        .route("/api/sessions/{id}/review-fetch", post(api_review_fetch))
        .route(
            "/api/session-maintenance",
            get(session_maintenance::inventory),
        )
        .route(
            "/api/sessions/{id}/cache-protection",
            get(api_session_cache_protection),
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
            "/api/sessions/{id}/tasks/acknowledge",
            post(notifications::acknowledge),
        )
        .route("/api/sessions/{id}/prompt", post(api_session_prompt))
        .route(
            "/api/sessions/{id}/context",
            get(session_context::get).put(session_context::set),
        )
        .route(
            "/api/sessions/{id}/context/send",
            post(session_context::send).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route("/api/sessions/{id}/fork", post(session_context::fork))
        .route(
            "/api/sessions/{id}/continue-here",
            post(session_context::continue_here),
        )
        .route(
            "/api/sessions/{id}/duplicate",
            post(session_context::duplicate).layer(middleware::from_fn_with_state(
                Arc::clone(&state.session_create_replays),
                session_create_replay::replay,
            )),
        )
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
pub(super) fn code_routes(state: &Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .merge(code_buffers::routes(state))
        .route("/api/workspaces", get(api_workspaces))
        .route("/api/sessions/{id}/files", get(api_search_files))
        .route("/api/code/sessions/{id}/tree", get(api_file_tree))
        .merge(code_api::search_routes())
        .merge(code_api::check_routes())
        .route("/api/code/sessions/{id}/manifest", get(api_code_manifest))
        .route("/api/code/sessions/{id}/changes", get(api_code_changes))
        .route(
            "/api/code/sessions/{id}/extensions",
            get(workspace_extensions::inventory),
        )
        .route(
            "/api/code/sessions/{id}/extensions/resources",
            get(workspace_extensions::resources),
        )
        .route(
            "/api/code/sessions/{id}/repository",
            get(api_code_repository),
        )
        .route("/api/code/sessions/{id}/commit", get(api_code_commit))
        .route(
            "/api/code/sessions/{id}/commit-diff",
            get(api_code_commit_diff),
        )
        .route("/api/code/sessions/{id}/diff", get(api_code_diff))
        .route("/api/code/sessions/{id}/file", get(api_code_file))
        .route("/api/code/sessions/{id}/file-raw", get(api_code_file_raw))
        .route("/api/code/sessions/{id}/language", get(api_code_language))
        .route(
            "/api/code/sessions/{id}/intelligence/hover",
            get(api_code_hover),
        )
        .route(
            "/api/code/sessions/{id}/intelligence/navigation",
            get(api_code_navigation),
        )
        .route(
            "/api/code/sessions/{id}/intelligence/outline",
            get(api_code_outline),
        )
        .route(
            "/api/code/sessions/{id}/buffer",
            put(api_code_buffer_open).delete(api_code_buffer_close),
        )
}
