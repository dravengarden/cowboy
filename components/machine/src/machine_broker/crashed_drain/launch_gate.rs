//! Replacement launch gating and transient-unit coordination against
//! returning crashed-worker drains.

use super::*;
use std::process::Stdio;
use tokio::io::AsyncReadExt as _;

const RETURNING_DRAIN_WAIT_TIMEOUT: Duration = Duration::from_secs(20);
const SYSTEMCTL_CALL_TIMEOUT: Duration = Duration::from_secs(10);
const WATCHDOG_RECONCILE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const WATCHDOG_INSPECT_RETRIES: usize = 3;
const WATCHDOG_MAX_RESCHEDULES: usize = 6;
const WATCHDOG_RESCHEDULE_DELAY: Duration = Duration::from_secs(5);

/// Session-local deferral returned when replacement launch must wait for or
/// yield to a returning crashed-worker drain or inconclusive host systemd
/// probe; must never quarantine a worker generation (R21-M6, R22-M6, R25-M8).
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

/// Owned exclusive reservation preventing worker registration AND replacement
/// spawn admission while an orphan-unit check, stop, or background stop-job
/// reconciliation is in flight (R22-H1, R23-H1, R25-H2).
pub(in crate::machine_broker) struct OrphanStopGuard {
    broker: Arc<Broker>,
    session_id: String,
}

impl Drop for OrphanStopGuard {
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
    /// Strict check matching `systemd_exit.rs`: unloaded transient unit with
    /// terminal active state and explicit empty `Job` (R25-M7).
    fn is_collected(&self) -> bool {
        transient_unit_collected(&self.load_state)
            && matches!(self.active_state.as_str(), "inactive" | "failed")
            && self.job.is_empty()
    }

