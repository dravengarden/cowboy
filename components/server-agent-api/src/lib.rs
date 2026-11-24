//! Controller Agent API surface (`cowboy_read`, `cowboy_execute`, control
//! dispatch, code reads, and interactive widgets).

#[cfg(unix)]
pub use cowboy::local_operator;
#[cfg(test)]
pub use cowboy::test_fifo;
pub use cowboy::{
    account_controls, acp, admin, agent_model, artifacts, auth_plugins, browser_device,
    client_auth, code_adapter, code_buffer_read, code_cache, code_review, composition, config,
    context_rollover, core, core_passkeys, core_security, deepseek_cache, diff_snapshot,
    durable_state, execution_environment, execution_protocol, files, generation_retention, logs,
    machine_auth, machine_control, machine_convergence, machine_lanes, machine_protocol,
    machine_transport, managed_calls, managed_config, mcp_binding, memory_observability,
    observability, oidc, operation_budget, otlp, passkey, persistence, plugin, plugin_activation,
    plugin_catalog, plugin_dir, plugin_host, plugin_host_bundle, plugin_operation, plugin_process,
    plugin_runtime, plugin_runtime_args, plugin_storage, product_auth, project_placement,
    prompt_origin, provider, provider_account, provider_behavior, provider_catalog,
    provider_service, remote_runtime, review_directory, runtime, runtime_router, runtime_trace,
    runtime_wire, scheduler, send_delivery, serve_args, service_identity, session_context,
    session_folders, store, subagent_transcripts, superagent, supervisor, task_notifications,
    telemetry_binding, telemetry_file, telemetry_plugin, transcript_anchors, usage, web_push,
    workspace, workspace_extensions, wrap_up,
};

pub use cowboy_server_runtime::agent_api;
pub mod server;
pub use cowboy_server_runtime::widgets;
