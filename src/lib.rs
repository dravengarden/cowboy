//! Cowboy shared library.
//!
//! The public surface is intentionally tiny: binaries in this package share
//! the daemon, agent-runtime broker, worker entry points, and their versioned
//! local IPC contract. Application modules remain crate-private so the runtime
//! wire protocol — not Rust's module layout — is the compatibility boundary.

#[cfg(feature = "full")]
mod acp;
#[cfg(feature = "full")]
mod acp_bridge;
#[cfg(feature = "full")]
mod admin;
#[cfg(feature = "full")]
mod agent_model;
#[cfg(feature = "full")]
mod agent_sink;
#[cfg(feature = "full")]
mod artifacts;
#[cfg(feature = "full")]
mod cgroup;
#[cfg(any(feature = "full", feature = "machine-host"))]
mod claude_shell;
#[cfg(feature = "full")]
pub mod cli;
#[cfg(any(feature = "full", feature = "code-adapter"))]
pub mod code_adapter;
#[cfg(feature = "full")]
mod code_cache;
#[cfg(any(feature = "full", feature = "code-adapter"))]
pub mod code_review;
#[cfg(feature = "full")]
mod core;
#[cfg(any(feature = "full", feature = "machine-host"))]
#[path = "provider/deepseek_cache.rs"]
mod deepseek_cache;
#[cfg(any(feature = "full", feature = "machine-host"))]
#[path = "provider/deepseek_context.rs"]
mod deepseek_context;
#[cfg(feature = "full")]
mod diff_snapshot;
#[cfg(any(feature = "full", feature = "code-adapter"))]
mod files;
#[cfg(any(feature = "full", feature = "machine-host"))]
mod grok;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod machine_auth;
#[cfg(feature = "machine-host")]
mod machine_broker;
#[cfg(feature = "machine-host")]
pub mod machine_cli;
#[cfg(feature = "machine-host")]
mod machine_components;
#[cfg(feature = "full")]
mod machine_control;
#[cfg(feature = "machine-host")]
pub mod machine_install;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod machine_protocol;
#[cfg(feature = "machine-host")]
mod machine_providers;
#[cfg(feature = "full")]
mod memory_observability;
#[cfg(feature = "full")]
mod observability;
#[cfg(feature = "full")]
mod persistence;
#[cfg(feature = "full")]
mod prompt_origin;
#[cfg(feature = "full")]
mod provider;
#[cfg(any(feature = "full", feature = "machine-host"))]
mod provider_behavior;
#[cfg(any(feature = "full", feature = "machine-host"))]
mod provider_catalog;
#[cfg(feature = "full")]
mod provider_info;
#[cfg(feature = "full")]
mod provider_service;
#[cfg(feature = "machine-host")]
mod provider_usage_spool;
#[cfg(feature = "full")]
mod remote_runtime;
#[cfg(feature = "full")]
mod runtime;
#[cfg(feature = "full")]
mod runtime_router;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod runtime_wire;
#[cfg(feature = "full")]
mod scheduler;
#[cfg(feature = "full")]
mod server;
#[cfg(feature = "machine-host")]
mod session_workspace;
#[cfg(feature = "full")]
mod store;
#[cfg(feature = "full")]
mod supervisor;
#[cfg(feature = "full")]
mod usage;
#[cfg(feature = "full")]
pub mod worker;
#[cfg(feature = "full")]
mod workspace;
#[cfg(any(feature = "full", feature = "machine-host", feature = "code-adapter"))]
mod workspace_roots;

#[cfg(all(test, feature = "full"))]
mod migration_policy;
#[cfg(all(test, feature = "full"))]
mod protocol_contract;
#[cfg(feature = "full")]
mod web_push;
