//! A deliberately narrow recovery action: close an interrupted, still-prepared
//! uninstall. This never grants a Machine effect or restores a worker/session.

use super::*;
use crate::machine_protocol::plugin_step::digest as canonical_digest;
use crate::operation_budget::OperationBudget;

pub(crate) const MAX_RESOLUTION_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResolutionAction {
    AbortBeforeEffects,
    CompleteVerifiedRemoval,
}

impl ResolutionAction {
    pub(crate) fn terminal_phase(self) -> Phase {
        match self {
            Self::AbortBeforeEffects => Phase::Aborted,
            Self::CompleteVerifiedRemoval => Phase::Completed,
        }
    }
}

pub(crate) fn can_complete_removal(operation: &Operation) -> bool {
    operation.phase == Phase::NeedsAttention
        && operation.attention_from == Some(Phase::Uninstalling)
        && matches!(
            operation.problem,
            Some(Problem::UnknownMachineOutcome | Problem::Interrupted)
        )
        && operation.cause.is_none()
        && operation.intent.session_ids.is_empty()
        && operation.intent.live_session_ids.is_empty()
        && operation.intent.active_session_ids.is_empty()
        && operation.intent.installation_revision.is_some()
}

pub(crate) fn can_abort_before_effects(operation: &Operation) -> bool {
    operation.phase == Phase::NeedsAttention
        && operation.attention_from == Some(Phase::Prepared)
        && matches!(
            operation.problem,
            Some(Problem::Interrupted | Problem::StorageFailure)
        )
        && operation.cause.is_none()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolutionIntent {
    pub schema: u16,
    pub resolution_id: String,
    pub operation_id: String,
    pub service_id: String,
    pub machine_id: String,
    pub plugin_id: String,
    pub actor: Actor,
    pub action: ResolutionAction,
    pub operation_digest: String,
    pub before_updated_at_ms: i64,
    pub expires_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_evidence: Option<crate::machine_protocol::plugin_recovery::RecoveryObservation>,
}

impl ResolutionIntent {
    pub(crate) fn new(
        resolution_id: String,
        actor: Actor,
        operation: &Operation,
        expires_at_ms: i64,
    ) -> Result<Self> {
        ensure!(
            can_abort_before_effects(operation),
            "operation may already have effects"
        );
        operation.intent.validate()?;
        let intent = Self {
            schema: 1,
            resolution_id,
            operation_id: operation.intent.operation_id.clone(),
            service_id: operation.intent.service_id.clone(),
            machine_id: operation.intent.machine_id.clone(),
            plugin_id: operation.intent.plugin_id.clone(),
            actor,
            action: ResolutionAction::AbortBeforeEffects,
            operation_digest: canonical_digest(&serde_json::to_vec(operation)?),
            before_updated_at_ms: operation.updated_at_ms,
            expires_at_ms,
            machine_evidence: None,
        };
        intent.validate()?;
        Ok(intent)
    }

    pub(crate) fn complete_removal(
        resolution_id: String,
        actor: Actor,
        operation: &Operation,
        expires_at_ms: i64,
        evidence: crate::machine_protocol::plugin_recovery::RecoveryObservation,
    ) -> Result<Self> {
        ensure!(
            can_complete_removal(operation),
            "uninstall is not eligible for receipt completion"
        );
        operation.intent.validate()?;
        let intent = Self {
            schema: 1,
            resolution_id,
            operation_id: operation.intent.operation_id.clone(),
            service_id: operation.intent.service_id.clone(),
            machine_id: operation.intent.machine_id.clone(),
            plugin_id: operation.intent.plugin_id.clone(),
            actor,
            action: ResolutionAction::CompleteVerifiedRemoval,
            operation_digest: canonical_digest(&serde_json::to_vec(operation)?),
            before_updated_at_ms: operation.updated_at_ms,
            expires_at_ms,
            machine_evidence: Some(evidence),
        };
        ensure!(
            intent.matches(operation)?,
            "Machine removal is not verified"
        );
        Ok(intent)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            matches!(
                (self.action, self.machine_evidence.is_some()),
                (ResolutionAction::AbortBeforeEffects, false)
                    | (ResolutionAction::CompleteVerifiedRemoval, true)
            ),
            "resolution evidence does not match action"
        );
        let actor = match &self.actor {
            Actor::Product { user_id } => user_id,
            Actor::Admin { account } => account,
        };
        ensure!(
            self.schema == 1 && bounded(actor, 256),
            "invalid resolution owner or schema"
        );
        ensure!(
            [&self.resolution_id, &self.operation_id]
                .into_iter()
                .all(|id| {
                    (16..=128).contains(&id.len())
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                })
                && bounded(&self.service_id, 128)
                && [&self.machine_id, &self.plugin_id].into_iter().all(|id| {
                    bounded(id, 128)
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                }),
            "invalid resolution identity"
        );
        ensure!(
            digest(&self.operation_digest) && self.expires_at_ms > 0,
            "invalid resolution evidence or deadline"
        );
        Ok(())
    }

    pub(crate) fn matches(&self, operation: &Operation) -> Result<bool> {
        self.validate()?;
        let candidate = match self.action {
            ResolutionAction::AbortBeforeEffects => can_abort_before_effects(operation),
            ResolutionAction::CompleteVerifiedRemoval => can_complete_removal(operation)
                && self.machine_evidence.as_ref().is_some_and(|evidence| operation.intent.machine_step().is_ok_and(|step|
                    matches!(evidence.basis(&step), crate::machine_protocol::plugin_recovery::RecoveryBasis::MatchingRemoval { .. }))),
        };
        Ok(candidate
            && self.operation_id == operation.intent.operation_id
            && self.service_id == operation.intent.service_id
            && self.machine_id == operation.intent.machine_id
            && self.plugin_id == operation.intent.plugin_id
            && self.before_updated_at_ms == operation.updated_at_ms
            && self.operation_digest == canonical_digest(&serde_json::to_vec(operation)?))
    }
}

