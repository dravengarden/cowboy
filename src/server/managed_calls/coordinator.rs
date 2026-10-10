//! Controller-owned grants and the private gateway action handler.
//!
//! A grant is issued for the current parent worker incarnation and installed
//! on the parent's execution Machine. Each forwarded action is accepted only
//! on that Machine's current connection, for the grant installed there, and
//! after `Authority` re-derived from the live parent worker still equals the
//! authority captured at issue. Worker replacement, owner or Provider
//! generation changes and closing the parent therefore revoke the grant.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::super::AppState;
use super::super::agent_tools;
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::MANAGED_CALL_PROTOCOL_VERSION;
use crate::machine_protocol::execution;
use crate::managed_calls::authority::Authority;
use crate::managed_calls::lifecycle::{Record, State};
use crate::managed_calls::protocol::{Action, Grant};
use crate::managed_calls::service::Ledger;
use crate::managed_calls::{Conversation, Request};

const RECONCILE_INTERVAL: Duration = Duration::from_secs(2);
const INSTALL_RETRY: Duration = Duration::from_secs(30);

struct GrantEntry {
    grant: Grant,
    authority: Authority,
    installed_on: Option<ConnectionToken>,
    retry_after: Option<Instant>,
}

#[derive(Default)]
pub(in crate::server) struct Coordinator {
    grants: Mutex<HashMap<String, GrantEntry>>,
    installing: Mutex<HashSet<String>>,
    pub(super) runners: Mutex<HashSet<String>>,
    pub(super) changed: tokio::sync::Notify,
    pub(super) approvals: super::approval::Approvals,
}

pub(super) fn error(code: &str, admission: &str) -> Value {
    json!({"schema":1,"state":"error","error":{"code":code,"admission":admission}})
}

fn random_id(prefix: &str) -> String {
    let mut bytes = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    format!(
        "{prefix}-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

/// Current authority for a parent, derived only from Controller state.
fn current_authority(
    state: &AppState,
    parent: &str,
) -> Option<(crate::core::SessionMeta, Authority)> {
    let info = state.hub.session_info(parent)?;
    let runtime = state.runtime_router.runtime(&info.meta.machine_id)?;
    let authority = runtime
        .managed_call_authority(&state.service_id, &info.meta)
        .ok()?;
    Some((info.meta, authority))
}

impl Coordinator {
    pub(in crate::server) fn notify(&self) {
        self.changed.notify_waiters();
    }

    /// Reconcile installed grants with live parent workers until shutdown.
    pub(in crate::server) fn start(state: &Arc<AppState>) {
        let weak = Arc::downgrade(state);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(RECONCILE_INTERVAL).await;
                let Some(state) = weak.upgrade() else { return };
                if *state.shutdown.borrow() {
                    return;
                }
                reconcile(&state);
            }
        });
        super::runner::recover(state);
    }
}

