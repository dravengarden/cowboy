//! Native session fork: a new session on the source's account, Provider and
//! Machines whose first thread is ACP `session/fork` of the source thread.
//! See `docs/session-fork.md`.
use super::*;
use crate::core::{ForkPoint, ForkRecord, ForkState, SessionMeta};

/// Providers whose pinned adapter fork was verified end to end.
const FORKABLE_PROVIDERS: [&str; 2] = ["claude-code", "codex"];
const MAX_MESSAGE_ID_BYTES: usize = 256;

/// Typed native-fork failure: `unsupported_fallback` is true ONLY for
/// pre-admission refusals where no destination was created and Linked Fork is a
/// safe substitute. Post-admission failures never set it.
struct ForkFailure {
    message: String,
    unsupported_fallback: bool,
}

impl ForkFailure {
    fn unsupported(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            unsupported_fallback: true,
        }
    }
}

impl From<String> for ForkFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            unsupported_fallback: false,
        }
    }
}

impl From<&str> for ForkFailure {
    fn from(message: &str) -> Self {
        Self {
            message: message.to_owned(),
            unsupported_fallback: false,
        }
    }
}

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
            && !matches!(record.state, ForkState::Refused | ForkState::Staging)
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

async fn resolve_latest_completed(
    state: &AppState,
    id: &str,
    generation: Option<u64>,
) -> Result<String, String> {
    if let Some(found) = state.hub.last_completed_answer_id(id) {
        return Ok(found);
    }
    if let Some(store) = state.store.as_ref()
        && let Some(generation) = generation
        && let Ok(Some((mid, _))) = store.latest_turn_completion(id, generation).await
    {
        return Ok(mid);
    }
    Err("Wait for this agent's first answer before forking".into())
}

async fn resolve_native_fork_before_seq(
    state: &AppState,
    id: &str,
    generation: Option<u64>,
    explicit_mid: Option<&str>,
) -> Result<Option<u64>, String> {
    let Some(mid) = explicit_mid else {
        return Ok(None);
    };
    if let Some(seq) = state.hub.fork_turn_before_seq(id, mid) {
        return Ok(Some(seq));
    }
    let Some(store) = state.store.as_ref() else {
        return Err("The selected historical answer is no longer available as a fork point".into());
    };
    if let Some(generation) = generation {
        let found = store
            .turn_completion_before_seq(id, generation, mid)
            .await
            .map_err(|_| "Turn completion storage unavailable; retry shortly".to_owned())?;
        if let Some(seq) = found {
            return Ok(Some(seq));
        }
    }
    let scanned = store
        .find_message_turn_before_seq(id, mid)
        .await
        .map_err(|_| "History storage unavailable while locating fork point".to_owned())?;
    scanned.map(Some).ok_or_else(|| {
        "The selected historical answer is no longer available as a fork point".into()
    })
}

fn fork_setting_key(destination: &str) -> String {
    format!(
        "{}{destination}",
        crate::core::settings_keys::SESSION_FORK_PREFIX
    )
}

fn native_selection(
    source: &SessionMeta,
    machine_id: &str,
    runtime_machine_id: Option<&str>,
    project: &str,
) -> NextAgent {
    NextAgent {
        provider: source.provider.clone(),
        provider_account_id: source.provider_account_id.clone(),
        machine_id: machine_id.to_owned(),
        cwd: project.to_owned(),
        runtime_machine_id: runtime_machine_id.map(str::to_owned),
        preset: None,
        inherit_settings: true,
        config: serde_json::Map::new(),
    }
}

fn destination_matches_native_source(created: &SessionMeta, source: &SessionMeta) -> bool {
    created.provider == source.provider
        && created.provider_account_id == source.provider_account_id
        && created.machine_id == source.machine_id
        && created.provider_auth_generation == source.provider_auth_generation
}

async fn persist_fork_state(
    state: &AppState,
    destination: &str,
    record: &ForkRecord,
    next_state: ForkState,
) -> Result<(), String> {
    let store = state.store.as_ref().ok_or("Fork storage unavailable")?;
    let mut updated = record.clone();
    updated.state = next_state;
    let value = serde_json::to_value(&updated).expect("fork record serializes");
    store
        .put_setting(&fork_setting_key(destination), &value)
        .await
        .map_err(|_| "Fork storage unavailable".to_owned())
}

