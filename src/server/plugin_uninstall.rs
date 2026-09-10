//! A finite, Service-owned uninstall coordinator. No generic DAG executor and
//! no automatic replay across Controller/Machine connection incarnations.

use super::*;
use crate::machine_control::{
    CommandFailure, CommandRequestError, ConnectionToken, PluginUninstallTransport,
};
use crate::machine_protocol::plugin_recovery::{RecoveryBasis, RecoveryObservation};
use crate::machine_protocol::plugin_step::{StepLookup, StepOutcome};
use crate::plugin_operation::{Actor, Phase, Problem, UninstallIntent};
use anyhow::{Result, ensure};

// Journal-aware reader floor 00e2b69b was activated before this descendant.
// Its rollback path keeps evidence/fences and pauses new uninstall admission.
const DURABLE_UNINSTALL_ENABLED: bool = true;
// Schema-two reader floor 6a420ff5 and Hawk's cold bootstrap are accepted.
const INSTALLATION_CAS_ENABLED: bool = true;

pub(super) fn requires_runtime_fence(command: &Inbound) -> bool {
    matches!(
        command,
        Inbound::Prompt { .. }
            | Inbound::Submit { .. }
            | Inbound::OpenSession { .. }
            | Inbound::ResetSession { .. }
            | Inbound::ResumeTurn { .. }
            | Inbound::RetryTurn { .. }
            | Inbound::SetConfigOption { .. }
            | Inbound::DeleteSession { .. }
    )
}

pub(super) fn request_actor(
    state: &AppState,
    authenticated: Option<Extension<AuthenticatedProductRequest>>,
    headers: &HeaderMap,
) -> Result<Actor, StatusCode> {
    // Match the middleware's precedence when both cookies are present.
    if let Ok(principal) =
        require_admin_role(&state.hub, headers, crate::admin::AdminRole::Operator)
    {
        return Ok(Actor::Admin {
            account: principal.account,
        });
    }
    authenticated
        .filter(|Extension(auth)| {
            auth.principal
                .role
                .at_least(crate::admin::AdminRole::Operator)
        })
        .map(|Extension(auth)| Actor::Product {
            user_id: auth.principal.user_id,
        })
        .ok_or(StatusCode::UNAUTHORIZED)
}

pub(super) async fn recover_fences(
    store: Option<&Store>,
    service: &str,
) -> Result<PluginLifecycleFences> {
    let mut fences = HashMap::new();
    if let Some(store) = store {
        for operation in store.recover_plugin_uninstalls(service).await? {
            fences.insert(
                (operation.intent.machine_id, operation.intent.plugin_id),
                PluginFenceState::NeedsReconcile,
            );
        }
    }
    tracing::info!(
        admission_enabled = DURABLE_UNINSTALL_ENABLED,
        installation_cas_enabled = INSTALLATION_CAS_ENABLED,
        fenced_slots = fences.len(),
        "Plugin uninstall journal recovered"
    );
    Ok(Arc::new(parking_lot::RwLock::new(fences)))
}

/// Once admitted, cancellation/panic may retain a fence but never drop it. The
/// persisted phase lets the next Controller reconstruct the same protection.
struct OperationFence {
    fences: PluginLifecycleFences,
    key: (String, String),
    keep: bool,
    finished: bool,
}

impl OperationFence {
    fn acquire(fences: &PluginLifecycleFences, key: (String, String)) -> Result<Self> {
        let mut active = fences.write();
        ensure!(
            !active.contains_key(&key),
            "Plugin lifecycle is changing or requires reconciliation"
        );
        active.insert(key.clone(), PluginFenceState::Uninstalling);
        Ok(Self {
            fences: Arc::clone(fences),
            key,
            keep: false,
            finished: false,
        })
    }

    fn finish(&mut self, phase: Phase) {
        let mut active = self.fences.write();
        match phase {
            Phase::Completed => {
                active.insert(self.key.clone(), PluginFenceState::Uninstalled);
            }
            Phase::Compensated | Phase::Aborted => {
                active.remove(&self.key);
            }
            _ => {
                active.insert(self.key.clone(), PluginFenceState::NeedsReconcile);
            }
        }
        self.finished = true;
    }
}

