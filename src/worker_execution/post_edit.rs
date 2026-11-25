//! Worker-side Antigravity post-edit diagnostic pull state (R4).
//!
//! Keeps diagnostic pull requests completely isolated from the execution
//! ledger, `RelayGuard`, `Client::frames()`, and reliable broker traffic:
//! - Waiters are registered BEFORE bounded enqueue (`WaiterGuard` removes on Drop).
//! - Requests are bound to `(broker_connection_id, turn_id, unit_generation, expires_at)`.
//! - `prepare_send` re-verifies all invariants and clamps remaining deadline
//!   downward right before byte 0 (never rounding upward).

use std::collections::HashMap;
use std::os::fd::AsRawFd as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::account_sandbox::RelayQuiet;
use crate::post_edit_inspect::{
    POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION, PostEditInspectOutcome, PostEditInspectReply,
    PostEditInspectRequest, PostEditScope, clamp_remaining_ms, valid_inspect_id,
    valid_relative_posix_path, valid_sha256_hex,
};
use crate::runtime_wire::Frame;

const BROKER_QUEUE_CAPACITY: usize = 4;
const MIN_UNIX_HEADROOM_BYTES: usize = 32_768;

#[derive(Clone, Debug)]
struct ActiveTurn {
    turn_id: String,
    nonce: String,
}

struct BrokerLink {
    connection_id: u64,
    protocol: u16,
    tx: mpsc::Sender<QueuedPostEdit>,
}

struct PendingEntry {
    waiter: oneshot::Sender<PostEditInspectReply>,
    broker_connection_id: u64,
    turn_id: String,
    unit_generation: u64,
    expires_at: Instant,
}

/// Owns one worker-to-broker post-edit diagnostic registration, detaching it
/// automatically on drop and writing queued requests with socket-headroom checks.
pub struct PostEditBrokerSession {
    execution: Option<std::sync::Arc<super::Client>>,
    conn_id: u64,
    rx: Option<mpsc::Receiver<QueuedPostEdit>>,
}

impl Drop for PostEditBrokerSession {
    fn drop(&mut self) {
        if let Some(execution) = &self.execution {
            execution.detach_post_edit_broker(self.conn_id);
        }
    }
}

impl PostEditBrokerSession {
    #[must_use]
    pub fn attach(execution: Option<std::sync::Arc<super::Client>>, peer_protocol: u16) -> Self {
        let (conn_id, rx) = match &execution {
            Some(exec) => {
                let (conn_id, rx) = exec.attach_post_edit_broker(peer_protocol);
                (conn_id, Some(rx))
            }
            None => (0, None),
        };
        Self {
            execution,
            conn_id,
            rx,
        }
    }

    pub fn complete_reply(&self, reply: PostEditInspectReply) {
        if let Some(execution) = &self.execution {
            execution.complete_post_edit_reply(reply, self.conn_id);
        }
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.rx.is_some()
    }

    pub async fn recv(&mut self) -> Option<QueuedPostEdit> {
        match self.rx.as_mut() {
            Some(rx) => rx.recv().await,
            None => std::future::pending().await,
        }
    }

    pub fn close_rx(&mut self) {
        self.rx = None;
    }

    pub async fn send_or_refuse(
        &self,
        mut item: QueuedPostEdit,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
    ) -> anyhow::Result<()> {
        if let Some(execution) = &self.execution
            && execution.prepare_post_edit_send(&mut item, self.conn_id)
        {
            if let Ok(encoded) = serde_json::to_vec(&item.frame)
                && unix_socket_has_post_edit_headroom(writer, encoded.len() + 4)
            {
                crate::runtime_wire::write_frame(writer, &item.frame).await?;
            } else {
                execution.refuse_post_edit_local(&item.request_id, "capacity");
            }
        }
        Ok(())
    }
}

pub struct QueuedPostEdit {
    pub request_id: String,
    pub broker_connection_id: u64,
    pub turn_id: String,
    pub unit_generation: u64,
    pub expires_at: Instant,
    pub frame: Frame,
}

#[derive(Default)]
pub(crate) struct State {
    next_conn_id: AtomicU64,
    broker: Mutex<Option<BrokerLink>>,
    active_turn: Mutex<Option<ActiveTurn>>,
    pending: Mutex<HashMap<String, PendingEntry>>,
}

struct WaiterGuard<'a> {
    state: &'a State,
    request_id: String,
}

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        self.state.pending.lock().remove(&self.request_id);
    }
}

impl State {
    pub(crate) fn attach_broker(&self, protocol: u16) -> (u64, mpsc::Receiver<QueuedPostEdit>) {
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel(BROKER_QUEUE_CAPACITY);
        *self.broker.lock() = Some(BrokerLink {
            connection_id: conn_id,
            protocol,
            tx,
        });
        // Drop any waiters belonging to an older broker connection.
        self.pending
            .lock()
            .retain(|_, entry| entry.broker_connection_id == conn_id);
        (conn_id, rx)
    }

