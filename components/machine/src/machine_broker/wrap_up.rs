//! Broker-side Antigravity wrap-up steering helpers: per-worker reliable-first
//! writer lane, handshake capability sanitization (outer Machine >= 53 AND
//! both runtime legs >= 8), and stateless hint forwarding outside
//! `pending_commands` / `CommandAck`.

use tokio::sync::mpsc;

use crate::runtime_wire::{DeliveryTarget, Frame, WorkerCommand, WorkerSnapshot, write_frame};
use crate::wrap_up::{
    clamp_delivery_capabilities_for_provider, effective_wrap_up_steering,
    wrap_up_hint_within_size_limit,
};

/// Bounded per-worker hint queue capacity from broker to worker UnixStream.
pub(super) const WORKER_HINT_CAPACITY: usize = 8;

/// Run a worker connection's UnixStream writer selecting reliable commands
/// strictly ahead (`biased;`) of bounded drop-on-full hints, always writing
/// complete frames with zero mid-frame cancellation.
pub(super) async fn run_worker_writer<W>(
    mut writer: W,
    mut reliable_rx: mpsc::UnboundedReceiver<Frame>,
    mut hint_rx: mpsc::Receiver<Frame>,
) where
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut hints_open = true;
    loop {
        let frame = if hints_open {
            tokio::select! {
                biased;
                frame = reliable_rx.recv() => match frame {
                    Some(frame) => frame,
                    None => break,
                },
                hint = hint_rx.recv() => match hint {
                    Some(hint) => hint,
                    None => {
                        hints_open = false;
                        continue;
                    }
                },
            }
        } else {
            match reliable_rx.recv().await {
                Some(frame) => frame,
                None => break,
            }
        };
        if write_frame(&mut writer, &frame).await.is_err() {
            break;
        }
    }
}

/// Sanitize a worker snapshot for the currently attached Controller using exact
/// `(controller_outer_protocol, controller_runtime_protocol, worker_protocol)`
/// and broker-verified provider identity (fails closed if provider is missing).
pub(super) fn sanitize_snapshot_for_controller(
    snapshot: &mut WorkerSnapshot,
    raw_wrap_up: bool,
    worker_protocol: u16,
    controller_outer_protocol: Option<u16>,
    controller_runtime_protocol: u16,
    declared_provider: Option<&str>,
) {
    let provider = snapshot
        .launch
        .as_ref()
        .map(|launch| launch.provider.as_str())
        .or(declared_provider);
    clamp_delivery_capabilities_for_provider(
        provider,
        &mut snapshot.supports_current_turn,
        &mut snapshot.delivery_controls.next_checkpoint,
    );
    let min_runtime_protocol = worker_protocol.min(controller_runtime_protocol);
    snapshot.wrap_up_steering = effective_wrap_up_steering(
        provider,
        raw_wrap_up,
        controller_outer_protocol,
        min_runtime_protocol,
        true,
    );
}

/// Sanitize a live `CURRENT_TURN_CAPABILITIES_UPDATE` event before forwarding
/// to the currently attached Controller.
pub(super) fn sanitize_capability_event(
    update: &mut serde_json::Value,
    raw_wrap_up: bool,
    worker_protocol: u16,
    controller_outer_protocol: Option<u16>,
    controller_runtime_protocol: u16,
    declared_provider: Option<&str>,
) {
    let eff = effective_wrap_up_steering(
        declared_provider,
        raw_wrap_up,
        controller_outer_protocol,
        worker_protocol.min(controller_runtime_protocol),
        true,
    );
    if let Some(obj) = update.as_object_mut() {
        if declared_provider.is_some_and(crate::wrap_up::is_antigravity_provider) {
            obj.insert("currentTurn".into(), serde_json::Value::Bool(false));
            obj.insert("nextCheckpoint".into(), serde_json::Value::Bool(false));
        }
        obj.insert("wrapUpSteering".into(), serde_json::Value::Bool(eff));
    }
}

/// Build and enqueue a bounded `WorkerCommand::WrapUpHint` onto a worker's
/// dedicated hint channel when all protocol, provider, target, and dedup
/// invariants hold. Returns `true` iff a new frame was enqueued.
#[allow(clippy::too_many_arguments)]
pub(super) fn try_forward_wrap_up_hint(
    session_id: &str,
    target: &DeliveryTarget,
    worker_epoch: &str,
    worker_snapshot: &WorkerSnapshot,
    raw_wrap_up: bool,
    worker_protocol: u16,
    controller_outer_protocol: Option<u16>,
    controller_runtime_protocol: u16,
    declared_provider: Option<&str>,
    last_hinted: &mut Option<DeliveryTarget>,
    hint_tx: &mpsc::Sender<Frame>,
) -> bool {
    let provider = worker_snapshot
        .launch
        .as_ref()
        .map(|launch| launch.provider.as_str())
        .or(declared_provider);
    if !effective_wrap_up_steering(
        provider,
        raw_wrap_up,
        controller_outer_protocol,
        worker_protocol.min(controller_runtime_protocol),
        true,
    ) {
        return false;
    }
    if worker_epoch != target.worker_epoch
        || worker_snapshot.current_turn_id.as_deref() != Some(target.turn_id.as_str())
        || worker_snapshot.drain_requested
    {
        return false;
    }
    if last_hinted.as_ref() == Some(target) {
        return false;
    }
    let frame = Frame::WorkerCommand {
        session_id: session_id.to_owned(),
        command: WorkerCommand::WrapUpHint {
            session_id: session_id.to_owned(),
            target: target.clone(),
        },
    };
    if !wrap_up_hint_within_size_limit(&frame) {
        return false;
    }
    if hint_tx.try_send(frame).is_ok() {
        *last_hinted = Some(target.clone());
        true
    } else {
        false
    }
}

