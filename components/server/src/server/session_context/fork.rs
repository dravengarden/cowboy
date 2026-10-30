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
    if let Some((destination, record)) = state.hub.fork_by_operation(&id, &request.operation) {
        if record.message_id != request.message_id {
            return Err("This operation already forked another point".into());
        }
        return Ok((destination, false));
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
    // this Machine. A newer login generation has another home.
    let generation = inheritance::target_generation(&state, &auth, &selection).await?;
    if generation.auth_generation != source.provider_auth_generation {
        return Err(
            "This subscription was signed in again since the session started; use Continue in new agent".into(),
        );
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
        operation: request.operation.clone(),
        state: ForkState::Pending,
    };
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
        return Err(format!(
            "The new session {destination} was admitted on another login or Machine and cannot fork this conversation; delete it and retry"
        ));
    }
    state.hub.open_fork(&destination, &record, &source.title);
    inheritance::apply(&state, &source, &created, &selection)?;
    state.hub.place_fork(&id, &destination, &actor)?;
    Ok((destination, true))
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
