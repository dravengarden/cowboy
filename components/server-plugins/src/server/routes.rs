//! HTTP routes for Machine Plugin installation, uninstallation, and recovery operations.

use super::*;

/// Machine Plugin operation, installation, uninstallation, and resolution routes.
pub fn routes() -> Router<Arc<PluginsState>> {
    Router::new()
        .route(
            "/api/machines/{id}/providers/{provider_id}",
            post(api_machine_plugin_install),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}",
            post(api_machine_plugin_install),
        )
        .route(
            "/api/machines/{id}/providers/{provider_id}/uninstall",
            post(api_machine_plugin_uninstall),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/uninstall",
            post(api_machine_plugin_uninstall),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations",
            get(api_machine_plugin_operations),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/installation-operations",
            get(plugin_install::api_machine_plugin_install_operations),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/machine-receipt",
            get(api_machine_plugin_operation_receipt),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/recovery-assessment",
            get(api_machine_plugin_recovery_assessment),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/resolution-plan",
            post(plugin_uninstall::resolution::api_resolution_plan),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/resolve",
            post(plugin_uninstall::resolution::api_resolve),
        )
        .route(
            "/api/machines/{id}/plugins/{provider_id}/operations/{operation_id}/resolution",
            get(plugin_uninstall::resolution::api_resolution_receipt),
        )
}