impl super::Broker {
    #[cfg(test)]
    pub(super) fn attach_controller(
        &self,
        protocol: u16,
        tx: &mpsc::UnboundedSender<Frame>,
    ) -> u64 {
        self.attach_controller_with_outer(
            protocol,
            Some(crate::machine_protocol::MACHINE_PROTOCOL_VERSION),
            tx,
        )
    }

    pub(super) fn sanitized_snapshots(
        &self,
        outer: Option<u16>,
        ctrl_proto: u16,
    ) -> Vec<WorkerSnapshot> {
        let mut welcome = self.snapshots();
        for snapshot in &mut welcome {
            self.sanitize_snapshot_for_controller_protocols(snapshot, outer, ctrl_proto);
        }
        welcome
    }

    pub(super) fn attach_worker_wrap_up(
        &self,
        session_id: &str,
        connection_id: u64,
        protocol: u16,
        hint_tx: mpsc::Sender<Frame>,
    ) {
        if let Some(worker) = self.workers.lock().get_mut(session_id)
            && worker.connection_id == connection_id
        {
            worker.negotiated_protocol = protocol;
            worker.hint_tx = Some(hint_tx);
        }
    }

    pub(super) fn sanitize_snapshot_for_controller_protocols(
        &self,
        worker: &mut WorkerSnapshot,
        outer: Option<u16>,
        ctrl_proto: u16,
    ) {
        let (raw, worker_proto) = self
            .workers
            .lock()
            .get(&worker.session_id)
            .map_or((false, 0), |p| {
                (p.raw_wrap_up_steering, p.negotiated_protocol)
            });
        let declared = self
            .sessions
            .lock()
            .get(&worker.session_id)
            .map(|s| s.provider.clone());
        sanitize_snapshot_for_controller(
            worker,
            raw,
            worker_proto,
            outer,
            ctrl_proto,
            declared.as_deref(),
        );
    }

