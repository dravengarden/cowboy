//! Teardown of a worker whose Provider runs in the account sandbox: the
//! worker counts as stopped only after the daemon confirms that no unit of
//! its session remains. The worker's snapshot names its unit when it is
//! known; the session's durable launch record covers a worker that was lost
//! before (or without) reporting one.

use super::*;

tokio::task_local! {
    /// Prompt `(session_id, command_id)` whose `DispatchedPrompts` ingress record
    /// was freshly created by this exact core-command invocation (R5-M1).
    pub(super) static OWNED_PROMPT_INGRESS: Option<(String, String)>;
}

impl Broker {
    pub(super) fn retire_if_not_queued(&self, session_id: &str, command_id: &str) {
        let pending = self.pending_commands.lock();
        let queued = pending.get(session_id).is_some_and(|queue| {
            queue.iter().any(|cmd| {
                matches!(
                    cmd,
                    WorkerCommand::Prompt {
                        command_id: queued_id,
                        ..
                    } if queued_id == command_id
                )
            })
        });
        if !queued {
            self.dispatched_prompts
                .lock()
                .retire_if_unhanded(session_id, command_id);
        }
    }

    pub(super) fn retire_owned_unhanded_prompt(&self, session_id: &str, command_id: &str) {
        let owns_ingress = OWNED_PROMPT_INGRESS
            .try_with(|owned| {
                owned
                    .as_ref()
                    .is_some_and(|(s, c)| s == session_id && c == command_id)
            })
            .unwrap_or(false);
        if owns_ingress {
            self.retire_if_not_queued(session_id, command_id);
        }
    }

    pub(super) fn retire_stale_lease_prompt(&self, command: &crate::runtime_wire::CoreCommand) {
        if let crate::runtime_wire::CoreCommand::Prompt {
            session_id,
            command_id,
            ..
        } = command
        {
            self.retire_owned_unhanded_prompt(session_id, command_id);
        }
    }

    pub(super) fn has_pending_core_command(&self, session_id: &str) -> bool {
        self.pending_core_commands
            .lock()
            .get(session_id)
            .copied()
            .unwrap_or(0)
            > 0
    }

    /// `true` when the daemon confirmed every account unit of the session
    /// gone (stopping any a lost worker left behind), or it has none.
    pub(super) async fn confirm_account_unit_gone(&self, session_id: &str) -> bool {
        let unit = self
            .workers
            .lock()
            .get(session_id)
            .and_then(|worker| worker.snapshot.account_unit.clone())
            .map(|unit| unit.unit);
        let store = Arc::clone(&self.args.provider_store);
        let session = session_id.to_owned();
        let confirmed = tokio::task::spawn_blocking(move || {
            if let Some(unit) = &unit {
                store.confirm_account_unit_gone(unit)?;
            }
            store.confirm_session_account_units_gone(&session)
        })
        .await;
        match confirmed {
            Ok(Ok(())) => true,
            outcome => {
                tracing::error!(
                    session = session_id,
                    ?outcome,
                    "account sandbox unit stop unconfirmed"
                );
                false
            }
        }
    }

    pub(super) async fn handle_readmit_account_session(
        self: &Arc<Self>,
        session: StartSession,
        binding: crate::provider_account::PreparedBinding,
        command_id: String,
    ) {
        let session_id = session.session_id.clone();
        let result = self.readmit_account_session(session, &binding).await;
        self.send_controller(Frame::CommandAck {
            session_id,
            command_id,
            accepted: result.is_ok(),
            reason: result.err().map(|error| error.to_string()),
        });
    }
    pub(super) fn revoke_cache_protection(
        &self,
        session: &StartSession,
        session_id: &str,
        reason: &'static str,
    ) {
        let configuration = session.provider_behavior.as_ref().map_or_else(
            || crate::provider_behavior::legacy_behavior(&session.provider).configuration,
            |behavior| behavior.configuration.clone(),
        );
        if !crate::deepseek_cache::supported_behavior(&configuration) {
            return;
        }
        #[cfg(test)]
        self.revoked_cache_protection
            .lock()
            .push((session_id.to_owned(), reason));
        let provider = session.provider.clone();
        let session_id = session_id.to_owned();
        tokio::spawn(async move {
            if let Err(error) =
                crate::deepseek_cache::revoke_local_snapshot(&configuration, &session_id).await
            {
                tracing::warn!(
                    provider,
                    session = %session_id,
                    reason,
                    %error,
                    "failed to revoke DeepSeek cache-protection snapshot"
                );
            } else {
                tracing::debug!(
                    provider,
                    session = %session_id,
                    reason,
                    "revoked DeepSeek cache-protection snapshot"
                );
            }
        });
    }
}

