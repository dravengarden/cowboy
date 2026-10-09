//! Host-delegated, explicit recovery of one execution lifetime. Native history
//! is retained. The durable intent is not a runnable binding on any reader.

use super::super::*;
use crate::execution_environment::{ExecutionBinding, RecoveryV1};
use crate::local_operator::Grant;
use crate::machine_protocol::execution::{Action, Request, Response};
use axum::extract::{Extension, Path, State};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct RecoverRequest {
    intent: RecoveryV1,
    #[serde(default)]
    interrupt_active_turn: bool,
}

pub(in crate::server) async fn list(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(
        json!({"schema":1,"sessions":state.hub.session_list().into_iter()
        .filter(|m| m.execution_binding.is_some()).map(|m| json!({
            "id":m.id,"title":m.title,"status":m.status,"provider":m.provider,
            "provider_version":m.provider_version,"native_session_id":m.agent_session_id,
            "background_tasks":m.background_tasks,"execution_binding":m.execution_binding,
        })).collect::<Vec<_>>()}),
    )
}

pub(in crate::server) async fn plan(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
) -> axum::response::Response {
    let Some(info) = state.hub.session_info(&session) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(record) = info.meta.execution_binding else {
        return StatusCode::CONFLICT.into_response();
    };
    let intent = if let Some(intent) = record.recovery() {
        intent
    } else {
        let Ok(previous) = record.decode() else {
            return StatusCode::CONFLICT.into_response();
        };
        RecoveryV1 {
            schema: 1,
            phase: "recovering".into(),
            operation_id: format!("execution-recover-{}-{}", previous.id, previous.revision),
            previous,
        }
    };
    Json(json!({"schema":1,"session_id":session,"supported":state.machine_control.machine_supports(
        &intent.previous.environment.machine_id,crate::machine_protocol::EXECUTION_RECOVERY_PROTOCOL_VERSION)
        && state.machine_control.machine_supports(&info.meta.machine_id,crate::machine_protocol::EXECUTION_RECOVERY_PROTOCOL_VERSION),
        "request":{"intent":intent,"interrupt_active_turn":false},
        "native_session_id":info.meta.agent_session_id,"status":info.meta.status,
        "effect":"Stop this execution lifetime and its commands, retain history and worktree, resume with a new lifetime. Unknown commands are not replayed."})).into_response()
}

