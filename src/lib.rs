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
mod agent_model;
#[cfg(feature = "full")]
mod agent_sink;
pub mod agentd;
#[cfg(feature = "full")]
mod artifacts;
#[cfg(feature = "full")]
mod cgroup;
#[cfg(feature = "full")]
pub mod cli;
#[cfg(feature = "full")]
mod code_cache;
#[cfg(feature = "full")]
mod code_review;
#[cfg(feature = "full")]
mod core;
#[cfg(feature = "full")]
mod diff_snapshot;
#[cfg(feature = "full")]
mod files;
#[cfg(feature = "full")]
mod inference;
pub mod machine_cli;
pub mod machine_protocol;
#[cfg(feature = "full")]
mod persistence;
#[cfg(feature = "full")]
mod provider;
#[cfg(feature = "full")]
mod remote_runtime;
#[cfg(feature = "full")]
mod runtime;
pub mod runtime_wire;
#[cfg(feature = "full")]
mod scheduler;
#[cfg(feature = "full")]
mod server;
#[cfg(feature = "full")]
mod skills;
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

#[cfg(all(test, feature = "full"))]
mod migration_policy;
#[cfg(all(test, feature = "full"))]
mod protocol_contract;
