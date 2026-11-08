//! Parent-authorized MCP sessions. Transport handles never substitute for the
//! original grant, policy epoch or either enrolled connection incarnation.
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use super::{AppState, authority::Admission};
use crate::machine_control::{ConnectionToken, MachineControl};
use crate::machine_protocol::MachineCommand;
use crate::managed_calls::protocol::Grant;
use crate::mcp_binding::{RelayAction, RelayRequest, TransportRequest};

pub(in crate::server) struct Relay {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    capacity: Arc<Semaphore>,
}

impl Default for Relay {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            capacity: Arc::new(Semaphore::new(32)),
        }
    }
}

struct Remote {
    control: Arc<MachineControl>,
    connection: ConnectionToken,
    handle: String,
    closed: AtomicBool,
}

impl Remote {
    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            let _ = self.control.send_on_connection(
                &self.connection,
                MachineCommand::McpTransport {
                    request_id: format!("mcp-close-{:032x}", rand::random::<u128>()),
                    request: TransportRequest::Close {
                        handle: self.handle.clone(),
                    },
                },
            );
        }
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.close();
    }
}

struct Session {
    admission: Mutex<Admission>,
    grant: Grant,
    runtime: ConnectionToken,
    remote: Remote,
    cancelled: watch::Sender<bool>,
    _permit: OwnedSemaphorePermit,
}

impl Session {
    fn current(&self, state: &AppState) -> Result<(), &'static str> {
        if *state.shutdown.borrow() {
            return Err("mcp_authority_ended");
        }
        self.scope_current(
            &state.extensions.bindings,
            &state.hub,
            &state.machine_control,
        )?;
        super::super::managed_calls::authorize(state, &self.remote.connection, &self.grant)?;
        Ok(())
    }

    fn scope_current(
        &self,
        policies: &super::Policies,
        hub: &super::Hub,
        control: &MachineControl,
    ) -> Result<(), &'static str> {
        if self.remote.closed.load(Ordering::Acquire)
            || !control.is_current(&self.runtime)
            || !control.is_current(&self.remote.connection)
        {
            return Err("mcp_authority_ended");
        }
        self.admission.lock().current(policies, hub)
    }

    fn subscribe_cancellation(&self) -> Result<watch::Receiver<bool>, &'static str> {
        let receiver = self.cancelled.subscribe();
        if *receiver.borrow() {
            return Err("mcp_authority_ended");
        }
        Ok(receiver)
    }

    fn end(&self) {
        self.cancelled.send_replace(true);
        self.remote.close();
    }

    fn owns(&self, connection: &ConnectionToken, grant: &Grant, request: &RelayRequest) -> bool {
        self.grant == *grant
            && self.remote.connection.same(connection)
            && self
                .admission
                .lock()
                .matches(&request.binding, &request.epoch)
    }
}

/// Close is synchronous so saturated read capacity cannot prevent cleanup.
pub(in crate::server) fn close(
    state: &AppState,
    connection: &ConnectionToken,
    grant: &Grant,
    request: &RelayRequest,
) -> Value {
    let result = (|| {
        if !grant.mcp || !grant.agent_api || request.validate().is_err() {
            return Err("mcp_not_admitted");
        }
        super::super::managed_calls::authorize(state, connection, grant)?;
        let RelayAction::Close { handle } = &request.action else {
            return Err("invalid_contract");
        };
        let session = state.extensions.mcp.sessions.lock().get(handle).cloned();
        let session = session
            .filter(|session| session.owns(connection, grant, request))
            .ok_or("mcp_handle_unavailable")?;
        retire(state, handle, &session);
        Ok(json!({"closed":true}))
    })();
    envelope(result.unwrap_or_else(failure))
}

pub(in crate::server) async fn handle(
    state: &Arc<AppState>,
    connection: &ConnectionToken,
    grant: &Grant,
    request: RelayRequest,
) -> Value {
    if matches!(request.action, RelayAction::Close { .. }) {
        return close(state, connection, grant, &request);
    }
    envelope(handle_inner(state, connection, grant, request).await)
}

fn envelope(value: Value) -> Value {
    if let Some(error) = value.get("error") {
        json!({"schema":1,"state":"error","error":error})
    } else {
        json!({"schema":1,"state":"ok","result":value})
    }
}