impl Drop for OperationFence {
    fn drop(&mut self) {
        if !self.finished {
            let mut active = self.fences.write();
            if self.keep {
                active.insert(self.key.clone(), PluginFenceState::NeedsReconcile);
            } else {
                active.remove(&self.key);
            }
        }
    }
}

async fn validate_intent(
    state: &Arc<AppState>,
    id: String,
    plan: PluginUninstallPlan,
) -> Result<UninstallIntent> {
    ensure!(now_ms() <= plan.expires_at_ms, "uninstall preview expired");
    ensure!(
        INSTALLATION_CAS_ENABLED || plan.installation_revision.is_none(),
        "installation CAS is reader-only"
    );
    let current = current_machine_plugin(state, &plan.machine_id, &plan.plugin_id)
        .await
        .map_err(anyhow::Error::msg)?;
    ensure!(
        current.generation_digest == plan.generation_digest
            && current.plugin_version == plan.plugin_version
            && current.contract_fingerprint == plan.contract_fingerprint,
        "Plugin release changed; refresh the uninstall plan"
    );
    ensure!(
        current.installation_revision == plan.installation_revision,
        "Plugin installation changed; refresh the uninstall plan"
    );
    let trusted = state.plugin_catalog.resolve(
        &plan.plugin_id,
        Some(&plan.plugin_version),
        Some(&plan.generation_digest),
    )?;
    ensure!(
        trusted.release.contract_fingerprint == plan.contract_fingerprint,
        "uninstall release is not trusted"
    );
    let mut sessions: Vec<_> = state
        .hub
        .session_list()
        .into_iter()
        .filter(|session| {
            current.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
                && session.machine_id == plan.machine_id
                && session.provider == plan.plugin_id
        })
        .collect();
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: Vec<_> = sessions.iter().map(|session| session.id.clone()).collect();
    let active: Vec<_> = sessions
        .iter()
        .filter(|session| provider_session_has_active_turn(session.status))
        .map(|session| session.id.clone())
        .collect();
    ensure!(
        ids == plan.session_ids && active == plan.active_session_ids,
        "uninstall impact changed; refresh the plan"
    );
    let live = ids
        .iter()
        .filter(|id| state.supervisor.has_live_worker(id))
        .cloned()
        .collect();
    let intent = UninstallIntent {
        schema: if plan.installation_revision.is_some() {
            2
        } else {
            1
        },
        operation_id: id,
        service_id: state.service_id.clone(),
        actor: plan.actor,
        machine_id: plan.machine_id,
        plugin_id: plan.plugin_id,
        plugin_version: plan.plugin_version,
        generation_digest: plan.generation_digest,
        installation_revision: plan.installation_revision,
        contract_fingerprint: plan.contract_fingerprint,
        session_ids: ids,
        active_session_ids: active,
        live_session_ids: live,
        purge_after_ms: plan.purge_after_ms,
        expires_at_ms: plan.expires_at_ms,
    };
    intent.validate()?;
    Ok(intent)
}

/// Closed effects used by this coordinator, injectable for crash-window tests.
/// No third-party implementation or deserialized intent can construct a port.
trait Effects: Sync {
    fn stop(&self, id: &str) -> bool;
    fn reload(&self, id: &str) -> Result<(), String>;
    fn uninstall(
        &self,
        intent: &UninstallIntent,
    ) -> impl std::future::Future<Output = Result<(), CommandRequestError>> + Send;
    fn reactivate(
        &self,
        intent: &UninstallIntent,
    ) -> impl std::future::Future<Output = Result<(), CommandRequestError>> + Send;
}

struct LiveEffects {
    state: Arc<AppState>,
    connection: ConnectionToken,
    transport: PluginUninstallTransport,
}

fn require_applied_step(result: StepLookup) -> Result<(), CommandRequestError> {
    let certainty = match result {
        StepLookup::Found { receipt } => match receipt.outcome {
            StepOutcome::Applied {} => return Ok(()),
            StepOutcome::Rejected { .. } => CommandFailure::Rejected,
            StepOutcome::Unknown { .. } => CommandFailure::Unknown,
        },
        StepLookup::NotFound {} | StepLookup::Unavailable { .. } => CommandFailure::Unknown,
    };
    Err(CommandRequestError {
        certainty,
        detail: "Machine uninstall requires receipt reconciliation".to_owned(),
    })
}