async fn verify_staging_source_intact(
    state: &AppState,
    id: &str,
    destination: &str,
    record: &ForkRecord,
) -> Result<Option<crate::core::LineageCaptureGuard>, String> {
    let has_edge = match state.store.as_ref() {
        Some(store) => store
            .lineage_edge(destination)
            .await
            .map_err(|error| format!("lineage lookup failed: {error}"))?
            .is_some(),
        None => false,
    };
    let before_seq = record.before_seq.unwrap_or(0);
    // Always pin the source and revalidate native thread + context generation
    // before promoting a staging native fork, even if the display edge already
    // exists, so a Clear or context rollover cannot leave a discarded thread ID.
    let pin_guard = lineage::pin_and_freeze_source(state, id, Some(before_seq))
        .await?
        .1;
    let identity = state.hub.snapshot_native_source_identity(id);
    let epoch_changed = match identity {
        Ok((live_thread, live_gen)) => {
            live_thread != record.source_agent_session_id
                || (record.source_context_generation.is_some()
                    && live_gen != record.source_context_generation)
        }
        Err(_) => true,
    };
    let cleared_prefix = if !has_edge
        && before_seq > 0
        && let Some(store) = state.store.as_ref()
    {
        store
            .latest_context_clear_seq(id)
            .await
            .map_err(|error| format!("clear boundary check failed: {error}"))?
            .is_some_and(|clear_seq| clear_seq >= before_seq)
    } else {
        false
    };
    if epoch_changed || cleared_prefix {
        let _ = persist_fork_state(state, destination, record, ForkState::Refused).await;
        state.hub.refuse_native_fork(destination);
        return Err(
            "Source conversation context changed or cleared before this fork finished setting up; start a new fork"
                .into(),
        );
    }
    Ok(Some(pin_guard))
}

async fn resume_native_fork(
    state: &Arc<AppState>,
    auth: &AuthenticatedProductRequest,
    id: &str,
    info: &SessionInfo,
    request: &ForkRequest,
    destination: &str,
    record: &ForkRecord,
) -> Result<(String, bool), ForkFailure> {
    if !request.latest_completed && record.message_id != request.message_id {
        return Err("This operation already forked another point".into());
    }
    if record
        .config
        .as_ref()
        .is_some_and(|saved| saved != &request.config)
    {
        return Err("This operation already forked with different options".into());
    }
    if record.state == ForkState::Refused {
        return Err(format!(
            "The new session {destination} could not fork this conversation; delete it and fork again"
        )
        .into());
    }
    if record.state != ForkState::Staging {
        owned(state, &auth.principal, destination)?;
        return Ok((destination.to_owned(), false));
    }
    let target_lock = write_lock(destination);
    let _target_guard = target_lock.lock().await;
    let current = state
        .hub
        .session_fork(destination)
        .unwrap_or_else(|| record.clone());
    if current.state == ForkState::Refused {
        return Err(format!(
            "The new session {destination} could not fork this conversation; delete it and fork again"
        )
        .into());
    }
    if current.state != ForkState::Staging {
        owned(state, &auth.principal, destination)?;
        return Ok((destination.to_owned(), false));
    }
    let _pin_guard = verify_staging_source_intact(state, id, destination, &current).await?;
    let source = info.meta.clone();
    let (machine_id, runtime_machine_id, project) = placement(&source)?;
    let selection = native_selection(
        &source,
        &machine_id,
        runtime_machine_id.as_deref(),
        &project,
    );
    let actor = folder_actor(state, &auth.principal);
    if state.hub.session_info(destination).is_none() {
        if state.hub.session_is_trashed(id) {
            return Err("Restore this session from Trash before forking it".into());
        }
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
            reserved_id: Some(destination.to_owned()),
            retained_auth_generation: source.provider_auth_generation,
        };
        let response =
            api_new_session(State(state.clone()), Extension(auth.clone()), Json(create)).await;
        if !response.status().is_success() {
            return Err(creation_error(response).await.into());
        }
    }
    let created = owned(state, &auth.principal, destination)?.meta;
    if !destination_matches_native_source(&created, &source) {
        let _ = persist_fork_state(state, destination, &current, ForkState::Refused).await;
        state.hub.refuse_native_fork(destination);
        return Err(format!(
            "The new session {destination} was admitted on another login or Machine and cannot fork this conversation; delete it and retry"
        )
        .into());
    }
    finish_native_fork(
        state,
        id,
        destination,
        &source,
        &created,
        &current,
        &selection,
        &request.config,
        &actor,
        current.before_seq.unwrap_or(0),
        matches!(auth.caller, crate::prompt_origin::RequestCaller::Person),
    )
    .await?;
    Ok((destination.to_owned(), false))
}

