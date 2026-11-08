//! Keepalive is Controller-owned. Neither a delayed ACK nor a new policy epoch
//! may revive the original admission; the independent watcher keeps revoking it.
use super::*;

pub(super) fn start(state: &Arc<AppState>, id: &str, session: &Arc<Session>) {
    let state = Arc::downgrade(state);
    let session = Arc::downgrade(session);
    let id = id.to_owned();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let (Some(state), Some(session)) = (state.upgrade(), session.upgrade()) else {
                return;
            };
            if renew(&state, &session).await.is_err() {
                retire(&state, &id, &session);
                return;
            }
        }
    });
}

async fn renew(state: &AppState, session: &Session) -> Result<(), &'static str> {
    session.current(state)?;
    let (candidate, deadline) = {
        let admission = session.admission.lock();
        (
            admission.renewal(&state.extensions.bindings, &state.hub)?,
            admission.deadline(),
        )
    };
    let mut cancelled = session.subscribe_cancellation()?;
    let request = TransportRequest::Renew {
        handle: session.remote.handle.clone(),
        profile: Box::new(candidate.profile(&state.service_id)),
    };
    let response = tokio::select! {
        biased;
        _ = cancelled.changed() => return Err("mcp_authority_ended"),
        () = tokio::time::sleep_until(deadline) => return Err("mcp_authority_ended"),
        response = state.machine_control.mcp_request_on_connection(&session.remote.connection, request) => response,
    }.map_err(|_| "mcp_unavailable")?;
    if response != json!({"renewed":true}) {
        return Err("mcp_unavailable");
    }
    session.current(state)?;
    session
        .admission
        .lock()
        .commit_renewal(candidate, &state.extensions.bindings, &state.hub)
}
