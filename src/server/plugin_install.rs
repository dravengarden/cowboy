//! A finite, core-owned install/upgrade attempt. The HTTP request observes the
//! attempt; it does not own its lifetime. This closes the legacy live-operation
//! gaps, but does not turn protocol-seven ACKs into durable install receipts.

use super::operator_approval::{InstallationAuthority, OperatorApproval};
use super::*;
use crate::machine_control::{CommandFailure, CommandRequestError, ConnectionToken};
use crate::machine_protocol::{DesiredPlugin, MachineCommand};

#[derive(Clone, Copy)]
enum Disposition {
    Previous,
    Uncertain,
    Installed,
}

struct InstallationFence {
    fences: PluginLifecycleFences,
    key: (String, String),
    previous: Option<PluginFenceState>,
    disposition: Disposition,
}

impl InstallationFence {
    fn acquire(fences: &PluginLifecycleFences, key: (String, String)) -> Result<Self, StatusCode> {
        let mut active = fences.write();
        let previous = active.get(&key).copied();
        if previous.is_some_and(|state| state != PluginFenceState::Uninstalled) {
            return Err(StatusCode::CONFLICT);
        }
        active.insert(key.clone(), PluginFenceState::Installing);
        Ok(Self {
            fences: Arc::clone(fences),
            key,
            previous,
            disposition: Disposition::Previous,
        })
    }
}

