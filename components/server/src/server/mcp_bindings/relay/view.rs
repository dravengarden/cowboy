//! View scopes (docs/widget-apps-mcp.md §7): a view's `tools/call` runs in its
//! own MCP process, opened through the same relay under the parent's live
//! grant, current policy epoch and execution connection as the agent's scope,
//! but never sharing the agent's process. A slow or failing view call can only
//! retire its own handle; the agent's calls never queue behind it.
//!
//! One scope per (session, binding) serves one request at a time. A
//! dispatched request is owned by the Controller until the backend answers or
//! times out, so a dropped HTTP request (frame teardown) drops only the
//! delivery. An idle or unleased scope stops taking calls at once and closes
//! its transport once nothing is in flight; authority loss closes at once.
use std::sync::atomic::AtomicU64;

use super::*;

const IDLE: Duration = Duration::from_secs(60);
const SWEEP: Duration = Duration::from_secs(5);
/// The MCP revision the Controller asks for, and the ones it accepts back
/// (the Agent API's own list).
const PROTOCOL_VERSION: &str = "2025-06-18";
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Default)]
pub(in crate::server) struct Views {
    scopes: Mutex<HashMap<(String, String), Arc<View>>>,
    /// Serializes opening so two first calls cannot start two processes.
    opening: tokio::sync::Mutex<()>,
}

struct View {
    key: (String, String),
    id: String,
    session: Arc<Session>,
    epoch: String,
    tools: Vec<String>,
    flight: Arc<tokio::sync::Mutex<()>>,
    closing: AtomicBool,
    last_used: Mutex<std::time::Instant>,
    next_id: AtomicU64,
}

/// One view-originated call, already checked against the lease and the
/// owner's grant by the caller.
pub(in crate::server) struct ViewCall {
    pub(in crate::server) session: String,
    pub(in crate::server) binding: String,
    pub(in crate::server) epoch: String,
    /// The app projection: the grant's `call_tools`, sorted.
    pub(in crate::server) tools: Vec<String>,
    pub(in crate::server) name: String,
    pub(in crate::server) arguments: Value,
    /// The frame lease the call was admitted under; rechecked after every
    /// opening wait, immediately before dispatch.
    pub(in crate::server) lease: String,
}

/// Why a view call has no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::server) enum ViewError {
    /// Nothing was sent to the backend.
    Refused(&'static str),
    /// The call was dispatched and its outcome is unknown; never retried.
    Uncertain(&'static str),
}

/// Runs `call` and returns the backend's JSON-RPC response object. The whole
/// run, opening included, is Controller-owned: a dropped HTTP request never
/// leaves a half-opened scope or cancels a dispatched call.
pub(in crate::server) async fn call(
    state: &Arc<AppState>,
    call: ViewCall,
) -> Result<Value, ViewError> {
    let state = Arc::clone(state);
    tokio::spawn(async move { run(&state, call).await })
        .await
        .map_err(|_| ViewError::Uncertain("mcp_unavailable"))?
}

async fn run(state: &Arc<AppState>, call: ViewCall) -> Result<Value, ViewError> {
    let view = scope(state, &call).await.map_err(ViewError::Refused)?;
    let guard = Arc::clone(&view.flight)
        .try_lock_owned()
        .map_err(|_| ViewError::Refused("mcp_busy"))?;
    if view.closing.load(Ordering::Acquire) {
        return Err(ViewError::Refused("mcp_view_closed"));
    }
    // Opening may have waited: the run that asked must still hold its lease.
    super::super::apps_bridge::authorized(state, &call.session, &call.lease)
        .await
        .map_err(ViewError::Refused)?;
    *view.last_used.lock() = std::time::Instant::now();
    let id = format!("view-{}", view.next_id.fetch_add(1, Ordering::Relaxed));
    let message = json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
        "params":{"name":call.name,"arguments":call.arguments}});
    let response = send(state, &view, message).await;
    *view.last_used.lock() = std::time::Instant::now();
    drop(guard);
    response.map_err(ViewError::Uncertain)
}

fn retire_view(state: &AppState, view: &Arc<View>) {
    view.closing.store(true, Ordering::Release);
    {
        let mut scopes = state.extensions.mcp.views.scopes.lock();
        if scopes
            .get(&view.key)
            .is_some_and(|current| Arc::ptr_eq(current, view))
        {
            scopes.remove(&view.key);
        }
    }
    retire(state, &view.id, &view.session);
}

