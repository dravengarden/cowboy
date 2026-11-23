//! Server Provider accounts, logins, usage, controls, revocation, and Machine browsers.

pub use crate::agent_model::Status;
pub use crate::core::{Hub, Inbound, Outbound};
pub use crate::machine_control::MachineControl;
pub use crate::product_auth::ProductPrincipal;
pub use crate::runtime_router::RuntimeRouter;
pub use crate::store::Store;
pub use crate::supervisor::Supervisor;
pub use crate::usage::{UsageService, now_ms};
pub use anyhow::{Context as _, Result};
pub use axum::{
    Extension, Json, Router,
    extract::{
        ConnectInfo, Path, Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, delete, get, post, put},
};
pub use base64::Engine as _;
pub use cowboy_server_auth::server::{
    AuthenticatedProductRequest, PluginFenceState, PluginLifecycleFences,
    ProductAuthSessionSnapshot, ProductRequestAuth, WAIT_PREFIX, active_machine_plugin,
    authorize_ws_upgrade, current_machine_plugin_from_store, machine_diagnostic_event_visible,
    machine_request_id, new_reset_idempotency_key, no_store_json, product_auth_deadline_instant,
    product_auth_session_message_for, provider_auth_sync, random_machine_token, send_json,
    send_json_with_timeout, sync_dataset, ws_close_auth_required,
};
pub use futures::{SinkExt, StreamExt};
pub use serde::{Deserialize, Serialize};
pub use sha2::{Digest as _, Sha256};
pub use std::collections::{BTreeMap, BTreeSet, HashMap};
pub use std::net::SocketAddr;
pub use std::path::PathBuf;
pub use std::sync::Arc;
pub use tokio::sync::watch;

pub mod helpers;
pub mod provider_accounts;
pub mod routes;

pub use helpers::*;

pub struct AccountsState {
    pub service_id: String,
    pub service_config_path: PathBuf,
    pub service_config: crate::config::Handle,
    pub hub: Hub,
    pub store: Option<Store>,
    pub supervisor: Arc<Supervisor>,
    pub shutdown: watch::Receiver<bool>,
    pub runtime_router: Arc<RuntimeRouter>,
    pub machine_control: Arc<MachineControl>,
    pub browsers: provider_accounts::browser::coordinator::Coordinator,
    pub account_controls: Arc<provider_accounts::controls::Cache>,
    pub plugin_catalog: Arc<crate::plugin_catalog::PluginCatalog>,
    pub provider_catalog: Arc<crate::provider_catalog::ProviderCatalog>,
    pub provider_auth: Arc<crate::provider_service::ProviderAuthService>,
    pub provider_auth_sync: Arc<provider_auth_sync::Coordinator>,
    pub provider_auth_executors: Arc<parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>>,
    pub plugin_lifecycle_fences: PluginLifecycleFences,
    pub usage: UsageService,
    pub public_origins: Arc<Vec<String>>,
    pub product_auth_enabled: bool,
    pub device_access: Arc<crate::client_auth::DeviceAccessSessions>,
    pub product_authentication: Arc<crate::auth_plugins::ProductAuthentication>,
}

impl<'a> From<&'a AccountsState> for ProductRequestAuth<'a> {
    fn from(state: &'a AccountsState) -> Self {
        Self {
            product_auth_enabled: state.product_auth_enabled,
            store: state.store.as_ref(),
            hub: &state.hub,
            device_access: &state.device_access,
            product_authentication: &state.product_authentication,
        }
    }
}

pub(crate) type AppState = AccountsState;

pub async fn current_machine_plugin(
    state: &Arc<AccountsState>,
    machine_id: &str,
    provider_id: &str,
) -> Result<crate::machine_protocol::PluginInventory, String> {
    current_machine_plugin_from_store(state.store.as_ref(), machine_id, provider_id).await
}

pub async fn product_auth_session_message(
    state: &AccountsState,
    cookie_token: &str,
) -> Option<ProductAuthSessionSnapshot> {
    product_auth_session_message_for(
        state.store.as_ref(),
        &state.hub,
        &state.product_authentication,
        cookie_token,
    )
    .await
}

pub const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(20);

pub async fn principal_still_valid(
    state: &AccountsState,
    via_cookie: bool,
    cookie_token: Option<&str>,
    bearer: Option<&str>,
    device_identity: Option<&crate::client_auth::DeviceAccessIdentity>,
    expected: &ProductPrincipal,
) -> bool {
    cowboy_server_auth::server::principal_still_valid_for(
        ProductRequestAuth::from(state),
        via_cookie,
        cookie_token,
        bearer,
        device_identity,
        expected,
    )
    .await
}
