//! Cross-boundary Controller + Machine integration and conformance tests.
//!
//! Kept outside `cowboy-server` so `cowboy-server`'s unit test graph compiles
//! independently of `cowboy-machine`.

#[cfg(all(test, feature = "machine-host"))]
pub use cowboy::{
    admin, auth_plugins, browser_device, client_auth, code_adapter, code_buffer_read, composition,
    core, execution_environment, execution_protocol, local_operator, machine_auth, machine_control,
    machine_protocol, observability, operation_budget, otlp, plugin_activation, plugin_catalog,
    plugin_dir, plugin_operation, plugin_storage, product_auth, provider_service, runtime_wire,
    store, telemetry_binding, telemetry_file, telemetry_plugin,
};
#[cfg(all(test, feature = "machine-host"))]
pub use cowboy_machine::{machine_cli, machine_plugins};

#[cfg(all(test, feature = "machine-host"))]
mod server;