fn reconcile(state: &Arc<AppState>) {
    let coordinator = &state.managed_calls;
    let mut desired: HashMap<String, Authority> = HashMap::new();
    for meta in state.hub.session_list() {
        if meta.closing
            || meta.system
            || meta
                .execution_binding
                .as_ref()
                .is_some_and(|binding| binding.managed().is_some())
        {
            continue;
        }
        let Some(runtime) = state.runtime_router.runtime(&meta.machine_id) else {
            continue;
        };
        if let Ok(authority) = runtime.managed_call_authority(&state.service_id, &meta) {
            desired.insert(meta.id, authority);
        }
    }
    let mut revoked = Vec::new();
    let mut install = Vec::new();
    {
        let mut grants = coordinator.grants.lock();
        grants.retain(|parent, entry| {
            let keep = desired
                .get(parent)
                .is_some_and(|authority| entry.authority.accepts(authority));
            if !keep {
                revoked.push((
                    entry.grant.clone(),
                    entry.authority.placement().machine_id.clone(),
                ));
            }
            keep
        });
        for (parent, authority) in desired {
            let machine = authority.placement().machine_id.clone();
            let Some((connection, protocol)) = state.machine_control.connection_protocol(&machine)
            else {
                continue;
            };
            if protocol < MANAGED_CALL_PROTOCOL_VERSION {
                continue;
            }
            let entry = grants.entry(parent.clone()).or_insert_with(|| GrantEntry {
                grant: Grant {
                    grant_id: random_id("grant"),
                    parent_session_id: parent.clone(),
                },
                authority,
                installed_on: None,
                retry_after: None,
            });
            let installed = entry
                .installed_on
                .as_ref()
                .is_some_and(|token| token.same(&connection));
            let waiting = entry
                .retry_after
                .is_some_and(|retry| Instant::now() < retry);
            if !installed && !waiting {
                install.push((entry.grant.clone(), machine, connection));
            }
        }
    }
    for (grant, machine) in revoked {
        // The grant is already gone from the registry, so every action that
        // still reaches its gateway is refused. Removing the context file
        // is cleanup, not the security boundary.
        let state = Arc::clone(state);
        tokio::spawn(async move {
            let _ = execution_action(
                &state,
                &machine,
                execution::Action::RevokeCallGateway { grant },
            )
            .await;
        });
    }
    for (grant, machine, connection) in install {
        if !coordinator
            .installing
            .lock()
            .insert(grant.parent_session_id.clone())
        {
            continue;
        }
        let state = Arc::clone(state);
        tokio::spawn(async move {
            let parent = grant.parent_session_id.clone();
            let result = execution_action(
                &state,
                &machine,
                execution::Action::InstallCallGateway {
                    grant: grant.clone(),
                },
            )
            .await;
            let mut grants = state.managed_calls.grants.lock();
            if let Some(entry) = grants.get_mut(&parent)
                && entry.grant == grant
            {
                if matches!(result, Ok(execution::Response::CallGateway))
                    && state.machine_control.is_current(&connection)
                {
                    entry.installed_on = Some(connection);
                    entry.retry_after = None;
                } else {
                    entry.retry_after = Some(Instant::now() + INSTALL_RETRY);
                    tracing::debug!(parent = %parent, "managed call gateway installation deferred");
                }
            }
            drop(grants);
            state.managed_calls.installing.lock().remove(&parent);
        });
    }
}

pub(super) async fn execution_action(
    state: &AppState,
    machine: &str,
    action: execution::Action,
) -> Result<execution::Response, String> {
    state
        .machine_control
        .execution_request(
            machine,
            execution::Request {
                service_id: state.service_id.clone(),
                machine_id: machine.to_owned(),
                action,
            },
        )
        .await
}

/// Authenticate one forwarded action. Returns the parent's current metadata
/// and ledger only when the grant is the one installed for this exact
/// connection and the live authority still equals the issued authority.
fn authorize(
    state: &AppState,
    connection: &ConnectionToken,
    grant: &Grant,
) -> Result<(crate::core::SessionMeta, Ledger), &'static str> {
    if !grant.validate() || !state.machine_control.is_current(connection) {
        return Err("permission_denied");
    }
    let issued = {
        let grants = state.managed_calls.grants.lock();
        let entry = grants
            .get(&grant.parent_session_id)
            .ok_or("permission_denied")?;
        if entry.grant != *grant
            || !entry
                .installed_on
                .as_ref()
                .is_some_and(|token| token.same(connection))
            || entry.authority.placement().machine_id != connection.machine_id()
        {
            return Err("permission_denied");
        }
        entry.authority.clone()
    };
    let (meta, current) =
        current_authority(state, &grant.parent_session_id).ok_or("permission_denied")?;
    if !issued.accepts(&current) {
        return Err("permission_denied");
    }
    let store = state.store.clone().ok_or("capacity")?;
    let ledger =
        Ledger::for_parent(store, &state.service_id, &meta).map_err(|_| "permission_denied")?;
    Ok((meta, ledger))
}

pub(super) fn envelope(record: &Record) -> Value {
    let next = if record.state.terminal() {
        json!({"argv":["cowboy","call","result",record.call_id]})
    } else {
        json!({"argv":["cowboy","call","wait",record.call_id]})
    };
    json!({
        "schema": 1,
        "state": record.state.code(),
        "call_id": record.call_id,
        "request_id": record.request_id,
        "resolved": {
            "provider": record.provider,
            "provider_version": record.provider_version,
            "runtime_machine_id": record.runtime_machine_id,
            "machine_id": record.placement.machine_id,
            "workspace_id": record.placement.workspace_id,
            "child_session_id": record.child_session_id,
            "input_revision": record.input_revision,
        },
        "labels": record.labels,
        "cancel_requested": record.cancel_requested_at_ms.is_some(),
        "error": record.error,
        "next": next,
    })
}

