//! Broker-side routing for unsequenced Antigravity post-edit diagnostic pulls
//! (`Runtime 9`, R4).
//!
//! Guarantees:
//! - Single-mutex routing state (`PostEditRouting`) so controller lease,
//!   pending table, expiration, and teardown are atomic.
//! - Broker mints a globally unique `broker_request_id` (`M15`) before
//!   forwarding to `machine_cli`, restoring the worker's original `request_id`
//!   only on validated reply correlation.
//! - Expired entries are evicted on admission, on scheduled TTL timers, and on
//!   `remove_worker` (`R3-M10`) so lost replies can never exhaust pending slots.
//! - `remove_controller(lease)` tears down ONLY the matching `lease` (`H10`).
//! - Replies and local refusals go ONLY to a worker's bounded low-priority
//!   `diag_reply_tx` (never `worker.tx`) and ONLY when `worker_protocol >= 9`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::machine_broker::wrap_up::{encoded_frame_bytes, unix_socket_has_hint_headroom};
use crate::post_edit_inspect::{
    POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION, PostEditInspectReply, PostEditInspectRequest,
    PostEditScope, clamp_remaining_ms,
};
use crate::runtime_wire::{Frame, write_frame};

pub(super) const WORKER_DIAG_REPLY_CAPACITY: usize = 4;

pub(super) struct NegotiatedPeerHello {
    pub(super) role: crate::runtime_wire::PeerRole,
    pub(super) protocol: u16,
    pub(super) session_id: Option<String>,
    pub(super) worker_epoch: Option<String>,
    pub(super) generation: Option<String>,
    pub(super) executable: Option<String>,
    pub(super) fallback_for: Option<String>,
    pub(super) outer_machine_protocol: Option<u16>,
    pub(super) post_edit_http: bool,
}

pub(super) async fn read_and_negotiate_hello(
    reader: &mut tokio::net::unix::OwnedReadHalf,
    writer: &mut tokio::net::unix::OwnedWriteHalf,
) -> anyhow::Result<Option<NegotiatedPeerHello>> {
    let hello = crate::runtime_wire::read_frame(reader)
        .await?
        .ok_or_else(|| anyhow::anyhow!("peer closed before hello"))?;
    let Frame::Hello {
        role,
        min_protocol,
        max_protocol,
        session_id,
        worker_epoch,
        generation,
        executable,
        fallback_for,
        outer_machine_protocol,
        post_edit_http,
        ..
    } = hello
    else {
        anyhow::bail!("first runtime frame was not hello");
    };
    let Some(protocol) = crate::runtime_wire::negotiate(
        crate::runtime_wire::MIN_PROTOCOL_VERSION,
        crate::runtime_wire::PROTOCOL_VERSION,
        min_protocol,
        max_protocol,
    ) else {
        write_frame(
            writer,
            &Frame::Reject {
                reason: format!(
                    "no protocol overlap: Machine broker {}..={}, peer {min_protocol}..={max_protocol}",
                    crate::runtime_wire::MIN_PROTOCOL_VERSION,
                    crate::runtime_wire::PROTOCOL_VERSION,
                ),
            },
        )
        .await?;
        return Ok(None);
    };
    Ok(Some(NegotiatedPeerHello {
        role,
        protocol,
        session_id,
        worker_epoch,
        generation,
        executable,
        fallback_for,
        outer_machine_protocol,
        post_edit_http,
    }))
}

pub(super) async fn run_core_peer(
    broker: Arc<super::Broker>,
    hello: NegotiatedPeerHello,
    mut reader: tokio::net::unix::OwnedReadHalf,
    writer: tokio::net::unix::OwnedWriteHalf,
) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::unbounded_channel::<Frame>();
    let (diag_req_tx, diag_req_rx) =
        mpsc::channel::<QueuedControllerPostEdit>(CONTROLLER_DIAG_REQ_CAPACITY);
    let lease =
        broker.attach_controller_with_outer(hello.protocol, hello.outer_machine_protocol, &tx);
    broker
        .post_edit
        .attach_controller(lease, hello.protocol, hello.post_edit_http, diag_req_tx);
    let writer_task = tokio::spawn(run_controller_unix_writer(
        writer,
        rx,
        diag_req_rx,
        Arc::clone(&broker.post_edit),
        lease,
    ));
    let core_result = super::handle_core(Arc::clone(&broker), lease, &mut reader).await;
    broker.remove_controller(lease);
    drop(tx);
    writer_task.abort();
    core_result
}

