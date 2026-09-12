use super::writer::{Change, apply};
use super::*;
use crate::machine_protocol::telemetry_binding::{BindingObservationSnapshot, BindingReceipt};

pub(crate) fn observed(intent: &Intent, outcome: BindingOutcome) -> BindingObservation {
    let step = intent.machine_step().unwrap();
    let current = match &outcome {
        BindingOutcome::Applied { after } => after.clone(),
        _ => step.expected.clone(),
    };
    let unresolved = matches!(
        outcome,
        BindingOutcome::Prepared {} | BindingOutcome::Unknown {}
    );
    BindingObservation::Observed {
        snapshot: Box::new(BindingObservationSnapshot {
            request_digest: step.request_digest().unwrap(),
            receipt: Some(Box::new(BindingReceipt {
                request_digest: step.request_digest().unwrap(),
                step,
                outcome,
            })),
            current: Some(current),
            unresolved,
        }),
    }
}

pub(crate) fn applied(intent: &Intent) -> BindingObservation {
    observed(
        intent,
        BindingOutcome::Applied {
            after: intent.machine_step().unwrap().after().unwrap(),
        },
    )
}

#[test]
fn plan_covers_actor_namespace_installation_and_closed_fields() {
    let intent = fixture("digest");
    let digest = intent.machine_step().unwrap().request_digest().unwrap();
    for change in ["actor", "namespace", "installation", "expiry"] {
        let mut changed = intent.clone();
        match change {
            "actor" => {
                changed.actor = Actor::Admin {
                    account: "other".into(),
                }
            }
            "namespace" => changed.expected = Some(BindingSnapshot::initial()),
            "installation" => {
                if let BindingChange::Select { installation, .. } = &mut changed.change {
                    installation.installation_revision = format!("installation-{}", "b".repeat(64))
                        .try_into()
                        .unwrap();
                }
            }
            _ => changed.expires_at_ms += 1,
        }
        assert_ne!(
            changed.machine_step().unwrap().request_digest().unwrap(),
            digest,
            "{change}"
        );
    }
    let mut value = serde_json::to_value(&intent).unwrap();
    value["grant"] = true.into();
    assert!(serde_json::from_value::<Intent>(value).is_err());
    let mut changed = intent;
    changed.actor = Actor::Admin {
        account: "bad\nactor".into(),
    };
    assert!(changed.machine_step().is_err());
}

fn complete(ledger: &mut Option<Ledger>, intent: &Intent) {
    let prepared = apply(ledger, &Change::Begin(intent)).unwrap().operation;
    let dispatching = apply(
        ledger,
        &Change::Advance {
            expected: &prepared,
            progress: Progress::Dispatching,
        },
    )
    .unwrap()
    .operation;
    apply(
        ledger,
        &Change::Advance {
            expected: &dispatching,
            progress: Progress::Completed {
                observation: applied(intent),
            },
        },
    )
    .unwrap();
}

#[test]
fn journal_fold_rejects_head_identity_owner_and_schema_corruption() {
    let intent = fixture("reader");
    let mut ledger = None;
    complete(&mut ledger, &intent);
    let ledger = ledger.unwrap();
    let encoded = ledger.encode(&intent.service_id).unwrap();
    assert_eq!(
        Ledger::decode(&encoded, &intent.service_id).unwrap(),
        ledger
    );
    assert!(Ledger::decode(&encoded, "other-service").is_err());
    for change in ["schema", "head", "owner", "duplicate", "outcome", "field"] {
        let mut value = serde_json::to_value(&ledger).unwrap();
        match change {
            "schema" => value["schema"] = 2.into(),
            "head" => value["current"] = serde_json::Value::Null,
            "owner" => value["machine_id"] = "other-machine".into(),
            "duplicate" => {
                let entry = value["operations"][0].clone();
                value["operations"].as_array_mut().unwrap().push(entry);
            }
            "outcome" => {
                value["operations"][0]["progress"]["observation"]["snapshot"]["receipt"]["outcome"] =
                    serde_json::json!({"state":"unknown"})
            }
            _ => value["capability"] = true.into(),
        }
        assert!(
            Ledger::decode(&value.to_string(), &intent.service_id).is_err(),
            "{change}"
        );
    }
}