/// Handle one forwarded gateway action.
pub(in crate::server) async fn handle(
    state: &Arc<AppState>,
    connection: &ConnectionToken,
    grant: Grant,
    action: Action,
) -> Value {
    if action.validate().is_err() {
        return error("invalid_contract", "not_submitted");
    }
    let (meta, ledger) = match authorize(state, connection, &grant) {
        Ok(authorized) => authorized,
        // Authorization precedes admission, so a refusal proves non-admission.
        Err(code) => return error(code, "not_submitted"),
    };
    match action {
        Action::Capabilities {} => capabilities(state, &meta, &ledger).await,
        Action::Start {
            provider,
            request,
            wait_ms,
            preset,
        } => {
            start(
                state,
                &meta,
                &ledger,
                &provider,
                *request,
                wait_ms,
                preset.as_deref(),
            )
            .await
        }
        Action::Inspect { call_id } => match ledger.inspect(&call_id).await {
            Ok(Some(record)) => envelope(&record),
            Ok(None) => error("not_found", "not_submitted"),
            Err(_) => error("capacity", "not_submitted"),
        },
        Action::Observe { request_id } => match ledger.observe(&request_id).await {
            Ok(Some(record)) => envelope(&record),
            Ok(None) => error("not_found", "not_submitted"),
            Err(_) => error("capacity", "not_submitted"),
        },
        Action::Wait {
            call_id,
            timeout_ms,
        } => match wait(state, &ledger, &call_id, timeout_ms).await {
            Some(record) => envelope(&record),
            None => error("not_found", "not_submitted"),
        },
        Action::Result { call_id } => match ledger.inspect(&call_id).await {
            Ok(Some(record)) if record.state.terminal() => {
                let mut value = envelope(&record);
                value["result"] = record.result.clone().unwrap_or(Value::Null);
                value
            }
            Ok(Some(record)) => {
                let mut value = envelope(&record);
                value["state"] = json!("error");
                value["error"] = json!({"code":"not_ready","admission":"accepted",
                    "call_state": record.state.code()});
                value
            }
            Ok(None) => error("not_found", "not_submitted"),
            Err(_) => error("capacity", "not_submitted"),
        },
        Action::Cancel { call_id } => cancel(state, &ledger, &call_id).await,
    }
}

async fn wait(state: &AppState, ledger: &Ledger, call: &str, timeout_ms: u64) -> Option<Record> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let notified = state.managed_calls.changed.notified();
        let record = ledger.inspect(call).await.ok()??;
        if record.state.terminal() || tokio::time::Instant::now() >= deadline {
            return Some(record);
        }
        let _ = tokio::time::timeout_at(
            deadline.min(tokio::time::Instant::now() + Duration::from_secs(2)),
            notified,
        )
        .await;
    }
}

pub(super) struct ProviderReadiness {
    pub available: bool,
    pub reason: Option<&'static str>,
    pub generation: Option<super::super::ResolvedProviderGeneration>,
    pub structured_output: bool,
}

/// Where a managed child's Agent runtime runs, and whether it can. The
/// execution Machine is preferred. When host policy places the Provider on
/// other Machines, one of them (the parent's runtime first) runs the child
/// against the execution Machine's snapshot through a keeper-enforced
/// read-only environment. Execution never moves; there is no local fallback.
pub(super) async fn child_runtime(
    state: &AppState,
    execution_machine: &str,
    parent_runtime: Option<&str>,
    provider: &str,
) -> (String, ProviderReadiness) {
    let local = provider_readiness(state, execution_machine, provider).await;
    if local.available
        || !matches!(
            local.reason,
            Some("runtime_policy" | "provider_unavailable")
        )
    {
        return (execution_machine.to_owned(), local);
    }
    let mut candidates: Vec<String> = parent_runtime.into_iter().map(str::to_owned).collect();
    if let Some(store) = &state.store
        && let Ok(machines) = store.list_machines().await
    {
        let mut registered: Vec<String> = machines
            .into_iter()
            .filter(|machine| !machine.revoked)
            .map(|machine| machine.id)
            .collect();
        registered.sort();
        candidates.extend(registered);
    }
    let mut seen = HashSet::new();
    candidates.retain(|machine| machine != execution_machine && seen.insert(machine.clone()));
    let mut remote_reason = None;
    for runtime in candidates {
        if !state.supervisor.runtime_allowed(provider, &runtime) {
            continue;
        }
        let readiness = runtime_readiness(state, execution_machine, &runtime, provider).await;
        if readiness.available {
            return (runtime, readiness);
        }
        remote_reason.get_or_insert(readiness.reason.unwrap_or("provider_unavailable"));
    }
    let readiness = match remote_reason {
        Some(reason) => ProviderReadiness {
            available: false,
            reason: Some(reason),
            generation: None,
            structured_output: false,
        },
        None => local,
    };
    (execution_machine.to_owned(), readiness)
}

