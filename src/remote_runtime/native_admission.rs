//! Fresh named-account admission at runtime transport boundaries: the
//! account's own Service vault, or (for a session created before its account
//! migrated) the native login binding its generation names. Neither path
//! ever falls back to the default account.
use super::*;

/// How a persisted named session is bound. Vault generations are issued
/// above every native generation of the account, so a session names a
/// native login only while it was registered (and not yet carried) on one.
enum Binding {
    Native(
        String,
        Box<crate::store::provider_accounts::logins::NativeCompletion>,
    ),
    Vault,
}

async fn binding(
    store: &crate::store::Store,
    owner: &str,
    meta: &crate::model::SessionMeta,
) -> Result<Binding, EventAdmission> {
    match store
        .native_login_for_registered_session(owner, &meta.id)
        .await
    {
        Ok(Some((operation, completion))) => Ok(Binding::Native(operation, Box::new(completion))),
        Ok(None) => Ok(Binding::Vault),
        Err(error) => Err(classify_admission_error(&error)),
    }
}

/// A Service-held lineage advances on every key change and the Machine
/// re-projects the newest key into older generation homes, so an older
/// session generation stays admitted like the default account's.
fn vault_admits(
    status: &crate::provider_service::ProviderAuthenticationStatus,
    generation: u64,
) -> bool {
    generation > 0
        && status.recipient_key.is_none()
        && status.authentication_state == crate::provider_service::ServiceAuthenticationState::Ready
        && status.auth_generation >= generation
}

/// The account vault holds a Ready key for this session at its generation or
/// newer; returns the vault and the key's credential scope.
fn portable_vault(
    shared: &Shared,
    meta: &crate::model::SessionMeta,
) -> Option<(Arc<crate::provider_service::ProviderAuthService>, String)> {
    let scope = shared
        .provider_auth
        .as_ref()?
        .existing_account(&meta.provider_account_id)?;
    let status = scope.status(&meta.provider)?;
    meta.provider_auth_generation
        .is_some_and(|generation| vault_admits(&status, generation))
        .then_some((scope, status.authentication_scope))
}

/// Run `action` under fresh portable admission: the owner's account lock
/// holds the exact persisted session row, the account is available and
/// serves the session Provider (its own, or one sharing its key in the same
/// vault), and the vault key is rechecked inside the lock. Disabling,
/// revocation and key commits take that lock, so none completes during a
/// bounded write or projection. Never consults the default account.
async fn with_portable<T: Send, F: std::future::Future<Output = Result<T>> + Send>(
    shared: &Shared,
    store: &crate::store::Store,
    owner: &str,
    meta: &crate::model::SessionMeta,
    action: impl FnOnce() -> F + Send,
) -> Result<T> {
    let (scope, credential) = portable_vault(shared, meta).context("portable_key_unavailable")?;
    let serves = |account: &crate::store::provider_accounts::Account| {
        account.provider_id == meta.provider
            || scope
                .status(&account.provider_id)
                .is_some_and(|own| own.authentication_scope == credential)
    };
    store
        .with_portable_account(
            owner,
            &meta.provider_account_id,
            crate::store::provider_accounts::portable::Use::Existing(meta),
            serves,
            || async {
                anyhow::ensure!(
                    portable_vault(shared, meta).is_some(),
                    "portable_key_unavailable"
                );
                action().await
            },
        )
        .await
}

/// The Service scope that owns a session's projected credentials, for
/// recording a rejection: the root, a portable account, or a native grant.
pub(super) fn credential_scope(
    shared: &Shared,
    provider_auth: &Arc<crate::provider_service::ProviderAuthService>,
    account: &crate::provider_account::ProviderAccountId,
    provider: &str,
) -> Result<Arc<crate::provider_service::ProviderAuthService>> {
    if account.is_default() {
        return Ok(Arc::clone(provider_auth));
    }
    if let Some(scope) = provider_auth
        .existing_account(account)
        .filter(|scope| scope.status(provider).is_some())
    {
        return Ok(scope);
    }
    shared
        .auth_recipient_key
        .as_deref()
        .context("runtime has no enrolled credential recipient")
        .and_then(|key| provider_auth.existing_machine_grant(account, key))
}

/// Outcome of fresh admission for a streamed worker event (R8-H1, R8-M2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EventAdmission {
    Admitted,
    /// Retryable failure (projection fence, shutdown, or transient store read).
    TransientlyDeferred,
    /// Authoritative refusal (revoked/disabled account, mismatched launch/binding).
    PermanentlyDenied,
}

impl EventAdmission {
    pub(super) fn is_admitted(self) -> bool {
        self == Self::Admitted
    }
}