async fn fork_session(
    state: Arc<AppState>,
    auth: AuthenticatedProductRequest,
    id: String,
    mut request: ForkRequest,
) -> Result<(String, bool), ForkFailure> {
    let lock = write_lock(&id);
    let _guard = lock.lock().await;
    current_authority(&state, &auth)?;
    let info = owned(&state, &auth.principal, &id)?;
    if !crate::managed_calls::valid_id(&request.operation) {
        return Err("A stable operation id is required".into());
    }
    valid_overrides(
        &request.config,
        matches!(auth.caller, crate::prompt_origin::RequestCaller::Person),
    )?;
    if let Some((destination, record)) = state.hub.fork_by_operation(&id, &request.operation) {
        return resume_native_fork(&state, &auth, &id, &info, &request, &destination, &record)
            .await;
    }
    if state.hub.session_is_trashed(&id) {
        return Err("Restore this session from Trash before forking it".into());
    }
    let source = info.meta.clone();
    forkable(&state, &source).map_err(ForkFailure::unsupported)?;
    let (source_thread, source_generation) = state
        .hub
        .snapshot_native_source_identity(&id)
        .map_err(ForkFailure::unsupported)?;
    if request.latest_completed {
        let latest = resolve_latest_completed(&state, &id, source_generation).await?;
        if let Some(fork) = request.reuse.as_deref().filter(|fork| {
            reusable_fork(&state.hub, &id, fork, &latest)
                && owned(&state, &auth.principal, fork).is_ok()
        }) {
            return Ok((fork.to_owned(), false));
        }
        request.message_id = Some(latest);
    }
    admit_native_point(&state.hub, &info, request.message_id.as_deref())?;
    let (machine_id, runtime_machine_id, project) = placement(&source)?;
    check_fork_machines(&state, &machine_id, runtime_machine_id.as_deref())
        .map_err(ForkFailure::unsupported)?;
    let selection = native_selection(
        &source,
        &machine_id,
        runtime_machine_id.as_deref(),
        &project,
    );
    let mut generation = inheritance::target_generation(&state, &auth, &selection).await?;
    if let Some(retained) = source.provider_auth_generation {
        provider_accounts::registration::retain_auth_generation(
            &mut generation,
            &source.provider_account_id,
            retained,
        )
        .map_err(ForkFailure::unsupported)?;
    } else if generation.auth_generation.is_some() {
        return Err(ForkFailure::unsupported(
            provider_accounts::registration::SIGNED_IN_AGAIN,
        ));
    }
    inheritance::preflight(&state, &auth, &source, &selection).await?;
    let actor = folder_actor(&state, &auth.principal);
    state.hub.continuation_folder(&id, &actor)?;

    let explicit_before = resolve_native_fork_before_seq(
        &state,
        &id,
        source_generation,
        request.message_id.as_deref(),
    )
    .await
    .map_err(|msg| {
        if msg.contains("no longer available as a fork point") {
            ForkFailure::unsupported(msg)
        } else {
            ForkFailure::from(msg)
        }
    })?;
    let (before_seq, _pin_guard) =
        lineage::pin_and_freeze_source(&state, &id, explicit_before).await?;
    // Revalidate source thread & generation under the Clear-blocking pin so an
    // interleaved context reset cannot pair an old thread/cutoff with new state.
    if state.hub.snapshot_native_source_identity(&id)? != (source_thread.clone(), source_generation)
    {
        return Err("Source conversation context changed while preparing the fork; retry".into());
    }
    let fresh_info = owned(&state, &auth.principal, &id)?;
    admit_point(&state.hub, &fresh_info, request.message_id.as_deref())?;
    let point = request
        .message_id
        .is_none()
        .then(|| state.hub.last_answer_id(&id))
        .flatten();
    let record = ForkRecord {
        source_session_id: id.clone(),
        source_agent_session_id: source_thread,
        message_id: request.message_id.clone(),
        point,
        before_seq: Some(before_seq),
        source_context_generation: source_generation,
        config: Some(request.config.clone()),
        operation: request.operation.clone(),
        state: ForkState::Staging,
    };
    create_and_finish_native_fork(
        &state,
        &auth,
        &id,
        &source,
        &record,
        &selection,
        &request.config,
        &actor,
        before_seq,
    )
    .await
}

fn admit_native_point(
    hub: &crate::core::Hub,
    info: &SessionInfo,
    message_id: Option<&str>,
) -> Result<(), ForkFailure> {
    admit_point(hub, info, message_id).map_err(|msg| {
        if let Some(mid) = message_id
            && matches!(
                hub.fork_point(&info.meta.id, mid),
                ForkPoint::NotTurnFinal | ForkPoint::Missing
            )
        {
            ForkFailure::unsupported(msg)
        } else {
            ForkFailure::from(msg)
        }
    })
}

