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
use super::super::background_tasks::CallOrigin;
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::execution;
use crate::managed_calls::authority::Authority;
use crate::managed_calls::lifecycle::{Record, State};
use crate::managed_calls::protocol::{Action, Grant};
use crate::managed_calls::service::Ledger;
use crate::managed_calls::{Conversation, Request};

const RECONCILE_INTERVAL: Duration = Duration::from_secs(2);
const INSTALL_RETRY: Duration = Duration::from_secs(30);
/// A finished child conversation stays continuable this long; afterwards its
/// session and snapshot are removed. The parent's call ledger remains.
const CHILD_RETENTION: Duration = Duration::from_secs(3 * 24 * 60 * 60);
const CHILD_REAP_INTERVAL: Duration = Duration::from_secs(60 * 60);
const CHILD_REAP_FIRST_DELAY: Duration = Duration::from_secs(10 * 60);
const CHILD_REAP_BATCH: usize = 32;
const CLOSE_ATTEMPTS: u32 = 5;

struct GrantEntry {
    grant: Grant,
    authority: Authority,
    mcp_epoch: Option<String>,
    installed_on: Option<ConnectionToken>,
    retry_after: Option<Instant>,
}

mod observation;
mod readiness;
pub(in crate::server) use observation::touch as touch_code_observation;
pub(in crate::server) use observation::{changes as code_changes, head as code_observation_head};
pub(super) use readiness::{child_runtime, runtime_readiness};

impl GrantEntry {
    fn accepts_installation(
        &self,
        current: Option<&(Authority, Option<String>)>,
        acknowledged: bool,
        connected: bool,
    ) -> bool {
        acknowledged
            && connected
            && current.is_some_and(|(authority, epoch)| {
                self.authority.accepts(authority) && self.mcp_epoch == *epoch
            })
    }
}

#[cfg(test)]
mod grant_tests;

#[derive(Default)]
pub(in crate::server) struct Coordinator {
    observations: observation::Observations,
    grants: Mutex<HashMap<String, GrantEntry>>,
    installing: Mutex<HashSet<String>>,
    pub(super) runners: Mutex<HashSet<String>>,
    pub(super) changed: tokio::sync::Notify,
    pub(super) approvals: super::approval::Approvals,
    /// Serializes continuation admission against child retirement.
    pub(super) child_lifecycle: tokio::sync::Mutex<()>,
    /// Children being retired: continuation is refused until they close.
    pub(super) retiring: Mutex<HashSet<String>>,
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
    current_authority_in(&state.hub, &state.runtime_router, &state.service_id, parent)
}

fn current_authority_in(
    hub: &crate::core::Hub,
    router: &crate::runtime_router::RuntimeRouter,
    service: &str,
    parent: &str,
) -> Option<(crate::core::SessionMeta, Authority)> {
    let info = hub.session_info(parent)?;
    let runtime = router.runtime(&info.meta.machine_id)?;
    let authority = runtime.managed_call_authority(service, &info.meta).ok()?;
    Some((info.meta, authority))
}

impl Coordinator {
    /// Whether a person is still being asked about this request.
    pub(in crate::server) fn approval_waiting(&self, parent: &str, request_id: &str) -> bool {
        self.approvals.waiting(parent, request_id)
    }

    pub(in crate::server) fn notify(&self) {
        self.changed.notify_waiters();
    }

    /// Reconcile installed grants with live parent workers until shutdown.
    pub(in crate::server) fn start(state: &Arc<AppState>) {
        observation::spawn(state);
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
        let weak = Arc::downgrade(state);
        tokio::spawn(async move {
            // Controller releases restart this process often; waiting a full
            // interval after each start could postpone retirement forever.
            let mut delay = CHILD_REAP_FIRST_DELAY;
            loop {
                tokio::time::sleep(delay).await;
                delay = CHILD_REAP_INTERVAL;
                let Some(state) = weak.upgrade() else { return };
                if *state.shutdown.borrow() {
                    return;
                }
                reap_expired_children(&state).await;
            }
        });
        super::runner::recover(state);
    }
}

