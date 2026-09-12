use super::*;
use crate::machine_protocol::telemetry_binding::{BindingRejection, BindingUnavailable};
use crate::operation_budget::TimeSample;
use crate::telemetry_binding::{
    tests::{applied, observed},
    writer::{Change, apply},
};
use std::time::Duration;

pub(crate) fn intent(
    before: &Operation,
    observation: Option<&BindingObservation>,
) -> ResolutionIntent {
    let action = match observation {
        Some(observation) => ResolutionAction::AcceptApplied {
            observation_digest: binding_digest(&serde_json::to_vec(observation).unwrap()),
        },
        None => ResolutionAction::AbortBeforeDispatch,
    };
    ResolutionIntent::new(
        "binding-resolution-fixture".into(),
        Actor::Product {
            user_id: "new-operator".into(),
        },
        before,
        action,
        chrono::Utc::now().timestamp_millis() + 60_000,
    )
    .unwrap()
}

pub(crate) fn permit(
    intent: &ResolutionIntent,
    before: &Operation,
    observation: Option<BindingObservation>,
) -> ResolutionPermit {
    ResolutionPermit::new(
        intent.clone(),
        before.clone(),
        observation,
        OperationBudget::new(
            intent.expires_at_ms,
            Duration::from_mins(1),
            TimeSample::now(),
        ),
    )
    .unwrap()
}

fn pending(progress: Progress) -> (Option<Ledger>, Operation) {
    let request = fixture("resolution");
    let mut ledger = None;
    let mut operation = apply(&mut ledger, &Change::Begin(&request))
        .unwrap()
        .operation;
    if progress != Progress::Prepared {
        operation = apply(
            &mut ledger,
            &Change::Advance {
                expected: &operation,
                progress,
            },
        )
        .unwrap()
        .operation;
    }
    (ledger, operation)
}

#[test]
fn resolution_is_atomic_full_cas_and_only_prepared_can_abort_offline() {
    let (mut ledger, before) = pending(Progress::Prepared);
    let intent = intent(&before, None);
    let permit = permit(&intent, &before, None);
    let old = ledger.clone();
    let result = apply(&mut ledger, &Change::Resolve(&permit)).unwrap();
    assert!(!result.admitted);
    assert_eq!(result.operation.progress, Progress::Aborted);
    assert!(ledger.as_ref().unwrap().current.is_none());
    assert_eq!(ledger.as_ref().unwrap().resolutions.len(), 1);
    let saved = ledger.clone();
    apply(&mut ledger, &Change::Resolve(&permit)).unwrap();
    assert_eq!(saved, ledger);
    assert!(
        apply(
            &mut ledger,
            &Change::Advance {
                expected: &before,
                progress: Progress::Dispatching
            }
        )
        .is_err()
    );
    let mut racing = old;
    apply(
        &mut racing,
        &Change::Advance {
            expected: &before,
            progress: Progress::Dispatching,
        },
    )
    .unwrap();
    let saved = racing.clone();
    assert!(apply(&mut racing, &Change::Resolve(&permit)).is_err());
    assert_eq!(saved, racing);
    let (_, attention) = pending(Progress::NeedsAttention {
        reason: Attention::Uncertain,
        observation: None,
    });
    assert!(
        ResolutionIntent::new(
            intent.resolution_id,
            intent.actor,
            &attention,
            ResolutionAction::AbortBeforeDispatch,
            intent.expires_at_ms
        )
        .is_err()
    );
}

