//! Cooperative drain handling for `WorkerCommand::Drain` and
//! `WorkerCommand::DrainCrashed`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::acp::AgentCommand;
use crate::runtime_wire::{
    CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION, DeliveryControls, Frame, RuntimeEvent,
    StartSession, WorkerSnapshot, WorkerState, write_frame,
};

use super::{Shared, command_ack};

const MAX_REMEMBERED_DECISIONS: usize = 8;

#[derive(Default)]
pub(super) struct State {
    pub(super) draining: AtomicBool,
    pub(super) native_cycle_active: AtomicBool,
    pub(super) pending_event_publishers: std::sync::atomic::AtomicUsize,
    /// Persistent ACP settlement result across broker reconnects and
    /// receipt-write failures: `Some(true)` = clean exit, `Some(false)` = panic.
    agent_settled: Mutex<Option<bool>>,
    /// Bounded recent `(command_id, accepted, reason)` decisions so a replayed
    /// `DrainCrashed` after a lost acceptance/refusal ACK returns the exact
    /// original decision.
    decisions: Mutex<VecDeque<(String, bool, Option<String>)>>,
    accepted_command_id: Mutex<Option<String>>,
}

pub(super) struct EventPublisherGuard {
    shared: std::sync::Arc<Shared>,
}

impl Drop for EventPublisherGuard {
    fn drop(&mut self) {
        self.shared
            .crashed_drain
            .pending_event_publishers
            .fetch_sub(1, Ordering::AcqRel);
        let _ = self.shared.notify.send(());
    }
}

impl State {
    pub(super) fn enter_event_publisher(
        &self,
        shared: &std::sync::Arc<Shared>,
    ) -> EventPublisherGuard {
        self.pending_event_publishers.fetch_add(1, Ordering::AcqRel);
        EventPublisherGuard {
            shared: std::sync::Arc::clone(shared),
        }
    }

    pub(super) fn agent_settled(&self) -> Option<bool> {
        *self.agent_settled.lock()
    }

    pub(super) fn record_agent_settled(&self, clean: bool) {
        let mut slot = self.agent_settled.lock();
        if slot.is_none() {
            *slot = Some(clean);
        }
    }

    fn remembered_decision(&self, command_id: &str) -> Option<(bool, Option<String>)> {
        self.decisions
            .lock()
            .iter()
            .find(|(id, _, _)| id == command_id)
            .map(|(_, accepted, reason)| (*accepted, reason.clone()))
    }

    fn remember_decision(&self, command_id: String, accepted: bool, reason: Option<String>) {
        let mut decisions = self.decisions.lock();
        if decisions.iter().any(|(id, _, _)| id == &command_id) {
            return;
        }
        if decisions.len() >= MAX_REMEMBERED_DECISIONS {
            decisions.pop_front();
        }
        decisions.push_back((command_id, accepted, reason));
    }
}

pub(super) fn initial_snapshot(
    session_id: String,
    worker_epoch: String,
    generation: String,
    executable: Option<String>,
    launch: StartSession,
) -> WorkerSnapshot {
    let agent_session_id = launch.agent_session_id.clone();
    let native_thread_materialized = Some(agent_session_id.is_some());
    WorkerSnapshot {
        session_id,
        worker_epoch,
        generation,
        executable,
        launch: Some(launch),
        state: WorkerState::Starting,
        agent_session_id,
        native_thread_materialized,
        current_turn_id: None,
        supports_current_turn: false,
        delivery_controls: DeliveryControls {
            version: 1,
            next_checkpoint: false,
        },
        wrap_up_steering: false,
        supports_subagent_cancel: true,
        last_runtime_seq: 0,
        pending_permissions: Vec::new(),
        config_options: None,
        context_used: Some(0),
        context_size: Some(0),
        pending_prompt_count: 0,
        drain_requested: false,
        exit_detail: None,
        background_tasks: Some(0),
        native_cycle_id: None,
        native_cycle_active: false,
        incarnation: None,
        account_unit: None,
        uncertain_effect: None,
    }
}

/// Caller MUST hold `shared.outbox.lock()` so suppression is totally ordered
/// with `handle_drain_crashed` (which holds `outbox` -> `snapshot`).
pub(super) fn should_suppress_event_locked(shared: &Shared, event: &RuntimeEvent) -> bool {
    shared.crashed_drain.draining.load(Ordering::Acquire)
        && matches!(
            event,
            RuntimeEvent::Status {
                state: WorkerState::Exited,
                ..
            }
        )
}

