//! One Controller task drives each accepted call through the existing
//! Machine, supervisor and Hub. Every transition is a durable CAS; a lost
//! launch reply or Controller restart observes the original child instead
//! of minting another one, and a completion that races a stop is retained.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::super::AppState;
use super::coordinator::{child_runtime, execution_action, runtime_readiness};
use crate::core::Status;
use crate::execution_environment::{ExecutionBinding, ManagedChildV1};
use crate::machine_protocol::execution;
use crate::managed_calls::lifecycle::{CallError, Record, State};
use crate::managed_calls::protocol::ChildRound;
use crate::managed_calls::{Conversation, OutputFormat, Request};
use crate::store::Store;

const POLL: Duration = Duration::from_millis(500);
/// A launch whose target never answers fails instead of waiting forever.
const LAUNCH_DEADLINE_MS: i64 = 10 * 60 * 1000;
/// Native cancellation grace before the exact child worker is stopped.
const STOP_GRACE_MS: i64 = 60 * 1000;
/// A recovered turn without any trace of its prompt is resubmitted.
const PROMPT_RECOVERY_MS: i64 = 15 * 1000;
const MAX_RESULT_TEXT: usize = 900 * 1024;

enum Step {
    Again,
    Wait(Duration),
    Done,
}

pub(super) fn ensure_runner(state: &Arc<AppState>, record: &Record) {
    if record.state.terminal() {
        return;
    }
    if !state
        .managed_calls
        .runners
        .lock()
        .insert(record.call_id.clone())
    {
        return;
    }
    let state = Arc::clone(state);
    let parent = record.placement.parent_session_id.clone();
    let call = record.call_id.clone();
    tokio::spawn(async move {
        drive(&state, &parent, &call).await;
        state.managed_calls.runners.lock().remove(&call);
        state.managed_calls.notify();
    });
}

