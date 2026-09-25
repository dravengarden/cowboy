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
use crate::plugin_operation::installation::{
    InstallIntent, InstallOperation, InstallPhase, InstallProblem,
};
use anyhow::ensure;
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

/// Process-local owner for a fresh reconciliation action. Dropping it for any
/// reason restores the uncertainty fence; only a committed terminal Service
/// phase removes the fence.
struct InstallationReconciliationFence {
    fences: PluginLifecycleFences,
    key: (String, String),
    finished: bool,
}

impl InstallationReconciliationFence {
    fn acquire(fences: &PluginLifecycleFences, key: (String, String)) -> anyhow::Result<Self> {
        let mut active = fences.write();
        ensure!(
            active.get(&key) == Some(&PluginFenceState::NeedsReconcile),
            "installation is not available for reconciliation"
        );
        active.insert(key.clone(), PluginFenceState::Installing);
        Ok(Self {
            fences: Arc::clone(fences),
            key,
            finished: false,
        })
    }

    fn finish(&mut self, next: Option<PluginFenceState>) {
        let mut active = self.fences.write();
        if let Some(next) = next {
            active.insert(self.key.clone(), next);
        } else {
            active.remove(&self.key);
        }
        self.finished = true;
    }
}

impl Drop for InstallationReconciliationFence {
    fn drop(&mut self) {
        if !self.finished {
            self.fences
                .write()
                .insert(self.key.clone(), PluginFenceState::NeedsReconcile);
        }
    }
}

/// Which precondition stopped an installation before it was sent.
///
/// The durable journal keeps recording the single `PreconditionsChanged`
/// problem, so this adds no persisted state and no new reader schema: it exists
/// only to make the HTTP refusal actionable. Naming the class is what the
/// caller was missing — "confirmation, compatibility, connection or
/// authentication preconditions changed" is true of all six of these, and an
/// operator (or an agent converging a fleet) cannot tell from it whether to
/// reconnect a Machine, refresh a Catalog, update Cowboy Machine, or simply
/// re-approve. None of these name a credential or a Machine-private detail;
/// each is state the caller can already read from its own endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Precondition {
    /// The approval expired, was revoked, or no longer matches this request.
    OperatorApproval,
    /// The Machine has no current control connection, or reconnected after the
    /// request captured one.
    MachineConnection,
    /// The Machine answered the target query without enabling installation
    /// admission — its Cowboy Machine build does not accept installs yet.
    MachineAdmission,
    /// The Machine did not return an observable installation target.
    MachineTarget,
    /// The exact release is no longer trusted/current in the Catalog.
    CatalogRelease,
    /// The Machine's reported capability inventory rejects this release.
    MachineCapability,
    /// The Controller's persistence is unavailable.
    Storage,
}

