//! Session HTTP handlers: creation, info, reload, hibernate, cache protection,
//! prompt submission, bootstrap and question pages.

use super::*;

pub(super) async fn api_session_info(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    match state.hub.session_info(&session_id) {
        Some(info) => Json(info).into_response(),
        None => (StatusCode::NOT_FOUND, "unknown session").into_response(),
    }
}

pub(super) async fn session_reload_target(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
) -> Result<ResolvedProviderGeneration, String> {
    if !state.runtime_router.connected(&meta.machine_id) {
        return Err("session Machine is not connected".to_owned());
    }
    if matches!(meta.status, Status::Busy | Status::Starting)
        || state.hub.session_has_in_flight_prompt(&meta.id)
    {
        return Err(
            "wait for the current turn to finish before loading a new Provider version".to_owned(),
        );
    }
    session_upgrade_target(state, meta).await
}

/// The installed release `meta` could adopt: every check of an explicit
/// Reload except that the session is between turns, so a busy session can
/// still be told an update is waiting.
pub(super) async fn session_upgrade_target(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
) -> Result<ResolvedProviderGeneration, String> {
    if !state.runtime_router.connected(&meta.machine_id) {
        return Err("session Machine is not connected".to_owned());
    }
    if !state.hub.provider_version_changeable(&meta.id) {
        return Err("a saved native session is required to load a new Provider version".to_owned());
    }
    if !meta.provider_account_id.is_default() {
        return named_upgrade_target(state, meta).await;
    }
    installed_upgrade_target(
        state,
        &meta.machine_id,
        &meta.provider,
        &meta.provider_version,
        &meta.provider_generation_digest,
    )
    .await
}