/// Why a live worker cannot hibernate now, if anything would be lost.
pub(super) fn hibernation_refusal(worker: &WorkerSnapshot) -> Option<&'static str> {
    if worker.state != WorkerState::Running || worker.current_turn_id.is_some() {
        Some("session is busy")
    } else if !worker.pending_permissions.is_empty() {
        Some("session is waiting for a permission decision")
    } else if worker.background_tasks.unwrap_or(0) > 0 {
        Some("session has background work in progress")
    } else if worker.pending_prompt_count > 0 {
        Some("session has a queued prompt")
    } else {
        None
    }
}

pub(super) fn cutover_concurrency() -> usize {
    std::thread::available_parallelism()
        .map_or(4, std::num::NonZeroUsize::get)
        .div_ceil(2)
        .clamp(2, 8)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn new_registered_peer(
    session_id: &str,
    epoch: &str,
    generation: &str,
    executable: Option<String>,
    connection_id: u64,
    tx: mpsc::UnboundedSender<Frame>,
    welcomed: bool,
    drain_requested: bool,
) -> WorkerPeer {
    WorkerPeer {
        connection_id,
        epoch: epoch.to_owned(),
        tx,
        snapshot: WorkerSnapshot {
            session_id: session_id.to_owned(),
            worker_epoch: epoch.to_owned(),
            generation: generation.to_owned(),
            executable,
            launch: None,
            state: crate::runtime_wire::WorkerState::Starting,
            agent_session_id: None,
            native_thread_materialized: None,
            current_turn_id: None,
            supports_current_turn: false,
            delivery_controls: crate::runtime_wire::DeliveryControls::default(),
            wrap_up_steering: false,
            supports_subagent_cancel: false,
            last_runtime_seq: 0,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested,
            exit_detail: None,
            background_tasks: None,
            native_cycle_id: None,
            native_cycle_active: false,
            incarnation: None,
            account_unit: None,
            uncertain_effect: None,
        },
        acked_runtime_seq: 0,
        earliest_unacked_seq: None,
        highest_runtime_seq: 0,
        replay_gate: None,
        welcomed,
        replay_deferred: false,
        last_seen: Instant::now(),
        receive_probe: None,
        liveness: None,
        negotiated_protocol: 0,
        raw_wrap_up_steering: false,
        hint_tx: None,
        diag_reply_tx: None,
        last_hinted_target: None,
        crashed_drain_fenced: false,
        peer_pid: None,
    }
}

pub(super) fn enqueue_pending_prompt(
    pending: &mut HashMap<String, VecDeque<WorkerCommand>>,
    session_id: &str,
    command_id: &str,
    command: WorkerCommand,
) -> Option<(String, String)> {
    let queue = pending.entry(session_id.to_owned()).or_default();
    if !queue
        .iter()
        .any(|queued| super::worker_command_id(queued) == Some(command_id))
    {
        queue.push_back(command);
    }
    None
}

pub(super) fn evicted_command_digest(command_id: &str) -> [u8; 16] {
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(command_id.as_bytes());
    let mut out = [0_u8; 16];
    out.copy_from_slice(&hash[..16]);
    out
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
    crate::wrap_up::clamp_delivery_capabilities_for_provider(
        provider,
        &mut snapshot.supports_current_turn,
        &mut snapshot.delivery_controls.next_checkpoint,
    );
    let min_runtime_protocol = worker_protocol.min(controller_runtime_protocol);
    snapshot.wrap_up_steering = crate::wrap_up::effective_wrap_up_steering(
        provider,
        raw_wrap_up,
        controller_outer_protocol,
        min_runtime_protocol,
        true,
    );
}
