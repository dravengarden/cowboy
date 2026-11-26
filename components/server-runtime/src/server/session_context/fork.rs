//! Native session fork: a new session on the source's account, Provider and
//! Machines whose first thread is ACP `session/fork` of the source thread.
//! See `docs/session-fork.md`.
use super::*;
use crate::core::{ForkPoint, ForkRecord, ForkState, SessionMeta};

/// Providers whose pinned adapter fork was verified end to end.
const FORKABLE_PROVIDERS: [&str; 2] = ["claude-code", "codex"];
const MAX_MESSAGE_ID_BYTES: usize = 256;

#[derive(Debug, Deserialize)]
pub struct ForkRequest {
    pub operation: String,
    /// Provider ID of a turn-final assistant message; omitted forks it all.
    #[serde(default)]
    pub message_id: Option<String>,
    /// Fork from the answer that ended the newest finished turn, which needs
    /// no idle source. Takes the place of `message_id`.
    #[serde(default)]
    pub latest_completed: bool,
    /// With `latest_completed`: an earlier fork to return instead while it
    /// still starts from that same answer, so follow-ups stay in one session.
    #[serde(default)]
    pub reuse: Option<String>,
    /// Live options (model, reasoning, …) the copy uses instead of the
    /// inherited ones; saved before its first launch.
    #[serde(default)]
    pub config: serde_json::Map<String, serde_json::Value>,
}

/// Execution Machine, runtime Machine and project of an admitted session.
pub fn placement(meta: &SessionMeta) -> Result<(String, Option<String>, String), String> {
    let project = meta
        .workspace_id
        .clone()
        .ok_or("This session has no project to fork")?;
    let Some(binding) = &meta.execution_binding else {
        return Ok((meta.machine_id.clone(), None, project));
    };
    let binding = binding
        .decode()
        .map_err(|_| "Wait until this session's workspace is ready before forking it")?;
    if binding.managed.is_some() {
        return Err("A managed call cannot be forked".into());
    }
    Ok((
        binding.environment.machine_id,
        Some(meta.machine_id.clone()),
        project,
    ))
}

fn admit_point(
    hub: &crate::core::Hub,
    info: &SessionInfo,
    message_id: Option<&str>,
) -> Result<(), String> {
    let Some(message_id) = message_id else {
        if info.queue_count > 0
            || hub.session_has_in_flight_prompt(&info.meta.id)
            || matches!(info.meta.status, Status::Busy | Status::Starting)
            || info.meta.background_tasks > 0
        {
            return Err(
                "Wait for this agent to finish its turn and queue before forking, or fork from an earlier answer".into(),
            );
        }
        return Ok(());
    };
    if message_id.is_empty() || message_id.len() > MAX_MESSAGE_ID_BYTES {
        return Err("Invalid fork message".into());
    }
    match hub.fork_point(&info.meta.id, message_id) {
        ForkPoint::Completed | ForkPoint::NotLoaded => Ok(()),
        ForkPoint::Running => Err("Wait for this turn to finish before forking from it".into()),
        ForkPoint::NotTurnFinal => Err("Fork from the last answer of a turn".into()),
        ForkPoint::Missing => Err("That message is not in this conversation".into()),
    }
}

/// A fork still answers for its source while nothing has finished since it
/// was taken. After that its copy of the code and the conversation are behind.
fn reusable_fork(hub: &crate::core::Hub, source: &str, fork: &str, latest: &str) -> bool {
    hub.session_fork(fork).is_some_and(|record| {
        record.source_session_id == source
            && record.message_id.as_deref() == Some(latest)
            && record.state != ForkState::Refused
    }) && hub.session_info(fork).is_some()
        && !hub.session_is_trashed(fork)
}

