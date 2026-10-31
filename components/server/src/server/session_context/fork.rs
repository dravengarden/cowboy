//! Native session fork: a new session on the source's account, Provider and
//! Machines whose first thread is ACP `session/fork` of the source thread.
//! See `docs/session-fork.md`.
use super::*;
use crate::core::{ForkPoint, ForkRecord, ForkState, SessionMeta};

/// Providers whose pinned adapter fork was verified end to end.
const FORKABLE_PROVIDERS: [&str; 2] = ["claude-code", "codex"];
const MAX_MESSAGE_ID_BYTES: usize = 256;

#[derive(Debug, Deserialize)]
pub(in crate::server) struct ForkRequest {
    pub operation: String,
    /// Provider ID of a turn-final assistant message; omitted forks it all.
    #[serde(default)]
    pub message_id: Option<String>,
    /// Live options (model, reasoning, …) the copy uses instead of the
    /// inherited ones; saved before its first launch.
    #[serde(default)]
    pub config: serde_json::Map<String, serde_json::Value>,
}

/// Save explicit option choices on a dormant destination; its first launch
/// replays them, like any Settings change on a sleeping session.
fn apply_overrides(
    state: &AppState,
    destination: &str,
    config: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    for (config_id, value) in config {
        state
            .hub
            .set_config_preference(destination, config_id.clone(), value.clone())?;
    }
    Ok(())
}

fn valid_overrides(config: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    if config.len() > 32 {
        return Err("Too many configuration choices".into());
    }
    for (config_id, value) in config {
        if config_id.is_empty()
            || config_id.len() > 128
            || !matches!(
                value,
                serde_json::Value::String(_) | serde_json::Value::Bool(_)
            )
        {
            return Err(
                "Configuration choices must be option ids with a string or boolean value".into(),
            );
        }
    }
    Ok(())
}

