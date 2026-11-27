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
const DRAIN_OUTBOX_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Default)]
pub(super) struct State {
    pub(super) draining: AtomicBool,
    /// True after a drain aborted due to an unacknowledged `uncertain_effect`;
    /// blocks worker exit across subsequent finish checks and reconnects until
    /// a fresh drain covers the sequence or an explicit terminal Stop arrives (R6-H1).
    pub(super) holding_uncertain_effect: AtomicBool,
    /// True once an explicit terminal `WorkerCommand::Stop` arrived (R6-H1).
    pub(super) terminal_stop: AtomicBool,
    /// True once `finish_if_drained` closed publisher admission and committed
    /// `CrashedDrainCompleted` (R6-H2).
    drain_committed: AtomicBool,
    /// Serializes late uncertainty/event publishers with `finish_if_drained` commit (R6-H2).
    pub(super) drain_commit_lock: Mutex<()>,
    pub(super) native_cycle_active: AtomicBool,
    pub(super) pending_event_publishers: std::sync::atomic::AtomicUsize,
    /// Persistent ACP settlement result across broker reconnects and
    /// receipt-write failures: `Some(true)` = clean exit, `Some(false)` = panic.
    agent_settled: Mutex<Option<bool>>,
    /// When the worker began waiting on final outbox flush after ACP settled,
    /// bounding cooperative drain fencing if ACKs are permanently withheld (R8-M2).
    drain_wait_started: Mutex<Option<std::time::Instant>>,
    /// Bounded recent `(command_id, accepted, reason)` decisions so a replayed
    /// `DrainCrashed` after a lost acceptance/refusal ACK returns the exact
    /// original decision.
    decisions: Mutex<VecDeque<(String, bool, Option<String>)>>,
    aborted_commands: Mutex<std::collections::HashSet<String>>,
    /// Every drain command ID ever evaluated (accepted, refused, or aborted) in
    /// this worker epoch so cache rotation of `decisions` never re-evaluates an
    /// old refusal as a fresh command (R15-L2).
    seen_drain_commands: Mutex<std::collections::HashSet<String>>,
    accepted_command_id: Mutex<Option<String>>,
    accepted_ack_seq: std::sync::atomic::AtomicU64,
    /// Pending explicit `CrashedDrainAborted` notice retained across reconnects
    /// until flushed to the broker (R7-M1, R8-M2).
    pending_abort: Mutex<Option<(Option<String>, Option<crate::runtime_wire::UncertainEffect>)>>,
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

    pub(super) fn drain_committed(&self) -> bool {
        self.drain_committed.load(Ordering::Acquire)
    }

    pub(super) fn mark_terminal_stop(&self) {
        self.terminal_stop.store(true, Ordering::Release);
        self.draining.store(false, Ordering::Release);
        self.holding_uncertain_effect
            .store(false, Ordering::Release);
        self.pending_abort.lock().take();
        self.drain_wait_started.lock().take();
    }

    /// Active accepted `DrainCrashed` command id carried on `Frame::Hello` so a
    /// restarted broker installs the drain fence atomically during worker
    /// registration before `Welcome` and `flush_pending` (R7-H4, R7-M6).
    pub(super) fn active_drain_command_id(&self) -> Option<String> {
        if !self.draining.load(Ordering::Acquire) {
            return None;
        }
        self.accepted_command_id.lock().clone()
    }

    pub(super) fn agent_settled(&self) -> Option<bool> {
        *self.agent_settled.lock()
    }

