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

mod configuration;

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
    if previous.state == State::Starting && next.state == State::Running {
        next.started_at_ms = Some(next.updated_at_ms);
        next.provider_account_id =
            state
                .hub
                .session_info(&next.child_session_id)
                .and_then(|child| {
                    observed_account(
                        &child.meta,
                        &next.provider,
                        next.runtime_machine_id.as_deref(),
                    )
                });
        next.configuration = state
            .supervisor
            .observed_configuration(&next.child_session_id)
            .map(|options| {
                ["model", "reasoning_effort", "effort"]
                    .into_iter()
                    .filter_map(|key| {
                        let observed = options
                            .as_array()
                            .and_then(|options| options.iter().find(|option| option["id"] == key))
                            .and_then(|option| option.get("currentValue"));
                        observed
                            .and_then(serde_json::Value::as_str)
                            .filter(|value| value.len() <= 256)
                            .map(|value| (key.to_owned(), value.to_owned()))
                    })
                    .collect()
            });
    }
    if next.state.terminal() {
        next.finished_at_ms = Some(next.updated_at_ms);
    }
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

/// A continued child can upgrade its Provider release while keeping its account
/// binding. The release is not an account identity and must not erase evidence.
fn observed_account(
    child: &crate::core::SessionMeta,
    provider: &str,
    runtime: Option<&str>,
) -> Option<crate::provider_account::ProviderAccountId> {
    (child.provider == provider && Some(child.machine_id.as_str()) == runtime)
        .then(|| child.provider_account_id.clone())
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
            notify_parent(state, &record);
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
                record.purpose.profile(),
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

/// Only a persisted selection of another call exempts a launch from its
/// deadline. Check this before any input read, snapshot RPC or child creation.
async fn launch_source(
    store: &Store,
    record: &Record,
    now: i64,
) -> Result<Option<Record>, &'static str> {
    let source = store
        .review_input_source(&record.placement.parent_session_id, &record.call_id)
        .await;
    if let Ok(Some(original)) = &source
        && original.call_id != record.call_id
    {
        return Ok(Some(original.clone()));
    }
    if now - record.created_at_ms > LAUNCH_DEADLINE_MS {
        return Err("launch_timeout");
    }
    source.map(|_| None).map_err(|_| RETRY)
}