impl Effects for LiveEffects {
    fn stop(&self, id: &str) -> bool {
        self.state.supervisor.delete_session(id)
    }
    fn reload(&self, id: &str) -> Result<(), String> {
        self.state.supervisor.reload_session(id, true)
    }
    async fn uninstall(&self, intent: &UninstallIntent) -> Result<(), CommandRequestError> {
        if self.transport == PluginUninstallTransport::Leased {
            let step = intent.machine_step().map_err(|_| CommandRequestError {
                certainty: CommandFailure::NotSent,
                detail: "invalid Machine step".to_owned(),
            })?;
            let observation = self
                .state
                .machine_control
                .plugin_uninstall_step(&self.connection, &step, false)
                .await?;
            return require_applied_step(observation.result);
        }
        let request_id = machine_request_id("plugin-uninstall");
        self.state
            .machine_control
            .command_on_connection(
                &self.connection,
                request_id.clone(),
                crate::machine_protocol::MachineCommand::UninstallPlugin {
                    request_id,
                    plugin_id: intent.plugin_id.clone(),
                    generation_digest: intent.generation_digest.clone(),
                },
            )
            .await
    }
    async fn reactivate(&self, intent: &UninstallIntent) -> Result<(), CommandRequestError> {
        if self.transport == PluginUninstallTransport::Leased {
            // A durable forward receipt does not grant installation-CAS or
            // authorize an unjournaled inverse. Keep the recovery fence.
            return Err(CommandRequestError {
                certainty: CommandFailure::NotSent,
                detail: "durable compensation requires a separately verified recovery step"
                    .to_owned(),
            });
        }
        let request_id = machine_request_id("plugin-reactivate");
        self.state
            .machine_control
            .reactivate_plugin_on_connection(
                &self.connection,
                request_id,
                crate::machine_control::RetainedPluginTarget {
                    plugin_id: &intent.plugin_id,
                    version: &intent.plugin_version,
                    digest: &intent.generation_digest,
                    fingerprint: &intent.contract_fingerprint,
                },
            )
            .await
    }
}

async fn attention(
    store: &Store,
    intent: &UninstallIntent,
    from: Phase,
    problem: Problem,
) -> Result<Phase> {
    store
        .advance_plugin_uninstall(
            &intent.operation_id,
            from,
            Phase::NeedsAttention,
            Some(problem),
        )
        .await?;
    Ok(Phase::NeedsAttention)
}

async fn compensate(
    store: &Store,
    intent: &UninstallIntent,
    effects: &impl Effects,
    from: Phase,
    cause: Problem,
) -> Result<Phase> {
    // A CAS serializes with the delete transaction. An uncertain COMMIT that
    // actually succeeded can never be followed by reactivation from this path.
    if store
        .advance_plugin_uninstall(
            &intent.operation_id,
            from,
            Phase::RestoringMachine,
            Some(cause),
        )
        .await
        .is_err()
    {
        if store
            .plugin_uninstall_operation(&intent.operation_id)
            .await?
            .is_some_and(|op| op.phase == Phase::Completed)
        {
            return Ok(Phase::Completed);
        }
        anyhow::bail!("uninstall compensation could not establish its durable precondition");
    }
    if now_ms() > intent.expires_at_ms || effects.reactivate(intent).await.is_err() {
        return attention(
            store,
            intent,
            Phase::RestoringMachine,
            Problem::CompensationFailed,
        )
        .await;
    }
    store
        .advance_plugin_uninstall(
            &intent.operation_id,
            Phase::RestoringMachine,
            Phase::RestoringSessions,
            Some(cause),
        )
        .await?;
    for id in &intent.live_session_ids {
        if effects.reload(id).is_err() {
            return attention(
                store,
                intent,
                Phase::RestoringSessions,
                Problem::CompensationFailed,
            )
            .await;
        }
    }
    if !intent.live_session_ids.is_empty() {
        // reload_session acknowledges enqueue, not verified native-session
        // readiness. Do not release the fence on that weaker observation.
        return attention(
            store,
            intent,
            Phase::RestoringSessions,
            Problem::WorkerRecoveryUnverified,
        )
        .await;
    }
    store
        .advance_plugin_uninstall(
            &intent.operation_id,
            Phase::RestoringSessions,
            Phase::Compensated,
            Some(cause),
        )
        .await?;
    Ok(Phase::Compensated)
}