/// Whether `source`'s native thread can be forked at all.
fn forkable(state: &AppState, source: &SessionMeta) -> Result<(), String> {
    // A native fork reads the source thread from its login's home, and a
    // retired legacy card's login exists nowhere any more.
    if source.provider_account_id.is_default()
        && provider_accounts::retirement::retired(&state.accounts, &source.provider)
    {
        return Err("This conversation ran on a retired legacy default account and cannot be forked; use Continue in new agent on a named account".into());
    }
    if !FORKABLE_PROVIDERS.contains(&source.provider.as_str()) {
        return Err(format!(
            "{} sessions cannot be forked yet; use Continue in new agent",
            source.provider
        ));
    }
    Ok(())
}

fn check_fork_machines(
    state: &AppState,
    machine_id: &str,
    runtime_machine_id: Option<&str>,
) -> Result<(), String> {
    let runtime = runtime_machine_id.unwrap_or(machine_id);
    for machine in [machine_id, runtime] {
        if !state.machine_control.machine_supports(
            machine,
            crate::machine_protocol::NATIVE_FORK_PROTOCOL_VERSION,
        ) {
            return Err(format!(
                "Machine {machine} must be updated before it can fork sessions"
            ));
        }
    }
    Ok(())
}

async fn resolve_latest_completed(state: &AppState, id: &str) -> Result<String, String> {
    if let Some(found) = state.hub.last_completed_answer_id(id) {
        return Ok(found);
    }
    if let Some(store) = state.store.as_ref()
        && let Ok(Some((mid, _))) = store.latest_turn_completion(id, 0).await
    {
        return Ok(mid);
    }
    Err("Wait for this agent's first answer before forking".into())
}

