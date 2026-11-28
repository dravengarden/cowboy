//! Replacement launch gating and transient-unit coordination against
//! returning crashed-worker drains.

use super::*;
use std::process::Stdio;
use tokio::io::AsyncReadExt as _;

const RETURNING_DRAIN_WAIT_TIMEOUT: Duration = Duration::from_secs(20);
const SYSTEMCTL_CALL_TIMEOUT: Duration = Duration::from_secs(10);
const WATCHDOG_RECONCILE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const WATCHDOG_INSPECT_RETRIES: usize = 3;

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

/// Returned when a quarantine-selected fallback waited behind a drain while a
/// canary rehabilitated the desired generation, so `spawn_with_fallback` can
/// retry the freshly rehabilitated desired generation instead of pinning a
/// stale fallback (R24-M7).
#[derive(Debug)]
pub(in crate::machine_broker) struct RehabilitatedGenerationError;

impl std::fmt::Display for RehabilitatedGenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("desired worker generation was rehabilitated during launch wait")
    }
}

impl std::error::Error for RehabilitatedGenerationError {}

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct TransientUnitStatus {
    load_state: String,
    active_state: String,
    sub_state: String,
    invocation_id: String,
    job: String,
}

impl TransientUnitStatus {
    fn is_collected(&self) -> bool {
        transient_unit_collected(&self.load_state) && self.job.is_empty()
    }

    /// Affirmative terminal observation with no pending systemd job (R24-M5).
    fn is_affirmatively_stopped(&self) -> bool {
        self.job.is_empty()
            && (transient_unit_collected(&self.load_state)
                || matches!(self.active_state.as_str(), "inactive" | "failed"))
    }

    fn is_active_or_activating(&self) -> bool {
        matches!(self.active_state.as_str(), "active" | "activating")
    }
}

/// Run `systemctl` with a hard timeout that explicitly kills and reaps the
/// child process before returning so no delayed `systemctl stop` can outlive
/// `OrphanStopGuard` and kill a replacement worker (R24-H1).
async fn run_systemctl_bounded(args: &[&str], timeout: Duration) -> Result<std::process::Output> {
    let mut child = Command::new("systemctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning systemctl {}", args.join(" ")))?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let run = async {
        let read_out = async {
            let mut buf = Vec::new();
            if let Some(ref mut pipe) = stdout_pipe {
                let _ = pipe.read_to_end(&mut buf).await;
            }
            buf
        };
        let read_err = async {
            let mut buf = Vec::new();
            if let Some(ref mut pipe) = stderr_pipe {
                let _ = pipe.read_to_end(&mut buf).await;
            }
            buf
        };
        let (status, stdout, stderr) = tokio::join!(child.wait(), read_out, read_err);
        let status = status?;
        Ok::<std::process::Output, std::io::Error>(std::process::Output {
            status,
            stdout,
            stderr,
        })
    };
    match tokio::time::timeout(timeout, run).await {
        Ok(result) => result.with_context(|| format!("waiting for systemctl {}", args.join(" "))),
        Err(_) => {
            // Explicitly kill and reap before returning so the reservation
            // cannot drop while a delayed `systemctl` child is still alive.
            let _ = child.start_kill();
            let _ = child.wait().await;
            anyhow::bail!("systemctl {} timed out after {timeout:?}", args.join(" "))
        }
    }
}

fn parse_transient_unit_status(fields: &str) -> Result<TransientUnitStatus> {
    let load_state = systemd_show_value(fields, "LoadState")
        .filter(|value| !value.is_empty())
        .context("systemctl show returned missing LoadState")?;
    let active_state = systemd_show_value(fields, "ActiveState")
        .filter(|value| !value.is_empty())
        .context("systemctl show returned missing ActiveState")?;
    let sub_state = systemd_show_value(fields, "SubState").unwrap_or_default();
    let invocation_id = systemd_show_value(fields, "InvocationID").unwrap_or_default();
    let job = systemd_show_value(fields, "Job").unwrap_or_default();
    Ok(TransientUnitStatus {
        load_state,
        active_state,
        sub_state,
        invocation_id,
        job,
    })
}