async fn launch(state: &Arc<AppState>, store: &Store, record: &Record) -> Step {
    if record.cancel_requested_at_ms.is_some() {
        // The launch outcome may already exist; observe it while stopping.
        advance(state, store, record, |next| next.state = State::Stopping).await;
        return Step::Again;
    }
    match launch_source(store, record, now_ms()).await {
        Ok(Some(original)) => return reuse_source(state, store, record, &original).await,
        Ok(None) => {}
        Err(RETRY) => return Step::Wait(POLL),
        Err(code) => {
            advance(state, store, record, fail(code, None)).await;
            return Step::Again;
        }
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
        profile: record.purpose.profile(),
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
            let category = match code.as_str() {
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
            if advance(state, store, record, fail(category, Some(code))).await && existing.is_none()
            {
                // A refused fresh round may already have created its child
                // directory, and no session will ever own or retire it.
                let _ = execution_action(
                    state,
                    &record.placement.machine_id,
                    execution::Action::CloseManagedChild {
                        child_session_id: child.to_owned(),
                    },
                )
                .await;
            }
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
    if let Some(step) = reuse_review(state, store, record, &request, &prepared.input_revision).await
    {
        return step;
    }
    if let Err(code) = configuration::ready(state, store, record).await {
        if code == RETRY {
            return Step::Wait(POLL);
        }
        // No prompt was admitted. Stop even a still-starting child so a late
        // ready event cannot leave an orphan worker after the failed call.
        state.supervisor.delete_session(child);
        advance(state, store, record, fail(code, None)).await;
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
            .submit_managed_prompt(child, &record.call_id, request.prompt());
    }
    Step::Again
}

/// Snapshot preparation and child registration spend no model tokens. Reserve
/// before activation so simultaneous requests and recovery share one prompt.
async fn reuse_review(
    state: &AppState,
    store: &Store,
    record: &Record,
    request: &Request,
    revision: &str,
) -> Option<Step> {
    let child = state.hub.session_info(&record.child_session_id)?;
    let identity = crate::managed_calls::review::identity(
        request,
        record,
        revision,
        &child.meta.provider_account_id,
        child.meta.provider_auth_generation,
    )?;
    let original = match store.reserve_review_input(record, &identity).await {
        Ok(original) => original,
        Err(_) => return Some(Step::Wait(POLL)),
    };
    if original.call_id == record.call_id {
        return None;
    }
    Some(reuse_source(state, store, record, &original).await)
}

fn source_failure(original: &Record) -> CallError {
    original.error.clone().unwrap_or_else(|| {
        let code = if original.state == State::Cancelled {
            "review_source_cancelled"
        } else if original
            .result
            .as_ref()
            .and_then(|result| result.get("truncated"))
            == Some(&Value::Bool(true))
        {
            "review_source_truncated"
        } else {
            "review_source_not_reusable"
        };
        CallError::new(code, Some(original.call_id.clone()))
    })
}

async fn reuse_source(state: &AppState, store: &Store, record: &Record, original: &Record) -> Step {
    if !original.state.terminal() {
        return Step::Wait(Duration::from_secs(2));
    }
    let complete = crate::managed_calls::review::complete(original);
    let mut result = original.result.clone().unwrap_or_else(|| json!({}));
    // Attribution belongs to the original call. Copying token counters into a
    // reuse receipt would make task totals charge the same inference twice.
    let Some(object) = result.as_object_mut() else {
        advance(state, store, record, fail("review_source_invalid", None)).await;
        return Step::Again;
    };
    object.remove("usage");
    result["reused_from"] = json!({
        "call_id":original.call_id,
        "child_session_id":original.child_session_id,
        "started_at_ms":original.started_at_ms,
        "finished_at_ms":original.finished_at_ms,
    });
    if !serde_json::to_vec(&result).is_ok_and(|bytes| bytes.len() <= 1024 * 1024) {
        advance(
            state,
            store,
            record,
            fail("review_source_too_large", Some(original.call_id.clone())),
        )
        .await;
        return Step::Again;
    }
    advance(state, store, record, |next| {
        next.input_revision.clone_from(&original.input_revision);
        next.result = Some(result);
        next.configuration.clone_from(&original.configuration);
        next.provider_account_id
            .clone_from(&original.provider_account_id);
        next.state = if complete {
            State::Completed
        } else {
            State::Failed
        };
        if !complete {
            next.error = Some(source_failure(original));
        }
    })
    .await;
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
    let presets = state
        .provider_catalog
        .package(&record.provider, version, digest)
        .map(|package| package.manifest.configuration.presets)
        .ok_or("preset_unavailable")?;
    let values: std::collections::BTreeMap<String, serde_json::Value> =
        if let Some(custom) = super::super::agent_tools::custom_values(preset) {
            // A custom choice may set only the options this generation's
            // presets set; the managed worker refuses any other.
            if !custom.keys().all(|id| {
                presets
                    .iter()
                    .any(|candidate| candidate.values.contains_key(id))
            }) {
                return Err("preset_unavailable");
            }
            custom
                .into_iter()
                .map(|(id, value)| (id, json!(value)))
                .collect()
        } else {
            presets
                .into_iter()
                .find(|candidate| candidate.id == *preset)
                .ok_or("preset_unavailable")?
                .values
                .into_iter()
                .map(|(id, value)| (id, json!(value)))
                .collect()
        };
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
        profile: record.purpose.profile(),
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
        profile: record.purpose.profile(),
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
        record.purpose.profile(),
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
                        account_id: None,
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
    Ok(())
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
    let mut result = json!({
        "text": &text[..end],
        "truncated": truncated,
        "structured": structured,
        "stop_reason": stop_reason,
    });
    // JSON escaping and the parsed copy can exceed the durable result limit
    // even when text fits. Leave room for usage, input and reuse attribution.
    if !serde_json::to_vec(&result).is_ok_and(|bytes| bytes.len() <= MAX_RESULT_TEXT) {
        result["truncated"] = Value::Bool(true);
        result["structured"] = Value::Null;
        // Six bytes is JSON's maximum expansion per UTF-8 byte.
        end = end.min((MAX_RESULT_TEXT - 1024) / 6);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        result["text"] = json!(&text[..end]);
    }
    result
}

async fn deliver_images(
    state: &AppState,
    store: &Store,
    record: &Record,
    updates: &[Value],
) -> Result<Vec<Value>, &'static str> {
    use crate::managed_calls::media::{self, Delivery};
    use base64::Engine as _;
    let artifacts = store.artifacts();
    let images = media::collect_turn(updates, &artifacts)?;
    let mut delivered = Vec::new();
    for image in images {
        let path = artifacts.path(&image.id).ok_or("artifact_missing")?;
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| "artifact_missing")?;
        let delivery = Delivery {
            parent_session_id: record.placement.parent_session_id.clone(),
            child_session_id: record.child_session_id.clone(),
            call_id: record.call_id.clone(),
            artifact: image.clone(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        let path = loop {
            match execution_action(
                state,
                &record.placement.machine_id,
                execution::Action::DeliverManagedArtifact {
                    delivery: Box::new(delivery.clone()),
                },
            )
            .await
            {
                Ok(execution::Response::ManagedArtifact { path }) => break path,
                Ok(execution::Response::ManagedRoundRefused { .. }) => {
                    return Err("artifact_delivery_failed");
                }
                _ if tokio::time::Instant::now() < deadline => {
                    // Retry only the exact digest delivery, never the native turn.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                _ => return Err("artifact_delivery_unavailable"),
            }
        };
        let mut value = serde_json::to_value(image).expect("artifact serializes");
        value["path"] = json!(path);
        value["machine_id"] = json!(record.placement.machine_id);
        delivered.push(value);
    }
    Ok(delivered)
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
        let scoped_review = request.as_ref().is_some_and(|request| {
            request
                .labels
                .contains_key(crate::managed_calls::review::BASE)
        });
        let incomplete_review = scoped_review
            && parsed.as_ref().is_none_or(|value| {
                value.get("review_complete") != Some(&Value::Bool(true))
                    || !matches!(
                        value["verdict"].as_str(),
                        Some("approve" | "needs-attention")
                    )
            });
        let mut result = result_value(&turn.final_text, stop_reason, parsed.as_ref());
        let truncated_review = scoped_review && result["truncated"] == true;
        if let Some(usage) = turn.usage {
            result["usage"] = json!(usage);
        }
        if let Some(request) = &request
            && let Some(base) = request.labels.get(crate::managed_calls::review::BASE)
        {
            result["review_input"] = json!({
                "root": request.context.root,
                "base": base,
                "task": request.labels.get(crate::managed_calls::review::TASK),
            });
        }
        if record.purpose.is_image()
            && let Some(failure) = turn
                .generated_images
                .iter()
                .find_map(crate::managed_calls::media::quota_failure)
        {
            result["native_failure"] = failure;
        }
        let image_error = if record.purpose.is_image() && code == "completed" {
            match deliver_images(state, store, record, &turn.generated_images).await {
                Ok(artifacts) => {
                    result["artifacts"] = json!(artifacts);
                    None
                }
                Err(code) => Some(code),
            }
        } else {
            None
        };
        let stopping = record.state == State::Stopping;
        let cancel_requested = record.cancel_requested_at_ms.is_some();
        advance(state, store, record, move |next| {
            next.result = Some(result);
            match code {
                "completed" if truncated_review => {
                    next.state = State::Failed;
                    next.error = Some(CallError::new("review_output_truncated", None));
                }
                "completed" if image_error.is_some() => {
                    next.state = State::Failed;
                    next.error = image_error.map(|code| CallError::new(code, None));
                }
                "completed" if wants_schema && parsed.is_none() => {
                    next.state = State::Failed;
                    next.error = Some(CallError::new("structured_output_invalid", None));
                }
                "completed" if incomplete_review => {
                    next.state = State::Failed;
                    next.error = Some(CallError::new("review_incomplete", None));
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
        // A replacement worker must confirm the original preset again before
        // this recovery path may send anything to the native model.
        if let Err(code) = configuration::ready(state, store, record).await {
            if code == RETRY {
                return Step::Wait(POLL);
            }
            state.supervisor.delete_session(child);
            advance(state, store, record, fail(code, None)).await;
            return Step::Again;
        }
        state
            .hub
            .submit_managed_prompt(child, &record.call_id, request.prompt());
    }
    Step::Wait(POLL)
}

/// Release a finished child's worker; its transcript and workspace stay.
/// Queue the parent's opted-in completion notice. A parent that is gone or
/// closing gets none; the Hub's call-scoped id absorbs a repeated notice.
fn notify_parent(state: &AppState, record: &Record) {
    if record
        .labels
        .get(crate::managed_calls::review::NOTIFY)
        .is_none_or(|value| value != "parent")
    {
        return;
    }
    let parent = &record.placement.parent_session_id;
    if state
        .hub
        .session_info(parent)
        .is_none_or(|info| info.meta.closing)
    {
        return;
    }
    state
        .hub
        .submit_call_notice(parent, &record.call_id, call_notice(record));
}

fn call_notice(record: &Record) -> String {
    let title = record
        .labels
        .get("title")
        .map(|title| format!(" ({title})"))
        .unwrap_or_default();
    let error = record
        .error
        .as_ref()
        .map(|error| format!(", error {}", error.code))
        .unwrap_or_default();
    format!(
        "[Cowboy] Managed call {}{title} finished: {}{error}. Read it with cowboy_read: \
         cowboy.calls.result(\"{}\",{{offset:0,chars:4000}}). A running, failed or partial \
         review is never evidence of a clean review.",
        record.call_id,
        record.state.code(),
        record.call_id
    )
}

fn settle(state: &AppState, record: &Record) {
    if state.supervisor.has_live_worker(&record.child_session_id) {
        let _ = state.supervisor.hibernate_session(&record.child_session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One durable timeline covers timeout, recovery and continuation.
    async fn review_waiters_recover_without_machine_access_and_keep_the_source_conversation() {
        use crate::managed_calls::service::{Choice, Ledger};
        let root =
            std::env::temp_dir().join(format!("cw-review-recovery-{}", rand::random::<u64>()));
        let store = Store::connect("sqlite::memory:", root.clone())
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let parent = serde_json::from_value(json!({
            "id":"parent","provider":"codex","machine_id":"hawk","workspace_id":"project",
            "cwd":"/workspace","title":"Review","status":"running"
        }))
        .unwrap();
        store.insert_session(&parent).await.unwrap();
        let ledger = Ledger::for_parent(store.clone(), "service", &parent).unwrap();
        let mut request = Request::parse(
            br#"{"schema":1,"request_id":"original","purpose":"review",
            "instruction":"Check persistence","context":{"scope":"current-worktree"},
            "access":"read-only","conversation":{"mode":"fresh"}}"#,
        )
        .unwrap();
        let original = ledger
            .admit(
                "codex",
                &request,
                "original",
                "source-child",
                &Choice::explicit(),
            )
            .await
            .unwrap();
        let (original, _) = ledger
            .claim(&original.call_id, "hawk", "1.0.0", "generation")
            .await
            .unwrap()
            .unwrap();
        let expired = original.created_at_ms + LAUNCH_DEADLINE_MS + 1000;
        assert_eq!(
            launch_source(&store, &original, expired)
                .await
                .err()
                .unwrap(),
            "launch_timeout"
        );
        assert!(
            launch_source(&store, &original, original.created_at_ms)
                .await
                .unwrap()
                .is_none()
        );
        store
            .reserve_review_input(&original, "snapshot")
            .await
            .unwrap();
        assert_eq!(
            launch_source(&store, &original, expired)
                .await
                .err()
                .unwrap(),
            "launch_timeout"
        );
        request.request_id = "duplicate".into();
        let duplicate = ledger
            .admit(
                "codex",
                &request,
                "duplicate",
                "unused-child",
                &Choice::explicit(),
            )
            .await
            .unwrap();
        let (duplicate, _) = ledger
            .claim(&duplicate.call_id, "hawk", "1.0.0", "generation")
            .await
            .unwrap()
            .unwrap();
        store
            .reserve_review_input(&duplicate, "snapshot")
            .await
            .unwrap();
        let recovered = launch_source(&store, &duplicate, expired)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.call_id, original.call_id);
        let mut completed = duplicate.clone();
        completed.state = State::Completed;
        completed.revision += 1;
        completed.finished_at_ms = Some(completed.updated_at_ms);
        completed.input_revision = Some(crate::managed_calls::canonical_digest(&json!("snapshot")));
        completed.result = Some(json!({
            "reused_from":{"call_id":"original","child_session_id":"source-child"},
            "structured":{"review_complete":true,"verdict":"approve"}
        }));
        assert!(ledger.advance(&duplicate, &completed).await.unwrap());
        assert_eq!(
            super::super::coordinator::envelope(&completed)["resolved"]["child_session_id"],
            "source-child"
        );
        request.request_id = "followup".into();
        request.conversation = Conversation::Continue {
            child_session_id: "unused-child".into(),
        };
        assert!(
            ledger
                .admit("codex", &request, "followup", "unused", &Choice::explicit())
                .await
                .is_err()
        );
        let mut failed = original.clone();
        failed.state = State::Failed;
        failed.revision += 1;
        failed.finished_at_ms = Some(failed.updated_at_ms);
        failed.error = Some(CallError::new("launch_timeout", None));
        assert!(ledger.advance(&original, &failed).await.unwrap());
        assert_eq!(source_failure(&failed).code, "launch_timeout");
        let notice = call_notice(&failed);
        assert!(
            notice.contains(&format!("Managed call {} ", failed.call_id))
                && notice.contains("finished: failed, error launch_timeout")
                && notice.contains(&format!("cowboy.calls.result(\"{}\"", failed.call_id)),
            "{notice}"
        );
        failed.state = State::Cancelled;
        failed.error = None;
        assert_eq!(source_failure(&failed).code, "review_source_cancelled");
        failed.state = State::Completed;
        failed.result = Some(json!({"truncated":true}));
        assert_eq!(source_failure(&failed).code, "review_source_truncated");
        request.conversation = Conversation::Continue {
            child_session_id: "source-child".into(),
        };
        assert!(
            ledger
                .admit("codex", &request, "followup", "unused", &Choice::explicit())
                .await
                .is_ok()
        );
        drop(ledger);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_review_results_fit_the_ledger_and_cannot_certify_completion() {
        for payload in [
            "x".repeat(600_000),
            "\0".repeat(200_000),
            "中文".repeat(200_000),
        ] {
            let structured = json!({"review_complete":true,"verdict":"approve","summary":payload});
            let text = serde_json::to_string(&structured).unwrap();
            let result = result_value(&text, "end_turn", Some(&structured));
            assert_eq!(result["truncated"], true);
            assert!(result["structured"].is_null());
            assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_RESULT_TEXT);
        }
    }

    #[test]
    fn account_observation_survives_child_release_updates_but_not_other_placements() {
        let mut child: crate::core::SessionMeta = serde_json::from_value(json!({
            "id": "child", "provider": "codex", "machine_id": "ovh",
            "provider_version": "1.0.0", "provider_generation_digest": "old",
            "cwd": "/workspace", "title": "Review", "status": "running"
        }))
        .unwrap();
        let account = Some(child.provider_account_id.clone());
        assert_eq!(observed_account(&child, "codex", Some("ovh")), account);
        child.provider_version = "2.0.0".into();
        child.provider_generation_digest = "new".into();
        assert_eq!(observed_account(&child, "codex", Some("ovh")), account);
        assert_eq!(observed_account(&child, "claude-code", Some("ovh")), None);
        assert_eq!(observed_account(&child, "codex", Some("hawk")), None);
        assert_eq!(observed_account(&child, "codex", None), None);
    }
}
