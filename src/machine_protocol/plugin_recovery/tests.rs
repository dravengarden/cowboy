use super::*;
use crate::machine_protocol::plugin_step::{StepUncertainty, digest, fixture};
use crate::machine_protocol::{
    MachineCommand, MachineEvent, PLUGIN_RECOVERY_OBSERVATION_PROTOCOL_VERSION,
};

fn revision(value: char) -> InstallationRevision {
    format!("installation-{}", value.to_string().repeat(64))
        .try_into()
        .unwrap()
}

fn evidence() -> (UninstallStep, RecoverySnapshot) {
    let mut step = fixture();
    step.schema = 2;
    step.installation_revision = Some(revision('a'));
    let snapshot = RecoverySnapshot {
        request_digest: step.request_digest().unwrap(),
        receipt: Some(Box::new(StepReceipt {
            step: step.clone(),
            request_digest: step.request_digest().unwrap(),
            outcome: StepOutcome::Applied {},
        })),
        installation: InstallationEvidence::Removed {
            revision: revision('b'),
            previous_revision: revision('a'),
            uninstall_request_digest: step.request_digest().unwrap(),
        },
        slot_fenced: false,
    };
    (step, snapshot)
}

fn observe(snapshot: RecoverySnapshot) -> RecoveryObservation {
    RecoveryObservation::Observed {
        snapshot: Box::new(snapshot),
    }
}

#[test]
fn matching_removal_requires_original_request_and_installation_not_just_absence() {
    let (step, snapshot) = evidence();
    assert_eq!(
        observe(snapshot.clone()).basis(&step),
        RecoveryBasis::MatchingRemoval {
            tombstone_revision: revision('b')
        }
    );
    for installation in [
        InstallationEvidence::Installed {
            revision: revision('c'),
            generation_digest: step.generation_digest.clone(),
        },
        InstallationEvidence::Removed {
            revision: revision('b'),
            previous_revision: revision('c'),
            uninstall_request_digest: step.request_digest().unwrap(),
        },
        InstallationEvidence::Removed {
            revision: revision('b'),
            previous_revision: revision('a'),
            uninstall_request_digest: digest(b"different operation"),
        },
        InstallationEvidence::Untracked {},
    ] {
        let mut changed = snapshot.clone();
        changed.installation = installation;
        assert_eq!(
            observe(changed).basis(&step),
            RecoveryBasis::InstallationChanged {}
        );
    }
}

#[test]
fn missing_and_unknown_receipts_are_not_completed_by_a_matching_tombstone() {
    let (step, mut snapshot) = evidence();
    snapshot.receipt = None;
    assert_eq!(
        observe(snapshot.clone()).basis(&step),
        RecoveryBasis::Unknown {
            reason: RecoveryUncertainty::MissingForwardReceipt
        }
    );
    let (_, mut snapshot) = evidence();
    snapshot.receipt.as_mut().unwrap().outcome = StepOutcome::Unknown {
        reason: StepUncertainty::Interrupted,
    };
    snapshot.slot_fenced = true;
    assert_eq!(
        observe(snapshot).basis(&step),
        RecoveryBasis::Unknown {
            reason: RecoveryUncertainty::ForwardOutcome
        }
    );
}

#[test]
fn rejection_and_legacy_receipts_never_supply_a_restoration_cas_basis() {
    let (mut step, mut snapshot) = evidence();
    snapshot.receipt.as_mut().unwrap().outcome = StepOutcome::Rejected {
        reason: StepRejection::Expired,
    };
    assert_eq!(
        observe(snapshot.clone()).basis(&step),
        RecoveryBasis::ForwardRejected {
            reason: StepRejection::Expired
        }
    );
    step.schema = 1;
    step.installation_revision = None;
    snapshot.request_digest = step.request_digest().unwrap();
    snapshot.receipt = Some(Box::new(StepReceipt {
        step: step.clone(),
        request_digest: snapshot.request_digest.clone(),
        outcome: StepOutcome::Applied {},
    }));
    assert_eq!(
        observe(snapshot).basis(&step),
        RecoveryBasis::LegacyUntracked {}
    );
}