fn reconcile(state: &Arc<AppState>) {
    let coordinator = &state.managed_calls;
    let mut desired: HashMap<String, (Authority, Option<String>)> = HashMap::new();
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
            let epoch = state
                .extensions
                .launches
                .grant_epoch(state, &meta, &authority);
            desired.insert(meta.id, (authority, epoch));
        }
    }
    let mut revoked = Vec::new();
    let mut install = Vec::new();
    {
        let mut grants = coordinator.grants.lock();
        grants.retain(|parent, entry| {
            let keep = desired.get(parent).is_some_and(|(authority, epoch)| {
                entry.authority.accepts(authority) && entry.mcp_epoch == *epoch
            });
            if !keep {
                revoked.push((
                    entry.grant.clone(),
                    entry.authority.placement().machine_id.clone(),
                ));
            }
            keep
        });
        for (parent, (authority, mcp_epoch)) in desired {
            let machine = authority.placement().machine_id.clone();
            let Some((connection, _)) = state.machine_control.connection_protocol(&machine) else {
                continue;
            };
            let entry = grants.entry(parent.clone()).or_insert_with(|| GrantEntry {
                grant: Grant {
                    grant_id: random_id("grant"),
                    parent_session_id: parent.clone(),
                    // Every admitted Machine speaks the Agent API and
                    // action protocols (MACHINE_PROTOCOL_FLOOR).
                    agent_api: true,
                    agent_actions: true,
                    // Exact ready launch proves the pinned CLI probe completed.
                    mcp: mcp_epoch.is_some(),
                },
                authority,
                mcp_epoch,
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
            let current = current_authority(&state, &parent).map(|(meta, authority)| {
                let epoch = state
                    .extensions
                    .launches
                    .grant_epoch(&state, &meta, &authority);
                (authority, epoch)
            });
            let mut grants = state.managed_calls.grants.lock();
            if let Some(entry) = grants.get_mut(&parent)
                && entry.grant == grant
            {
                if entry.accepts_installation(
                    current.as_ref(),
                    matches!(result, Ok(execution::Response::CallGateway)),
                    state.machine_control.is_current(&connection),
                ) {
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
pub(in crate::server) fn authorize(
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
        (entry.authority.clone(), entry.mcp_epoch.clone())
    };
    let (meta, current) =
        current_authority(state, &grant.parent_session_id).ok_or("permission_denied")?;
    if !issued.0.accepts(&current)
        || (grant.mcp
            && state
                .extensions
                .launches
                .grant_epoch(state, &meta, &current)
                != issued.1)
    {
        return Err("permission_denied");
    }
    let store = state.store.clone().ok_or("capacity")?;
    let ledger =
        Ledger::for_parent(store, &state.service_id, &meta).map_err(|_| "permission_denied")?;
    Ok((meta, ledger))
}

/// The parent's live MCP grant and the execution connection it was
/// acknowledged on: what a view scope relays under (docs/widget-apps-mcp.md §7).
pub(in crate::server) fn mcp_parent(
    state: &AppState,
    parent: &str,
) -> Option<(Grant, ConnectionToken)> {
    let (grant, connection) = {
        let grants = state.managed_calls.grants.lock();
        let entry = grants.get(parent).filter(|entry| entry.grant.mcp)?;
        (entry.grant.clone(), entry.installed_on.clone()?)
    };
    authorize(state, &connection, &grant)
        .is_ok()
        .then_some((grant, connection))
}

/// Report only a live grant acknowledged on its original execution connection.
pub(in crate::server) fn mcp_ready(state: &AppState, parent: &str) -> bool {
    mcp_parent(state, parent).is_some()
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
    if matches!(action, Action::AgentInvoke { .. }) && !grant.agent_actions {
        return error("unsupported_capability", "not_submitted");
    }
    let (meta, ledger) = match authorize(state, connection, &grant) {
        Ok(authorized) => authorized,
        // Authorization precedes admission, so a refusal proves non-admission.
        Err(code) => return error(code, "not_submitted"),
    };
    if let Action::Mcp { request } = action {
        return super::super::mcp_bindings::relay::handle(state, connection, &grant, *request)
            .await;
    }
    dispatch(state, &meta, &ledger, &grant, action).await
}

/// Act for an already authorized parent: Agent API actions, then calls.
async fn dispatch(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
    ledger: &Ledger,
    grant: &Grant,
    action: Action,
) -> Value {
    match action {
        Action::Mcp { .. } => error("invalid_contract", "not_submitted"),
        Action::AgentStatus {} => super::super::agent_api::status(state, meta, grant),
        Action::AgentInvoke {
            action,
            arguments,
            operation,
        } => {
            super::super::agent_api::invoke(
                state,
                meta,
                ledger,
                &action,
                arguments,
                operation.as_deref(),
            )
            .await
        }
        Action::AgentRead {
            code,
            language,
            format,
            timeout_ms,
        } => {
            super::super::agent_api::read(
                state,
                meta,
                ledger,
                super::super::agent_api::Program {
                    code,
                    typescript: language.as_deref() != Some("js"),
                    module: format.as_deref() == Some("module"),
                    timeout_ms: timeout_ms.unwrap_or(crate::agent_api::DEFAULT_TIMEOUT_MS),
                },
            )
            .await
        }
        action => dispatch_calls(state, meta, ledger, action).await,
    }
}

/// The managed-call actions alone. Agent API programs observe calls through
/// this, never through [`dispatch`], so a program cannot start another one.
pub(in crate::server) async fn dispatch_calls(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
    ledger: &Ledger,
    action: Action,
) -> Value {
    dispatch_calls_from(state, meta, ledger, action, &CallOrigin::Gateway).await
}

/// [`dispatch_calls`] for a caller with a structured identity the accepted
/// Call's transcript anchor links back to (an MCP `calls.start`).
pub(in crate::server) async fn dispatch_calls_from(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
    ledger: &Ledger,
    action: Action,
    origin: &CallOrigin,
) -> Value {
    match super::super::superagent::restricted(state, meta).await {
        Ok(false) => {}
        Ok(true) => return error("superagent_scope_denied", "not_submitted"),
        Err(code) => return error(code, "not_submitted"),
    }
    match action {
        Action::Capabilities {} => capabilities(state, meta, ledger).await,
        Action::Start {
            provider,
            request,
            wait_ms,
            preset,
        } => {
            start(
                state,
                meta,
                ledger,
                &provider,
                *request,
                wait_ms,
                preset.as_deref(),
                origin,
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
        } => match wait(state, ledger, &call_id, timeout_ms).await {
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
        Action::Cancel { call_id } => cancel(state, ledger, &call_id).await,
        Action::Mcp { .. }
        | Action::AgentStatus {}
        | Action::AgentRead { .. }
        | Action::AgentInvoke { .. } => error("invalid_contract", "not_submitted"),
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

/// On-demand native capability discovery. No reviewer prompt, preset catalog,
/// quota refresh or external Provider request is needed to ask for an image.
pub(in crate::server) async fn image_capabilities(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    ledger: &Ledger,
    capability: &str,
    if_revision: Option<&str>,
) -> Value {
    let profile = cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1;
    if !profile.capabilities().contains(&capability) {
        return error("unsupported_capability", "not_submitted");
    }
    let policy = agent_tools::effective(state, meta).calls;
    let mut providers = Vec::new();
    for provider in agent_tools::CALL_TARGETS {
        let target = policy
            .targets
            .iter()
            .find(|target| target.agent == provider);
        let (runtime, readiness) = child_runtime(
            state,
            meta.owner_user_id.as_deref(),
            &ledger.placement().machine_id,
            Some(meta.machine_id.as_str()),
            provider,
            profile,
        )
        .await;
        let reason =
            agent_tools::image_refusal(&policy, &meta.provider, provider).or(readiness.reason);
        providers.push(
            json!({"id":provider,"available":reason.is_none() && readiness.available,
            "reason":reason,"runtime_machine_id":runtime,
            "version":readiness.generation.as_ref().map(|g| &g.version),
            "generation":readiness.generation.as_ref().map(|g| &g.digest),
            "preset":target.and_then(|t| t.preset.as_ref())}),
        );
    }
    let mut value = json!({"capability":capability,"providers":providers,
        "action":"calls.start","receipt_domain":"calls",
        "input":{"instruction":"required","files":"up to 4 relative PNG/JPEG/WebP paths; only these are copied",
            "child_session_id":"optional: continue this parent's image conversation; edit needs files or continuation",
            "provider":"auto or an available provider","preset":"optional signed preset"},
        "output":"calls.result: verified artifacts with path on execution machine; no inline bytes",
        "limits":{"max_images":crate::managed_calls::media::MAX_IMAGES,"max_image_bytes":crate::managed_calls::media::MAX_IMAGE_BYTES},
        "availability":"installed signed capability; native availability and quota are checked by the Provider at execution"});
    let revision = crate::managed_calls::canonical_digest(&value);
    if if_revision == Some(revision.as_str()) {
        return json!({"revision":revision,"unchanged":true});
    }
    value["revision"] = json!(revision);
    value
}

/// Cached quota evidence for review selection: legacy Service collectors for
/// legacy defaults, each named default account's own usage read otherwise.
async fn quota_evidence(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    accounts: &[String],
    named: &[crate::provider_account::ProviderAccountId],
) -> Value {
    if !state.product_auth_enabled || meta.owner_user_id.is_some() {
        let now = chrono::Utc::now().timestamp_millis();
        let (mut quota, interval) = state
            .usage
            .cached_view(|snapshot, _, _| {
                (
                    super::selection::quota(snapshot, accounts, now),
                    snapshot.refresh_interval_ms,
                )
            })
            .await;
        if let (Some(owner), Some(list)) = (
            meta.owner_user_id.as_deref(),
            quota["accounts"].as_array_mut(),
        ) {
            for id in named {
                let read = super::super::provider_accounts::usage::read_one(state, owner, id)
                    .await
                    .unwrap_or_else(|code| json!({"account_id":id,"reason":code}));
                list.push(super::selection::named_quota(&read, now, interval));
            }
        }
        quota
    } else {
        json!({"status":"permission_denied"})
    }
}

async fn capabilities(state: &AppState, meta: &crate::core::SessionMeta, ledger: &Ledger) -> Value {
    let placement = ledger.placement();
    let policy = agent_tools::effective(state, meta).calls;
    let mut providers = Vec::new();
    let mut read_only_providers = Vec::new();
    let mut image_providers = Vec::new();
    let mut named = Vec::new();
    for provider in agent_tools::CALL_TARGETS {
        let target = policy
            .targets
            .iter()
            .find(|target| target.agent == provider);
        let (runtime, readiness) = child_runtime(
            state,
            meta.owner_user_id.as_deref(),
            &placement.machine_id,
            Some(meta.machine_id.as_str()),
            provider,
            cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
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
        let native_capabilities: Vec<&str> = readiness
            .generation
            .as_ref()
            .and_then(|g| {
                state
                    .provider_catalog
                    .package(provider, &g.version, &g.digest)
            })
            .map(|p| {
                p.manifest
                    .runtime
                    .managed_profiles
                    .keys()
                    .flat_map(|profile| profile.capabilities().iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        if available {
            read_only_providers.push(provider);
        }
        // Through the image profile, as scoped discovery and admission do: a
        // Machine can run read-only work and still refuse images.
        if native_capabilities.contains(&"image.generate")
            && agent_tools::image_refusal(&policy, &meta.provider, provider).is_none()
            && child_runtime(
                state,
                meta.owner_user_id.as_deref(),
                &placement.machine_id,
                Some(meta.machine_id.as_str()),
                provider,
                cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1,
            )
            .await
            .1
            .available
        {
            image_providers.push(provider);
        }
        // What admission ranks: the generation this runtime would launch.
        let runnable_presets = readiness.presets(state, provider);
        providers.push(json!({
            "id": provider,
            "alias": if provider == "codex" { "codex" } else { "claude" },
            "available": available,
            "native_capabilities":native_capabilities,
            "runtime_machine_id": available.then_some(runtime),
            "reason": reason,
            "version": readiness.generation.as_ref().map(|generation| &generation.version),
            "preset": target.and_then(|target| target.preset.as_ref()),
            "options": target.and_then(|target| target.options.as_ref()),
            // A named default's own usage, never the Provider-wide legacy card.
            "quota_account": if readiness.account.is_default() { readiness.generation.as_ref().and_then(|generation| state.provider_catalog.account_usage_provider(provider, &generation.version, &generation.digest)) } else { named.push(readiness.account.clone()); Some(readiness.account.as_str().to_owned()) },
            // `--preset` choices; the session's Tools choose otherwise.
            "presets": runnable_presets,
            "review_candidates": super::selection::candidates(&runnable_presets, provider, &policy.review, false).iter().filter_map(|p| p["id"].as_str()).collect::<Vec<_>>(),
            "access": ["read-only"],
            "output": if readiness.structured_output { json!(["text","json_schema"]) } else { json!(["text"]) },
            "conversation": ["fresh","continue"],
        }));
    }
    let accounts: Vec<String> = providers
        .iter()
        .filter_map(|provider| provider["quota_account"].as_str().map(str::to_owned))
        .filter(|account| !named.iter().any(|id| id.as_str() == account))
        .collect();
    let quota = quota_evidence(state, meta, &accounts, &named).await;
    // Account observations must not outlive the parent/user permission that
    // was checked before the readiness and cache awaits.
    if let Err(code) =
        super::super::agent_api::read_parent(&state.hub, state.store.as_ref(), meta).await
    {
        return error(code, "not_submitted");
    }
    json!({
        "schema": 1,
        "state": "ok",
        "enabled": policy.enabled,
        "default": policy.default,
        "review_selection": super::selection::GUIDANCE,
        "review_preferences": policy.review,
        "review_input_reuse": true,
        "quota": quota,
        "providers": providers,
        "capabilities": [
            {"id": "read_only", "tasks": ["review", "design_review", "analysis"],
             "enabled": policy.enabled, "providers": read_only_providers},
            {"id": "image", "tasks": ["image.generate", "image.edit"],
             "enabled": policy.enabled && policy.images, "self": policy.self_images,
             "providers": image_providers},
        ],
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

/// The first candidate whose child can run now, in preference order; else
/// the first refusal.
async fn first_ready(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    ledger: &Ledger,
    targets: Vec<agent_tools::CallTarget>,
    profile: cowboy_provider_sdk::ManagedRuntimeProfile,
) -> Result<(agent_tools::CallTarget, Option<(String, String)>), &'static str> {
    let mut refusal = None;
    for target in targets {
        let (_, readiness) = child_runtime(
            state,
            meta.owner_user_id.as_deref(),
            &ledger.placement().machine_id,
            Some(ledger.runtime_machine_id()),
            &target.agent,
            profile,
        )
        .await;
        if readiness.available {
            let generation = readiness
                .generation
                .map(|generation| (generation.version, generation.digest));
            return Ok((target, generation));
        }
        refusal.get_or_insert(readiness.reason.unwrap_or("provider_unavailable"));
    }
    Err(refusal.unwrap_or("provider_unavailable"))
}

#[allow(clippy::too_many_arguments)] // The admission inputs plus the anchor's origin.
async fn start(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
    ledger: &Ledger,
    requested: &str,
    request: Request,
    wait_ms: u64,
    preset: Option<&str>,
    origin: &CallOrigin,
) -> Value {
    // The caller's gateways time this exchange from its receipt; approval,
    // readiness and admission spend the same budget as the observation.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
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
            return envelope(&wait_started(state, ledger, &original, deadline).await);
        }
        Ok(None) => {}
        Err(_) => return error("capacity", "not_submitted"),
    }
    let policy = agent_tools::effective(state, meta).calls;
    // Before any approval: a person changes this in Tools, and neither Allow
    // once nor Allow for session may override it.
    if let Some(code) = agent_tools::capability_refusal(&policy, request.purpose.is_image()) {
        return error(code, "not_submitted");
    }
    let own_family = agent_tools::own_family(&policy, &request);
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
    let (mut targets, selection) =
        match agent_tools::choose(&policy, &meta.provider, requested, own_family) {
            agent_tools::Choice::Candidates { targets, selection } => (targets, selection),
            agent_tools::Choice::Refused(code @ ("calls_disabled" | "policy_denied")) => {
                match await_approval(state, meta, &policy, requested, &request, code) {
                    Ok(admitted) => admitted,
                    Err(refusal) => return refusal,
                }
            }
            agent_tools::Choice::Refused(code) | agent_tools::Choice::Closed(code) => {
                return error(code, "not_submitted");
            }
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
        for target in &mut targets {
            target.preset = Some(preset.to_owned());
            target.options = None;
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
    let (mut target, generation) =
        match first_ready(state, meta, ledger, targets, request.purpose.profile()).await {
            Ok(ready) => ready,
            Err(reason) => return error(reason, "not_submitted"),
        };
    // Discovery and admission use the generation the claim will pin, not the
    // latest Catalog: an id may be absent or name another model on it.
    let runnable = agent_tools::presets_on(
        state,
        &target.agent,
        generation.as_ref().map(|(v, d)| (v.as_str(), d.as_str())),
    );
    if preset.is_some_and(|id| {
        !runnable
            .as_array()
            .is_some_and(|all| all.iter().any(|p| p["id"].as_str() == Some(id)))
    }) {
        return error("preset_unavailable", "not_submitted");
    }
    if continued.is_none()
        && matches!(
            request.purpose,
            crate::managed_calls::Purpose::Review | crate::managed_calls::Purpose::DesignReview
        )
    {
        let high_risk = request
            .labels
            .get(crate::managed_calls::review::RISK)
            .is_some_and(|risk| risk == "high");
        let choices =
            super::selection::candidates(&runnable, &target.agent, &policy.review, high_risk);
        // Generic Tools pins apply to ordinary calls. Review uses its own
        // ordered list unless this invocation explicitly names a signed preset.
        target.preset = match super::selection::review_preset(&choices, preset) {
            Ok(selected) => Some(selected),
            Err(code) => return error(code, "not_submitted"),
        };
        target.options = None;
    }
    let call_id = random_id("call");
    let child_id = state.supervisor.reserve_session_id();
    let choice = crate::managed_calls::service::Choice {
        preset: target.recorded_preset(),
        selection,
        generation: generation.filter(|_| request.purpose.is_review()),
    };
    // Continuation admission holds the child lifecycle fence while it
    // rechecks the child and inserts, so retirement cannot interleave.
    let fence = match &request.conversation {
        Conversation::Continue { child_session_id } => {
            let fence = state.managed_calls.child_lifecycle.lock().await;
            if state
                .managed_calls
                .retiring
                .lock()
                .contains(child_session_id)
                || state
                    .hub
                    .session_info(child_session_id)
                    .is_none_or(|child| child.meta.closing)
            {
                return error("conversation_unavailable", "not_submitted");
            }
            Some(fence)
        }
        Conversation::Fresh {} => None,
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
    // The inserted call now protects its child; never hold the fence while
    // waiting for the call to start.
    drop(fence);
    // I8: one transcript anchor per accepted Call, after the ledger commit. A
    // crash before this push is repaired at the next start. Only the admission
    // that inserted the record writes it: an identical concurrent admission
    // observes the same record and must not replace its provenance.
    if record.call_id == call_id {
        let approval_request_id = state
            .managed_calls
            .approvals
            .admitted(&meta.id, &request.request_id);
        super::super::background_tasks::push_call_anchor(
            &state.hub,
            &meta.id,
            &record.call_id,
            &record.request_id,
            origin,
            approval_request_id.as_deref(),
        );
    }
    super::runner::ensure_runner(state, &record);
    state.managed_calls.notify();
    envelope(&wait_started(state, ledger, &record, deadline).await)
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
                .record_occurrence(crate::task_notifications::Occurrence::approval(
                    &meta.id,
                    &request.request_id,
                ));
            state
                .hub
                .broadcast_call_approval(&meta.id, approvals.view(&meta.id, &meta.provider));
            let mut refusal = error(code, "not_submitted");
            refusal["approval"] = json!("pending");
            Err(refusal)
        }
        super::approval::Gate::Approved => {
            let once = agent_tools::allow_once(policy.clone(), &meta.provider, requested);
            match agent_tools::choose(&once, &meta.provider, requested, false) {
                agent_tools::Choice::Candidates { targets, selection } => Ok((targets, selection)),
                agent_tools::Choice::Refused(code) | agent_tools::Choice::Closed(code) => {
                    Err(error(code, "not_submitted"))
                }
            }
        }
    }
}

/// Bounded initial observation: return once the call left the launch
/// phases, reached a terminal state, or the wait elapsed.
async fn wait_started(
    state: &AppState,
    ledger: &Ledger,
    record: &Record,
    deadline: tokio::time::Instant,
) -> Record {
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
            next.finished_at_ms = Some(next.updated_at_ms);
        }
        if store.advance_managed_call(&record, &next).await.ok()? {
            // Withdraw a still-queued prompt at once, so no drain can start it
            // before the runner's next poll. Withdrawing it proves no native
            // turn started, so the stop completes now (Running may only end
            // through Stopping); a lost write leaves the runner's ordinary stop.
            if next.state == State::Running
                && state
                    .hub
                    .discard_managed_prompt(&record.child_session_id, call)
            {
                next = finish_unstarted(store, next).await;
            }
            super::runner::ensure_runner(state, &next);
            state.managed_calls.notify();
            return Some(next);
        }
    }
    None
}

/// Running → Stopping → Cancelled for a call whose prompt was withdrawn
/// before any worker saw it, committed in one transaction. Returns the latest
/// record written.
async fn finish_unstarted(store: &crate::store::Store, running: Record) -> Record {
    let steps = unstarted_cancellation(&running);
    if store
        .advance_managed_call_through(&running, &steps)
        .await
        .unwrap_or(false)
    {
        steps.into_iter().last().unwrap_or(running)
    } else {
        running
    }
}

/// The Stopping and Cancelled records of a running call whose prompt was
/// withdrawn before any worker saw it.
pub(super) fn unstarted_cancellation(running: &Record) -> Vec<Record> {
    let mut steps = Vec::with_capacity(2);
    let mut current = running.clone();
    for phase in [State::Stopping, State::Cancelled] {
        let mut next = current.clone();
        next.state = phase;
        next.revision = current.revision + 1;
        next.updated_at_ms = chrono::Utc::now()
            .timestamp_millis()
            .max(current.updated_at_ms);
        if phase.terminal() {
            next.finished_at_ms = Some(next.updated_at_ms);
        }
        steps.push(next.clone());
        current = next;
    }
    steps
}

/// Where a managed child lives: its parent and the Machine holding its
/// snapshot (and a remote child's environment).
fn child_location(meta: &crate::core::SessionMeta) -> Option<(String, String)> {
    let binding = meta.execution_binding.as_ref()?;
    let identity = binding.managed()?;
    // The snapshot (and a remote child's environment) is on the execution
    // Machine; a remote child's worker is stopped on its runtime Machine.
    let execution_machine = match binding.managed_child() {
        Some(local) => local.machine_id,
        None => binding.decode().ok()?.environment.machine_id,
    };
    Some((identity.parent_session_id, execution_machine))
}

/// Delete one managed child conversation. An active call keeps its child; the
/// call ledger (and its result summary) belongs to the parent and remains.
pub(in crate::server) fn delete_child(state: &Arc<AppState>, child: &str) -> Result<(), String> {
    let info = state
        .hub
        .session_info(child)
        .ok_or("Session no longer exists")?;
    child_location(&info.meta).ok_or("Not a managed child")?;
    if state.store.is_none() {
        return Err("Managed calls require durable storage".to_owned());
    }
    let state = Arc::clone(state);
    let child = child.to_owned();
    // Deletion consults durable call state first, so it runs asynchronously
    // like other target-owned closures.
    tokio::spawn(async move {
        if matches!(
            retire_child(&state, &child, |_| true).await,
            Retirement::Active
        ) {
            state.hub.broadcast_error(
                Some(child.clone()),
                "Cancel this child's managed call before deleting it".to_owned(),
            );
        }
    });
    Ok(())
}

enum Retirement {
    Closed,
    /// Its newest call is active, or not yet eligible for retirement.
    Active,
    /// Marked retiring; the snapshot is not confirmed closed yet.
    Pending,
}

/// Retire one child when the eligibility check accepts its newest terminal
/// call.
///
/// Under the child lifecycle fence this rechecks the newest call and marks the
/// child retiring, so continuation admission (which holds the same fence while
/// it inserts) either sees the mark and refuses, or wins and makes the newest
/// call active. The session is removed only after its Machine confirms the
/// snapshot is closed; otherwise it stays retiring and a later sweep retries.
async fn retire_child(
    state: &Arc<AppState>,
    child: &str,
    eligible: impl Fn(&Record) -> bool,
) -> Retirement {
    let Some(store) = state.store.clone() else {
        return Retirement::Active;
    };
    let Some((parent, machine)) = state
        .hub
        .session_info(child)
        .and_then(|info| child_location(&info.meta))
    else {
        return Retirement::Active;
    };
    {
        let _fence = state.managed_calls.child_lifecycle.lock().await;
        let retiring = state.managed_calls.retiring.lock().contains(child);
        if !retiring {
            match store.managed_child(&parent, child).await {
                Ok(Some(record)) if record.state.terminal() && eligible(&record) => {}
                Ok(None) => {}
                _ => return Retirement::Active,
            }
            state.managed_calls.retiring.lock().insert(child.to_owned());
        }
    }
    state.supervisor.delete_session(child);
    // Retry only explicit refusals (another holder of the child lock). A
    // transport error or timeout may leave the first close still queued on
    // the Machine; adding more would only pile up behind it.
    for attempt in 0..CLOSE_ATTEMPTS {
        match execution_action(
            state,
            &machine,
            execution::Action::CloseManagedChild {
                child_session_id: child.to_owned(),
            },
        )
        .await
        {
            Ok(execution::Response::Closed) => {
                state.hub.delete_session(child);
                state.managed_calls.retiring.lock().remove(child);
                return Retirement::Closed;
            }
            Ok(_) if attempt + 1 < CLOSE_ATTEMPTS => {
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            }
            _ => break,
        }
    }
    tracing::warn!(child = %child, machine = %machine, "managed child snapshot not closed; will retry");
    Retirement::Pending
}

/// Whether a child's newest call ended long enough ago to retire it.
fn child_expired(record: &Record, now_ms: i64) -> bool {
    record.state.terminal()
        && now_ms.saturating_sub(record.finished_at_ms.unwrap_or(record.updated_at_ms))
            >= i64::try_from(CHILD_RETENTION.as_millis()).unwrap_or(i64::MAX)
}

/// Retire child conversations whose newest call finished before the
/// retention window, or whose parent is gone, one at a time so cleanup never
/// floods a Machine. Children still marked retiring are retried as well.
async fn reap_expired_children(state: &Arc<AppState>) {
    let Some(store) = state.store.clone() else {
        return;
    };
    let now = chrono::Utc::now().timestamp_millis();
    let mut closed = 0;
    let mut attempted = 0;
    for meta in state.hub.session_list() {
        if attempted >= CHILD_REAP_BATCH {
            break;
        }
        if matches!(
            meta.status,
            crate::core::Status::Busy | crate::core::Status::Starting
        ) {
            continue;
        }
        let Some((parent, _)) = child_location(&meta) else {
            continue;
        };
        let orphaned = state.hub.session_info(&parent).is_none();
        let retiring = state.managed_calls.retiring.lock().contains(&meta.id);
        if !retiring && !orphaned {
            // Cheap prefilter; retire_child rechecks under the fence.
            match store.managed_child(&parent, &meta.id).await {
                Ok(Some(record)) if child_expired(&record, now) => {}
                _ => continue,
            }
        }
        attempted += 1;
        let eligible = |record: &Record| orphaned || child_expired(record, now);
        if matches!(
            retire_child(state, &meta.id, eligible).await,
            Retirement::Closed
        ) {
            closed += 1;
        }
    }
    if attempted > 0 {
        tracing::info!(
            attempted,
            closed,
            "retired expired managed child conversations"
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record(state: &str, finished_at_ms: Option<i64>, updated_at_ms: i64) -> Record {
        serde_json::from_value(json!({
            "schema": 1, "call_id": "call-1", "request_id": "request-1",
            "request_digest": "sha256:00", "provider": "codex", "purpose": "analysis",
            "access": "read-only", "labels": {},
            "placement": {"service_id": "svc-1", "parent_session_id": "parent-1",
                "parent_revision": "sha256:00", "machine_id": "hawk",
                "workspace_id": "columbus", "cwd": "/tmp"},
            "child_session_id": "child-1", "state": state, "revision": 2,
            "created_at_ms": 0, "updated_at_ms": updated_at_ms, "started_at_ms": null,
            "finished_at_ms": finished_at_ms
        }))
        .unwrap()
    }

    #[test]
    fn only_children_finished_before_the_retention_window_expire() {
        let window = i64::try_from(CHILD_RETENTION.as_millis()).unwrap();
        let now = window * 2;
        assert!(child_expired(
            &record("completed", Some(now - window), now),
            now
        ));
        assert!(child_expired(
            &record("failed", None, now - window - 1),
            now
        ));
        // Still continuable inside the window.
        assert!(!child_expired(
            &record("completed", Some(now - window + 1), now),
            now
        ));
        // An active call keeps its child however old it is.
        assert!(!child_expired(&record("running", Some(0), 0), now));
    }
}

mod admission_text;
pub(super) use admission_text::*;