/// Whether a child of `provider` can run with its runtime on `runtime` and
/// its execution on `execution_machine`.
pub(super) async fn runtime_readiness(
    state: &AppState,
    execution_machine: &str,
    runtime: &str,
    provider: &str,
) -> ProviderReadiness {
    let readiness = provider_readiness(state, runtime, provider).await;
    if runtime == execution_machine || !readiness.available {
        return readiness;
    }
    let checked = match &readiness.generation {
        Some(generation) => remote_ready(state, execution_machine, runtime, generation).await,
        None => Err("provider_unavailable"),
    };
    match checked {
        Ok(()) => readiness,
        Err(reason) => ProviderReadiness {
            available: false,
            reason: Some(reason),
            generation: None,
            structured_output: false,
        },
    }
}

/// A split child needs both Machines to understand remote managed bindings
/// and the exact runtime Provider to accept the target's executor.
async fn remote_ready(
    state: &AppState,
    execution_machine: &str,
    runtime_machine: &str,
    generation: &super::super::ResolvedProviderGeneration,
) -> Result<(), &'static str> {
    for machine in [execution_machine, runtime_machine] {
        match state.machine_control.connection_protocol(machine) {
            Some((_, protocol))
                if protocol >= crate::machine_protocol::MANAGED_ENVIRONMENT_PROTOCOL_VERSION => {}
            Some(_) => return Err("unsupported_capability"),
            None => return Err("machine_unavailable"),
        }
    }
    let executor = super::super::execution::sessions::executor(state, execution_machine)
        .await
        .map_err(|_| "machine_unavailable")?;
    if !super::super::execution::sessions::accepts(&generation.behavior, &executor) {
        return Err("unsupported_capability");
    }
    Ok(())
}

/// Whether the exact installed Provider on the target can run this request.
pub(super) async fn provider_readiness(
    state: &AppState,
    machine: &str,
    provider: &str,
) -> ProviderReadiness {
    let unavailable = |reason| ProviderReadiness {
        available: false,
        reason: Some(reason),
        generation: None,
        structured_output: false,
    };
    let Some(store) = state.store.as_ref() else {
        return unavailable("provider_unavailable");
    };
    let Ok(machines) = store.list_machines().await else {
        return unavailable("provider_unavailable");
    };
    let Some(record) = machines
        .iter()
        .find(|record| record.id == machine && !record.revoked)
    else {
        return unavailable("provider_unavailable");
    };
    if !state.runtime_router.connected(machine) {
        return unavailable("machine_unavailable");
    }
    // Host policy may pin a Provider's runtime to other Machines. A managed
    // child runs on the parent's execution Machine, never elsewhere.
    if !state.supervisor.runtime_allowed(provider, machine) {
        return unavailable("runtime_policy");
    }
    let inventory = super::super::execution::sessions::providers(record);
    let authentication = state.provider_auth.status(provider);
    let generation = match super::super::resolve_provider_generation(
        &state.provider_catalog,
        &state.plugin_catalog,
        &inventory,
        provider,
        authentication.as_ref(),
    ) {
        Ok(generation) => generation,
        Err(detail) => {
            return unavailable(if detail.contains("auth") || detail.contains("sign") {
                "authentication_required"
            } else {
                "provider_unavailable"
            });
        }
    };
    let supports = state
        .provider_catalog
        .package(provider, &generation.version, &generation.digest)
        .is_some_and(|package| {
            package
                .manifest
                .runtime
                .launch_arguments(Some(cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1))
                .is_ok()
        });
    if !supports {
        return ProviderReadiness {
            available: false,
            reason: Some("unsupported_capability"),
            generation: Some(generation),
            structured_output: false,
        };
    }
    ProviderReadiness {
        available: true,
        reason: None,
        generation: Some(generation),
        // The managed read-only profile contract includes the native turn
        // constraint; an adapter declaring it must apply the schema.
        structured_output: true,
    }
}

