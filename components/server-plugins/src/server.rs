//! Server Plugin installation, uninstallation, and lifecycle recovery modules.

pub use crate::core::{Hub, Inbound};
pub use crate::machine_control::MachineControl;
pub use crate::product_auth::ProductPrincipal;
pub use crate::store::Store;
pub use crate::supervisor::Supervisor;
pub use crate::usage::now_ms;
pub use anyhow::{Context as _, Result};
pub use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
pub use base64::Engine as _;
pub use cowboy_server_auth::server::{
    AuthenticatedProductRequest, PluginFenceState, PluginLifecycleFences, ProductRequestAuth,
    RouteAuth, active_machine_plugin, classify_route, current_machine_plugin_from_store,
    no_store_json, operator_approval, product_continuation, provider_auth_sync,
    provider_session_has_active_turn, random_machine_token, require_admin_role,
};
pub use serde::{Deserialize, Serialize};
pub use std::collections::HashMap;
pub use std::sync::Arc;

pub mod lifecycle;
pub mod plugin_install;
pub mod plugin_uninstall;

pub use lifecycle::*;
pub use plugin_install::{api_machine_plugin_install, api_machine_plugin_install_operations};
pub use plugin_uninstall::{
    api_machine_plugin_operation_receipt, api_machine_plugin_operations,
    api_machine_plugin_recovery_assessment, api_machine_plugin_uninstall,
};

pub const PROVIDER_UNINSTALL_RETENTION_MS: i64 = 3 * 24 * 60 * 60 * 1_000;
pub const PROVIDER_UNINSTALL_PLAN_TTL_MS: i64 = 10 * 60 * 1_000;

#[derive(Clone)]
pub struct PluginUninstallPlan {
    pub actor: crate::plugin_operation::Actor,
    pub machine_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
    pub installation_revision:
        Option<crate::machine_protocol::installation_revision::InstallationRevision>,
    pub contract_fingerprint: String,
    pub session_ids: Vec<String>,
    pub active_session_ids: Vec<String>,
    pub purge_after_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInstallRequest {
    pub operation_id: String,
    pub version: String,
    pub digest: String,
}

#[derive(Debug, Deserialize)]
pub struct PluginUninstallRequest {
    pub plan_id: String,
    #[serde(default)]
    pub confirm_active_sessions: bool,
}

pub struct PluginsState {
    pub service_id: String,
    pub hub: Hub,
    pub store: Option<Store>,
    pub supervisor: Arc<Supervisor>,
    pub machine_control: Arc<MachineControl>,
    pub plugin_catalog: Arc<crate::plugin_catalog::PluginCatalog>,
    pub plugin_storage: crate::plugin_storage::PluginStorage,
    pub provider_catalog: Arc<crate::provider_catalog::ProviderCatalog>,
    pub provider_auth: Arc<crate::provider_service::ProviderAuthService>,
    pub provider_auth_sync: Arc<provider_auth_sync::Coordinator>,
    pub plugin_lifecycle_fences: PluginLifecycleFences,
    pub desired_machine_plugins: Arc<crate::machine_convergence::PluginDesiredSource>,
    pub plugin_uninstall_plans: parking_lot::Mutex<HashMap<String, PluginUninstallPlan>>,
    pub plugin_resolution_plans: plugin_uninstall::resolution::ResolutionPlans,
    pub product_auth_enabled: bool,
    pub device_access: Arc<crate::client_auth::DeviceAccessSessions>,
    pub product_authentication: Arc<crate::auth_plugins::ProductAuthentication>,
}

impl<'a> From<&'a PluginsState> for ProductRequestAuth<'a> {
    fn from(state: &'a PluginsState) -> Self {
        Self {
            product_auth_enabled: state.product_auth_enabled,
            store: state.store.as_ref(),
            hub: &state.hub,
            device_access: &state.device_access,
            product_authentication: &state.product_authentication,
        }
    }
}

pub(crate) type AppState = PluginsState;

pub fn provider_compatibility_response(
    problem: cowboy_provider_sdk::ProviderCompatibilityProblem,
) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": "provider_incompatible",
            "code": problem.code,
            "detail": problem.detail,
        })),
    )
        .into_response()
}

pub fn provider_auth_sync_required_before_install(
    authentication: Option<&crate::provider_service::ProviderAuthenticationStatus>,
    installed: Option<&crate::machine_protocol::PluginInventory>,
) -> bool {
    let Some(authentication) = authentication else {
        return false;
    };
    !installed.is_some_and(|provider| {
        provider.auth_generation == Some(authentication.auth_generation)
            && provider.replica_state == crate::machine_protocol::ProviderReplicaState::Current
    })
}

pub async fn provider_auth_envelope_for_machine(
    state: &Arc<PluginsState>,
    machine_id: &str,
    provider_id: &str,
) -> Result<crate::machine_protocol::SealedProviderAuth, String> {
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| "Provider authentication sync requires persistence".to_owned())?;
    let public_key = store
        .machine_encryption_public_key(machine_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Machine {machine_id:?} has no enrolled encryption key"))?;
    state
        .provider_auth
        .seal_for_machine(provider_id, &public_key)
        .map_err(|error| error.to_string())
}
