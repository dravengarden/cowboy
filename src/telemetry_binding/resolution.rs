//! Explicit local bookkeeping, never a dispatch, compensation or egress grant.
//! The old operation is evidence; only a new core confirmation creates a permit.
#![cfg_attr(not(test), allow(dead_code))]

use super::*;
use crate::machine_protocol::telemetry_binding::BindingDigest;
use crate::operation_budget::OperationBudget;

pub(crate) const MAX_RESOLUTION_BYTES: usize = 96 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ResolutionAction {
    AbortBeforeDispatch,
    AcceptApplied { observation_digest: BindingDigest },
    RecordRejected { observation_digest: BindingDigest },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolutionIntent {
    pub schema: u16,
    pub resolution_id: String,
    pub operation_id: String,
    pub service_id: String,
    pub machine_id: String,
    pub actor: Actor,
    pub operation_digest: BindingDigest,
    pub action: ResolutionAction,
    pub expires_at_ms: i64,
}

fn id(value: &str, minimum: usize) -> bool {
    (minimum..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

impl ResolutionIntent {
    pub(crate) fn new(
        resolution_id: String,
        actor: Actor,
        before: &Operation,
        action: ResolutionAction,
        expires_at_ms: i64,
    ) -> Result<Self> {
        let intent = Self {
            schema: 1,
            resolution_id,
            operation_id: before.intent.operation_id.clone(),
            service_id: before.intent.service_id.clone(),
            machine_id: before.intent.machine_id.clone(),
            actor,
            operation_digest: binding_digest(&serde_json::to_vec(before)?),
            action,
            expires_at_ms,
        };
        intent.check_before(before)?;
        Ok(intent)
    }

    pub(crate) fn digest(&self) -> Result<BindingDigest> {
        let actor = match &self.actor {
            Actor::Product { user_id } => user_id,
            Actor::Admin { account } => account,
        };
        ensure!(
            self.schema == 1
                && id(&self.resolution_id, 16)
                && id(&self.operation_id, 16)
                && valid_service(&self.service_id)
                && id(&self.machine_id, 1)
                && !actor.is_empty()
                && actor.len() <= 256
                && !actor.chars().any(char::is_control)
                && (1..=9_007_199_254_740_991).contains(&self.expires_at_ms),
            "invalid binding resolution intent"
        );
        Ok(binding_digest(&serde_json::to_vec(self)?))
    }

    pub(crate) fn check_before(&self, before: &Operation) -> Result<()> {
        self.digest()?;
        before.after()?;
        ensure!(
            self.operation_id == before.intent.operation_id
                && self.service_id == before.intent.service_id
                && self.machine_id == before.intent.machine_id
                && self.operation_digest == binding_digest(&serde_json::to_vec(before)?),
            "binding resolution target changed"
        );
        ensure!(
            matches!(
                (&self.action, &before.progress),
                (ResolutionAction::AbortBeforeDispatch, Progress::Prepared)
                    | (
                        ResolutionAction::AcceptApplied { .. }
                            | ResolutionAction::RecordRejected { .. },
                        Progress::Dispatching | Progress::NeedsAttention { .. }
                    )
            ),
            "binding operation cannot be resolved by this action"
        );
        Ok(())
    }

    pub(crate) fn conclusion(
        &self,
        before: &Operation,
        observation: Option<BindingObservation>,
    ) -> Result<Progress> {
        self.check_before(before)?;
        let after = match (&self.action, observation) {
            (ResolutionAction::AbortBeforeDispatch, None) => Progress::Aborted,
            (ResolutionAction::AcceptApplied { observation_digest }, Some(observation))
                if *observation_digest == binding_digest(&serde_json::to_vec(&observation)?) =>
            {
                Progress::Completed { observation }
            }
            (ResolutionAction::RecordRejected { observation_digest }, Some(observation))
                if *observation_digest == binding_digest(&serde_json::to_vec(&observation)?) =>
            {
                Progress::Rejected { observation }
            }
            _ => anyhow::bail!("binding resolution observation changed"),
        };
        Operation {
            intent: before.intent.clone(),
            progress: after.clone(),
        }
        .after()?;
        Ok(after)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolutionRecord {
    pub intent: ResolutionIntent,
    pub before: Operation,
    pub after: Progress,
    pub resolved_at_ms: i64,
}

impl ResolutionRecord {
    pub(super) fn validate(&self, operation: &Operation) -> Result<()> {
        let observation = match &self.after {
            Progress::Completed { observation } | Progress::Rejected { observation } => {
                Some(observation.clone())
            }
            _ => None,
        };
        ensure!(
            self.before.intent == operation.intent
                && self.after == operation.progress
                && self.intent.conclusion(&self.before, observation)? == self.after
                && (1..self.intent.expires_at_ms).contains(&self.resolved_at_ms)
                && serde_json::to_vec(self)?.len() <= MAX_RESOLUTION_BYTES,
            "invalid binding resolution audit"
        );
        Ok(())
    }
}

// Deliberately no Clone, Debug or serde. The core transfers the SAME original
// confirmation budget here after rechecking its credential and fresh evidence.
pub(crate) struct ResolutionPermit {
    intent: ResolutionIntent,
    before: Operation,
    after: Progress,
    budget: OperationBudget,
}

impl ResolutionPermit {
    pub(crate) fn new(
        intent: ResolutionIntent,
        before: Operation,
        observation: Option<BindingObservation>,
        budget: OperationBudget,
    ) -> Result<Self> {
        let after = intent.conclusion(&before, observation)?;
        ensure!(!budget.expired(), "binding resolution confirmation expired");
        Ok(Self {
            intent,
            before,
            after,
            budget,
        })
    }

    pub(crate) fn intent(&self) -> &ResolutionIntent {
        &self.intent
    }

    pub(crate) fn within_budget(&self) -> bool {
        !self.budget.expired()
    }

    #[cfg(test)]
    pub(crate) fn expire_for_test(&self) {
        self.budget.expire_for_test();
    }
}

pub(super) fn apply(
    ledger: &mut Option<Ledger>,
    permit: &ResolutionPermit,
) -> Result<writer::Updated> {
    ensure!(
        permit.within_budget(),
        "binding resolution confirmation expired"
    );
    let original = ledger
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing binding journal"))?;
    if let Some(record) = original.resolutions.iter().find(|r| {
        r.intent.resolution_id == permit.intent.resolution_id
            || r.intent.operation_id == permit.intent.operation_id
    }) {
        ensure!(
            record.intent == permit.intent,
            "binding resolution identity conflict"
        );
        return Ok(writer::Updated {
            operation: Operation {
                intent: record.before.intent.clone(),
                progress: record.after.clone(),
            },
            admitted: false,
        });
    }
    ensure!(
        original.operations.last() == Some(&permit.before),
        "binding resolution CAS failed"
    );
    let operation = Operation {
        intent: permit.before.intent.clone(),
        progress: permit.after.clone(),
    };
    let record = ResolutionRecord {
        intent: permit.intent.clone(),
        before: permit.before.clone(),
        after: permit.after.clone(),
        resolved_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    record.validate(&operation)?;
    // Validate the complete prospective document before replacing any state.
    let mut updated = original.clone();
    *updated.operations.last_mut().expect("checked above") = operation.clone();
    updated.current = operation.after()?;
    updated.resolutions.push(record);
    updated.schema = 2;
    updated.encode(&permit.intent.service_id)?;
    ensure!(
        permit.within_budget(),
        "binding resolution confirmation expired"
    );
    *ledger = Some(updated);
    Ok(writer::Updated {
        operation,
        admitted: false,
    })
}

#[cfg(test)]
pub(crate) mod tests;