    pub(super) fn record_agent_settled(&self, clean: bool) {
        let mut slot = self.agent_settled.lock();
        if slot.is_none() {
            *slot = Some(clean);
            if self.draining.load(Ordering::Acquire) {
                self.drain_wait_started
                    .lock()
                    .get_or_insert_with(std::time::Instant::now);
            }
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
        self.seen_drain_commands.lock().insert(command_id.clone());
        let mut decisions = self.decisions.lock();
        if decisions.iter().any(|(id, _, _)| id == &command_id) {
            return;
        }
        if decisions.len() >= MAX_REMEMBERED_DECISIONS {
            decisions.pop_front();
        }
        decisions.push_back((command_id, accepted, reason));
    }

    /// Overwrite any cached acceptance for an aborted drain command with the
    /// terminal refusal outcome so a broker reconnect replay never restores
    /// drain authority after abort (R13-M3).
    fn was_aborted(&self, command_id: &str) -> bool {
        self.aborted_commands.lock().contains(command_id)
    }

    fn record_abort_decision(&self, command_id: Option<&str>, reason: &str) {
        *self.accepted_command_id.lock() = None;
        let Some(command_id) = command_id else {
            return;
        };
        self.aborted_commands.lock().insert(command_id.to_owned());
        let mut decisions = self.decisions.lock();
        if let Some((_, accepted, stored_reason)) =
            decisions.iter_mut().find(|(id, _, _)| id == command_id)
        {
            *accepted = false;
            *stored_reason = Some(reason.to_owned());
            return;
        }
        if decisions.len() >= MAX_REMEMBERED_DECISIONS {
            decisions.pop_front();
        }
        decisions.push_back((command_id.to_owned(), false, Some(reason.to_owned())));
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
        crashed_drain_active: false,
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

pub(super) fn apply_status_to_snapshot(
    shared: &Shared,
    snapshot: &mut WorkerSnapshot,
    state: WorkerState,
    detail: &Option<String>,
) {
    snapshot.state = if state == WorkerState::Running && snapshot.drain_requested {
        WorkerState::Draining
    } else {
        state
    };
    snapshot.exit_detail = if matches!(state, WorkerState::Crashed | WorkerState::Exited) {
        detail.clone()
    } else {
        None
    };
    shared.readiness_window(state == WorkerState::Starting);
    if matches!(state, WorkerState::Exited | WorkerState::Crashed) {
        snapshot.current_turn_id = None;
        snapshot.native_cycle_id = None;
    }
    if state == WorkerState::Exited {
        shared
            .crashed_drain
            .native_cycle_active
            .store(false, Ordering::Release);
        snapshot.native_cycle_active = false;
    }
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
    shared: &std::sync::Arc<Shared>,
    cmd_tx: &mut Option<mpsc::UnboundedSender<AgentCommand>>,
    command_id: String,
    worker_epoch: String,
    acknowledged_uncertain_sequence: u64,
) -> Frame {
    if shared.crashed_drain.was_aborted(&command_id) {
        return command_ack(
            shared,
            command_id,
            false,
            Some("crashed_worker_drain_aborted: previous drain aborted".to_owned()),
        );
    }
    if let Some((accepted, reason)) = shared.crashed_drain.remembered_decision(&command_id) {
        return command_ack(shared, command_id, accepted, reason);
    }
    if shared
        .crashed_drain
        .seen_drain_commands
        .lock()
        .contains(&command_id)
        || shared.seen_commands.lock().contains(&command_id)
    {
        // Evicted drain command decisions must fail closed rather than
        // synthesizing a false acceptance after cache rotation (R14-L1).
        return command_ack(
            shared,
            command_id,
            false,
            Some("crashed_worker_drain_decision_evicted: retry with a new command".to_owned()),
        );
    }
    let refusal = {
        let _outbox = shared.outbox.lock();
        let snapshot = shared.snapshot.lock();
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
        } else if has_unacknowledged_uncertain_effect_locked(
            shared,
            &snapshot,
            acknowledged_uncertain_sequence,
        ) {
            Some("crashed worker has unacknowledged uncertain effect")
        } else {
            // Do not latch `snapshot.drain_requested = true` on crashed drain:
            // the broker projects `drain_requested = true` on wire snapshots
            // while active/pending, and keeping it false avoids blocking
            // post-abort prompt flushing in `flush_pending_guarded` (R7-M4).
            shared
                .crashed_drain
                .accepted_ack_seq
                .store(acknowledged_uncertain_sequence, Ordering::Release);
            shared
                .crashed_drain
                .holding_uncertain_effect
                .store(false, Ordering::Release);
            shared.crashed_drain.pending_abort.lock().take();
            if shared.crashed_drain.agent_settled().is_some() {
                *shared.crashed_drain.drain_wait_started.lock() = Some(std::time::Instant::now());
            } else {
                shared.crashed_drain.drain_wait_started.lock().take();
            }
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
    if let Some(execution) = shared.execution.clone() {
        // Settle/fence execution publishers under EventPublisherGuard so
        // finish_if_drained joins them before emitting CrashedDrainCompleted (R6-H2).
        let pub_guard = shared.crashed_drain.enter_event_publisher(shared);
        tokio::spawn(async move {
            let _guard = pub_guard;
            super::execution_launch::settle(execution).await;
        });
    }
    if let Some(tx) = cmd_tx.take() {
        shared.forward(&tx, AgentCommand::Cancel);
    }
    command_ack(shared, command_id, true, None)
}

fn has_unacknowledged_uncertain_effect_locked(
    shared: &Shared,
    snapshot: &WorkerSnapshot,
    acknowledged_uncertain_sequence: u64,
) -> bool {
    snapshot
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
}

/// Restate any retained `Frame::CrashedDrainAborted` on Runtime 10+ reconnect
/// while `holding_uncertain_effect` remains true, so a broker restart or socket
/// drop right after an abort can never strand the broker's drain fence (R7-M1).
/// Accepted active drains are carried on `Frame::Hello.accepted_crashed_drain`
/// instead of a post-handshake frame (R7-H4, R7-M6).
pub(super) async fn send_post_handshake_state<W: tokio::io::AsyncWrite + Unpin>(
    shared: &Shared,
    writer: &mut W,
) -> Result<()> {
    if shared.observation.peer_protocol.load(Ordering::Relaxed)
        < CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
    {
        return Ok(());
    }
    if !shared
        .crashed_drain
        .holding_uncertain_effect
        .load(Ordering::Acquire)
    {
        return Ok(());
    }
    let pending_abort = shared.crashed_drain.pending_abort.lock().clone();
    if let Some((command_id, uncertain_effect)) = pending_abort {
        write_frame(
            writer,
            &Frame::CrashedDrainAborted {
                session_id: shared.session_id.clone(),
                worker_epoch: shared.worker_epoch.clone(),
                command_id,
                uncertain_effect,
            },
        )
        .await?;
    }
    Ok(())
}

pub(super) fn subscribe_ledger_changes(
    shared: &Shared,
) -> Option<tokio::sync::watch::Receiver<u64>> {
    shared
        .execution
        .as_ref()
        .and_then(|execution| execution.ledger())
        .map(|ledger| ledger.subscribe_changes())
}

pub(super) async fn wait_ledger_change(rx: &mut Option<tokio::sync::watch::Receiver<u64>>) -> bool {
    match rx.as_mut() {
        Some(rx) => rx.changed().await.is_ok(),
        None => std::future::pending::<bool>().await,
    }
}

enum CommitDecision {
    NotReady,
    Aborted(
        Option<String>,
        Option<crate::runtime_wire::UncertainEffect>,
        Box<WorkerSnapshot>,
    ),
    CompleteWithReceipt(Option<String>),
    ExitWithoutReceipt,
}

fn decide_drain_commit(shared: &Shared, settled_cleanly: bool) -> CommitDecision {
    let _commit = shared.crashed_drain.drain_commit_lock.lock();
    // Hold `outbox` across both emptiness and `pending_event_publishers` checks
    // through `drain_committed = true` (canonical `drain_commit_lock -> outbox
    // -> snapshot` order) so a deferred `TurnEnded` publisher cannot push an
    // event and drop its guard between the two reads (R9-H1).
    let outbox = shared.outbox.lock();
    let outbox_empty = outbox.is_empty();
    let publishers_in_flight = shared
        .crashed_drain
        .pending_event_publishers
        .load(Ordering::Acquire)
        > 0
        || shared
            .execution
            .as_ref()
            .is_some_and(|client| client.relays().recording_in_flight());
    if shared.crashed_drain.terminal_stop.load(Ordering::Acquire) {
        return if publishers_in_flight || !outbox_empty {
            CommitDecision::NotReady
        } else {
            CommitDecision::ExitWithoutReceipt
        };
    }
    if !outbox_empty {
        let timed_out = shared.crashed_drain.draining.load(Ordering::Acquire)
            && !publishers_in_flight
            && shared
                .crashed_drain
                .drain_wait_started
                .lock()
                .get_or_insert_with(std::time::Instant::now)
                .elapsed()
                >= DRAIN_OUTBOX_FLUSH_TIMEOUT;
        if timed_out {
            let command_id = shared.crashed_drain.accepted_command_id.lock().clone();
            shared.crashed_drain.record_abort_decision(
                command_id.as_deref(),
                "crashed_worker_drain_aborted: outbox flush timed out",
            );
            shared
                .crashed_drain
                .holding_uncertain_effect
                .store(true, Ordering::Release);
            shared
                .crashed_drain
                .draining
                .store(false, Ordering::Release);
            shared.crashed_drain.drain_wait_started.lock().take();
            let snapshot = shared.snapshot.lock().clone();
            let effect = snapshot.uncertain_effect.clone();
            *shared.crashed_drain.pending_abort.lock() = Some((command_id.clone(), effect.clone()));
            return CommitDecision::Aborted(command_id, effect, Box::new(snapshot));
        }
        return CommitDecision::NotReady;
    }
    if publishers_in_flight {
        return CommitDecision::NotReady;
    }
    if !shared.crashed_drain.draining.load(Ordering::Acquire) {
        return CommitDecision::ExitWithoutReceipt;
    }
    let ledger = shared.execution.as_ref().and_then(|client| client.ledger());
    let ledger_effect = match ledger.as_ref() {
        Some(ledger) => match ledger.close_uncertain_reports_and_read() {
            Ok(effect) => effect,
            Err(()) => return CommitDecision::NotReady,
        },
        None => None,
    };
    let ack_seq = shared
        .crashed_drain
        .accepted_ack_seq
        .load(Ordering::Acquire);
    let mut snapshot = shared.snapshot.lock();
    if let Some((method, sequence)) = ledger_effect {
        crate::runtime_wire::merge_uncertain_effect(
            &mut snapshot.uncertain_effect,
            crate::runtime_wire::UncertainEffect { method, sequence },
        );
    }
    if let Some(effect) = snapshot.uncertain_effect.clone()
        && effect.sequence.max(1) > ack_seq
    {
        if let Some(ledger) = ledger.as_ref() {
            ledger.reopen_uncertain_reports();
        }
        let command_id = shared.crashed_drain.accepted_command_id.lock().clone();
        shared.crashed_drain.record_abort_decision(
            command_id.as_deref(),
            "crashed_worker_drain_aborted: unacknowledged uncertain effect",
        );
        shared
            .crashed_drain
            .holding_uncertain_effect
            .store(true, Ordering::Release);
        shared
            .crashed_drain
            .draining
            .store(false, Ordering::Release);
        shared.crashed_drain.drain_wait_started.lock().take();
        *shared.crashed_drain.pending_abort.lock() =
            Some((command_id.clone(), Some(effect.clone())));
        CommitDecision::Aborted(command_id, Some(effect), Box::new(snapshot.clone()))
    } else if settled_cleanly
        && shared.observation.peer_protocol.load(Ordering::Relaxed)
            >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
    {
        shared
            .crashed_drain
            .drain_committed
            .store(true, Ordering::Release);
        CommitDecision::CompleteWithReceipt(shared.crashed_drain.accepted_command_id.lock().clone())
    } else {
        CommitDecision::ExitWithoutReceipt
    }
}

/// If the ACP thread has ended and every outbox event has been acknowledged by
/// the Controller, atomically fence execution uncertainty reports under
/// `drain_commit_lock`, read the ledger's uncertain effect, and either abort
/// (retaining `pending_abort` while holding) or emit `Frame::CrashedDrainCompleted`
/// ONLY when ACP settled cleanly (R7-H1, R7-M1, R7-M5, R8-M2).
pub(super) async fn finish_if_drained<W: tokio::io::AsyncWrite + Unpin>(
    shared: &Shared,
    writer: &mut W,
) -> Result<bool> {
    let Some(settled_cleanly) = shared.crashed_drain.agent_settled() else {
        return Ok(false);
    };
    if shared
        .crashed_drain
        .holding_uncertain_effect
        .load(Ordering::Acquire)
        && !shared.crashed_drain.draining.load(Ordering::Acquire)
        && !shared.crashed_drain.terminal_stop.load(Ordering::Acquire)
    {
        return Ok(false);
    }
    match decide_drain_commit(shared, settled_cleanly) {
        CommitDecision::NotReady => Ok(false),
        CommitDecision::Aborted(command_id, uncertain_effect, snapshot) => {
            if shared.observation.peer_protocol.load(Ordering::Relaxed)
                >= CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION
            {
                write_frame(
                    writer,
                    &Frame::CrashedDrainAborted {
                        session_id: shared.session_id.clone(),
                        worker_epoch: shared.worker_epoch.clone(),
                        command_id,
                        uncertain_effect,
                    },
                )
                .await?;
            }
            write_frame(writer, &Frame::Snapshot { worker: snapshot }).await?;
            Ok(false)
        }
        CommitDecision::CompleteWithReceipt(command_id) => {
            write_frame(
                writer,
                &Frame::CrashedDrainCompleted {
                    session_id: shared.session_id.clone(),
                    worker_epoch: shared.worker_epoch.clone(),
                    command_id,
                },
            )
            .await?;
            Ok(true)
        }
        CommitDecision::ExitWithoutReceipt => Ok(true),
    }
}

#[cfg(test)]
mod r13_tests {
    use super::*;

    #[test]
    fn aborted_drain_command_replay_returns_terminal_refusal_instead_of_cached_acceptance() {
        let state = State::default();
        state.remember_decision("dc-r13".into(), true, None);
        *state.accepted_command_id.lock() = Some("dc-r13".into());
        assert_eq!(state.remembered_decision("dc-r13"), Some((true, None)));

        state.record_abort_decision(
            Some("dc-r13"),
            "crashed_worker_drain_aborted: unacknowledged uncertain effect",
        );
        assert!(state.accepted_command_id.lock().is_none());
        let (accepted, reason) = state
            .remembered_decision("dc-r13")
            .expect("aborted decision recorded");
        assert!(!accepted);
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.starts_with("crashed_worker_drain_aborted:"))
        );
        // Evicting the 8-entry decisions ring buffer must NOT forget the abort (R14-L1).
        for i in 0..16 {
            state.remember_decision(format!("dc-fill-{i}"), false, Some("busy".into()));
        }
        assert!(state.remembered_decision("dc-r13").is_none());
        assert!(state.was_aborted("dc-r13"));
        // Refused command IDs evicted from the 8-entry cache remain in seen_drain_commands (R15-L2).
        assert!(state.remembered_decision("dc-fill-0").is_none());
        assert!(state.seen_drain_commands.lock().contains("dc-fill-0"));
    }
}