/// Resume every accepted, non-terminal call after a Controller start.
pub(super) fn recover(state: &Arc<AppState>) {
    let state = Arc::clone(state);
    tokio::spawn(async move {
        // Machines reconnect and the Hub restores sessions shortly after
        // start; observations before that would misread a live child.
        tokio::time::sleep(Duration::from_secs(10)).await;
        let Some(store) = state.store.clone() else {
            return;
        };
        match store.active_managed_calls().await {
            Ok(records) => {
                for record in records {
                    ensure_runner(&state, &record);
                }
            }
            Err(error) => tracing::warn!(%error, "managed call recovery unavailable"),
        }
    });
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn advance(
    state: &AppState,
    store: &Store,
    previous: &Record,
    change: impl FnOnce(&mut Record),
) -> bool {
    let mut next = previous.clone();
    change(&mut next);
    next.revision = previous.revision + 1;
    next.updated_at_ms = now_ms().max(previous.updated_at_ms);
    match store.advance_managed_call(previous, &next).await {
        Ok(advanced) => {
            if advanced {
                state.managed_calls.notify();
            }
            advanced
        }
        Err(error) => {
            tracing::warn!(%error, call = %previous.call_id, "managed call transition refused");
            false
        }
    }
}

fn fail(code: &'static str, detail: Option<String>) -> impl FnOnce(&mut Record) {
    move |record| {
        record.state = State::Failed;
        record.error = Some(CallError::new(code, detail));
    }
}

async fn drive(state: &Arc<AppState>, parent: &str, call: &str) {
    let Some(store) = state.store.clone() else {
        return;
    };
    let mut misses = 0;
    loop {
        if *state.shutdown.borrow() {
            return;
        }
        let record = match store.managed_call(parent, call).await {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(_) => {
                misses += 1;
                if misses > 120 {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if record.state.terminal() {
            settle(state, &record);
            return;
        }
        // A deleted or closing parent stops its owned active calls.
        if record.cancel_requested_at_ms.is_none()
            && state
                .hub
                .session_info(parent)
                .is_none_or(|info| info.meta.closing)
        {
            let requested = now_ms();
            let cancelled = advance(state, &store, &record, |next| {
                next.cancel_requested_at_ms = Some(requested);
                if next.state == State::Queued {
                    next.state = State::Cancelled;
                }
            })
            .await;
            if !cancelled {
                tokio::time::sleep(POLL).await;
            }
            continue;
        }
        let step = match record.state {
            State::Queued => claim(state, &store, &record).await,
            State::Starting => launch(state, &store, &record).await,
            State::Running | State::WaitingInput | State::Stopping => {
                observe(state, &store, &record).await
            }
            State::Completed | State::Failed | State::Cancelled => Step::Done,
        };
        match step {
            Step::Again => {}
            Step::Wait(duration) => tokio::time::sleep(duration).await,
            Step::Done => return,
        }
    }
}

async fn claim(state: &Arc<AppState>, store: &Store, record: &Record) -> Step {
    if record.cancel_requested_at_ms.is_some() {
        advance(state, store, record, |next| next.state = State::Cancelled).await;
        return Step::Again;
    }
    let Some(parent) = state.hub.session_info(&record.placement.parent_session_id) else {
        advance(state, store, record, fail("parent_unavailable", None)).await;
        return Step::Again;
    };
    let Ok(ledger) = crate::managed_calls::service::Ledger::for_parent(
        store.clone(),
        &state.service_id,
        &parent.meta,
    ) else {
        advance(state, store, record, fail("parent_unavailable", None)).await;
        return Step::Again;
    };
    let (runtime, version, digest) = match state.hub.session_info(&record.child_session_id) {
        // A continued conversation keeps its own runtime and exact Provider
        // generation.
        Some(child) => (
            child.meta.machine_id.clone(),
            child.meta.provider_version.clone(),
            child.meta.provider_generation_digest.clone(),
        ),
        None => {
            let (runtime, readiness) = child_runtime(
                state,
                &record.placement.machine_id,
                Some(parent.meta.machine_id.as_str()),
                &record.provider,
            )
            .await;
            match readiness.generation {
                Some(generation) if readiness.available => {
                    (runtime, generation.version, generation.digest)
                }
                _ => {
                    advance(
                        state,
                        store,
                        record,
                        fail(readiness.reason.unwrap_or("provider_unavailable"), None),
                    )
                    .await;
                    return Step::Again;
                }
            }
        }
    };
    match ledger
        .claim(&record.call_id, &runtime, &version, &digest)
        .await
    {
        Ok(_) => {
            state.managed_calls.notify();
            Step::Again
        }
        Err(_) => Step::Wait(Duration::from_secs(1)),
    }
}

fn output_schema(request: &Request) -> Option<Value> {
    match &request.output {
        OutputFormat::Text {} => None,
        OutputFormat::JsonSchema { schema } => Some(schema.clone()),
    }
}

/// Stop a live continued child before its workspace is refreshed in place.
async fn quiesce(state: &AppState, child: &str) -> Result<(), &'static str> {
    if !state.supervisor.has_live_worker(child) {
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        match state.supervisor.hibernate_session(child) {
            Ok(()) => break,
            Err(detail) if detail.contains("already hibernated") => return Ok(()),
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(_) => return Err("conversation_busy"),
        }
    }
    while state.supervisor.has_live_worker(child) {
        if tokio::time::Instant::now() >= deadline {
            return Err("conversation_busy");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(())
}

async fn launch(state: &Arc<AppState>, store: &Store, record: &Record) -> Step {
    if record.cancel_requested_at_ms.is_some() {
        // The launch outcome may already exist; observe it while stopping.
        advance(state, store, record, |next| next.state = State::Stopping).await;
        return Step::Again;
    }
    if now_ms() - record.created_at_ms > LAUNCH_DEADLINE_MS {
        advance(state, store, record, fail("launch_timeout", None)).await;
        return Step::Again;
    }
    let request = match store
        .managed_call_input(&record.placement.parent_session_id, &record.call_id)
        .await
    {
        Ok(Some(request)) => request,
        _ => return Step::Wait(Duration::from_secs(1)),
    };
    let child = record.child_session_id.as_str();
    let existing = state.hub.session_info(child);
    let existing_workspace = existing.as_ref().and_then(|existing| {
        child_workspace(&existing.meta)
            .filter(|(parent, machine, _)| {
                *parent == record.placement.parent_session_id
                    && *machine == record.placement.machine_id
            })
            .map(|(_, _, workspace)| workspace)
    });
    if existing.is_some() {
        if existing_workspace.is_none() {
            advance(state, store, record, fail("identity_mismatch", None)).await;
            return Step::Again;
        }
        if let Err(code) = quiesce(state, child).await {
            advance(state, store, record, fail(code, None)).await;
            return Step::Again;
        }
    } else if matches!(request.conversation, Conversation::Continue { .. }) {
        advance(state, store, record, fail("conversation_unavailable", None)).await;
        return Step::Again;
    }
    let round = ChildRound {
        child_session_id: child.to_owned(),
        parent_session_id: record.placement.parent_session_id.clone(),
        call_id: record.call_id.clone(),
        workspace_id: record.placement.workspace_id.clone(),
        source_cwd: request
            .context
            .root
            .clone()
            .unwrap_or_else(|| record.placement.cwd.clone()),
        files: request.context.files.clone(),
        output_schema: output_schema(&request),
        profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
    };
    let prepared = match execution_action(
        state,
        &record.placement.machine_id,
        execution::Action::PrepareManagedRound {
            round: Box::new(round),
        },
    )
    .await
    {
        Ok(execution::Response::ManagedRound { prepared }) => prepared,
        Ok(execution::Response::ManagedRoundRefused { code }) => {
            let code = match code.as_str() {
                "input_changed" => "input_changed",
                "child_busy" => return Step::Wait(Duration::from_secs(1)),
                "submodule_unsupported"
                | "symlink_unsupported"
                | "filter_unsupported"
                | "subdirectory_unsupported"
                | "context_unsupported"
                | "index_unsupported"
                | "unborn_head" => "unsupported_input",
                "context_unreadable" => "context_unreadable",
                _ => "snapshot_unavailable",
            };
            advance(state, store, record, fail(code, None)).await;
            return Step::Again;
        }
        Ok(_) => {
            advance(state, store, record, fail("snapshot_unavailable", None)).await;
            return Step::Again;
        }
        // Retrying the same call id observes the same target receipt.
        Err(_) => return Step::Wait(Duration::from_secs(5)),
    };
    if existing.is_none() {
        match register_child(state, record, &prepared.cwd).await {
            Ok(()) => {}
            // Both preparations are idempotent; repeat them on the next pass.
            Err(RETRY) => return Step::Wait(Duration::from_secs(5)),
            Err(code) => {
                advance(state, store, record, fail(code, None)).await;
                return Step::Again;
            }
        }
    } else if existing_workspace.as_deref() != Some(prepared.cwd.as_str()) {
        advance(state, store, record, fail("identity_mismatch", None)).await;
        return Step::Again;
    }
    let Some(cursor) = state.hub.last_event_seq(child) else {
        return Step::Wait(Duration::from_secs(1));
    };
    let revision = prepared.input_revision.clone();
    // Record the cursor before the prompt: recovery then either observes the
    // prompt or resubmits it under the same idempotent message id.
    if advance(state, store, record, |next| {
        next.state = State::Running;
        next.child_cursor = Some(cursor);
        next.input_revision = Some(revision);
    })
    .await
    {
        state
            .hub
            .submit_managed_prompt(child, &record.call_id, request.instruction.clone());
    }
    Step::Again
}

/// Record the policy's preset as the child's configuration preferences.
/// Every worker start replays them before its first prompt, so each round of
/// a continued conversation keeps the same model and reasoning. A preset
/// missing from the exact generation is refused rather than guessed.
fn apply_preset(
    state: &AppState,
    record: &Record,
    version: &str,
    digest: &str,
) -> Result<(), &'static str> {
    let Some(preset) = &record.preset else {
        return Ok(());
    };
    let values = state
        .provider_catalog
        .package(&record.provider, version, digest)
        .and_then(|package| {
            package
                .manifest
                .configuration
                .presets
                .into_iter()
                .find(|candidate| candidate.id == *preset)
        })
        .ok_or("preset_unavailable")?
        .values;
    for (config_id, value) in values {
        state
            .hub
            .set_config_preference(&record.child_session_id, config_id, json!(value))
            .map_err(|_| "preset_unavailable")?;
    }
    Ok(())
}

/// A transient preparation failure; the same launch is repeated.
const RETRY: &str = "retry";

/// The parent, execution Machine and snapshot workspace of a managed child,
/// whether its runtime is that Machine's or another's.
fn child_workspace(meta: &crate::core::SessionMeta) -> Option<(String, String, String)> {
    let binding = meta.execution_binding.as_ref()?;
    if let Some(local) = binding.managed_child() {
        return Some((local.parent_session_id, local.machine_id, local.cwd));
    }
    let remote = binding.decode().ok()?;
    let managed = remote.managed?;
    (remote.workspace.worktree_id == meta.id).then_some((
        managed.parent_session_id,
        remote.environment.machine_id,
        remote.workspace.cwd,
    ))
}

fn local_binding(record: &Record, cwd: &str) -> Result<ExecutionBinding, &'static str> {
    let identity = ManagedChildV1 {
        schema: 1,
        phase: "managed_child".into(),
        session_id: record.child_session_id.clone(),
        parent_session_id: record.placement.parent_session_id.clone(),
        machine_id: record.placement.machine_id.clone(),
        workspace_id: record.placement.workspace_id.clone(),
        cwd: cwd.to_owned(),
        profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
    };
    identity.validate().map_err(|_| "identity_mismatch")?;
    Ok(ExecutionBinding::from_record(
        serde_json::to_value(&identity).map_err(|_| "identity_mismatch")?,
    ))
}

/// Prepare the runtime Machine's private entry, then the execution Machine's
/// read-only environment over the child's snapshot. Both are idempotent.
async fn remote_binding(
    state: &AppState,
    record: &Record,
    runtime: &str,
    cwd: &str,
) -> Result<ExecutionBinding, &'static str> {
    let child = &record.child_session_id;
    let location = match execution_action(
        state,
        runtime,
        execution::Action::PrepareRuntime {
            session_id: child.clone(),
        },
    )
    .await
    {
        Ok(execution::Response::RuntimePrepared { runtime: location })
            if location.machine_id == runtime =>
        {
            location
        }
        Ok(execution::Response::Refused { .. } | execution::Response::RuntimePrepared { .. }) => {
            return Err("runtime_unavailable");
        }
        _ => return Err(RETRY),
    };
    let binding = match execution_action(
        state,
        &record.placement.machine_id,
        execution::Action::PrepareManagedEnvironment {
            child_session_id: child.clone(),
            runtime: location.clone(),
        },
    )
    .await
    {
        Ok(execution::Response::Prepared { binding }) => binding,
        Ok(execution::Response::Refused { .. }) => return Err("environment_unavailable"),
        _ => return Err(RETRY),
    };
    let expected = crate::execution_environment::ManagedBindingV1 {
        parent_session_id: record.placement.parent_session_id.clone(),
        profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
    };
    if binding.validate().is_err()
        || binding.managed.as_ref() != Some(&expected)
        || binding.runtime != location
        || binding.environment.machine_id != record.placement.machine_id
        || binding.workspace.worktree_id != *child
        || binding.workspace.cwd != cwd
    {
        return Err("identity_mismatch");
    }
    Ok(ExecutionBinding::from_record(
        serde_json::to_value(&binding).map_err(|_| "identity_mismatch")?,
    ))
}

async fn register_child(state: &AppState, record: &Record, cwd: &str) -> Result<(), &'static str> {
    let parent = state
        .hub
        .session_info(&record.placement.parent_session_id)
        .ok_or("parent_unavailable")?;
    let (Some(version), Some(digest)) =
        (&record.provider_version, &record.provider_generation_digest)
    else {
        return Err("provider_unavailable");
    };
    // The runtime chosen when the call was claimed; never re-selected.
    let runtime = record
        .runtime_machine_id
        .clone()
        .ok_or("provider_unavailable")?;
    let readiness = runtime_readiness(
        state,
        &record.placement.machine_id,
        &runtime,
        &record.provider,
    )
    .await;
    let generation = readiness
        .generation
        .filter(|generation| generation.version == *version && generation.digest == *digest)
        .ok_or("provider_changed")?;
    let remote = runtime != record.placement.machine_id;
    let binding = if remote {
        remote_binding(state, record, &runtime, cwd).await?
    } else {
        local_binding(record, cwd)?
    };
    // A split child's worker runs in its runtime Machine's private entry.
    let session_cwd = match binding.decode() {
        Ok(remote) => remote.runtime.cwd,
        Err(_) => cwd.to_owned(),
    };
    let workspaces = state
        .store
        .as_ref()
        .ok_or("capacity")?
        .list_machines()
        .await
        .map_err(|_| "capacity")?
        .into_iter()
        .find(|machine| machine.id == record.placement.machine_id)
        .and_then(|machine| machine.inventory.get("workspaces").cloned())
        .and_then(|value| {
            serde_json::from_value::<Vec<crate::machine_protocol::MachineWorkspace>>(value).ok()
        })
        .unwrap_or_default();
    let workspace = workspaces
        .iter()
        .find(|workspace| workspace.id == record.placement.workspace_id);
    let owner =
        parent
            .meta
            .owner_user_id
            .as_deref()
            .map(|user_id| crate::supervisor::SessionOwner {
                user_id,
                username: parent.meta.owner_username.as_deref(),
            });
    state
        .provider_auth
        .with_scheduling_generation(
            &record.provider,
            generation.auth_generation.is_some(),
            generation.auth_generation,
            || {
                state.supervisor.register_session_on_with_id(
                    &record.child_session_id,
                    &record.provider,
                    Some(session_cwd.clone()),
                    crate::core::SessionOrigin::Api,
                    false,
                    crate::supervisor::SessionPlacement {
                        machine_id: &runtime,
                        workspace,
                        execution_binding: Some(&binding),
                    },
                    crate::supervisor::ProviderGeneration {
                        version: &generation.version,
                        digest: &generation.digest,
                        auth_generation: generation.auth_generation,
                        behavior: Some(&generation.behavior),
                    },
                    owner,
                )
            },
        )
        .map_err(|_| "authentication_required")?
        .map_err(|_| "provider_unavailable")?;
    apply_preset(state, record, version, digest)?;
    state
        .supervisor
        .start_registered_session(&record.child_session_id)
        .map_err(|_| "provider_unavailable")
}

fn stop_code(stop_reason: &str) -> &'static str {
    match stop_reason.to_ascii_lowercase().replace('_', "").as_str() {
        "endturn" | "done" => "completed",
        "cancelled" | "canceled" => "cancelled",
        "refusal" => "refusal",
        "maxtokens" => "max_tokens",
        "maxturnrequests" => "max_turn_requests",
        _ => "provider_error",
    }
}