/// Events carry no launch identity. Only the currently admitted worker's
/// snapshot can supply it; an epoch from a replaced worker cannot borrow it.
#[allow(clippy::too_many_lines)]
pub(super) async fn event(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    project: impl FnOnce() + Send,
) -> EventAdmission {
    let cached = shared.workers.lock().get(session_id).cloned();
    let mut from_deferred = false;
    let replay_snapshot = cached
        .as_ref()
        .filter(|worker| worker.worker_epoch == worker_epoch)
        .cloned()
        .or_else(|| {
            shared
                .draining_replay_snapshots
                .lock()
                .get(&(session_id.to_owned(), worker_epoch.to_owned()))
                .cloned()
        })
        .or_else(|| {
            let deferred = shared
                .deferred_admission_snapshots
                .lock()
                .get(&(session_id.to_owned(), worker_epoch.to_owned()))
                .cloned();
            if deferred.is_some() {
                from_deferred = true;
            }
            deferred
        });
    let named = shared
        .hub
        .session_info(session_id)
        .is_some_and(|info| !info.meta.provider_account_id.is_default())
        || replay_snapshot
            .as_ref()
            .or(cached.as_ref())
            .and_then(|worker| worker.launch.as_ref())
            .is_some_and(|launch| !launch.provider_account_id.is_default());
    let execution_bound = shared
        .hub
        .session_info(session_id)
        .is_some_and(|info| info.meta.execution_binding.is_some());
    if !named {
        let fenced = shared.projection_fence.lock();
        if *fenced || shared.shutdown.load(Ordering::Acquire) {
            return EventAdmission::TransientlyDeferred;
        }
        if execution_bound {
            let Some(current) = shared
                .workers
                .lock()
                .get(session_id)
                .filter(|worker| worker.worker_epoch == worker_epoch)
                .cloned()
                .or_else(|| {
                    shared
                        .draining_replay_snapshots
                        .lock()
                        .get(&(session_id.to_owned(), worker_epoch.to_owned()))
                        .cloned()
                })
            else {
                return EventAdmission::PermanentlyDenied;
            };
            if current.worker_epoch != worker_epoch
                || !shared.hub.matches_runtime_snapshot_binding(&current)
            {
                return EventAdmission::PermanentlyDenied;
            }
        }
        project();
        return EventAdmission::Admitted;
    }
    let Some(worker) = replay_snapshot else {
        return EventAdmission::PermanentlyDenied;
    };
    if worker.worker_epoch != worker_epoch {
        return EventAdmission::PermanentlyDenied;
    }
    let projected = AtomicBool::new(false);
    let outcome = snapshot_outcome(shared, &worker, || {
        let matches_cached = shared
            .workers
            .lock()
            .get(session_id)
            .is_some_and(|current| {
                current.worker_epoch == worker_epoch && current.launch == worker.launch
            });
        let matches_draining_replay = !matches_cached
            && shared
                .draining_replay_snapshots
                .lock()
                .get(&(session_id.to_owned(), worker_epoch.to_owned()))
                .is_some_and(|current| {
                    current.worker_epoch == worker_epoch && current.launch == worker.launch
                });
        let promoted_deferred = if !matches_cached && !matches_draining_replay && from_deferred {
            let still_deferred = shared
                .deferred_admission_snapshots
                .lock()
                .remove(&(session_id.to_owned(), worker_epoch.to_owned()))
                .is_some_and(|current| {
                    current.worker_epoch == worker_epoch && current.launch == worker.launch
                });
            if still_deferred {
                if !shared.resetting.lock().contains(session_id) {
                    let _ = super::project_snapshot(shared, &worker);
                }
                true
            } else {
                false
            }
        } else {
            false
        };
        if matches_cached || matches_draining_replay || promoted_deferred {
            project();
            projected.store(true, Ordering::Release);
        }
    })
    .await;
    if outcome == EventAdmission::PermanentlyDenied && from_deferred {
        shared
            .deferred_admission_snapshots
            .lock()
            .remove(&(session_id.to_owned(), worker_epoch.to_owned()));
    }
    if projected.load(Ordering::Acquire) {
        EventAdmission::Admitted
    } else if outcome == EventAdmission::Admitted {
        EventAdmission::PermanentlyDenied
    } else {
        outcome
    }
}

/// Project a snapshot only while its current native binding is admitted.
/// The callback includes cache/declaration updates, not just Hub projection.
/// Retains transiently deferred connected snapshots so subsequent replayed
/// events retry full admission instead of dropping events as `PermanentlyDenied`
/// (R18-H1).
pub(super) async fn snapshot(
    shared: &Shared,
    worker: &WorkerSnapshot,
    project: impl FnOnce() + Send,
) -> bool {
    let outcome = snapshot_outcome(shared, worker, project).await;
    let key = (worker.session_id.clone(), worker.worker_epoch.clone());
    match outcome {
        EventAdmission::TransientlyDeferred if worker.has_connected_owner() => {
            let mut deferred = shared.deferred_admission_snapshots.lock();
            match deferred.get_mut(&key) {
                Some(existing) if existing.last_runtime_seq > worker.last_runtime_seq => {
                    if let Some(effect) = worker.uncertain_effect.clone() {
                        crate::runtime_wire::merge_uncertain_effect(
                            &mut existing.uncertain_effect,
                            effect,
                        );
                    }
                }
                Some(existing) => *existing = worker.clone(),
                None => {
                    deferred.insert(key, worker.clone());
                }
            }
        }
        EventAdmission::Admitted | EventAdmission::PermanentlyDenied => {
            shared.deferred_admission_snapshots.lock().remove(&key);
        }
        EventAdmission::TransientlyDeferred => {}
    }
    outcome.is_admitted()
}