#[allow(clippy::too_many_arguments)]
async fn create_and_finish_native_fork(
    state: &Arc<AppState>,
    auth: &AuthenticatedProductRequest,
    id: &str,
    source: &SessionMeta,
    record: &ForkRecord,
    selection: &NextAgent,
    overrides: &serde_json::Map<String, serde_json::Value>,
    actor: &crate::session_folders::FolderActor,
    before_seq: u64,
) -> Result<(String, bool), ForkFailure> {
    let destination = state.supervisor.reserve_session_id();
    let target_lock = write_lock(&destination);
    let _target_guard = target_lock.lock().await;
    let store = state.store.as_ref().ok_or("Fork storage unavailable")?;
    let key = fork_setting_key(&destination);
    let value = serde_json::to_value(record).expect("fork record serializes");
    if !store
        .compare_and_put_setting(&key, None, &value)
        .await
        .map_err(|_| "Fork storage unavailable")?
    {
        return Err("Fork storage changed; retry".into());
    }
    state.hub.stage_fork(&destination, record);
    let create = NewSessionRequest {
        provider: source.provider.clone(),
        provider_account_id: Some(source.provider_account_id.clone()),
        machine_id: selection.machine_id.clone(),
        runtime_machine_id: selection.runtime_machine_id.clone(),
        cwd: Some(selection.cwd.clone()),
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
        return Err(creation_error(response).await.into());
    }
    let created = owned(state, &auth.principal, &destination)?.meta;
    if !destination_matches_native_source(&created, source) {
        let _ = persist_fork_state(state, &destination, record, ForkState::Refused).await;
        state.hub.refuse_native_fork(&destination);
        return Err(format!(
            "The new session {destination} was admitted on another login or Machine and cannot fork this conversation; delete it and retry"
        )
        .into());
    }
    finish_native_fork(
        state,
        id,
        &destination,
        source,
        &created,
        record,
        selection,
        overrides,
        actor,
        before_seq,
        matches!(auth.caller, crate::prompt_origin::RequestCaller::Person),
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
    before_seq: u64,
    owner_authorized: bool,
) -> Result<(), String> {
    ensure_lineage_edge(state, destination, source, "native_fork", before_seq)
        .await
        .map_err(|error| {
            format!(
                "The new session {destination} was created but its lineage could not be saved ({error}); retry with the same operation to finish setup"
            )
        })?;
    state.hub.open_fork(destination, record, &source.title);
    inheritance::apply(state, source, created, selection)?;
    apply_overrides(state, destination, overrides, owner_authorized)?;
    state.hub.place_fork(id, destination, actor)?;
    lineage::flush_store_barrier(state).await.map_err(|error| {
        format!(
            "The new session {destination} was created but durable persistence did not finish ({error}); retry with the same operation"
        )
    })?;
    persist_fork_state(state, destination, record, ForkState::Pending).await?;
    state.hub.activate_native_fork(destination);
    Ok(())
}

/// Record a direct lineage edge once; on setup retry keep any already-committed
/// edge/cutoff instead of overwriting it with a newer source state.
async fn ensure_lineage_edge(
    state: &AppState,
    destination: &str,
    source: &SessionMeta,
    kind: &str,
    before_seq: u64,
) -> Result<(), String> {
    if let Some(store) = state.store.as_ref()
        && let Some(existing) = store
            .lineage_edge(destination)
            .await
            .map_err(|error| format!("lineage lookup failed: {error}"))?
    {
        if existing.parent_session_id != source.id {
            return Err("destination already belongs to another lineage".into());
        }
        return Ok(());
    }
    lineage::record_edge(state, destination, source, kind, before_seq, false).await
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
        return if copy_workspace == Some(true) {
            Err(format!(
                "Machine {machine_id} does not support forking worktrees yet"
            ))
        } else {
            Ok((None, None))
        };
    }
    // Only branch from a materialized source worktree; an unmaterialized or
    // still-preparing session cannot serve as a base worktree.
    let materialized = info
        .meta
        .workspace_id
        .as_deref()
        .is_some_and(|w| !w.is_empty())
        && !info.meta.cwd.is_empty()
        && !info.meta.awaits_workspace_preparation()
        && !state.workspace_preparations.lock().contains(&info.meta.id)
        && info.meta.status != Status::Starting
        && (info.meta.workspace_source_path.as_deref() != Some(info.meta.cwd.as_str())
            || info.meta.agent_session_id.is_some());
    if !materialized {
        return if copy_workspace == Some(true) {
            Err("Source worktree is not materialized yet; retry once the session is ready".into())
        } else {
            Ok((None, None))
        };
    }
    let busy = matches!(info.meta.status, Status::Busy | Status::Starting)
        || state.hub.session_has_in_flight_prompt(&info.meta.id)
        || info.queue_count > 0
        || info.meta.background_tasks > 0;
    let supports_head_only = state.machine_control.machine_supports(
        machine_id,
        crate::machine_protocol::FORK_WORKSPACE_MODE_PROTOCOL_VERSION,
    );
    match include_uncommitted {
        Some(true) if busy => Err(
            "Source agent or background work is active; uncheck Copy uncommitted changes to branch from committed HEAD, or wait until idle".into(),
        ),
        Some(false) if !supports_head_only => Err(format!(
            "Machine {machine_id} must be updated before it can fork from committed HEAD without uncommitted changes"
        )),
        Some(false) => Ok((Some(info.meta.id.clone()), Some(false))),
        None if busy && supports_head_only => Ok((Some(info.meta.id.clone()), Some(false))),
        None if busy && copy_workspace == Some(true) => Err(format!(
            "Machine {machine_id} must be updated before it can fork an active session's worktree"
        )),
        None if busy => Ok((None, None)),
        _ => Ok((Some(info.meta.id.clone()), None)),
    }
}