async fn execute(store: &Store, intent: &UninstallIntent, effects: &impl Effects) -> Result<Phase> {
    if now_ms() > intent.expires_at_ms {
        store
            .advance_plugin_uninstall(
                &intent.operation_id,
                Phase::Prepared,
                Phase::Aborted,
                Some(Problem::PreconditionsChanged),
            )
            .await?;
        return Ok(Phase::Aborted);
    }
    store
        .advance_plugin_uninstall(
            &intent.operation_id,
            Phase::Prepared,
            Phase::StoppingSessions,
            None,
        )
        .await?;
    for id in &intent.session_ids {
        if effects.stop(id) != intent.live_session_ids.contains(id) {
            return attention(
                store,
                intent,
                Phase::StoppingSessions,
                Problem::PreconditionsChanged,
            )
            .await;
        }
    }
    store
        .advance_plugin_uninstall(
            &intent.operation_id,
            Phase::StoppingSessions,
            Phase::Uninstalling,
            None,
        )
        .await?;
    match effects.uninstall(intent).await {
        Ok(()) => {}
        Err(error) => {
            return match error.certainty {
                CommandFailure::Rejected => {
                    compensate(
                        store,
                        intent,
                        effects,
                        Phase::Uninstalling,
                        Problem::MachineRejected,
                    )
                    .await
                }
                CommandFailure::NotSent => {
                    attention(
                        store,
                        intent,
                        Phase::Uninstalling,
                        Problem::MachineUnavailable,
                    )
                    .await
                }
                CommandFailure::Unknown => {
                    attention(
                        store,
                        intent,
                        Phase::Uninstalling,
                        Problem::UnknownMachineOutcome,
                    )
                    .await
                }
            };
        }
    }
    store
        .advance_plugin_uninstall(
            &intent.operation_id,
            Phase::Uninstalling,
            Phase::MachineUninstalled,
            None,
        )
        .await?;
    if now_ms() > intent.expires_at_ms {
        return attention(
            store,
            intent,
            Phase::MachineUninstalled,
            Problem::PreconditionsChanged,
        )
        .await;
    }
    match store.commit_plugin_uninstall(intent).await {
        Ok(()) => Ok(Phase::Completed),
        Err(_) => {
            compensate(
                store,
                intent,
                effects,
                Phase::MachineUninstalled,
                Problem::StorageFailure,
            )
            .await
        }
    }
}

async fn run_admitted(
    state: Arc<AppState>,
    id: String,
    plan: PluginUninstallPlan,
    mut fence: OperationFence,
) -> Response {
    let result = async {
        let store = state.store.as_ref().context("Plugin lifecycle requires persistence")?;
        let connection = state.machine_control.operation_connection(&plan.machine_id).map_err(anyhow::Error::msg)?;
        let intent = validate_intent(&state, id.clone(), plan).await?;
        let step = intent.machine_step()?;
        let transport = state.machine_control.plugin_uninstall_transport(&connection, &step).map_err(anyhow::Error::msg)?;
        if transport == PluginUninstallTransport::Leased {
            let observation = state.machine_control.plugin_uninstall_step(&connection, &step, true)
                .await.map_err(|_| anyhow::anyhow!("Machine operation preflight unavailable"))?;
            ensure!(observation.admission_enabled && observation.result == StepLookup::NotFound {},
                "Machine durable uninstall is not admitting a new step; no workers were stopped");
        }
        // Even an intent COMMIT error can be ambiguous. Keep the memory fence;
        // startup either finds the record or safely forgets a no-effect attempt.
        fence.keep = true;
        store.begin_plugin_uninstall(&intent).await?;
        let effects = LiveEffects { state: Arc::clone(&state), connection, transport };
        let phase = match execute(store, &intent, &effects).await {
            Ok(phase) => phase,
            Err(_) => {
                if let Some(op) = store.plugin_uninstall_operation(&id).await? {
                    if op.phase.terminal() || op.phase == Phase::NeedsAttention { op.phase } else {
                        attention(store, &intent, op.phase, Problem::StorageFailure).await?
                    }
                } else { anyhow::bail!("uninstall journal unavailable"); }
            }
        };
        fence.finish(phase);
        if phase == Phase::Completed {
            for session_id in &intent.session_ids { state.hub.detach_session(session_id); }
            return Ok(Json(serde_json::json!({
                "operation_id": id, "phase": phase, "provider_id": intent.plugin_id,
                "machine_id": intent.machine_id, "deleted_session_ids": intent.session_ids,
                "purge_after_ms": intent.purge_after_ms,
            })).into_response());
        }
        Ok((StatusCode::CONFLICT, format!("Plugin uninstall operation {id}: {}. Inspect this target's operations before retrying.", phase.as_str())).into_response())
    }.await;
    result.unwrap_or_else(|_: anyhow::Error| {
        // No Machine error strings, payloads, policy files or credentials enter
        // the journal/diagnostic response. The operation identity is sufficient.
        (StatusCode::CONFLICT, format!("Plugin uninstall operation {id} could not finish; inspect its operations and reconciliation state.")).into_response()
    })
}