    pub(crate) fn detach_broker(&self, conn_id: u64) {
        let mut broker = self.broker.lock();
        if broker
            .as_ref()
            .is_some_and(|link| link.connection_id == conn_id)
        {
            *broker = None;
        }
        drop(broker);
        self.pending
            .lock()
            .retain(|_, entry| entry.broker_connection_id != conn_id);
    }

    pub(crate) fn set_active_turn(&self, turn: Option<(String, String)>) {
        let next = turn.map(|(turn_id, nonce)| ActiveTurn { turn_id, nonce });
        let current_id = next.as_ref().map(|t| t.turn_id.clone());
        *self.active_turn.lock() = next;
        self.pending
            .lock()
            .retain(|_, entry| current_id.as_deref() == Some(entry.turn_id.as_str()));
    }

    /// Re-verify every invariant immediately before writing byte 0 to the
    /// broker Unix socket, recomputing `remaining_ms` downward without ever
    /// rounding up. Returns `false` if the queued item must be dropped.
    pub(crate) fn prepare_send(
        &self,
        item: &mut QueuedPostEdit,
        current_conn_id: u64,
        relays: &RelayQuiet,
    ) -> bool {
        if item.broker_connection_id != current_conn_id
            || relays.draining()
            || relays.drain_generation() != item.unit_generation
        {
            self.pending.lock().remove(&item.request_id);
            return false;
        }
        let active_matches = self
            .active_turn
            .lock()
            .as_ref()
            .is_some_and(|turn| turn.turn_id == item.turn_id);
        if !active_matches {
            self.pending.lock().remove(&item.request_id);
            return false;
        }
        let remaining_ms = item
            .expires_at
            .saturating_duration_since(Instant::now())
            .as_millis();
        let Some(clamped) = clamp_remaining_ms(u64::try_from(remaining_ms).unwrap_or(u64::MAX))
        else {
            self.pending.lock().remove(&item.request_id);
            return false;
        };
        let waiter_live = self.pending.lock().get(&item.request_id).is_some_and(|e| {
            e.broker_connection_id == current_conn_id
                && e.turn_id == item.turn_id
                && e.unit_generation == item.unit_generation
                && !e.waiter.is_closed()
        });
        if !waiter_live {
            self.pending.lock().remove(&item.request_id);
            return false;
        }
        if let Frame::PostEditInspectRequest { request } = &mut item.frame {
            request.remaining_ms = clamped;
            true
        } else {
            false
        }
    }

    pub(crate) fn refuse_local(&self, request_id: &str, reason: &str) {
        if let Some(entry) = self.pending.lock().remove(request_id) {
            let _ = entry.waiter.send(PostEditInspectReply {
                session_id: String::new(),
                worker_epoch: String::new(),
                request_id: request_id.to_owned(),
                scope: PostEditScope {
                    binding_id: String::new(),
                    revision: 0,
                    incarnation: String::new(),
                },
                outcome: PostEditInspectOutcome::Refused {
                    reason: reason.to_owned(),
                },
            });
        }
    }

