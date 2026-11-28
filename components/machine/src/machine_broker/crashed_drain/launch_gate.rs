//! Replacement launch gating and transient-unit coordination against
//! returning crashed-worker drains.

use super::*;

const RETURNING_DRAIN_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Session-local deferral returned when replacement launch must wait for or
/// yield to a returning crashed-worker drain; must never quarantine a worker
/// generation (R21-M6).
#[derive(Debug)]
pub(in crate::machine_broker) struct DrainContentionError(pub(in crate::machine_broker) String);

impl std::fmt::Display for DrainContentionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DrainContentionError {}

pub(in crate::machine_broker) struct SpawnedLaunchGuard<'a> {
    broker: &'a Broker,
    session_id: String,
}

impl Drop for SpawnedLaunchGuard<'_> {
    fn drop(&mut self) {
        self.broker.spawned_launches.lock().remove(&self.session_id);
    }
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
        let output = Command::new("systemctl")
            .args([
                "--user",
                "show",
                unit,
                "--property=LoadState",
                "--property=ActiveState",
            ])
            .output()
            .await
            .with_context(|| format!("checking transient worker unit {unit}"))?;
        let fields = String::from_utf8_lossy(&output.stdout);
        let load_state = systemd_show_value(&fields, "LoadState").unwrap_or_default();
        let active_state = systemd_show_value(&fields, "ActiveState").unwrap_or_default();
        if transient_unit_collected(&load_state) {
            return Ok(());
        }
        if !stopped_orphan
            && matches!(active_state.as_str(), "active" | "activating")
            && started.elapsed() >= ORPHAN_EXIT_GRACE
        {
            if let Some((broker, session_id)) = broker
                && !broker.should_stop_orphan_unit(session_id)
            {
                broker.wait_for_returning_crashed_drain(session_id).await?;
                started = tokio::time::Instant::now();
                deadline = started + TRANSIENT_UNIT_COLLECT_TIMEOUT;
                continue;
            }
            tracing::warn!(%unit, %active_state, "recycling orphaned transient worker unit");
            let status = Command::new("systemctl")
                .args(["--user", "stop", unit])
                .status()
                .await
                .with_context(|| format!("stopping orphaned transient worker unit {unit}"))?;
            if !status.success() {
                let output = Command::new("systemctl")
                    .args(["--user", "show", unit, "--property=LoadState"])
                    .output()
                    .await
                    .with_context(|| {
                        format!("rechecking transient worker unit {unit} after stop")
                    })?;
                let fields = String::from_utf8_lossy(&output.stdout);
                let load_state = systemd_show_value(&fields, "LoadState").unwrap_or_default();
                if transient_unit_collected(&load_state) {
                    return Ok(());
                }
                anyhow::bail!("systemctl stop {unit} exited {status} (LoadState={load_state})");
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
            let (has_fence, has_live_worker) = {
                let workers = self.workers.lock();
                (
                    self.has_any_crashed_drain_fence_locked(&workers, session_id),
                    workers.contains_key(session_id),
                )
            };
            if !has_fence {
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
        session_id: &str,
    ) -> Result<SpawnedLaunchGuard<'_>> {
        loop {
            self.wait_for_returning_crashed_drain(session_id).await?;
            let workers = self.workers.lock();
            if self.has_any_crashed_drain_fence_locked(&workers, session_id) {
                drop(workers);
                continue;
            }
            if workers.contains_key(session_id) {
                return Err(anyhow::Error::new(DrainContentionError(format!(
                    "worker for session {session_id} is already attached"
                ))));
            }
            // Canonical lock order: workers -> session_states -> spawned_launches -> crashed_drains.
            // Initialize Starting strictly after any returning drain has settled
            // so readiness polling cannot read the retired worker's Crashed state (R21-M1).
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
}
