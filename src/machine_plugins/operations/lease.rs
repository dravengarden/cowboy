//! Ephemeral execution authority for one authenticated Machine connection.
//! Neither connection ownership nor a monotonic deadline can be deserialized
//! from a durable receipt. This does not grant recovery or Provider credentials.

use crate::machine_protocol::plugin_step::{StepUnavailable, UninstallStep};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant};

const MAX_EXECUTION_TIME: Duration = Duration::from_mins(1);

/// The connection task is the unique owner. Detached command tasks receive only
/// leases; retaining one must not keep a disconnected connection authorized.
pub(crate) struct PluginExecutionScope {
    connected: Arc<AtomicBool>,
    service: Option<String>,
    machine: String,
}

impl PluginExecutionScope {
    pub(crate) fn new(service: Option<&str>, machine: &str) -> Self {
        Self {
            connected: Arc::new(AtomicBool::new(true)),
            service: service.map(str::to_owned),
            machine: machine.to_owned(),
        }
    }

    /// Called synchronously on command receipt, BEFORE spawning or waiting for
    /// a lifecycle lock. The wire supplies no duration or reusable capability.
    pub(crate) fn uninstall(
        &self,
        step: &UninstallStep,
    ) -> Result<UninstallExecutionLease, StepUnavailable> {
        self.uninstall_at(step, TimeSample::now())
    }

    fn uninstall_at(
        &self,
        step: &UninstallStep,
        received: TimeSample,
    ) -> Result<UninstallExecutionLease, StepUnavailable> {
        let request_digest = step
            .request_digest()
            .map_err(|_| StepUnavailable::InvalidRequest)?;
        if !self.connected.load(Ordering::Acquire)
            || self.service.as_deref() != Some(step.service_id.as_str())
            || self.machine != step.machine_id
        {
            return Err(StepUnavailable::WrongOwner);
        }
        let remaining = step.expires_at_ms.saturating_sub(received.wall_ms).max(0);
        let budget = Duration::from_millis(remaining.unsigned_abs()).min(MAX_EXECUTION_TIME);
        Ok(UninstallExecutionLease {
            connected: Arc::clone(&self.connected),
            request_digest,
            received: received.monotonic,
            deadline: received.monotonic + budget,
            expires_at_ms: step.expires_at_ms,
            wall_high_water: AtomicI64::new(received.wall_ms),
            expired: AtomicBool::new(received.wall_ms <= 0 || remaining == 0),
        })
    }
}

impl Drop for PluginExecutionScope {
    fn drop(&mut self) {
        self.connected.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
struct TimeSample {
    monotonic: Instant,
    wall_ms: i64,
}

impl TimeSample {
    fn now() -> Self {
        Self {
            monotonic: Instant::now(),
            wall_ms: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/// No Clone, Serialize or Deserialize: one admitted command, exact request,
/// original connection and process-local deadline. A saved result is evidence,
/// not an instance of this type.
pub(crate) struct UninstallExecutionLease {
    connected: Arc<AtomicBool>,
    request_digest: String,
    received: Instant,
    deadline: Instant,
    expires_at_ms: i64,
    wall_high_water: AtomicI64,
    expired: AtomicBool,
}

impl UninstallExecutionLease {
    pub(super) fn matches(&self, step: &UninstallStep) -> bool {
        step.request_digest()
            .is_ok_and(|digest| digest == self.request_digest)
    }

    pub(super) fn connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub(super) fn expired(&self) -> bool {
        self.expired_at(TimeSample::now())
    }

    pub(super) fn before_effect(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.connected() && !self.expired(),
            "Plugin execution lease ended before removal"
        );
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn expire_for_test(&self) {
        self.expired.store(true, Ordering::Release);
    }

    fn expired_at(&self, now: TimeSample) -> bool {
        let previous_wall = self
            .wall_high_water
            .fetch_max(now.wall_ms, Ordering::AcqRel);
        if now.monotonic < self.received
            || now.monotonic >= self.deadline
            || now.wall_ms <= 0
            || now.wall_ms < previous_wall
            || now.wall_ms >= self.expires_at_ms
        {
            self.expired.store(true, Ordering::Release);
        }
        self.expired.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests;