async fn handle_inner(
    state: &Arc<AppState>,
    connection: &ConnectionToken,
    grant: &Grant,
    request: RelayRequest,
) -> Value {
    if !grant.mcp || !grant.agent_api || request.validate().is_err() {
        return failure("mcp_not_admitted");
    }
    match &request.action {
        RelayAction::Open => match open(state, connection, grant, &request).await {
            Ok(handle) => json!({"handle":handle}),
            Err(code) => failure(code),
        },
        RelayAction::Close { .. } => failure("invalid_contract"),
        RelayAction::Request { handle, .. } => {
            let session = state.extensions.mcp.sessions.lock().get(handle).cloned();
            let Some(session) = session.filter(|session| session.owns(connection, grant, &request))
            else {
                return failure("mcp_handle_unavailable");
            };
            if let Err(code) = session.current(state) {
                retire(state, handle, &session);
                return failure(code);
            }
            let RelayAction::Request { message, .. } = &request.action else {
                unreachable!()
            };
            let mut flight = InFlight {
                state: Arc::downgrade(state),
                session: Arc::clone(&session),
                handle: handle.clone(),
                completed: false,
            };
            let mut cancelled = match session.subscribe_cancellation() {
                Ok(receiver) => receiver,
                Err(code) => return failure(code),
            };
            let response = tokio::select! {
                biased;
                _ = cancelled.changed() => return failure("mcp_authority_ended"),
                result = state.machine_control.mcp_request_on_connection(&session.remote.connection,
                    TransportRequest::Request { handle: session.remote.handle.clone(), message: message.clone() }) => result,
            };
            if let Err(code) = session.current(state) {
                retire(state, handle, &session);
                return failure(code);
            }
            match response {
                Ok(value) if value.get("response").is_some() => {
                    flight.completed = true;
                    value
                }
                _ => {
                    retire(state, handle, &session);
                    failure("mcp_unavailable")
                }
            }
        }
    }
}

fn failure(code: &str) -> Value {
    json!({"error":{"code":code}})
}

fn retire(state: &AppState, handle: &str, session: &Session) {
    session.end();
    state.extensions.mcp.sessions.lock().remove(handle);
}

async fn open(
    state: &Arc<AppState>,
    connection: &ConnectionToken,
    grant: &Grant,
    request: &RelayRequest,
) -> Result<String, &'static str> {
    let permit = Arc::clone(&state.extensions.mcp.capacity)
        .try_acquire_owned()
        .map_err(|_| "mcp_capacity")?;
    let (meta, _) = super::super::managed_calls::authorize(state, connection, grant)?;
    let admission = state.extensions.bindings.admit(
        &state.hub,
        &meta.id,
        &meta.machine_id,
        &request.epoch,
        &request.binding,
    )?;
    let profile = admission.profile(&state.service_id);
    if profile.execution_machine != connection.machine_id() {
        return Err("mcp_wrong_execution_machine");
    }
    let (runtime, _) = state
        .machine_control
        .connection_protocol(&profile.runtime_machine)
        .ok_or("mcp_runtime_unavailable")?;
    let value = state
        .machine_control
        .mcp_request_on_connection(
            connection,
            TransportRequest::Open {
                profile: Box::new(profile),
            },
        )
        .await
        .map_err(|_| "mcp_open_unknown")?;
    let handle = value
        .get("handle")
        .and_then(Value::as_str)
        .ok_or("mcp_unavailable")?
        .to_owned();
    TransportRequest::Close {
        handle: handle.clone(),
    }
    .validate()
    .map_err(|_| "mcp_invalid_handle")?;
    let session = Arc::new(Session {
        admission: Mutex::new(admission),
        grant: grant.clone(),
        runtime,
        remote: Remote {
            control: Arc::clone(&state.machine_control),
            connection: connection.clone(),
            handle,
            closed: AtomicBool::new(false),
        },
        cancelled: watch::channel(false).0,
        _permit: permit,
    });
    session.current(state)?;
    let id = format!("relay-{:032x}", rand::random::<u128>());
    {
        let mut sessions = state.extensions.mcp.sessions.lock();
        if sessions.contains_key(&id) {
            return Err("mcp_capacity");
        }
        sessions.insert(id.clone(), Arc::clone(&session));
    }
    watch_session(state, &id, &session);
    renewal::start(state, &id, &session);
    if let Err(code) = session.current(state) {
        retire(state, &id, &session);
        return Err(code);
    }
    Ok(id)
}

fn watch_session(state: &Arc<AppState>, id: &str, session: &Arc<Session>) {
    let state = Arc::downgrade(state);
    let id = id.to_owned();
    let session = Arc::downgrade(session);
    tokio::spawn(async move {
        let Some(initial) = session.upgrade() else {
            return;
        };
        let mut changes = initial.admission.lock().changes.clone();
        drop(initial);
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            let (Some(state), Some(session)) = (state.upgrade(), session.upgrade()) else {
                return;
            };
            if session.current(&state).is_err() {
                retire(&state, &id, &session);
                return;
            }
            let deadline = session.admission.lock().deadline();
            drop(state);
            drop(session);
            tokio::select! { _ = changes.changed() => {}, _ = tick.tick() => {}, () = tokio::time::sleep_until(deadline) => {} }
        }
    });
}

struct InFlight {
    state: std::sync::Weak<AppState>,
    session: Arc<Session>,
    handle: String,
    completed: bool,
}
impl Drop for InFlight {
    fn drop(&mut self) {
        if !self.completed {
            self.session.end();
            if let Some(state) = self.state.upgrade() {
                state.extensions.mcp.sessions.lock().remove(&self.handle);
            }
        }
    }
}

#[cfg(test)]
mod tests;

mod renewal;