    pub(super) fn handle_wrap_up_hint(
        &self,
        lease: u64,
        session_id: &str,
        target: &DeliveryTarget,
    ) {
        let Some((outer, ctrl_proto)) = self
            .controller
            .lock()
            .as_ref()
            .filter(|c| c.lease == lease)
            .map(|c| (c.outer_machine_protocol, c.protocol))
        else {
            return;
        };
        let declared = self
            .sessions
            .lock()
            .get(session_id)
            .map(|s| s.provider.clone());
        if let Some(worker) = self.workers.lock().get_mut(session_id)
            && let Some(hint_tx) = worker.hint_tx.as_ref()
        {
            try_forward_wrap_up_hint(
                session_id,
                target,
                &worker.epoch,
                &worker.snapshot,
                worker.raw_wrap_up_steering,
                worker.negotiated_protocol,
                outer,
                ctrl_proto,
                declared.as_deref(),
                &mut worker.last_hinted_target,
                hint_tx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_wire::{DeliveryControls, WorkerState, read_frame};

    fn sample_snapshot(provider: Option<&str>) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: "s1".into(),
            worker_epoch: "e1".into(),
            generation: "g1".into(),
            executable: None,
            launch: provider.map(|p| crate::runtime_wire::StartSession {
                session_id: "s1".into(),
                provider: p.into(),
                provider_version: "0.2.35".into(),
                provider_generation_digest: "sha256:abc".into(),
                provider_account_id: Default::default(),
                provider_auth_generation: None,
                provider_behavior: None,
                cwd: "/tmp".into(),
                generation: "g1".into(),
                agent_session_id: None,
                system: false,
                context_window: None,
                auto_compact_token_limit: None,
                cache_protection: None,
                fallback_for: None,
                adopt_only: false,
                execution_binding: None,
                extensions: None,
                mcp_plan: None,
                native_fork: None,
            }),
            state: WorkerState::Busy,
            agent_session_id: None,
            native_thread_materialized: None,
            current_turn_id: Some("t1".into()),
            supports_current_turn: true,
            delivery_controls: DeliveryControls {
                version: 1,
                next_checkpoint: true,
            },
            wrap_up_steering: true,
            supports_subagent_cancel: false,
            last_runtime_seq: 1,
            pending_permissions: vec![],
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
            exit_detail: None,
            background_tasks: None,
            native_cycle_id: None,
            incarnation: None,
            account_unit: None,
            uncertain_effect: None,
        }
    }

    #[test]
    fn sanitize_fails_closed_on_missing_provider_or_legacy_protocol() {
        let mut s = sample_snapshot(Some("antigravity"));
        sanitize_snapshot_for_controller(&mut s, true, 8, Some(53), 8, None);
        assert!(s.wrap_up_steering);
        assert!(!s.supports_current_turn);
        assert!(!s.delivery_controls.next_checkpoint);

        // Machine 52 + Runtime 8 -> false
        let mut s52 = sample_snapshot(Some("antigravity"));
        sanitize_snapshot_for_controller(&mut s52, true, 8, Some(52), 8, None);
        assert!(!s52.wrap_up_steering);

        // Machine 53 + Runtime 7 -> false
        let mut s7 = sample_snapshot(Some("antigravity"));
        sanitize_snapshot_for_controller(&mut s7, true, 7, Some(53), 8, None);
        assert!(!s7.wrap_up_steering);

        // Missing provider -> fails closed
        let mut missing = sample_snapshot(None);
        sanitize_snapshot_for_controller(&mut missing, true, 8, Some(53), 8, None);
        assert!(!missing.wrap_up_steering);
    }

    #[tokio::test]
    async fn worker_writer_delivers_stop_intact_after_hint_with_zero_stream_corruption() {
        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel();
        let (hint_tx, hint_rx) = mpsc::channel(WORKER_HINT_CAPACITY);
        let target = DeliveryTarget {
            worker_epoch: "e1".into(),
            turn_id: "t1".into(),
        };
        let mut last = None;
        let snap = sample_snapshot(Some("antigravity"));
        assert!(try_forward_wrap_up_hint(
            "s1",
            &target,
            "e1",
            &snap,
            true,
            8,
            Some(53),
            8,
            None,
            &mut last,
            &hint_tx
        ));
        // Duplicate hint for same turn is deduplicated.
        assert!(!try_forward_wrap_up_hint(
            "s1",
            &target,
            "e1",
            &snap,
            true,
            8,
            Some(53),
            8,
            None,
            &mut last,
            &hint_tx
        ));
        reliable_tx
            .send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::Cancel {
                    command_id: "stop-1".into(),
                },
            })
            .unwrap();
        // Small duplex buffer forces partial progress / backpressure while reader pauses.
        let (client, mut server) = tokio::io::duplex(64);
        let writer = tokio::spawn(run_worker_writer(client, reliable_rx, hint_rx));
        let first = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            first,
            Frame::WorkerCommand {
                command: WorkerCommand::Cancel { .. },
                ..
            }
        ));
        let second = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            second,
            Frame::WorkerCommand {
                command: WorkerCommand::WrapUpHint { .. },
                ..
            }
        ));
        drop(reliable_tx);
        drop(hint_tx);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn surviving_busy_r1_worker_reconnects_against_disabled_broker_and_settles_reliably() {
        let mut snap = sample_snapshot(Some("antigravity"));
        sanitize_snapshot_for_controller(&mut snap, true, 8, Some(52), 8, None);
        assert!(!snap.wrap_up_steering);
        assert!(!snap.supports_current_turn);

        let (reliable_tx, reliable_rx) = mpsc::unbounded_channel();
        let (hint_tx, hint_rx) = mpsc::channel(WORKER_HINT_CAPACITY);
        let target = DeliveryTarget {
            worker_epoch: "e1".into(),
            turn_id: "t1".into(),
        };
        let mut last = None;
        assert!(!try_forward_wrap_up_hint(
            "s1",
            &target,
            "e1",
            &snap,
            true,
            8,
            Some(52),
            8,
            None,
            &mut last,
            &hint_tx
        ));
        reliable_tx
            .send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::Permission {
                    command_id: "perm-1".into(),
                    request_id: "req-1".into(),
                    option_id: Some("allow_once".into()),
                },
            })
            .unwrap();
        reliable_tx
            .send(Frame::WorkerCommand {
                session_id: "s1".into(),
                command: WorkerCommand::Cancel {
                    command_id: "cancel-1".into(),
                },
            })
            .unwrap();
        let (client, mut server) = tokio::io::duplex(256);
        let writer = tokio::spawn(run_worker_writer(client, reliable_rx, hint_rx));
        let first = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            first,
            Frame::WorkerCommand {
                command: WorkerCommand::Permission { .. },
                ..
            }
        ));
        let second = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            second,
            Frame::WorkerCommand {
                command: WorkerCommand::Cancel { .. },
                ..
            }
        ));
        drop(reliable_tx);
        drop(hint_tx);
        writer.await.unwrap();
    }

    #[test]
    fn non_antigravity_provider_preserves_current_turn_and_never_advertises_wrap_up() {
        let mut codex = sample_snapshot(Some("codex"));
        sanitize_snapshot_for_controller(&mut codex, true, 8, Some(53), 8, None);
        assert!(!codex.wrap_up_steering);
        assert!(codex.supports_current_turn);
        assert!(codex.delivery_controls.next_checkpoint);
    }
}