    pub(crate) fn complete_reply(
        &self,
        reply: PostEditInspectReply,
        expected_session: &str,
        expected_epoch: &str,
        expected_scope: &PostEditScope,
        current_conn_id: u64,
        relays: &RelayQuiet,
    ) {
        if reply.session_id != expected_session
            || reply.worker_epoch != expected_epoch
            || &reply.scope != expected_scope
            || !reply.validate()
        {
            return;
        }
        let active_turn_id = self.active_turn.lock().as_ref().map(|t| t.turn_id.clone());
        let mut pending = self.pending.lock();
        let valid = pending.get(&reply.request_id).is_some_and(|entry| {
            entry.broker_connection_id == current_conn_id
                && active_turn_id.as_deref() == Some(entry.turn_id.as_str())
                && !relays.draining()
                && relays.drain_generation() == entry.unit_generation
                && Instant::now() < entry.expires_at
        });
        if !valid {
            return;
        }
        if let Some(entry) = pending.remove(&reply.request_id) {
            let _ = entry.waiter.send(reply);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn inspect(
        &self,
        session_id: &str,
        worker_epoch: &str,
        scope: PostEditScope,
        managed: bool,
        unit_generation: u64,
        relays: &RelayQuiet,
        params: &Value,
    ) -> Value {
        if managed {
            return refused("restricted");
        }
        if relays.draining() || relays.drain_generation() != unit_generation {
            return refused("unavailable");
        }
        let Some(path) = params.get("path").and_then(Value::as_str) else {
            return refused("invalid_request");
        };
        let Some(expected_lf_sha256) = params.get("expectedLfSha256").and_then(Value::as_str)
        else {
            return refused("invalid_request");
        };
        let Some(turn_nonce) = params.get("turnNonce").and_then(Value::as_str) else {
            return refused("invalid_request");
        };
        let Some(timeout_ms) = params.get("timeoutMs").and_then(Value::as_u64) else {
            return refused("invalid_request");
        };
        let canary_inspect_id = match params.get("canaryInspectId") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) if valid_inspect_id(id) => Some(id.clone()),
            _ => return refused("invalid_request"),
        };
        if !valid_relative_posix_path(path) || !valid_sha256_hex(expected_lf_sha256) {
            return refused("invalid_request");
        }
        let Some(remaining_ms) = clamp_remaining_ms(timeout_ms) else {
            return refused("timeout");
        };
        let Some(turn_id) = self
            .active_turn
            .lock()
            .as_ref()
            .filter(|turn| constant_time_eq(turn.nonce.as_bytes(), turn_nonce.as_bytes()))
            .map(|turn| turn.turn_id.clone())
        else {
            return refused("unavailable");
        };
        let Some((conn_id, tx)) = self.broker.lock().as_ref().and_then(|link| {
            (link.protocol >= POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION)
                .then(|| (link.connection_id, link.tx.clone()))
        }) else {
            return refused("unsupported");
        };

        let request_id = format!("wpe-{:032x}", rand::random::<u128>());
        let expires_at = Instant::now() + Duration::from_millis(remaining_ms);
        let request = PostEditInspectRequest {
            session_id: session_id.to_owned(),
            worker_epoch: worker_epoch.to_owned(),
            request_id: request_id.clone(),
            turn_id: turn_id.clone(),
            scope,
            path: path.to_owned(),
            expected_lf_sha256: expected_lf_sha256.to_owned(),
            remaining_ms,
            canary_inspect_id,
        };
        if !request.validate() {
            return refused("invalid_request");
        }

        let (waiter_tx, waiter_rx) = oneshot::channel();
        // Register waiter BEFORE enqueue; WaiterGuard guarantees removal on Drop/cancel.
        self.pending.lock().insert(
            request_id.clone(),
            PendingEntry {
                waiter: waiter_tx,
                broker_connection_id: conn_id,
                turn_id: turn_id.clone(),
                unit_generation,
                expires_at,
            },
        );
        let _guard = WaiterGuard {
            state: self,
            request_id: request_id.clone(),
        };
        let queued = QueuedPostEdit {
            request_id,
            broker_connection_id: conn_id,
            turn_id,
            unit_generation,
            expires_at,
            frame: Frame::PostEditInspectRequest {
                request: Box::new(request),
            },
        };
        if tx.try_send(queued).is_err() {
            return refused("capacity");
        }
        match tokio::time::timeout_at(tokio::time::Instant::from_std(expires_at), waiter_rx).await {
            Ok(Ok(reply)) => match reply.outcome {
                PostEditInspectOutcome::Projected {
                    payload,
                    matches_expected,
                } => json!({
                    "state": "projected",
                    "payload": payload,
                    "matchesExpected": matches_expected,
                }),
                PostEditInspectOutcome::Refused { reason } => json!({
                    "state": "refused",
                    "reason": reason,
                }),
            },
            Ok(Err(_)) => refused("unavailable"),
            Err(_) => refused("timeout"),
        }
    }
}

fn refused(reason: &str) -> Value {
    json!({ "state": "refused", "reason": reason })
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[allow(unsafe_code)]
pub fn unix_socket_has_post_edit_headroom(
    writer: &tokio::net::unix::OwnedWriteHalf,
    frame_bytes: usize,
) -> bool {
    let fd = writer.as_ref().as_raw_fd();
    let mut sndbuf: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            std::ptr::from_mut(&mut sndbuf).cast(),
            &mut len,
        )
    };
    if rc != 0 || sndbuf <= 0 {
        return false;
    }
    let outq: usize = {
        #[cfg(target_os = "linux")]
        {
            let mut queued: libc::c_int = 0;
            let ioctl_rc = unsafe { libc::ioctl(fd, libc::TIOCOUTQ, &mut queued) };
            if ioctl_rc != 0 || queued < 0 {
                return false;
            }
            usize::try_from(queued).unwrap_or(usize::MAX)
        }
        #[cfg(not(target_os = "linux"))]
        {
            0
        }
    };
    let effective_capacity = usize::try_from(sndbuf).unwrap_or(0) / 2;
    effective_capacity
        >= outq
            .saturating_add(frame_bytes)
            .saturating_add(MIN_UNIX_HEADROOM_BYTES)
}