pub(super) fn start_worker_writer(
    writer: tokio::net::unix::OwnedWriteHalf,
) -> (
    mpsc::UnboundedSender<Frame>,
    mpsc::Sender<Frame>,
    mpsc::Sender<Frame>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = mpsc::unbounded_channel::<Frame>();
    let (hint_tx, hint_rx) = mpsc::channel::<Frame>(super::wrap_up::WORKER_HINT_CAPACITY);
    let (diag_reply_tx, diag_reply_rx) = mpsc::channel::<Frame>(WORKER_DIAG_REPLY_CAPACITY);
    let writer_task = tokio::spawn(super::wrap_up::run_worker_unix_writer(
        writer,
        rx,
        hint_rx,
        diag_reply_rx,
    ));
    (tx, hint_tx, diag_reply_tx, writer_task)
}

pub(super) fn handle_worker_request(
    broker: &super::Broker,
    session_id: &str,
    epoch: &str,
    connection_id: u64,
    request: PostEditInspectRequest,
) {
    let worker_info = broker
        .workers
        .lock()
        .get(session_id)
        .filter(|w| w.connection_id == connection_id && w.epoch == epoch)
        .and_then(|w| {
            w.diag_reply_tx
                .clone()
                .map(|tx| (w.negotiated_protocol, tx))
        });
    let Some((worker_protocol, diag_reply_tx)) = worker_info else {
        return;
    };
    let binding = (!broker.cancelled_sessions.lock().contains(session_id))
        .then(|| {
            broker
                .sessions
                .lock()
                .get(session_id)
                .and_then(|launch| launch.execution_binding.as_ref())
                .and_then(|binding| binding.decode().ok())
        })
        .flatten();
    match binding {
        Some(b)
            if request.session_id == session_id
                && request.worker_epoch == epoch
                && request
                    .scope
                    .matches_parts(&b.id, b.revision, &b.environment.incarnation)
                && b.managed.is_none() =>
        {
            broker.post_edit.admit_from_worker(
                request,
                connection_id,
                worker_protocol,
                &diag_reply_tx,
            );
        }
        Some(b) if b.managed.is_some() => {
            try_send_worker_refusal(
                worker_protocol,
                &diag_reply_tx,
                request.session_id,
                request.worker_epoch,
                request.request_id,
                request.scope,
                "restricted",
            );
        }
        _ => {
            try_send_worker_refusal(
                worker_protocol,
                &diag_reply_tx,
                request.session_id,
                request.worker_epoch,
                request.request_id,
                request.scope,
                "unavailable",
            );
        }
    }
}
pub(super) const CONTROLLER_DIAG_REQ_CAPACITY: usize = 8;
pub(super) const BROKER_MAX_PENDING: usize = 4;

pub(super) struct QueuedControllerPostEdit {
    pub(super) broker_request_id: String,
    pub(super) controller_lease: u64,
    pub(super) expires_at: Instant,
    pub(super) frame: Frame,
}

struct ActiveController {
    lease: u64,
    protocol: u16,
    post_edit_http: bool,
    diag_tx: mpsc::Sender<QueuedControllerPostEdit>,
}

struct PendingInspect {
    session_id: String,
    connection_id: u64,
    worker_epoch: String,
    scope: PostEditScope,
    worker_request_id: String,
    worker_protocol: u16,
    worker_diag_reply_tx: mpsc::Sender<Frame>,
    controller_lease: u64,
    expires_at: Instant,
}

struct PostEditRouting {
    active_controller: Option<ActiveController>,
    pending: HashMap<String, PendingInspect>,
    next_seq: u64,
    nonce: u64,
}

