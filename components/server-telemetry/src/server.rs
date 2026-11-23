//! Re-exported auth primitives and the telemetry binding coordinator.

pub use cowboy_server_auth::server::http_policy::*;
pub use cowboy_server_auth::server::product_auth_api::*;
pub use cowboy_server_auth::server::types::*;
pub use cowboy_server_auth::server::{
    AuthenticatedProductRequest, PluginFenceState, PluginLifecycleFences, ProductRequestAuth,
    RouteAuth, http_policy, operator_approval, product_auth_api, types,
};

pub mod telemetry_binding;
