//! A finite, core-owned durable install/upgrade attempt. The HTTP request
//! observes it; neither a disconnected observer nor restart owns replay rights.
//! Retained protocol-seven ACKs remain readable. Fresh execution requires the
//! exact protocol-nineteen target and durable Machine receipt.

use super::operator_approval::{InstallationAuthority, OperatorApproval};
use super::*;
use crate::machine_control::{CommandFailure, CommandRequestError, ConnectionToken};
use crate::machine_protocol::DesiredPlugin;
use crate::machine_protocol::plugin_install::{
    InstallLookup, InstallObservation, InstallOutcome, InstallStep, InstallTargetObservation,
    InstallTargetQuery,
};
use crate::plugin_operation::installation::{InstallIntent, InstallPhase, InstallProblem};
use axum::http::HeaderValue;

pub(super) mod journal;
use journal::Progress;
pub(super) use journal::api_machine_plugin_install_operations;

// The 2026-09-14 reader floor is active in both lanes, next recovery and cold
// bootstrap (docs/releases/plugin-install-receipt-readers-2026-09-14.md).
// Fresh execution still needs the original Operator confirmation, protocol-19
// connection, exact observed target and independently enabled Machine writer.
// Release acceptance also requires the connected writer/recovery matrix.
pub(super) const DURABLE_INSTALL_ENABLED: bool = true;

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
    RejectedBeforeStaging,
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
            Self::RejectedBeforeStaging => {
                "Machine durably rejected this installation before staging began. The original attempt will not be replayed. Refresh the target before a new confirmation."
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
    fn install(
        &self,
        step: &InstallStep,
    ) -> impl std::future::Future<Output = Result<InstallObservation, CommandRequestError>> + Send;
}

async fn coordinate(
    effects: &impl Effects,
    fence: &mut InstallationFence,
    progress: &mut Progress<'_>,
) -> anyhow::Result<Outcome> {
    // Legacy records are read-only evidence, not a route back into execution.
    let step = progress.machine_step()?;
    if !effects.authorized().await {
        return progress
            .abort(fence, InstallProblem::PreconditionsChanged)
            .await;
    }
    if effects.needs_auth_sync(true).await {
        progress
            .advance(InstallPhase::SyncingAuthentication, None)
            .await?;
        if !effects.authorized().await {
            return progress
                .abort(fence, InstallProblem::PreconditionsChanged)
                .await;
        }
        if !effects.sync_auth().await {
            return progress
                .abort(fence, InstallProblem::AuthenticationSyncFailed)
                .await;
        }
    }
    progress.advance(InstallPhase::Installing, None).await?;
    // A journal commit can wait on storage. Recheck after it, at enqueue.
    if !effects.authorized().await {
        return progress
            .abort(fence, InstallProblem::TransportNotSent)
            .await;
    }
    // Cancellation/panic after this point may leave a remote effect. Only
    // proven NotSent or a committed pre-Staging rejection can release the fence.
    fence.disposition = Disposition::Uncertain;
    match effects.install(&step).await {
        Ok(InstallObservation {
            result: InstallLookup::Found { receipt },
            ..
        }) => {
            // Store validates full plan/actor/target binding and atomically
            // commits the receipt together with its Service phase.
            progress.machine_receipt(&receipt).await?;
            match receipt.outcome {
                InstallOutcome::Applied { .. } => {}
                InstallOutcome::Rejected { .. } => {
                    fence.disposition = Disposition::Previous;
                    return Ok(Outcome::RejectedBeforeStaging);
                }
                InstallOutcome::Pending { .. } | InstallOutcome::Unknown { .. } => {
                    return Ok(Outcome::NeedsReconcile);
                }
            }
        }
        Err(error) if error.certainty == CommandFailure::NotSent => {
            return progress
                .abort(fence, InstallProblem::TransportNotSent)
                .await;
        }
        // Missing/unavailable evidence and a generic ACK prove neither success
        // nor absence of effects. Never resend or infer rollback from inventory.
        Ok(_) | Err(_) => {
            progress
                .advance(
                    InstallPhase::NeedsAttention,
                    Some(InstallProblem::UnknownMachineOutcome),
                )
                .await?;
            return Ok(Outcome::NeedsReconcile);
        }
    }
    if effects.needs_auth_sync(false).await
        && (!effects.authorized().await || !effects.sync_auth().await)
    {
        progress
            .advance(
                InstallPhase::AuthenticationPending,
                Some(InstallProblem::AuthenticationSyncFailed),
            )
            .await?;
        fence.disposition = Disposition::Installed;
        return Ok(Outcome::AuthenticationPending);
    }
    progress.advance(InstallPhase::Completed, None).await?;
    fence.disposition = Disposition::Installed;
    Ok(Outcome::Installed)
}

struct LiveEffects {
    state: Arc<AppState>,
    machine: String,
    release: crate::plugin_catalog::VerifiedPluginRelease,
    connection: ConnectionToken,
    authority: InstallationAuthority,
    intent: InstallIntent,
}

impl Effects for LiveEffects {
    async fn authorized(&self) -> bool {
        let state = &self.state;
        let desired = self.release.desired();
        let valid = self.authority.within_budget()
            && self.authority.intent() == &self.intent
            && state.machine_control.is_current(&self.connection)
            && self.release.current(&state.plugin_catalog)
            && plugin_install_compatibility(state, &self.machine, desired)
                .await
                .is_ok_and(|problem| problem.is_none())
            && (desired.release.plugin_kind != cowboy_plugin_sdk::PluginKind::AgentProvider
                || agent_plugin_install_compatibility(state, &self.machine, desired)
                    .await
                    .is_ok_and(|problem| problem.is_none()))
            && self
                .authority
                .check(
                    ProductRequestAuth::from(state.as_ref()),
                    &state.service_id,
                    &self.machine,
                    desired,
                )
                .await
            && state.machine_control.is_current(&self.connection)
            // The Operator/compatibility reads may have waited for storage.
            // Recheck Catalog trust after those awaits, immediately before the
            // caller can enqueue an effect on the captured connection.
            && self.release.current(&state.plugin_catalog)
            && self.authority.within_budget();
        if !valid {
            self.authority.revoke();
        }
        valid
    }