async fn fork_session(
    state: Arc<AppState>,
    auth: AuthenticatedProductRequest,
    id: String,
    mut request: ForkRequest,
) -> Result<(String, bool), String> {
    let lock = write_lock(&id);
    let _guard = lock.lock().await;
    current_authority(&state, &auth)?;
    let info = owned(&state, &auth.principal, &id)?;
    if !crate::managed_calls::valid_id(&request.operation) {
        return Err("A stable operation id is required".into());
    }
    valid_overrides(&request.config)?;
    if let Some((destination, record)) = state.hub.fork_by_operation(&id, &request.operation) {
        // The latest answer moves, so a retry cannot restate the point it took.
        if !request.latest_completed && record.message_id != request.message_id {
            return Err("This operation already forked another point".into());
        }
        if record.state == ForkState::Refused {
            return Err(format!(
                "The new session {destination} could not fork this conversation; delete it and fork again"
            ));
        }
        return Ok((destination, false));
    }
    if state.hub.session_is_trashed(&id) {
        return Err("Restore this session from Trash before forking it".into());
    }
    let source = info.meta.clone();
    forkable(&state, &source)?;
    let source_thread = state
        .hub
        .agent_session_id_for_resume(&id)
        .ok_or("This conversation has nothing to fork yet")?;
    if request.latest_completed {
        let latest = resolve_latest_completed(&state, &id).await?;
        if let Some(fork) = request
            .reuse
            .as_deref()
            .filter(|fork| reusable_fork(&state.hub, &id, fork, &latest))
        {
            return Ok((fork.to_owned(), false));
        }
        request.message_id = Some(latest);
    }
    admit_point(&state.hub, &info, request.message_id.as_deref())?;
    let (machine_id, runtime_machine_id, project) = placement(&source)?;
    check_fork_machines(&state, &machine_id, runtime_machine_id.as_deref())?;
    let selection = NextAgent {
        provider: source.provider.clone(),
        provider_account_id: source.provider_account_id.clone(),
        machine_id: machine_id.clone(),
        cwd: project.clone(),
        runtime_machine_id: runtime_machine_id.clone(),
        preset: None,
        inherit_settings: true,
        config: serde_json::Map::new(),
    };
    // The adapter reads the source thread from the login's native home on
    // this Machine, and each auth generation has its own home. A Service
    // token refresh advances the generation about every eight hours without
    // a new sign-in, so the fork keeps the source's generation; only a named
    // subscription that really signed in again is refused.
    let mut generation = inheritance::target_generation(&state, &auth, &selection).await?;
    if let Some(retained) = source.provider_auth_generation {
        provider_accounts::registration::retain_auth_generation(
            &mut generation,
            &source.provider_account_id,
            retained,
        )?;
    } else if generation.auth_generation.is_some() {
        return Err(provider_accounts::registration::SIGNED_IN_AGAIN.into());
    }
    inheritance::preflight(&state, &auth, &source, &selection).await?;
    let actor = folder_actor(&state, &auth.principal);
    state.hub.continuation_folder(&id, &actor)?;

    let destination = state.supervisor.reserve_session_id();
    let target_lock = write_lock(&destination);
    let _target_guard = target_lock.lock().await;
    let record = ForkRecord {
        source_session_id: id.clone(),
        source_agent_session_id: source_thread,
        message_id: request.message_id.clone(),
        point: request
            .message_id
            .is_none()
            .then(|| state.hub.last_answer_id(&id))
            .flatten(),
        operation: request.operation.clone(),
        state: ForkState::Pending,
    };
    // Durable before the destination exists: a restart must never restore it
    // without its fork intent, and its workspace preparation reads the base.
    let store = state.store.as_ref().ok_or("Fork storage unavailable")?;
    let key = format!(
        "{}{destination}",
        crate::core::settings_keys::SESSION_FORK_PREFIX
    );
    let value = serde_json::to_value(&record).expect("fork record serializes");
    if !store
        .compare_and_put_setting(&key, None, &value)
        .await
        .map_err(|_| "Fork storage unavailable")?
    {
        return Err("Fork storage changed; retry".into());
    }
    state.hub.stage_fork(&destination, &record);
    let create = NewSessionRequest {
        provider: source.provider.clone(),
        provider_account_id: Some(source.provider_account_id.clone()),
        machine_id,
        runtime_machine_id,
        cwd: Some(project),
        preset: None,
        folder_id: None,
        system: false,
        origin: SessionOrigin::Web,
        initial_prompt: None,
        context_sources: vec![],
        reserved_id: Some(destination.clone()),
        retained_auth_generation: source.provider_auth_generation,
    };
    let response =
        api_new_session(State(state.clone()), Extension(auth.clone()), Json(create)).await;
    if !response.status().is_success() {
        state.hub.unstage_fork(&destination);
        return Err(creation_error(response).await);
    }
    let created = owned(&state, &auth.principal, &destination)?.meta;
    if created.provider != source.provider
        || created.provider_account_id != source.provider_account_id
        || created.machine_id != source.machine_id
        || created.provider_auth_generation != source.provider_auth_generation
    {
        state.hub.refuse_native_fork(&destination);
        return Err(format!(
            "The new session {destination} was admitted on another login or Machine and cannot fork this conversation; delete it and retry"
        ));
    }
    finish_native_fork(
        &state,
        &id,
        &destination,
        &source,
        &created,
        &record,
        &selection,
        &request.config,
        &actor,
    )
    .await?;
    Ok((destination, true))
}