pub(super) async fn api_machine_plugin_uninstall(
    State(state): State<Arc<AppState>>,
    Path((machine_id, provider_id)): Path<(String, String)>,
    authenticated: Option<Extension<AuthenticatedProductRequest>>,
    headers: HeaderMap,
    Json(request): Json<PluginUninstallRequest>,
) -> Response {
    if !DURABLE_UNINSTALL_ENABLED {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Plugin uninstall is paused while its durable recovery reader floor is activated",
        )
            .into_response();
    }
    let actor = match request_actor(&state, authenticated, &headers) {
        Ok(actor) => actor,
        Err(status) => return status.into_response(),
    };
    let plan = match consume_preview(
        &mut state.plugin_uninstall_plans.lock(),
        &actor,
        &machine_id,
        &provider_id,
        &request,
        now_ms(),
    ) {
        Ok(plan) => plan,
        Err(error) => return (StatusCode::CONFLICT, error).into_response(),
    };
    let fence =
        match OperationFence::acquire(&state.plugin_lifecycle_fences, (machine_id, provider_id)) {
            Ok(fence) => fence,
            Err(error) => return (StatusCode::CONFLICT, error.to_string()).into_response(),
        };
    // The JoinHandle is an observer. Dropping the HTTP future does not cancel
    // the admitted operation; runtime shutdown leaves durable progress fenced.
    match tokio::spawn(run_admitted(state, request.plan_id, plan, fence)).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Plugin lifecycle interrupted; reconciliation is required",
        )
            .into_response(),
    }
}

fn consume_preview(
    plans: &mut HashMap<String, PluginUninstallPlan>,
    actor: &Actor,
    machine: &str,
    plugin: &str,
    request: &PluginUninstallRequest,
    now: i64,
) -> Result<PluginUninstallPlan, &'static str> {
    let plan = plans
        .get(&request.plan_id)
        .ok_or("uninstall preview missing or consumed; inspect operations before retrying")?;
    if &plan.actor != actor
        || plan.machine_id != machine
        || plan.plugin_id != plugin
        || plan.expires_at_ms < now
    {
        return Err("uninstall preview owner, target or expiry changed");
    }
    if !plan.active_session_ids.is_empty() && !request.confirm_active_sessions {
        return Err("active sessions require an explicit second confirmation");
    }
    Ok(plans
        .remove(&request.plan_id)
        .expect("validated under the same lock"))
}

pub(super) async fn api_machine_plugin_operations(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin)): Path<(String, String)>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match store.plugin_uninstall_history(&state.service_id, &machine, &plugin).await {
        Ok(operations) => Json(serde_json::json!({
            "admission_enabled": DURABLE_UNINSTALL_ENABLED,
            "requires_reconciliation": state.plugin_lifecycle_fences.read().get(&(machine, plugin)) == Some(&PluginFenceState::NeedsReconcile),
            "operations": operations.into_iter().map(|op| serde_json::json!({
                "operation_id": op.intent.operation_id, "phase": op.phase, "problem": op.problem,
                "attention_from": op.attention_from,
                "cause": op.cause,
                "plugin_version": op.intent.plugin_version, "generation_digest": op.intent.generation_digest,
                "affected_session_count": op.intent.session_ids.len(), "purge_after_ms": op.intent.purge_after_ms,
                "created_at_ms": op.created_at_ms, "updated_at_ms": op.updated_at_ms,
            })).collect::<Vec<_>>(),
        })).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "Plugin operation evidence is unavailable").into_response(),
    }
}

