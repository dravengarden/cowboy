//! A fresh, finite Machine bookkeeping purpose. No binding replay or egress.
use super::telemetry_binding::{
    BindingCommitFailure, BindingDigest, BindingObservation, BindingObservationSnapshot,
    BindingOutcome, BindingReceipt, BindingRejection, BindingStep, BindingUnavailable,
    binding_digest,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_RECOVERY_BYTES: usize = 64 * 1024;

/// Audit identity, not a deserializable Operator credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryActor {
    Product { user_id: String },
    Admin { account: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    RejectInterruptedPrepared,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRequest {
    pub schema: u16,
    pub resolution_id: String,
    pub actor: RecoveryActor,
    /// Complete original Service operation, including its unresolved progress.
    pub service_operation_digest: BindingDigest,
    pub action: RecoveryAction,
    pub step: BindingStep,
    pub expected_observation_digest: BindingDigest,
    /// New confirmation's deadline; the old step may have expired long ago.
    pub expires_at_ms: i64,
}

impl RecoveryRequest {
    pub(crate) fn validate(&self) -> Result<()> {
        self.step.validate_commit()?;
        let actor = match &self.actor {
            RecoveryActor::Product { user_id } => user_id,
            RecoveryActor::Admin { account } => account,
        };
        ensure!(
            self.schema == 1
                && (16..=128).contains(&self.resolution_id.len())
                && self
                    .resolution_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                && !actor.is_empty()
                && actor.len() <= 256
                && !actor.chars().any(char::is_control)
                && (1..=9_007_199_254_740_991).contains(&self.expires_at_ms)
                && serde_json::to_vec(self)?.len() <= 16 * 1024,
            "invalid Machine binding recovery request"
        );
        ensure!(
            self.expected_observation_digest
                == binding_digest(&serde_json::to_vec(&prepared(&self.step)?)?),
            "recovery requires exact Prepared evidence"
        );
        Ok(())
    }

    pub(crate) fn digest(&self) -> Result<BindingDigest> {
        self.validate()?;
        Ok(binding_digest(&serde_json::to_vec(self)?))
    }

    pub(crate) fn expects(&self, observation: &BindingObservation) -> bool {
        self.validate().is_ok()
            && observation.matches(&self.step)
            && serde_json::to_vec(observation)
                .is_ok_and(|bytes| binding_digest(&bytes) == self.expected_observation_digest)
    }
}

pub(crate) fn prepared(step: &BindingStep) -> Result<BindingObservation> {
    Ok(BindingObservation::Observed {
        snapshot: Box::new(BindingObservationSnapshot {
            request_digest: step.request_digest()?,
            receipt: Some(Box::new(BindingReceipt {
                step: step.clone(),
                request_digest: step.request_digest()?,
                outcome: BindingOutcome::Prepared {},
            })),
            current: Some(step.expected.clone()),
            unresolved: true,
        }),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryReceipt {
    pub request: RecoveryRequest,
    pub binding: BindingReceipt,
    pub resolved_at_ms: i64,
}

impl RecoveryReceipt {
    pub(crate) fn validate(&self) -> Result<()> {
        self.request.validate()?;
        ensure!(
            self.binding.matches(&self.request.step)
                && self.binding.outcome
                    == BindingOutcome::Rejected {
                        reason: BindingRejection::AuthorizationEnded
                    }
                && (1..self.request.expires_at_ms).contains(&self.resolved_at_ms)
                && serde_json::to_vec(self)?.len() <= MAX_RECOVERY_BYTES,
            "invalid Machine binding recovery audit"
        );
        Ok(())
    }
    pub(crate) fn matches(&self, request: &RecoveryRequest) -> bool {
        self.request == *request && self.validate().is_ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySnapshot {
    pub request_digest: BindingDigest,
    pub receipt: Option<Box<RecoveryReceipt>>,
    /// Current observation, distinct from the historical recovery receipt.
    pub binding: BindingObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryObservation {
    Observed { snapshot: Box<RecoverySnapshot> },
    Unavailable { reason: BindingUnavailable },
}

impl RecoveryObservation {
    pub(crate) fn matches(&self, request: &RecoveryRequest) -> bool {
        let Ok(digest) = request.digest() else {
            return false;
        };
        match self {
            Self::Unavailable { .. } => true,
            Self::Observed { snapshot } => {
                snapshot.request_digest == digest
                    && snapshot.binding.matches(&request.step)
                    && snapshot.receipt.as_ref().is_none_or(|receipt| {
                        receipt.matches(request)
                            && matches!(&snapshot.binding, BindingObservation::Observed { snapshot }
                                    if snapshot.receipt.as_deref() == Some(&receipt.binding))
                    })
                    && serde_json::to_vec(self)
                        .is_ok_and(|bytes| bytes.len() <= MAX_RECOVERY_BYTES + 16 * 1024)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryResult {
    Observed { observation: RecoveryObservation },
    Unavailable { failure: BindingCommitFailure },
}

#[cfg(test)]
pub(crate) fn fixture() -> RecoveryRequest {
    let step = super::telemetry_binding::execution_fixture();
    RecoveryRequest {
        schema: 1,
        resolution_id: "machine-binding-recovery-fixture".into(),
        actor: RecoveryActor::Product {
            user_id: "fresh-operator".into(),
        },
        service_operation_digest: binding_digest(b"complete Service operation fixture"),
        action: RecoveryAction::RejectInterruptedPrepared,
        expected_observation_digest: binding_digest(
            &serde_json::to_vec(&prepared(&step).unwrap()).unwrap(),
        ),
        step,
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 60_000,
    }
}

#[cfg(test)]
pub(crate) fn observed_fixture(request: &RecoveryRequest) -> RecoveryObservation {
    let binding = BindingReceipt {
        step: request.step.clone(),
        request_digest: request.step.request_digest().unwrap(),
        outcome: BindingOutcome::Rejected {
            reason: BindingRejection::AuthorizationEnded,
        },
    };
    RecoveryObservation::Observed {
        snapshot: Box::new(RecoverySnapshot {
            request_digest: request.digest().unwrap(),
            receipt: Some(Box::new(RecoveryReceipt {
                request: request.clone(),
                binding: binding.clone(),
                resolved_at_ms: chrono::Utc::now().timestamp_millis(),
            })),
            binding: BindingObservation::Observed {
                snapshot: Box::new(BindingObservationSnapshot {
                    request_digest: binding.request_digest.clone(),
                    receipt: Some(Box::new(binding)),
                    current: Some(request.step.expected.clone()),
                    unresolved: false,
                }),
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::{MachineCommand, MachineEvent};

    #[test]
    fn recovery_is_closed_schema_exact_prepared_and_separate_protocol_seventeen() {
        let request = fixture();
        assert!(request.expects(&prepared(&request.step).unwrap()));
        for field in [
            "schema",
            "resolution_id",
            "actor",
            "action",
            "expires_at_ms",
            "extra",
        ] {
            let mut json = serde_json::to_value(&request).unwrap();
            json[field] = match field {
                "schema" => 2.into(),
                "actor" => serde_json::json!({"kind":"product", "user_id":"", "token":"forbidden"}),
                "expires_at_ms" => 0.into(),
                _ => "invalid".into(),
            };
            assert!(
                !serde_json::from_value::<RecoveryRequest>(json)
                    .is_ok_and(|r| r.validate().is_ok()),
                "{field}"
            );
        }
        let mut old = request.clone();
        old.step.schema = 1;
        old.step.expected_namespace = None;
        old.expected_observation_digest =
            binding_digest(&serde_json::to_vec(&prepared(&old.step).unwrap()).unwrap());
        assert!(old.validate().is_err());
        let mut unknown = prepared(&request.step).unwrap();
        let BindingObservation::Observed { snapshot } = &mut unknown else {
            panic!()
        };
        snapshot.receipt.as_mut().unwrap().outcome = BindingOutcome::Unknown {};
        assert!(!request.expects(&unknown));
        for command in [
            MachineCommand::RecoverTelemetryBinding {
                request_id: "mutation".into(),
                recovery: Box::new(request.clone()),
            },
            MachineCommand::QueryTelemetryRecovery {
                request_id: "query".into(),
                recovery: Box::new(request.clone()),
            },
        ] {
            assert_eq!(command.minimum_protocol(), 17);
            let bytes = serde_json::to_vec(&command).unwrap();
            assert_eq!(
                bytes,
                serde_json::to_vec(&serde_json::from_slice::<MachineCommand>(&bytes).unwrap())
                    .unwrap()
            );
        }
        let result = RecoveryResult::Observed {
            observation: observed_fixture(&request),
        };
        let event = MachineEvent::TelemetryBindingRecovered {
            request_id: "reply".into(),
            result: Box::new(result),
        };
        let bytes = serde_json::to_vec(&event).unwrap();
        assert_eq!(
            bytes,
            serde_json::to_vec(&serde_json::from_slice::<MachineEvent>(&bytes).unwrap()).unwrap()
        );
    }

    #[test]
    fn every_confirmation_field_is_bound_and_audit_cannot_disagree_with_binding() {
        let request = fixture();
        let observation = observed_fixture(&request);
        assert!(observation.matches(&request));
        for field in ["identity", "actor", "operation", "deadline", "step"] {
            let mut changed = request.clone();
            match field {
                "identity" => changed.resolution_id.push('x'),
                "actor" => {
                    changed.actor = RecoveryActor::Admin {
                        account: "different".into(),
                    }
                }
                "operation" => changed.service_operation_digest = binding_digest(b"changed"),
                "deadline" => changed.expires_at_ms += 1,
                _ => {
                    changed.step.expires_at_ms += 1;
                    changed.expected_observation_digest = binding_digest(
                        &serde_json::to_vec(&prepared(&changed.step).unwrap()).unwrap(),
                    );
                }
            }
            assert_ne!(
                request.digest().unwrap(),
                changed.digest().unwrap(),
                "{field}"
            );
            assert!(!observation.matches(&changed));
        }
        for field in ["prepared", "missing", "unavailable", "time", "actor"] {
            let mut changed = observation.clone();
            let RecoveryObservation::Observed { snapshot } = &mut changed else {
                panic!()
            };
            match field {
                "prepared" => snapshot.binding = prepared(&request.step).unwrap(),
                "missing" => {
                    let BindingObservation::Observed { snapshot } = &mut snapshot.binding else {
                        panic!()
                    };
                    snapshot.receipt = None;
                }
                "unavailable" => {
                    snapshot.binding = BindingObservation::Unavailable {
                        reason: BindingUnavailable::Storage,
                    }
                }
                "time" => snapshot.receipt.as_mut().unwrap().resolved_at_ms = request.expires_at_ms,
                _ => {
                    snapshot.receipt.as_mut().unwrap().request.actor = RecoveryActor::Admin {
                        account: "different".into(),
                    }
                }
            }
            assert!(!changed.matches(&request), "{field}");
        }
    }
}