#[allow(clippy::too_many_lines)]
async fn snapshot_outcome(
    shared: &Shared,
    worker: &WorkerSnapshot,
    project: impl FnOnce() + Send,
) -> EventAdmission {
    let meta = shared
        .hub
        .session_info(&worker.session_id)
        .map(|info| info.meta);
    let named = meta
        .as_ref()
        .is_some_and(|meta| !meta.provider_account_id.is_default())
        || worker
            .launch
            .as_ref()
            .is_some_and(|launch| !launch.provider_account_id.is_default());
    if !named {
        let fenced = shared.projection_fence.lock();
        if *fenced || shared.shutdown.load(Ordering::Acquire) {
            return EventAdmission::TransientlyDeferred;
        }
        project();
        return EventAdmission::Admitted;
    }
    let (Some(meta), Some(launch)) = (meta, worker.launch.as_ref()) else {
        return EventAdmission::PermanentlyDenied;
    };
    if !matches(&meta, launch) || meta.provider_account_id.is_default() {
        warn_declined_launch(&meta, worker);
        return EventAdmission::PermanentlyDenied;
    }
    let Some(owner) = meta.owner_user_id.as_deref() else {
        return EventAdmission::PermanentlyDenied;
    };
    let Some((machine, store)) = shared.native_store.lock().clone() else {
        return EventAdmission::TransientlyDeferred;
    };
    if meta.machine_id != machine {
        return EventAdmission::PermanentlyDenied;
    }
    let selection = match binding(&store, owner, &meta).await {
        Ok(selection) => selection,
        Err(denial) => return denial,
    };
    let Binding::Native(operation, completion) = selection else {
        return match with_portable(shared, &store, owner, &meta, || async {
            let fenced = shared.projection_fence.lock();
            anyhow::ensure!(
                !*fenced && !shared.shutdown.load(Ordering::Acquire),
                "runtime_disconnected"
            );
            anyhow::ensure!(
                shared.hub.session_info(&meta.id).is_some_and(|info| {
                    !info.meta.closing
                        && info.meta.owner_user_id == meta.owner_user_id
                        && info.meta.machine_id == machine
                        && matches(&info.meta, launch)
                }),
                "portable_session_binding_changed"
            );
            project();
            Ok(())
        })
        .await
        {
            Ok(()) => EventAdmission::Admitted,
            Err(error) => classify_admission_error(&error),
        };
    };
    let profile = &completion.profile;
    if !shared.auth_recipient_key.as_ref().is_some_and(|key| {
        crate::digest::hex_sha256(key.as_bytes()) == completion.recipient_fingerprint
    }) || meta.provider != profile.provider_id
        || meta.provider_auth_generation != Some(profile.generation)
    {
        return EventAdmission::PermanentlyDenied;
    }
    match store
        .with_native_session_release_binding(owner, &operation, &completion, &meta, || async {
            let fenced = shared.projection_fence.lock();
            anyhow::ensure!(
                !*fenced && !shared.shutdown.load(Ordering::Acquire),
                "runtime_disconnected"
            );
            let current = shared
                .hub
                .session_info(&meta.id)
                .context("session_unavailable")?
                .meta;
            anyhow::ensure!(
                !current.closing
                    && current.owner_user_id == meta.owner_user_id
                    && current.machine_id == machine
                    && matches(&current, launch),
                "native_session_binding_changed"
            );
            project();
            Ok(())
        })
        .await
    {
        Ok(()) => EventAdmission::Admitted,
        Err(error) => classify_admission_error(&error),
    }
}

