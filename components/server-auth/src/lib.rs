//! Controller product authentication, HTTP policy, secure transport,
//! product continuation, admin API, and operator approval.

#[cfg(unix)]
pub use cowboy::local_operator;
pub use cowboy::{
    admin, agent_model, auth_plugins, browser_device, cli, client_auth, core, core_passkeys,
    core_security, machine_auth, machine_control, machine_convergence, machine_protocol, oidc,
    operation_budget, otlp, passkey, persistence, plugin_activation, plugin_catalog, plugin_dir,
    plugin_host, plugin_operation, plugin_storage, product_auth, provider_catalog, runtime,
    runtime_router, store, supervisor, telemetry_binding, telemetry_plugin,
};

pub mod server;