impl Default for PostEditRouting {
    fn default() -> Self {
        Self {
            active_controller: None,
            pending: HashMap::new(),
            next_seq: 0,
            nonce: rand::random::<u64>(),
        }
    }
}

#[derive(Default)]
pub(super) struct PostEditState {
    inner: Mutex<PostEditRouting>,
}

fn try_send_worker_refusal(
    worker_protocol: u16,
    worker_diag_reply_tx: &mpsc::Sender<Frame>,
    session_id: String,
    worker_epoch: String,
    request_id: String,
    scope: PostEditScope,
    reason: &'static str,
) {
    if worker_protocol < POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION {
        return;
    }
    let reply = PostEditInspectReply::refused(session_id, worker_epoch, request_id, scope, reason);
    let _ = worker_diag_reply_tx.try_send(Frame::PostEditInspectReply {
        reply: Box::new(reply),
    });
}

fn expire_stale_locked(state: &mut PostEditRouting, now: Instant) {
    state.pending.retain(|_, entry| entry.expires_at > now);
}

impl PostEditState {
    pub(super) fn attach_controller(
        &self,
        lease: u64,
        protocol: u16,
        post_edit_http: bool,
        diag_tx: mpsc::Sender<QueuedControllerPostEdit>,
    ) {
        let mut stale = Vec::new();
        {
            let mut state = self.inner.lock();
            expire_stale_locked(&mut state, Instant::now());
            state.active_controller = Some(ActiveController {
                lease,
                protocol,
                post_edit_http,
                diag_tx,
            });
            let drop_keys: Vec<String> = state
                .pending
                .iter()
                .filter(|(_, entry)| entry.controller_lease != lease)
                .map(|(k, _)| k.clone())
                .collect();
            for key in drop_keys {
                if let Some(entry) = state.pending.remove(&key) {
                    stale.push(entry);
                }
            }
        }
        for entry in stale {
            try_send_worker_refusal(
                entry.worker_protocol,
                &entry.worker_diag_reply_tx,
                entry.session_id,
                entry.worker_epoch,
                entry.worker_request_id,
                entry.scope,
                "unavailable",
            );
        }
    }

    pub(super) fn remove_controller(&self, lease: u64) {
        let mut orphaned = Vec::new();
        {
            let mut state = self.inner.lock();
            if state
                .active_controller
                .as_ref()
                .is_some_and(|current| current.lease == lease)
            {
                state.active_controller = None;
            }
            let drop_keys: Vec<String> = state
                .pending
                .iter()
                .filter(|(_, entry)| entry.controller_lease == lease)
                .map(|(k, _)| k.clone())
                .collect();
            for key in drop_keys {
                if let Some(entry) = state.pending.remove(&key) {
                    orphaned.push(entry);
                }
            }
        }
        for entry in orphaned {
            try_send_worker_refusal(
                entry.worker_protocol,
                &entry.worker_diag_reply_tx,
                entry.session_id,
                entry.worker_epoch,
                entry.worker_request_id,
                entry.scope,
                "unavailable",
            );
        }
    }

    pub(super) fn remove_worker(&self, session_id: &str, connection_id: u64) {
        self.inner.lock().pending.retain(|_, entry| {
            !(entry.session_id == session_id && entry.connection_id == connection_id)
        });
    }

    pub(super) fn expire_one(&self, broker_request_id: &str) {
        let mut state = self.inner.lock();
        let now = Instant::now();
        if state
            .pending
            .get(broker_request_id)
            .is_some_and(|entry| entry.expires_at <= now)
        {
            state.pending.remove(broker_request_id);
        }
    }