async fn capabilities(state: &AppState, meta: &crate::core::SessionMeta, ledger: &Ledger) -> Value {
    let placement = ledger.placement();
    let policy = agent_tools::effective(state, meta).calls;
    let mut providers = Vec::new();
    for provider in agent_tools::CALL_TARGETS {
        let target = policy
            .targets
            .iter()
            .find(|target| target.agent == provider);
        let (runtime, readiness) = child_runtime(
            state,
            &placement.machine_id,
            Some(meta.machine_id.as_str()),
            provider,
        )
        .await;
        // The session's policy decides first; readiness explains the rest.
        let reason = if agent_tools::same_family(provider, &meta.provider) {
            Some("same_agent")
        } else if !policy.enabled {
            Some("calls_disabled")
        } else if target.is_none() {
            Some("policy_denied")
        } else {
            readiness.reason
        };
        let available = reason.is_none() && readiness.available;
        providers.push(json!({
            "id": provider,
            "alias": if provider == "codex" { "codex" } else { "claude" },
            "available": available,
            "runtime_machine_id": available.then_some(runtime),
            "reason": reason,
            "version": readiness.generation.as_ref().map(|generation| &generation.version),
            "preset": target.and_then(|target| target.preset.as_ref()),
            // `--preset` choices; the session's Tools choose otherwise.
            "presets": agent_tools::presets(state, provider),
            "access": ["read-only"],
            "output": if readiness.structured_output { json!(["text","json_schema"]) } else { json!(["text"]) },
            "conversation": ["fresh","continue"],
        }));
    }
    json!({
        "schema": 1,
        "state": "ok",
        "enabled": policy.enabled,
        "default": policy.default,
        "providers": providers,
        "purposes": ["review","design_review","analysis"],
        "limits": {
            "max_request_bytes": crate::managed_calls::MAX_REQUEST_BYTES,
            "max_wait_ms": crate::managed_calls::protocol::MAX_WAIT_MS,
            "max_context_files": 32,
            "max_concurrent": policy.max_concurrent,
            "max_per_session": policy.max_per_session,
        },
        "placement": {
            "runtime_machine_id": meta.machine_id,
            "machine_id": placement.machine_id,
            "workspace_id": placement.workspace_id,
            // The default snapshot root; `context.root` may name another work
            // tree of a repository registered on this Machine.
            "cwd": placement.cwd,
        },
    })
}

/// A repeated request id names the same call: an explicit agent must match,
/// and Auto matches a call Auto chose.
fn same_request(original: &Record, requested: &str, preset: Option<&str>) -> bool {
    if preset.is_some_and(|preset| original.preset.as_deref() != Some(preset)) {
        return false;
    }
    if requested == agent_tools::AUTO {
        original.selection.as_deref() == Some(agent_tools::AUTO)
    } else {
        original.provider == requested
    }
}

