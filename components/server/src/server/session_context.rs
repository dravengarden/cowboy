//! Editable context and fresh-agent handoff. Preferences have no runtime side
//! effects; only an explicit, revision-fenced send can consume them.
use super::*;
use crate::core::SessionInfo;
use crate::session_context::{Context, Delivery, Injection, Link, NextAgent, Source};

mod fork;
mod inheritance;
pub(super) use fork::{duplicate, fork};

// Serializes short durable mutations and creation admission, including requests
// coming through MCP. A dropped HTTP request does not cancel the transaction.
type ContextLocks = std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>;
static WRITES: std::sync::LazyLock<parking_lot::Mutex<ContextLocks>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(ContextLocks::new()));

fn write_lock(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = WRITES.lock();
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(id).and_then(std::sync::Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(id.to_owned(), Arc::downgrade(&lock));
    lock
}

fn owned(state: &AppState, principal: &ProductPrincipal, id: &str) -> Result<SessionInfo, String> {
    state
        .hub
        .session_info(id)
        .filter(|info| {
            !info.meta.system
                && !info.meta.closing
                && !state.hub.is_managed_child(id)
                && (!state.product_auth_enabled
                    || info.meta.owner_user_id.as_deref() == Some(principal.user_id.as_str()))
        })
        .ok_or_else(|| "Session unavailable".into())
}

fn current_authority(state: &AppState, auth: &AuthenticatedProductRequest) -> Result<(), String> {
    if state.product_auth_enabled
        && !auth
            .permissions
            .as_ref()
            .is_some_and(|permissions| permissions.current(&state.hub, &auth.principal.username))
    {
        return Err("Permissions changed; reconnect before editing context".into());
    }
    Ok(())
}

pub(super) fn validate_sources(
    state: &AppState,
    principal: &ProductPrincipal,
    ids: &[String],
) -> Result<(), String> {
    crate::session_context::validate_sources(ids)?;
    for id in ids {
        owned(state, principal, id)?;
    }
    Ok(())
}

fn capture(
    state: &AppState,
    principal: &ProductPrincipal,
    ids: &[String],
) -> Result<Vec<Source>, String> {
    validate_sources(state, principal, ids)?;
    ids.iter()
        .map(|id| {
            Ok(Source {
                session_id: id.clone(),
                before: state
                    .hub
                    .last_event_seq(id)
                    .ok_or("Session unavailable")?
                    .saturating_add(1),
            })
        })
        .collect()
}

fn delivered_links(state: &AppState, sources: &[Source]) -> Vec<Link> {
    sources
        .iter()
        .map(|source| Link {
            session_id: source.session_id.clone(),
            before: source.before,
            title: state
                .hub
                .session_info(&source.session_id)
                .map(|info| info.meta.title)
                .unwrap_or_default(),
        })
        .collect()
}

async fn save(state: &AppState, id: &str, context: &Context) -> Result<(), String> {
    let store = state.store.as_ref().ok_or("Context storage unavailable")?;
    let key = crate::session_context::key(id);
    let expected = state.hub.setting(&key);
    let value = serde_json::to_value(context).expect("context serializes");
    if !store
        .compare_and_put_setting(&key, expected.as_ref(), &value)
        .await
        .map_err(|_| "Context storage unavailable")?
    {
        return Err("Context changed; refresh before trying again".into());
    }
    state.hub.load_settings(vec![(key, value)]);
    state.hub.context_state_changed(id);
    crate::session_context::announce(&state.hub, id, context);
    Ok(())
}

pub(super) async fn initialize(
    state: &AppState,
    principal: &ProductPrincipal,
    id: &str,
    sources: Vec<String>,
    initial_cmid: Option<String>,
) -> Result<(), String> {
    if sources.is_empty() {
        return Ok(());
    }
    let snapshots = capture(state, principal, &sources)?;
    let links = if initial_cmid.is_some() {
        delivered_links(state, &snapshots)
    } else {
        Vec::new()
    };
    let context = Context {
        revision: 1,
        applied_revision: u64::from(initial_cmid.is_some()),
        sources,
        delivery: initial_cmid.as_ref().map(|cmid| Delivery {
            operation: format!("create-{id}"),
            fingerprint: crate::admin::hex_sha256(cmid.as_bytes()),
            session_id: id.to_owned(),
            state: "preparing".into(),
            error: None,
            source_trashed: false,
        }),
        injection: initial_cmid.map(|cmid| Injection {
            cmid,
            sources: snapshots,
            follows_first_prompt: true,
        }),
        links,
        ..Context::default()
    };
    save(state, id, &context).await
}

/// Durably name the prompt that carries linked context before it reaches a
/// runtime. Without that record the prompt stays queued: links are never sent
/// on evidence that a restart could lose. Only one dispatch per session runs.
pub(super) async fn claim_carrier(
    state: &AppState,
    id: &str,
    cmid: Option<&str>,
) -> Result<(), String> {
    claim_carrier_in(&state.hub, state.store.as_ref(), id, cmid).await
}

async fn claim_carrier_in(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    id: &str,
    cmid: Option<&str>,
) -> Result<(), String> {
    let unavailable = "Linked context could not be recorded; the message stays queued";
    if let Some(promoted) = crate::session_context::promotion(hub, id, cmid) {
        let store = store.ok_or(unavailable)?;
        let key = crate::session_context::key(id);
        let value = serde_json::to_value(&promoted).expect("context serializes");
        if !store
            .compare_and_put_setting(&key, hub.setting(&key).as_ref(), &value)
            .await
            .unwrap_or(false)
        {
            return Err(unavailable.into());
        }
        hub.load_settings(vec![(key, value)]);
        crate::session_context::announce(hub, id, &promoted);
    }
    let Some((key, expected, value)) = crate::session_context::carrier_claim(hub, id, cmid) else {
        return Ok(());
    };
    let store = store.ok_or(unavailable)?;
    let committed = match store
        .compare_and_put_setting(&key, expected.as_ref(), &value)
        .await
    {
        Ok(true) => true,
        // A lost reply may already have committed this exact record.
        _ => matches!(
            store
                .compare_and_put_setting(&key, Some(&value), &value)
                .await,
            Ok(true)
        ),
    };
    if committed {
        hub.load_settings(vec![(key, value)]);
        return Ok(());
    }
    // Refresh the stored record so the requeued prompt is decided on it.
    if let Ok(settings) = store.load_settings().await
        && let Some((key, stored)) = settings.into_iter().find(|(stored, _)| *stored == key)
    {
        hub.load_settings(vec![(key, stored)]);
    }
    Err(unavailable.into())
}

/// Publish the preference revision only after registration. An initial prompt
/// stays fenced until its ordinary submission has been admitted.
pub(super) async fn initialized(state: &AppState, id: &str) -> Result<(), String> {
    let mut context = crate::session_context::read(&state.hub, id)?;
    if let Some(delivery) = &mut context.delivery
        && delivery.operation == format!("create-{id}")
        && delivery.state == "preparing"
    {
        delivery.state = "accepted".into();
        advance_applied(&mut context)?;
        save(state, id, &context).await?;
    } else if context.revision > 0 {
        crate::session_context::announce(&state.hub, id, &context);
    }
    Ok(())
}

fn view(
    state: &AppState,
    principal: &ProductPrincipal,
    id: &str,
) -> Result<serde_json::Value, String> {
    owned(state, principal, id)?;
    let context = crate::session_context::read(&state.hub, id)?;
    let links: Vec<_> = context.known_links().iter().map(|link| {
        match owned(state, principal, &link.session_id) {
            Ok(info) => serde_json::json!({"id":link.session_id,"available":true,"title":info.meta.title,"provider":info.meta.provider,"provider_account_id":info.meta.provider_account_id,"before":link.before}),
            Err(_) => serde_json::json!({"id":link.session_id,"available":false,"title":link.title}),
        }
    }).collect();
    let sources: Vec<_> = context.sources.iter().map(|source| {
        match owned(state, principal, source) {
            Ok(info) => serde_json::json!({"id":source,"available":true,"title":info.meta.title,"provider":info.meta.provider,"provider_account_id":info.meta.provider_account_id,"workspace":info.meta.workspace_name}),
            Err(_) => serde_json::json!({"id":source,"available":false}),
        }
    }).collect();
    let mut delivery = context.delivery.clone();
    // A reserved intent is never an instruction to replay after a restart.
    if let Some(delivery) = &mut delivery {
        delivery.fingerprint.clear();
    }
    Ok(serde_json::json!({
        "revision":context.revision,"applied_revision":context.applied_revision,
        "sources":context.sources,"source_sessions":sources,"links":links,"pending_links":crate::session_context::pending_links(&context),"next_agent":context.next_agent,
        "delivery":delivery,"max_sources":crate::session_context::MAX_SOURCES,
    }))
}

fn failure(error: String) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({"error":error})),
    )
        .into_response()
}

pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
) -> Response {
    match view(&state, &auth.principal, &id) {
        Ok(value) => Json(value).into_response(),
        Err(error) => failure(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Edit {
    revision: u64,
    sources: Vec<String>,
    #[serde(default)]
    next_agent: Option<NextAgent>,
    /// Explicit acknowledgement after inspecting an uncertain send. It clears
    /// only the pending preference; it never cancels or replays its destination.
    #[serde(default)]
    resolve_operation: Option<String>,
}

/// Links wait for the next prompt without holding anything. Only a new agent
/// needs this session settled first.
fn editable(info: &SessionInfo, context: &Context, new_agent: bool) -> Result<(), String> {
    if context
        .delivery
        .as_ref()
        .is_some_and(|delivery| matches!(delivery.state.as_str(), "preparing" | "unknown"))
    {
        return Err("A send is unresolved. Check its destination before changing context; do not send it again.".into());
    }
    if new_agent
        && (info.queue_count > 0 || context.injection.is_some() && info.meta.status == Status::Busy)
    {
        return Err(
            "Wait for the submitted message or resolve the queue before changing context".into(),
        );
    }
    Ok(())
}

fn validate_target(target: &NextAgent) -> Result<(), String> {
    for value in [&target.provider, &target.machine_id, &target.cwd] {
        if value.is_empty() || value.len() > 256 || value.contains('\0') {
            return Err("Choose a Provider and registered project".into());
        }
    }
    if target.machine_id == "local" || target.runtime_machine_id.as_deref() == Some("local") {
        return Err(
            "Fresh agents require an enrolled Machine and an isolated managed worktree".into(),
        );
    }
    if target
        .runtime_machine_id
        .as_ref()
        .is_some_and(|id| !crate::managed_calls::valid_id(id))
        || target
            .preset
            .as_ref()
            .is_some_and(|id| id.len() > 128 || id.contains('\0'))
    {
        return Err("Invalid agent selection".into());
    }
    Ok(())
}

pub(super) async fn set(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
    Json(edit): Json<Edit>,
) -> Response {
    let task = tokio::spawn(async move {
        let lock = write_lock(&id);
        let _guard = lock.lock().await;
        current_authority(&state, &auth)?;
        let info = owned(&state, &auth.principal, &id)?;
        let current = crate::session_context::read(&state.hub, &id)?;
        let mut check = current.clone();
        if let Some(operation) = &edit.resolve_operation {
            if !current
                .delivery
                .as_ref()
                .is_some_and(|delivery| &delivery.operation == operation)
                || !edit.sources.is_empty()
                || edit.next_agent.is_some()
            {
                return Err(
                    "Resolve only the exact inspected send, without submitting a replacement"
                        .into(),
                );
            }
            check.delivery = None;
        }
        let links_only = edit.next_agent.is_none();
        if edit.revision != current.revision {
            return Err("Context changed on another device; refresh before saving".into());
        }
        editable(&info, &check, !links_only)?;
        validate_sources(&state, &auth.principal, &edit.sources)?;
        if let Some(target) = &edit.next_agent {
            validate_target(target)?;
            provider_accounts::resolve_for_create(
                &state,
                &auth.principal.user_id,
                &target.provider,
                Some(&target.provider_account_id),
            )
            .await
            .map_err(str::to_owned)?;
        } else if edit.sources.contains(&id) {
            return Err("Linking this session to itself requires a fresh agent".into());
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or("Context revision exhausted")?;
        let links = current.known_links();
        // Staging a new agent leaves the waiting links in place: going back
        // must not move their point.
        let waiting = if links_only {
            let links = delivered_links(&state, &capture(&state, &auth.principal, &edit.sources)?);
            links
                .into_iter()
                .map(|link| {
                    current
                        .waiting
                        .iter()
                        .find(|known| known.session_id == link.session_id)
                        .cloned()
                        .unwrap_or(link)
                })
                .collect()
        } else {
            current.waiting.clone()
        };
        let delivery = current
            .delivery
            .filter(|delivery| matches!(delivery.state.as_str(), "accepted" | "failed"));
        let saved = Context {
            revision,
            applied_revision: if links_only {
                revision
            } else {
                current.applied_revision
            },
            sources: if links_only { Vec::new() } else { edit.sources },
            next_agent: edit.next_agent,
            delivery,
            // A claimed injection may still be retried by its prompt.
            injection: current.injection,
            links,
            waiting,
            workspace_base: current.workspace_base,
        };
        current_authority(&state, &auth)?;
        owned(&state, &auth.principal, &id)?;
        let _queue_guard = state.hub.begin_context_edit(&id, links_only)?;
        save(&state, &id, &saved).await?;
        view(&state, &auth.principal, &id)
    })
    .await;
    match task {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => failure(error),
        Err(_) => failure("Context save outcome is unknown; refresh before retrying".into()),
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Send {
    revision: u64,
    operation: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    content: Vec<serde_json::Value>,
}

fn fingerprint(request: &Send) -> String {
    crate::admin::hex_sha256(&serde_json::to_vec(request).expect("send serializes"))
}

fn replay(context: &Context, request: &Send) -> Result<Option<Delivery>, String> {
    if let Some(previous) = &context.delivery {
        if previous.operation == request.operation {
            if previous.fingerprint != fingerprint(request) {
                return Err("This send id already names another message".into());
            }
            return Ok(Some(previous.clone()));
        }
        if matches!(previous.state.as_str(), "preparing" | "unknown") {
            return Err(
                "The previous send is unresolved; inspect its receipt before sending again".into(),
            );
        }
    }
    Ok(None)
}

fn advance_applied(context: &mut Context) -> Result<(), String> {
    context.revision = context
        .revision
        .checked_add(1)
        .ok_or("Context revision exhausted")?;
    context.applied_revision = context.revision;
    Ok(())
}

async fn creation_error(response: Response) -> String {
    let status = response.status();
    let fallback = format!("Session creation refused (HTTP {})", status.as_u16());
    let Ok(bytes) = axum::body::to_bytes(response.into_body(), 4096).await else {
        return fallback;
    };
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        return value
            .get("error")
            .and_then(|error| error.as_str().or_else(|| error.get("code")?.as_str()))
            .filter(|text| !text.trim().is_empty())
            .map_or(fallback, str::to_owned);
    }
    String::from_utf8(bytes.to_vec())
        .ok()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(fallback)
}

/// A continuation in the source's project on its Machine picks up the work
/// where it stands: a new worktree holding the source's commits and uncommitted
/// changes. The source's own worktree stays intact for Undo and Restore. A
/// session that never ran has no worktree, and an older Machine cannot copy.
fn workspace_base(
    state: &AppState,
    source: &crate::core::SessionMeta,
    target: &NextAgent,
) -> Option<String> {
    let (machine_id, _, project) = fork::placement(source).ok()?;
    (machine_id == target.machine_id
        && project == target.cwd
        && state.hub.agent_session_id_for_resume(&source.id).is_some()
        && state.machine_control.machine_supports(
            &machine_id,
            crate::machine_protocol::NATIVE_FORK_PROTOCOL_VERSION,
        ))
    .then(|| source.id.clone())
}

async fn prepare_target(
    state: &Arc<AppState>,
    auth: &AuthenticatedProductRequest,
    id: &str,
    context: &Context,
    delivery: &Delivery,
) -> Result<(), String> {
    let target_id = &delivery.session_id;
    if let Some(target) = &context.next_agent {
        validate_target(target)?;
        let source = owned(state, &auth.principal, id)?.meta;
        inheritance::preflight(state, auth, &source, target).await?;
        let actor = folder_actor(state, &auth.principal);
        state.hub.continuation_folder(id, &actor)?;
        save(
            state,
            target_id,
            &Context {
                revision: 1,
                sources: context.sources.clone(),
                delivery: Some(delivery.clone()),
                workspace_base: workspace_base(state, &source, target),
                ..Context::default()
            },
        )
        .await?;
        let create = NewSessionRequest {
            provider: target.provider.clone(),
            provider_account_id: Some(target.provider_account_id.clone()),
            machine_id: target.machine_id.clone(),
            runtime_machine_id: target.runtime_machine_id.clone(),
            cwd: Some(target.cwd.clone()),
            preset: target.preset.clone(),
            folder_id: None,
            system: false,
            origin: SessionOrigin::Web,
            initial_prompt: None,
            context_sources: vec![],
            reserved_id: Some(target_id.clone()),
            retained_auth_generation: None,
        };
        let response =
            api_new_session(State(state.clone()), Extension(auth.clone()), Json(create)).await;
        if !response.status().is_success() {
            return Err(creation_error(response).await);
        }
        current_authority(state, auth)?;
        let source = owned(state, &auth.principal, id)?.meta;
        let destination = owned(state, &auth.principal, target_id)?.meta;
        if target.inherit_settings {
            inheritance::apply(state, &source, &destination, target)?;
        }
        state
            .hub
            .place_continuation(id, target_id, target.inherit_settings, &actor)?;
    }
    current_authority(state, auth)?;
    owned(state, &auth.principal, id)?;
    let admitted = crate::session_context::read(&state.hub, id)?;
    if admitted.revision != context.revision
        || !admitted
            .delivery
            .as_ref()
            .is_some_and(|previous| previous.operation == delivery.operation)
    {
        return Err("The pending handoff changed during preparation; inspect its destination before retrying".into());
    }
    validate_sources(state, &auth.principal, &context.sources)?;
    owned(state, &auth.principal, target_id)?;
    Ok(())
}

/// No prompt has been submitted on this path. Preserve that definite outcome
/// and release any dormant destination's fence. Failed persistence remains
/// observation-only; never guess that a post-dispatch error means failure.
async fn preparation_failed(
    state: &AppState,
    id: &str,
    context: &mut Context,
    delivery: &mut Delivery,
    error: String,
) -> Result<(), String> {
    delivery.state = "failed".into();
    delivery.error = Some(
        if delivery.session_id != id && state.hub.session_info(&delivery.session_id).is_some() {
            format!(
                "{error} A destination was created but no message was sent. Inspect or delete that session before retrying."
            )
        } else {
            error
        },
    );
    if delivery.session_id != id
        && state
            .hub
            .setting(&crate::session_context::key(&delivery.session_id))
            .is_some()
    {
        let mut target = crate::session_context::read(&state.hub, &delivery.session_id)?;
        advance_applied(&mut target)?;
        target.sources.clear();
        target.next_agent = None;
        target.injection = None;
        target.delivery = Some(delivery.clone());
        save(state, &delivery.session_id, &target).await?;
    }
    context.revision = context
        .revision
        .checked_add(1)
        .ok_or("Context revision exhausted")?;
    context.delivery = Some(delivery.clone());
    save(state, id, context).await
}

async fn deliver(
    state: Arc<AppState>,
    auth: AuthenticatedProductRequest,
    id: String,
    request: Send,
) -> Result<Delivery, String> {
    let lock = write_lock(&id);
    let _guard = lock.lock().await;
    current_authority(&state, &auth)?;
    let info = owned(&state, &auth.principal, &id)?;
    let mut context = crate::session_context::read(&state.hub, &id)?;
    if let Some(previous) = replay(&context, &request)? {
        return Ok(previous);
    }
    if !crate::managed_calls::valid_id(&request.operation)
        || build_prompt_blocks(&request.text, &request.content).is_none()
    {
        return Err("A nonempty message and a stable operation id are required".into());
    }
    if context.next_agent.is_some() && crate::session_context::is_command(&request.text) {
        return Err("A command needs a conversation, so it cannot start the new agent. Write its first message, or go Back to run this here.".into());
    }
    // Sending names its own carrier, for callers that want a receipt.
    let links_waiting = context.next_agent.is_none() && !context.waiting.is_empty();
    if request.revision != context.revision
        || (context.applied_revision == context.revision && !links_waiting)
    {
        return Err("Context changed or was already applied; refresh before sending".into());
    }
    if links_waiting {
        context.sources = crate::session_context::pending_links(&context);
        context.waiting.clear();
    }
    if info.queue_count > 0
        || (context.next_agent.is_some()
            && (state.hub.session_has_in_flight_prompt(&id)
                || matches!(info.meta.status, Status::Busy | Status::Starting)
                || info.meta.background_tasks > 0))
    {
        return Err("Wait for this agent to finish, or stop it and resolve its queue before starting the new context".into());
    }
    let sources = capture(&state, &auth.principal, &context.sources)?;
    let target_id = if context.next_agent.is_some() {
        state.supervisor.reserve_session_id()
    } else {
        id.clone()
    };
    let target_lock = (target_id != id).then(|| write_lock(&target_id));
    let _target_guard = match target_lock.as_ref() {
        Some(lock) => Some(lock.lock().await),
        None => None,
    };
    let mut delivery = Delivery {
        operation: request.operation.clone(),
        fingerprint: fingerprint(&request),
        session_id: target_id.clone(),
        state: "preparing".into(),
        error: None,
        source_trashed: false,
    };
    context.delivery = Some(delivery.clone());
    save(&state, &id, &context).await?;

    if let Err(error) = prepare_target(&state, &auth, &id, &context, &delivery).await {
        preparation_failed(&state, &id, &mut context, &mut delivery, error).await?;
        return Ok(delivery);
    }
    let cmid = format!("context-{}", request.operation);
    let mut target_context = if target_id == id {
        context.clone()
    } else {
        Context {
            revision: 1,
            sources: context.sources.clone(),
            delivery: Some(delivery.clone()),
            workspace_base: context
                .next_agent
                .as_ref()
                .and_then(|target| workspace_base(&state, &info.meta, target)),
            ..Context::default()
        }
    };
    advance_applied(&mut target_context)?;
    target_context.next_agent = None;
    let links = delivered_links(&state, &sources);
    target_context.injection = Some(Injection {
        cmid: cmid.clone(),
        sources,
        follows_first_prompt: true,
    });
    if let Err(error) = save(&state, &target_id, &target_context).await {
        preparation_failed(&state, &id, &mut context, &mut delivery, error).await?;
        return Ok(delivery);
    }
    // No retry ever reaches this dispatch twice: the durable intent above is
    // observation-only, even if the process dies before recording acceptance.
    state
        .hub
        .submit_context(&target_id, request.text, request.content, cmid);
    delivery.state = "accepted".into();
    target_context.remember_links(links);
    if target_id != id {
        target_context.delivery = Some(delivery.clone());
        save(&state, &target_id, &target_context).await?;
    }
    if target_id == id {
        context = target_context;
    } else {
        advance_applied(&mut context)?;
    }
    context.applied_revision = context.revision;
    context.next_agent = None;
    if target_id != id {
        context.sources.clear();
        context.waiting.clear();
    }
    context.delivery = Some(delivery.clone());
    save(&state, &id, &context).await?;
    // The work moved on, so the original leaves the list and stops holding a
    // Machine slot. It stays readable until Trash retention ends. Best effort:
    // a refusal only leaves it as read-only history beside the continuation.
    if target_id != id {
        match super::session_trash::trash(&state, &auth.principal, &[], std::slice::from_ref(&id)) {
            Ok(()) => {
                delivery.source_trashed = true;
                // A replayed receipt still offers Undo. Trash itself succeeded,
                // so a failed write only loses that offer; Restore remains.
                context.delivery = Some(delivery.clone());
                if let Err(reason) = save(&state, &id, &context).await {
                    tracing::info!(session = %id, %reason, "handoff Trash outcome not recorded");
                }
            }
            Err(reason) => {
                tracing::info!(session = %id, %reason, "handed-off session stayed out of Trash");
                if state.supervisor.has_live_worker(&id)
                    && let Err(reason) = state.supervisor.hibernate_session(&id)
                {
                    tracing::info!(session = %id, %reason, "handed-off session kept its idle worker");
                }
            }
        }
    }
    Ok(delivery)
}

pub(super) async fn send(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
    Json(request): Json<Send>,
) -> Response {
    let response_state = state.clone();
    match tokio::spawn(deliver(state, auth, id, request)).await {
        Ok(Ok(mut receipt)) => {
            receipt.fingerprint.clear();
            let session = response_state
                .hub
                .session_info(&receipt.session_id)
                .map(|info| info.meta);
            let mut value = serde_json::to_value(receipt).expect("receipt serializes");
            value["session"] = serde_json::to_value(session).expect("session serializes");
            (StatusCode::ACCEPTED, Json(value)).into_response()
        }
        Ok(Err(error)) => failure(error),
        Err(_) => {
            failure("Send outcome is unknown; inspect session context before retrying".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn carrier_is_recorded_once_and_survives_a_lost_reply() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let hub = crate::core::Hub::new();
        let context = serde_json::json!({
            "revision": 2, "applied_revision": 2, "sources": ["source"],
            "injection": {"cmid": "context-op", "follows_first_prompt": true,
                "sources": [{"session_id": "source", "before": 3}]},
        });
        hub.load_settings(vec![(crate::session_context::key("d"), context)]);
        let carrier = |hub: &crate::core::Hub| {
            hub.setting(&crate::session_context::carrier_key("d"))
                .map(|value| value["cmid"].clone())
        };
        claim_carrier_in(&hub, Some(&store), "d", Some("c-1"))
            .await
            .unwrap();
        assert_eq!(carrier(&hub), Some(serde_json::json!("c-1")));
        // Only the first dispatched prompt becomes the carrier.
        claim_carrier_in(&hub, Some(&store), "d", Some("c-2"))
            .await
            .unwrap();
        assert_eq!(carrier(&hub), Some(serde_json::json!("c-1")));
        // A lost reply: storage committed c-1 but memory did not observe it.
        hub.load_settings(vec![(
            crate::session_context::carrier_key("d"),
            serde_json::Value::Null,
        )]);
        claim_carrier_in(&hub, Some(&store), "d", Some("c-1"))
            .await
            .unwrap();
        assert_eq!(carrier(&hub), Some(serde_json::json!("c-1")));
        // Another prompt is refused, refreshes memory, and then goes without links.
        hub.load_settings(vec![(
            crate::session_context::carrier_key("d"),
            serde_json::Value::Null,
        )]);
        assert!(
            claim_carrier_in(&hub, Some(&store), "d", Some("c-3"))
                .await
                .is_err()
        );
        assert_eq!(carrier(&hub), Some(serde_json::json!("c-1")));
        claim_carrier_in(&hub, Some(&store), "d", Some("c-3"))
            .await
            .unwrap();
        // Without durable storage nothing is sent with links.
        let fresh = crate::core::Hub::new();
        fresh.load_settings(vec![(
            crate::session_context::key("d"),
            hub.setting(&crate::session_context::key("d")).unwrap(),
        )]);
        assert!(
            claim_carrier_in(&fresh, None, "d", Some("c-1"))
                .await
                .is_err()
        );
    }
    #[test]
    fn lost_reply_replays_receipt_but_changed_message_cannot_reuse_send_id() {
        let mut request = Send {
            revision: 4,
            operation: "send-1".into(),
            text: "continue".into(),
            content: vec![],
        };
        let context = Context {
            revision: 4,
            delivery: Some(Delivery {
                operation: request.operation.clone(),
                fingerprint: fingerprint(&request),
                session_id: "destination".into(),
                state: "preparing".into(),
                error: None,
                source_trashed: false,
            }),
            ..Context::default()
        };
        assert_eq!(
            replay(&context, &request).unwrap().unwrap().session_id,
            "destination"
        );
        request.text = "another message".into();
        assert!(replay(&context, &request).is_err());
        request.operation = "send-2".into();
        assert!(replay(&context, &request).is_err());
    }
    #[test]
    fn new_target_is_a_closed_noncredential_contract() {
        let base = serde_json::json!({"provider":"claude-code","provider_account_id":"default","machine_id":"ovh","cwd":"project"});
        let target: NextAgent = serde_json::from_value(base.clone()).unwrap();
        assert!(validate_target(&target).is_ok());
        assert!(target.inherit_settings);
        let mut with_credentials = base;
        with_credentials["credentials"] = serde_json::json!("secret");
        assert!(serde_json::from_value::<NextAgent>(with_credentials).is_err());
    }
}
