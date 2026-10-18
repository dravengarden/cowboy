//! Explicit maintenance and safe on-demand recovery of execution lifetimes. Native history
//! is retained. The durable intent is not a runnable binding on any reader.

use super::super::*;
use crate::execution_environment::{BindingV1, ExecutionBinding, RecoveryV1};
use crate::local_operator::Grant;
use crate::machine_protocol::execution::{Action, Request, Response};
use axum::extract::{Extension, Path, State};
use serde::Deserialize;
use serde_json::json;

/// A bounded optimization, never durable authority. A host reboot changes
/// the enrolled target connection; a changed binding cannot reuse a cache hit.
#[derive(Default)]
pub(in crate::server) struct Readiness {
    entries:
        parking_lot::Mutex<HashMap<String, (BindingV1, crate::machine_control::ConnectionToken)>>,
}

impl Readiness {
    fn ready(&self, control: &MachineControl, session: &str, binding: &BindingV1) -> bool {
        self.entries
            .lock()
            .get(session)
            .is_some_and(|(cached, connection)| cached == binding && control.is_current(connection))
    }

    fn remember(
        &self,
        control: &MachineControl,
        session: &str,
        binding: &BindingV1,
        connection: crate::machine_control::ConnectionToken,
    ) {
        if !control.is_current(&connection) {
            return;
        }
        let mut entries = self.entries.lock();
        // Eviction only costs another Describe, never permits a replacement.
        if entries.len() >= 512
            && !entries.contains_key(session)
            && let Some(old) = entries.keys().next().cloned()
        {
            entries.remove(&old);
        }
        entries.insert(session.into(), (binding.clone(), connection));
    }

    pub(super) fn forget(&self, session: &str, binding: &BindingV1) {
        let mut entries = self.entries.lock();
        if entries
            .get(session)
            .is_some_and(|(cached, _)| cached == binding)
        {
            entries.remove(session);
        }
    }
}

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
            "delivery":state.runtime_router.runtime(&m.machine_id).map(|runtime| runtime.delivery_diagnostics(&m.id)),
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
    match checked(&state, &session, Some(&grant), request).await {
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
    grant: Option<&Grant>,
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
    if grant.is_some()
        && !request.interrupt_active_turn
        && !already_committed
        && (state.hub.session_has_in_flight_prompt(session)
            || current.background_tasks > 0
            || current.status == Status::Busy)
    {
        return Err(
            "Session has unfinished work; explicitly request interruption or wait for idle".into(),
        );
    }
    if !authority_current(state, session, grant).await {
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
    if !already_committed && grant.is_some() {
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
    if !authority_current(state, session, grant).await {
        return Err("Local Operator authority changed; recovery remains fenced".into());
    }
    let target_connection = state.machine_control.operation_connection(machine)?;
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
        if !authority_current(state, session, grant).await {
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
    state.execution_readiness.remember(
        &state.machine_control,
        session,
        &binding,
        target_connection,
    );
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
        "outcome":"recovered","runtime_readiness":"observe_session","queue":if grant.is_some() {"paused_for_inspection"} else {"pending_delivery"}}),
        reload,
    ))
}

/// Dispatch owns one not-yet-delivered prompt. An idle surviving worker may
/// be replaced only after target-proven loss; an active turn or child may not.
async fn authority_current(state: &AppState, session: &str, grant: Option<&Grant>) -> bool {
    if let Some(grant) = grant {
        return grant.current();
    }
    if !idle_for_recovery(state, session) {
        return false;
    }
    let Some(store) = &state.store else {
        return false;
    };
    matches!(store.managed_call_counts(session).await, Ok((0, _)))
        && idle_for_recovery(state, session)
}

fn idle_for_recovery(state: &AppState, session: &str) -> bool {
    let Some(info) = state.hub.session_info(session) else {
        return false;
    };
    info.meta.background_tasks == 0
        && info.meta.status != Status::Busy
        && state
            .runtime_router
            .runtime(&info.meta.machine_id)
            .is_some_and(|runtime| replaceable_worker(&runtime.delivery_diagnostics(session)))
}

fn replaceable_worker(delivery: &serde_json::Value) -> bool {
    let worker = &delivery["worker"];
    delivery["connected"] == true
        && delivery["pending_prompts"]
            .as_array()
            .is_some_and(Vec::is_empty)
        && delivery["waiting_configuration"] == false
        && delivery["resetting"] == false
        && (worker.is_null()
            || (worker["current_turn_id"].is_null()
                && worker["pending_prompt_count"] == 0
                && worker["drain_requested"] == false
                && matches!(
                    worker["state"].as_str(),
                    Some("running" | "exited" | "crashed")
                )))
}