#[cfg(test)]
mod tests;

/// Fresh authorized observation only. Never execute, advance the journal,
/// soft-delete a session, or clear a fence based on a remote query.
pub(super) async fn api_machine_plugin_operation_receipt(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin, operation)): Path<(String, String, String)>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let intent = match store.plugin_uninstall_operation(&operation).await {
        Ok(Some(op))
            if op.intent.service_id == state.service_id
                && op.intent.machine_id == machine
                && op.intent.plugin_id == plugin =>
        {
            op.intent
        }
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let result = async {
        let connection = state
            .machine_control
            .operation_connection(&machine)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            state.machine_control.durable_plugin_steps(&connection),
            "Machine lacks durable step queries"
        );
        let observation = state
            .machine_control
            .plugin_uninstall_step(&connection, &intent.machine_step()?, true)
            .await
            .map_err(|_| anyhow::anyhow!("Machine receipt query failed"))?;
        Ok::<_, anyhow::Error>(observation)
    }
    .await;
    match result {
        Ok(observation) => Json(serde_json::json!({
            "operation_id": operation,
            "machine_observation": observation,
            "reconciliation_performed": false,
        }))
        .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Machine durable receipt is unavailable; operation remains unchanged",
        )
            .into_response(),
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum RecoveryRequirement {
    FreshPolicyAndAuthAuthority,
    RetainedArtifactsAndProbes,
    BoundedExecutionLease,
    ExactSessionAndWorkerRestoration,
}

#[derive(serde::Serialize)]
struct RecoveryAssessment {
    schema: u16,
    operation_id: String,
    service_phase: Phase,
    service_problem: Option<Problem>,
    service_cause: Option<Problem>,
    attention_from: Option<Phase>,
    affected_session_count: usize,
    machine_observation: RecoveryObservation,
    basis: RecoveryBasis,
    recovery_execution_available: bool,
    reconciliation_performed: bool,
    not_verified: [RecoveryRequirement; 4],
}

async fn inspect_recovery(
    store: &Store,
    operation: crate::plugin_operation::Operation,
    control: &MachineControl,
    connection: &ConnectionToken,
) -> Result<RecoveryAssessment> {
    let step = operation.intent.machine_step()?;
    let observation = control
        .plugin_uninstall_recovery(connection, &step)
        .await
        .map_err(|_| anyhow::anyhow!("Machine recovery observation unavailable"))?;
    // Service and Machine are separate transaction domains. Detect a local
    // change while awaiting the snapshot; do not claim cross-site atomicity.
    ensure!(
        store
            .plugin_uninstall_operation(&operation.intent.operation_id)
            .await?
            .as_ref()
            == Some(&operation),
        "Service operation changed during recovery observation"
    );
    Ok(RecoveryAssessment {
        schema: 1,
        basis: observation.basis(&step),
        machine_observation: observation,
        operation_id: operation.intent.operation_id,
        service_phase: operation.phase,
        service_problem: operation.problem,
        service_cause: operation.cause,
        attention_from: operation.attention_from,
        affected_session_count: operation.intent.session_ids.len(),
        recovery_execution_available: false,
        reconciliation_performed: false,
        not_verified: [
            RecoveryRequirement::FreshPolicyAndAuthAuthority,
            RecoveryRequirement::RetainedArtifactsAndProbes,
            RecoveryRequirement::BoundedExecutionLease,
            RecoveryRequirement::ExactSessionAndWorkerRestoration,
        ],
    })
}

/// Existing Operator middleware authorizes observation only. No supplied intent,
/// mutation flag, credentials, activation, worker reload or journal transition.
pub(super) async fn api_machine_plugin_recovery_assessment(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin, operation)): Path<(String, String, String)>,
) -> Response {
    let Some(store) = state.store.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let operation = match store.plugin_uninstall_operation(&operation).await {
        Ok(Some(op))
            if op.intent.service_id == state.service_id
                && op.intent.machine_id == machine
                && op.intent.plugin_id == plugin =>
        {
            op
        }
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Ok(connection) = state.machine_control.operation_connection(&machine) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match inspect_recovery(store, operation, &state.machine_control, &connection).await {
        Ok(assessment) => Json(assessment).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Recovery assessment unavailable; this query made no changes",
        )
            .into_response(),
    }
}
