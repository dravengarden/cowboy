//! The adopt-only declarations the Controller caches from worker reports and
//! re-sends on every Machine (re)connect.
use super::{CoreCommand, Shared, StartSession, WorkerSnapshot, snapshot};

/// Cache the worker's launch as its adopt-only declaration. Returns that
/// declaration when the Machine needs it sent again (see below).
pub(super) fn update_declaration(shared: &Shared, worker: &WorkerSnapshot) -> Option<StartSession> {
    if !snapshot::admits_worker(shared, worker) {
        return None;
    }
    let mut session = worker.launch.clone()?;
    session.adopt_only = true;
    // A worker never reports its session-extension switches, and the Machine
    // may fill a report from its own stale declaration. The session's current
    // ones win, as any relaunch declares them; an ordinary launch records no
    // declaration here at all. Every Machine (re)connect re-sends this cache,
    // and the Machine relaunches from it on a generation cutover, which
    // refuses an MCP launch without them (2026-11-25, sess-1794380205511
    // crashed after an OVH Machine restart).
    let current = shared
        .hub
        .session_info(&session.session_id)
        .map(|info| crate::controller_hooks::launch_extensions(&shared.hub, &info.meta));
    // A draining MCP worker that reported after this connection's declarations
    // waits for them: the Machine keeps it until it can relaunch it, and its
    // own idle and permission checks decide when.
    let repair = worker.drain_requested
        && session.mcp_plan.is_some()
        && session.extensions.is_none()
        && current.is_some()
        && shared
            .declaration_repairs
            .lock()
            .get(&session.session_id)
            .is_none_or(|epoch| epoch != &worker.worker_epoch);
    let mut declarations = shared.declarations.lock();
    let declared = declarations
        .get(&session.session_id)
        .and_then(|old| old.extensions);
    session.extensions = current.or(session.extensions).or(declared);
    declarations.insert(session.session_id.clone(), session.clone());
    repair.then_some(session)
}

/// Send a repaired declaration after the snapshot acknowledged earlier ones,
/// once per worker epoch. A reset holds its own declaration: the claim and
/// the insertion happen under the locks a reset takes, so neither replaces
/// the other.
pub(super) fn resend_declaration(shared: &Shared, session: StartSession, epoch: &str) {
    let key = format!("ensure:{}", session.session_id);
    let resetting = shared.resetting.lock();
    if resetting.contains(&session.session_id) {
        return;
    }
    let mut pending = shared.pending.lock();
    if pending.contains_key(&key) {
        return;
    }
    shared
        .declaration_repairs
        .lock()
        .insert(session.session_id.clone(), epoch.to_owned());
    shared.sent.lock().remove(&key);
    pending.insert(key, CoreCommand::EnsureSession { session });
    drop(pending);
    drop(resetting);
    let _ = shared.notify.send(());
}