/// Classify store/account admission errors: only explicit policy denials are
/// `PermanentlyDenied`; SQLite/store transaction or I/O errors and disconnects
/// remain retryable `TransientlyDeferred` (R9-M2).
fn classify_admission_error(error: &anyhow::Error) -> EventAdmission {
    const PERMANENT_CODES: &[&str] = &[
        "portable_key_unavailable",
        "named_account_required",
        "account_unavailable",
        "portable_credentials_unmarked",
        "native_identity_mismatch",
        "portable_session_scope_mismatch",
        "portable_session_binding_changed",
        "login_not_admitted",
        "native_identity_not_bound",
        "invalid_native_generation",
        "native_generation_ambiguous",
        "native_completion_scope_mismatch",
        "native_session_scope_mismatch",
        "native_binding_scope_mismatch",
        "native_login_scope_mismatch",
        "native_transition_scope_mismatch",
        "native_logout_scope_mismatch",
        "portable_revocation_scope_mismatch",
        "portable_wipe_scope_mismatch",
        "native_generation_superseded",
        "machine_unavailable",
        "machine_enrollment_changed",
        "native_binding_changed",
        "native_session_binding_changed",
        "session_unavailable",
    ];
    let message = error.to_string();
    if PERMANENT_CODES.iter().any(|code| message.contains(code)) {
        EventAdmission::PermanentlyDenied
    } else {
        EventAdmission::TransientlyDeferred
    }
}

/// The last worker epoch whose mismatched launch was reported, per session.
static DECLINED: std::sync::LazyLock<Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(Default::default);

/// A declined snapshot leaves the worker unknown to this Controller, so its
/// execution requests are refused as retryable until startup times out. Say
/// so once per worker epoch instead of failing silently for minutes.
fn warn_declined_launch(meta: &crate::model::SessionMeta, worker: &WorkerSnapshot) {
    // A running worker awaiting its reload stop is expected to lag the new
    // declaration; only a starting one waits on admission it cannot get.
    let (WorkerState::Starting, Some(launch)) = (worker.state, worker.launch.as_ref()) else {
        return;
    };
    let mut declined = DECLINED.lock();
    if declined.get(&meta.id) == Some(&worker.worker_epoch) {
        return;
    }
    declined.insert(meta.id.clone(), worker.worker_epoch.clone());
    tracing::warn!(
        session = %meta.id,
        worker_epoch = %worker.worker_epoch,
        declared_version = %meta.provider_version,
        launch_version = %launch.provider_version,
        declared_auth_generation = ?meta.provider_auth_generation,
        launch_auth_generation = ?launch.provider_auth_generation,
        "starting named worker does not match the session declaration; worker is not admitted"
    );
}

fn launch(command: &CoreCommand) -> Option<&StartSession> {
    match command {
        CoreCommand::EnsureSession { session } => Some(session),
        CoreCommand::RecoverExecutionSession { session, .. }
        | CoreCommand::ReadmitAccountSession { session, .. } => Some(session),
        _ => None,
    }
}

fn matches(meta: &crate::model::SessionMeta, session: &StartSession) -> bool {
    meta.id == session.session_id
        && meta.provider == session.provider
        && meta.provider_account_id == session.provider_account_id
        && meta.provider_version == session.provider_version
        && meta.provider_generation_digest == session.provider_generation_digest
        && meta.provider_auth_generation == session.provider_auth_generation
        && meta.cwd == session.cwd
        && meta.execution_binding == session.execution_binding
        && meta.system == session.system
        && meta.provider_behavior == session.provider_behavior
}

fn unchanged(before: &crate::model::SessionMeta, current: &crate::model::SessionMeta) -> bool {
    current.owner_user_id == before.owner_user_id
        && current.machine_id == before.machine_id
        && current.provider_account_id == before.provider_account_id
        && current.provider == before.provider
        && current.provider_version == before.provider_version
        && current.provider_generation_digest == before.provider_generation_digest
        && current.provider_auth_generation == before.provider_auth_generation
        && current.cwd == before.cwd
        && current.execution_binding == before.execution_binding
        && current.system == before.system
        && current.provider_behavior == before.provider_behavior
}

// Re-evaluate after queueing and after native-account awaits; a prepared launch
// is not permission to ignore a policy configured while it was waiting.
fn mcp_launch_current(hub: &crate::core::Hub, command: &CoreCommand) -> bool {
    let Some(launch) = launch(command) else {
        return true;
    };
    hub.session_info(&launch.session_id).is_some_and(|info| {
        crate::controller_hooks::launch_mcp(hub, &info.meta)
            .is_ok_and(|plan| plan == launch.mcp_plan)
    })
}

/// A Controller restart re-declares surviving workers before its in-memory MCP
/// launch owners are recovered. That declaration is merely deferred: the
/// Machine keeps its own, and recovery rebuilds the same content-addressed plan.
/// A worker that launched with an MCP plan the session has since replaced (a
/// policy edit or Plugin release) is deferred the same way: adoption can never
/// move its plan, and the first send after its turn replaces it.
pub(super) fn declaration_denied(shared: &Shared, session: &StartSession) {
    let mcp_deferred = !mcp_launch_current(
        &shared.hub,
        &CoreCommand::EnsureSession {
            session: session.clone(),
        },
    );
    if mcp_deferred {
        tracing::info!(session = %session.session_id, "worker declaration deferred until its MCP launch owner recovers or the worker is replaced");
        return;
    }
    shared.hub.broadcast_error(
        Some(session.session_id.clone()),
        "Worker recovery admission denied".into(),
    );
}

