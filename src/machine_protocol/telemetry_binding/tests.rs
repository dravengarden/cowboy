use super::*;

#[test]
fn counters_preserve_u64_exactly_and_reject_noncanonical_wire_values() {
    for raw in ["0", "1", "9007199254740993", "18446744073709551615"] {
        let json = format!("\"{raw}\"");
        let revision: BindingRevision = serde_json::from_str(&json).unwrap();
        let epoch: PolicyEpoch = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&revision).unwrap(), json);
        assert_eq!(serde_json::to_string(&epoch).unwrap(), json);
    }
    for raw in [
        "0",
        "null",
        "true",
        "{}",
        "\"01\"",
        "\"+1\"",
        "\"-1\"",
        "\"1.0\"",
        "\" 1\"",
        "\"18446744073709551616\"",
    ] {
        assert!(serde_json::from_str::<BindingRevision>(raw).is_err());
        assert!(serde_json::from_str::<PolicyEpoch>(raw).is_err());
    }
}

#[test]
fn steps_bind_exact_intent_and_never_lower_policy_or_reuse_revision() {
    let first = fixture();
    first.validate().unwrap();
    let after = first.after().unwrap();
    assert_eq!(after.revision, BindingRevision(1));
    assert_eq!(after.policy_epoch, PolicyEpoch(1));
    let mut next = first.clone();
    next.expected = after;
    next.change = BindingChange::Revoke {
        policy_epoch: PolicyEpoch(2),
    };
    next.validate().unwrap();
    let revoked = next.after().unwrap();
    assert_eq!(revoked.revision, BindingRevision(2));
    assert!(revoked.selection.is_none());
    next.expected = revoked;
    next.change = first.change.clone();
    assert!(
        next.validate().is_err(),
        "restoring old configuration must not lower policy epoch"
    );
    next.expected.policy_epoch = PolicyEpoch(1);
    next.expected.revision = BindingRevision(u64::MAX);
    assert!(
        next.validate().is_err(),
        "revision overflow must not wrap to the initial slot"
    );
    for field in [
        "operation_id",
        "service_id",
        "machine_id",
        "plan_digest",
        "expires_at_ms",
    ] {
        let mut changed = serde_json::to_value(&first).unwrap();
        changed[field] = match field {
            "plan_digest" => {
                serde_json::to_value(binding_digest(b"different actor or authorization")).unwrap()
            }
            "expires_at_ms" => serde_json::json!(1_800_000_000_000_i64),
            _ => serde_json::json!("different-identity-0001"),
        };
        let changed: BindingStep = serde_json::from_value(changed).unwrap();
        assert_ne!(
            changed.request_digest().unwrap(),
            first.request_digest().unwrap()
        );
    }
}

#[test]
fn binding_codecs_are_closed_and_do_not_accept_secret_or_authority_fields() {
    let step = fixture();
    for (pointer, field) in [
        ("", "authorized"),
        ("/change", "endpoint"),
        ("/expected", "token"),
        ("/change/installation", "auth_generation"),
    ] {
        let mut value = serde_json::to_value(&step).unwrap();
        value.pointer_mut(pointer).unwrap()[field] = serde_json::json!("fixture-private-value");
        assert!(serde_json::from_value::<BindingStep>(value).is_err());
    }
    let json = serde_json::to_string(&step).unwrap();
    let duplicated = json.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1);
    assert!(serde_json::from_str::<BindingStep>(&duplicated).is_err());
    for value in [
        "sha256:abc".into(),
        format!("sha256:{}", "A".repeat(64)),
        "installation-123".into(),
    ] {
        assert!(BindingDigest::try_from(value).is_err());
    }
    let mut invalid = step;
    invalid.expected.selection = invalid.after().unwrap().selection;
    assert!(
        invalid.validate().is_err(),
        "initial state cannot smuggle an active binding"
    );
}

#[test]
fn observation_correlates_missing_receipts_and_rejects_forged_head_or_completion() {
    let step = fixture();
    let receipt = BindingReceipt {
        request_digest: step.request_digest().unwrap(),
        step: step.clone(),
        outcome: BindingOutcome::Applied {
            after: step.after().unwrap(),
        },
    };
    let mut snapshot = BindingObservationSnapshot {
        request_digest: receipt.request_digest.clone(),
        receipt: Some(Box::new(receipt)),
        current: Some(step.after().unwrap()),
        unresolved: false,
    };
    let check = |snapshot: &BindingObservationSnapshot| {
        BindingObservation::Observed {
            snapshot: Box::new(snapshot.clone()),
        }
        .matches(&step)
    };
    assert!(check(&snapshot));
    snapshot.current = Some(BindingSnapshot::initial());
    assert!(!check(&snapshot));
    snapshot.current = Some(step.after().unwrap());
    snapshot.current.as_mut().unwrap().selection = None;
    assert!(
        !check(&snapshot),
        "same revision cannot describe a different selection"
    );
    snapshot.receipt.as_mut().unwrap().outcome = BindingOutcome::Unknown {};
    snapshot.current = Some(step.expected.clone());
    assert!(!check(&snapshot));
    snapshot.unresolved = true;
    assert!(check(&snapshot));
    snapshot.receipt = None;
    snapshot.request_digest = binding_digest(b"foreign request");
    assert!(
        !check(&snapshot),
        "NotFound is also bound to the complete query"
    );
}

#[test]
fn protocol_fourteen_only_adds_read_only_binding_queries() {
    use crate::machine_protocol::{MachineCommand, MachineEvent};
    let command = MachineCommand::QueryTelemetryBinding {
        request_id: "rpc-only".into(),
        step: Box::new(fixture()),
    };
    assert_eq!(command.minimum_protocol(), 14);
    let encoded = serde_json::to_string(&command).unwrap();
    assert_eq!(
        serde_json::from_str::<MachineCommand>(&encoded).unwrap(),
        command
    );
    assert!(
        serde_json::from_str::<MachineCommand>(
            &encoded.replace("query_telemetry_binding", "apply_telemetry_binding")
        )
        .is_err()
    );
    let event = MachineEvent::TelemetryBindingObservation {
        request_id: "rpc-only".into(),
        observation: Box::new(BindingObservation::Unavailable {
            reason: BindingUnavailable::WrongOwner,
        }),
    };
    assert_eq!(
        serde_json::from_str::<MachineEvent>(&serde_json::to_string(&event).unwrap()).unwrap(),
        event
    );
}
