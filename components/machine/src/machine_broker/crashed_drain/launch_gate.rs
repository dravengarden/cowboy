//! Replacement launch gating and transient-unit coordination against
//! returning crashed-worker drains.

use super::*;

const RETURNING_DRAIN_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Session-local deferral returned when replacement launch must wait for or
/// yield to a returning crashed-worker drain; must never quarantine a worker
/// generation (R21-M6, R22-M6).
#[derive(Debug)]
pub(in crate::machine_broker) struct DrainContentionError(pub(in crate::machine_broker) String);

impl std::fmt::Display for DrainContentionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DrainContentionError {}

/// Transient registration deferral while an orphan systemd unit stop is in
/// flight; `handle_peer` closes the socket cleanly without sending terminal
/// `Frame::Reject` (R22-H1).
#[derive(Debug)]
pub(in crate::machine_broker) struct RetryableRegistrationError(
    pub(in crate::machine_broker) String,
);

impl std::fmt::Display for RetryableRegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RetryableRegistrationError {}

pub(in crate::machine_broker) struct SpawnedLaunchGuard<'a> {
    broker: &'a Broker,
    session_id: String,
}

impl Drop for SpawnedLaunchGuard<'_> {
    fn drop(&mut self) {
        self.broker.spawned_launches.lock().remove(&self.session_id);
    }
}

/// Exclusive reservation preventing worker registration AND replacement spawn
/// admission while an orphan-unit check, stop, or terminal publication is in
/// flight (R22-H1, R23-H1, R23-M1).
pub(in crate::machine_broker) struct OrphanStopGuard<'a> {
    broker: &'a Broker,
    session_id: String,
}

impl Drop for OrphanStopGuard<'_> {
    fn drop(&mut self) {
        self.broker
            .orphan_stopping_sessions
            .lock()
            .remove(&self.session_id);
        self.broker.crashed_drains.settled.notify_waiters();
    }
}

async fn inspect_transient_unit(unit: &str) -> Result<(String, String, String)> {
    let output = Command::new("systemctl")
        .args([
            "--user",
            "show",
            unit,
            "--property=LoadState",
            "--property=ActiveState",
            "--property=InvocationID",
        ])
        .output()
        .await
        .with_context(|| format!("checking transient worker unit {unit}"))?;
    let fields = String::from_utf8_lossy(&output.stdout);
    let load_state = systemd_show_value(&fields, "LoadState").unwrap_or_default();
    let active_state = systemd_show_value(&fields, "ActiveState").unwrap_or_default();
    let invocation_id = systemd_show_value(&fields, "InvocationID").unwrap_or_default();
    Ok((load_state, active_state, invocation_id))
}