#[test]
fn only_exact_current_terminal_machine_evidence_can_resolve_dispatch() {
    let (ledger, before) = pending(Progress::Dispatching);
    let good = applied(&before.intent);
    let request = intent(&before, Some(&good));
    for change in [
        "unknown",
        "prepared",
        "unavailable",
        "receipt",
        "head",
        "digest",
        "unresolved",
        "wrong-outcome",
    ] {
        let mut bad = good.clone();
        match change {
            "unknown" => bad = observed(&before.intent, BindingOutcome::Unknown {}),
            "prepared" => bad = observed(&before.intent, BindingOutcome::Prepared {}),
            "unavailable" => {
                bad = BindingObservation::Unavailable {
                    reason: BindingUnavailable::Storage,
                }
            }
            "wrong-outcome" => {
                bad = observed(
                    &before.intent,
                    BindingOutcome::Rejected {
                        reason: BindingRejection::Expired,
                    },
                )
            }
            _ => {
                if let BindingObservation::Observed { snapshot } = &mut bad {
                    match change {
                        "receipt" => snapshot.receipt = None,
                        "head" => snapshot.current = Some(BindingSnapshot::initial()),
                        "digest" => snapshot.request_digest = binding_digest(b"changed"),
                        _ => snapshot.unresolved = true,
                    }
                }
            }
        }
        assert!(
            request.conclusion(&before, Some(bad.clone())).is_err(),
            "{change}"
        );
        // Even a new confirmation of bad evidence cannot make it terminal.
        let fresh = intent(&before, Some(&bad));
        assert!(fresh.conclusion(&before, Some(bad)).is_err(), "{change}");
    }
    let mut completed = ledger.clone();
    apply(
        &mut completed,
        &Change::Resolve(&permit(&request, &before, Some(good))),
    )
    .unwrap();
    assert_eq!(
        completed.unwrap().current,
        Some(before.intent.machine_step().unwrap().after().unwrap())
    );
    let rejected = observed(
        &before.intent,
        BindingOutcome::Rejected {
            reason: BindingRejection::Expired,
        },
    );
    let mut request = request;
    request.action = ResolutionAction::RecordRejected {
        observation_digest: binding_digest(&serde_json::to_vec(&rejected).unwrap()),
    };
    let mut rejected_ledger = ledger;
    apply(
        &mut rejected_ledger,
        &Change::Resolve(&permit(&request, &before, Some(rejected))),
    )
    .unwrap();
    assert_eq!(
        rejected_ledger.unwrap().current,
        Some(BindingSnapshot::initial())
    );
}

#[test]
fn schema_two_audit_roundtrips_and_rechecksummed_structural_corruption_fails_closed() {
    let (mut ledger, before) = pending(Progress::NeedsAttention {
        reason: Attention::Uncertain,
        observation: None,
    });
    let observation = applied(&before.intent);
    let request = intent(&before, Some(&observation));
    // Generic Advance must not acquire the independent resolution purpose.
    assert!(
        apply(
            &mut ledger.clone(),
            &Change::Advance {
                expected: &before,
                progress: Progress::Completed {
                    observation: observation.clone()
                }
            }
        )
        .is_err()
    );
    apply(
        &mut ledger,
        &Change::Resolve(&permit(&request, &before, Some(observation))),
    )
    .unwrap();
    let ledger = ledger.unwrap();
    assert_eq!(ledger.schema, 2);
    let encoded = ledger.encode(&request.service_id).unwrap();
    assert_eq!(
        Ledger::decode(&encoded, &request.service_id).unwrap(),
        ledger
    );
    for change in [
        "schema",
        "missing",
        "duplicate",
        "before",
        "digest",
        "after",
        "owner",
        "deadline",
        "field",
    ] {
        let mut value = serde_json::to_value(&ledger).unwrap();
        match change {
            "schema" => value["schema"] = 1.into(),
            "missing" => value["resolutions"] = serde_json::json!([]),
            "duplicate" => {
                let record = value["resolutions"][0].clone();
                value["resolutions"].as_array_mut().unwrap().push(record);
            }
            "before" => {
                value["resolutions"][0]["before"]["progress"]["reason"] = "head_changed".into()
            }
            "digest" => {
                value["resolutions"][0]["intent"]["operation_digest"] =
                    String::from(binding_digest(b"changed")).into()
            }
            "after" => value["resolutions"][0]["after"] = serde_json::json!({"phase":"aborted"}),
            "owner" => value["resolutions"][0]["intent"]["machine_id"] = "another-machine".into(),
            "deadline" => value["resolutions"][0]["resolved_at_ms"] = request.expires_at_ms.into(),
            _ => value["resolutions"][0]["grant"] = true.into(),
        }
        assert!(
            Ledger::decode(&value.to_string(), &request.service_id).is_err(),
            "{change}"
        );
    }
}