    /// Admit a validated worker request, returning `(broker_request_id, ttl)`
    /// when forwarded to `machine_cli` so the caller can arm a bounded expiry
    /// timer (`R3-M10`).
    pub(super) fn admit_from_worker(
        self: &Arc<Self>,
        mut request: PostEditInspectRequest,
        connection_id: u64,
        worker_protocol: u16,
        worker_diag_reply_tx: &mpsc::Sender<Frame>,
    ) {
        if !request.validate() {
            try_send_worker_refusal(
                worker_protocol,
                worker_diag_reply_tx,
                request.session_id,
                request.worker_epoch,
                request.request_id,
                request.scope,
                "invalid_request",
            );
            return;
        }
        let Some(clamped_ms) = clamp_remaining_ms(request.remaining_ms) else {
            try_send_worker_refusal(
                worker_protocol,
                worker_diag_reply_tx,
                request.session_id,
                request.worker_epoch,
                request.request_id,
                request.scope,
                "timeout",
            );
            return;
        };

        let now = Instant::now();
        let expires_at = now + Duration::from_millis(clamped_ms);
        let worker_request_id = request.request_id.clone();

        let (broker_request_id, refusal_reason) = {
            let mut state = self.inner.lock();
            expire_stale_locked(&mut state, now);
            if let Some((controller_lease, controller_protocol, post_edit_http, diag_tx)) = state
                .active_controller
                .as_ref()
                .map(|c| (c.lease, c.protocol, c.post_edit_http, c.diag_tx.clone()))
            {
                if controller_protocol < POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION
                    || !post_edit_http
                {
                    (None, Some("unsupported"))
                } else if state.pending.len() >= BROKER_MAX_PENDING {
                    (None, Some("capacity"))
                } else {
                    state.next_seq = state.next_seq.saturating_add(1);
                    let broker_id = format!("bpe-{:016x}-{}", state.nonce, state.next_seq);
                    let session_id = request.session_id.clone();
                    let worker_epoch = request.worker_epoch.clone();
                    let scope = request.scope.clone();
                    request.request_id = broker_id.clone();
                    request.remaining_ms = clamped_ms;
                    let queued = QueuedControllerPostEdit {
                        broker_request_id: broker_id.clone(),
                        controller_lease,
                        expires_at,
                        frame: Frame::PostEditInspectRequest {
                            request: Box::new(request.clone()),
                        },
                    };
                    if diag_tx.try_send(queued).is_ok() {
                        state.pending.insert(
                            broker_id.clone(),
                            PendingInspect {
                                session_id,
                                connection_id,
                                worker_epoch,
                                scope,
                                worker_request_id: worker_request_id.clone(),
                                worker_protocol,
                                worker_diag_reply_tx: worker_diag_reply_tx.clone(),
                                controller_lease,
                                expires_at,
                            },
                        );
                        (Some(broker_id), None)
                    } else {
                        (None, Some("capacity"))
                    }
                }
            } else {
                (None, Some("unavailable"))
            }
        };

        if let Some(reason) = refusal_reason {
            try_send_worker_refusal(
                worker_protocol,
                worker_diag_reply_tx,
                request.session_id,
                request.worker_epoch,
                worker_request_id,
                request.scope,
                reason,
            );
            return;
        }

        if let Some(broker_id) = broker_request_id {
            let state = Arc::clone(self);
            tokio::spawn(async move {
                tokio::time::sleep_until(tokio::time::Instant::from_std(expires_at)).await;
                state.expire_one(&broker_id);
            });
        }
    }

    pub(super) fn prepare_controller_send(
        &self,
        item: &mut QueuedControllerPostEdit,
        lease: u64,
    ) -> bool {
        let mut state = self.inner.lock();
        let now = Instant::now();
        expire_stale_locked(&mut state, now);
        if item.controller_lease != lease
            || state
                .active_controller
                .as_ref()
                .is_none_or(|c| c.lease != lease)
        {
            state.pending.remove(&item.broker_request_id);
            return false;
        }
        let Some(entry) = state.pending.get(&item.broker_request_id) else {
            return false;
        };
        if entry.controller_lease != lease {
            return false;
        }
        let remaining_ms = item.expires_at.saturating_duration_since(now).as_millis();
        let Some(clamped) = clamp_remaining_ms(u64::try_from(remaining_ms).unwrap_or(u64::MAX))
        else {
            state.pending.remove(&item.broker_request_id);
            return false;
        };
        if let Frame::PostEditInspectRequest { request } = &mut item.frame {
            request.remaining_ms = clamped;
            true
        } else {
            false
        }
    }

