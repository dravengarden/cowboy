use super::*;
use crate::machine_protocol::{
    MachineCommand,
    telemetry_recovery::{RecoveryRequest, fixture, observed_fixture},
};

fn evidence(request: &RecoveryRequest) -> (RecoveryAuditQuery, RecoveryAuditObservation) {
    let query = RecoveryAuditQuery {
        schema: 1,
        step: request.step.clone(),
    };
    let super::super::telemetry_recovery::RecoveryObservation::Observed { snapshot } =
        observed_fixture(request)
    else {
        panic!()
    };
    let observation = RecoveryAuditObservation::Observed {
        snapshot: Box::new(RecoveryAuditSnapshot {
            query_digest: query.digest().unwrap(),
            receipt: snapshot.receipt,
            binding: snapshot.binding,
        }),
    };
    (query, observation)
}

#[test]
fn audit_is_a_closed_read_only_purpose_with_its_own_protocol_and_digest() {
    let mut request = fixture();
    request.step.expires_at_ms = 1;
    request.expected_observation_digest = binding_digest(
        &serde_json::to_vec(&super::super::telemetry_recovery::prepared(&request.step).unwrap())
            .unwrap(),
    );
    let (query, observation) = evidence(&request);
    assert!(observation.matches(&query));
    let encoded = serde_json::to_value(&query).unwrap();
    assert!(serde_json::from_value::<RecoveryRequest>(encoded.clone()).is_err());
    assert!(
        serde_json::from_value::<RecoveryAuditQuery>(serde_json::to_value(&request).unwrap())
            .is_err()
    );
    for field in ["actor", "expires_at_ms", "resolution_id", "grant"] {
        let mut changed = encoded.clone();
        changed[field] = serde_json::json!("not authority");
        assert!(serde_json::from_value::<RecoveryAuditQuery>(changed).is_err());
    }
    let command = MachineCommand::QueryTelemetryRecoveryAudit {
        request_id: "audit-query".into(),
        query: Box::new(query.clone()),
    };
    assert_eq!(command.minimum_protocol(), 18);
    let wire = serde_json::to_string(&command).unwrap();
    assert!(matches!(
        serde_json::from_str::<MachineCommand>(&wire).unwrap(),
        MachineCommand::QueryTelemetryRecoveryAudit { .. }
    ));
    let mut changed = query.clone();
    changed.step.plan_digest = binding_digest(b"foreign intent");
    assert!(!observation.matches(&changed));
    changed = query.clone();
    changed.schema = 2;
    assert!(changed.digest().is_err());
    changed = query;
    changed.step.schema = 1;
    changed.step.expected_namespace = None;
    assert!(changed.digest().is_err());
}

#[test]
fn audit_checks_full_binding_linkage_and_never_folds_unavailable_into_absence() {
    let (query, observation) = evidence(&fixture());
    for boundary in ["digest", "step", "time", "binding", "unavailable"] {
        let mut changed = observation.clone();
        let RecoveryAuditObservation::Observed { snapshot } = &mut changed else {
            panic!()
        };
        match boundary {
            "digest" => snapshot.query_digest = binding_digest(b"wrong query"),
            "step" => {
                snapshot.receipt.as_mut().unwrap().request.step.plan_digest =
                    binding_digest(b"wrong step");
            }
            "time" => snapshot.receipt.as_mut().unwrap().resolved_at_ms = 0,
            "binding" => {
                let BindingObservation::Observed { snapshot } = &mut snapshot.binding else {
                    panic!()
                };
                snapshot.receipt = None;
            }
            _ => {
                snapshot.receipt = None;
                snapshot.binding = BindingObservation::Unavailable {
                    reason: BindingUnavailable::Storage,
                };
            }
        }
        assert!(!changed.matches(&query), "{boundary}");
    }
}
