//! Service-owned binding evidence. This is not an export or recovery grant.
//! The first journal row permanently fences legacy Service export admission.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::machine_protocol::telemetry_binding::{
    BindingChange, BindingNamespace, BindingObservation, BindingOutcome, BindingSnapshot,
    BindingStep, binding_digest, valid_service,
};
use crate::plugin_operation::Actor;

pub(crate) const MAX_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_OPERATIONS: usize = 1024;
const MAX_INTENT_BYTES: usize = 16 * 1024;
const MAX_OBSERVATION_BYTES: usize = 16 * 1024;

pub(crate) mod resolution;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Intent {
    pub schema: u16,
    pub operation_id: String,
    pub service_id: String,
    pub actor: Actor,
    pub machine_id: String,
    /// No Machine namespace is distinct from a managed initial/revoked head.
    pub expected: Option<BindingSnapshot>,
    pub change: BindingChange,
    pub expires_at_ms: i64,
}

impl Intent {
    pub(crate) fn machine_step(&self) -> Result<BindingStep> {
        let actor = match &self.actor {
            Actor::Product { user_id } => user_id,
            Actor::Admin { account } => account,
        };
        ensure!(
            matches!(self.schema, 1 | 2)
                && !actor.is_empty()
                && actor.len() <= 256
                && !actor.chars().any(char::is_control),
            "invalid Service binding schema or actor"
        );
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= MAX_INTENT_BYTES,
            "binding intent exceeds budget"
        );
        let step = BindingStep {
            schema: self.schema,
            operation_id: self.operation_id.clone(),
            service_id: self.service_id.clone(),
            machine_id: self.machine_id.clone(),
            plan_digest: binding_digest(&bytes),
            expected_namespace: (self.schema == 2).then_some(if self.expected.is_some() {
                BindingNamespace::Managed
            } else {
                BindingNamespace::Unmanaged
            }),
            expected: self
                .expected
                .clone()
                .unwrap_or_else(BindingSnapshot::initial),
            change: self.change.clone(),
            expires_at_ms: self.expires_at_ms,
        };
        step.validate()?;
        ensure!(
            step.after()?.policy_epoch == step.expected.policy_epoch.next()?,
            "binding must use exactly the next policy epoch"
        );
        Ok(step)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Attention {
    AuthorizationEnded,
    Uncertain,
    InvalidEvidence,
    HeadChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Progress {
    Prepared,
    Dispatching,
    /// Only a Prepared operation can be aborted without dispatch evidence.
    Aborted,
    Completed {
        observation: BindingObservation,
    },
    Rejected {
        observation: BindingObservation,
    },
    NeedsAttention {
        reason: Attention,
        observation: Option<BindingObservation>,
    },
}

impl Progress {
    fn unresolved(&self) -> bool {
        matches!(
            self,
            Self::Prepared | Self::Dispatching | Self::NeedsAttention { .. }
        )
    }

    fn permits(&self, next: &Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Prepared,
                Self::Dispatching | Self::Aborted | Self::NeedsAttention { .. }
            ) | (
                Self::Dispatching,
                Self::Completed { .. } | Self::Rejected { .. } | Self::NeedsAttention { .. }
            )
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Operation {
    pub intent: Intent,
    pub progress: Progress,
}

impl Operation {
    /// Validate a whole receipt and its exact current head, not merely an ID or
    /// a historical Applied flag. Authority is deliberately not reconstructed.
    fn after(&self) -> Result<Option<BindingSnapshot>> {
        let step = self.intent.machine_step()?;
        let observation = match &self.progress {
            Progress::Completed { observation } | Progress::Rejected { observation } => {
                Some(observation)
            }
            Progress::NeedsAttention { observation, .. } => observation.as_ref(),
            _ => None,
        };
        if let Some(observation) = observation {
            ensure!(
                serde_json::to_vec(observation)?.len() <= MAX_OBSERVATION_BYTES
                    && observation.matches(&step),
                "invalid Service binding observation"
            );
        }
        if matches!(
            self.progress,
            Progress::Completed { .. } | Progress::Rejected { .. }
        ) {
            let Some(BindingObservation::Observed { snapshot }) = observation else {
                anyhow::bail!("binding completion requires a Machine observation");
            };
            ensure!(!snapshot.unresolved, "Machine binding remains unresolved");
            let receipt = snapshot
                .receipt
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing binding receipt"))?;
            match (&self.progress, &receipt.outcome) {
                (Progress::Completed { .. }, BindingOutcome::Applied { after }) => {
                    ensure!(
                        snapshot.current.as_ref() == Some(after),
                        "binding head changed after application"
                    );
                }
                (Progress::Rejected { .. }, BindingOutcome::Rejected { .. }) => {
                    ensure!(
                        snapshot.current.as_ref() == Some(&step.expected),
                        "binding head changed after rejection"
                    );
                }
                _ => anyhow::bail!("binding completion outcome mismatch"),
            }
            return Ok(snapshot.current.clone());
        }
        Ok(self.intent.expected.clone())
    }
}

/// One fixed Service export slot. Moving to another Machine is intentionally
/// not an implicit fallback or a one-sided rewrite of this ledger's owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ledger {
    schema: u16,
    service_id: String,
    machine_id: String,
    pub current: Option<BindingSnapshot>,
    pub operations: Vec<Operation>,
    /// Schema one remains byte-compatible until the first explicit resolution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolutions: Vec<resolution::ResolutionRecord>,
}

impl Ledger {
    /// Evidence required by a fresh, independently authorized export attempt.
    /// This predicate itself never creates a sending capability.
    #[cfg_attr(not(all(test, feature = "machine-host")), allow(dead_code))]
    pub(crate) fn permits_export(
        &self,
        attempt: &crate::machine_protocol::telemetry_export::ExportAttempt,
    ) -> bool {
        self.service_id == attempt.service_id
            && self.machine_id == attempt.machine_id
            && self.current.as_ref() == Some(&attempt.binding)
            && attempt.binding.selection.is_some()
            && !self.operations.iter().any(|op| op.progress.unresolved())
    }

    pub(crate) fn decode(document: &str, service: &str) -> Result<Self> {
        ensure!(
            document.len() <= MAX_BYTES,
            "Service binding journal exceeds budget"
        );
        let ledger: Self = serde_json::from_str(document)
            .map_err(|_| anyhow::anyhow!("invalid Service binding journal"))?;
        ledger.validate(service)?;
        Ok(ledger)
    }

    fn validate(&self, service: &str) -> Result<()> {
        ensure!(
            matches!(self.schema, 1 | 2) && valid_service(service) && self.service_id == service,
            "binding journal owner changed"
        );
        ensure!(
            (self.schema == 1) == self.resolutions.is_empty()
                && self.resolutions.len() <= self.operations.len(),
            "invalid binding resolution schema or capacity"
        );
        ensure!(
            !self.operations.is_empty() && self.operations.len() <= MAX_OPERATIONS,
            "invalid binding journal capacity"
        );
        let mut head = None;
        let mut seen = std::collections::HashSet::new();
        for (index, operation) in self.operations.iter().enumerate() {
            let intent = &operation.intent;
            ensure!(
                intent.service_id == self.service_id
                    && intent.machine_id == self.machine_id
                    && intent.expected == head
                    && seen.insert(&intent.operation_id),
                "binding journal chain or identity changed"
            );
            Self::check_restoration(intent, &self.operations[..index])?;
            head = operation.after()?;
            ensure!(
                !operation.progress.unresolved() || index + 1 == self.operations.len(),
                "binding operation follows unresolved evidence"
            );
        }
        ensure!(
            head == self.current,
            "Service binding journal head mismatch"
        );
        let mut resolved = std::collections::HashSet::new();
        let mut resolution_ids = std::collections::HashSet::new();
        let mut previous_resolution = None;
        for record in &self.resolutions {
            ensure!(
                resolved.insert(&record.intent.operation_id)
                    && resolution_ids.insert(&record.intent.resolution_id),
                "duplicate binding resolution"
            );
            let (index, operation) = self
                .operations
                .iter()
                .enumerate()
                .find(|(_, op)| op.intent.operation_id == record.intent.operation_id)
                .ok_or_else(|| anyhow::anyhow!("missing resolved binding operation"))?;
            ensure!(
                previous_resolution.is_none_or(|previous| previous < index),
                "binding resolution order changed"
            );
            previous_resolution = Some(index);
            record.validate(operation)?;
        }
        Ok(())
    }

    fn check_restoration(intent: &Intent, previous: &[Operation]) -> Result<()> {
        if let BindingChange::Restore {
            forward_request_digest,
            selection,
            ..
        } = &intent.change
        {
            let forward = previous
                .iter()
                .find(|operation| {
                    matches!(operation.progress, Progress::Completed { .. })
                        && operation
                            .intent
                            .machine_step()
                            .and_then(|s| s.request_digest())
                            .is_ok_and(|d| &d == forward_request_digest)
                })
                .ok_or_else(|| anyhow::anyhow!("missing completed forward binding"))?;
            let step = forward.intent.machine_step()?;
            ensure!(
                intent.expected.as_ref() == Some(&step.after()?)
                    && selection == &step.expected.selection,
                "binding restoration target changed"
            );
        }
        Ok(())
    }

    pub(crate) fn encode(&self, service: &str) -> Result<String> {
        self.validate(service)?;
        let document = serde_json::to_string(self)?;
        ensure!(
            document.len() <= MAX_BYTES,
            "Service binding journal capacity exhausted"
        );
        Ok(document)
    }
}

// Staged finite writer. Protocol 15 exists, but no HTTP mutation or production
// admission switch is exported. Tests exercise these exact paths.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod writer {
    use super::*;

    pub(crate) enum Change<'a> {
        Begin(&'a Intent),
        Advance {
            expected: &'a Operation,
            progress: Progress,
        },
        Resolve(&'a resolution::ResolutionPermit),
    }

    impl Change<'_> {
        pub(crate) fn service(&self) -> &str {
            match self {
                Self::Begin(intent) => &intent.service_id,
                Self::Advance { expected, .. } => &expected.intent.service_id,
                Self::Resolve(permit) => &permit.intent().service_id,
            }
        }

        pub(crate) fn within_budget(&self) -> bool {
            match self {
                Self::Resolve(permit) => permit.within_budget(),
                _ => true,
            }
        }
    }

    pub(crate) struct Updated {
        pub operation: Operation,
        /// An identical retry returns evidence, never permission to dispatch.
        pub admitted: bool,
    }

    pub(crate) fn apply(ledger: &mut Option<Ledger>, change: &Change<'_>) -> Result<Updated> {
        if let Some(ledger) = ledger.as_ref() {
            ledger.validate(change.service())?;
        }
        match change {
            Change::Resolve(permit) => resolution::apply(ledger, permit),
            Change::Begin(intent) => {
                intent.machine_step()?;
                if let Some(ledger) = ledger {
                    ensure!(
                        ledger.service_id == intent.service_id
                            && ledger.machine_id == intent.machine_id,
                        "binding slot owner changed"
                    );
                    if let Some(existing) = ledger
                        .operations
                        .iter()
                        .find(|op| op.intent.operation_id == intent.operation_id)
                    {
                        ensure!(
                            &existing.intent == *intent,
                            "binding operation identity conflict"
                        );
                        return Ok(Updated {
                            operation: existing.clone(),
                            admitted: false,
                        });
                    }
                    ensure!(
                        ledger.current == intent.expected
                            && ledger
                                .operations
                                .last()
                                .is_none_or(|op| !op.progress.unresolved()),
                        "binding slot changed or remains fenced"
                    );
                    Ledger::check_restoration(intent, &ledger.operations)?;
                } else {
                    ensure!(
                        intent.expected.is_none(),
                        "unmanaged Service binding changed"
                    );
                    Ledger::check_restoration(intent, &[])?;
                }
                let operation = Operation {
                    intent: (*intent).clone(),
                    progress: Progress::Prepared,
                };
                let ledger = ledger.get_or_insert_with(|| Ledger {
                    schema: 1,
                    service_id: intent.service_id.clone(),
                    machine_id: intent.machine_id.clone(),
                    current: None,
                    operations: Vec::new(),
                    resolutions: Vec::new(),
                });
                ledger.operations.push(operation.clone());
                let bytes = ledger.encode(&intent.service_id)?.len();
                // Reserve the largest bounded completion plus a new head before
                // the intent can fence legacy admission. Never prune evidence.
                ensure!(
                    bytes
                        + MAX_OBSERVATION_BYTES
                        + MAX_INTENT_BYTES
                        + resolution::MAX_RESOLUTION_BYTES
                        <= MAX_BYTES,
                    "binding completion capacity exhausted"
                );
                Ok(Updated {
                    operation,
                    admitted: true,
                })
            }
            Change::Advance { expected, progress } => {
                let ledger = ledger
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("missing binding intent"))?;
                let current = ledger
                    .operations
                    .last_mut()
                    .ok_or_else(|| anyhow::anyhow!("missing binding operation"))?;
                ensure!(
                    current == *expected && current.progress.permits(progress),
                    "binding operation CAS failed"
                );
                current.progress = progress.clone();
                ledger.current = current.after()?;
                let operation = current.clone();
                ledger.validate(change.service())?;
                Ok(Updated {
                    operation,
                    admitted: false,
                })
            }
        }
    }
}