struct DuplicatePlan {
    destination: String,
    before_seq: u64,
    ws_base: Option<String>,
    ws_include_uncommitted: Option<bool>,
    resumed: bool,
}

enum DuplicateLookup {
    AlreadyReady(String),
    Reserved(DuplicatePlan),
    Fresh,
}

fn interpret_existing_duplicate(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    fingerprint: &str,
    receipt: crate::store::DuplicateReceipt,
) -> Result<DuplicateLookup, String> {
    if receipt.fingerprint != fingerprint {
        return Err("This operation already created a different copy".into());
    }
    if receipt.state == "cleared" {
        return Err(
            "The source conversation was cleared before this fork finished setting up; start a new fork"
                .into(),
        );
    }
    if receipt.state == "deleted" {
        return Err("The copy created by this operation was deleted; start a new fork".into());
    }
    if receipt.state == "ready" {
        if let Some(info) = state.hub.session_info(&receipt.destination_session_id) {
            if info.meta.closing {
                return Err(
                    "The copy created by this operation is currently closing; wait a moment and check the session list"
                        .into(),
                );
            }
            owned(state, &auth.principal, &receipt.destination_session_id)?;
            return Ok(DuplicateLookup::AlreadyReady(
                receipt.destination_session_id,
            ));
        }
        return Err("The copy created by this operation was deleted; start a new fork".into());
    }
    Ok(DuplicateLookup::Reserved(DuplicatePlan {
        destination: receipt.destination_session_id,
        before_seq: receipt.before_seq,
        ws_base: receipt.workspace_base,
        ws_include_uncommitted: receipt.workspace_include_uncommitted,
        resumed: true,
    }))
}

async fn lookup_existing_duplicate(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    source_id: &str,
    operation: &str,
    fingerprint: &str,
) -> Result<DuplicateLookup, String> {
    let Some(store) = state.store.as_ref() else {
        return Ok(DuplicateLookup::Fresh);
    };
    let Some(receipt) = store
        .duplicate_receipt(&auth.principal.user_id, source_id, operation)
        .await
        .map_err(|_| "Duplicate receipt storage unavailable".to_owned())?
    else {
        return Ok(DuplicateLookup::Fresh);
    };
    interpret_existing_duplicate(state, auth, fingerprint, receipt)
}

async fn reserve_duplicate_operation(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    source_id: &str,
    operation: &str,
    fingerprint: &str,
    before_seq: u64,
    workspace: (Option<String>, Option<bool>),
) -> Result<DuplicateLookup, String> {
    let (ws_base, ws_include_uncommitted) = workspace;
    let candidate = state.supervisor.reserve_session_id();
    let Some(store) = state.store.as_ref() else {
        return Ok(DuplicateLookup::Reserved(DuplicatePlan {
            destination: candidate,
            before_seq,
            ws_base,
            ws_include_uncommitted,
            resumed: false,
        }));
    };
    let now = chrono::Utc::now().timestamp_millis();
    let outcome = store
        .reserve_duplicate_receipt(&crate::store::DuplicateReceipt {
            owner_user_id: auth.principal.user_id.clone(),
            source_session_id: source_id.to_owned(),
            operation_id: operation.to_owned(),
            fingerprint: fingerprint.to_owned(),
            destination_session_id: candidate.clone(),
            before_seq,
            workspace_base: ws_base.clone(),
            workspace_include_uncommitted: ws_include_uncommitted,
            state: "reserved".to_owned(),
            created_at_ms: now,
            updated_at_ms: now,
        })
        .await
        .map_err(|_| "Duplicate receipt storage unavailable".to_owned())?;
    match outcome {
        crate::store::ReserveOutcome::Reserved => Ok(DuplicateLookup::Reserved(DuplicatePlan {
            destination: candidate,
            before_seq,
            ws_base,
            ws_include_uncommitted,
            resumed: false,
        })),
        crate::store::ReserveOutcome::Existing(receipt) => {
            interpret_existing_duplicate(state, auth, fingerprint, receipt)
        }
    }
}

async fn verify_duplicate_destination_binding(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    source_id: &str,
    destination: &str,
) -> Result<(), String> {
    let info = owned(state, &auth.principal, destination)?;
    if info.meta.forked_from.as_deref() == Some(source_id) {
        return Ok(());
    }
    if state
        .hub
        .setting(&crate::session_context::key(destination))
        .and_then(|v| serde_json::from_value::<Context>(v).ok())
        .is_some_and(|ctx| {
            ctx.sources.iter().any(|s| s == source_id)
                || ctx.waiting.iter().any(|l| l.session_id == source_id)
        })
    {
        return Ok(());
    }
    if let Some(store) = state.store.as_ref()
        && let Ok(Some(edge)) = store.lineage_edge(destination).await
        && edge.parent_session_id == source_id
    {
        return Ok(());
    }
    Err(format!(
        "Reserved destination {destination} does not belong to this fork operation; start a new fork"
    ))
}