/// Core-only, process-local admission. Not Clone/Debug/serde; the Service mints
/// it only after checking the NEW confirmation's credential and Operator role.
/// The Store repeats the budget and complete evidence CAS under its write lock.
pub(crate) struct ResolutionPermit {
    intent: ResolutionIntent,
    budget: OperationBudget,
}

impl ResolutionPermit {
    pub(crate) fn new(intent: ResolutionIntent, budget: OperationBudget) -> Self {
        Self { intent, budget }
    }

    pub(crate) fn intent(&self) -> &ResolutionIntent {
        &self.intent
    }

    pub(crate) fn within_budget(&self) -> bool {
        !self.budget.expired()
    }

    #[cfg(test)]
    pub(crate) fn for_test(intent: ResolutionIntent) -> Self {
        let budget = OperationBudget::new(
            intent.expires_at_ms,
            std::time::Duration::from_mins(1),
            crate::operation_budget::TimeSample::now(),
        );
        Self::new(intent, budget)
    }

    #[cfg(test)]
    pub(crate) fn expire_for_test(&self) {
        self.budget.expire_for_test();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ResolutionReceipt {
    pub intent: ResolutionIntent,
    pub resolved_at_ms: i64,
}

impl ResolutionReceipt {
    pub(crate) fn matches_completed(&self, operation: &Operation) -> Result<bool> {
        let terminal = self.intent.action.terminal_phase();
        if operation.phase != terminal || operation.updated_at_ms != self.resolved_at_ms {
            return Ok(false);
        }
        let mut before = operation.clone();
        before.phase = Phase::NeedsAttention;
        before.updated_at_ms = self.intent.before_updated_at_ms;
        self.intent.matches(&before)
    }
}

#[cfg(test)]
pub(crate) fn fixture() -> Operation {
    Operation {
        intent: super::fixture("no-effect-resolution"),
        phase: Phase::NeedsAttention,
        problem: Some(Problem::Interrupted),
        cause: None,
        attention_from: Some(Phase::Prepared),
        created_at_ms: 1,
        updated_at_ms: 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_removal_requires_exact_applied_tombstone_and_no_sessions() {
        use crate::machine_protocol::plugin_recovery::{
            InstallationEvidence, RecoveryObservation, RecoverySnapshot,
        };
        use crate::machine_protocol::plugin_step::{StepOutcome, StepReceipt, StepUncertainty};
        let mut op = fixture();
        op.attention_from = Some(Phase::Uninstalling);
        op.problem = Some(Problem::UnknownMachineOutcome);
        op.intent.schema = 2;
        op.intent.installation_revision = Some(
            format!("installation-{}", "a".repeat(64))
                .try_into()
                .unwrap(),
        );
        let step = op.intent.machine_step().unwrap();
        let mut snapshot = RecoverySnapshot {
            request_digest: step.request_digest().unwrap(),
            receipt: Some(Box::new(StepReceipt {
                step: step.clone(),
                request_digest: step.request_digest().unwrap(),
                outcome: StepOutcome::Applied {},
            })),
            installation: InstallationEvidence::Removed {
                revision: format!("installation-{}", "b".repeat(64))
                    .try_into()
                    .unwrap(),
                previous_revision: step.installation_revision.clone().unwrap(),
                uninstall_request_digest: step.request_digest().unwrap(),
            },
            slot_fenced: false,
        };
        let make = |op: &Operation, snapshot: RecoverySnapshot| {
            ResolutionIntent::complete_removal(
                "resolution-complete-0001".into(),
                op.intent.actor.clone(),
                op,
                chrono::Utc::now().timestamp_millis() + 60_000,
                RecoveryObservation::Observed {
                    snapshot: Box::new(snapshot),
                },
            )
        };
        assert!(make(&op, snapshot.clone()).is_ok());
        let mut changed = op.clone();
        changed
            .intent
            .session_ids
            .push("session-not-approved".into());
        assert!(make(&changed, snapshot.clone()).is_err());
        snapshot.slot_fenced = true;
        assert!(make(&op, snapshot.clone()).is_err());
        snapshot.slot_fenced = false;
        snapshot.receipt.as_mut().unwrap().outcome = StepOutcome::Unknown {
            reason: StepUncertainty::EffectFailure,
        };
        assert!(make(&op, snapshot).is_err());
    }

    #[test]
    fn only_proven_pre_effect_interruption_is_a_candidate() {
        let op = fixture();
        assert!(can_abort_before_effects(&op));
        for phase in [
            Phase::StoppingSessions,
            Phase::Uninstalling,
            Phase::MachineUninstalled,
            Phase::RestoringMachine,
            Phase::RestoringSessions,
            Phase::Aborted,
            Phase::Completed,
            Phase::Compensated,
            Phase::NeedsAttention,
        ] {
            let mut changed = op.clone();
            changed.attention_from = Some(phase);
            assert!(!can_abort_before_effects(&changed), "{phase:?}");
        }
        let mut changed = op.clone();
        changed.attention_from = None;
        assert!(!can_abort_before_effects(&changed));
        changed = op.clone();
        changed.phase = Phase::Prepared;
        assert!(
            !can_abort_before_effects(&changed),
            "live coordinator is not recovery"
        );
        changed = op;
        changed.cause = Some(Problem::MachineRejected);
        assert!(!can_abort_before_effects(&changed));
        changed.cause = None;
        changed.problem = Some(Problem::UnknownMachineOutcome);
        assert!(!can_abort_before_effects(&changed));
    }

    #[test]
    fn resolution_binds_the_complete_snapshot_and_is_a_closed_action() {
        let op = fixture();
        let intent = ResolutionIntent::new(
            "resolution-00000001".into(),
            Actor::Admin {
                account: "new-operator".into(),
            },
            &op,
            1_000_000,
        )
        .unwrap();
        assert!(intent.matches(&op).unwrap());
        for field in [
            "actor", "impact", "deadline", "cause", "time", "service", "target",
        ] {
            let mut changed = op.clone();
            match field {
                "actor" => changed.intent.actor = intent.actor.clone(),
                "impact" => changed.intent.session_ids.push("other-session".into()),
                "deadline" => changed.intent.expires_at_ms += 1,
                "cause" => changed.problem = Some(Problem::StorageFailure),
                "time" => changed.updated_at_ms += 1,
                "service" => changed.intent.service_id = "another-service".into(),
                _ => changed.intent.machine_id = "another-machine".into(),
            }
            assert!(!intent.matches(&changed).unwrap(), "{field}");
        }
        let encoded = serde_json::to_value(&intent).unwrap();
        assert_eq!(
            serde_json::from_value::<ResolutionIntent>(encoded.clone()).unwrap(),
            intent
        );
        for (key, value) in [
            ("action", serde_json::json!("clear_fence")),
            ("force", serde_json::json!(true)),
            ("schema", serde_json::json!(2)),
        ] {
            let mut bad = encoded.clone();
            bad[key] = value;
            assert!(
                serde_json::from_value::<ResolutionIntent>(bad)
                    .map(|i| i.validate().is_err())
                    .unwrap_or(true)
            );
        }
    }
}
