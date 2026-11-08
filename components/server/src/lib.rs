//! The Controller server on top of the shared cowboy crate.
//!
//! The base modules the server uses are re-exported at its root, so the server
//! code keeps addressing them as crate::<module> while the surface it depends
//! on stays explicit. The base crate is built with its default `full` feature.
//! A server module that starts using another base module adds it here.

#[cfg(unix)]
pub use cowboy::local_operator;
#[cfg(test)]
pub use cowboy::test_fifo;
pub use cowboy::{
    acp, admin, agent_api, agent_model, auth_plugins, browser_device, cli, client_auth,
    code_adapter, code_buffer_read, code_cache, code_review, composition, config, core,
    core_passkeys, core_security, deepseek_cache, diff_snapshot, durable_state,
    execution_environment, execution_protocol, files, generation_retention, logs, machine_auth,
    machine_control, machine_convergence, machine_lanes, machine_protocol, machine_transport,
    managed_calls, managed_config, mcp_binding, memory_observability, observability, oidc,
    operation_budget, otlp, passkey, persistence, plugin, plugin_activation, plugin_catalog,
    plugin_dir, plugin_host, plugin_host_bundle, plugin_operation, plugin_process, plugin_runtime,
    plugin_runtime_args, plugin_storage, product_auth, project_placement, prompt_origin, provider,
    provider_account, provider_behavior, provider_catalog, provider_service, remote_runtime,
    review_directory, runtime, runtime_router, runtime_trace, runtime_wire, scheduler,
    send_delivery, service_identity, session_context, session_folders, store, superagent,
    supervisor, task_notifications, telemetry_binding, telemetry_file, telemetry_plugin,
    transcript_anchors, usage, web_push, widgets, workspace, workspace_extensions,
};

mod server;

#[cfg(test)]
pub use cowboy_machine::{
    execution_host, machine_broker, machine_cli, machine_code_plugins, machine_plugins,
};

#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod controller_policy_tests;
#[cfg(test)]
mod plugin_catalog_tests;

/// Register the server entry points the shared CLI and supervisor call.
pub fn install_controller_hooks() {
    cowboy::controller_hooks::install(cowboy::controller_hooks::ControllerHooks {
        serve: |args| Box::pin(server::serve(args)),
        init_tracing: server::init_tracing,
        launch_extensions: server::launch_extensions,
        launch_mcp: server::launch_mcp,
    });
}
