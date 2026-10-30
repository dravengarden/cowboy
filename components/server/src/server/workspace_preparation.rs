//! Machine workspace preparation for Web sessions.
//!
//! Web creation registers a session against the stable source root and asks
//! its Machine to prepare a session-owned worktree in the background. That
//! request is not durable: a Controller restart, a dropped Machine connection
//! or a transient Git timeout can end it before the prepared path is
//! published. Such a session still points at the source checkout, where no
//! worker may start. This module owns the preparation task and re-runs it
//! (idempotently, the Machine reuses an existing session worktree) for an
//! interrupted preparation once the Machine is back, and for a failed one when
//! the user sends, retries or reloads.
use super::*;

/// Wait reason projected while a queued prompt waits for its workspace.
pub(super) const WAIT_REASON: &str = "preparing the workspace";
const RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// Start preparing `session_id`'s workspace unless that is already running.
/// Returns `false` when a preparation for the session was already in flight.
pub(super) fn schedule(
    state: &Arc<AppState>,
    session_id: &str,
    machine_id: &str,
    source_path: &str,
    eager: bool,
) -> bool {
    if !state
        .workspace_preparations
        .lock()
        .insert(session_id.to_owned())
    {
        return false;
    }
    let state = Arc::clone(state);
    let session_id = session_id.to_owned();
    let machine_id = machine_id.to_owned();
    let source_path = source_path.to_owned();
    tokio::spawn(async move {
        let result = prepare(&state, &session_id, &machine_id, &source_path, eager).await;
        state.workspace_preparations.lock().remove(&session_id);
        if let Err(error) = result {
            // A queued prompt stays queued for explicit intervention. Leaving
            // an (explicit) wait would re-dispatch and re-prepare every tick.
            session_maintenance::clear_wait(&state.hub, &session_id);
            state.hub.set_status(
                &session_id,
                crate::agent_model::Status::Crashed,
                Some(error),
            );
        }
    });
    true
}

/// Re-run preparation for a session left on its source checkout by a failed
/// or interrupted preparation. Returns the dispatch wait to retain while the
/// workspace is prepared, or `None` when the session has a runnable cwd.
pub(super) fn resume(state: &Arc<AppState>, session_id: &str) -> Option<String> {
    let meta = state.hub.session_info(session_id)?.meta;
    if !meta.awaits_workspace_preparation() || has_worker(state, &meta) {
        return None;
    }
    let source_path = meta.workspace_source_path.as_deref()?;
    // Preparation completion drains the queue itself.
    state.hub.set_explicit_wait(session_id, false);
    if schedule(
        state,
        session_id,
        &meta.machine_id,
        source_path,
        meta.system,
    ) {
        tracing::info!(
            session = %session_id,
            machine = %meta.machine_id,
            status = ?meta.status,
            "re-running Machine workspace preparation for session"
        );
        state.hub.set_status(
            session_id,
            crate::agent_model::Status::Starting,
            Some("Preparing the workspace".to_owned()),
        );
    }
    Some(format!("{}{WAIT_REASON}", session_maintenance::WAIT_PREFIX))
}

fn has_worker(state: &AppState, meta: &crate::core::SessionMeta) -> bool {
    state
        .runtime_router
        .runtime(&meta.machine_id)
        .is_some_and(|runtime| runtime.has_worker(&meta.id))
}

/// Interrupted preparations (still `Starting`, nothing in flight) resume once
/// their Machine runtime is connected. Failed (`Crashed`) ones wait for an
/// explicit send, Retry or Reload so a persistent failure cannot loop.
pub(super) fn start_recovery(state: &Arc<AppState>) {
    let state = Arc::downgrade(state);
    tokio::spawn(async move {
        // Only a session observed interrupted on two consecutive scans is
        // resumed, so a launch that has not registered its worker yet (a
        // shared non-Git root) is never mistaken for a lost preparation.
        let mut suspects = std::collections::HashSet::new();
        loop {
            tokio::time::sleep(RECOVERY_INTERVAL).await;
            let Some(state) = state.upgrade() else {
                return;
            };
            if *state.shutdown.borrow() {
                return;
            }
            let current: std::collections::HashSet<String> = interrupted(&state)
                .into_iter()
                .map(|meta| meta.id)
                .collect();
            let confirmed = current
                .intersection(&suspects)
                .filter_map(|id| state.hub.session_info(id).map(|info| info.meta))
                .collect::<Vec<_>>();
            suspects = current;
            for meta in confirmed {
                let Some(source_path) = meta.workspace_source_path.as_deref() else {
                    continue;
                };
                if schedule(&state, &meta.id, &meta.machine_id, source_path, meta.system) {
                    tracing::warn!(
                        session = %meta.id,
                        machine = %meta.machine_id,
                        "resuming interrupted Machine workspace preparation"
                    );
                }
            }
        }
    });
}