async fn start(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
    ledger: &Ledger,
    requested: &str,
    request: Request,
    wait_ms: u64,
    preset: Option<&str>,
) -> Value {
    // A repeated request id observes the original call, even if the Provider
    // has since become unavailable or the policy changed; conflicting inputs
    // are refused.
    match ledger.observe(&request.request_id).await {
        Ok(Some(original)) => {
            if original.request_digest != request.digest()
                || !same_request(&original, requested, preset)
            {
                return error("request_conflict", "not_submitted");
            }
            super::runner::ensure_runner(state, &original);
            return envelope(&wait_started(state, ledger, &original, wait_ms).await);
        }
        Ok(None) => {}
        Err(_) => return error("capacity", "not_submitted"),
    }
    let policy = agent_tools::effective(state, meta).calls;
    // A continued conversation keeps its own agent.
    let continued = match &request.conversation {
        Conversation::Continue { child_session_id } => {
            match state.hub.session_info(child_session_id) {
                Some(child) if !child.meta.closing => Some(child.meta.provider),
                _ => return error("conversation_unavailable", "not_submitted"),
            }
        }
        Conversation::Fresh {} => None,
    };
    let requested = match &continued {
        Some(agent) if requested == agent_tools::AUTO || requested == agent => agent.as_str(),
        Some(_) => return error("conversation_unavailable", "not_submitted"),
        None => requested,
    };
    let (mut targets, selection) = match agent_tools::choose(&policy, &meta.provider, requested) {
        agent_tools::Choice::Candidates { targets, selection } => (targets, selection),
        agent_tools::Choice::Refused(code @ ("calls_disabled" | "policy_denied")) => {
            match await_approval(state, meta, &policy, requested, &request, code) {
                Ok(admitted) => admitted,
                Err(refusal) => return refusal,
            }
        }
        agent_tools::Choice::Refused(code) => return error(code, "not_submitted"),
    };
    if continued.is_some() {
        targets.truncate(1);
    }
    // An explicit preset names one of that agent's signed presets; a continued
    // conversation keeps the configuration its child already has.
    if let Some(preset) = preset {
        if continued.is_some() {
            return error("invalid_contract", "not_submitted");
        }
        if !agent_tools::preset_exists(state, requested, preset) {
            return error("preset_unavailable", "not_submitted");
        }
        for target in &mut targets {
            target.preset = Some(preset.to_owned());
        }
    }
    // A soft per-session budget: concurrent admissions may pass it together.
    match state
        .store
        .as_ref()
        .map(|store| store.managed_call_counts(&meta.id))
    {
        Some(counts) => match counts.await {
            Ok((active, _)) if active >= u64::from(policy.max_concurrent) => {
                return error("concurrency_limit", "not_submitted");
            }
            Ok((_, total)) if total >= u64::from(policy.max_per_session) => {
                return error("call_limit", "not_submitted");
            }
            Ok(_) => {}
            Err(_) => return error("capacity", "not_submitted"),
        },
        None => return error("capacity", "not_submitted"),
    }
    let mut chosen = None;
    let mut refusal = None;
    for target in targets {
        let (_, readiness) = child_runtime(
            state,
            &ledger.placement().machine_id,
            Some(ledger.runtime_machine_id()),
            &target.agent,
        )
        .await;
        if readiness.available {
            chosen = Some(target);
            break;
        }
        refusal.get_or_insert(readiness.reason.unwrap_or("provider_unavailable"));
    }
    let Some(target) = chosen else {
        return error(refusal.unwrap_or("provider_unavailable"), "not_submitted");
    };
    let call_id = random_id("call");
    let child_id = state.supervisor.reserve_session_id();
    let choice = crate::managed_calls::service::Choice {
        preset: target.preset.clone(),
        selection,
    };
    let record = match ledger
        .admit(&target.agent, &request, &call_id, &child_id, &choice)
        .await
    {
        Ok(record) => record,
        Err(error_detail) => {
            let text = error_detail.to_string();
            let code = if text.contains("conflict") {
                "request_conflict"
            } else if text.contains("unavailable or incompatible") || text.contains("not owned") {
                "conversation_unavailable"
            } else {
                // The active-child index refuses a second concurrent turn.
                "conversation_busy"
            };
            return error(code, "not_submitted");
        }
    };
    state
        .managed_calls
        .approvals
        .admitted(&meta.id, &request.request_id);
    super::runner::ensure_runner(state, &record);
    state.managed_calls.notify();
    envelope(&wait_started(state, ledger, &record, wait_ms).await)
}

/// A policy refusal becomes a question for the session's person: the agent
/// keeps resubmitting this request while it waits, and the parent's viewers
/// see one prompt to allow it once, for the session, or decline it.
fn await_approval(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    policy: &agent_tools::CallsPolicy,
    requested: &str,
    request: &Request,
    code: &'static str,
) -> Result<(Vec<agent_tools::CallTarget>, &'static str), Value> {
    let approvals = &state.managed_calls.approvals;
    match approvals.refused(&meta.id, request, requested, code) {
        super::approval::Gate::Declined => Err(error("calls_declined", "not_submitted")),
        super::approval::Gate::Pending => {
            state
                .hub
                .broadcast_call_approval(&meta.id, approvals.view(&meta.id, &meta.provider));
            let mut refusal = error(code, "not_submitted");
            refusal["approval"] = json!("pending");
            Err(refusal)
        }
        super::approval::Gate::Approved => {
            let once = agent_tools::allow_once(policy.clone(), &meta.provider, requested);
            match agent_tools::choose(&once, &meta.provider, requested) {
                agent_tools::Choice::Candidates { targets, selection } => Ok((targets, selection)),
                agent_tools::Choice::Refused(code) => Err(error(code, "not_submitted")),
            }
        }
    }
}