/// Outcome of attempting to admit and write one queued `CoreCommand`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SendOutcome {
    /// Command passed admission, was marked in `attempted_sends`, and was written.
    Sent,
    /// Pre-write admission denied the command (caller should invoke `reset_admission::denied`).
    Denied,
    /// Command was removed, fenced, or disconnected before `write_broker_frame`
    /// began (`attempted_sends` was NOT marked; skip without denying).
    Skipped,
}

/// Atomically recheck shutdown, membership in `pending`, `fenced_commands`,
/// and prompt send fences right before `write_broker_frame`, then mark
/// `attempted_sends` with zero awaits between marking and writing (Round 2 #5).
/// Lock order: `config_startups` -> `recovery_fences` -> `pending` ->
/// `attempted_sends` -> `fenced_commands`.
fn reserve_for_write(
    shared: &Shared,
    expected: Option<(&str, u64)>,
    command: &CoreCommand,
) -> bool {
    let startups = shared.config_startups.lock();
    let recovery_fences = shared.recovery_fences.lock();
    let pending = shared.pending.lock();
    if shared.shutdown.load(Ordering::Acquire) {
        return false;
    }
    if let Some((key, version)) = expected {
        if pending.version(key) != Some(version)
            || shared.fenced_commands.lock().contains(key)
            || matches!(command, CoreCommand::Prompt { session_id, .. }
                if startups.contains(session_id) || recovery_fences.contains_key(session_id))
        {
            return false;
        }
        if !shared.attempted_sends.lock().insert(key.to_owned()) {
            shared.retransmitted_sends.lock().insert(key.to_owned());
        }
    }
    true
}

/// `Denied` is a pre-write denial; `Skipped` means the command became unfitted
/// (disconnected/fenced/removed/superseded) before `write_broker_frame` began.
/// Any failure after starting a write closes the transport: partially written
/// frames must never be treated as safe rejection.
#[allow(clippy::too_many_lines)]
pub(super) async fn send<W: tokio::io::AsyncWrite + Unpin + Send>(
    shared: &Shared,
    writer: &mut W,
    expected: Option<(&str, u64)>,
    command: CoreCommand,
) -> Result<SendOutcome> {
    if matches!(command, CoreCommand::DrainCrashedWorker { .. })
        && (shared.machine_protocol
            < crate::machine_protocol::CRASHED_WORKER_DRAIN_PROTOCOL_VERSION
            || shared.protocol.load(Ordering::Acquire)
                < crate::runtime_wire::CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION)
    {
        return Ok(SendOutcome::Denied);
    }
    if matches!(
        command,
        CoreCommand::StopSession { .. }
            | CoreCommand::Cancel { .. }
            | CoreCommand::CancelSubagent { .. }
            | CoreCommand::QuiesceAccountSession { .. }
            | CoreCommand::DrainCrashedWorker { .. }
    ) {
        if !reserve_for_write(shared, expected, &command) {
            return Ok(SendOutcome::Skipped);
        }
        write_broker_frame(writer, &Frame::CoreCommand { command }).await?;
        return Ok(SendOutcome::Sent);
    }
    let meta = command
        .session_id()
        .and_then(|id| shared.hub.session_info(id))
        .map(|info| info.meta);
    if command.session_id().is_some() && meta.is_none() {
        return Ok(SendOutcome::Denied);
    }
    if !mcp_launch_current(&shared.hub, &command) {
        return Ok(SendOutcome::Denied);
    }
    let named = launch(&command).is_some_and(|session| !session.provider_account_id.is_default())
        || meta
            .as_ref()
            .is_some_and(|meta| !meta.provider_account_id.is_default());
    if !named {
        if !reserve_for_write(shared, expected, &command) {
            return Ok(SendOutcome::Skipped);
        }
        write_broker_frame(writer, &Frame::CoreCommand { command }).await?;
        return Ok(SendOutcome::Sent);
    }
    let Some(meta) = meta else {
        return Ok(SendOutcome::Denied);
    };
    if launch(&command).is_some_and(|session| !matches(&meta, session)) {
        return Ok(SendOutcome::Denied);
    }
    let Some(owner) = meta.owner_user_id.as_deref() else {
        return Ok(SendOutcome::Denied);
    };
    let Some((machine, store)) = shared.native_store.lock().clone() else {
        return Ok(SendOutcome::Denied);
    };
    if meta.machine_id != machine || meta.provider_account_id.is_default() {
        return Ok(SendOutcome::Denied);
    }
    let Ok(selection) = binding(&store, owner, &meta).await else {
        return Ok(SendOutcome::Denied);
    };
    let Binding::Native(operation, completion) = selection else {
        return send_portable(shared, writer, &store, owner, &meta, expected, command).await;
    };
    if !shared.auth_recipient_key.as_ref().is_some_and(|key| {
        crate::digest::hex_sha256(key.as_bytes()) == completion.recipient_fingerprint
    }) {
        return Ok(SendOutcome::Denied);
    }
    let profile = &completion.profile;
    if let CoreCommand::ReadmitAccountSession { binding, .. } = &command
        && binding.target != *profile
    {
        return Ok(SendOutcome::Denied);
    }
    if meta.provider != profile.provider_id
        || meta.provider_auth_generation != Some(profile.generation)
    {
        return Ok(SendOutcome::Denied);
    }
    // A Provider reload persists the session's new release through the
    // asynchronous store writer, and its launch can reach admission first:
    // the stored binding then still names the previous release. Give the
    // writer a moment instead of refusing the launch and stranding the
    // session in Starting.
    for attempt in 1..=BINDING_ATTEMPTS {
        let started = AtomicBool::new(false);
        let skipped = AtomicBool::new(false);
        let command = command.clone();
        let result = store
            .with_native_session_release_binding(owner, &operation, &completion, &meta, || async {
                validate_delivery(shared, &meta, &command)?;
                if !reserve_for_write(shared, expected, &command) {
                    skipped.store(true, Ordering::Release);
                    anyhow::bail!("send_skipped");
                }
                started.store(true, Ordering::Release);
                write_broker_frame(writer, &Frame::CoreCommand { command }).await
            })
            .await;
        match result {
            Ok(()) => return Ok(SendOutcome::Sent),
            Err(_) if skipped.load(Ordering::Acquire) => return Ok(SendOutcome::Skipped),
            Err(error) if started.load(Ordering::Acquire) => return Err(error),
            Err(error)
                if attempt < BINDING_ATTEMPTS
                    && error
                        .chain()
                        .any(|cause| cause.to_string() == "native_session_binding_changed") =>
            {
                tokio::time::sleep(BINDING_RETRY).await;
            }
            Err(_) => return Ok(SendOutcome::Denied),
        }
    }
    Ok(SendOutcome::Denied)
}

