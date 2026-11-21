//! The persistence layer of the Cowboy base crate and every module it builds on.
//!
//! `store` and the modules it reaches live here so an edit to the Hub, the CLI,
//! the supervisor or any other module above the store does not recompile this
//! crate. Nothing here may depend on the base crate (`cowboy`); the base
//! crate re-exports every module at its old path, so `crate::store` and
//! `cowboy::store` keep resolving.

#![allow(clippy::len_without_is_empty, clippy::new_without_default)]

#[cfg(feature = "full")]
pub mod account_controls;
#[cfg(feature = "full")]
pub mod admin;
#[cfg(feature = "full")]
pub mod agent_model;
#[cfg(feature = "full")]
pub mod artifacts;
#[cfg(feature = "full")]
pub mod client_auth;
#[cfg(feature = "full")]
pub mod context_rollover;
#[cfg(feature = "full")]
pub mod core_passkeys;
#[cfg(feature = "full")]
pub mod core_passkeys_contract;
#[cfg(feature = "full")]
pub mod core_security;
#[cfg(feature = "full")]
pub mod credentials;
#[cfg(feature = "full")]
pub mod digest;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod execution_target_environment;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod first_party_sources;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod managed_calls;
#[cfg(feature = "full")]
pub mod migration_compat;
#[cfg(feature = "full")]
pub mod model;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod operation_budget;
#[cfg(feature = "full")]
pub mod passkey;
#[cfg(feature = "full")]
pub mod persistence;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod plugin_auth_probe;
#[cfg(feature = "full")]
pub mod plugin_dir;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod plugin_host;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod plugin_host_bundle;
#[cfg(feature = "full")]
pub mod plugin_operation;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod plugin_runtime_args;
#[cfg(feature = "full")]
pub mod plugin_storage;
#[cfg(feature = "full")]
pub mod product_auth;
#[cfg(feature = "full")]
pub mod prompt_origin;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub mod provider_behavior;
#[cfg(feature = "full")]
pub mod provider_service;
#[cfg(feature = "full")]
pub mod review_directory;
#[cfg(feature = "full")]
pub mod session_folders;
#[cfg(feature = "machine-host")]
pub mod session_workspace;
#[cfg(feature = "full")]
pub mod store;
#[cfg(feature = "full")]
pub mod superagent;
#[cfg(feature = "full")]
pub mod task_notifications;
#[cfg(feature = "full")]
pub mod telemetry_binding;
#[cfg(feature = "full")]
pub mod transcript_anchors;

// Wire modules re-exported at the root, matching the base crate, so
// `crate::machine_protocol` and its neighbours resolve the same way here.
#[cfg(feature = "full")]
pub use cowboy_wire::agent_api as agent_api_limits;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::execution_environment;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::execution_protocol;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::machine_auth;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::machine_protocol;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::mcp_binding;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::provider_account;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::runtime_wire;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub use cowboy_wire::service_identity;
#[cfg(feature = "full")]
#[doc(hidden)]
pub use png;

/// Create a FIFO for tests (shared with the base and server crates).
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