pub(super) async fn api_session_reload_plan(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    let Some(info) = state.hub.session_info(&session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    match session_reload_target(&state, &info.meta).await {
        Ok(target) => Json(serde_json::json!({
            "current_version": info.meta.provider_version,
            "automatic_updates": session_provider_updates::enabled(&state.hub, &session_id),
            "target_version": target.version,
            "target_digest": target.digest,
            "upgrade_available": target.digest != info.meta.provider_generation_digest,
        }))
        .into_response(),
        Err(error) => Json(serde_json::json!({
            "current_version": info.meta.provider_version,
            "automatic_updates": session_provider_updates::enabled(&state.hub, &session_id),
            "upgrade_available": false,
            "blocked_reason": error,
        }))
        .into_response(),
    }
}

pub(super) async fn api_session_reload(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionReloadQuery>,
) -> Response {
    let Some(info) = state.hub.session_info(&session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let _maintenance = match session_maintenance::reload_lease(&state, &session_id) {
        Ok(lease) => lease,
        Err(error) => return (StatusCode::CONFLICT, error).into_response(),
    };
    if query.upgrade_provider {
        let _fence = match ProviderReloadFence::acquire(
            &state.plugin_lifecycle_fences,
            (info.meta.machine_id.clone(), info.meta.provider.clone()),
        ) {
            Ok(fence) => fence,
            Err(error) => return (StatusCode::CONFLICT, error).into_response(),
        };
        let target = match session_reload_target(&state, &info.meta).await {
            Ok(target) => target,
            Err(error) => return (StatusCode::CONFLICT, error).into_response(),
        };
        if query.expected_generation_digest.as_deref() != Some(target.digest.as_str()) {
            return (
                StatusCode::CONFLICT,
                "installed Provider changed; reopen Reload and confirm the new version",
            )
                .into_response();
        }
        return match apply_session_provider_reload(&state, &info.meta, &target, false).await {
            Ok(()) => {
                (StatusCode::ACCEPTED, "reloading installed Provider version").into_response()
            }
            Err(error) => (StatusCode::CONFLICT, error).into_response(),
        };
    }
    let reload_key = provider_fence_key_for_session(&state.hub, &session_id);
    let reload_fence = state.plugin_lifecycle_fences.read();
    if let Some(refusal) = reload_key
        .as_ref()
        .and_then(|key| reload_fence.get(key))
        .and_then(|fence| provider_fence_refusal(*fence))
    {
        return (StatusCode::CONFLICT, refusal).into_response();
    }
    tracing::info!(
        session = %session_id,
        status = ?info.meta.status,
        confirm_active_turn = query.confirm_active_turn,
        "session runtime reload requested"
    );
    if execution::sessions::retry_blocked(&state, &session_id)
        || (info.meta.status != Status::Starting
            && workspace_preparation::resume(&state, &session_id).is_some())
    {
        return (StatusCode::ACCEPTED, "preparing the workspace").into_response();
    }
    match state
        .supervisor
        .reload_session(&session_id, query.confirm_active_turn)
    {
        Ok(()) => (StatusCode::ACCEPTED, "reloading").into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

/// Free an idle session's Agent process and Machine slot without ending it.
pub(super) async fn api_session_hibernate(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    let Some(session) = state
        .hub
        .session_list()
        .into_iter()
        .find(|session| session.id == session_id)
    else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if !state.machine_control.machine_supports(
        &session.machine_id,
        crate::machine_protocol::SESSION_HIBERNATION_PROTOCOL_VERSION,
    ) {
        return (
            StatusCode::CONFLICT,
            "this session's Machine does not support hibernation yet",
        )
            .into_response();
    }
    match state.supervisor.hibernate_session(&session_id) {
        Ok(()) => (StatusCode::ACCEPTED, "hibernating").into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

pub(super) async fn api_session_cache_protection(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    let Some(session) = state
        .hub
        .session_list()
        .into_iter()
        .find(|session| session.id == session_id)
    else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let configuration = session.provider_behavior.as_ref().map_or_else(
        || crate::provider::legacy_behavior(&session.provider).configuration,
        |behavior| behavior.configuration.clone(),
    );
    let preferences = state
        .hub
        .config_preferences(&session.id)
        .unwrap_or_else(|| serde_json::json!({}));
    let Some(enabled) = crate::managed_config::cache_protection(&configuration, &preferences)
    else {
        return (
            StatusCode::BAD_REQUEST,
            crate::deepseek_cache::unavailable_message(),
        )
            .into_response();
    };
    if !enabled {
        return Json(serde_json::json!({
            "state": "disabled",
            "algorithm": "adaptive-replay-v1",
            "minimumHitTokens": crate::deepseek_cache::minimum_hit_tokens(),
            "contextUsed": session.context_used,
        }))
        .into_response();
    }
    match state
        .machine_control
        .adapter_request(
            &session.machine_id,
            "provider-cache-status",
            serde_json::json!({
                "configuration": configuration,
                "sessionId": session.id,
            }),
        )
        .await
    {
        Ok(mut status) => {
            if let Some(object) = status.as_object_mut() {
                object.insert(
                    "minimumHitTokens".to_owned(),
                    crate::deepseek_cache::minimum_hit_tokens().into(),
                );
                object.insert("contextUsed".to_owned(), session.context_used.into());
            }
            Json(status).into_response()
        }
        Err(error) => {
            tracing::warn!(session = %session.id, machine = %session.machine_id, %error, "Provider cache-protection status unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "cache-protection status is temporarily unavailable",
            )
                .into_response()
        }
    }
}

/// Inject a prompt into a session FROM THE BACKEND (machine-driven). The
/// This bypasses the WS user-input gate precisely because it is the backend,
/// not an interactive client. Works on any session, including `system` ones.
pub(super) async fn api_session_prompt(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(req): Json<SessionPromptRequest>,
) -> Response {
    if crate::session_context::pending(&state.hub, &session_id) {
        return (
            StatusCode::CONFLICT,
            "Context changes are pending; use sessions.context_send with its revision",
        )
            .into_response();
    }
    use crate::send_delivery::SendDelivery;
    let delivery = match SendDelivery::resolve(req.delivery, req.force, req.front) {
        Ok(delivery) => delivery,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    if matches!(
        delivery,
        SendDelivery::CurrentTurn | SendDelivery::NextCheckpoint
    ) {
        return send_delivery::submit(state, session_id, req).await;
    }
    if delivery == SendDelivery::Interrupt && req.target.is_some() {
        return send_delivery::submit(state, session_id, req).await;
    }
    // Keep the read guard through dispatch. Provider uninstall acquires the
    // write guard before it snapshots active turns, so a prompt can never slip
    // between the fence check and the explicit active-turn confirmation.
    let provider_key = provider_fence_key_for_session(&state.hub, &session_id);
    let plugin_prompt_fence = state.plugin_lifecycle_fences.read();
    if let Some(refusal) = provider_key
        .as_ref()
        .and_then(|key| plugin_prompt_fence.get(key))
        .and_then(|fence| provider_fence_refusal(*fence))
    {
        return (StatusCode::CONFLICT, refusal).into_response();
    }
    let Some(info) = state.hub.session_info(&session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if info.meta.trashed_at_ms.is_some() {
        return (StatusCode::CONFLICT, session_trash::TRASHED_REFUSAL).into_response();
    }
    if info
        .meta
        .execution_binding
        .as_ref()
        .is_some_and(|binding| binding.managed().is_some())
    {
        return (
            StatusCode::CONFLICT,
            "this child conversation is controlled by its parent's managed call",
        )
            .into_response();
    }
    if build_prompt_blocks(&req.text, &req.content).is_none() {
        return (StatusCode::BAD_REQUEST, "empty prompt: no text or content").into_response();
    }
    // API and UI prompts share one durable queue and cold-admission boundary.
    // Resource pressure must not turn an accepted backend wake into a 404.
    let cmid = Some(
        req.operation
            .unwrap_or_else(|| format!("api:{}", uuid::Uuid::new_v4())),
    );
    if delivery == SendDelivery::Interrupt {
        let Some(queued) =
            state
                .hub
                .force_submit_once(&session_id, req.text, req.content, cmid, true)
        else {
            return (StatusCode::ACCEPTED, "already accepted").into_response();
        };
        if queued
            && matches!(
                state.hub.status(&session_id),
                Some(Status::Busy | Status::Starting)
            )
        {
            if let Err(error) = force_cancel_with_watchdog(&state, &session_id) {
                return (StatusCode::CONFLICT, error).into_response();
            }
        } else {
            state.hub.drain_now(&session_id);
        }
    } else if req.front {
        let _ = state
            .hub
            .force_submit(&session_id, req.text, req.content, cmid, false);
    } else {
        state.hub.submit(&session_id, req.text, req.content, cmid);
    }
    (StatusCode::ACCEPTED, "queued").into_response()
}

pub(super) async fn api_new_session(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Json(mut req): Json<NewSessionRequest>,
) -> Response {
    let principal = authenticated.principal.clone();
    if !principal.role.at_least(crate::admin::AdminRole::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let (provider_account_id, runtime) =
        match resolve_new_session_defaults(&state, &principal, &mut req).await {
            Ok(resolved) => resolved,
            Err(response) => return response,
        };
    if let Some(runtime) = runtime.filter(|runtime| runtime != &req.machine_id) {
        if req.system {
            return (
                StatusCode::BAD_REQUEST,
                "Remote directory sessions cannot be system sessions",
            )
                .into_response();
        }
        let mut request: execution::sessions::CreateRequest = serde_json::from_value(serde_json::json!({
            "provider": req.provider, "machine_id": req.machine_id, "runtime_machine_id": runtime,
            "cwd": req.cwd.unwrap_or_default(), "origin": req.origin, "initial_prompt": req.initial_prompt,
            "folder_id": req.folder_id, "preset": req.preset,
            "provider_account_id": provider_account_id,
            "context_sources": req.context_sources,
        })).expect("typed creation request");
        request.reserved_id = req.reserved_id;
        request.retained_auth_generation = req.retained_auth_generation;
        return execution::sessions::create(State(state), Extension(authenticated), Json(request))
            .await;
    }
    if req.machine_id == "local" && (req.folder_id.is_some() || req.preset.is_some()) {
        return (
            StatusCode::BAD_REQUEST,
            "Directory defaults require an enrolled Machine",
        )
            .into_response();
    }
    if !state
        .supervisor
        .runtime_allowed(&req.provider, &req.machine_id)
    {
        return (
            StatusCode::CONFLICT,
            "Provider policy does not permit this runtime Machine",
        )
            .into_response();
    }
    let cwd = req.cwd.take();
    let session_id = req
        .reserved_id
        .take()
        .unwrap_or_else(|| state.supervisor.reserve_session_id());
    if web_session_is_missing_machine(&req.machine_id, &req.origin) {
        return (
            StatusCode::CONFLICT,
            "Web session creation requires a connected Machine so its workspace can be isolated",
        )
            .into_response();
    }
    if state
        .plugin_lifecycle_fences
        .read()
        .contains_key(&(req.machine_id.clone(), req.provider.clone()))
    {
        return (
            StatusCode::CONFLICT,
            "Provider is changing on the selected Machine",
        )
            .into_response();
    }
    if req.machine_id != "local" {
        return create_machine_session(
            &state,
            &principal,
            req,
            session_id,
            cwd,
            provider_account_id,
        )
        .await;
    }
    create_local_session(
        &state,
        &principal,
        req,
        session_id,
        cwd,
        provider_account_id,
    )
    .await
}

/// Applies the request's directory defaults and resolves its Provider
/// account and the runtime Machine a directory session should execute on.
#[expect(
    clippy::result_large_err,
    reason = "HTTP rejection is an owned Axum response returned directly by its handler"
)]
async fn resolve_new_session_defaults(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    req: &mut NewSessionRequest,
) -> Result<(crate::provider_account::ProviderAccountId, Option<String>), Response> {
    if let Err(error) = session_context::validate_sources(state, principal, &req.context_sources) {
        return Err((StatusCode::BAD_REQUEST, error).into_response());
    }
    use crate::session_folders::defaults::DefaultKey;
    let defaults = match folder_defaults::resolve(state, principal, req.folder_id.as_deref()) {
        Ok(value) => value,
        Err(error) => return Err((StatusCode::BAD_REQUEST, error).into_response()),
    };
    if req.provider.is_empty() {
        req.provider = folder_defaults::value(&defaults, DefaultKey::Provider)
            .unwrap_or_else(|| "claude-code".into());
    }
    let provider_account_id = match provider_accounts::resolve_for_create(
        state,
        &principal.user_id,
        &req.provider,
        req.provider_account_id.as_ref(),
    )
    .await
    {
        Ok(id) => id,
        Err(code) => {
            return Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error":{"code":code}})),
            )
                .into_response());
        }
    };
    if req.machine_id == "local" {
        req.machine_id = folder_defaults::value(&defaults, DefaultKey::Machine)
            .unwrap_or(std::mem::take(&mut req.machine_id));
    }
    if req.cwd.is_none() {
        req.cwd = folder_defaults::value(&defaults, DefaultKey::Project);
    }
    if req.folder_id.is_some() && (req.machine_id == "local" || req.cwd.is_none()) {
        if req.cwd.is_none() && defaults.values.get(&DefaultKey::Project) == Some(&None) {
            return Err((
                StatusCode::BAD_REQUEST,
                "This directory has no project binding; select a project explicitly",
            )
                .into_response());
        }
        match folder_defaults::default_workspace(
            state,
            (req.machine_id != "local").then_some(req.machine_id.as_str()),
            req.cwd.as_deref(),
        )
        .await
        {
            Ok((machine, project)) => {
                req.machine_id = machine;
                req.cwd = Some(project);
            }
            Err(error) => return Err((StatusCode::BAD_REQUEST, error).into_response()),
        }
    }
    if req.preset.is_none() {
        req.preset = folder_defaults::value(&defaults, DefaultKey::Preset);
    }
    let runtime = req
        .runtime_machine_id
        .as_deref()
        .map(str::to_owned)
        .or_else(|| folder_defaults::value(&defaults, DefaultKey::RuntimeMachine))
        .or_else(|| {
            req.folder_id.as_ref().and_then(|_| {
                state
                    .project_placement
                    .snapshot()
                    .default_runtime_machine_id
                    .clone()
            })
        });
    Ok((provider_account_id, runtime))
}

/// Registers a session whose Provider runs on an enrolled Machine and
/// schedules its workspace preparation.
async fn create_machine_session(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    req: NewSessionRequest,
    session_id: String,
    mut cwd: Option<String>,
    provider_account_id: crate::provider_account::ProviderAccountId,
) -> Response {
    if !state
        .project_placement
        .snapshot()
        .allows(&req.machine_id, &req.machine_id)
    {
        return (
            StatusCode::CONFLICT,
            "Machine policy does not permit a local AI session on this project Machine",
        )
            .into_response();
    }
    let Some(store) = state.store.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "machine registry unavailable",
        )
            .into_response();
    };
    let machine = store.list_machines().await.ok().and_then(|machines| {
        machines
            .into_iter()
            .find(|machine| machine.id == req.machine_id && !machine.revoked)
    });
    let Some(machine) = machine else {
        return (StatusCode::NOT_FOUND, "unknown or revoked machine").into_response();
    };
    let workspaces: Vec<crate::machine_protocol::MachineWorkspace> = machine
        .inventory
        .get("workspaces")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let workspace = if req.folder_id.is_some() {
        folder_defaults::workspace(&workspaces, cwd.as_deref())
    } else {
        resolve_machine_workspace(&workspaces, cwd.as_deref())
    };
    let selected_workspace = match workspace {
        Ok(workspace) => workspace.clone(),
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    cwd = Some(selected_workspace.canonical_path.clone());
    let capacity: crate::machine_protocol::MachineCapacity = machine
        .inventory
        .get("capacity")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    if capacity.draining {
        return (StatusCode::CONFLICT, "Machine is draining").into_response();
    }
    let providers = machine
        .inventory
        .get("plugins")
        .or_else(|| machine.inventory.get("providers"))
        .cloned()
        .and_then(|value| {
            serde_json::from_value::<Vec<crate::machine_protocol::PluginInventory>>(value).ok()
        })
        .unwrap_or_default();
    let mut provider_generation = match provider_accounts::registration::generation(
        state,
        &providers,
        &req.provider,
        &principal.user_id,
        &provider_account_id,
        &req.machine_id,
    )
    .await
    {
        Ok(generation) => generation,
        Err(error) => return (StatusCode::CONFLICT, error).into_response(),
    };
    let scheduling_generation = provider_generation.auth_generation;
    if let Some(retained) = req.retained_auth_generation
        && let Err(error) = provider_accounts::registration::retain_auth_generation(
            &mut provider_generation,
            &provider_account_id,
            retained,
        )
    {
        return (StatusCode::CONFLICT, error).into_response();
    }

    let Some(source_path) = cwd.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            "remote session requires a trusted workspace",
        )
            .into_response();
    };
    let configuration = match folder_defaults::preset_values(
        state,
        &req.provider,
        &provider_generation,
        req.preset.as_deref(),
    ) {
        Ok(values) => values,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let registration = state.supervisor.prepare_session_on_with_id(
        &session_id,
        &req.provider,
        Some(source_path.to_owned()),
        req.origin,
        req.system,
        crate::supervisor::SessionPlacement {
            machine_id: &req.machine_id,
            workspace: Some(&selected_workspace),
            execution_binding: None,
        },
        crate::supervisor::ProviderGeneration {
            account_id: Some(&provider_account_id),
            version: &provider_generation.version,
            digest: &provider_generation.digest,
            auth_generation: provider_generation.auth_generation,
            behavior: Some(&provider_generation.behavior),
        },
        product_session_owner(state.product_auth_enabled, principal),
    );
    let registration = match registration {
        Ok(registration) => registration,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };
    if let Err(error) = session_context::initialize(
        state,
        principal,
        &session_id,
        req.context_sources,
        req.initial_prompt
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .map(|_| format!("initial:{session_id}")),
    )
    .await
    {
        return (StatusCode::CONFLICT, error).into_response();
    }
    if let Err(error) =
        provider_accounts::registration::register(state, registration, scheduling_generation).await
    {
        return (StatusCode::CONFLICT, error).into_response();
    }

    if let Err(error) = folder_defaults::apply_created(
        state,
        principal,
        &session_id,
        req.folder_id.as_deref(),
        configuration,
    ) {
        return (StatusCode::BAD_REQUEST, error).into_response();
    }
    let initial_prompt = req.initial_prompt;
    let eager = req.system;
    if let Some(prompt) = initial_prompt.filter(|p| !p.trim().is_empty()) {
        state.hub.requeue_prompt(
            &session_id,
            prompt,
            vec![],
            Some(format!("initial:{session_id}")),
        );
        session_maintenance::wait(&state.hub, &session_id, "preparing the workspace");
    }
    if let Err(error) = session_context::initialized(state, &session_id).await {
        return (StatusCode::CONFLICT, error).into_response();
    }
    workspace_preparation::schedule(state, &session_id, &req.machine_id, source_path, eager);
    (
        StatusCode::CREATED,
        Json(NewSessionResponse {
            session_id,
            provider_account_id: provider_account_id.clone(),
            provider_version: provider_generation.version,
            provider_generation_digest: provider_generation.digest,
            provider_auth_generation: provider_generation.auth_generation,
        }),
    )
        .into_response()
}

/// Starts a session on the Controller host runtime.
async fn create_local_session(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    req: NewSessionRequest,
    session_id: String,
    cwd: Option<String>,
    provider_account_id: crate::provider_account::ProviderAccountId,
) -> Response {
    if let Err(error) = session_context::initialize(
        state,
        principal,
        &session_id,
        req.context_sources,
        req.initial_prompt
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .map(|_| format!("initial:{session_id}")),
    )
    .await
    {
        return (StatusCode::CONFLICT, error).into_response();
    }
    match state.supervisor.new_session_on_with_id(
        &session_id,
        &req.provider,
        cwd,
        req.origin,
        req.system,
        &req.machine_id,
        crate::supervisor::ProviderGeneration {
            account_id: Some(&provider_account_id),
            version: "",
            digest: "",
            auth_generation: None,
            behavior: None,
        },
        product_session_owner(state.product_auth_enabled, principal),
    ) {
        Ok(session_id) => {
            if let Some(prompt) = req
                .initial_prompt
                .filter(|prompt| !prompt.trim().is_empty())
                && let Err(message) = state.supervisor.send(
                    &session_id,
                    AgentCommand::Prompt(
                        vec![ContentBlock::from(prompt)],
                        Some(format!("initial:{session_id}")),
                        None,
                    ),
                )
            {
                state.hub.set_status(
                    &session_id,
                    crate::agent_model::Status::Crashed,
                    Some(format!("Initial prompt failed: {message}")),
                );
                return (
                    StatusCode::CONFLICT,
                    "Initial prompt failed; inspect session context before retrying",
                )
                    .into_response();
            }
            if let Err(error) = session_context::initialized(state, &session_id).await {
                return (StatusCode::CONFLICT, error).into_response();
            }
            (
                StatusCode::CREATED,
                Json(NewSessionResponse {
                    session_id,
                    provider_version: String::new(),
                    provider_generation_digest: String::new(),
                    provider_account_id,
                    provider_auth_generation: None,
                }),
            )
                .into_response()
        }
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}

/// Hydrate only the session the reader actually opened. The WebSocket connect
/// path deliberately carries global metadata only; replaying every transcript,
/// config option and queue state made mobile reconnects multi-megabyte
/// affairs. Live events can overlap this response and are deduplicated by seq.
pub(super) async fn api_session_bootstrap(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionBootstrapQuery>,
) -> Response {
    session_configuration::prepare(&state, &session_id);
    let since_seq = query.since_seq;
    let subagent_state =
        background_tasks::subagent_state(&state.hub, state.store.as_ref(), &session_id, since_seq)
            .await;
    let Some(messages) = focused_session_bootstrap(&state.hub, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let task_acknowledgements = notifications::acknowledgements(
        state.store.as_ref(),
        &authenticated.principal.user_id,
        &session_id,
    )
    .await;
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(SessionBootstrapResponse {
            messages,
            subagent_state,
            task_acknowledgements,
        }),
    )
        .into_response()
}

pub(super) async fn api_question_pages(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<QuestionPagesQuery>,
) -> Response {
    if state.hub.session_info(&session_id).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let limit = query.limit.unwrap_or(64).clamp(1, 100);
    let result = match &state.store {
        Some(store) => store
            .question_page_summaries(&session_id, query.before, limit)
            .await
            .map(|(pages, next_before_seq, total)| QuestionPagesResponse {
                total,
                exact: true,
                pages,
                next_before_seq,
            }),
        None => Ok(state
            .hub
            .question_page_summaries(&session_id, query.before, limit)
            .map_or(
                QuestionPagesResponse {
                    total: 0,
                    exact: false,
                    pages: Vec::new(),
                    next_before_seq: None,
                },
                |(pages, next_before_seq, total, exact)| QuestionPagesResponse {
                    total: u64::try_from(total).unwrap_or(u64::MAX),
                    exact,
                    pages,
                    next_before_seq,
                },
            )),
    };
    match result {
        Ok(response) => Json(response).into_response(),
        Err(error) => {
            tracing::warn!(
                session = %session_id,
                error = %error,
                "question page count query failed"
            );
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "question pages unavailable",
            )
                .into_response()
        }
    }
}

pub(super) async fn api_question_page(
    State(state): State<Arc<AppState>>,
    Path((session_id, page_id)): Path<(String, u64)>,
) -> Response {
    if state.hub.session_info(&session_id).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let result = match &state.store {
        Some(store) => store.question_page_at(&session_id, page_id).await,
        None => Ok(state.hub.question_page_at(&session_id, page_id)),
    };
    match result {
        Ok(Some(events)) => (
            // The newest question page can still grow after its root prompt is
            // persisted. A reader may request it while the agent is producing
            // the answer, so treating this route as immutable can pin that
            // partial response on one device for a year. Cursor history remains
            // immutable; explicit question-page reads always revalidate.
            [(header::CACHE_CONTROL, "no-store")],
            Json(HistoryResponse {
                events,
                next_before_seq: None,
                reached_start: false,
            }),
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(
                session = %session_id,
                page_id,
                %error,
                "question page query failed"
            );
            (StatusCode::SERVICE_UNAVAILABLE, "question page unavailable").into_response()
        }
    }
}
