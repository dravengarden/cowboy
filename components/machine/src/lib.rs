//! The Machine host on top of the shared cowboy crate.
//!
//! The base modules this crate uses are re-exported at its root, grouped by
//! the base crate's own feature gates, so the moved code keeps addressing
//! them as crate::<module> while the surface it depends on stays explicit.

pub use cowboy::logs;
#[cfg(all(
    any(test, feature = "test-support"),
    any(feature = "full", feature = "machine-host")
))]
pub use cowboy::test_fifo;
#[cfg(feature = "full")]
pub use cowboy::{
    acp, agent_model, agent_sink, artifacts, core, machine_control, observability, provider,
    remote_runtime, telemetry_file, worker, worker_execution,
};
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy::{
    cgroup, claude_shell, code_buffer_read, config, deepseek_cache, deepseek_context,
    durable_state, execution_environment, execution_protocol, execution_target_environment,
    generation_retention, legacy_provider_release, machine_auth, machine_install, machine_lanes,
    machine_protocol, machine_transport, managed_calls, operation_budget, otlp, owned_json,
    plugin_auth_probe, plugin_host, plugin_host_bundle, plugin_process, plugin_runtime_args,
    provider_account, provider_behavior, runtime_trace, runtime_wire, service_identity,
    session_deletion_admission, telemetry_plugin, workspace_extensions, workspace_roots,
};
#[cfg(feature = "machine-host")]
pub use cowboy::{machine_components, provider_usage_spool, session_workspace};

#[cfg(feature = "machine-host")]
mod boot_identity;

#[cfg(feature = "machine-host")]
pub mod code_observer;
#[cfg(feature = "machine-host")]
pub mod execution_host;
#[cfg(feature = "machine-host")]
pub mod launch_ledger;
pub(crate) mod login_registry;
#[cfg(feature = "machine-host")]
pub mod machine_broker;
#[cfg(feature = "machine-host")]
pub mod machine_browser;
#[cfg(feature = "machine-host")]
pub mod machine_cli;
#[cfg(feature = "machine-host")]
pub mod machine_code_plugins;
#[cfg(feature = "machine-host")]
pub mod machine_plugins;
#[cfg(feature = "machine-host")]
pub mod memory_relay;

#[cfg(all(test, target_os = "linux", feature = "machine-host"))]
mod cgroup_containment_tests;
#[cfg(all(test, feature = "full", feature = "machine-host"))]
mod machine_control_binding_tests;
#[cfg(all(test, feature = "full", feature = "machine-host"))]
mod machine_control_matrix_tests;
#[cfg(all(test, feature = "full", feature = "machine-host"))]
mod worker_execution_tests;
#[cfg(all(test, feature = "full", feature = "machine-host"))]
mod worker_trace_tests;