#[test]
fn restore_requires_completed_exact_forward_and_advances_both_axes() {
    let forward = fixture("forward");
    let mut ledger = None;
    complete(&mut ledger, &forward);
    let step = forward.machine_step().unwrap();
    let after = step.after().unwrap();
    let mut restore = fixture("restore");
    restore.expected = Some(after.clone());
    restore.change = BindingChange::Restore {
        forward_request_digest: step.request_digest().unwrap(),
        selection: None,
        policy_epoch: after.policy_epoch.next().unwrap(),
    };
    for change in ["forward", "selection", "epoch", "machine"] {
        let mut changed = restore.clone();
        if let BindingChange::Restore {
            forward_request_digest,
            selection,
            policy_epoch,
        } = &mut changed.change
        {
            match change {
                "forward" => *forward_request_digest = binding_digest(b"unknown"),
                "selection" => *selection = after.selection.clone(),
                "epoch" => *policy_epoch = after.policy_epoch,
                _ => changed.machine_id = "another-machine".into(),
            }
        }
        assert!(
            apply(&mut ledger.clone(), &Change::Begin(&changed)).is_err(),
            "{change}"
        );
    }
    complete(&mut ledger, &restore);
    let final_head = ledger.as_ref().unwrap().current.as_ref().unwrap();
    assert!(final_head.revision > after.revision && final_head.policy_epoch > after.policy_epoch);
    assert!(final_head.selection.is_none());
    restore.operation_id.push_str("-again");
    assert!(
        apply(&mut ledger, &Change::Begin(&restore)).is_err(),
        "binding ABA cannot restore twice"
    );
}

#[test]
fn prepared_unknown_and_historical_application_never_prove_completion() {
    let intent = fixture("unresolved");
    let mut ledger = None;
    let prepared = apply(&mut ledger, &Change::Begin(&intent))
        .unwrap()
        .operation;
    let dispatching = apply(
        &mut ledger,
        &Change::Advance {
            expected: &prepared,
            progress: Progress::Dispatching,
        },
    )
    .unwrap()
    .operation;
    for outcome in [BindingOutcome::Prepared {}, BindingOutcome::Unknown {}] {
        let observation = observed(&intent, outcome);
        assert!(
            apply(
                &mut ledger.clone(),
                &Change::Advance {
                    expected: &dispatching,
                    progress: Progress::Completed {
                        observation: observation.clone()
                    }
                }
            )
            .is_err()
        );
        let mut fenced = ledger.clone();
        apply(
            &mut fenced,
            &Change::Advance {
                expected: &dispatching,
                progress: Progress::NeedsAttention {
                    reason: Attention::Uncertain,
                    observation: Some(observation),
                },
            },
        )
        .unwrap();
        let next = fixture("other-operation");
        assert!(apply(&mut fenced, &Change::Begin(&next)).is_err());
    }
    let mut historical = applied(&intent);
    if let BindingObservation::Observed { snapshot } = &mut historical {
        let mut next = intent.machine_step().unwrap();
        next.expected = next.after().unwrap();
        next.change = BindingChange::Revoke {
            policy_epoch: next.expected.policy_epoch.next().unwrap(),
        };
        snapshot.current = Some(next.after().unwrap());
    }
    assert!(
        historical.matches(&intent.machine_step().unwrap()),
        "valid historical evidence is not current authority"
    );
    assert!(
        apply(
            &mut ledger,
            &Change::Advance {
                expected: &dispatching,
                progress: Progress::Completed {
                    observation: historical
                }
            }
        )
        .is_err()
    );
}

#[test]
fn duplicate_is_history_and_capacity_does_not_prune_or_reissue() {
    let intent = fixture("duplicate");
    let mut ledger = None;
    let original = apply(&mut ledger, &Change::Begin(&intent))
        .unwrap()
        .operation;
    assert!(
        !apply(&mut ledger, &Change::Begin(&intent))
            .unwrap()
            .admitted
    );
    apply(
        &mut ledger,
        &Change::Advance {
            expected: &original,
            progress: Progress::Aborted,
        },
    )
    .unwrap();
    for index in 1..MAX_OPERATIONS {
        ledger.as_mut().unwrap().operations.push(Operation {
            intent: fixture(&format!("capacity-{index}")),
            progress: Progress::Aborted,
        });
    }
    ledger.as_ref().unwrap().encode(&intent.service_id).unwrap();
    let retained = ledger.clone();
    assert!(apply(&mut ledger, &Change::Begin(&fixture("capacity-overflow"))).is_err());
    let mut retained = retained;
    assert!(
        !apply(&mut retained, &Change::Begin(&intent))
            .unwrap()
            .admitted,
        "capacity does not turn historical ID into a new command"
    );
}
