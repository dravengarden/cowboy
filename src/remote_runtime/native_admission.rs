//! Fresh native-account admission at runtime transport boundaries.
use super::*;

/// Events carry no launch identity. Only the currently admitted worker's
/// snapshot can supply it; an epoch from a replaced worker cannot borrow it.
pub(super) async fn event(
    shared: &Shared,
    session_id: &str,
    worker_epoch: &str,
    project: impl FnOnce(),
) -> bool {
    let worker = shared.workers.lock().get(session_id).cloned();
    let named = shared
        .hub
        .session_info(session_id)
        .is_some_and(|info| !info.meta.provider_account_id.is_default())
        || worker
            .as_ref()
            .and_then(|worker| worker.launch.as_ref())
            .is_some_and(|launch| !launch.provider_account_id.is_default());
    if !named {
        project();
        return true;
    }
    let Some(worker) = worker else {
        return false;
    };
    if worker.worker_epoch != worker_epoch {
        return false;
    }
    let projected = AtomicBool::new(false);
    snapshot(shared, &worker, || {
        // Recheck after the asynchronous DB admission. Do not hold the cache
        // mutex across projection: projection itself updates that cache.
        if shared
            .workers
            .lock()
            .get(session_id)
            .is_some_and(|current| {
                current.worker_epoch == worker_epoch && current.launch == worker.launch
            })
        {
            project();
            projected.store(true, Ordering::Release);
        }
    })
    .await;
    projected.load(Ordering::Acquire)
}

/// Project a snapshot only while its current native binding is admitted.
/// The callback includes cache/declaration updates, not just Hub projection.
pub(super) async fn snapshot(
    shared: &Shared,
    worker: &WorkerSnapshot,
    project: impl FnOnce(),
) -> bool {
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
        project();
        return true;
    }
    let (Some(meta), Some(launch)) = (meta, worker.launch.as_ref()) else {
        return false;
    };
    if !matches(&meta, launch) || meta.provider_account_id.is_default() {
        return false;
    }
    let Some(owner) = meta.owner_user_id.as_deref() else {
        return false;
    };
    let Some((machine, store)) = shared.native_store.lock().clone() else {
        return false;
    };
    if meta.machine_id != machine {
        return false;
    }
    let Ok(Some((operation, completion))) = store
        .native_login_for_registered_session(owner, &meta.id)
        .await
    else {
        return false;
    };
    let profile = &completion.profile;
    if !shared.auth_recipient_key.as_ref().is_some_and(|key| {
        crate::admin::hex_sha256(key.as_bytes()) == completion.recipient_fingerprint
    }) || meta.provider != profile.provider_id
        // A compatible later release keeps the login (see native_generation);
        // the Machine re-verifies the authentication contract at launch.
        || meta.provider_auth_generation != Some(profile.generation)
    {
        return false;
    }
    store
        .with_native_session_release_binding(owner, &operation, &completion, &meta, || async {
            anyhow::ensure!(
                !shared.shutdown.load(Ordering::Acquire),
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
        .is_ok()
}

fn launch(command: &CoreCommand) -> Option<&StartSession> {
    match command {
        CoreCommand::EnsureSession { session } => Some(session),
        CoreCommand::RecoverExecutionSession { session, .. }
        | CoreCommand::ReadmitAccountSession { session, .. } => Some(session),
        _ => None,
    }
}

fn matches(meta: &crate::core::SessionMeta, session: &StartSession) -> bool {
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

fn unchanged(before: &crate::core::SessionMeta, current: &crate::core::SessionMeta) -> bool {
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

/// False is a pre-write denial. Any failure after starting a write closes the
/// transport: partially written frames must never be treated as safe rejection.
pub(super) async fn send<W: tokio::io::AsyncWrite + Unpin>(
    shared: &Shared,
    writer: &mut W,
    command: CoreCommand,
) -> Result<bool> {
    if matches!(
        command,
        CoreCommand::StopSession { .. }
            | CoreCommand::Cancel { .. }
            | CoreCommand::QuiesceAccountSession { .. }
    ) {
        write_broker_frame(writer, &Frame::CoreCommand { command }).await?;
        return Ok(true);
    }
    let meta = command
        .session_id()
        .and_then(|id| shared.hub.session_info(id))
        .map(|info| info.meta);
    if command.session_id().is_some() && meta.is_none() {
        return Ok(false);
    }
    let named = launch(&command).is_some_and(|session| !session.provider_account_id.is_default())
        || meta
            .as_ref()
            .is_some_and(|meta| !meta.provider_account_id.is_default());
    if !named {
        write_broker_frame(writer, &Frame::CoreCommand { command }).await?;
        return Ok(true);
    }
    let Some(meta) = meta else {
        return Ok(false);
    };
    if launch(&command).is_some_and(|session| !matches(&meta, session)) {
        return Ok(false);
    }
    let Some(owner) = meta.owner_user_id.as_deref() else {
        return Ok(false);
    };
    let Some((machine, store)) = shared.native_store.lock().clone() else {
        return Ok(false);
    };
    if meta.machine_id != machine || meta.provider_account_id.is_default() {
        return Ok(false);
    }
    let selection = store
        .native_login_for_registered_session(owner, &meta.id)
        .await;
    let Ok(Some((operation, completion))) = selection else {
        return Ok(false);
    };
    if !shared.auth_recipient_key.as_ref().is_some_and(|key| {
        crate::admin::hex_sha256(key.as_bytes()) == completion.recipient_fingerprint
    }) {
        return Ok(false);
    }
    let profile = &completion.profile;
    if let CoreCommand::ReadmitAccountSession { binding, .. } = &command
        && binding.target != *profile
    {
        return Ok(false);
    }
    if meta.provider != profile.provider_id
        || meta.provider_auth_generation != Some(profile.generation)
    {
        return Ok(false);
    }
    // A Provider reload persists the session's new release through the
    // asynchronous store writer, and its launch can reach admission first:
    // the stored binding then still names the previous release. Give the
    // writer a moment instead of refusing the launch and stranding the
    // session in Starting.
    for attempt in 1..=BINDING_ATTEMPTS {
        let started = AtomicBool::new(false);
        let command = command.clone();
        let result = store
            .with_native_session_release_binding(owner, &operation, &completion, &meta, || async {
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
                anyhow::ensure!(unchanged(&meta, &current), "native_session_binding_changed");
                started.store(true, Ordering::Release);
                write_broker_frame(writer, &Frame::CoreCommand { command }).await
            })
            .await;
        match result {
            Ok(()) => return Ok(true),
            Err(error) if started.load(Ordering::Acquire) => return Err(error),
            Err(error)
                if attempt < BINDING_ATTEMPTS
                    && error
                        .chain()
                        .any(|cause| cause.to_string() == "native_session_binding_changed") =>
            {
                tokio::time::sleep(BINDING_RETRY).await;
            }
            Err(_) => return Ok(false),
        }
    }
    Ok(false)
}

/// How long admission waits for a just-reloaded session's stored binding.
const BINDING_ATTEMPTS: u32 = 40;
const BINDING_RETRY: std::time::Duration = std::time::Duration::from_millis(50);