/// Describe is read-only. Only a target-proven dead lifetime authorizes a new
/// incarnation; link failures and uncertain commands remain fenced, never replayed.
pub(in crate::server) async fn before_dispatch(
    state: &Arc<AppState>,
    session: &str,
) -> Result<(), String> {
    use crate::execution_protocol::{Command, Response as KeeperResponse, Scope};
    let info = state
        .hub
        .session_info(session)
        .ok_or("Session no longer exists")?;
    let Some(record) = &info.meta.execution_binding else {
        return Ok(());
    };
    if record.managed().is_some() {
        return Ok(());
    }
    if !state.runtime_router.connected(&info.meta.machine_id) {
        return Err("runtime is not connected".into());
    }
    let target = record
        .decode()
        .ok()
        .map(|binding| binding.environment.machine_id)
        .or_else(|| {
            record
                .recovery()
                .map(|intent| intent.previous.environment.machine_id)
        });
    if !target.as_ref().is_some_and(|target| {
        state
            .project_placement
            .snapshot()
            .allows(&info.meta.machine_id, target)
    }) || !state
        .supervisor
        .runtime_allowed(&info.meta.provider, &info.meta.machine_id)
    {
        return Err("Machine policy does not permit execution dispatch".into());
    }
    let intent = if let Some(intent) = record.recovery() {
        if !intent.operation_id.starts_with("execution-wake-") {
            return Err("Explicit execution maintenance must finish before sending".into());
        }
        intent
    } else {
        let previous = record
            .decode()
            .map_err(|_| "Execution preparation is not ready")?;
        if state
            .runtime_router
            .runtime(&info.meta.machine_id)
            .is_some_and(|runtime| runtime.has_worker(session))
            && state
                .execution_readiness
                .ready(&state.machine_control, session, &previous)
        {
            return finish_ready_worker(state, session, &previous).await;
        }
        let target_connection = state
            .machine_control
            .operation_connection(&previous.environment.machine_id)?;
        let response = state
            .machine_control
            .execution_request(
                &previous.environment.machine_id,
                Request {
                    service_id: state.service_id.clone(),
                    machine_id: previous.environment.machine_id.clone(),
                    action: Action::Call {
                        session_id: session.into(),
                        binding: Box::new(previous.clone()),
                        command: Command::Describe,
                    },
                },
            )
            .await?;
        match response {
            Response::Call { response: KeeperResponse::Ready { scope, .. } } if scope == Scope::from_binding(&previous) => {
                state.execution_readiness.remember(&state.machine_control, session, &previous, target_connection);
                return finish_ready_worker(state, session, &previous).await;
            }
            Response::Refused { reason: crate::machine_protocol::execution::Refusal::EnvironmentLost } => {}
            _ => return Err("Execution target unavailable or identity changed; retained prompt was not delivered".into()),
        }
        RecoveryV1 {
            schema: 1,
            phase: "recovering".into(),
            operation_id: format!("execution-wake-{}-{}", previous.id, previous.revision),
            previous,
        }
    };
    finish_auto_recovery(state, session, intent).await
}

fn wake_intent(previous: BindingV1) -> RecoveryV1 {
    RecoveryV1 {
        schema: 1,
        phase: "recovering".into(),
        operation_id: format!("execution-wake-{}-{}", previous.id, previous.revision),
        previous,
    }
}

async fn finish_ready_worker(
    state: &Arc<AppState>,
    session: &str,
    binding: &BindingV1,
) -> Result<(), String> {
    let info = state
        .hub
        .session_info(session)
        .ok_or("Session no longer exists")?;
    let runtime = state
        .runtime_router
        .runtime(&info.meta.machine_id)
        .ok_or("runtime disconnected")?;
    if runtime.worker_matches_execution(session, binding) {
        return Ok(());
    }
    let Some(previous) = runtime.worker_execution_binding(session) else {
        return if runtime.has_worker(session) {
            Err("Worker execution identity is unknown; explicit recovery is required".into())
        } else {
            Ok(())
        };
    };
    let intent = wake_intent(previous);
    if !intent.accepts(binding) {
        return Err(
            "Worker and target do not identify the same recovery; inspect before replacing".into(),
        );
    }
    // A committed binding can outlive its Controller before worker replacement.
    // Observe the same target-owned receipt, then finish the original operation.
    finish_auto_recovery(state, session, intent).await
}

