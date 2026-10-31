//! Cowboy shared library.
//!
//! The public surface is intentionally tiny: binaries in this package share
//! the daemon, agent-runtime broker, worker entry points, and their versioned
//! local IPC contract. Application modules remain crate-private so the runtime
//! wire protocol — not Rust's module layout — is the compatibility boundary.

// Modules that used to be crate-private are now public so cowboy-server can use them.
#![allow(clippy::len_without_is_empty, clippy::new_without_default)]

/// The memory-first allocator the controller links (`src/main.rs`). Sibling
/// binaries such as the Machine host select it through this export: a
/// dependency of their own would change `Cargo.lock`, which retained workers
/// pin byte for byte (`retained-worker-interface-files` in `flake.nix`).
#[cfg(all(not(target_env = "msvc"), not(target_family = "wasm")))]
pub use tikv_jemallocator::Jemalloc;

#[cfg(feature = "full")]
pub mod acp;
#[cfg(feature = "full")]
pub mod acp_bridge;
#[cfg(feature = "full")]
pub mod admin;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod agent_api;
#[cfg(feature = "full")]
pub mod agent_model;
#[cfg(feature = "full")]
pub mod agent_sink;
#[cfg(feature = "full")]
pub mod artifacts;
#[cfg(feature = "full")]
pub mod auth_plugins;
#[cfg(feature = "full")]
pub mod browser_device;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod cgroup;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod claude_shell;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod cli;
#[cfg(feature = "full")]
pub mod client_auth;
#[cfg(feature = "full")]
pub mod client_auth_client;
#[cfg(any(feature = "full", feature = "code-adapter"))]
pub mod code_adapter;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod code_buffer_read;
#[cfg(feature = "full")]
pub mod code_cache;
#[cfg(any(feature = "full", feature = "code-adapter"))]
pub mod code_review;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod component_proof;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod composition;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod config;
#[cfg(feature = "full")]
pub mod controller_hooks;
#[cfg(feature = "full")]
pub mod core;
#[cfg(feature = "full")]
pub mod core_passkeys;
#[cfg(feature = "full")]
pub mod core_security;
#[cfg(any(feature = "full", feature = "machine-host"))]
#[path = "provider/deepseek_cache.rs"]
pub mod deepseek_cache;
#[cfg(any(feature = "full", feature = "machine-host"))]
#[path = "provider/deepseek_context.rs"]
pub mod deepseek_context;
#[cfg(feature = "full")]
pub mod diff_snapshot;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod durable_state;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::execution_environment;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::execution_protocol;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod execution_target_environment;
#[cfg(any(feature = "full", feature = "code-adapter"))]
pub mod files;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod first_party_sources;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod generation_retention;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod legacy_provider_release;
#[cfg(all(feature = "full", unix))]
pub mod local_operator;
pub mod logs;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::machine_auth;
#[cfg(feature = "machine-host")]
pub mod machine_components;
#[cfg(feature = "full")]
pub mod machine_control;
#[cfg(feature = "full")]
pub mod machine_convergence;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod machine_install;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod machine_lanes;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::machine_protocol;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod machine_transport;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod managed_calls;
#[cfg(feature = "full")]
#[path = "provider/managed_config.rs"]
pub mod managed_config;
#[cfg(feature = "full")]
pub mod memory_observability;
#[cfg(feature = "full")]
pub mod observability;
#[cfg(feature = "full")]
pub mod oidc;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod operation_budget;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod otlp;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod owned_json;
#[cfg(feature = "full")]
pub mod passkey;
#[cfg(feature = "full")]
pub mod persistence;
#[cfg(feature = "full")]
pub mod plugin;
#[cfg(feature = "full")]
pub mod plugin_activation;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod plugin_auth_probe;
#[cfg(feature = "full")]
pub mod plugin_catalog;
#[cfg(feature = "full")]
pub mod plugin_dir;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod plugin_host;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod plugin_host_bundle;
#[cfg(feature = "full")]
pub mod plugin_operation;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod plugin_process;
#[cfg(feature = "full")]
pub mod plugin_runtime;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod plugin_runtime_args;
#[cfg(feature = "full")]
pub mod plugin_storage;
#[cfg(feature = "full")]
pub mod product_auth;
#[cfg(feature = "full")]
pub mod product_cli;
#[cfg(feature = "full")]
pub mod project_placement;
#[cfg(feature = "full")]
pub mod prompt_origin;
#[cfg(feature = "full")]
pub mod provider;
#[cfg(feature = "full")]
pub mod transcript_anchors;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::provider_account;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod provider_behavior;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod provider_catalog;
#[cfg(feature = "full")]
pub mod provider_info;
#[cfg(feature = "full")]
pub mod provider_service;
#[cfg(feature = "machine-host")]
pub mod provider_usage_spool;
#[cfg(feature = "full")]
pub mod remote_runtime;
#[cfg(feature = "full")]
pub mod review_directory;
#[cfg(feature = "full")]
pub mod runtime;
#[cfg(feature = "full")]
pub mod runtime_router;
#[cfg(feature = "full")]
pub mod runtime_telemetry;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod runtime_trace;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::runtime_wire;
#[cfg(feature = "full")]
pub mod scheduler;
#[cfg(feature = "full")]
pub mod send_delivery;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::service_identity;
#[cfg(feature = "full")]
pub mod session_context;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod session_deletion_admission;
#[cfg(feature = "full")]
pub mod session_folders;
#[cfg(feature = "machine-host")]
pub mod session_workspace;
#[cfg(feature = "full")]
pub mod store;
#[cfg(feature = "full")]
pub mod supervisor;
#[cfg(feature = "full")]
pub mod telemetry_binding;
#[cfg(feature = "full")]
pub mod telemetry_file;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod telemetry_plugin;
#[cfg(feature = "full")]
pub mod usage;
#[cfg(feature = "full")]
pub mod worker;
#[cfg(feature = "full")]
pub mod worker_execution;
#[cfg(feature = "full")]
pub mod worker_telemetry;
#[cfg(feature = "full")]
pub mod workspace;
#[cfg(any(feature = "full", feature = "machine-host", feature = "code-adapter"))]
pub mod workspace_roots;

#[cfg(feature = "full")]
pub mod migration_compat;
#[cfg(all(test, feature = "full"))]
pub mod migration_policy;
#[cfg(all(test, feature = "full"))]
pub mod protocol_contract;
#[cfg(feature = "full")]
pub mod web_push;

#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod workspace_extensions;

/// FIFO fixtures use the POSIX utility: rustix's *at constructors are absent
/// on macOS. Keep special-file rejection tests active on both host platforms.
#[cfg(all(
    any(test, feature = "test-support"),
    any(feature = "full", feature = "machine-host")
))]
pub fn test_fifo(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    let status = std::process::Command::new("mkfifo")
        .args(["-m", "600"])
        .arg(path.as_ref())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("mkfifo failed: {status}")))
    }
}
