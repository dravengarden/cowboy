//! Repairs execution lifetimes lost with their host. A reboot strands every
//! idle session on a binding nothing would otherwise check until its next
//! prompt, and any earlier worker replacement would crash on it.

use super::super::*;
use super::recovery;
use crate::core::SessionMeta;
use std::time::Duration;

const PASS_INTERVAL: Duration = Duration::from_secs(30);
/// Each recovery cold-starts a worker; a reboot must not start them all at once.
const MAX_RECOVERIES_PER_PASS: usize = 4;

pub(in crate::server) async fn run(state: Arc<AppState>) {
    let mut shutdown = state.shutdown.clone();
    let mut tick = tokio::time::interval(PASS_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tick.tick() => {},
        }
        pass(&state, &shutdown).await;
    }
}

async fn pass(state: &Arc<AppState>, shutdown: &tokio::sync::watch::Receiver<bool>) {
    let mut recovered = 0;
    for meta in state.hub.session_list() {
        if *shutdown.borrow() || recovered >= MAX_RECOVERIES_PER_PASS {
            return;
        }
        if !unverified(state, &meta) {
            continue;
        }
        // The same preflight a prompt runs, so idle, in-flight and busy
        // sessions keep its fences. A refusal is retried on a later pass.
        let intent = match recovery::before_dispatch(state, &meta.id).await {
            Ok(Some(intent)) => intent,
            Ok(None) => continue,
            Err(error) => {
                tracing::debug!(session = %meta.id, %error, "execution lifetime not reconciled");
                continue;
            }
        };
        match state.supervisor.recover_execution_session(&meta.id, intent) {
            Ok(()) => {
                recovered += 1;
                tracing::info!(
                    event_name = "cowboy.execution.lifetime_reconciled",
                    session = %meta.id,
                    "recreated an idle session's lost execution lifetime"
                );
            }
            Err(error) => {
                tracing::warn!(session = %meta.id, %error, "replacing the worker of a recovered lifetime failed");
            }
        }
    }
}

/// An explicit pause holds a session for inspection; a dormant one wakes when
/// it next needs a worker. Everything else idle must not wait for a prompt.
fn awaits_reconciliation(status: Status, paused: bool) -> bool {
    !paused && matches!(status, Status::Running | Status::Interrupted)
}

/// Only a Describe on the current target connection counts as verified.
fn unverified(state: &AppState, meta: &SessionMeta) -> bool {
    let Some(record) = &meta.execution_binding else {
        return false;
    };
    let binding = record.decode().ok();
    if record.managed().is_some()
        || (binding.is_none() && record.recovery().is_none())
        || !awaits_reconciliation(meta.status, meta.paused)
        || state.hub.session_is_trashed(&meta.id)
    {
        return false;
    }
    let Some(runtime) = state.runtime_router.runtime(&meta.machine_id) else {
        return false;
    };
    runtime.has_worker(&meta.id)
        && !binding.is_some_and(|binding| {
            recovery::verified(state, &meta.id, &binding)
                && runtime.worker_matches_execution(&meta.id, &binding)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_idle_unpaused_sessions_are_reconciled() {
        assert!(awaits_reconciliation(Status::Running, false));
        assert!(awaits_reconciliation(Status::Interrupted, false));
        assert!(!awaits_reconciliation(Status::Running, true));
        for status in [
            Status::Busy,
            Status::Starting,
            Status::Exited,
            Status::Crashed,
        ] {
            assert!(!awaits_reconciliation(status, false), "{status:?}");
        }
    }
}