async fn inspect_transient_unit(unit: &str) -> Result<TransientUnitStatus> {
    let output = run_systemctl_bounded(
        &[
            "--user",
            "show",
            unit,
            "--property=LoadState",
            "--property=ActiveState",
            "--property=SubState",
            "--property=InvocationID",
            "--property=Job",
        ],
        SYSTEMCTL_CALL_TIMEOUT,
    )
    .await
    .with_context(|| format!("checking transient worker unit {unit}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "systemctl show {unit} failed ({}): {}",
            output.status,
            stderr.trim()
        );
    }
    let fields = String::from_utf8_lossy(&output.stdout);
    parse_transient_unit_status(&fields)
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
        let status = inspect_transient_unit(unit).await?;
        if status.is_collected() {
            return Ok(());
        }
        if !stopped_orphan
            && status.is_active_or_activating()
            && !status.invocation_id.is_empty()
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
            let current = inspect_transient_unit(unit).await?;
            if current.is_collected() {
                return Ok(());
            }
            if !current.is_active_or_activating()
                || current.invocation_id.is_empty()
                || current.invocation_id != status.invocation_id
            {
                started = tokio::time::Instant::now();
                deadline = started + TRANSIENT_UNIT_COLLECT_TIMEOUT;
                continue;
            }
            tracing::warn!(
                %unit,
                active_state = %current.active_state,
                invocation_id = %current.invocation_id,
                "recycling orphaned transient worker unit"
            );
            let stop_output =
                run_systemctl_bounded(&["--user", "stop", unit], SYSTEMCTL_CALL_TIMEOUT)
                    .await
                    .with_context(|| format!("stopping orphaned transient worker unit {unit}"))?;
            if !stop_output.status.success() {
                let recheck = inspect_transient_unit(unit).await?;
                if recheck.is_collected() {
                    return Ok(());
                }
                anyhow::bail!(
                    "systemctl stop {unit} exited {} (LoadState={})",
                    stop_output.status,
                    recheck.load_state
                );
            }
            stopped_orphan = true;
            continue;
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "transient worker unit {unit} was still {}/{}/{} (job={}) after {TRANSIENT_UNIT_COLLECT_TIMEOUT:?}",
                status.load_state,
                status.active_state,
                status.sub_state,
                status.job,
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

    fn reconnect_token_still_valid(&self, session_id: &str, token: u64) -> bool {
        self.awaiting_reconnect.lock().get(session_id).copied() == Some(token)
            && self.sessions.lock().contains_key(session_id)
            && !self.cancelled_sessions.lock().contains(session_id)
            && !self
                .permanently_deleted_sessions
                .lock()
                .contains(session_id)
            && !self.account_quiesced_sessions.lock().contains(session_id)
            && !self.resetting_sessions.lock().contains_key(session_id)
    }

    fn publish_watchdog_crashed_if_token_valid(&self, session_id: &str, token: u64) -> bool {
        // Revalidate token and lifecycle declarations immediately before
        // publishing `Crashed` while holding `session_lifecycle_gate` and
        // `OrphanStopGuard` (R24-M4).
        if !self.reconnect_token_still_valid(session_id, token)
            || self.workers.lock().contains_key(session_id)
        {
            return false;
        }
        {
            let mut waiting = self.awaiting_reconnect.lock();
            if waiting.get(session_id).copied() != Some(token) {
                return false;
            }
            waiting.remove(session_id);
        }
        self.publish_session_state(session_id, WorkerState::Crashed);
        true
    }

    /// Reconnect watchdog helper: takes `session_lifecycle_gate`, validates the
    /// per-arm reconnect token, reserves `OrphanStopGuard`, requires affirmative
    /// systemd unit state (never treating bus errors or transitional states as
    /// worker absence), kills & reaps any timed-out `systemctl stop`, reconciles
    /// pending stop jobs before dropping the reservation, and revalidates the
    /// token through `Crashed` publication (R22-H1, R23-H1, R24-H1, R24-M4, R24-M5).
    pub(in crate::machine_broker) async fn reap_unreconnected_worker_unit(
        &self,
        session_id: &str,
        token: u64,
    ) {
        if self.awaiting_reconnect.lock().get(session_id).copied() != Some(token) {
            return;
        }
        let gate = self.session_lifecycle_gate(session_id);
        let _lifecycle_guard = gate.lock().await;
        if !self.reconnect_token_still_valid(session_id, token) {
            return;
        }
        let Some(_stop_guard) = self.try_reserve_orphan_stop(session_id, false) else {
            return;
        };
        let unit = worker_unit_name(&self.args.socket, session_id);
        let mut initial = None;
        for attempt in 0..WATCHDOG_INSPECT_RETRIES {
            match inspect_transient_unit(&unit).await {
                Ok(status) => {
                    initial = Some(status);
                    break;
                }
                Err(error) => {
                    tracing::warn!(
                        session = %session_id,
                        attempt,
                        %error,
                        "reconnect watchdog systemd inspection failed; retrying without publishing Crashed"
                    );
                    if attempt + 1 < WATCHDOG_INSPECT_RETRIES {
                        tokio::time::sleep(WATCHDOG_RECONCILE_POLL_INTERVAL).await;
                    }
                }
            }
        }
        let Some(status) = initial else {
            return;
        };
        if status.is_affirmatively_stopped() {
            tracing::info!(
                session = %session_id,
                load_state = %status.load_state,
                active_state = %status.active_state,
                "declared worker has no running unit; reporting it stopped"
            );
            let _ = self.publish_watchdog_crashed_if_token_valid(session_id, token);
            return;
        }
        if !status.is_active_or_activating() || status.invocation_id.is_empty() {
            // Transitional state (e.g. `deactivating` or pending Job): wait
            // boundedly for affirmative terminal state without issuing a stop
            // or publishing false `Crashed` (R24-M5).
            if self
                .reconcile_unit_stopped(&unit, None, TRANSIENT_UNIT_COLLECT_TIMEOUT)
                .await
            {
                let _ = self.publish_watchdog_crashed_if_token_valid(session_id, token);
            }
            return;
        }
        // Re-inspect under the held reservation and require the exact inspected
        // InvocationID before issuing `systemctl stop` (R23-H1).
        let Ok(current) = inspect_transient_unit(&unit).await else {
            return;
        };
        if current.is_affirmatively_stopped() {
            let _ = self.publish_watchdog_crashed_if_token_valid(session_id, token);
            return;
        }
        if !current.is_active_or_activating() || current.invocation_id != status.invocation_id {
            return;
        }
        tracing::error!(
            session = %session_id,
            invocation_id = %current.invocation_id,
            "declared worker never reconnected; applying session-level extreme recovery"
        );
        match run_systemctl_bounded(&["--user", "stop", &unit], SYSTEMCTL_CALL_TIMEOUT).await {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                tracing::warn!(
                    session = %session_id,
                    status = %output.status,
                    "stopping missing worker unit failed"
                );
            }
            Err(error) => {
                tracing::warn!(
                    session = %session_id,
                    %error,
                    "stopping missing worker unit timed out or failed; child killed and reaped"
                );
            }
        }
        // Reconcile any stop job already submitted to systemd while still
        // holding `OrphanStopGuard` and `session_lifecycle_gate`, and publish
        // `Crashed` only after affirmative stopped observation (R24-H1, R24-M5).
        if self
            .reconcile_unit_stopped(
                &unit,
                Some(current.invocation_id.as_str()),
                TRANSIENT_UNIT_COLLECT_TIMEOUT,
            )
            .await
        {
            let _ = self.publish_watchdog_crashed_if_token_valid(session_id, token);
        }
    }

    async fn reconcile_unit_stopped(
        &self,
        unit: &str,
        expected_invocation: Option<&str>,
        budget: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            match inspect_transient_unit(unit).await {
                Ok(status) if status.is_affirmatively_stopped() => return true,
                Ok(status) => {
                    if let Some(expected) = expected_invocation
                        && !status.invocation_id.is_empty()
                        && status.invocation_id != expected
                    {
                        return false;
                    }
                }
                Err(_) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(WATCHDOG_RECONCILE_POLL_INTERVAL).await;
        }
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
        fallback_from_quarantine: bool,
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
            // Coordinate quarantine-based fallback with `rehabilitate_generation`
            // BEFORE claiming the spawn slot or installing a pin: if a canary
            // rehabilitated the desired generation while we waited behind a
            // drain, retry the desired generation instead of pinning a stale
            // fallback (R24-M7).
            if let Some(failed_generation) = session.fallback_for.as_deref()
                && !self.try_pin_fallback_for_slot(
                    session_id,
                    &session.generation,
                    failed_generation,
                    &session.provider,
                    fallback_from_quarantine,
                )
            {
                drop(workers);
                return Err(anyhow::Error::new(RehabilitatedGenerationError));
            }
            // Canonical lock order: workers -> session_states -> spawned_launches -> orphan_stopping_sessions -> crashed_drains.
            // Initialize Starting only once the actual spawn slot is acquired so
            // contention never leaks state (R21-M1, R22-M1, R23-H1, R23-M5).
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

    /// Release `launching` or atomically hand it to a queued `EnsureSession`
    /// retry that arrived while the previous launch was running or finalizing
    /// (R24-M2). Canonical lock order: cancelled_sessions ->
    /// permanently_deleted_sessions -> workers -> launching ->
    /// pending_ensure_retries.
    fn release_launch_or_handoff_retry(self: &Arc<Self>, session_id: &str, previous_failed: bool) {
        let queued_retry = {
            let cancelled = self.cancelled_sessions.lock();
            let deleted = self.permanently_deleted_sessions.lock();
            let workers = self.workers.lock();
            let mut launching = self.launching.lock();
            let retry = self.pending_ensure_retries.lock().remove(session_id);
            let eligible = previous_failed
                && !workers.contains_key(session_id)
                && !cancelled.contains(session_id)
                && !deleted.contains(session_id)
                && !self.account_quiesced_sessions.lock().contains(session_id)
                && !self.resetting_sessions.lock().contains_key(session_id);
            if eligible && retry.is_some() {
                // Keep `launching` owned for the queued retry.
                retry
            } else {
                launching.remove(session_id);
                None
            }
        };
        if let Some((retry_session, retry_attempt_id)) = queued_retry {
            let broker = Arc::clone(self);
            tokio::spawn(async move {
                broker
                    .run_queued_ensure_retry(retry_session, retry_attempt_id)
                    .await;
            });
        }
    }

    async fn run_queued_ensure_retry(self: &Arc<Self>, session: StartSession, attempt_id: String) {
        let session_id = session.session_id.clone();
        if let Err(reason) = self.gate_ensure_against_crashed_drain(&session_id).await {
            self.finish_ensure_launch(
                &session_id,
                &attempt_id,
                Err(anyhow::Error::new(DrainContentionError(reason))),
            );
            return;
        }
        if let Some(worker) = self
            .workers
            .lock()
            .get(&session_id)
            .map(|peer| peer.snapshot.clone())
        {
            self.send_controller(Frame::Snapshot {
                worker: Box::new(worker),
            });
            self.release_launch_or_handoff_retry(&session_id, false);
            return;
        }
        let result = self.spawn_with_fallback(session, None).await;
        self.finish_ensure_launch(&session_id, &attempt_id, result);
    }

    pub(in crate::machine_broker) fn finish_ensure_launch(
        self: &Arc<Self>,
        session_id: &str,
        attempt_id: &str,
        result: Result<()>,
    ) {
        let failed = result.is_err();
        if let Err(error) = result {
            if error.downcast_ref::<DrainContentionError>().is_some() {
                // Keep `launching` owned through negative ACK publication so a
                // retry arriving in this window queues cleanly into
                // `pending_ensure_retries` instead of being dropped (R23-M2, R24-M2).
                self.command_rejected(session_id, attempt_id.to_owned(), error.to_string());
                self.release_launch_or_handoff_retry(session_id, true);
                return;
            }
            // Keep `launching` owned through failure publication so a
            // concurrent retry cannot acquire the slot and read this
            // attempt's Crashed state (R22-M1).
            let cancelled = self.cancelled_sessions.lock();
            if cancelled.contains(session_id) {
                drop(cancelled);
                self.release_launch_or_handoff_retry(session_id, false);
                tracing::debug!(session = %session_id, %error, "deleted session launch stopped");
                return;
            }
            if self.resetting_sessions.lock().contains_key(session_id) {
                drop(cancelled);
                self.release_launch_or_handoff_retry(session_id, false);
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
            self.command_rejected(session_id, attempt_id.to_owned(), error.to_string());
            drop(cancelled);
        }
        self.release_launch_or_handoff_retry(session_id, failed);
    }

    /// Finalize a rollout cutover launch while keeping `launching` held through
    /// failure publication, deferring `DrainContentionError` without writing
    /// terminal `Crashed`, and handing off any queued Ensure retry (R24-M1, R24-M2).
    pub(in crate::machine_broker) fn finish_rollout_launch(
        self: &Arc<Self>,
        session_id: &str,
        old_generation: &str,
        result: Result<()>,
    ) {
        let failed = result.is_err();
        match result {
            Ok(()) => {
                tracing::info!(
                    session = %session_id,
                    %old_generation,
                    "worker generation cutover launched"
                );
            }
            Err(error) => {
                if error.downcast_ref::<DrainContentionError>().is_some() {
                    tracing::info!(
                        session = %session_id,
                        %old_generation,
                        %error,
                        "worker generation cutover deferred by active drain contention"
                    );
                    self.command_rejected(
                        session_id,
                        format!("rollout:{session_id}"),
                        error.to_string(),
                    );
                    self.release_launch_or_handoff_retry(session_id, true);
                    return;
                }
                let cancelled = self.cancelled_sessions.lock();
                if cancelled.contains(session_id) {
                    drop(cancelled);
                    self.release_launch_or_handoff_retry(session_id, false);
                    tracing::debug!(session = %session_id, %error, "deleted session rollout stopped");
                    return;
                }
                if self.resetting_sessions.lock().contains_key(session_id) {
                    drop(cancelled);
                    self.release_launch_or_handoff_retry(session_id, false);
                    tracing::debug!(
                        session = %session_id,
                        %error,
                        "superseded worker rollout stopped for context reset"
                    );
                    return;
                }
                tracing::error!(
                    session = %session_id,
                    %old_generation,
                    error = %format!("{error:#}"),
                    "worker cutover and fallback failed"
                );
                self.startup_failures
                    .lock()
                    .insert(session_id.to_owned(), error.to_string());
                self.publish_session_state(session_id, WorkerState::Crashed);
                self.command_rejected(
                    session_id,
                    format!("rollout:{session_id}"),
                    error.to_string(),
                );
                drop(cancelled);
            }
        }
        self.release_launch_or_handoff_retry(session_id, failed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_transient_unit_status_rejects_empty_or_missing_properties() {
        assert!(parse_transient_unit_status("").is_err());
        assert!(parse_transient_unit_status("LoadState=\nActiveState=\n").is_err());
        assert!(parse_transient_unit_status("LoadState=not-found\n").is_err());
        let stopped = parse_transient_unit_status(
            "LoadState=not-found\nActiveState=inactive\nSubState=dead\nInvocationID=\nJob=\n",
        )
        .unwrap();
        assert!(stopped.is_collected());
        assert!(stopped.is_affirmatively_stopped());

        let pending_job = parse_transient_unit_status(
            "LoadState=loaded\nActiveState=inactive\nSubState=dead\nInvocationID=\nJob=42\n",
        )
        .unwrap();
        assert!(!pending_job.is_collected());
        assert!(!pending_job.is_affirmatively_stopped());

        let deactivating = parse_transient_unit_status(
            "LoadState=loaded\nActiveState=deactivating\nSubState=stop-sigterm\nInvocationID=abc\nJob=\n",
        )
        .unwrap();
        assert!(!deactivating.is_affirmatively_stopped());
        assert!(!deactivating.is_active_or_activating());
    }
}
