//! Explicit session MCP policy. Installed Plugins are never implicit grants.
//! The relay must retain and revalidate one admission, including at delivery.
use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;

use super::{AppState, AuthenticatedProductRequest, session_is_visible};
use crate::core::{Hub, SessionMeta, settings_keys::SESSION_MCP_BINDINGS_PREFIX};
use crate::mcp_binding::{AppGrant, Binding, valid_epoch, validate_app_grants, validate_bindings};

pub(super) mod apps;
mod authority;
pub(super) mod defaults;
pub(super) mod launch;
mod launch_prepare;
pub(super) mod relay;

#[cfg(test)]
mod tests;

/// Serialize durable CAS and memory projection so a later update cannot be
/// projected before an earlier commit. No asynchronous settings write is queued.
pub(super) struct Policies {
    mutation: Mutex<()>,
    /// Serializes directory-default application, which spans CAS and preparation.
    defaults: Mutex<()>,
    owner: Arc<()>,
    capacity: Arc<tokio::sync::Semaphore>,
    changes: tokio::sync::watch::Sender<()>,
}

impl Default for Policies {
    fn default() -> Self {
        Self {
            mutation: Mutex::new(()),
            defaults: Mutex::new(()),
            owner: Arc::new(()),
            capacity: Arc::new(tokio::sync::Semaphore::new(16)),
            changes: tokio::sync::watch::channel(()).0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    runtime_machine: String,
    execution_machine: String,
    workspace: String,
    cwd: String,
    provider: String,
    owner: Option<String>,
    execution_binding: Option<crate::execution_environment::ExecutionBinding>,
}

impl Placement {
    fn from_meta(meta: &SessionMeta) -> Result<Self, &'static str> {
        if meta.system {
            return Err("managed_session_denied");
        }
        if meta.closing || meta.trashed_at_ms.is_some() {
            return Err("session_unavailable");
        }
        let (execution_machine, workspace) = match &meta.execution_binding {
            Some(binding) => {
                let binding = binding.decode().map_err(|_| "invalid_execution_binding")?;
                if binding.runtime.machine_id != meta.machine_id || binding.runtime.cwd != meta.cwd
                {
                    return Err("invalid_execution_placement");
                }
                if binding.managed.is_some() {
                    return Err("managed_session_denied");
                }
                (binding.environment.machine_id, binding.workspace.id)
            }
            None => (
                meta.machine_id.clone(),
                meta.workspace_id.clone().ok_or("workspace_required")?,
            ),
        };
        Ok(Self {
            runtime_machine: meta.machine_id.clone(),
            execution_machine,
            workspace,
            cwd: meta.cwd.clone(),
            provider: meta.provider.clone(),
            owner: meta.owner_user_id.clone(),
            execution_binding: meta.execution_binding.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    schema: u16,
    epoch: String,
    placement: Placement,
    bindings: Vec<Binding>,
    /// Owner grants for signed MCP Apps views, by binding name. Kept out of
    /// every process profile; absent for policies without views.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    apps: std::collections::BTreeMap<String, Vec<AppGrant>>,
}

impl Stored {
    fn load(hub: &Hub, session: &str) -> Option<Self> {
        Self::decode(hub.setting(&key(session))?)
    }

    fn decode(value: serde_json::Value) -> Option<Self> {
        let stored: Self = serde_json::from_value(value).ok()?;
        (stored.schema == 1
            && valid_epoch(&stored.epoch)
            && validate_bindings(&stored.bindings).is_ok()
            && validate_app_grants(&stored.bindings, &stored.apps).is_ok())
        .then_some(stored)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Update {
    expected_epoch: Option<String>,
    bindings: Vec<Binding>,
    #[serde(default)]
    apps: std::collections::BTreeMap<String, Vec<AppGrant>>,
}

fn key(session: &str) -> String {
    format!("{SESSION_MCP_BINDINGS_PREFIX}{session}")
}

impl Policies {
    async fn replace(
        &self,
        hub: &Hub,
        store: &crate::store::Store,
        meta: &SessionMeta,
        update: Update,
    ) -> Result<Stored, &'static str> {
        validate_bindings(&update.bindings).map_err(|_| "invalid_bindings")?;
        validate_app_grants(&update.bindings, &update.apps).map_err(|_| "invalid_bindings")?;
        let _mutation = self.mutation.lock().await;
        let current = hub
            .session_info(&meta.id)
            .ok_or("session_unavailable")?
            .meta;
        if hub.is_managed_child(&meta.id) {
            return Err("managed_session_denied");
        }
        let placement = Placement::from_meta(&current)?;
        if placement != Placement::from_meta(meta)? {
            return Err("placement_changed");
        }
        // CAS distinguishes a missing row from an existing JSON null tombstone.
        let expected = hub.with_settings_mut(|settings| settings.get(&key(&meta.id)).cloned());
        let previous = expected
            .clone()
            .filter(|value| !value.is_null())
            .and_then(Stored::decode);
        let corrupt = expected.as_ref().is_some_and(|value| !value.is_null()) && previous.is_none();
        if corrupt && (!update.bindings.is_empty() || update.expected_epoch.is_some()) {
            return Err("invalid_stored_policy");
        }
        if !corrupt
            && previous.as_ref().map(|stored| &stored.epoch) != update.expected_epoch.as_ref()
        {
            return Err("epoch_conflict");
        }
        let stored = Stored {
            schema: 1,
            epoch: format!("{:032x}", rand::random::<u128>()),
            placement,
            bindings: update.bindings,
            apps: update.apps,
        };
        let value = json!(stored);
        if !store
            .compare_and_put_setting(&key(&meta.id), expected.as_ref(), &value)
            .await
            .map_err(|_| "storage_unavailable")?
        {
            return Err("epoch_conflict");
        }
        hub.load_settings(vec![(key(&meta.id), value)]);
        self.changes.send_replace(());
        Ok(stored)
    }
}

fn view(policies: &Policies, hub: &Hub, meta: &SessionMeta) -> serde_json::Value {
    let stored = Stored::load(hub, &meta.id);
    saved_view(policies, hub, meta, stored.as_ref())
}

fn saved_view(
    policies: &Policies,
    hub: &Hub,
    meta: &SessionMeta,
    stored: Option<&Stored>,
) -> serde_json::Value {
    let policy_current = stored.as_ref().is_some_and(|s| {
        !s.bindings.is_empty()
            && s.bindings.iter().all(|binding| {
                policies
                    .admit(hub, &meta.id, &meta.machine_id, &s.epoch, &binding.name)
                    .is_ok_and(|admission| admission.current(policies, hub).is_ok())
            })
    });
    let launch_prepared = policy_current && hub.resolve_mcp_launch(meta).is_ok();
    json!({"schema":1,"epoch":stored.as_ref().map(|s| &s.epoch),"policy_current":policy_current,"launch_prepared":launch_prepared,
        "bindings":stored.as_ref().map_or(&[][..], |s| s.bindings.as_slice()),
        "apps":stored.as_ref().map(|s| s.apps.clone()).unwrap_or_default(),
        "worker_ready":false,"runtime_ready":false})
}

pub(super) fn routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/api/sessions/{id}/mcp-bindings/prepare",
            axum::routing::post(launch_prepare::prepare)
                .layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/sessions/{id}/mcp-apps/frames",
            axum::routing::post(apps::open).layer(axum::extract::DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/sessions/{id}/mcp-apps/frames/{lease}/renew",
            axum::routing::post(apps::renew),
        )
        .route(
            "/api/sessions/{id}/mcp-apps/frames/{lease}",
            axum::routing::delete(apps::close),
        )
        .route(
            "/api/sessions/{id}/mcp-bindings",
            axum::routing::get(read)
                .put(configure)
                .layer(axum::extract::DefaultBodyLimit::max(16 * 1024)),
        )
}

pub(super) async fn read(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(session): Path<String>,
) -> Response {
    if !session_is_visible(&state.hub, &auth.principal, &session) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(info) = state.hub.session_info(&session) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Json(observation(&state, &info.meta)).into_response()
}

pub(super) fn observation(state: &AppState, meta: &SessionMeta) -> serde_json::Value {
    let mut response = view(&state.extensions.bindings, &state.hub, meta);
    response["directory_default"] = defaults::observation(&state.hub, &meta.id);
    state
        .extensions
        .launches
        .observe(state, meta, &mut response);
    response
}

pub(super) async fn configure(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(session): Path<String>,
    Json(update): Json<Update>,
) -> Response {
    if !session_is_visible(&state.hub, &auth.principal, &session) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(info) = state.hub.session_info(&session) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !auth
        .principal
        .can_mutate(info.meta.owner_user_id.as_deref())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = Arc::clone(&state.extensions.bindings.capacity).try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    // Reuse the accepted-transaction lifetime of send_delivery::set_policy:
    // losing the HTTP connection must not strand a committed policy projection.
    match tokio::spawn(async move {
        let _permit = permit;
        let Some(store) = state.store.as_ref() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        match state
            .extensions
            .bindings
            .replace(&state.hub, store, &info.meta, update)
            .await
        {
            Ok(stored) => {
                if !stored.bindings.is_empty() {
                    let _ = state
                        .extensions
                        .launches
                        .prepare(&state, &info.meta.id, &stored.epoch)
                        .await;
                }
                let mut response = saved_view(
                    &state.extensions.bindings,
                    &state.hub,
                    &info.meta,
                    Some(&stored),
                );
                state
                    .extensions
                    .launches
                    .observe(&state, &info.meta, &mut response);
                Json(response).into_response()
            }
            Err(code) => (
                match code {
                    "epoch_conflict" | "invalid_stored_policy" | "placement_changed" => {
                        StatusCode::CONFLICT
                    }
                    "session_unavailable" => StatusCode::NOT_FOUND,
                    "storage_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
                    _ => StatusCode::BAD_REQUEST,
                },
                Json(json!({"error":code})),
            )
                .into_response(),
        }
    })
    .await
    {
        Ok(response) => response,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Configured bindings require a current Controller-owned preparation.
/// Missing and tombstoned policies are deliberately different from corrupt rows.
pub(crate) fn launch_mcp(
    hub: &Hub,
    session: &SessionMeta,
) -> anyhow::Result<Option<Box<crate::mcp_binding::NativeLaunchPlan>>> {
    let Some(value) = hub
        .setting(&key(&session.id))
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    let stored =
        Stored::decode(value).ok_or_else(|| anyhow::anyhow!("invalid stored MCP policy"))?;
    if stored.bindings.is_empty() {
        return Ok(None);
    }
    hub.resolve_mcp_launch(session).map(Some)
}