fn result_value(text: &str, stop_reason: &str, structured: Option<&Value>) -> Value {
    let truncated = text.len() > MAX_RESULT_TEXT;
    let mut end = text.len().min(MAX_RESULT_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    json!({
        "text": &text[..end],
        "truncated": truncated,
        "structured": structured,
        "stop_reason": stop_reason,
    })
}

async fn observe(state: &Arc<AppState>, store: &Store, record: &Record) -> Step {
    let child = record.child_session_id.as_str();
    let Some(cursor) = record.child_cursor else {
        // Only a call that recorded its cursor can have sent a prompt.
        let cancelled = record.state == State::Stopping;
        advance(state, store, record, move |next| {
            if cancelled {
                next.state = State::Cancelled;
            } else {
                next.state = State::Failed;
                next.error = Some(CallError::new("launch_lost", None));
            }
        })
        .await;
        return Step::Again;
    };
    if record.state == State::Running && record.cancel_requested_at_ms.is_some() {
        if advance(state, store, record, |next| next.state = State::Stopping).await {
            let _ = state
                .supervisor
                .send(child, crate::acp::AgentCommand::Cancel);
        }
        return Step::Again;
    }
    let Some(turn) = state.hub.managed_turn(child, cursor) else {
        let cancelled = record.state == State::Stopping;
        advance(state, store, record, move |next| {
            if cancelled {
                next.state = State::Cancelled;
            } else {
                next.state = State::Failed;
                next.error = Some(CallError::new("child_removed", None));
            }
        })
        .await;
        return Step::Again;
    };
    if let Some(stop_reason) = turn.stop_reason.as_deref() {
        let request = store
            .managed_call_input(&record.placement.parent_session_id, &record.call_id)
            .await
            .ok()
            .flatten();
        let wants_schema = request
            .as_ref()
            .is_some_and(|request| output_schema(request).is_some());
        let code = stop_code(stop_reason);
        let parsed = (wants_schema && code == "completed")
            .then(|| serde_json::from_str::<Value>(turn.final_text.trim()).ok())
            .flatten();
        let result = result_value(&turn.final_text, stop_reason, parsed.as_ref());
        let stopping = record.state == State::Stopping;
        let cancel_requested = record.cancel_requested_at_ms.is_some();
        advance(state, store, record, move |next| {
            next.result = Some(result);
            match code {
                "completed" if wants_schema && parsed.is_none() => {
                    next.state = State::Failed;
                    next.error = Some(CallError::new("structured_output_invalid", None));
                }
                // Completion wins a stop race: keep the actual result.
                "completed" => next.state = State::Completed,
                "cancelled" if stopping || cancel_requested => {
                    if stopping {
                        next.state = State::Cancelled;
                    } else {
                        next.state = State::Stopping;
                    }
                }
                code => {
                    next.state = State::Failed;
                    next.error = Some(CallError::new(code, None));
                }
            }
        })
        .await;
        return Step::Again;
    }
    let elapsed = now_ms() - record.updated_at_ms;
    match turn.status {
        Status::Crashed | Status::Interrupted if turn.prompted || elapsed > PROMPT_RECOVERY_MS => {
            let detail = state.hub.latest_crash_detail(child);
            let stopping = record.state == State::Stopping;
            let partial = result_value(&turn.final_text, "crashed", None);
            advance(state, store, record, move |next| {
                next.result = Some(partial);
                if stopping {
                    next.state = State::Cancelled;
                } else {
                    next.state = State::Failed;
                    next.error = Some(CallError::new("child_crashed", detail));
                }
            })
            .await;
            return Step::Again;
        }
        Status::Exited if record.state == State::Stopping => {
            advance(state, store, record, |next| next.state = State::Cancelled).await;
            return Step::Again;
        }
        _ => {}
    }
    if record.state == State::Stopping && elapsed > STOP_GRACE_MS {
        // Stop only this exact child worker; its turn never confirmed.
        state.supervisor.delete_session(child);
    }
    if record.state == State::Running
        && !turn.prompted
        && elapsed > PROMPT_RECOVERY_MS
        && let Ok(Some(request)) = store
            .managed_call_input(&record.placement.parent_session_id, &record.call_id)
            .await
    {
        // The cursor was recorded but no trace of the prompt exists: the
        // previous Controller stopped between the two. Same message id.
        state
            .hub
            .submit_managed_prompt(child, &record.call_id, request.instruction);
    }
    Step::Wait(POLL)
}

/// Release a finished child's worker; its transcript and workspace stay.
fn settle(state: &AppState, record: &Record) {
    if state.supervisor.has_live_worker(&record.child_session_id) {
        let _ = state.supervisor.hibernate_session(&record.child_session_id);
    }
}