/// Execution Machine, runtime Machine and project of an admitted session.
fn placement(meta: &SessionMeta) -> Result<(String, Option<String>, String), String> {
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

async fn fork_session(
    state: Arc<AppState>,
    auth: AuthenticatedProductRequest,
    id: String,
    request: ForkRequest,
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
        if record.message_id != request.message_id {
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
    if !FORKABLE_PROVIDERS.contains(&source.provider.as_str()) {
        return Err(format!(
            "{} sessions cannot be forked yet; use Continue in new agent",
            source.provider
        ));
    }
    let source_thread = state
        .hub
        .agent_session_id_for_resume(&id)
        .ok_or("This conversation has nothing to fork yet")?;
    admit_point(&state.hub, &info, request.message_id.as_deref())?;
    let (machine_id, runtime_machine_id, project) = placement(&source)?;
    let runtime = runtime_machine_id.as_deref().unwrap_or(&machine_id);
    for machine in [machine_id.as_str(), runtime] {
        if !state.machine_control.machine_supports(
            machine,
            crate::machine_protocol::NATIVE_FORK_PROTOCOL_VERSION,
        ) {
            return Err(format!(
                "Machine {machine} must be connected and updated before it can fork sessions"
            ));
        }
    }
    let selection = NextAgent {
        provider: source.provider.clone(),
        provider_account_id: source.provider_account_id.clone(),
        machine_id: machine_id.clone(),
        cwd: project.clone(),
        runtime_machine_id: runtime_machine_id.clone(),
        preset: None,
        inherit_settings: true,
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
    state.hub.open_fork(&destination, &record, &source.title);
    inheritance::apply(&state, &source, &created, &selection)?;
    apply_overrides(&state, &destination, &request.config)?;
    state.hub.place_fork(&id, &destination, &actor)?;
    Ok((destination, true))
}

/// Admitted copies by owner, source and operation, with the request they
/// answered: a retried or dropped request returns the same copy, and the same
/// operation never creates a second one for a different request.
type DuplicateKey = (String, String, String);
/// The copy's id and the request fingerprint it answered.
type DuplicateAnswer = (String, String);
static DUPLICATES: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashMap<DuplicateKey, DuplicateAnswer>>,
> = std::sync::LazyLock::new(Default::default);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct DuplicateRequest {
    /// Stable per distinct request; a retry with it returns the same copy.
    pub operation: String,
    pub provider: String,
    pub provider_account_id: crate::provider_account::ProviderAccountId,
    /// A signed preset of the target installation (`provider/preset`).
    #[serde(default)]
    pub preset: Option<String>,
    /// Explicit live options for the copy (same Provider only).
    #[serde(default)]
    pub config: serde_json::Map<String, serde_json::Value>,
    /// Copy compatible settings from the source (default on).
    #[serde(default = "inherit_by_default")]
    pub inherit_settings: bool,
}

fn inherit_by_default() -> bool {
    true
}

/// A copy on another subscription or Provider (or where a native fork is
/// unavailable): a new agent on the source's Machines and project that reads
/// the source as linked context on its first message. The source is untouched.
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
    let replay_key = (
        auth.principal.user_id.clone(),
        id.clone(),
        request.operation.clone(),
    );
    let fingerprint = serde_json::json!({
        "provider": request.provider,
        "account": request.provider_account_id,
        "preset": request.preset,
        "config": request.config,
        "inherit": request.inherit_settings,
    })
    .to_string();
    if let Some((destination, answered)) = DUPLICATES.lock().get(&replay_key).cloned() {
        if answered != fingerprint {
            return Err("This operation already created a different copy".into());
        }
        if state.hub.session_info(&destination).is_some() {
            return Ok(destination);
        }
    }
    if state.hub.session_is_trashed(&id) {
        return Err("Restore this session from Trash before duplicating it".into());
    }
    if state.hub.is_managed_child(&id) {
        return Err("A managed call cannot be duplicated".into());
    }
    if !request.config.is_empty() && request.provider != info.meta.provider {
        return Err(
            "Model and reasoning choices apply to the same Provider; choose a preset".into(),
        );
    }
    let source = info.meta.clone();
    let (machine_id, runtime_machine_id, project) = placement(&source)?;
    let selection = NextAgent {
        provider: request.provider.clone(),
        provider_account_id: request.provider_account_id.clone(),
        machine_id: machine_id.clone(),
        cwd: project.clone(),
        runtime_machine_id: runtime_machine_id.clone(),
        preset: request.preset.clone(),
        inherit_settings: request.inherit_settings,
    };
    inheritance::preflight(&state, &auth, &source, &selection).await?;
    let actor = folder_actor(&state, &auth.principal);
    state.hub.continuation_folder(&id, &actor)?;
    let destination = state.supervisor.reserve_session_id();
    // Fenced until placed, like a fork or continuation destination.
    let target_lock = write_lock(&destination);
    let _target_guard = target_lock.lock().await;
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
        context_sources: vec![id.clone()],
        reserved_id: Some(destination.clone()),
        retained_auth_generation: None,
    };
    let response =
        api_new_session(State(state.clone()), Extension(auth.clone()), Json(create)).await;
    if !response.status().is_success() {
        return Err(creation_error(response).await);
    }
    DUPLICATES
        .lock()
        .insert(replay_key, (destination.clone(), fingerprint));
    let finish = || -> Result<(), String> {
        current_authority(&state, &auth)?;
        let source = owned(&state, &auth.principal, &id)?.meta;
        let created = owned(&state, &auth.principal, &destination)?.meta;
        if selection.inherit_settings {
            inheritance::apply(&state, &source, &created, &selection)?;
        }
        apply_overrides(&state, &destination, &request.config)?;
        state.hub.place_copy(&id, &destination, &actor)
    };
    finish().map_err(|error| {
        format!("The copy {destination} was created but not finished ({error}); delete it and duplicate again")
    })?;
    Ok(destination)
}

pub(in crate::server) async fn duplicate(
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
                .map(|info| info.meta);
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

pub(in crate::server) async fn fork(
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
                .map(|info| info.meta);
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