#[allow(clippy::too_many_arguments)]
async fn finish_native_fork(
    state: &AppState,
    id: &str,
    destination: &str,
    source: &SessionMeta,
    created: &SessionMeta,
    record: &ForkRecord,
    selection: &NextAgent,
    overrides: &serde_json::Map<String, serde_json::Value>,
    actor: &crate::session_folders::FolderActor,
) -> Result<(), String> {
    let target_mid = record.message_id.as_deref().or(record.point.as_deref());
    let mut before_seq = target_mid.and_then(|mid| state.hub.fork_turn_before_seq(id, mid));
    if before_seq.is_none()
        && let (Some(mid), Some(store)) = (target_mid, state.store.as_ref())
    {
        before_seq = store
            .turn_completion_before_seq(id, 0, mid)
            .await
            .ok()
            .flatten();
    }
    let before_seq = before_seq
        .or_else(|| state.hub.next_event_seq(id))
        .unwrap_or(0);
    if let Err(error) =
        lineage::record_edge(state, destination, source, "native_fork", before_seq, false).await
    {
        tracing::warn!(session = %destination, source = %id, %error, "native fork lineage edge not recorded");
    }
    state.hub.open_fork(destination, record, &source.title);
    inheritance::apply(state, source, created, selection)?;
    apply_overrides(state, destination, overrides)?;
    state.hub.place_fork(id, destination, actor)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuplicateRequest {
    /// Stable per distinct request; a retry with it returns the same copy.
    pub operation: String,
    pub provider: String,
    pub provider_account_id: crate::provider_account::ProviderAccountId,
    /// A signed preset of the target installation (`provider/preset`).
    #[serde(default)]
    pub preset: Option<String>,
    /// Explicit options for the copy, as the target offers them
    /// (`duplicate/options`); saved before its first launch.
    #[serde(default)]
    pub config: serde_json::Map<String, serde_json::Value>,
    /// Copy compatible settings from the source (default on).
    #[serde(default = "inherit_by_default")]
    pub inherit_settings: bool,
    /// Optional exclusive sequence cutoff (e.g. in-transcript "Fork from here").
    #[serde(default)]
    pub before_seq: Option<u64>,
    /// Whether to branch the new worktree from the source worktree (default on
    /// when the source has a workspace and the Machine supports fork worktrees).
    #[serde(default)]
    pub copy_workspace: Option<bool>,
    /// Whether to copy staged/unstaged/untracked files alongside the source's
    /// committed HEAD. When omitted on a busy source, safely defaults to
    /// committed HEAD only (or conversation-only on older Machines).
    #[serde(default)]
    pub include_uncommitted: Option<bool>,
}

fn inherit_by_default() -> bool {
    true
}

fn resolve_duplicate_workspace(
    state: &AppState,
    info: &SessionInfo,
    machine_id: &str,
    copy_workspace: Option<bool>,
    include_uncommitted: Option<bool>,
) -> Result<(Option<String>, Option<bool>), String> {
    if copy_workspace == Some(false) {
        return Ok((None, None));
    }
    let has_fork_worktree = state.machine_control.machine_supports(
        machine_id,
        crate::machine_protocol::NATIVE_FORK_PROTOCOL_VERSION,
    );
    if !has_fork_worktree {
        return Ok((None, None));
    }
    let busy = matches!(info.meta.status, Status::Busy | Status::Starting)
        || state.hub.session_has_in_flight_prompt(&info.meta.id);
    let supports_head_only = state.machine_control.machine_supports(
        machine_id,
        crate::machine_protocol::FORK_WORKSPACE_MODE_PROTOCOL_VERSION,
    );
    match include_uncommitted {
        Some(true) if busy => Err(
            "Source agent is running; uncheck Copy uncommitted changes to branch from committed HEAD, or wait for the turn to finish".into(),
        ),
        Some(false) if !supports_head_only => Err(format!(
            "Machine {machine_id} must be updated before it can fork from committed HEAD without uncommitted changes"
        )),
        Some(false) => Ok((Some(info.meta.id.clone()), Some(false))),
        None if busy && supports_head_only => Ok((Some(info.meta.id.clone()), Some(false))),
        None if busy => Ok((None, None)),
        _ => Ok((Some(info.meta.id.clone()), None)),
    }
}

async fn check_duplicate_receipt(
    state: &AppState,
    owner: &str,
    source_id: &str,
    operation: &str,
    fingerprint: &str,
) -> Result<Option<String>, String> {
    let Some(store) = state.store.as_ref() else {
        return Ok(None);
    };
    let existing = store
        .duplicate_receipt(owner, source_id, operation)
        .await
        .map_err(|_| "Duplicate receipt storage unavailable")?;
    let Some(receipt) = existing else {
        return Ok(None);
    };
    if receipt.fingerprint != fingerprint {
        return Err("This operation already created a different copy".into());
    }
    if state
        .hub
        .session_info(&receipt.destination_session_id)
        .is_some()
    {
        return Ok(Some(receipt.destination_session_id));
    }
    Ok(None)
}

/// A copy on another subscription or Provider (or where a native fork is
/// unavailable): a new agent on the source's Machines and project that reads
/// the source as linked context on its first message and shares its worktree
/// base. The source is untouched.
async fn duplicate_session(
    state: Arc<AppState>,
    auth: AuthenticatedProductRequest,
    id: String,
    request: DuplicateRequest,
) -> Result<String, String> {
    if !crate::managed_calls::valid_id(&request.operation) {
        return Err("A stable operation id is required".into());
    }
    let lock = write_lock(&id);
    let _guard = lock.lock().await;
    current_authority(&state, &auth)?;
    let info = owned(&state, &auth.principal, &id)?;
    valid_overrides(&request.config)?;
    let fingerprint = serde_json::json!({
        "provider": request.provider,
        "account": request.provider_account_id,
        "preset": request.preset,
        "config": request.config,
        "inherit": request.inherit_settings,
        "before_seq": request.before_seq,
        "copy_workspace": request.copy_workspace,
        "include_uncommitted": request.include_uncommitted,
    })
    .to_string();
    if let Some(existing) = check_duplicate_receipt(
        &state,
        &auth.principal.user_id,
        &id,
        &request.operation,
        &fingerprint,
    )
    .await?
    {
        return Ok(existing);
    }
    if state.hub.session_is_trashed(&id) {
        return Err("Restore this session from Trash before duplicating it".into());
    }
    if state.hub.is_managed_child(&id) {
        return Err("A managed call cannot be duplicated".into());
    }
    let source = info.meta.clone();
    let (machine_id, runtime_machine_id, project) = placement(&source)?;
    let (ws_base, ws_include_uncommitted) = resolve_duplicate_workspace(
        &state,
        &info,
        &machine_id,
        request.copy_workspace,
        request.include_uncommitted,
    )?;
    let selection = NextAgent {
        provider: request.provider.clone(),
        provider_account_id: request.provider_account_id.clone(),
        machine_id: machine_id.clone(),
        cwd: project.clone(),
        runtime_machine_id: runtime_machine_id.clone(),
        preset: request.preset.clone(),
        inherit_settings: request.inherit_settings,
        config: serde_json::Map::new(),
    };
    inheritance::preflight(&state, &auth, &source, &selection).await?;
    let actor = folder_actor(&state, &auth.principal);
    state.hub.continuation_folder(&id, &actor)?;

    let live_next = state.hub.next_event_seq(&id).unwrap_or(0);
    let before_seq = request
        .before_seq
        .map_or(live_next, |req| req.min(live_next));
    let destination = state.supervisor.reserve_session_id();
    let target_lock = write_lock(&destination);
    let _target_guard = target_lock.lock().await;

    // Persist cutoff, waiting link, and workspace base BEFORE api_new_session
    // schedules workspace preparation so preparation immediately sees ws_base
    // and the first prompt carries the exact frozen before_seq.
    save(
        &state,
        &destination,
        &Context {
            revision: 1,
            applied_revision: 1,
            sources: vec![id.clone()],
            waiting: vec![Link {
                session_id: id.clone(),
                before: before_seq,
                title: source.title.clone(),
            }],
            workspace_base: ws_base,
            workspace_include_uncommitted: ws_include_uncommitted,
            ..Context::default()
        },
    )
    .await?;

    let create = NewSessionRequest {
        provider: request.provider.clone(),
        provider_account_id: Some(request.provider_account_id.clone()),
        machine_id,
        runtime_machine_id,
        cwd: Some(project),
        preset: request.preset.clone(),
        folder_id: None,
        system: false,
        origin: SessionOrigin::Web,
        initial_prompt: None,
        context_sources: vec![],
        reserved_id: Some(destination.clone()),
        retained_auth_generation: None,
    };
    let response =
        api_new_session(State(state.clone()), Extension(auth.clone()), Json(create)).await;
    if !response.status().is_success() {
        state.hub.set_setting(
            crate::session_context::key(&destination),
            serde_json::Value::Null,
        );
        return Err(creation_error(response).await);
    }
    if let Some(store) = state.store.as_ref() {
        let _ = store
            .put_duplicate_receipt(&crate::store::DuplicateReceipt {
                owner_user_id: auth.principal.user_id.clone(),
                source_session_id: id.clone(),
                operation_id: request.operation.clone(),
                fingerprint,
                destination_session_id: destination.clone(),
                created_at_ms: chrono::Utc::now().timestamp_millis(),
            })
            .await;
    }
    if let Err(error) = lineage::record_edge(
        &state,
        &destination,
        &source,
        "linked_fork",
        before_seq,
        false,
    )
    .await
    {
        tracing::warn!(session = %destination, source = %id, %error, "linked fork lineage edge not recorded");
    }
    state.hub.set_forked_from(&destination, Some(id.clone()));
    finish_duplicate_session(&state, &auth, &id, &destination, &selection, &request.config, &actor)
        .map_err(|error| {
            format!("The copy {destination} was created but not finished ({error}); delete it and duplicate again")
        })?;
    Ok(destination)
}

fn finish_duplicate_session(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    id: &str,
    destination: &str,
    selection: &NextAgent,
    overrides: &serde_json::Map<String, serde_json::Value>,
    actor: &crate::session_folders::FolderActor,
) -> Result<(), String> {
    current_authority(state, auth)?;
    let source = owned(state, &auth.principal, id)?.meta;
    let created = owned(state, &auth.principal, destination)?.meta;
    if selection.inherit_settings {
        inheritance::apply(state, &source, &created, selection)?;
    }
    apply_overrides(state, destination, overrides)?;
    state.hub.place_copy(id, destination, actor)
}

/// The target a duplicate would create, without creating it.
#[derive(Debug, Deserialize)]
pub struct DuplicateOptionsRequest {
    pub provider: String,
    pub provider_account_id: crate::provider_account::ProviderAccountId,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default = "inherit_by_default")]
    pub inherit_settings: bool,
}