#[test]
fn resolution_budget_is_sticky_and_conflicting_ids_never_rewrite_history() {
    let (mut ledger, before) = pending(Progress::Prepared);
    let request = intent(&before, None);
    let expired = permit(&request, &before, None);
    expired.expire_for_test();
    let saved = ledger.clone();
    assert!(apply(&mut ledger, &Change::Resolve(&expired)).is_err());
    assert_eq!(saved, ledger);
    apply(
        &mut ledger,
        &Change::Resolve(&permit(&request, &before, None)),
    )
    .unwrap();
    let saved = ledger.clone();
    let mut conflicting = request;
    conflicting.resolution_id.push_str("-again");
    assert!(
        apply(
            &mut ledger,
            &Change::Resolve(&permit(&conflicting, &before, None))
        )
        .is_err()
    );
    assert_eq!(saved, ledger);
    let mut next = before.intent.clone();
    next.operation_id.push_str("-next");
    let next = apply(&mut ledger, &Change::Begin(&next)).unwrap().operation;
    let mut collision = intent(&next, None);
    collision.resolution_id = conflicting.resolution_id.trim_end_matches("-again").into();
    let saved = ledger.clone();
    assert!(
        apply(
            &mut ledger,
            &Change::Resolve(&permit(&collision, &next, None))
        )
        .is_err()
    );
    assert_eq!(saved, ledger);
}

#[test]
fn resolved_forward_can_only_restore_its_exact_prior_selection_at_a_new_epoch() {
    let (mut ledger, before) = pending(Progress::Dispatching);
    let observation = applied(&before.intent);
    let request = intent(&before, Some(&observation));
    apply(
        &mut ledger,
        &Change::Resolve(&permit(&request, &before, Some(observation))),
    )
    .unwrap();
    let step = before.intent.machine_step().unwrap();
    let after = step.after().unwrap();
    let mut restore = fixture("resolved-restore");
    restore.expected = Some(after.clone());
    restore.change = BindingChange::Restore {
        forward_request_digest: step.request_digest().unwrap(),
        selection: None,
        policy_epoch: after.policy_epoch.next().unwrap(),
    };
    let prepared = apply(&mut ledger, &Change::Begin(&restore))
        .unwrap()
        .operation;
    let mut abort = intent(&prepared, None);
    abort.resolution_id.push_str("-second");
    apply(
        &mut ledger,
        &Change::Resolve(&permit(&abort, &prepared, None)),
    )
    .unwrap();
    assert_eq!(ledger.as_ref().unwrap().current, Some(after));
    let mut reordered = ledger.clone().unwrap();
    reordered.resolutions.reverse();
    assert!(reordered.encode(&request.service_id).is_err());
    restore.operation_id.push_str("-confirm-again");
    let prepared = apply(&mut ledger, &Change::Begin(&restore))
        .unwrap()
        .operation;
    let dispatched = apply(
        &mut ledger,
        &Change::Advance {
            expected: &prepared,
            progress: Progress::Dispatching,
        },
    )
    .unwrap()
    .operation;
    apply(
        &mut ledger,
        &Change::Advance {
            expected: &dispatched,
            progress: Progress::Completed {
                observation: applied(&restore),
            },
        },
    )
    .unwrap();
    let head = ledger.unwrap().current.unwrap();
    assert!(head.revision > step.after().unwrap().revision);
    assert!(head.policy_epoch > step.after().unwrap().policy_epoch);
    assert!(head.selection.is_none());
}