async fn scope(state: &Arc<AppState>, call: &ViewCall) -> Result<Arc<View>, &'static str> {
    let views = &state.extensions.mcp.views;
    let _opening = views.opening.lock().await;
    let key = (call.session.clone(), call.binding.clone());
    let existing = views.scopes.lock().get(&key).cloned();
    if let Some(view) = existing {
        let authority = view.session.current(state).is_ok();
        if authority
            && view.epoch == call.epoch
            && view.tools == call.tools
            && !view.closing.load(Ordering::Acquire)
        {
            return Ok(view);
        }
        // Authority loss closes at once; otherwise a scope is replaced only
        // once nothing is in flight, so a confirmed write is never cut off.
        if authority {
            let _idle = Arc::clone(&view.flight)
                .try_lock_owned()
                .map_err(|_| "mcp_busy")?;
            retire_view(state, &view);
        } else {
            retire_view(state, &view);
        }
    }
    let (grant, connection) = super::super::super::managed_calls::mcp_parent(state, &call.session)
        .ok_or("mcp_parent_unavailable")?;
    let permit = Arc::clone(&state.extensions.mcp.capacity)
        .try_acquire_owned()
        .map_err(|_| "mcp_capacity")?;
    let (meta, _) = super::super::super::managed_calls::authorize(state, &connection, &grant)?;
    let admission = state
        .extensions
        .bindings
        .admit(
            &state.hub,
            &meta.id,
            &meta.machine_id,
            &call.epoch,
            &call.binding,
        )?
        .for_view(call.tools.clone());
    let (id, session) = open_admitted(state, &connection, &grant, admission, permit).await?;
    let view = Arc::new(View {
        key: key.clone(),
        id,
        session,
        epoch: call.epoch.clone(),
        tools: call.tools.clone(),
        flight: Arc::new(tokio::sync::Mutex::new(())),
        closing: AtomicBool::new(false),
        last_used: Mutex::new(std::time::Instant::now()),
        next_id: AtomicU64::new(1),
    });
    handshake(state, &view).await?;
    views.scopes.lock().insert(key, Arc::clone(&view));
    sweep(state, &view);
    Ok(view)
}

/// The backend handshake the native bridge performs for the agent; a failed
/// one closes the new handle and the call is refused, never replayed.
async fn handshake(state: &AppState, view: &Arc<View>) -> Result<(), &'static str> {
    let initialize = json!({"jsonrpc":"2.0","id":"view-0","method":"initialize","params":{
        "protocolVersion":PROTOCOL_VERSION,"capabilities":{},
        "clientInfo":{"name":"cowboy-view-scope","version":env!("CARGO_PKG_VERSION")}}});
    let reply = send(state, view, initialize).await?;
    if !reply["result"]["protocolVersion"]
        .as_str()
        .is_some_and(|version| PROTOCOL_VERSIONS.contains(&version))
    {
        retire_view(state, view);
        return Err("mcp_handshake_failed");
    }
    send(
        state,
        view,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await
    .map(drop)
}

async fn send(state: &AppState, view: &Arc<View>, message: Value) -> Result<Value, &'static str> {
    let session = &view.session;
    if let Err(code) = session.current(state) {
        retire_view(state, view);
        return Err(code);
    }
    let mut cancelled = session.subscribe_cancellation()?;
    let response = tokio::select! {
        biased;
        _ = cancelled.changed() => return Err("mcp_authority_ended"),
        result = state.machine_control.mcp_request_on_connection(&session.remote.connection,
            TransportRequest::Request { handle: session.remote.handle.clone(), message }) => result,
    };
    if let Err(code) = session.current(state) {
        retire_view(state, view);
        return Err(code);
    }
    match response {
        Ok(mut value) if value.get("response").is_some() => Ok(value["response"].take()),
        _ => {
            // The Machine closes a process whose request failed or timed out.
            retire_view(state, view);
            Err("mcp_unavailable")
        }
    }
}

/// Closes a scope nobody uses: no frame of its binding holds a lease, or no
/// call for a minute. New calls are refused at once; the transport closes
/// only once a dispatched request has answered or timed out.
fn sweep(state: &Arc<AppState>, view: &Arc<View>) {
    let state = Arc::downgrade(state);
    let view = Arc::downgrade(view);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SWEEP).await;
            let (Some(state), Some(view)) = (state.upgrade(), view.upgrade()) else {
                return;
            };
            if view.closing.load(Ordering::Acquire) {
                return;
            }
            let (session, binding) = &view.key;
            let unused = view.last_used.lock().elapsed() >= IDLE
                || !state.extensions.apps.leased(session, binding);
            if unused {
                view.closing.store(true, Ordering::Release);
                // Wait for any dispatched request before closing.
                let _flight = view.flight.lock().await;
                retire_view(&state, &view);
                return;
            }
        }
    });
}