/// Opaque, sticky admission fence. Recovery may observe evidence but cannot
/// activate a managed exporter or silently fall back to private legacy config.
#[derive(Clone)]
pub(crate) struct LegacyFence(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl LegacyFence {
    pub(crate) async fn recover(
        store: Option<&crate::store::Store>,
        service: &str,
    ) -> Result<Self> {
        let managed = match store {
            Some(store) => store.telemetry_binding_ledger(service).await?.is_some(),
            None => false,
        };
        tracing::info!(
            admission_enabled = false,
            managed_namespace = managed,
            "Service telemetry binding reader recovered"
        );
        Ok(Self(std::sync::Arc::new(
            std::sync::atomic::AtomicBool::new(managed),
        )))
    }

    pub(crate) fn allows_legacy(&self) -> bool {
        !self.0.load(std::sync::atomic::Ordering::Acquire)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn close(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn unmanaged_fixture() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )))
    }
}

#[cfg(test)]
pub(crate) fn fixture(suffix: &str) -> Intent {
    let step = crate::machine_protocol::telemetry_binding::fixture();
    Intent {
        schema: 1,
        operation_id: format!("service-binding-{suffix}"),
        service_id: step.service_id,
        actor: Actor::Product {
            user_id: "operator-fixture".into(),
        },
        machine_id: step.machine_id,
        expected: None,
        change: step.change,
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 60_000,
    }
}

#[cfg(test)]
pub(crate) mod tests;