fn duplicate_fingerprint(request: &DuplicateRequest) -> String {
    serde_json::json!({
        "provider": request.provider,
        "account": request.provider_account_id,
        "preset": request.preset,
        "config": request.config,
        "inherit": request.inherit_settings,
        "before_seq": request.before_seq,
        "copy_workspace": request.copy_workspace,
        "include_uncommitted": request.include_uncommitted,
    })
    .to_string()
}

/// A copy on another subscription or Provider (or where a native fork is
/// unavailable): replays ready receipts before source preflight, reserves its
/// durable capture plan before creation, resumes incomplete setup on retry, and
/// waits for durable storage before marking the receipt ready.
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
    valid_overrides(
        &request.config,
        matches!(auth.caller, crate::prompt_origin::RequestCaller::Person),
    )?;
    let fingerprint = duplicate_fingerprint(&request);
    let existing =
        lookup_existing_duplicate(&state, &auth, &id, &request.operation, &fingerprint).await?;
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
        config: serde_json::Map::new(),
    };
    let actor = folder_actor(&state, &auth.principal);

    let (plan, _pin_guard) = match prepare_duplicate_plan(
        &state,
        &auth,
        &id,
        &info,
        &selection,
        &actor,
        &request,
        &fingerprint,
        existing,
    )
    .await?
    {
        Ok(ready) => return Ok(ready),
        Err(planned) => planned,
    };

    let target_lock = write_lock(&plan.destination);
    let _target_guard = target_lock.lock().await;
    if state.hub.session_info(&plan.destination).is_none() {
        create_duplicate_destination(
            &state,
            &auth,
            &id,
            &source.title,
            &plan,
            &request,
            machine_id,
            runtime_machine_id,
            project,
        )
        .await?;
    } else {
        verify_duplicate_destination_binding(&state, &auth, &id, &plan.destination).await?;
    }
    ensure_lineage_edge(&state, &plan.destination, &source, "linked_fork", plan.before_seq)
        .await
        .map_err(|error| {
            format!(
                "The copy {} was created but its lineage could not be saved ({error}); retry with the same operation to finish setup",
                plan.destination
            )
        })?;
    state
        .hub
        .set_forked_from(&plan.destination, Some(id.clone()));
    finish_duplicate_session(
        &state,
        &auth,
        &id,
        &plan.destination,
        &selection,
        &request.config,
        &actor,
    )
    .map_err(|error| {
        format!(
            "The copy {} was created but not finished ({error}); retry with the same operation to finish setup",
            plan.destination
        )
    })?;
    lineage::flush_store_barrier(&state).await.map_err(|error| {
        format!(
            "The copy {} was created but durable persistence did not finish ({error}); retry with the same operation",
            plan.destination
        )
    })?;
    if let Some(store) = state.store.as_ref() {
        let completed = store
            .complete_duplicate_receipt(
                &auth.principal.user_id,
                &id,
                &request.operation,
                chrono::Utc::now().timestamp_millis(),
            )
            .await
            .map_err(|_| {
                "Duplicate receipt completion failed; retry with the same operation".to_owned()
            })?;
        if !completed {
            return Err(
                "The copy created by this operation is no longer available; start a new fork"
                    .into(),
            );
        }
    }
    Ok(plan.destination)
}

#[allow(clippy::too_many_arguments)]
async fn prepare_duplicate_plan(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    id: &str,
    info: &SessionInfo,
    selection: &NextAgent,
    actor: &crate::session_folders::FolderActor,
    request: &DuplicateRequest,
    fingerprint: &str,
    existing: DuplicateLookup,
) -> Result<Result<String, (DuplicatePlan, Option<crate::core::LineageCaptureGuard>)>, String> {
    match existing {
        DuplicateLookup::AlreadyReady(dest) => Ok(Ok(dest)),
        DuplicateLookup::Reserved(plan) => {
            let needs_create = state.hub.session_info(&plan.destination).is_none();
            if needs_create {
                preflight_duplicate_source(state, auth, id, &info.meta, selection, actor).await?;
            }
            let pin = verify_reserved_duplicate_prefix(
                state,
                auth,
                id,
                &request.operation,
                &plan,
                needs_create,
            )
            .await?;
            Ok(Err((plan, pin)))
        }
        DuplicateLookup::Fresh => {
            preflight_duplicate_source(state, auth, id, &info.meta, selection, actor).await?;
            let (ws_base, ws_include_uncommitted) = resolve_duplicate_workspace(
                state,
                info,
                &selection.machine_id,
                request.copy_workspace,
                request.include_uncommitted,
            )?;
            let (before_seq, pin_guard) =
                lineage::pin_and_freeze_source(state, id, request.before_seq).await?;
            match reserve_duplicate_operation(
                state,
                auth,
                id,
                &request.operation,
                fingerprint,
                before_seq,
                (ws_base, ws_include_uncommitted),
            )
            .await?
            {
                DuplicateLookup::AlreadyReady(dest) => Ok(Ok(dest)),
                DuplicateLookup::Reserved(plan) => Ok(Err((plan, Some(pin_guard)))),
                DuplicateLookup::Fresh => unreachable!(),
            }
        }
    }
}