    pub(super) fn drop_controller_send(&self, broker_request_id: &str) {
        let entry = self.inner.lock().pending.remove(broker_request_id);
        if let Some(entry) = entry {
            try_send_worker_refusal(
                entry.worker_protocol,
                &entry.worker_diag_reply_tx,
                entry.session_id,
                entry.worker_epoch,
                entry.worker_request_id,
                entry.scope,
                "capacity",
            );
        }
    }

    pub(super) fn route_reply_from_controller(&self, lease: u64, mut reply: PostEditInspectReply) {
        if !reply.validate() {
            return;
        }
        let matched = {
            let mut state = self.inner.lock();
            let now = Instant::now();
            expire_stale_locked(&mut state, now);
            let valid = state.pending.get(&reply.request_id).is_some_and(|entry| {
                entry.controller_lease == lease
                    && entry.session_id == reply.session_id
                    && entry.worker_epoch == reply.worker_epoch
                    && entry.scope == reply.scope
                    && entry.expires_at > now
            });
            if !valid {
                return;
            }
            state.pending.remove(&reply.request_id)
        };
        if let Some(entry) = matched
            && entry.worker_protocol >= POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION
        {
            reply.request_id = entry.worker_request_id;
            let _ = entry
                .worker_diag_reply_tx
                .try_send(Frame::PostEditInspectReply {
                    reply: Box::new(reply),
                });
        }
    }
}

