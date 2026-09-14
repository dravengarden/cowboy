use super::*;

#[test]
fn install_identity_and_enums_are_closed_and_bounded() {
    let step = fixture();
    step.validate().unwrap();
    let original = step.request_digest().unwrap();
    for field in [
        "operation_id",
        "service_id",
        "machine_id",
        "plugin_id",
        "plugin_version",
        "plan_digest",
        "generation_digest",
        "contract_fingerprint",
        "envelope_digest",
        "expires_at_ms",
        "expected",
    ] {
        let mut json = serde_json::to_value(&step).unwrap();
        json[field] = match field {
            "expires_at_ms" => (step.expires_at_ms + 1).into(),
            "plugin_version" => "1.0.1".into(),
            "expected" => {
                serde_json::json!({"state":"removed", "revision": format!("installation-{}", "b".repeat(64))})
            }
            field if field.ends_with("digest") || field == "contract_fingerprint" => {
                digest(b"changed").into()
            }
            _ => "changed-identity-0001".into(),
        };
        let changed: InstallStep = serde_json::from_value(json).unwrap();
        assert_ne!(changed.request_digest().unwrap(), original, "{field}");
    }
    for (field, value) in [
        ("schema", serde_json::json!(2)),
        ("operation_id", serde_json::json!("short")),
        ("plugin_id", serde_json::json!("../escape")),
        ("service_id", serde_json::json!("service\n")),
        ("expires_at_ms", serde_json::json!(0)),
        ("envelope_digest", serde_json::json!("sha256:BAD")),
    ] {
        let mut json = serde_json::to_value(&step).unwrap();
        json[field] = value;
        assert!(
            serde_json::from_value::<InstallStep>(json)
                .unwrap()
                .validate()
                .is_err(),
            "{field}"
        );
    }
    let mut json = serde_json::to_value(step).unwrap();
    json["resume"] = true.into();
    assert!(serde_json::from_value::<InstallStep>(json).is_err());
    assert!(
        serde_json::from_value::<InstallTarget>(
            serde_json::json!({"state":"vacant", "revision":null})
        )
        .is_err()
    );
}

#[test]
fn only_forward_process_owned_phases_can_complete_and_never_reuse_a_revision() {
    let mut step = fixture();
    let revision: InstallationRevision = format!("installation-{}", "b".repeat(64))
        .try_into()
        .unwrap();
    step.expected = InstallTarget::Removed {
        revision: revision.clone(),
    };
    let receipt = InstallReceipt {
        request_digest: step.request_digest().unwrap(),
        step: step.clone(),
        outcome: InstallOutcome::Applied { revision },
    };
    assert!(!receipt.matches(&step));
    let prepared = InstallOutcome::Pending {
        phase: InstallPhase::Prepared,
    };
    let staging = InstallOutcome::Pending {
        phase: InstallPhase::Staging,
    };
    assert!(prepared.fenced());
    assert!(staging.fenced());
    assert!(!receipt.outcome.fenced());
    assert!(staging.follows(&prepared));
    assert!(!prepared.follows(&staging));
    assert!(!receipt.outcome.follows(&prepared));
    let rejected = InstallOutcome::Rejected {
        reason: InstallRejection::Expired,
    };
    assert!(!rejected.fenced());
    assert!(rejected.follows(&prepared));
    assert!(!rejected.follows(&staging));
    assert!(!staging.follows(&rejected));
    let wrong_kind = InstallReceipt {
        outcome: InstallOutcome::Pending {
            phase: InstallPhase::ProjectingAuthentication,
        },
        ..receipt
    };
    assert!(!wrong_kind.matches(&step));
}

#[test]
fn queries_require_protocol_nineteen_and_have_distinct_reply_codecs() {
    use crate::machine_protocol::{
        MachineCommand, MachineEvent, PLUGIN_INSTALL_ATTEMPT_PROTOCOL_VERSION,
    };
    let step = fixture();
    for command in [
        MachineCommand::QueryPluginInstallStep {
            request_id: "query".into(),
            step: Box::new(step.clone()),
        },
        MachineCommand::ObservePluginInstallation {
            request_id: "target".into(),
            query: Box::new(step.target_query()),
        },
    ] {
        assert_eq!(
            command.minimum_protocol(),
            PLUGIN_INSTALL_ATTEMPT_PROTOCOL_VERSION
        );
        assert_eq!(
            serde_json::from_slice::<MachineCommand>(&serde_json::to_vec(&command).unwrap())
                .unwrap(),
            command
        );
    }
    let event = MachineEvent::PluginInstallationStep {
        request_id: "query".into(),
        observation: Box::new(InstallObservation {
            admission_enabled: false,
            result: InstallLookup::NotFound {},
        }),
    };
    assert_eq!(
        serde_json::from_slice::<MachineEvent>(&serde_json::to_vec(&event).unwrap()).unwrap(),
        event
    );
}