async fn send_portable<W: tokio::io::AsyncWrite + Unpin + Send>(
    shared: &Shared,
    writer: &mut W,
    store: &crate::store::Store,
    owner: &str,
    meta: &crate::core::SessionMeta,
    expected: Option<(&str, u64)>,
    command: CoreCommand,
) -> Result<SendOutcome> {
    // Native readmission has no meaning for a Service-held key.
    if matches!(command, CoreCommand::ReadmitAccountSession { .. }) {
        return Ok(SendOutcome::Denied);
    }
    let started = AtomicBool::new(false);
    let skipped = AtomicBool::new(false);
    let result = with_portable(shared, store, owner, meta, || async {
        validate_delivery(shared, meta, &command)?;
        if !reserve_for_write(shared, expected, &command) {
            skipped.store(true, Ordering::Release);
            anyhow::bail!("send_skipped");
        }
        started.store(true, Ordering::Release);
        write_broker_frame(writer, &Frame::CoreCommand { command }).await
    })
    .await;
    match result {
        Ok(()) => Ok(SendOutcome::Sent),
        Err(_) if skipped.load(Ordering::Acquire) => Ok(SendOutcome::Skipped),
        // Any failure after starting a write closes the transport.
        Err(error) if started.load(Ordering::Acquire) => Err(error),
        Err(_) => Ok(SendOutcome::Denied),
    }
}

/// How long admission waits for a just-reloaded session's stored binding.
const BINDING_ATTEMPTS: u32 = 40;
const BINDING_RETRY: std::time::Duration = std::time::Duration::from_millis(50);

