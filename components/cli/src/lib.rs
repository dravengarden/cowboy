//! The `cowboy` CLI, ACP bridge, product CLI, and Machine installer.

#![allow(clippy::len_without_is_empty, clippy::new_without_default)]

#[cfg(all(unix, any(feature = "full", feature = "machine-host")))]
pub use cowboy::agent_mcp;
#[cfg(all(feature = "full", unix))]
pub use cowboy::local_operator;
#[cfg(all(
    any(test, feature = "test-support"),
    any(feature = "full", feature = "machine-host")
))]
pub use cowboy::test_fifo;
#[cfg(feature = "full")]
pub use cowboy::{
    account_controls, acp, admin, agent_model, agent_sink, artifacts, auth_plugins, browser_device,
    client_auth, code_adapter, code_cache, code_review, context_rollover, controller_hooks, core,
    core_passkeys, core_passkeys_contract, core_security, credentials, diff_snapshot, digest,
    files, machine_control, machine_convergence, managed_config, mcp_binding, memory_observability,
    model, native_configuration, observability, oidc, passkey, persistence, plugin,
    plugin_activation, plugin_catalog, plugin_dir, plugin_operation, plugin_runtime,
    plugin_storage, product_auth, project_placement, prompt_origin, provider, provider_info,
    provider_service, remote_runtime, review_directory, runtime, runtime_router, runtime_telemetry,
    scheduler, send_delivery, serve_args, session_context, session_folders, store,
    subagent_transcripts, superagent, supervisor, task_notifications, telemetry_binding,
    telemetry_file, transcript_anchors, usage, web_push, worker, worker_execution,
    worker_telemetry, workspace,
};
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy::{
    component_proof, composition, config, durable_state, execution_environment, execution_protocol,
    first_party_sources, generation_retention, legacy_provider_release, logs, machine_auth,
    machine_lanes, machine_protocol, machine_transport, managed_calls, operation_budget, otlp,
    owned_json, plugin_host, plugin_host_bundle, plugin_process, plugin_runtime_args,
    provider_account, provider_behavior, provider_catalog, runtime_trace, runtime_wire,
    service_identity, session_deletion_admission, telemetry_plugin, workspace_extensions,
    workspace_roots,
};

#[cfg(feature = "full")]
pub mod acp_bridge;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod cli;
#[cfg(feature = "full")]
pub mod client_auth_client;
#[cfg(feature = "machine-host")]
pub mod machine_components;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod machine_install;
#[cfg(feature = "full")]
pub mod product_cli;