    async fn needs_auth_sync(&self, before_install: bool) -> bool {
        if self.release.desired().release.plugin_kind
            != cowboy_plugin_sdk::PluginKind::AgentProvider
        {
            return false;
        }
        let plugin = &self.release.desired().release.plugin_id;
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
            &self.release.desired().release.plugin_id,
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

    async fn install(&self, step: &InstallStep) -> Result<InstallObservation, CommandRequestError> {
        if !self.authority.matches_live_step(step)
            || !self.release.current(&self.state.plugin_catalog)
        {
            self.authority.revoke();
            return Err(CommandRequestError {
                certainty: CommandFailure::NotSent,
                detail: "installation confirmation no longer matches the step".into(),
            });
        }
        dispatch(
            &self.state.machine_control,
            &self.connection,
            self.release.desired(),
            step,
        )
        .await
    }
}

async fn dispatch(
    control: &MachineControl,
    connection: &ConnectionToken,
    desired: &DesiredPlugin,
    step: &InstallStep,
) -> Result<InstallObservation, CommandRequestError> {
    control
        .plugin_installation_step(connection, step, Some(desired))
        .await
}

async fn run_admitted(effects: LiveEffects, mut fence: InstallationFence) -> Response {
    // Preserve the existing typed compatibility response at initial admission.
    match plugin_install_compatibility(&effects.state, &effects.machine, effects.release.desired())
        .await
    {
        Ok(Some(problem)) => return plugin_compatibility_response(problem),
        Err(_) => return Outcome::NotDispatched.response(),
        Ok(None) => {}
    }
    if effects.release.desired().release.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
    {
        match agent_plugin_install_compatibility(
            &effects.state,
            &effects.machine,
            effects.release.desired(),
        )
        .await
        {
            Ok(Some(problem)) => return provider_compatibility_response(problem),
            Err(_) => return Outcome::NotDispatched.response(),
            Ok(None) => {}
        }
    }
    let Some(store) = effects.state.store.as_ref() else {
        return Outcome::NotDispatched.response();
    };
    if !effects.authorized().await {
        return Outcome::NotDispatched.response();
    }
    // Even COMMIT failure may be ambiguous. From here, only a durable terminal
    // transition can release this process's reservation.
    fence.disposition = Disposition::Uncertain;
    let result = if store.begin_plugin_install(&effects.intent).await.is_ok() {
        let mut progress = Progress::new(store, &effects.intent);
        match coordinate(&effects, &mut fence, &mut progress).await {
            Ok(outcome) => outcome,
            Err(_) => {
                progress.storage_failure().await;
                Outcome::NeedsReconcile
            }
        }
    } else {
        Outcome::NeedsReconcile
    };
    let mut response = result.response();
    if let Ok(id) = HeaderValue::from_str(&effects.intent.operation_id) {
        response
            .headers_mut()
            .insert("x-cowboy-plugin-operation", id);
    }
    response
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
    if !DURABLE_INSTALL_ENABLED {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Plugin installation is paused while its durable recovery reader floor is activated",
        )
            .into_response();
    }
    if !crate::plugin_operation::installation::valid_operation_id(&request.operation_id) {
        return (StatusCode::BAD_REQUEST, "Invalid Plugin operation identity").into_response();
    }
    let approval = match OperatorApproval::capture(
        ProductRequestAuth::from(state.as_ref()),
        &state.service_id,
        authenticated.as_ref().map(|Extension(auth)| auth),
        &headers,
    ) {
        Ok(approval) => approval,
        Err(status) => return status.into_response(),
    };
    let Some(store) = state.store.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    // A repeated identity is observation only, even after expiry, Catalog change,
    // reconnect or process restart. It never creates a replacement grant.
    match store.plugin_install_operation(&request.operation_id).await {
        Ok(Some(operation)) => {
            return journal::duplicate_response(
                &operation,
                &state.service_id,
                approval.actor(),
                &machine,
                &plugin,
                &request,
            );
        }
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Ok(None) => {}
    }
    let release = match state.plugin_catalog.resolve_verified_exact(
        &plugin,
        &request.version,
        &request.digest,
    ) {
        Ok(release) => release,
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
    // Pure observation on the original connection; inventory absence is never
    // a Vacant target. Time spent here consumes the original approval budget.
    let query = InstallTargetQuery {
        schema: 1,
        service_id: state.service_id.clone(),
        machine_id: machine.clone(),
        plugin_id: plugin.clone(),
    };
    let target = match state
        .machine_control
        .plugin_installation_target(&connection, &query)
        .await
    {
        Ok(InstallTargetObservation::Observed {
            admission_enabled: true,
            target,
            ..
        }) => target,
        _ => return Outcome::NotDispatched.response(),
    };
    let authority =
        match approval.bind_installation(&machine, release.desired(), request.operation_id, target)
        {
            Ok(authority) => authority,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
    let intent = authority.intent().clone();
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
            release,
            connection,
            authority,
            intent,
        },
        fence,
    ))
    .await
}

#[cfg(test)]
mod tests;