/// The options a copy on another Provider or subscription would offer, so
/// the fork form can configure every option before the copy exists.
pub async fn duplicate_options(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
    Json(request): Json<DuplicateOptionsRequest>,
) -> Response {
    let result = async {
        current_authority(&state, &auth)?;
        let source = owned(&state, &auth.principal, &id)?.meta;
        let (machine_id, runtime_machine_id, project) = placement(&source)?;
        let selection = NextAgent {
            provider: request.provider,
            provider_account_id: request.provider_account_id,
            machine_id,
            cwd: project,
            runtime_machine_id,
            preset: request.preset,
            inherit_settings: request.inherit_settings,
            config: serde_json::Map::new(),
        };
        inheritance::target_options(&state, &auth, &source, &selection).await
    }
    .await;
    match result {
        Ok(options) => Json(serde_json::json!({ "options": options })).into_response(),
        Err(error) => failure(error),
    }
}

pub async fn duplicate(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
    Json(request): Json<DuplicateRequest>,
) -> Response {
    let response_state = state.clone();
    // A dropped HTTP request does not cancel an admitted creation; the
    // creation-replay layer returns the same copy for a retried key.
    match tokio::spawn(duplicate_session(state, auth, id, request)).await {
        Ok(Ok(session_id)) => {
            let session = response_state
                .hub
                .session_info(&session_id)
                .map(|info| info.meta.for_client());
            (
                StatusCode::CREATED,
                Json(serde_json::json!({"session_id": session_id, "session": session})),
            )
                .into_response()
        }
        Ok(Err(error)) => failure(error),
        Err(_) => {
            failure("Duplicate outcome is unknown; inspect the session list before retrying".into())
        }
    }
}