    /// Affirmative terminal observation with consistent load/active state and
    /// explicit empty systemd `Job` (R24-M5, R25-M7).
    fn is_affirmatively_stopped(&self) -> bool {
        self.job.is_empty()
            && matches!(self.active_state.as_str(), "inactive" | "failed")
            && matches!(self.load_state.as_str(), "not-found" | "loaded" | "masked")
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
    // Require explicit `Job=` property line so truncated output cannot pass as
    // job-free (R25-M7).
    let job = systemd_show_value(fields, "Job").context("systemctl show returned missing Job")?;
    let sub_state = systemd_show_value(fields, "SubState").unwrap_or_default();
    let invocation_id = systemd_show_value(fields, "InvocationID").unwrap_or_default();
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
    broker: Option<(&Arc<Broker>, &str)>,
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
        // Retry inconclusive systemd `show` probes until deadline and return a
        // typed `DrainContentionError` deferral if they stay inconclusive so a
        // user-bus hiccup never quarantines a worker generation (R25-M8).
        let status = match inspect_transient_unit(unit).await {
            Ok(status) => status,
            Err(error) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow::Error::new(DrainContentionError(format!(
                        "transient worker unit {unit} inspection remained inconclusive: {error}"
                    ))));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        if status.is_collected() {
            return Ok(());
        }
        if !stopped_orphan
            && status.is_active_or_activating()
            && !status.invocation_id.is_empty()
            && started.elapsed() >= ORPHAN_EXIT_GRACE
        {
            let stop_guard = if let Some((broker, session_id)) = broker {
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
            let current = match inspect_transient_unit(unit).await {
                Ok(current) => current,
                Err(error) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(anyhow::Error::new(DrainContentionError(format!(
                            "transient worker unit {unit} re-inspection failed: {error}"
                        ))));
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
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
            let stop_res =
                run_systemctl_bounded(&["--user", "stop", unit], SYSTEMCTL_CALL_TIMEOUT).await;
            let stop_ok = stop_res.as_ref().is_ok_and(|out| out.status.success());
            let confirmed_stopped = if stop_ok {
                reconcile_unit_stopped_bounded(unit, TRANSIENT_UNIT_COLLECT_TIMEOUT).await
            } else {
                inspect_transient_unit(unit)
                    .await
                    .is_ok_and(|recheck| recheck.is_affirmatively_stopped())
            };
            if !confirmed_stopped {
                // Keep `OrphanStopGuard` held in a background reconciler until
                // systemd confirms the submitted stop job has cleared (R25-H2).
                if let Some(guard) = stop_guard {
                    spawn_orphan_stop_reconciler(guard, unit.to_owned(), None);
                }
                let detail = match stop_res {
                    Ok(out) => format!("systemctl stop {unit} exited {}", out.status),
                    Err(err) => format!("{err}"),
                };
                return Err(anyhow::Error::new(DrainContentionError(format!(
                    "orphaned transient unit {unit} stop is still reconciling ({detail}); retry"
                ))));
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

async fn reconcile_unit_stopped_bounded(unit: &str, budget: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if inspect_transient_unit(unit)
            .await
            .is_ok_and(|status| status.is_affirmatively_stopped())
        {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(WATCHDOG_RECONCILE_POLL_INTERVAL).await;
    }
}

/// Hold `OrphanStopGuard` across backoff polling until systemd affirmatively
/// confirms the stopped unit is terminal and job-free (or the session is
/// permanently deleted), preventing any returning worker or replacement launch
/// from racing an unconfirmed systemd stop job (R25-H2).
fn spawn_orphan_stop_reconciler(
    stop_guard: OrphanStopGuard,
    unit: String,
    watchdog_token: Option<u64>,
) {
    tokio::spawn(async move {
        let mut attempt: u32 = 0;
        loop {
            let delay = Duration::from_millis((200_u64 << attempt.min(5)).min(5_000));
            attempt = attempt.saturating_add(1);
            tokio::time::sleep(delay).await;
            if stop_guard
                .broker
                .permanently_deleted_sessions
                .lock()
                .contains(&stop_guard.session_id)
            {
                return;
            }
            if inspect_transient_unit(&unit)
                .await
                .is_ok_and(|status| status.is_affirmatively_stopped())
            {
                if let Some(token) = watchdog_token {
                    let gate = stop_guard
                        .broker
                        .session_lifecycle_gate(&stop_guard.session_id);
                    let _lifecycle_guard = gate.lock().await;
                    let _ = stop_guard
                        .broker
                        .publish_watchdog_crashed_if_token_valid(&stop_guard.session_id, token);
                }
                return;
            }
        }
    });
}

impl Broker {
    pub(in crate::machine_broker) async fn prepare_session_transient_unit(
        self: &Arc<Self>,
        session_id: &str,
    ) -> Result<String> {
        let unit = worker_unit_name(&self.args.socket, session_id);
        prepare_transient_unit_with_owner_check(Some((self, session_id)), &unit).await?;
        Ok(unit)
    }

    pub(in crate::machine_broker) fn try_reserve_orphan_stop(
        self: &Arc<Self>,
        session_id: &str,
        allow_during_launching: bool,
    ) -> Option<OrphanStopGuard> {
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
            broker: Arc::clone(self),
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

    fn reschedule_watchdog_after_inconclusive(
        self: &Arc<Self>,
        session_id: &str,
        token: u64,
        reschedules_left: usize,
    ) {
        if reschedules_left == 0 {
            let mut waiting = self.awaiting_reconnect.lock();
            if waiting.get(session_id).copied() == Some(token) {
                waiting.remove(session_id);
            }
            return;
        }
        if !self.reconnect_token_still_valid(session_id, token) {
            return;
        }
        let broker = Arc::clone(self);
        let session_id = session_id.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(WATCHDOG_RESCHEDULE_DELAY).await;
            broker
                .reap_unreconnected_worker_unit_inner(&session_id, token, reschedules_left - 1)
                .await;
        });
    }

    /// Reconnect watchdog entry point (R22-H1, R23-H1, R24-H1, R24-M4, R24-M5,
    /// R25-H2, R25-M6, R25-M7).
    pub(in crate::machine_broker) async fn reap_unreconnected_worker_unit(
        self: &Arc<Self>,
        session_id: &str,
        token: u64,
    ) {
        self.reap_unreconnected_worker_unit_inner(session_id, token, WATCHDOG_MAX_RESCHEDULES)
            .await;
    }

    async fn reap_unreconnected_worker_unit_inner(
        self: &Arc<Self>,
        session_id: &str,
        token: u64,
        reschedules_left: usize,
    ) {
        if self.awaiting_reconnect.lock().get(session_id).copied() != Some(token) {
            return;
        }
        let gate = self.session_lifecycle_gate(session_id);
        let _lifecycle_guard = gate.lock().await;
        if !self.reconnect_token_still_valid(session_id, token) {
            return;
        }
        let Some(stop_guard) = self.try_reserve_orphan_stop(session_id, false) else {
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
            drop(stop_guard);
            drop(_lifecycle_guard);
            self.reschedule_watchdog_after_inconclusive(session_id, token, reschedules_left);
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
            if reconcile_unit_stopped_bounded(&unit, TRANSIENT_UNIT_COLLECT_TIMEOUT).await {
                let _ = self.publish_watchdog_crashed_if_token_valid(session_id, token);
            } else {
                drop(stop_guard);
                drop(_lifecycle_guard);
                self.reschedule_watchdog_after_inconclusive(session_id, token, reschedules_left);
            }
            return;
        }
        let Ok(current) = inspect_transient_unit(&unit).await else {
            drop(stop_guard);
            drop(_lifecycle_guard);
            self.reschedule_watchdog_after_inconclusive(session_id, token, reschedules_left);
            return;
        };
        if current.is_affirmatively_stopped() {
            let _ = self.publish_watchdog_crashed_if_token_valid(session_id, token);
            return;
        }
        if !current.is_active_or_activating() || current.invocation_id != status.invocation_id {
            drop(stop_guard);
            drop(_lifecycle_guard);
            self.reschedule_watchdog_after_inconclusive(session_id, token, reschedules_left);
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
        if reconcile_unit_stopped_bounded(&unit, TRANSIENT_UNIT_COLLECT_TIMEOUT).await {
            let _ = self.publish_watchdog_crashed_if_token_valid(session_id, token);
        } else {
            // Keep `OrphanStopGuard` held across background reconciliation until
            // the submitted stop job is affirmatively cleared (R25-H2).
            drop(_lifecycle_guard);
            spawn_orphan_stop_reconciler(stop_guard, unit, Some(token));
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

    /// Atomically inspect whether a queued `EnsureSession` retry is eligible to
    /// take over `launching` without releasing the slot yet.
    fn take_eligible_queued_retry(
        &self,
        session_id: &str,
        previous_failed: bool,
    ) -> Option<(StartSession, String)> {
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
        if eligible && let Some((retry_session, retry_attempt_id)) = retry {
            // Keep `launching` owned and record the handed-off attempt ID so
            // replays of the handed-off attempt coalesce cleanly (R25-M5).
            self.active_ensure_attempts
                .lock()
                .insert(session_id.to_owned(), retry_attempt_id.clone());
            Some((retry_session, retry_attempt_id))
        } else {
            self.active_ensure_attempts.lock().remove(session_id);
            launching.remove(session_id);
            None
        }
    }

    fn spawn_handed_off_retry(self: &Arc<Self>, retry: Option<(StartSession, String)>) {
        if let Some((retry_session, retry_attempt_id)) = retry {
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
        // Clone snapshot and drop `workers` guard BEFORE `send_controller` or
        // `take_eligible_queued_retry` so `workers` is never re-locked (R25-H1).
        let existing_snapshot = self
            .workers
            .lock()
            .get(&session_id)
            .map(|peer| peer.snapshot.clone());
        if let Some(worker) = existing_snapshot {
            self.send_controller(Frame::Snapshot {
                worker: Box::new(worker),
            });
            let _ = self.take_eligible_queued_retry(&session_id, false);
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
        let Err(error) = result else {
            let _ = self.take_eligible_queued_retry(session_id, false);
            return;
        };
        if error.downcast_ref::<DrainContentionError>().is_some() {
            self.command_rejected(session_id, attempt_id.to_owned(), error.to_string());
            let retry = self.take_eligible_queued_retry(session_id, true);
            self.spawn_handed_off_retry(retry);
            return;
        }
        let cancelled_or_resetting = self.cancelled_sessions.lock().contains(session_id)
            || self.resetting_sessions.lock().contains_key(session_id);
        if cancelled_or_resetting {
            tracing::debug!(
                session = %session_id,
                %error,
                "deleted or resetting session launch stopped"
            );
            let _ = self.take_eligible_queued_retry(session_id, false);
            return;
        }
        // Check for a queued retry BEFORE emitting an uncorrelated `Crashed`
        // snapshot: if a newer retry is already taking over `launching`, emit
        // only this superseded attempt's correlated rejection ACK and suppress
        // terminal `Crashed` publication so the retry's waiting prompts stay
        // intact (R25-M3).
        let has_queued_retry = self.pending_ensure_retries.lock().contains_key(session_id);
        if has_queued_retry {
            tracing::warn!(
                session = %session_id,
                %attempt_id,
                reason = crate::logs::failure_reason(&error),
                error = %error,
                "superseded worker launch failed; handing off to queued retry without Crashed snapshot"
            );
            self.command_rejected(session_id, attempt_id.to_owned(), error.to_string());
            let retry = self.take_eligible_queued_retry(session_id, true);
            self.spawn_handed_off_retry(retry);
            return;
        }
        let cancelled = self.cancelled_sessions.lock();
        if cancelled.contains(session_id) || self.resetting_sessions.lock().contains_key(session_id)
        {
            drop(cancelled);
            let _ = self.take_eligible_queued_retry(session_id, false);
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
        let retry = self.take_eligible_queued_retry(session_id, true);
        self.spawn_handed_off_retry(retry);
    }

    pub(in crate::machine_broker) fn finish_rollout_launch(
        self: &Arc<Self>,
        session_id: &str,
        old_generation: &str,
        result: Result<()>,
    ) {
        let Err(error) = result else {
            tracing::info!(
                session = %session_id,
                %old_generation,
                "worker generation cutover launched"
            );
            let _ = self.take_eligible_queued_retry(session_id, false);
            return;
        };
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
            let retry = self.take_eligible_queued_retry(session_id, true);
            self.spawn_handed_off_retry(retry);
            return;
        }
        let cancelled_or_resetting = self.cancelled_sessions.lock().contains(session_id)
            || self.resetting_sessions.lock().contains_key(session_id);
        if cancelled_or_resetting {
            tracing::debug!(
                session = %session_id,
                %error,
                "deleted or resetting session rollout stopped"
            );
            let _ = self.take_eligible_queued_retry(session_id, false);
            return;
        }
        if self.pending_ensure_retries.lock().contains_key(session_id) {
            tracing::warn!(
                session = %session_id,
                %old_generation,
                error = %format!("{error:#}"),
                "superseded rollout launch failed; handing off to queued ensure retry"
            );
            self.command_rejected(
                session_id,
                format!("rollout:{session_id}"),
                error.to_string(),
            );
            let retry = self.take_eligible_queued_retry(session_id, true);
            self.spawn_handed_off_retry(retry);
            return;
        }
        let cancelled = self.cancelled_sessions.lock();
        if cancelled.contains(session_id) || self.resetting_sessions.lock().contains_key(session_id)
        {
            drop(cancelled);
            let _ = self.take_eligible_queued_retry(session_id, false);
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
        let retry = self.take_eligible_queued_retry(session_id, true);
        self.spawn_handed_off_retry(retry);
    }
}

impl Broker {
    pub(in crate::machine_broker) fn pin_session_failure_fallback(
        &self,
        session_id: &str,
        worker_generation: &str,
        failed_generation: &str,
    ) {
        self.fallback_pins
            .lock()
            .insert(session_id.to_owned(), worker_generation.to_owned());
        self.fallback_targets
            .lock()
            .insert(session_id.to_owned(), failed_generation.to_owned());
        self.session_failure_fallback_pins
            .lock()
            .insert(session_id.to_owned());
    }

    pub(in crate::machine_broker) fn try_pin_fallback_for_slot(
        &self,
        session_id: &str,
        worker_generation: &str,
        failed_generation: &str,
        provider: &str,
        fallback_from_quarantine: bool,
    ) -> bool {
        let generation_key = (failed_generation.to_owned(), provider.to_owned());
        let healthy_generations = self.healthy_generations.lock();
        if fallback_from_quarantine && healthy_generations.contains(&generation_key) {
            return false;
        }
        self.fallback_pins
            .lock()
            .insert(session_id.to_owned(), worker_generation.to_owned());
        self.fallback_targets
            .lock()
            .insert(session_id.to_owned(), failed_generation.to_owned());
        if fallback_from_quarantine {
            self.session_failure_fallback_pins.lock().remove(session_id);
        } else {
            self.session_failure_fallback_pins
                .lock()
                .insert(session_id.to_owned());
        }
        drop(healthy_generations);
        true
    }

    pub(in crate::machine_broker) fn pin_registered_fallback_if_eligible(
        &self,
        session_id: &str,
        generation: &str,
        desired: &str,
        fallback_for: Option<&str>,
        fallback_provider: Option<&str>,
    ) {
        let Some(failed) = fallback_for.filter(|failed| desired.is_empty() || *failed == desired)
        else {
            return;
        };
        let pinned_now = match fallback_provider {
            Some(provider) => {
                let keep_session_pin = self
                    .session_failure_fallback_pins
                    .lock()
                    .contains(session_id);
                self.try_pin_fallback_for_slot(
                    session_id,
                    generation,
                    failed,
                    provider,
                    !keep_session_pin,
                )
            }
            None => {
                self.pin_fallback(session_id, generation, failed);
                true
            }
        };
        if pinned_now {
            *self.previous_generation.lock() = Some(generation.to_owned());
        }
    }

    pub(in crate::machine_broker) async fn run_session_failure_fallback(
        self: &Arc<Self>,
        selected: &StartSession,
        failed_generation: &str,
        first_error: anyhow::Error,
    ) -> Result<()> {
        match self.spawn_and_wait_ready(selected, false).await {
            Ok(()) => Ok(()),
            Err(fallback_error)
                if fallback_error
                    .downcast_ref::<DrainContentionError>()
                    .is_some() =>
            {
                Err(fallback_error)
            }
            Err(fallback_error) => {
                if !self
                    .cancelled_sessions
                    .lock()
                    .contains(&selected.session_id)
                {
                    self.pin_session_failure_fallback(
                        &selected.session_id,
                        &selected.generation,
                        failed_generation,
                    );
                }
                Err(fallback_error).with_context(|| {
                    format!("fallback after generation launch failed: {first_error}")
                })
            }
        }
    }

    pub(in crate::machine_broker) fn reserve_or_queue_ensure_launch(
        &self,
        session: &StartSession,
        attempt_id: &str,
    ) -> bool {
        let displaced_retry = {
            let mut launching = self.launching.lock();
            if !launching.insert(session.session_id.clone()) {
                let is_active_replay = self
                    .active_ensure_attempts
                    .lock()
                    .get(&session.session_id)
                    .is_some_and(|active_id| active_id == attempt_id);
                if is_active_replay {
                    Some(None)
                } else {
                    Some(
                        self.pending_ensure_retries
                            .lock()
                            .insert(
                                session.session_id.clone(),
                                (session.clone(), attempt_id.to_owned()),
                            )
                            .map(|(_, old_id)| old_id)
                            .filter(|old_id| old_id != attempt_id),
                    )
                }
            } else {
                self.active_ensure_attempts
                    .lock()
                    .insert(session.session_id.clone(), attempt_id.to_owned());
                None
            }
        };
        if let Some(old_id) = displaced_retry {
            if let Some(old_attempt_id) = old_id {
                self.command_rejected(
                    &session.session_id,
                    old_attempt_id,
                    "superseded by newer ensure retry".into(),
                );
            }
            return false;
        }
        true
    }

    pub(in crate::machine_broker) async fn spawn_rollout_after_disconnect(
        self: &Arc<Self>,
        session_id: String,
        old_generation: String,
        resumable_agent_session_id: Option<String>,
    ) {
        // Revalidate under `session_lifecycle_gate` before reserving
        // `launching` or updating `sessions`/`session_states` so a delayed
        // rollout disconnect can never overwrite a concurrent reset or
        // deletion (R25-M1).
        let session = {
            let gate = self.session_lifecycle_gate(&session_id);
            let _guard = gate.lock().await;
            if self.cancelled_sessions.lock().contains(&session_id)
                || self
                    .permanently_deleted_sessions
                    .lock()
                    .contains(&session_id)
                || self.account_session_stopped(&session_id)
                || self.resetting_sessions.lock().contains_key(&session_id)
                || self.workers.lock().contains_key(&session_id)
            {
                return;
            }
            let Some(mut session) = self.sessions.lock().get(&session_id).cloned() else {
                return;
            };
            if !self.launching.lock().insert(session_id.clone()) {
                return;
            }
            session.agent_session_id = resumable_agent_session_id;
            session.generation = self.desired_generation.lock().clone();
            session.fallback_for = None;
            self.session_states
                .lock()
                .insert(session_id.clone(), WorkerState::Starting);
            self.sessions
                .lock()
                .insert(session_id.clone(), session.clone());
            session
        };
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let _permit = broker.cutover_permit(&session_id).await;
            let result = broker
                .spawn_with_fallback(session, Some(old_generation.clone()))
                .await;
            broker.finish_rollout_launch(&session_id, &old_generation, result);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_transient_unit_status_rejects_empty_or_missing_properties() {
        assert!(parse_transient_unit_status("").is_err());
        assert!(parse_transient_unit_status("LoadState=\nActiveState=\nJob=\n").is_err());
        assert!(
            parse_transient_unit_status("LoadState=not-found\nActiveState=inactive\n").is_err()
        );

        let stopped = parse_transient_unit_status(
            "LoadState=not-found\nActiveState=inactive\nSubState=dead\nInvocationID=\nJob=\n",
        )
        .unwrap();
        assert!(stopped.is_collected());
        assert!(stopped.is_affirmatively_stopped());

        // not-found alongside transitional ActiveState must NOT count as stopped (R25-M7).
        let transitional_not_found = parse_transient_unit_status(
            "LoadState=not-found\nActiveState=deactivating\nSubState=stop-sigterm\nInvocationID=\nJob=\n",
        )
        .unwrap();
        assert!(!transitional_not_found.is_collected());
        assert!(!transitional_not_found.is_affirmatively_stopped());

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