impl Drop for InstallationFence {
    fn drop(&mut self) {
        let next = match self.disposition {
            Disposition::Previous => self.previous,
            Disposition::Uncertain => Some(PluginFenceState::NeedsReconcile),
            Disposition::Installed => None,
        };
        let mut active = self.fences.write();
        if let Some(next) = next {
            active.insert(self.key.clone(), next);
        } else {
            active.remove(&self.key);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Installed,
    NotDispatched,
    AuthenticationPending,
    NeedsReconcile,
}

impl Outcome {
    fn response(self) -> Response {
        let detail = match self {
            Self::Installed => return StatusCode::NO_CONTENT.into_response(),
            Self::NotDispatched => {
                "Plugin installation was not sent: confirmation, compatibility, connection or authentication preconditions changed. Refresh before confirming again."
            }
            Self::AuthenticationPending => {
                "Plugin installed, but Service authentication reconciliation is pending. This does not authorize replaying the installation."
            }
            Self::NeedsReconcile => {
                "Plugin installation outcome is uncertain. This installation slot remains fenced; do not retry or infer recovery from current inventory."
            }
        };
        (StatusCode::CONFLICT, detail).into_response()
    }
}

trait Effects: Sync {
    fn authorized(&self) -> impl std::future::Future<Output = bool> + Send;
    fn needs_auth_sync(
        &self,
        before_install: bool,
    ) -> impl std::future::Future<Output = bool> + Send;
    fn sync_auth(&self) -> impl std::future::Future<Output = bool> + Send;
    fn install(&self) -> impl std::future::Future<Output = Result<(), CommandRequestError>> + Send;
}

async fn coordinate(effects: &impl Effects, fence: &mut InstallationFence) -> Outcome {
    if !effects.authorized().await {
        return Outcome::NotDispatched;
    }
    if effects.needs_auth_sync(true).await
        && (!effects.authorized().await || !effects.sync_auth().await)
    {
        return Outcome::NotDispatched;
    }
    if !effects.authorized().await {
        return Outcome::NotDispatched;
    }
    // Cancellation/panic after this point may leave a remote effect. Only a
    // proven NotSent transport result can restore the previous local fence.
    fence.disposition = Disposition::Uncertain;
    match effects.install().await {
        Ok(()) => fence.disposition = Disposition::Installed,
        Err(error) if error.certainty == CommandFailure::NotSent => {
            fence.disposition = Disposition::Previous;
            return Outcome::NotDispatched;
        }
        // A generic rejected ACK also cannot prove that activation/auth writes
        // were rolled back. Machine installation tracking independently fences
        // interrupted local transitions; no unjournaled inverse is attempted.
        Err(_) => return Outcome::NeedsReconcile,
    }
    if effects.needs_auth_sync(false).await
        && (!effects.authorized().await || !effects.sync_auth().await)
    {
        return Outcome::AuthenticationPending;
    }
    Outcome::Installed
}

struct LiveEffects {
    state: Arc<AppState>,
    machine: String,
    desired: DesiredPlugin,
    connection: ConnectionToken,
    authority: InstallationAuthority,
}

impl Effects for LiveEffects {
    async fn authorized(&self) -> bool {
        let state = &self.state;
        let release = &self.desired.release;
        let valid = self.authority.within_budget()
            && state.machine_control.is_current(&self.connection)
            && state
                .plugin_catalog
                .resolve(
                    &release.plugin_id,
                    Some(&release.plugin_version),
                    Some(&release.artifact_digest),
                )
                .is_ok_and(|desired| desired == self.desired)
            && plugin_install_compatibility(state, &self.machine, &self.desired)
                .await
                .is_ok_and(|problem| problem.is_none())
            && (release.plugin_kind != cowboy_plugin_sdk::PluginKind::AgentProvider
                || agent_plugin_install_compatibility(state, &self.machine, &self.desired)
                    .await
                    .is_ok_and(|problem| problem.is_none()))
            && self
                .authority
                .check(
                    ProductRequestAuth::from(state.as_ref()),
                    &state.service_id,
                    &self.machine,
                    &self.desired,
                )
                .await
            && state.machine_control.is_current(&self.connection)
            // The Operator/compatibility reads may have waited for storage.
            // Recheck Catalog trust after those awaits, immediately before the
            // caller can enqueue an effect on the captured connection.
            && state.plugin_catalog.resolve(
                &release.plugin_id, Some(&release.plugin_version), Some(&release.artifact_digest),
            ).is_ok_and(|desired| desired == self.desired)
            && self.authority.within_budget();
        if !valid {
            self.authority.revoke();
        }
        valid
    }

    async fn needs_auth_sync(&self, before_install: bool) -> bool {
        if self.desired.release.plugin_kind != cowboy_plugin_sdk::PluginKind::AgentProvider {
            return false;
        }
        let plugin = &self.desired.release.plugin_id;
        let authentication = self.state.provider_auth.status(plugin);
        if !before_install {
            return authentication.is_some();
        }
        let installed = current_machine_plugin(&self.state, &self.machine, plugin)
            .await
            .ok();
        provider_auth_sync_required_before_install(authentication.as_ref(), installed.as_ref())
    }

    async fn sync_auth(&self) -> bool {
        let Ok(envelope) = provider_auth_envelope_for_machine(
            &self.state,
            &self.machine,
            &self.desired.release.plugin_id,
        )
        .await
        else {
            return false;
        };
        // Key lookup may wait for storage. Check again at enqueue, retaining the
        // original connection rather than silently following a reconnect.
        self.authorized().await
            && self
                .state
                .provider_auth_sync
                .apply(&self.state.machine_control, &self.connection, envelope)
                .await
                .is_ok()
    }

    async fn install(&self) -> Result<(), CommandRequestError> {
        dispatch(&self.state.machine_control, &self.connection, &self.desired).await
    }
}

async fn dispatch(
    control: &MachineControl,
    connection: &ConnectionToken,
    desired: &DesiredPlugin,
) -> Result<(), CommandRequestError> {
    let request_id = machine_request_id("plugin-install");
    control
        .command_on_connection(
            connection,
            request_id.clone(),
            MachineCommand::InstallPlugin {
                request_id,
                plugin: Box::new(desired.clone()),
            },
        )
        .await
}

async fn run_admitted(effects: LiveEffects, mut fence: InstallationFence) -> Response {
    // Preserve the existing typed compatibility response at initial admission.
    match plugin_install_compatibility(&effects.state, &effects.machine, &effects.desired).await {
        Ok(Some(problem)) => return plugin_compatibility_response(problem),
        Err(_) => return Outcome::NotDispatched.response(),
        Ok(None) => {}
    }
    if effects.desired.release.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider {
        match agent_plugin_install_compatibility(&effects.state, &effects.machine, &effects.desired)
            .await
        {
            Ok(Some(problem)) => return provider_compatibility_response(problem),
            Err(_) => return Outcome::NotDispatched.response(),
            Ok(None) => {}
        }
    }
    coordinate(&effects, &mut fence).await.response()
}

async fn observe(
    attempt: impl std::future::Future<Output = Response> + Send + 'static,
) -> Response {
    match tokio::spawn(attempt).await {
        Ok(response) => response,
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Plugin installation interrupted; inspect the installation before another lifecycle action.").into_response(),
    }
}

pub(super) async fn api_machine_plugin_install(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin)): Path<(String, String)>,
    authenticated: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(request): Json<PluginInstallRequest>,
) -> Response {
    let approval = match OperatorApproval::capture(
        ProductRequestAuth::from(state.as_ref()),
        &state.service_id,
        authenticated.as_ref().map(|Extension(auth)| auth),
        &headers,
    ) {
        Ok(approval) => approval,
        Err(status) => return status.into_response(),
    };
    let desired =
        match state
            .plugin_catalog
            .resolve(&plugin, Some(&request.version), Some(&request.digest))
        {
            Ok(desired) => desired,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    "Select an exact trusted Plugin release",
                )
                    .into_response();
            }
        };
    let connection = match state.machine_control.operation_connection(&machine) {
        Ok(connection) => connection,
        Err(_) => return Outcome::NotDispatched.response(),
    };
    let authority = match approval.bind_installation(&machine, &desired) {
        Ok(authority) => authority,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let fence =
        match InstallationFence::acquire(&state.plugin_lifecycle_fences, (machine.clone(), plugin))
        {
            Ok(fence) => fence,
            Err(status) => {
                return (
                    status,
                    "Plugin lifecycle is changing or requires reconciliation",
                )
                    .into_response();
            }
        };
    observe(run_admitted(
        LiveEffects {
            state,
            machine,
            desired,
            connection,
            authority,
        },
        fence,
    ))
    .await
}

#[cfg(test)]
mod tests;