pub(in crate::machine_broker) async fn prepare_transient_unit_with_owner_check(
    broker: Option<(&Broker, &str)>,
    unit: &str,
) -> Result<()> {
    let mut started = tokio::time::Instant::now();
    let mut deadline = started + TRANSIENT_UNIT_COLLECT_TIMEOUT;
    let mut stopped_orphan = false;
    loop {
        if let Some((broker, session_id)) = broker
            && !broker.should_stop_orphan_unit(session_id)
        {
            broker.wait_for_returning_crashed_drain(session_id).await?;
            started = tokio::time::Instant::now();
            deadline = started + TRANSIENT_UNIT_COLLECT_TIMEOUT;
            continue;
        }
        let (load_state, active_state, invocation_id) = inspect_transient_unit(unit).await?;
        if transient_unit_collected(&load_state) {
            return Ok(());
        }
        if !stopped_orphan
            && matches!(active_state.as_str(), "active" | "activating")
            && !invocation_id.is_empty()
            && started.elapsed() >= ORPHAN_EXIT_GRACE
        {
            let _stop_guard = if let Some((broker, session_id)) = broker {
                let Some(guard) = broker.try_reserve_orphan_stop(session_id, true) else {
                    broker.wait_for_returning_crashed_drain(session_id).await?;
                    started = tokio::time::Instant::now();
                    deadline = started + TRANSIENT_UNIT_COLLECT_TIMEOUT;
                    continue;
                };
                Some(guard)
            } else {
                None
            };
            // Re-inspect while holding the exclusive orphan-stop reservation and
            // require a non-empty matching InvocationID before stopping (R22-H1, R23-H1).
            let (current_load, current_active, current_invocation) =
                inspect_transient_unit(unit).await?;
            if transient_unit_collected(&current_load) {
                return Ok(());
            }
            if !matches!(current_active.as_str(), "active" | "activating")
                || current_invocation.is_empty()
                || current_invocation != invocation_id
            {
                started = tokio::time::Instant::now();
                deadline = started + TRANSIENT_UNIT_COLLECT_TIMEOUT;
                continue;
            }
            tracing::warn!(
                %unit,
                active_state = %current_active,
                invocation_id = %current_invocation,
                "recycling orphaned transient worker unit"
            );
            let status = Command::new("systemctl")
                .args(["--user", "stop", unit])
                .status()
                .await
                .with_context(|| format!("stopping orphaned transient worker unit {unit}"))?;
            if !status.success() {
                let (recheck_load, _, _) = inspect_transient_unit(unit).await?;
                if transient_unit_collected(&recheck_load) {
                    return Ok(());
                }
                anyhow::bail!("systemctl stop {unit} exited {status} (LoadState={recheck_load})");
            }
            stopped_orphan = true;
            continue;
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "transient worker unit {unit} was still {load_state}/{active_state} after {TRANSIENT_UNIT_COLLECT_TIMEOUT:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

impl Broker {
    pub(in crate::machine_broker) async fn prepare_session_transient_unit(
        &self,
        session_id: &str,
    ) -> Result<String> {
        let unit = worker_unit_name(&self.args.socket, session_id);
        prepare_transient_unit_with_owner_check(Some((self, session_id)), &unit).await?;
        Ok(unit)
    }

    pub(in crate::machine_broker) fn try_reserve_orphan_stop(
        &self,
        session_id: &str,
        allow_during_launching: bool,
    ) -> Option<OrphanStopGuard<'_>> {
        let workers = self.workers.lock();
        if workers.contains_key(session_id)
            || (!allow_during_launching && self.launching.lock().contains(session_id))
            || self.spawned_launches.lock().contains(session_id)
            || self.has_any_crashed_drain_fence_locked(&workers, session_id)
        {
            return None;
        }
        if !self
            .orphan_stopping_sessions
            .lock()
            .insert(session_id.to_owned())
        {
            return None;
        }
        Some(OrphanStopGuard {
            broker: self,
            session_id: session_id.to_owned(),
        })
    }

    /// Reconnect watchdog helper: guards the absence check, `InvocationID`-bound
    /// orphan stop, and terminal `Crashed` publication under the same exclusive
    /// `OrphanStopGuard` that serializes against Ensure spawn admission and
    /// worker registration (R22-H1, R23-H1, R23-M1).
    pub(in crate::machine_broker) async fn reap_unreconnected_worker_unit(&self, session_id: &str) {
        if !self.awaiting_reconnect.lock().remove(session_id)
            || !self.sessions.lock().contains_key(session_id)
        {
            return;
        }
        let Some(_stop_guard) = self.try_reserve_orphan_stop(session_id, false) else {
            return;
        };
        let unit = worker_unit_name(&self.args.socket, session_id);
        let Ok((load_state, active_state, invocation_id)) =
            tokio::time::timeout(Duration::from_secs(10), inspect_transient_unit(&unit))
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("timeout")))
        else {
            return;
        };
        if transient_unit_collected(&load_state)
            || !matches!(active_state.as_str(), "active" | "activating")
        {
            tracing::info!(
                session = %session_id,
                "declared worker has no running unit; reporting it stopped"
            );
            self.publish_session_state(session_id, WorkerState::Crashed);
            return;
        }
        if invocation_id.is_empty() {
            return;
        }
        // Re-inspect under the held reservation and require the exact inspected
        // InvocationID before issuing `systemctl stop` (R23-H1).
        let Ok((current_load, current_active, current_invocation)) =
            tokio::time::timeout(Duration::from_secs(10), inspect_transient_unit(&unit))
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("timeout")))
        else {
            return;
        };
        if transient_unit_collected(&current_load)
            || !matches!(current_active.as_str(), "active" | "activating")
        {
            self.publish_session_state(session_id, WorkerState::Crashed);
            return;
        }
        if current_invocation != invocation_id {
            return;
        }
        tracing::error!(
            session = %session_id,
            invocation_id = %current_invocation,
            "declared worker never reconnected; applying session-level extreme recovery"
        );
        let stop = Command::new("systemctl")
            .args(["--user", "stop", &unit])
            .status();
        match tokio::time::timeout(Duration::from_secs(10), stop).await {
            Ok(Ok(status)) if status.success() => {}
            Ok(Ok(status)) => {
                tracing::warn!(session = %session_id, %status, "stopping missing worker unit failed");
            }
            Ok(Err(error)) => {
                tracing::warn!(session = %session_id, %error, "stopping missing worker unit failed");
            }
            Err(_) => {
                tracing::warn!(session = %session_id, "stopping missing worker unit timed out");
            }
        }
        self.publish_session_state(session_id, WorkerState::Crashed);
    }

    pub(in crate::machine_broker) async fn wait_for_returning_crashed_drain(
        &self,
        session_id: &str,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + RETURNING_DRAIN_WAIT_TIMEOUT;
        loop {
            if self.cancelled_sessions.lock().contains(session_id) {
                anyhow::bail!("session {session_id} was deleted during launch");
            }
            let notified = self.crashed_drains.settled.notified();
            let (has_fence, has_live_worker, orphan_stopping) = {
                let workers = self.workers.lock();
                (
                    self.has_any_crashed_drain_fence_locked(&workers, session_id),
                    workers.contains_key(session_id),
                    self.orphan_stopping_sessions.lock().contains(session_id),
                )
            };
            if !has_fence && !orphan_stopping {
                if has_live_worker {
                    return Err(anyhow::Error::new(DrainContentionError(format!(
                        "crashed worker drain did not retire existing worker for session {session_id}"
                    ))));
                }
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow::Error::new(DrainContentionError(format!(
                    "timed out waiting for returning crashed worker drain on session {session_id}"
                ))));
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let _ = tokio::time::timeout(remaining, notified).await;
        }
    }

    pub(in crate::machine_broker) fn should_stop_orphan_unit(&self, session_id: &str) -> bool {
        let workers = self.workers.lock();
        !workers.contains_key(session_id)
            && !self.has_any_crashed_drain_fence_locked(&workers, session_id)
    }

    pub(in crate::machine_broker) async fn acquire_spawned_launch_slot(
        &self,
        session: &StartSession,
    ) -> Result<SpawnedLaunchGuard<'_>> {
        let session_id = session.session_id.as_str();
        loop {
            self.wait_for_returning_crashed_drain(session_id).await?;
            let workers = self.workers.lock();
            if self.has_any_crashed_drain_fence_locked(&workers, session_id)
                || self.orphan_stopping_sessions.lock().contains(session_id)
            {
                drop(workers);
                continue;
            }
            if workers.contains_key(session_id) {
                return Err(anyhow::Error::new(DrainContentionError(format!(
                    "worker for session {session_id} is already attached"
                ))));
            }
            // Canonical lock order: workers -> session_states -> spawned_launches -> orphan_stopping_sessions -> crashed_drains.
            // Initialize Starting and defer fallback pinning until the actual
            // spawn slot is acquired so contention never leaks state (R21-M1, R22-M1, R23-H1, R23-M5).
            let mut session_states = self.session_states.lock();
            self.spawned_launches.lock().insert(session_id.to_owned());
            self.crashed_drains
                .inner
                .lock()
                .recently_drained
                .remove(session_id);
            session_states.insert(session_id.to_owned(), WorkerState::Starting);
            drop(session_states);
            drop(workers);
            if let Some(failed_generation) = session.fallback_for.as_deref() {
                self.pin_fallback(session_id, &session.generation, failed_generation);
            }
            self.startup_failures.lock().remove(session_id);
            return Ok(SpawnedLaunchGuard {
                broker: self,
                session_id: session_id.to_owned(),
            });
        }
    }

    /// Atomically move a dormant session to `WorkerState::Exited` and
    /// invalidate any cached drain receipt under `session_states -> crashed_drains`
    /// (matching handover lock order) so handover can never observe a torn
    /// state/receipt pair (R21-M5).
    pub(in crate::machine_broker) fn mark_dormant_exited_and_invalidate_drain(
        &self,
        session_id: &str,
    ) -> (Option<WorkerState>, bool) {
        let mut states = self.session_states.lock();
        let prev = states.insert(session_id.to_owned(), WorkerState::Exited);
        let had_receipt = self
            .crashed_drains
            .inner
            .lock()
            .recently_drained
            .remove(session_id)
            .is_some();
        (prev, had_receipt)
    }

    #[cfg(test)]
    pub(in crate::machine_broker) fn invalidate_recently_drained(&self, session_id: &str) {
        self.crashed_drains
            .inner
            .lock()
            .recently_drained
            .remove(session_id);
    }

    pub(in crate::machine_broker) fn finish_ensure_launch(
        &self,
        session_id: &str,
        result: Result<()>,
    ) {
        if let Err(error) = result {
            if error.downcast_ref::<DrainContentionError>().is_some() {
                // Keep `launching` owned through negative ACK publication so a
                // retry cannot claim the slot before this attempt's ACK is
                // enqueued onto the Controller channel (R23-M2).
                self.command_rejected(
                    session_id,
                    format!("ensure:{session_id}"),
                    error.to_string(),
                );
                self.launching.lock().remove(session_id);
                return;
            }
            // Keep `launching` owned through failure publication so a
            // concurrent retry cannot acquire the slot and read this
            // attempt's Crashed state (R22-M1).
            let cancelled = self.cancelled_sessions.lock();
            if cancelled.contains(session_id) {
                drop(cancelled);
                self.launching.lock().remove(session_id);
                tracing::debug!(session = %session_id, %error, "deleted session launch stopped");
                return;
            }
            if self.resetting_sessions.lock().contains_key(session_id) {
                drop(cancelled);
                self.launching.lock().remove(session_id);
                tracing::debug!(
                    session = %session_id,
                    %error,
                    "superseded worker launch stopped for context reset"
                );
                return;
            }
            tracing::error!(
                session = %session_id,
                reason = crate::logs::failure_reason(&error),
                error = %error,
                "worker launch failed"
            );
            self.startup_failures
                .lock()
                .insert(session_id.to_owned(), error.to_string());
            self.publish_session_state(session_id, WorkerState::Crashed);
            self.command_rejected(
                session_id,
                format!("ensure:{session_id}"),
                error.to_string(),
            );
            drop(cancelled);
        }
        self.launching.lock().remove(session_id);
    }
}