#[test]
fn pending_conflicting_and_unavailable_slots_never_offer_matching_removal() {
    let (step, mut snapshot) = evidence();
    snapshot.slot_fenced = true;
    assert_eq!(
        observe(snapshot.clone()).basis(&step),
        RecoveryBasis::SlotFenced {}
    );
    snapshot.installation = InstallationEvidence::Pending {
        revision: revision('c'),
    };
    assert_eq!(
        observe(snapshot.clone()).basis(&step),
        RecoveryBasis::Unknown {
            reason: RecoveryUncertainty::InstallationPending
        }
    );
    snapshot.installation = InstallationEvidence::Unavailable {
        reason: InstallationUnavailable::ActiveLinkMismatch,
    };
    assert_eq!(
        observe(snapshot).basis(&step),
        RecoveryBasis::Unknown {
            reason: RecoveryUncertainty::InstallationUnavailable
        }
    );
}

#[test]
fn evidence_checks_complete_identity_and_internal_invariants() {
    let (step, snapshot) = evidence();
    let mut changed = snapshot.clone();
    changed.request_digest = digest(b"different query");
    assert!(!observe(changed).matches(&step));
    let mut changed = snapshot.clone();
    changed.receipt.as_mut().unwrap().step.plan_digest = digest(b"different actor");
    assert!(!observe(changed).matches(&step));
    let mut changed = snapshot.clone();
    changed.installation = InstallationEvidence::Removed {
        revision: revision('b'),
        previous_revision: revision('b'),
        uninstall_request_digest: step.request_digest().unwrap(),
    };
    assert!(!observe(changed).matches(&step));
    let mut changed = snapshot.clone();
    changed.installation = InstallationEvidence::Installed {
        revision: revision('a'),
        generation_digest: "sha256:BAD".into(),
    };
    assert!(!observe(changed).matches(&step));
    let mut changed = snapshot.clone();
    changed.installation = InstallationEvidence::Pending {
        revision: revision('b'),
    };
    assert!(!observe(changed).matches(&step));
    let mut changed = snapshot;
    changed.receipt.as_mut().unwrap().outcome = StepOutcome::Unknown {
        reason: StepUncertainty::EffectFailure,
    };
    assert!(!observe(changed).matches(&step));
}

#[test]
fn closed_wire_is_read_only_protocol_twelve_with_no_authority_fields() {
    let (step, snapshot) = evidence();
    let command = MachineCommand::QueryPluginUninstallRecovery {
        request_id: "query".into(),
        step: Box::new(step),
    };
    assert_eq!(
        command.minimum_protocol(),
        PLUGIN_RECOVERY_OBSERVATION_PROTOCOL_VERSION
    );
    assert_eq!(
        serde_json::from_slice::<MachineCommand>(&serde_json::to_vec(&command).unwrap()).unwrap(),
        command
    );
    let event = MachineEvent::PluginUninstallRecovery {
        request_id: "query".into(),
        observation: Box::new(observe(snapshot.clone())),
    };
    assert_eq!(
        serde_json::from_slice::<MachineEvent>(&serde_json::to_vec(&event).unwrap()).unwrap(),
        event
    );
    let mut wire = serde_json::to_value(observe(snapshot)).unwrap();
    wire["authorized"] = true.into();
    assert!(serde_json::from_value::<RecoveryObservation>(wire).is_err());
    assert!(
        serde_json::from_str::<InstallationEvidence>(r#"{"state":"untracked","restore":true}"#)
            .is_err()
    );
    assert!(serde_json::from_str::<InstallationEvidence>(r#"{"state":"restored"}"#).is_err());
    assert!(
        serde_json::from_str::<InstallationEvidence>(
            r#"{"state":"untracked","state":"untracked"}"#
        )
        .is_err()
    );
}