impl Precondition {
    fn detail(self) -> &'static str {
        match self {
            Self::OperatorApproval => {
                "Plugin installation was not sent: the operator approval expired, was revoked, or no longer matches this request. Approve again, then confirm."
            }
            Self::MachineConnection => {
                "Plugin installation was not sent: the Machine has no current control connection, or it reconnected after this request captured one. Confirm the Machine is connected, then retry with the same operation ID."
            }
            Self::MachineAdmission => {
                "Plugin installation was not sent: this Machine reports installation admission disabled. Update Cowboy Machine on that host before installing or upgrading Plugins there."
            }
            Self::MachineTarget => {
                "Plugin installation was not sent: the Machine did not report an observable installation target. Confirm the Machine is connected and its Cowboy Machine build supports Plugin installation."
            }
            Self::CatalogRelease => {
                "Plugin installation was not sent: the selected release is no longer current in the trusted Catalog. Refresh the Catalog and re-read the exact version and digest."
            }
            Self::MachineCapability => {
                "Plugin installation was not sent: the Machine's reported capability inventory does not accept this release. Update Cowboy Machine on that host."
            }
            Self::Storage => {
                "Plugin installation was not sent: Controller persistence is unavailable."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Installed,
    NotDispatched(Precondition),
    RejectedBeforeStaging,
    AuthenticationPending,
    NeedsReconcile,
}

impl Outcome {
    fn response(self) -> Response {
        let detail = match self {
            Self::Installed => return StatusCode::NO_CONTENT.into_response(),
            Self::NotDispatched(precondition) => precondition.detail(),
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
    /// `Ok(())` when every precondition still holds; otherwise the first one
    /// that did not, so a refusal can say what to do about it.
    fn authorized(&self) -> impl std::future::Future<Output = Result<(), Precondition>> + Send;
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
    if let Err(precondition) = effects.authorized().await {
        return progress
            .abort(fence, InstallProblem::PreconditionsChanged, precondition)
            .await;
    }
    if effects.needs_auth_sync(true).await {
        progress
            .advance(InstallPhase::SyncingAuthentication, None)
            .await?;
        if let Err(precondition) = effects.authorized().await {
            return progress
                .abort(fence, InstallProblem::PreconditionsChanged, precondition)
                .await;
        }
        if !effects.sync_auth().await {
            return progress
                .abort(
                    fence,
                    InstallProblem::AuthenticationSyncFailed,
                    Precondition::OperatorApproval,
                )
                .await;
        }
    }
    progress.advance(InstallPhase::Installing, None).await?;
    // A journal commit can wait on storage. Recheck after it, at enqueue.
    if let Err(precondition) = effects.authorized().await {
        return progress
            .abort(fence, InstallProblem::TransportNotSent, precondition)
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
                .abort(
                    fence,
                    InstallProblem::TransportNotSent,
                    Precondition::MachineConnection,
                )
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
        && (effects.authorized().await.is_err() || !effects.sync_auth().await)
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

impl LiveEffects {
    /// The precondition sequence behind `authorized`. Ordered so the caller is
    /// told the earliest thing that has to be true: a stale approval is not
    /// worth reporting as a Catalog problem, and a disconnected Machine is not
    /// worth reporting as an approval problem.
    async fn first_unmet(&self) -> Result<(), Precondition> {
        let state = &self.state;
        let desired = self.release.desired();
        if !self.authority.within_budget() || self.authority.intent() != &self.intent {
            return Err(Precondition::OperatorApproval);
        }
        if !state.machine_control.is_current(&self.connection) {
            return Err(Precondition::MachineConnection);
        }
        if !self.release.current(&state.plugin_catalog) {
            return Err(Precondition::CatalogRelease);
        }
        if !plugin_install_compatibility(state, &self.machine, desired)
            .await
            .is_ok_and(|problem| problem.is_none())
        {
            return Err(Precondition::MachineCapability);
        }
        if desired.release.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
            && !agent_plugin_install_compatibility(state, &self.machine, desired)
                .await
                .is_ok_and(|problem| problem.is_none())
        {
            return Err(Precondition::MachineCapability);
        }
        if !self
            .authority
            .check(
                ProductRequestAuth::from(state.as_ref()),
                &state.service_id,
                &self.machine,
                desired,
            )
            .await
        {
            return Err(Precondition::OperatorApproval);
        }
        // The Operator/compatibility reads may have waited for storage. Recheck
        // the connection and Catalog trust after those awaits, immediately
        // before the caller can enqueue an effect on the captured connection.
        if !state.machine_control.is_current(&self.connection) {
            return Err(Precondition::MachineConnection);
        }
        if !self.release.current(&state.plugin_catalog) {
            return Err(Precondition::CatalogRelease);
        }
        if !self.authority.within_budget() {
            return Err(Precondition::OperatorApproval);
        }
        Ok(())
    }
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
    async fn authorized(&self) -> Result<(), Precondition> {
        // Same checks, same order, same short-circuit as the previous `&&`
        // chain — including the deliberate re-checks after each await, since a
        // connection or Catalog can change while an earlier check waits on
        // storage. The only difference is that the first failure is now named.
        let result = self.first_unmet().await;
        if result.is_err() {
            self.authority.revoke();
        }
        result
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
        self.authorized().await.is_ok()
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
        Err(_) => return Outcome::NotDispatched(Precondition::MachineCapability).response(),
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
            Err(_) => return Outcome::NotDispatched(Precondition::MachineCapability).response(),
            Ok(None) => {}
        }
    }
    let Some(store) = effects.state.store.as_ref() else {
        return Outcome::NotDispatched(Precondition::Storage).response();
    };
    if let Err(precondition) = effects.authorized().await {
        return Outcome::NotDispatched(precondition).response();
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

fn reconciliation_candidate(operation: &InstallOperation) -> bool {
    operation.intent.schema == 2
        && operation.phase == InstallPhase::NeedsAttention
        && match operation.attention_from {
            Some(InstallPhase::Installing) => operation
                .machine_receipt
                .as_ref()
                .is_none_or(|receipt| matches!(&receipt.outcome, InstallOutcome::Pending { .. })),
            Some(InstallPhase::MachineAcknowledged) => matches!(
                operation
                    .machine_receipt
                    .as_ref()
                    .map(|receipt| &receipt.outcome),
                Some(InstallOutcome::Applied { .. })
            ),
            _ => false,
        }
}

async fn commit_reconciled_receipt(
    store: &Store,
    before: &InstallOperation,
    receipt: &crate::machine_protocol::plugin_install::InstallReceipt,
) -> anyhow::Result<InstallOperation> {
    match store
        .reconcile_plugin_install_receipt(before, receipt)
        .await
    {
        Ok(saved) => Ok(saved),
        Err(error) => {
            // A COMMIT response can be lost. Observe this exact receipt and
            // transition; never repeat the write against a changed snapshot.
            let Some(saved) = store
                .plugin_install_operation(&before.intent.operation_id)
                .await?
            else {
                return Err(error);
            };
            ensure!(
                saved.intent == before.intent
                    && saved.machine_receipt.as_ref() == Some(receipt)
                    && matches!(
                        (&receipt.outcome, saved.phase),
                        (
                            InstallOutcome::Applied { .. },
                            InstallPhase::MachineAcknowledged
                        ) | (InstallOutcome::Rejected { .. }, InstallPhase::Aborted)
                    ),
                "installation receipt reconciliation was not committed"
            );
            Ok(saved)
        }
    }
}

async fn commit_reconciled_final_phase(
    store: &Store,
    intent: &InstallIntent,
    phase: InstallPhase,
    problem: Option<InstallProblem>,
) -> anyhow::Result<()> {
    if let Err(error) = store
        .advance_plugin_install(intent, InstallPhase::MachineAcknowledged, phase, problem)
        .await
    {
        if store
            .plugin_install_operation(&intent.operation_id)
            .await?
            .is_some_and(|saved| {
                saved.intent == *intent && saved.phase == phase && saved.problem == problem
            })
        {
            return Ok(());
        }
        let _ = store
            .advance_plugin_install(
                intent,
                InstallPhase::MachineAcknowledged,
                InstallPhase::NeedsAttention,
                Some(InstallProblem::StorageFailure),
            )
            .await;
        return Err(error);
    }
    Ok(())
}

fn reconciliation_response(operation: &InstallOperation) -> Response {
    no_store_json(
        StatusCode::OK,
        serde_json::json!({
            "schema": 1,
            "operation_id": operation.intent.operation_id,
            "phase": operation.phase,
            "machine_receipt": operation.machine_receipt.as_ref().map(|receipt| &receipt.outcome),
            "reconciliation_performed": true,
        }),
    )
}

/// Fresh local-Operator recovery of one exact historical installation. This
/// queries the Machine's original step and commits only a matching terminal
/// receipt. It never sends InstallPluginStep, resolves from current inventory,
/// or recreates the original confirmation.
pub(super) async fn confirmed_reconcile_install(
    state: Arc<AppState>,
    machine: String,
    plugin: String,
    operation_id: String,
    approval: OperatorApproval,
) -> Response {
    if !crate::plugin_operation::installation::valid_operation_id(&operation_id) {
        return (StatusCode::BAD_REQUEST, "Invalid Plugin operation identity").into_response();
    }
    let Some(store) = state.store.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let before = match store.plugin_install_operation(&operation_id).await {
        Ok(Some(operation))
            if operation.intent.service_id == state.service_id
                && operation.intent.machine_id == machine
                && operation.intent.plugin_id == plugin =>
        {
            operation
        }
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if !reconciliation_candidate(&before) {
        return (
            StatusCode::CONFLICT,
            "Installation has no exact terminal receipt recovery candidate",
        )
            .into_response();
    }
    let mut fence = match InstallationReconciliationFence::acquire(
        &state.plugin_lifecycle_fences,
        (machine.clone(), plugin.clone()),
    ) {
        Ok(fence) => fence,
        Err(_) => {
            return (
                StatusCode::CONFLICT,
                "Plugin lifecycle is changing or does not require reconciliation",
            )
                .into_response();
        }
    };
    let authority = match approval.bind_installation_reconciliation(&before) {
        Ok(authority) => authority,
        Err(_) => return StatusCode::CONFLICT.into_response(),
    };
    if !authority
        .check(
            ProductRequestAuth::from(state.as_ref()),
            &state.service_id,
            &before,
        )
        .await
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let receipt = if before.attention_from == Some(InstallPhase::MachineAcknowledged) {
        before
            .machine_receipt
            .clone()
            .expect("validated reconciliation candidate")
    } else {
        let connection = match state.machine_control.operation_connection(&machine) {
            Ok(connection) => connection,
            Err(_) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Machine is unavailable; installation remains fenced",
                )
                    .into_response();
            }
        };
        let step = match before.intent.machine_step() {
            Ok(step) => step,
            Err(_) => return StatusCode::CONFLICT.into_response(),
        };
        let observation = state
            .machine_control
            .plugin_installation_step(&connection, &step, None)
            .await;
        if !state.machine_control.is_current(&connection)
            || !authority
                .check(
                    ProductRequestAuth::from(state.as_ref()),
                    &state.service_id,
                    &before,
                )
                .await
        {
            return (
                StatusCode::CONFLICT,
                "Machine connection or Operator authority changed; installation remains fenced",
            )
                .into_response();
        }
        match observation {
            Ok(InstallObservation {
                result: InstallLookup::Found { receipt },
                ..
            }) if matches!(
                receipt.outcome,
                InstallOutcome::Applied { .. } | InstallOutcome::Rejected { .. }
            ) =>
            {
                *receipt
            }
            Ok(InstallObservation {
                result: InstallLookup::Found { .. },
                ..
            }) => {
                return (
                    StatusCode::CONFLICT,
                    "Machine installation outcome is still nonterminal; installation remains fenced",
                )
                    .into_response();
            }
            _ => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Exact Machine installation receipt is unavailable; installation remains fenced",
                )
                    .into_response();
            }
        }
    };
    if !authority
        .check(
            ProductRequestAuth::from(state.as_ref()),
            &state.service_id,
            &before,
        )
        .await
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let acknowledged = match commit_reconciled_receipt(store, &before, &receipt).await {
        Ok(operation) => operation,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if acknowledged.phase == InstallPhase::Aborted {
        let previous = matches!(
            before.intent.machine_target,
            Some(crate::machine_protocol::plugin_install::InstallTarget::Removed { .. })
        )
        .then_some(PluginFenceState::Uninstalled);
        fence.finish(previous);
        return reconciliation_response(&acknowledged);
    }

    let needs_auth = before.intent.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
        && state.provider_auth.status(&plugin).is_some();
    let auth_current = if needs_auth {
        match state.machine_control.operation_connection(&machine) {
            Ok(connection)
                if authority
                    .check(
                        ProductRequestAuth::from(state.as_ref()),
                        &state.service_id,
                        &before,
                    )
                    .await
                    && state.machine_control.is_current(&connection) =>
            {
                match provider_auth_envelope_for_machine(&state, &machine, &plugin).await {
                    Ok(envelope)
                        if authority
                            .check(
                                ProductRequestAuth::from(state.as_ref()),
                                &state.service_id,
                                &before,
                            )
                            .await
                            && state.machine_control.is_current(&connection) =>
                    {
                        state
                            .provider_auth_sync
                            .apply(&state.machine_control, &connection, envelope)
                            .await
                            .is_ok()
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    } else {
        true
    };
    let (phase, problem) = if auth_current {
        (InstallPhase::Completed, None)
    } else {
        (
            InstallPhase::AuthenticationPending,
            Some(InstallProblem::AuthenticationSyncFailed),
        )
    };
    if commit_reconciled_final_phase(store, &before.intent, phase, problem)
        .await
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    // The durable phase is terminal now. Release the process-local slot before
    // the final read so cancellation or a read outage cannot recreate an
    // in-memory reconciliation fence that startup would not reconstruct.
    fence.finish(None);
    let Some(completed) = store
        .plugin_install_operation(&operation_id)
        .await
        .ok()
        .flatten()
    else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    reconciliation_response(&completed)
}

pub(super) async fn api_machine_plugin_install(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin)): Path<(String, String)>,
    authenticated: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(request): Json<PluginInstallRequest>,
) -> Response {
    if let Some(refusal) = super::service_managed_refusal(&state, &machine) {
        return refusal;
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
    confirmed_install(state, machine, plugin, request, approval).await
}

pub(super) async fn confirmed_install(
    state: Arc<AppState>,
    machine: String,
    plugin: String,
    request: PluginInstallRequest,
    approval: OperatorApproval,
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
        Err(_) => return Outcome::NotDispatched(Precondition::MachineConnection).response(),
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
        // A Machine that answers with admission disabled is a different fact
        // from one that could not answer at all, and the two need different
        // fixes: update Cowboy Machine there, versus reconnect it.
        Ok(InstallTargetObservation::Observed {
            admission_enabled: false,
            ..
        }) => return Outcome::NotDispatched(Precondition::MachineAdmission).response(),
        _ => return Outcome::NotDispatched(Precondition::MachineTarget).response(),
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