fn validate_delivery(
    shared: &Shared,
    meta: &crate::core::SessionMeta,
    command: &CoreCommand,
) -> Result<()> {
    anyhow::ensure!(
        !shared.shutdown.load(Ordering::Acquire),
        "runtime_disconnected"
    );
    let current = shared
        .hub
        .session_info(&meta.id)
        .context("session_unavailable")?
        .meta;
    anyhow::ensure!(!current.closing, "session_closing");
    anyhow::ensure!(unchanged(meta, &current), "native_session_binding_changed");
    anyhow::ensure!(
        mcp_launch_current(&shared.hub, command),
        "mcp_launch_changed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deepseek_packages() -> Vec<cowboy_provider_sdk::ProviderPackage> {
        [
            include_str!("../../plugins/claude-deepseek/provider.json"),
            include_str!("../../plugins/codex-deepseek/provider.json"),
            include_str!("../../plugins/deepseek-harness/provider.json"),
        ]
        .into_iter()
        .map(|source| {
            let source: cowboy_provider_sdk::StandardProviderSource =
                serde_json::from_str(source).unwrap();
            cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
        })
        .collect()
    }

    const OWNER: &str = "0123456789abcdef0123456789abcdef";

    /// A migrated store with the owner user and an enrolled `hawk` Machine.
    async fn fixture_store(root: &std::path::Path) -> crate::store::Store {
        let url = format!("sqlite://{}", root.join("db.sqlite3").display());
        let store = crate::store::Store::connect(&url, root.join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        store
            .insert_user(&crate::store::ProductUser {
                id: OWNER.into(),
                username: "owner".into(),
                password_algo: "fixture".into(),
                password_hash: "fixture".into(),
                created_at_ms: 0,
                updated_at_ms: 0,
                disabled_at_ms: None,
            })
            .await
            .unwrap();
        let machine_key = {
            use base64::Engine as _;
            let public = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([3; 32]));
            base64::engine::general_purpose::STANDARD.encode(public.as_bytes())
        };
        let token = store
            .create_machine_enrollment("hawk", "hawk", 60)
            .await
            .unwrap();
        store
            .consume_machine_enrollment(&token, &format!("ssh-ed25519 {machine_key}"), &machine_key)
            .await
            .unwrap();
        store
    }

    fn meta(
        provider: &str,
        account: &crate::provider_account::ProviderAccountId,
        generation: u64,
    ) -> crate::model::SessionMeta {
        serde_json::from_value(serde_json::json!({
            "id":format!("s-{provider}"), "provider":provider, "machine_id":"hawk",
            "workspace_id":"project", "cwd":"/tmp", "title":"named", "status":"starting",
            "owner_user_id":OWNER, "provider_account_id":account,
            "provider_auth_generation":generation, "provider_version":"1.0.0",
            "provider_generation_digest":"sha256:fixture"
        }))
        .unwrap()
    }

    async fn persist(
        store: &crate::store::Store,
        session: &crate::model::SessionMeta,
    ) -> Result<()> {
        store
            .with_portable_account(
                OWNER,
                &session.provider_account_id,
                crate::store::provider_accounts::portable::Use::Insert(session),
                |_| true,
                || async { Ok(()) },
            )
            .await
    }

    async fn portable_admitted(
        shared: &Shared,
        store: &crate::store::Store,
        owner: &str,
        meta: &crate::model::SessionMeta,
    ) -> bool {
        with_portable(shared, store, owner, meta, || async { Ok(()) })
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn portable_named_sessions_need_their_own_available_ready_account_key() {
        use crate::store::provider_accounts::{Change, Mutation, MutationResult};
        let root = tempfile::tempdir().unwrap();
        let store = fixture_store(root.path()).await;
        let create = |provider: &str| Mutation {
            expected_revision: 0,
            change: Change::Create {
                provider_id: provider.into(),
                label: "DeepSeek".into(),
            },
        };
        let MutationResult::Applied(created) = store
            .mutate_provider_accounts(OWNER, "create", &create("claude-deepseek"))
            .await
            .unwrap()
        else {
            panic!("create");
        };
        let id = created.account_id;
        let MutationResult::Applied(unkeyed) = store
            .mutate_provider_accounts("other", "create", &create("codex-deepseek"))
            .await
            .unwrap()
        else {
            panic!("create");
        };
        let auth = Arc::new(
            crate::provider_service::ProviderAuthService::open(&root.path().join("auth")).unwrap(),
        );
        let packages = deepseek_packages();
        let bundle = crate::machine_protocol::PortableCredentialBundle {
            portable_schema: "deepseek-api-key-v1".into(),
            method_id: "api-key".into(),
            values: std::collections::BTreeMap::from([("api_key".into(), "c2stbmFtZWQ=".into())]),
        };
        // The default account holds a key too; a named session never uses it.
        auth.commit_shared(&packages, &bundle, None, "codex-deepseek", Some(0))
            .unwrap();
        auth.account(&id)
            .unwrap()
            .commit_shared(&packages, &bundle, None, "claude-deepseek", Some(0))
            .unwrap();
        let mut runtime = RemoteRuntime::for_test(crate::core::Hub::new(), vec![]);
        Arc::get_mut(&mut Arc::get_mut(&mut runtime).unwrap().shared)
            .unwrap()
            .provider_auth = Some(Arc::clone(&auth));
        let shared = &runtime.shared;
        let unpersisted = meta("claude-deepseek", &id, 1);
        assert!(!portable_admitted(shared, &store, OWNER, &unpersisted).await);
        // One claude-deepseek account serves every member of its scope.
        for provider in ["claude-deepseek", "codex-deepseek", "deepseek-harness"] {
            let session = meta(provider, &id, 1);
            assert!(matches!(
                binding(&store, OWNER, &session).await,
                Ok(Binding::Vault)
            ));
            persist(&store, &session).await.unwrap();
            assert!(portable_admitted(shared, &store, OWNER, &session).await);
        }
        let session = meta("codex-deepseek", &id, 1);
        assert!(!portable_admitted(shared, &store, "other", &session).await);
        assert!(!portable_admitted(shared, &store, OWNER, &meta("codex-deepseek", &id, 2)).await);
        assert!(!portable_admitted(shared, &store, OWNER, &meta("codex-deepseek", &id, 0)).await);
        // An account without its own key is refused despite the default key.
        let other = meta("codex-deepseek", &unkeyed.account_id, 1);
        assert!(!portable_admitted(shared, &store, "other", &other).await);
        // A subscription session without a native login is vault-bound too:
        // it never falls back to the default account.
        let subscription = meta("claude-code", &id, 1);
        assert!(matches!(
            binding(&store, OWNER, &subscription).await,
            Ok(Binding::Vault)
        ));
        assert!(!portable_admitted(shared, &store, OWNER, &subscription).await);
        // A newer key keeps older sessions admitted, like the Service account.
        auth.account(&id)
            .unwrap()
            .commit_shared(&packages, &bundle, None, "claude-deepseek", Some(1))
            .unwrap();
        assert!(portable_admitted(shared, &store, OWNER, &session).await);
        let disable = Mutation {
            expected_revision: 1,
            change: Change::SetEnabled {
                id: id.clone(),
                enabled: false,
            },
        };
        store
            .mutate_provider_accounts(OWNER, "disable", &disable)
            .await
            .unwrap();
        assert!(!portable_admitted(shared, &store, OWNER, &session).await);
    }

    #[tokio::test]
    async fn an_unrecovered_mcp_owner_defers_the_restart_declaration_silently() {
        let hub = crate::core::Hub::new();
        let meta: crate::core::SessionMeta = serde_json::from_value(serde_json::json!({
            "id":"s", "provider":"codex", "machine_id":"hawk",
            "workspace_id":"project", "cwd":"/tmp", "title":"MCP", "status":"running"
        }))
        .unwrap();
        hub.project_persisted_session(meta);
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![]);
        let declaration: StartSession = serde_json::from_value(serde_json::json!({
            "session_id":"s", "provider":"codex", "cwd":"/tmp", "generation":""
        }))
        .unwrap();
        let mut frames = hub.subscribe();
        let key = format!(
            "{}s",
            crate::core::settings_keys::SESSION_MCP_BINDINGS_PREFIX
        );
        hub.set_setting(key.clone(), serde_json::json!({"bindings":[]}));
        declaration_denied(&runtime.shared, &declaration);
        while let Ok(frame) = frames.try_recv() {
            assert!(
                !matches!(**frame, crate::core::Outbound::Error { .. }),
                "deferred"
            );
        }
        hub.set_setting(key, serde_json::Value::Null);
        declaration_denied(&runtime.shared, &declaration);
        let mut reported = false;
        while let Ok(frame) = frames.try_recv() {
            reported |= matches!(**frame, crate::core::Outbound::Error { .. });
        }
        assert!(reported);
        // A surviving worker whose plan the session has since replaced.
        let stale: StartSession = serde_json::from_value(serde_json::json!({
            "session_id":"s", "provider":"codex", "cwd":"/tmp", "generation":"",
            "mcp_plan":{"schema":1,"command":"/bin/cowboy",
                "bindings":[{"name":"suger","descriptor":"/old.json"}]}
        }))
        .unwrap();
        declaration_denied(&runtime.shared, &stale);
        while let Ok(frame) = frames.try_recv() {
            assert!(
                !matches!(**frame, crate::core::Outbound::Error { .. }),
                "a stale plan is deferred, not reported"
            );
        }
    }

    #[test]
    fn classify_admission_error_marks_ambiguous_and_scope_mismatch_permanent() {
        for code in [
            "native_generation_ambiguous: multiple active generations",
            "native_completion_scope_mismatch: wrong machine",
        ] {
            assert_eq!(
                classify_admission_error(&anyhow::anyhow!("{code}")),
                EventAdmission::PermanentlyDenied,
                "{code} must fail permanently rather than defer"
            );
        }
        assert_eq!(
            classify_admission_error(&anyhow::anyhow!("sqlite database is locked")),
            EventAdmission::TransientlyDeferred
        );
    }

    #[tokio::test]
    async fn binding_lookup_classifies_invalid_native_generation_as_permanent_denial() {
        let dir = tempfile::tempdir().unwrap();
        let store = fixture_store(dir.path()).await;
        let account: crate::provider_account::ProviderAccountId =
            serde_json::from_value(serde_json::json!("00000000-0000-4000-8000-000000000001"))
                .unwrap();
        let session = meta("codex", &account, 0);
        // Insert a registered session row whose provider_auth_generation is 0
        // (non-positive), triggering `invalid_native_generation` in Store lookup.
        store
            .insert_session(&session)
            .await
            .expect("insert session row with zero generation");
        let outcome = binding(&store, OWNER, &session).await;
        assert_eq!(
            outcome.err(),
            Some(EventAdmission::PermanentlyDenied),
            "invalid_native_generation must classify as PermanentlyDenied"
        );
    }
}