pub async fn fork(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
    Json(request): Json<ForkRequest>,
) -> Response {
    let response_state = state.clone();
    // A dropped HTTP request does not cancel an admitted creation.
    match tokio::spawn(fork_session(state, auth, id, request)).await {
        Ok(Ok((session_id, created))) => {
            let session = response_state
                .hub
                .session_info(&session_id)
                .map(|info| info.meta.for_client());
            (
                if created {
                    StatusCode::CREATED
                } else {
                    StatusCode::OK
                },
                Json(serde_json::json!({"session_id": session_id, "session": session})),
            )
                .into_response()
        }
        Ok(Err(error)) => failure(error),
        Err(_) => {
            failure("Fork outcome is unknown; inspect the session list before retrying".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Hub, SessionRegistration};

    fn source(hub: &Hub) -> SessionInfo {
        hub.create_session(SessionRegistration {
            id: "source".to_owned(),
            provider: "claude-code".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "ovh".to_owned(),
            workspace_id: Some("columbus".to_owned()),
            workspace_name: Some("columbus".to_owned()),
            workspace_source_path: Some("/home/draven/columbus".to_owned()),
            execution_binding: None,
            cwd: "/home/draven/columbus".to_owned(),
            title: "Source".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.session_info("source").unwrap()
    }

    #[test]
    fn copy_overrides_are_option_ids_with_scalar_values() {
        let config = |value: serde_json::Value| {
            serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(value).unwrap()
        };
        assert!(
            valid_overrides(&config(serde_json::json!({"model": "opus", "fast": true}))).is_ok()
        );
        assert!(valid_overrides(&config(serde_json::json!({"model": {"nested": 1}}))).is_err());
        assert!(valid_overrides(&config(serde_json::json!({"": "opus"}))).is_err());
        let many = (0..33)
            .map(|index| (format!("o{index}"), serde_json::json!("x")))
            .collect();
        assert!(valid_overrides(&many).is_err());
    }

    #[test]
    fn a_local_session_forks_onto_its_own_machine_and_project() {
        let hub = Hub::new();
        let info = source(&hub);
        assert_eq!(
            placement(&info.meta),
            Ok(("ovh".to_owned(), None, "columbus".to_owned()))
        );
        let mut detached = info.meta;
        detached.workspace_id = None;
        assert!(placement(&detached).is_err());
    }

    #[test]
    fn a_fork_is_reused_only_from_the_same_answer_of_its_own_source() {
        let hub = Hub::new();
        source(&hub);
        let fork = |id: &str, source: &str, message: &str, state: ForkState| {
            hub.create_local_session(
                id.to_owned(),
                "claude-code".to_owned(),
                "/tmp".to_owned(),
                "t".to_owned(),
                crate::core::SessionOrigin::Web,
                false,
            );
            hub.stage_fork(
                id,
                &ForkRecord {
                    source_session_id: source.to_owned(),
                    source_agent_session_id: "thread".to_owned(),
                    message_id: Some(message.to_owned()),
                    point: None,
                    operation: id.to_owned(),
                    state,
                },
            );
        };
        fork("current", "source", "a1", ForkState::Forked);
        fork("refused", "source", "a1", ForkState::Refused);
        fork("foreign", "other", "a1", ForkState::Forked);

        assert!(reusable_fork(&hub, "source", "current", "a1"));
        assert!(!reusable_fork(&hub, "source", "current", "a2"), "behind");
        assert!(!reusable_fork(&hub, "source", "refused", "a1"));
        assert!(!reusable_fork(&hub, "source", "foreign", "a1"));
        assert!(!reusable_fork(&hub, "source", "missing", "a1"));
    }

    #[test]
    fn a_whole_fork_waits_for_an_idle_source_but_a_message_fork_does_not() {
        let hub = Hub::new();
        source(&hub);
        let chunk = |kind: &str, message_id: Option<&str>| {
            hub.push(
                "source",
                crate::agent_model::Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": kind, "messageId": message_id,
                        "content": {"type": "text", "text": "x"},
                    }),
                },
            );
        };
        chunk("user_message_chunk", None);
        chunk("agent_message_chunk", Some("a1"));
        chunk("user_message_chunk", None);
        hub.set_status("source", Status::Busy, None);
        let info = hub.session_info("source").unwrap();
        assert!(admit_point(&hub, &info, None).is_err());
        assert_eq!(admit_point(&hub, &info, Some("a1")), Ok(()));
        assert!(admit_point(&hub, &info, Some("gone")).is_err());
        assert!(admit_point(&hub, &info, Some("")).is_err());
        hub.set_status("source", Status::Running, None);
        let info = hub.session_info("source").unwrap();
        assert_eq!(admit_point(&hub, &info, None), Ok(()));
    }
}