pub(in crate::server) async fn recover(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
    Extension(grant): Extension<Arc<Grant>>,
    Json(request): Json<RecoverRequest>,
) -> axum::response::Response {
    let intent = request.intent.clone();
    if !grant.current() {
        return StatusCode::FORBIDDEN.into_response();
    }
    {
        let mut maintenance = state.execution_closures.lock();
        if maintenance.contains_key(&session) {
            return (
                StatusCode::CONFLICT,
                "Execution maintenance is already in progress",
            )
                .into_response();
        }
        maintenance.insert(session.clone(), super::Maintenance::Recover);
    }
    struct Fence<'a>(&'a AppState, &'a str);
    impl Drop for Fence<'_> {
        fn drop(&mut self) {
            self.0.execution_closures.lock().remove(self.1);
        }
    }
    let fence = Fence(&state, &session);
    match checked(&state, &session, &grant, request).await {
        Ok((value, reload)) => {
            drop(fence);
            if reload
                && let Err(error) = state.supervisor.recover_execution_session(&session, intent)
            {
                return (StatusCode::CONFLICT, error).into_response();
            }
            Json(value).into_response()
        }
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

async fn checked(
    state: &Arc<AppState>,
    session: &str,
    grant: &Grant,
    request: RecoverRequest,
) -> Result<(serde_json::Value, bool), String> {
    let intent = request.intent;
    intent.validate()?;
    let machine = &intent.previous.environment.machine_id;
    if !state.machine_control.machine_supports(
        machine,
        crate::machine_protocol::EXECUTION_RECOVERY_PROTOCOL_VERSION,
    ) {
        return Err("Target Machine does not support explicit execution recovery".into());
    }
    let store = state.store.as_ref().ok_or("Durable storage unavailable")?;
    let current = state
        .hub
        .session_info(session)
        .ok_or("Session no longer exists")?
        .meta;
    if !state.machine_control.machine_supports(
        &current.machine_id,
        crate::machine_protocol::EXECUTION_RECOVERY_PROTOCOL_VERSION,
    ) {
        return Err("Runtime Machine does not support explicit execution recovery".into());
    }
    let expected = current
        .execution_binding
        .as_ref()
        .ok_or("No execution binding")?;
    let original = ExecutionBinding::from_record(
        serde_json::to_value(&intent.previous).map_err(|_| "Invalid original binding")?,
    );
    let pending = ExecutionBinding::from_record(
        serde_json::to_value(&intent).map_err(|_| "Invalid recovery intent")?,
    );
    let already_committed = expected.decode().is_ok_and(|b| intent.accepts(&b));
    if expected != &original && expected != &pending && !already_committed {
        return Err("Session binding changed; inspect the original operation".into());
    }
    original.for_runtime(&current.machine_id, &current.cwd)?;
    if !request.interrupt_active_turn
        && !already_committed
        && (state.hub.session_has_in_flight_prompt(session)
            || current.background_tasks > 0
            || current.status == Status::Busy)
    {
        return Err(
            "Session has unfinished work; explicitly request interruption or wait for idle".into(),
        );
    }
    if !grant.current() {
        return Err("Local Operator authority changed".into());
    }
    if expected == &original {
        if !store
            .commit_execution_binding(session, original.record(), pending.record())
            .await
            .map_err(|_| "Cannot persist execution recovery intent")?
        {
            return Err("Durable execution binding changed".into());
        }
        state
            .hub
            .accept_execution_recovery(session, &original, pending.clone())?;
    }
    if !already_committed {
        state.hub.set_paused(session, true);
        // No target effects can pass the intent fence. Cancel is allowed even
        // when launch is unavailable and does not start a replacement worker.
        let _ = state.supervisor.send(session, AgentCommand::Cancel);
        state.hub.set_status(
            session,
            Status::Interrupted,
            Some("Execution environment maintenance; history retained".into()),
        );
    }
    if !grant.current() {
        return Err("Local Operator authority changed; recovery remains fenced".into());
    }
    let response = state
        .machine_control
        .execution_request(
            machine,
            Request {
                service_id: state.service_id.clone(),
                machine_id: machine.clone(),
                action: Action::Recover {
                    session_id: session.into(),
                    intent: Box::new(intent.clone()),
                },
            },
        )
        .await?;
    let binding = match response {
        Response::Prepared { binding } => binding,
        Response::Refused { reason } => {
            return Err(format!(
                "Target recovery refused ({reason:?}); repeat only the saved request to observe it"
            ));
        }
        _ => return Err("Unexpected target recovery response".into()),
    };
    if !intent.accepts(&binding) {
        return Err("Target recovery returned a different binding".into());
    }
    let prepared = ExecutionBinding::from_record(
        serde_json::to_value(&binding).map_err(|_| "Invalid recovered binding")?,
    );
    if already_committed {
        if &prepared != expected {
            return Err("Recovery operation differs from the installed binding".into());
        }
    } else {
        if !grant.current() {
            return Err("Local Operator authority changed; target receipt retained".into());
        }
        let meta = state
            .hub
            .session_info(session)
            .ok_or("Session removed")?
            .meta;
        let mut candidate = meta.clone();
        candidate.execution_binding = Some(prepared.clone());
        candidate.require_runtime_launch()?;
        if !store
            .commit_execution_binding(session, pending.record(), prepared.record())
            .await
            .map_err(|_| "Cannot commit recovered execution binding")?
        {
            return Err("Durable recovery intent changed".into());
        }
        state
            .hub
            .accept_execution_recovery(session, &pending, prepared)?;
    }
    // Once the new binding is installed, identity validation itself fences the
    // old worker. Let the replacement's startup requests through immediately.
    // A lost reply after commit must not restart a healthy replacement. A
    // failed/disconnected reload remains explicitly retryable with this receipt.
    let observed = state
        .hub
        .session_info(session)
        .ok_or("Session removed")?
        .meta;
    let reload = !already_committed
        || !state
            .runtime_router
            .runtime(&observed.machine_id)
            .is_some_and(|runtime| runtime.worker_matches_execution(session, &binding))
        || matches!(
            observed.status,
            Status::Interrupted | Status::Crashed | Status::Exited
        );
    Ok((
        json!({"schema":1,"operation_id":intent.operation_id,"session_id":session,
        "execution_binding":binding,"native_session_id":current.agent_session_id,
        "outcome":"recovered","runtime_readiness":"observe_session","queue":"paused_for_inspection"}),
        reload,
    ))
}
