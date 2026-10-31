//! Route one Machine's memory request to the home Machine. Identity comes from
//! this Controller's session record, never from the request. Memory is
//! optional: every failure here answers "unavailable", which clients treat as
//! continuing without memory, and nothing is retried or queued.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

use super::AppState;
use crate::machine_protocol::MachineCommand;
use crate::machine_protocol::memory_relay::{Identity, Request, Response};

/// Slightly beyond the home Machine's own deadlines, so that one answers first.
const TURN_DEADLINE: Duration = Duration::from_secs(9);
const TOOL_DEADLINE: Duration = Duration::from_secs(32);
/// Memory has its own budget: it must never take capacity from execution.
const MAX_IN_FLIGHT: usize = 64;

struct Waiter {
    home: String,
    sender: oneshot::Sender<Response>,
}

pub(super) struct Relay {
    next: AtomicU64,
    pending: Mutex<HashMap<String, Waiter>>,
    capacity: Arc<Semaphore>,
}

impl Default for Relay {
    fn default() -> Self {
        Self {
            next: AtomicU64::default(),
            pending: Mutex::default(),
            capacity: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
        }
    }
}

impl Relay {
    pub(super) fn try_admit(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.capacity).try_acquire_owned().ok()
    }

    /// Complete one request, from the Machine it was sent to only.
    pub(super) fn complete(&self, machine_id: &str, request_id: &str, response: Response) {
        let mut pending = self.pending.lock();
        if pending
            .get(request_id)
            .is_some_and(|waiter| waiter.home == machine_id)
            && let Some(waiter) = pending.remove(request_id)
        {
            let _ = waiter.sender.send(response);
        }
    }
}

/// The identity a Machine may act for: its own, memory-enabled session.
fn identity(
    state: &AppState,
    machine_id: &str,
    session_id: &str,
) -> Result<Identity, &'static str> {
    let meta = state
        .hub
        .session_info(session_id)
        .ok_or("unknown session")?
        .meta;
    if meta.machine_id != machine_id {
        return Err("session runs on another Machine");
    }
    if meta.system {
        return Err("system sessions have no memory");
    }
    let switches = super::launch_extensions(&state.hub, &meta);
    if !switches.memory_tools && !switches.memory_recall {
        return Err("memory is off for this session");
    }
    let binding = meta
        .execution_binding
        .as_ref()
        .map(crate::execution_environment::ExecutionBinding::decode)
        .transpose()
        .map_err(|_| "unreadable execution binding")?;
    let (machine, workspace) = binding.map_or_else(
        || (meta.machine_id.clone(), meta.workspace_id.clone()),
        |binding| (binding.environment.machine_id, Some(binding.workspace.id)),
    );
    Ok(Identity {
        provider: meta.provider,
        session: meta.id,
        machine,
        workspace,
    })
}

pub(super) async fn handle(
    state: &Arc<AppState>,
    machine_id: &str,
    session_id: &str,
    request: Request,
) -> Response {
    match relay(state, machine_id, session_id, request).await {
        Ok(response) => response,
        Err(reason) => {
            tracing::debug!(
                machine = machine_id,
                session = session_id,
                reason,
                "memory relay refused"
            );
            Response::unavailable()
        }
    }
}

async fn relay(
    state: &Arc<AppState>,
    machine_id: &str,
    session_id: &str,
    request: Request,
) -> Result<Response, &'static str> {
    request.validate().map_err(|_| "invalid request")?;
    let identity = identity(state, machine_id, session_id)?;
    let home = state
        .service_config
        .get(&crate::config::schema::MEMORY_HOME_MACHINES)
        .into_iter()
        .next()
        .ok_or("no memory home Machine is configured")?;
    let (connection, _) = state
        .machine_control
        .connection_protocol(&home)
        .ok_or("the memory home Machine is not connected")?;
    let request_id = format!(
        "memory-{}",
        state.memory_relay.next.fetch_add(1, Ordering::Relaxed)
    );
    let deadline = if request.path == "/mcp" {
        TOOL_DEADLINE
    } else {
        TURN_DEADLINE
    };
    let (sender, receiver) = oneshot::channel();
    state
        .memory_relay
        .pending
        .lock()
        .insert(request_id.clone(), Waiter { home, sender });
    let sent = state.machine_control.send_on_connection(
        &connection,
        MachineCommand::MemoryRelay {
            request_id: request_id.clone(),
            identity,
            request,
        },
    );
    let response = if sent.is_ok() {
        tokio::time::timeout(deadline, receiver)
            .await
            .ok()
            .and_then(Result::ok)
    } else {
        None
    };
    state.memory_relay.pending.lock().remove(&request_id);
    response.ok_or("the memory home Machine did not answer")
}