pub(super) fn handle_drain(shared: &Shared) -> Frame {
    let idle = {
        let mut snapshot = shared.snapshot.lock();
        snapshot.drain_requested = true;
        snapshot.state == WorkerState::Running
    };
    if idle {
        shared.emit(RuntimeEvent::Status {
            state: WorkerState::Draining,
            detail: Some("waiting for generation handoff".to_owned()),
        });
    }
    command_ack(shared, "drain".to_owned(), true, None)
}

/// Atomically validate that this worker is still an idle `Crashed` owner for
/// `worker_epoch` while holding the canonical `outbox` -> `snapshot` lock
/// order, so a racing ACP `Status::Exited` can never slip in after acceptance.
pub(super) fn handle_drain_crashed(
    shared: &Shared,
    cmd_tx: &mut Option<mpsc::UnboundedSender<AgentCommand>>,
    command_id: String,
    worker_epoch: String,
    acknowledged_uncertain_sequence: u64,
) -> Frame {
    if let Some((accepted, reason)) = shared.crashed_drain.remembered_decision(&command_id) {
        return command_ack(shared, command_id, accepted, reason);
    }
    if shared.seen_commands.lock().contains(&command_id) {
        return command_ack(shared, command_id, true, None);
    }
    let refusal = {
        let _outbox = shared.outbox.lock();
        let mut snapshot = shared.snapshot.lock();
        if snapshot.worker_epoch != worker_epoch {
            Some("crashed worker epoch changed")
        } else if snapshot.state != WorkerState::Crashed {
            Some("worker is not crashed")
        } else if snapshot.current_turn_id.is_some() {
            Some("session is busy")
        } else if snapshot.native_cycle_id.is_some()
            || snapshot.native_cycle_active
            || shared
                .crashed_drain
                .native_cycle_active
                .load(Ordering::Acquire)
        {
            Some("session has active native work")
        } else if !snapshot.pending_permissions.is_empty() {
            Some("session is waiting for a permission decision")
        } else if snapshot.background_tasks.unwrap_or(0) > 0 {
            Some("session has background work in progress")
        } else if snapshot.pending_prompt_count > 0 {
            Some("session has a queued prompt")
        } else if snapshot
            .uncertain_effect
            .as_ref()
            .is_some_and(|effect| effect.sequence.max(1) > acknowledged_uncertain_sequence)
            || shared
                .execution
                .as_ref()
                .and_then(|client| client.ledger())
                .is_some_and(|ledger| {
                    ledger.uncertain_effect().is_some()
                        && ledger.uncertain_sequence().max(1) > acknowledged_uncertain_sequence
                })
        {
            Some("crashed worker has unacknowledged uncertain effect")
        } else {
            snapshot.drain_requested = true;
            shared.crashed_drain.draining.store(true, Ordering::Release);
            None
        }
    };
    if let Some(reason) = refusal {
        let reason = Some(reason.to_owned());
        shared
            .crashed_drain
            .remember_decision(command_id.clone(), false, reason.clone());
        return command_ack(shared, command_id, false, reason);
    }
    shared
        .crashed_drain
        .remember_decision(command_id.clone(), true, None);
    *shared.crashed_drain.accepted_command_id.lock() = Some(command_id.clone());
    shared.mark_command(&command_id);
    if let Some(tx) = cmd_tx.take() {
        shared.forward(&tx, AgentCommand::Cancel);
    }
    command_ack(shared, command_id, true, None)
}

/// If the ACP thread has ended and every outbox event has been acknowledged by
/// the Controller, emit `Frame::CrashedDrainCompleted` ONLY when ACP settled
/// cleanly (`agent_settled_cleanly`, never on panic or forced SIGTERM cutoff).
pub(super) async fn finish_if_drained<W: tokio::io::AsyncWrite + Unpin>(
    shared: &Shared,
    writer: &mut W,
) -> Result<bool> {
    let Some(settled_cleanly) = shared.crashed_drain.agent_settled() else {
        return Ok(false);
    };
    if shared
        .crashed_drain
        .pending_event_publishers
        .load(Ordering::Acquire)
        > 0
        || !shared.outbox.lock().is_empty()
    {
        return Ok(false);
    }
    if settled_cleanly
        && shared.crashed_drain.draining.load(Ordering::Acquire)
        && shared.observation.peer_protocol.load(Ordering::Relaxed)
            >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
    {
        let command_id = shared.crashed_drain.accepted_command_id.lock().clone();
        write_frame(
            writer,
            &Frame::CrashedDrainCompleted {
                session_id: shared.session_id.clone(),
                worker_epoch: shared.worker_epoch.clone(),
                command_id,
            },
        )
        .await?;
    }
    Ok(true)
}
