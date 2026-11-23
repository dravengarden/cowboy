use super::*;
use crate::machine_protocol::telemetry_binding::{
    BindingCommitFailure, BindingCommitResult, BindingObservation, BindingOutcome, BindingRejection,
};
use crate::machine_protocol::telemetry_recovery::{RecoveryObservation, RecoveryResult, prepared};

async fn event(socket: &mut Socket) -> Result<MachineEvent, Failure> {
    loop {
        match receive(socket).await? {
            MachineFrame::Heartbeat { .. }
            | MachineFrame::Event {
                event: MachineEvent::Inventory { .. },
            } => {}
            MachineFrame::Event {
                event: MachineEvent::PluginInventory { plugins, .. },
            } if plugins.is_empty() => {}
            MachineFrame::Event { event } => return Ok(event),
            _ => return Err(Failure::WrongObservation),
        }
    }
}

pub(super) async fn exercise(
    socket: &mut Socket,
    scenario: Scenario,
    policy: PolicyCase,
    state: &mut State,
    root: &Path,
    cold_read: u8,
) -> Result<(), Failure> {
    let admitted = policy.admits(scenario.purpose());
    let expected_before = state.evidence.machine.clone();
    match scenario {
        Scenario::MachineBinding => {
            let step = &state.evidence.step;
            send(
                socket,
                MachineFrame::Command {
                    command: MachineCommand::CommitTelemetryBinding {
                        request_id: "writer-binding-once".into(),
                        step: Box::new(step.clone()),
                    },
                },
            )
            .await?;
            let MachineEvent::TelemetryBindingCommitted { request_id, result } =
                event(socket).await?
            else {
                return Err(Failure::WrongObservation);
            };
            let mut expected = prepared(step).map_err(|_| Failure::Setup)?;
            let BindingObservation::Observed { snapshot } = &mut expected else {
                return Err(Failure::Setup);
            };
            let after = step.after().map_err(|_| Failure::Setup)?;
            snapshot.current = Some(after.clone());
            snapshot.unresolved = false;
            snapshot.receipt.as_mut().ok_or(Failure::Setup)?.outcome =
                BindingOutcome::Applied { after };
            let expected_result = if admitted {
                BindingCommitResult::Observed {
                    observation: expected.clone(),
                }
            } else {
                BindingCommitResult::Unavailable {
                    failure: BindingCommitFailure::ReaderOnly,
                }
            };
            if request_id != "writer-binding-once" || *result != expected_result {
                return Err(Failure::WrongObservation);
            }
            if admitted {
                state.evidence.binding = expected.clone();
                let RecoveryObservation::Observed { snapshot } = &mut state.evidence.audit else {
                    return Err(Failure::Setup);
                };
                snapshot.binding = expected;
            }
        }
        Scenario::MachineRecovery => {
            let request = &state.evidence.recovery;
            send(
                socket,
                MachineFrame::Command {
                    command: MachineCommand::RecoverTelemetryBinding {
                        request_id: "writer-recovery-once".into(),
                        recovery: Box::new(request.clone()),
                    },
                },
            )
            .await?;
            let MachineEvent::TelemetryBindingRecovered { request_id, result } =
                event(socket).await?
            else {
                return Err(Failure::WrongObservation);
            };
            if request_id != "writer-recovery-once" {
                return Err(Failure::WrongObservation);
            }
            if admitted {
                let RecoveryResult::Observed { observation } = *result else {
                    return Err(Failure::WrongObservation);
                };
                let RecoveryObservation::Observed { snapshot } = &observation else {
                    return Err(Failure::WrongObservation);
                };
                let mut expected = prepared(&request.step).map_err(|_| Failure::Setup)?;
                let BindingObservation::Observed { snapshot: binding } = &mut expected else {
                    return Err(Failure::Setup);
                };
                binding.unresolved = false;
                binding.receipt.as_mut().ok_or(Failure::Setup)?.outcome =
                    BindingOutcome::Rejected {
                        reason: BindingRejection::AuthorizationEnded,
                    };
                if !observation.matches(request)
                    || snapshot.receipt.is_none()
                    || snapshot.binding != expected
                    || (cold_read == 2 && observation != state.evidence.audit)
                {
                    return Err(Failure::WrongObservation);
                }
                state.evidence.binding = expected;
                state.evidence.audit = observation;
            } else if *result
                != (RecoveryResult::Unavailable {
                    failure: BindingCommitFailure::ReaderOnly,
                })
            {
                return Err(Failure::WrongObservation);
            }
        }
        Scenario::ServiceResolution => return Err(Failure::Setup),
    }
    if admitted && cold_read == 1 {
        let bytes = std::fs::read(root.join("machine").join(JOURNAL))
            .map_err(|_| Failure::EvidenceChanged)?;
        if expected_before.as_ref() == Some(&bytes) {
            return Err(Failure::EvidenceChanged);
        }
        state.evidence.machine = Some(bytes);
    }
    // Every rejection and every duplicate after reopen must leave exact bytes.
    unchanged(root, &state.evidence).map_err(|_| Failure::EvidenceChanged)
}