async fn verify_reserved_duplicate_prefix(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    id: &str,
    operation: &str,
    plan: &DuplicatePlan,
    needs_create: bool,
) -> Result<Option<crate::core::LineageCaptureGuard>, String> {
    let has_edge = match state.store.as_ref() {
        Some(store) => store
            .lineage_edge(&plan.destination)
            .await
            .map_err(|error| format!("lineage lookup failed: {error}"))?
            .is_some(),
        None => false,
    };
    if has_edge || plan.before_seq == 0 {
        return Ok(None);
    }
    let guard = lineage::pin_and_freeze_source(state, id, Some(plan.before_seq))
        .await?
        .1;
    if let Some(store) = state.store.as_ref()
        && store
            .latest_context_clear_seq(id)
            .await
            .map_err(|error| format!("clear boundary check failed: {error}"))?
            .is_some_and(|clear_seq| clear_seq >= plan.before_seq)
    {
        if needs_create {
            let _ = store
                .delete_duplicate_receipt(&auth.principal.user_id, id, operation)
                .await;
        }
        return Err(
            "The source conversation was cleared before this fork finished setting up; start a new fork"
                .into(),
        );
    }
    Ok(Some(guard))
}

async fn preflight_duplicate_source(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    id: &str,
    source: &SessionMeta,
    selection: &NextAgent,
    actor: &crate::session_folders::FolderActor,
) -> Result<(), String> {
    if state.hub.session_is_trashed(id) {
        return Err("Restore this session from Trash before duplicating it".into());
    }
    if state.hub.is_managed_child(id) {
        return Err("A managed call cannot be duplicated".into());
    }
    inheritance::preflight(state, auth, source, selection).await?;
    state.hub.continuation_folder(id, actor)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn create_duplicate_destination(
    state: &Arc<AppState>,
    auth: &AuthenticatedProductRequest,
    id: &str,
    source_title: &str,
    plan: &DuplicatePlan,
    request: &DuplicateRequest,
    machine_id: String,
    runtime_machine_id: Option<String>,
    project: String,
) -> Result<(), String> {
    if let Err(error) = save(
        state,
        &plan.destination,
        &Context {
            revision: 1,
            applied_revision: 1,
            sources: vec![id.to_owned()],
            waiting: vec![Link {
                session_id: id.to_owned(),
                before: plan.before_seq,
                title: source_title.to_owned(),
            }],
            workspace_base: plan.ws_base.clone(),
            workspace_include_uncommitted: plan.ws_include_uncommitted,
            ..Context::default()
        },
    )
    .await
    {
        if !plan.resumed
            && let Some(store) = state.store.as_ref()
        {
            let _ = store
                .delete_duplicate_receipt(&auth.principal.user_id, id, &request.operation)
                .await;
        }
        return Err(error);
    }
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
        reserved_id: Some(plan.destination.clone()),
        retained_auth_generation: None,
    };
    let response =
        api_new_session(State(state.clone()), Extension(auth.clone()), Json(create)).await;
    if !response.status().is_success() {
        state.hub.set_setting(
            crate::session_context::key(&plan.destination),
            serde_json::Value::Null,
        );
        if state.hub.session_info(&plan.destination).is_none()
            && let Some(store) = state.store.as_ref()
        {
            let _ = store
                .delete_duplicate_receipt(&auth.principal.user_id, id, &request.operation)
                .await;
        }
        return Err(creation_error(response).await);
    }
    Ok(())
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
    apply_overrides(
        state,
        destination,
        overrides,
        matches!(auth.caller, crate::prompt_origin::RequestCaller::Person),
    )?;
    state.hub.place_copy(id, destination, actor)
}

/// The target a duplicate would create, without creating it.
#[derive(Debug, Deserialize)]
pub struct DuplicateOptionsRequest {
    pub provider: String,
    pub provider_account_id: crate::provider_account::ProviderAccountId,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub runtime_machine_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default = "inherit_by_default")]
    pub inherit_settings: bool,
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn source_execution_machine(source: &SessionMeta) -> Result<String, String> {
    let Some(binding) = &source.execution_binding else {
        return Ok(source.machine_id.clone());
    };
    let binding = binding
        .decode()
        .map_err(|_| "Wait until this session's workspace is ready before forking it")?;
    if binding.managed.is_some() {
        return Err("A managed call cannot be forked".into());
    }
    Ok(binding.environment.machine_id)
}