async fn finish_auto_recovery(
    state: &Arc<AppState>,
    session: &str,
    intent: RecoveryV1,
) -> Result<(), String> {
    {
        let mut maintenance = state.execution_closures.lock();
        if maintenance.contains_key(session) {
            return Err("Execution maintenance is already in progress".into());
        }
        maintenance.insert(session.into(), super::Maintenance::Recover);
    }
    struct Fence<'a>(&'a AppState, &'a str);
    impl Drop for Fence<'_> {
        fn drop(&mut self) {
            self.0.execution_closures.lock().remove(self.1);
        }
    }
    let fence = Fence(state, session);
    let request = RecoverRequest {
        intent: intent.clone(),
        interrupt_active_turn: false,
    };
    let (_, reload) = checked(state, session, None, request).await?;
    drop(fence);
    if reload {
        state
            .supervisor
            .recover_execution_session(session, intent)?;
    }
    tracing::info!(
        session,
        "recovered proven lost execution lifetime before prompt delivery"
    );
    Ok(())
}

#[cfg(test)]
mod dispatch_safety_tests {
    use super::*;

    #[test]
    fn surviving_old_worker_reconstructs_the_original_committed_wake_identity() {
        let previous = crate::execution_environment::fixture().decode().unwrap();
        let mut committed = previous.clone();
        committed.revision += 1;
        committed.environment.id = "55555555555555555555555555555555".into();
        committed.environment.incarnation = "44444444444444444444444444444444".into();
        let original = wake_intent(previous.clone());
        let after_controller_restart = wake_intent(previous);
        assert_eq!(original, after_controller_restart);
        assert!(after_controller_restart.accepts(&committed));
        committed.revision += 1;
        assert!(!after_controller_restart.accepts(&committed));
    }

    #[test]
    fn ready_binding_is_invalidated_by_reconnect_even_with_a_reused_epoch() {
        let control = MachineControl::new(crate::service_identity::ServiceIdentity::fixture());
        let readiness = Readiness::default();
        let binding: BindingV1 = serde_json::from_value(json!({
            "schema":1,"id":"11111111111111111111111111111111","revision":1,
            "runtime":{"machine_id":"runtime","cwd":"/runtime"},
            "environment":{"machine_id":"target","id":"22222222222222222222222222222222",
                "incarnation":"33333333333333333333333333333333",
                "executor_digest":format!("sha256:{}", "a".repeat(64)),"protocol":1},
            "workspace":{"id":"fixture","worktree_id":"sess-1","source_path":"/source","cwd":"/workspace"},
            "access":"project"
        })).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let first = control.install("target".into(), "same-epoch".into(), false, 4, tx);
        readiness.remember(&control, "sess-1", &binding, first.clone());
        assert!(readiness.ready(&control, "sess-1", &binding));
        let (tx, _rx) = mpsc::unbounded_channel();
        let second = control.install("target".into(), "same-epoch".into(), false, 4, tx);
        assert!(!readiness.ready(&control, "sess-1", &binding));
        let mut next = binding.clone();
        next.revision += 1;
        readiness.remember(&control, "sess-1", &next, second);
        readiness.remember(&control, "sess-1", &binding, first);
        assert!(readiness.ready(&control, "sess-1", &next));
        readiness.forget("sess-1", &binding);
        assert!(readiness.ready(&control, "sess-1", &next));
        readiness.forget("sess-1", &next);
        assert!(!readiness.ready(&control, "sess-1", &next));
    }

    #[test]
    fn only_observed_idle_workers_may_replace_a_lost_target() {
        let idle = json!({"connected":true,"pending_prompts":[],"waiting_configuration":false,
            "resetting":false,"worker":{"state":"running","current_turn_id":null,
            "pending_prompt_count":0,"drain_requested":false}});
        assert!(replaceable_worker(&idle));
        let mut cold = idle.clone();
        cold["worker"] = serde_json::Value::Null;
        assert!(replaceable_worker(&cold));
        for (field, value) in [
            ("current_turn_id", json!("turn-1")),
            ("pending_prompt_count", json!(1)),
            ("drain_requested", json!(true)),
            ("state", json!("starting")),
            ("state", json!("busy")),
            ("state", json!("unknown")),
        ] {
            let mut active = idle.clone();
            active["worker"][field] = value;
            assert!(!replaceable_worker(&active), "unsafe worker field {field}");
        }
        for (field, value) in [
            ("connected", json!(false)),
            ("pending_prompts", json!([{"written_to_connection":false}])),
            ("waiting_configuration", json!(true)),
            ("resetting", json!(true)),
        ] {
            let mut active = idle.clone();
            active[field] = value;
            assert!(
                !replaceable_worker(&active),
                "unsafe delivery field {field}"
            );
        }
        assert!(!replaceable_worker(&json!({})));
    }
}