pub(super) async fn run_controller_unix_writer(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    mut reliable_rx: mpsc::UnboundedReceiver<Frame>,
    mut diag_rx: mpsc::Receiver<QueuedControllerPostEdit>,
    post_edit: Arc<PostEditState>,
    lease: u64,
) {
    let mut diag_open = true;
    loop {
        enum Next {
            Reliable(Frame),
            Diag(QueuedControllerPostEdit),
        }
        let next = if diag_open {
            tokio::select! {
                biased;
                frame = reliable_rx.recv() => match frame {
                    Some(frame) => Next::Reliable(frame),
                    None => break,
                },
                item = diag_rx.recv() => match item {
                    Some(item) => Next::Diag(item),
                    None => {
                        diag_open = false;
                        continue;
                    }
                },
            }
        } else {
            match reliable_rx.recv().await {
                Some(frame) => Next::Reliable(frame),
                None => break,
            }
        };
        match next {
            Next::Reliable(frame) => {
                if write_frame(&mut writer, &frame).await.is_err() {
                    break;
                }
            }
            Next::Diag(mut item) => {
                if !post_edit.prepare_controller_send(&mut item, lease) {
                    continue;
                }
                let Some(frame_bytes) = encoded_frame_bytes(&item.frame) else {
                    post_edit.drop_controller_send(&item.broker_request_id);
                    continue;
                };
                if !unix_socket_has_hint_headroom(&writer, frame_bytes) {
                    post_edit.drop_controller_send(&item.broker_request_id);
                    continue;
                }
                if write_frame(&mut writer, &item.frame).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post_edit_inspect::PostEditInspectOutcome;

    fn sample_req(id: &str, remaining_ms: u64) -> PostEditInspectRequest {
        PostEditInspectRequest {
            session_id: "sess-1".into(),
            worker_epoch: "epoch-1".into(),
            request_id: id.into(),
            turn_id: "turn-1".into(),
            scope: PostEditScope {
                binding_id: "bind-1".into(),
                revision: 1,
                incarnation: "inc-1".into(),
            },
            path: "src/lib.rs".into(),
            expected_lf_sha256: "a".repeat(64),
            remaining_ms,
            canary_inspect_id: None,
        }
    }

    #[tokio::test]
    async fn mints_broker_id_correlates_reply_and_scopes_controller_teardown_by_lease() {
        let state = Arc::new(PostEditState::default());
        let (ctrl1_tx, mut ctrl1_rx) = mpsc::channel(CONTROLLER_DIAG_REQ_CAPACITY);
        let (ctrl2_tx, _ctrl2_rx) = mpsc::channel(CONTROLLER_DIAG_REQ_CAPACITY);
        let (worker_tx, mut worker_rx) = mpsc::channel(WORKER_DIAG_REPLY_CAPACITY);

        state.attach_controller(1, 9, true, ctrl1_tx);
        state.admit_from_worker(sample_req("wpe-1", 4000), 10, 9, &worker_tx);

        let mut queued = ctrl1_rx.recv().await.unwrap();
        assert_ne!(queued.broker_request_id, "wpe-1");
        assert!(queued.broker_request_id.starts_with("bpe-"));
        assert!(state.prepare_controller_send(&mut queued, 1));

        let Frame::PostEditInspectRequest { request } = queued.frame else {
            panic!("expected request");
        };
        assert_eq!(request.request_id, queued.broker_request_id);

        // Wrong lease reply is ignored.
        state.route_reply_from_controller(
            2,
            PostEditInspectReply {
                session_id: request.session_id.clone(),
                worker_epoch: request.worker_epoch.clone(),
                request_id: request.request_id.clone(),
                scope: request.scope.clone(),
                outcome: PostEditInspectOutcome::Projected {
                    payload: serde_json::json!({"deliver": true}),
                    matches_expected: true,
                },
            },
        );
        assert!(worker_rx.try_recv().is_err());

        // Matching lease reply restores worker's original request_id.
        state.route_reply_from_controller(
            1,
            PostEditInspectReply {
                session_id: request.session_id,
                worker_epoch: request.worker_epoch,
                request_id: request.request_id,
                scope: request.scope,
                outcome: PostEditInspectOutcome::Projected {
                    payload: serde_json::json!({"deliver": true}),
                    matches_expected: true,
                },
            },
        );
        let Frame::PostEditInspectReply { reply } = worker_rx.recv().await.unwrap() else {
            panic!("expected reply");
        };
        assert_eq!(reply.request_id, "wpe-1");

        // Replacing controller lease 1 with lease 2 and then calling remove_controller(1)
        // must NOT tear down lease 2 (H10).
        state.attach_controller(2, 9, true, ctrl2_tx);
        state.remove_controller(1);
        assert!(
            state
                .inner
                .lock()
                .active_controller
                .as_ref()
                .is_some_and(|c| c.lease == 2)
        );
    }

    #[tokio::test]
    async fn lost_replies_expire_automatically_and_never_exhaust_pending_slots() {
        let state = Arc::new(PostEditState::default());
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(CONTROLLER_DIAG_REQ_CAPACITY);
        let (worker_tx, mut worker_rx) = mpsc::channel(WORKER_DIAG_REPLY_CAPACITY);
        state.attach_controller(1, 9, true, ctrl_tx);

        for i in 0..BROKER_MAX_PENDING {
            state.admit_from_worker(sample_req(&format!("wpe-{i}"), 260), 10, 9, &worker_tx);
            let _ = ctrl_rx.recv().await.unwrap();
        }
        // 5th concurrent request hits capacity.
        state.admit_from_worker(sample_req("wpe-cap", 1000), 10, 9, &worker_tx);
        let Frame::PostEditInspectReply { reply } = worker_rx.recv().await.unwrap() else {
            panic!("expected capacity refusal");
        };
        assert_eq!(
            reply.outcome,
            PostEditInspectOutcome::Refused {
                reason: "capacity".into()
            }
        );

        // After TTL, lost entries expire automatically without any new admission.
        tokio::time::sleep(Duration::from_millis(320)).await;
        assert!(state.inner.lock().pending.is_empty());

        // Subsequent request is admitted cleanly.
        state.admit_from_worker(sample_req("wpe-after", 1000), 10, 9, &worker_tx);
        assert!(ctrl_rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn legacy_worker_protocol_never_receives_post_edit_frames() {
        let state = Arc::new(PostEditState::default());
        let (worker_tx, mut worker_rx) = mpsc::channel(WORKER_DIAG_REPLY_CAPACITY);
        // No controller attached, worker protocol 8 -> zero frames emitted.
        state.admit_from_worker(sample_req("wpe-legacy", 1000), 10, 8, &worker_tx);
        assert!(worker_rx.try_recv().is_err());
    }
}
