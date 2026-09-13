//! Host-owned continuous export policy, not a saved Operator confirmation.
//! Loading an explicit private startup policy is the only constructor. Binding
//! evidence, wire requests and Plugin packages cannot construct an activation.

use super::{PrivateSnapshot, read_private_snapshot};
use crate::machine_protocol::telemetry_binding::{BindingDigest, BindingSnapshot, valid_service};
use crate::machine_protocol::telemetry_export::{ATTEMPT_BUDGET, ExportAttempt};
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::otlp::{Export, Signal};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema: u16,
    service_id: String,
    machine_id: String,
    binding: BindingSnapshot,
    signals: Signals,
    startup: Startup,
}

enum Startup {
    /// Explicit standing host authority, independently revalidated at every
    /// startup. It permits NEW batches, never journal/file/attempt replay.
    ActivateExactBinding,
}

impl<'de> Deserialize<'de> for Startup {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        match String::deserialize(decoder)?.as_str() {
            "activate_exact_binding" => Ok(Self::ActivateExactBinding),
            _ => Err(serde::de::Error::custom(
                "invalid managed telemetry startup choice",
            )),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Signals {
    logs: bool,
    metrics: bool,
    traces: bool,
}

impl Signals {
    fn allows(&self, signal: Signal) -> bool {
        match signal {
            Signal::Logs => self.logs,
            Signal::Metrics => self.metrics,
            Signal::Traces => self.traces,
        }
    }
}

// No Debug/Clone/serde on activation or per-attempt authority. The Arc shares
// revocation; it does not copy a reusable batch grant.
pub(crate) struct BackgroundPolicy {
    path: PathBuf,
    config: Configuration,
    snapshot: PrivateSnapshot,
    stopped: AtomicBool,
}

impl BackgroundPolicy {
    pub(crate) fn load(path: &Path, service: &str) -> Result<Arc<Self>> {
        ensure!(
            path.is_absolute(),
            "managed telemetry policy must be absolute"
        );
        let (config, snapshot) = read_private_snapshot::<Configuration>(path)?;
        ensure!(
            config.schema == 1
                && valid_service(service)
                && config.service_id == service
                && (1..=128).contains(&config.machine_id.len())
                && config
                    .machine_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
            "invalid managed telemetry policy owner or schema"
        );
        config.binding.validate()?;
        ensure!(
            config.binding.selection.is_some(),
            "managed export requires a selected binding"
        );
        ensure!(
            config.signals.logs || config.signals.metrics || config.signals.traces,
            "managed telemetry policy requires a signal"
        );
        let Startup::ActivateExactBinding = config.startup;
        Ok(Arc::new(Self {
            path: path.to_owned(),
            config,
            snapshot,
            stopped: AtomicBool::new(false),
        }))
    }

    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    fn current(&self) -> bool {
        let valid = !self.stopped.load(Ordering::Acquire)
            && read_private_snapshot::<Configuration>(&self.path)
                .is_ok_and(|(_, current)| self.snapshot.matches(&current));
        if !valid {
            self.stop();
        }
        valid
    }

    /// Startup prerequisite. The typed export scope repeats the same binding
    /// predicate per batch; neither check is the source of policy authority.
    pub(crate) async fn check_binding(&self, store: &crate::store::Store) -> bool {
        let valid = self.current()
            && store
                .telemetry_binding_ledger(&self.config.service_id)
                .await
                .is_ok_and(|ledger| {
                    ledger.is_some_and(|ledger| {
                        ledger.permits_binding(
                            &self.config.service_id,
                            &self.config.machine_id,
                            &self.config.binding,
                        )
                    })
                })
            && self.current();
        if !valid {
            self.stop();
        }
        valid
    }

    pub(crate) fn allows(&self, signal: Signal) -> bool {
        self.config.signals.allows(signal)
    }

    pub(crate) fn admit(
        self: &Arc<Self>,
        payload: Export,
    ) -> Result<(ExportAttempt, BackgroundPermit)> {
        // Capture before validation or scheduling. Queueing after this point
        // consumes the same budget and cannot choose a replacement connection.
        let received = TimeSample::now();
        ensure!(
            self.current() && self.allows(payload.signal),
            "managed telemetry policy ended"
        );
        let attempt = ExportAttempt {
            schema: 1,
            attempt_id: format!("background-{:032x}", rand::random::<u128>()),
            service_id: self.config.service_id.clone(),
            machine_id: self.config.machine_id.clone(),
            binding: self.config.binding.clone(),
            payload,
            expires_at_ms: chrono::Utc::now().timestamp_millis().saturating_add(15_000),
        };
        let permit = BackgroundPermit {
            policy: self.clone(),
            digest: attempt.request_digest()?,
            budget: OperationBudget::new(attempt.expires_at_ms, ATTEMPT_BUDGET, received),
            revoked: AtomicBool::new(false),
        };
        ensure!(permit.check(&attempt), "managed telemetry attempt expired");
        Ok((attempt, permit))
    }
}

pub(crate) struct BackgroundPermit {
    policy: Arc<BackgroundPolicy>,
    digest: BindingDigest,
    budget: OperationBudget,
    revoked: AtomicBool,
}

impl BackgroundPermit {
    pub(crate) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }

    pub(crate) fn reject_binding(&self) {
        self.policy.stop();
        self.revoke();
    }

    pub(crate) fn remaining(&self) -> Duration {
        if self.revoked.load(Ordering::Acquire) || self.policy.stopped.load(Ordering::Acquire) {
            Duration::ZERO
        } else {
            self.budget.remaining()
        }
    }

    pub(crate) fn check(&self, attempt: &ExportAttempt) -> bool {
        let valid = !self.remaining().is_zero()
            && self.policy.current()
            && attempt
                .request_digest()
                .is_ok_and(|digest| digest == self.digest)
            && !self.remaining().is_zero();
        if !valid {
            self.revoke();
        }
        valid
    }
}

#[cfg(test)]
mod tests;