fn duplicate_options_placement(
    source: &SessionMeta,
    machine_id: Option<&str>,
    runtime_machine_id: Option<&str>,
    cwd: Option<&str>,
) -> Result<(String, Option<String>, String), String> {
    if source
        .execution_binding
        .as_ref()
        .and_then(|binding| binding.decode().ok())
        .is_some_and(|binding| binding.managed.is_some())
    {
        return Err("A managed call cannot be forked".into());
    }
    let req_machine = non_empty(machine_id);
    let req_runtime = non_empty(runtime_machine_id);
    let req_cwd = non_empty(cwd);
    if req_machine.is_none() && req_runtime.is_none() && req_cwd.is_none() {
        return placement(source);
    }
    let project = req_cwd
        .or_else(|| non_empty(source.workspace_id.as_deref()))
        .ok_or("This session has no project to fork")?;
    let (machine_id, runtime_machine_id) = match (req_machine, req_runtime) {
        (Some(machine), runtime) => (machine, runtime),
        (None, Some(runtime)) => (source_execution_machine(source)?, Some(runtime)),
        (None, None) => {
            let machine = source_execution_machine(source)?;
            let runtime = source
                .execution_binding
                .as_ref()
                .map(|_| source.machine_id.clone());
            (machine, runtime)
        }
    };
    Ok((machine_id, runtime_machine_id, project))
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
        let (machine_id, runtime_machine_id, project) = duplicate_options_placement(
            &source,
            request.machine_id.as_deref(),
            request.runtime_machine_id.as_deref(),
            request.cwd.as_deref(),
        )?;
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
        Ok(Err(error)) if error.unsupported_fallback => (
            StatusCode::CONFLICT,
            [("x-cowboy-fork-fallback", "unsupported")],
            Json(serde_json::json!({
                "error": error.message,
                "code": "native_fork_unsupported",
            })),
        )
            .into_response(),
        Ok(Err(error)) => failure(error.message),
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
            valid_overrides(
                &config(serde_json::json!({"model": "opus", "fast": true})),
                true
            )
            .is_ok()
        );
        assert!(
            valid_overrides(&config(serde_json::json!({"model": {"nested": 1}})), true).is_err()
        );
        assert!(valid_overrides(&config(serde_json::json!({"": "opus"})), true).is_err());
        let many = (0..33)
            .map(|index| (format!("o{index}"), serde_json::json!("x")))
            .collect();
        assert!(valid_overrides(&many, true).is_err());
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
    fn duplicate_options_placement_prefers_selected_target_machines() {
        let hub = Hub::new();
        let mut meta = source(&hub).meta;
        meta.machine_id = "hawk".into();
        // Omitting target machines keeps the source placement.
        assert_eq!(
            duplicate_options_placement(&meta, None, None, None),
            Ok(("hawk".to_owned(), None, "columbus".to_owned()))
        );
        assert_eq!(
            duplicate_options_placement(&meta, Some(" "), Some(""), Some(" ")),
            Ok(("hawk".to_owned(), None, "columbus".to_owned()))
        );
        // Switching from Hawk local to OVH split execution resolves against OVH.
        assert_eq!(
            duplicate_options_placement(&meta, Some("hawk"), Some("ovh"), Some("columbus")),
            Ok((
                "hawk".to_owned(),
                Some("ovh".to_owned()),
                "columbus".to_owned()
            ))
        );
        // Switching from OVH split execution back to Hawk local clears the source runtime machine.
        meta.machine_id = "ovh".into();
        meta.execution_binding = Some(crate::execution_environment::fixture());
        assert_eq!(
            duplicate_options_placement(&meta, None, None, None),
            Ok((
                "hawk".to_owned(),
                Some("ovh".to_owned()),
                "columbus".to_owned()
            ))
        );
        assert_eq!(
            duplicate_options_placement(&meta, Some("hawk"), None, None),
            Ok(("hawk".to_owned(), None, "columbus".to_owned()))
        );
        assert_eq!(
            duplicate_options_placement(&meta, None, Some("falcon"), None),
            Ok((
                "hawk".to_owned(),
                Some("falcon".to_owned()),
                "columbus".to_owned()
            ))
        );
        // Replacing a session with no registered project previews options once
        // the caller supplies a destination project and Machine.
        meta.workspace_id = None;
        meta.execution_binding = None;
        assert!(duplicate_options_placement(&meta, None, None, None).is_err());
        assert_eq!(
            duplicate_options_placement(&meta, Some("hawk"), Some("ovh"), Some("columbus")),
            Ok((
                "hawk".to_owned(),
                Some("ovh".to_owned()),
                "columbus".to_owned()
            ))
        );
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
                    before_seq: None,
                    source_context_generation: None,
                    config: None,
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