/// Bounded initial observation: return once the call left the launch
/// phases, reached a terminal state, or the wait elapsed.
async fn wait_started(state: &AppState, ledger: &Ledger, record: &Record, wait_ms: u64) -> Record {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
    let mut current = record.clone();
    loop {
        let notified = state.managed_calls.changed.notified();
        if let Ok(Some(record)) = ledger.inspect(&current.call_id).await {
            current = record;
        }
        if current.state.terminal() || tokio::time::Instant::now() >= deadline {
            return current;
        }
        let _ = tokio::time::timeout_at(
            deadline.min(tokio::time::Instant::now() + Duration::from_secs(2)),
            notified,
        )
        .await;
    }
}

async fn cancel(state: &Arc<AppState>, ledger: &Ledger, call: &str) -> Value {
    match cancel_call(state, &ledger.placement().parent_session_id, call).await {
        Some(record) => envelope(&record),
        None => error("not_found", "not_submitted"),
    }
}

/// Record a durable stop request (or cancel a call that never launched).
/// Idempotent: a terminal or already stopping call is returned unchanged.
pub(super) async fn cancel_call(state: &Arc<AppState>, parent: &str, call: &str) -> Option<Record> {
    let store = state.store.as_ref()?;
    for _ in 0..8 {
        let record = store.managed_call(parent, call).await.ok()??;
        if record.state.terminal() || record.cancel_requested_at_ms.is_some() {
            super::runner::ensure_runner(state, &record);
            return Some(record);
        }
        let mut next = record.clone();
        next.revision += 1;
        next.updated_at_ms = chrono::Utc::now()
            .timestamp_millis()
            .max(record.updated_at_ms);
        next.cancel_requested_at_ms = Some(next.updated_at_ms);
        if record.state == State::Queued {
            next.state = State::Cancelled;
        }
        if store.advance_managed_call(&record, &next).await.ok()? {
            super::runner::ensure_runner(state, &next);
            state.managed_calls.notify();
            return Some(next);
        }
    }
    None
}

/// Delete one managed child conversation. An active call keeps its child; the
/// call ledger (and its result summary) belongs to the parent and remains.
pub(in crate::server) fn delete_child(state: &Arc<AppState>, child: &str) -> Result<(), String> {
    let info = state
        .hub
        .session_info(child)
        .ok_or("Session no longer exists")?;
    let binding = info
        .meta
        .execution_binding
        .as_ref()
        .ok_or("Not a managed child")?;
    let identity = binding.managed().ok_or("Not a managed child")?;
    // The snapshot (and a remote child's environment) is on the execution
    // Machine; a remote child's worker is stopped on its runtime Machine.
    let execution_machine = match binding.managed_child() {
        Some(local) => local.machine_id,
        None => {
            binding
                .decode()
                .map_err(|_| "Not a managed child")?
                .environment
                .machine_id
        }
    };
    let store = state
        .store
        .clone()
        .ok_or("Managed calls require durable storage")?;
    let state = Arc::clone(state);
    let child = child.to_owned();
    // Deletion consults durable call state first, so it runs asynchronously
    // like other target-owned closures.
    tokio::spawn(async move {
        let active = store
            .managed_child(&identity.parent_session_id, &child)
            .await
            .map(|record| record.is_some_and(|record| !record.state.terminal()));
        if !matches!(active, Ok(false)) {
            state.hub.broadcast_error(
                Some(child.clone()),
                "Cancel this child's managed call before deleting it".to_owned(),
            );
            return;
        }
        state.supervisor.delete_session(&child);
        state.hub.delete_session(&child);
        let _ = execution_action(
            &state,
            &execution_machine,
            execution::Action::CloseManagedChild {
                child_session_id: child,
            },
        )
        .await;
    });
    Ok(())
}

/// After a parent is deleted its calls are cancelled by their runners; once
/// they are terminal, its child conversations and snapshots are removed.
pub(in crate::server) fn parent_deleted(state: &Arc<AppState>, parent: &str) {
    let state = Arc::clone(state);
    let parent = parent.to_owned();
    tokio::spawn(async move {
        let Some(store) = state.store.clone() else {
            return;
        };
        for _ in 0..240 {
            let active = store.active_managed_calls().await.map(|records| {
                records
                    .iter()
                    .any(|record| record.placement.parent_session_id == parent)
            });
            if matches!(active, Ok(false)) {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        for meta in state.hub.session_list() {
            let Some(identity) = meta
                .execution_binding
                .as_ref()
                .and_then(crate::execution_environment::ExecutionBinding::managed_child)
            else {
                continue;
            };
            if identity.parent_session_id == parent {
                let _ = delete_child(&state, &meta.id);
            }
        }
    });
}
