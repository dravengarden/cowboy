//! Controller side of Machine bulk lanes (see `crate::machine_lanes`).

use super::*;
use crate::machine_lanes::{LANE_FRESH, LANE_MIN_BYTES, LaneSet};
use std::collections::HashMap;
use std::sync::LazyLock;

/// What a lane needs from its authenticated primary connection.
#[derive(Clone)]
pub(super) struct LaneTarget {
    pub connection: crate::machine_control::ConnectionToken,
    pub machine_id: String,
    pub execution_calls: Arc<tokio::sync::Semaphore>,
    pub primary: mpsc::UnboundedSender<Message>,
    pub lanes: Arc<LaneSet<Message>>,
    pub features: (bool, bool, bool, bool),
}

/// Lane tokens of authenticated primary connections. A token is registered
/// only after the Machine's challenge proof is accepted and removed when its
/// primary connection ends.
static TARGETS: LazyLock<parking_lot::Mutex<HashMap<String, LaneTarget>>> =
    LazyLock::new(Default::default);

pub(super) struct Registration(String);

impl Drop for Registration {
    fn drop(&mut self) {
        TARGETS.lock().remove(&self.0);
    }
}

pub(super) fn register(token: String, target: LaneTarget) -> Registration {
    TARGETS.lock().insert(token.clone(), target);
    Registration(token)
}

/// Where a reply goes: a fresh lane when it is large and one is open,
/// otherwise the primary connection.
#[derive(Clone)]
pub(super) struct ReplyRoute {
    pub primary: mpsc::UnboundedSender<Message>,
    pub lanes: Arc<LaneSet<Message>>,
}

impl ReplyRoute {
    pub(super) fn send_json<T: Serialize>(&self, message: &T) -> Result<(), ()> {
        let text = serde_json::to_string(message).map_err(|_| ())?;
        let message = Message::Text(text.into());
        let message = if message_len(&message) >= LANE_MIN_BYTES {
            match self.lanes.send(message, Some(LANE_FRESH)) {
                Ok(_) => return Ok(()),
                Err(message) => message,
            }
        } else {
            message
        };
        queue_machine_message(&self.primary, message)
    }
}

fn message_len(message: &Message) -> usize {
    match message {
        Message::Text(text) => text.len(),
        _ => 0,
    }
}

/// Run one execution request and send its reply by `route`, exactly as the
/// primary connection always has.
pub(super) fn spawn_execution_request(
    state: &Arc<AppState>,
    connection: &crate::machine_control::ConnectionToken,
    machine_id: &str,
    execution_calls: &Arc<tokio::sync::Semaphore>,
    request: Box<crate::execution_protocol::RuntimeRequest>,
    route: ReplyRoute,
) {
    let permit = Arc::clone(execution_calls).try_acquire_owned();
    let execution_state = Arc::clone(state);
    let execution_connection = connection.clone();
    let runtime_machine = machine_id.to_owned();
    tokio::spawn(async move {
        let reply = if let Ok(_permit) = permit {
            execution::forward(
                &execution_state,
                &execution_connection,
                &runtime_machine,
                *request,
            )
            .await
        } else {
            crate::execution_protocol::RuntimeReply {
                session_id: request.session_id,
                worker_epoch: request.worker_epoch,
                request_id: request.request_id,
                scope: crate::execution_protocol::Scope::from_binding(&request.binding),
                response: crate::machine_protocol::execution::Response::Refused {
                    reason: crate::machine_protocol::execution::Refusal::Capacity,
                },
            }
        };
        if execution_state
            .machine_control
            .is_current(&execution_connection)
        {
            let _ = route.send_json(&crate::machine_protocol::MachineFrame::Runtime {
                frame: crate::runtime_wire::Frame::ExecutionReply {
                    reply: Box::new(reply),
                },
            });
        }
    });
}

pub(super) async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let target = headers
        .get(crate::machine_lanes::LANE_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|token| TARGETS.lock().get(token).cloned());
    let Some(target) = target else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !state.machine_control.is_current(&target.connection) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| serve(socket, state, target))
}

async fn serve(socket: WebSocket, state: Arc<AppState>, target: LaneTarget) {
    let id = crate::machine_lanes::next_lane_id();
    let (sink, mut stream) = socket.split();
    let (tx, rx) = mpsc::unbounded_channel();
    let credit = Arc::new(crate::machine_transport::Credit::default());
    let mut writer = tokio::spawn(write_machine_messages(
        sink,
        rx,
        target.features,
        Some(Arc::clone(&credit)),
    ));
    target.lanes.add(id, tx.clone());
    tracing::info!(machine = %target.machine_id, lane = id, lanes = target.lanes.len(), "Machine bulk lane opened");
    let mut decoder = crate::machine_transport::Decoder::new(target.features);
    let route = ReplyRoute {
        primary: target.primary.clone(),
        lanes: Arc::clone(&target.lanes),
    };
    loop {
        let message = tokio::select! {
            message = stream.next() => message,
            _ = &mut writer => break,
        };
        let Some(Ok(message)) = message else { break };
        if !state.machine_control.is_current(&target.connection) {
            break;
        }
        target.lanes.touch(id);
        let text = match message {
            Message::Binary(bytes) => {
                let decoded = decoder
                    .chunk((target.features.0, target.features.1), &bytes)
                    .await;
                if let Some(credit) = decoder.credit() {
                    let _ = tx.send(credit);
                }
                match decoded {
                    Ok(Some(text)) => text,
                    Ok(None) => continue,
                    Err(_) => break,
                }
            }
            Message::Text(text) => {
                if decoder.text(&text).is_err() {
                    break;
                }
                text.to_string()
            }
            Message::Ping(value) => {
                let _ = tx.send(Message::Pong(value));
                continue;
            }
            Message::Pong(value) => {
                credit.observe(&value);
                continue;
            }
            Message::Close(_) => break,
        };
        // A lane carries execution requests only.
        let Ok(crate::machine_protocol::MachineFrame::Runtime {
            frame: crate::runtime_wire::Frame::ExecutionRequest { request },
        }) = serde_json::from_str(&text)
        else {
            break;
        };
        spawn_execution_request(
            &state,
            &target.connection,
            &target.machine_id,
            &target.execution_calls,
            request,
            route.clone(),
        );
    }
    target.lanes.remove(id);
    writer.abort();
    tracing::info!(machine = %target.machine_id, lane = id, lanes = target.lanes.len(), "Machine bulk lane closed");
}