fn interrupted(state: &AppState) -> Vec<crate::core::SessionMeta> {
    let in_flight = state.workspace_preparations.lock().clone();
    state
        .hub
        .session_list()
        .into_iter()
        .filter(|meta| {
            meta.status == crate::agent_model::Status::Starting
                && !meta.closing
                && meta.awaits_workspace_preparation()
                && !in_flight.contains(&meta.id)
                && !state.hub.session_is_trashed(&meta.id)
                && !has_worker(state, meta)
                && state
                    .runtime_router
                    .runtime(&meta.machine_id)
                    .is_some_and(|runtime| runtime.connected())
        })
        .collect()
}

async fn prepare(
    state: &Arc<AppState>,
    session_id: &str,
    machine_id: &str,
    source_path: &str,
    eager: bool,
) -> Result<(), String> {
    let prepared = state
        .machine_control
        .adapter_request(
            machine_id,
            "workspace",
            serde_json::json!({
                "root": source_path,
                "session_id": session_id,
                "base_session_id": state
                    .hub
                    .session_fork(session_id)
                    .map(|record| record.source_session_id),
            }),
        )
        .await
        .map_err(|error| format!("Machine could not prepare an isolated workspace: {error}"))
        .and_then(|value| {
            serde_json::from_value::<PreparedMachineWorkspace>(value)
                .map_err(|error| format!("Machine returned invalid workspace metadata: {error}"))
        })?;
    tracing::info!(
        session_id = %session_id,
        machine_id = %machine_id,
        source_path = %prepared.source_path,
        prepared_path = %prepared.path,
        revision = ?prepared.revision,
        isolated = prepared.isolated,
        created = prepared.created,
        "prepared Machine workspace for session"
    );
    // A shared non-Git root prepares to its source path, so preparation
    // changes nothing; never disturb a worker that started there meanwhile.
    if state
        .hub
        .session_info(session_id)
        .is_some_and(|info| info.meta.cwd == prepared.path && has_worker(state, &info.meta))
    {
        return Ok(());
    }
    loop {
        match plugin_fence_state_for_session(&state.hub, &state.plugin_lifecycle_fences, session_id)
        {
            Some(PluginFenceState::Uninstalling) => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Some(PluginFenceState::Uninstalled | PluginFenceState::NeedsReconcile) => {
                return Ok(());
            }
            Some(PluginFenceState::Installing) | None => break,
        }
    }
    let prepared_session = if eager {
        state
            .hub
            .update_session_cwd(session_id, prepared.path)
            .and_then(|()| state.supervisor.start_registered_session(session_id))
    } else {
        state
            .supervisor
            .finish_session_preparation(session_id, |hub| {
                hub.update_session_cwd(session_id, prepared.path)
            })
    };
    prepared_session.map_err(|error| format!("Session preparation failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web_session(hub: &Hub, status: Status) -> crate::core::SessionMeta {
        hub.create_session(crate::core::SessionRegistration {
            id: "s".to_owned(),
            provider: "codex".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: Some("columbus".to_owned()),
            workspace_name: Some("columbus".to_owned()),
            workspace_source_path: Some("/home/draven/columbus".to_owned()),
            execution_binding: None,
            cwd: "/home/draven/columbus".to_owned(),
            title: "test".to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.set_status("s", status, Some("detail".to_owned()));
        hub.session_info("s").expect("session").meta
    }

    #[test]
    fn failed_or_interrupted_preparation_still_awaits_its_workspace() {
        let hub = Hub::new();
        for status in [Status::Starting, Status::Crashed] {
            assert!(web_session(&hub, status).awaits_workspace_preparation());
        }
        // A successfully prepared shared (non-Git) root keeps its source cwd.
        for status in [Status::Exited, Status::Running, Status::Busy] {
            let meta = web_session(&hub, status);
            assert!(meta.on_workspace_source_root());
            assert!(!meta.awaits_workspace_preparation());
        }
        hub.set_status("s", Status::Crashed, None);
        hub.update_session_cwd("s", "/srv/sessions/s".to_owned())
            .expect("prepared cwd");
        assert!(
            !hub.session_info("s")
                .expect("session")
                .meta
                .awaits_workspace_preparation()
        );
    }
}
