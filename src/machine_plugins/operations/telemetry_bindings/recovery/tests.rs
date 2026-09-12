use super::super::tests::{ledger, save};
use super::*;
use crate::machine_plugins::PluginExecutionScope;
use crate::machine_protocol::telemetry_recovery::{fixture, prepared};
use std::sync::atomic::{AtomicUsize, Ordering};

fn retained(root: &Path, request: &RecoveryRequest, outcome: BindingOutcome) -> Journal {
    let owner = Journal::open(root).unwrap();
    save(
        &owner.telemetry_bindings.path,
        ledger(vec![BindingReceipt {
            step: request.step.clone(),
            request_digest: request.step.request_digest().unwrap(),
            outcome,
        }]),
    );
    drop(owner);
    Journal::open(root).unwrap()
}

fn scope(request: &RecoveryRequest) -> PluginExecutionScope {
    PluginExecutionScope::new(Some(&request.step.service_id), &request.step.machine_id)
}

fn resolved(observation: RecoveryObservation) -> RecoverySnapshot {
    let RecoveryObservation::Observed { snapshot } = observation else {
        panic!("unavailable evidence");
    };
    *snapshot
}

fn enable(journal: &Journal) {
    journal.telemetry_bindings.state.lock().recovery_writer = true;
}

#[test]
fn reopened_prepared_closes_at_same_head_and_keeps_audited_history_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let mut request = fixture();
    request.step.expires_at_ms = 1; // old deadline is evidence, not revived authority
    request.expected_observation_digest =
        binding_digest(&serde_json::to_vec(&prepared(&request.step).unwrap()).unwrap());
    let journal = retained(root.path(), &request, BindingOutcome::Prepared {});
    let path = journal.telemetry_bindings.path.clone();
    let original = fs::read(&path).unwrap();
    enable(&journal);
    let owner = scope(&request);
    let result = journal
        .telemetry_bindings
        .recover(&request, &owner.telemetry_recovery(&request).unwrap())
        .unwrap();
    assert!(result.matches(&request));
    let result = resolved(result);
    let receipt = result.receipt.unwrap();
    assert!(receipt.matches(&request));
    assert_eq!(
        receipt.binding.outcome,
        BindingOutcome::Rejected {
            reason: BindingRejection::AuthorizationEnded
        }
    );
    let BindingObservation::Observed { snapshot } = result.binding else {
        panic!();
    };
    assert_eq!(snapshot.current, Some(request.step.expected.clone()));
    assert!(!snapshot.unresolved);
    assert!(journal.telemetry_bindings.ensure_legacy_allowed().is_err());
    let saved = fs::read(&path).unwrap();
    assert_ne!(saved, original);
    drop(journal);
    let journal = Journal::open(root.path()).unwrap();
    assert!(!journal.telemetry_bindings.state.lock().recovery_writer);
    let history = journal
        .telemetry_bindings
        .recover(&request, &owner.telemetry_recovery(&request).unwrap())
        .unwrap();
    assert_eq!(resolved(history).receipt.as_ref(), Some(&receipt));
    assert_eq!(saved, fs::read(&path).unwrap());
    assert!(journal.telemetry_bindings.ensure_legacy_allowed().is_err());
}

#[test]
fn recovery_requires_its_own_gate_reopen_proof_and_exact_prepared_state() {
    for boundary in [
        "closed",
        "ordinary-writer",
        "active",
        "unknown",
        "applied",
        "poisoned",
        "foreign",
    ] {
        let root = tempfile::tempdir().unwrap();
        let request = fixture();
        let outcome = match boundary {
            "unknown" => BindingOutcome::Unknown {},
            "applied" => BindingOutcome::Applied {
                after: request.step.after().unwrap(),
            },
            _ => BindingOutcome::Prepared {},
        };
        let journal = retained(root.path(), &request, outcome);
        if !matches!(boundary, "closed" | "ordinary-writer") {
            enable(&journal);
        }
        match boundary {
            "ordinary-writer" => journal.telemetry_bindings.state.lock().writer = true,
            "active" => journal.telemetry_bindings.state.lock().reopened_prepared = None,
            "poisoned" => journal.telemetry_bindings.state.lock().poisoned = true,
            _ => {}
        }
        let owner = scope(&request);
        let lease = owner.telemetry_recovery(&request).unwrap();
        let mut changed = request.clone();
        if boundary == "foreign" {
            changed.resolution_id.push_str("-foreign");
        }
        let bytes = fs::read(&journal.telemetry_bindings.path).unwrap();
        assert!(
            journal
                .telemetry_bindings
                .recover(&changed, &lease)
                .is_err(),
            "{boundary}"
        );
        assert_eq!(bytes, fs::read(&journal.telemetry_bindings.path).unwrap());
    }
}

#[test]
fn live_query_and_legacy_preflight_revalidate_disk_and_poison_stickily() {
    let root = tempfile::tempdir().unwrap();
    let request = fixture();
    let journal = retained(root.path(), &request, BindingOutcome::Prepared {});
    let path = &journal.telemetry_bindings.path;
    let bytes = fs::read(path).unwrap();
    atomic_write(path, b"{}", 0o600).unwrap();
    assert!(matches!(
        journal.telemetry_bindings.query(&request.step),
        BindingObservation::Unavailable {
            reason: BindingUnavailable::Storage
        }
    ));
    atomic_write(path, &bytes, 0o600).unwrap();
    assert!(matches!(
        journal.telemetry_bindings.query_recovery(&request),
        RecoveryObservation::Unavailable {
            reason: BindingUnavailable::Storage
        }
    ));
    assert!(journal.telemetry_bindings.ensure_legacy_allowed().is_err());
    drop(journal);
    let journal = Journal::open(root.path()).unwrap();
    assert!(request.expects(&journal.telemetry_bindings.query(&request.step)));

    let absent = tempfile::tempdir().unwrap();
    let owner = Journal::open(absent.path()).unwrap();
    owner.telemetry_bindings.ensure_legacy_allowed().unwrap();
    save(&owner.telemetry_bindings.path, ledger(vec![]));
    assert!(
        owner.telemetry_bindings.ensure_legacy_allowed().is_err(),
        "an out-of-band namespace cannot be bypassed from the absent cache"
    );
    assert!(owner.telemetry_bindings.state.lock().poisoned);
}

#[test]
fn concurrent_recovery_ids_can_close_the_prepared_receipt_only_once() {
    let root = tempfile::tempdir().unwrap();
    let request = fixture();
    let journal = retained(root.path(), &request, BindingOutcome::Prepared {});
    enable(&journal);
    let mut competing = request.clone();
    competing.resolution_id.push('x');
    let barrier = std::sync::Barrier::new(2);
    let successes = std::thread::scope(|threads| {
        [&request, &competing]
            .map(|request| {
                let (journal, barrier) = (&journal, &barrier);
                threads.spawn(move || {
                    let owner = scope(request);
                    let lease = owner.telemetry_recovery(request).unwrap();
                    barrier.wait();
                    journal.telemetry_bindings.recover(request, &lease).is_ok()
                })
            })
            .into_iter()
            .map(|task| usize::from(task.join().unwrap()))
            .sum::<usize>()
    });
    assert_eq!(successes, 1);
    assert_eq!(
        journal
            .telemetry_bindings
            .state
            .lock()
            .ledger
            .as_ref()
            .unwrap()
            .resolutions
            .len(),
        1
    );
}

#[tokio::test]
async fn actual_interrupted_writer_preserves_audit_across_fresh_binding_and_second_recovery() {
    let root = tempfile::tempdir().unwrap();
    let mut request = fixture();
    request.step.change = BindingChange::Revoke {
        policy_epoch: "1".to_owned().try_into().unwrap(),
    };
    request.expected_observation_digest =
        binding_digest(&serde_json::to_vec(&prepared(&request.step).unwrap()).unwrap());
    let open = || {
        MachinePluginStore::new(
            root.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap()
    };
    let owner = scope(&request);
    let machine = open();
    machine.enable_binding_writer_for_test();
    machine.interrupt_binding_for_test(&request.step).await;
    drop(machine);
    let machine = open();
    machine.enable_binding_recovery_for_test();
    let result = machine
        .recover_telemetry_binding(&request, owner.telemetry_recovery(&request).unwrap())
        .await;
    let RecoveryResult::Observed { observation } = result else {
        panic!("{result:?}")
    };
    let original = resolved(observation).receipt.unwrap();
    let mut next = request.step.clone();
    next.operation_id = "binding-after-recovery".into();
    machine.enable_binding_writer_for_test();
    // Closing the attempt did not erase the managed namespace, even at head 0.
    let rejected = machine
        .commit_telemetry_binding_command(&next, owner.telemetry_binding(&next).unwrap())
        .await;
    assert!(matches!(
        rejected,
        crate::machine_protocol::telemetry_binding::BindingCommitResult::Unavailable {
            failure: Failure::Rejected(BindingRejection::TargetChanged)
        }
    ));
    next.expected_namespace = Some(BindingNamespace::Managed);
    let applied = machine
        .commit_telemetry_binding_command(&next, owner.telemetry_binding(&next).unwrap())
        .await;
    assert!(matches!(
        applied,
        crate::machine_protocol::telemetry_binding::BindingCommitResult::Observed { .. }
    ));
    let history = resolved(
        machine
            .telemetry_recovery_observation(&request, Some("service-test"), "machine-test")
            .await,
    );
    assert_eq!(history.receipt.as_deref(), Some(original.as_ref()));
    assert!(
        matches!(history.binding, BindingObservation::Observed { snapshot } if snapshot.current == Some(next.after().unwrap()) && !snapshot.unresolved)
    );
    let mut second = request.clone();
    second.resolution_id.push('x');
    second.step.operation_id = "second-interrupted-binding".into();
    second.step.expected_namespace = Some(BindingNamespace::Managed);
    second.step.expected = next.after().unwrap();
    second.step.change = BindingChange::Revoke {
        policy_epoch: "2".to_owned().try_into().unwrap(),
    };
    second.expected_observation_digest =
        binding_digest(&serde_json::to_vec(&prepared(&second.step).unwrap()).unwrap());
    machine.interrupt_binding_for_test(&second.step).await;
    drop(machine);
    let machine = open();
    machine.enable_binding_recovery_for_test();
    assert!(matches!(
        machine
            .recover_telemetry_binding(&second, owner.telemetry_recovery(&second).unwrap())
            .await,
        RecoveryResult::Observed { .. }
    ));
    drop(machine);
    let machine = open();
    assert_eq!(
        machine
            .operations
            .telemetry_bindings
            .state
            .lock()
            .ledger
            .as_ref()
            .unwrap()
            .resolutions
            .len(),
        2
    );
    assert!(
        machine
            .telemetry_recovery_observation(&request, Some("service-test"), "machine-test")
            .await
            .matches(&request)
    );
}

#[test]
fn failures_before_and_after_rename_keep_the_running_owner_poisoned_until_reopen() {
    for renamed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let request = fixture();
        let journal = retained(root.path(), &request, BindingOutcome::Prepared {});
        enable(&journal);
        let path = journal.telemetry_bindings.path.clone();
        let owner = scope(&request);
        let writes = AtomicUsize::new(0);
        let result = journal.telemetry_bindings.recover_with_io(
            &request,
            &owner.telemetry_recovery(&request).unwrap(),
            |bytes| {
                writes.fetch_add(1, Ordering::Relaxed);
                if renamed {
                    atomic_write(&path, bytes, 0o600)?;
                }
                anyhow::bail!("hermetic rename/directory-flush failure");
            },
        );
        assert_eq!(
            result,
            Err(Failure::Unavailable(BindingUnavailable::Storage))
        );
        assert_eq!(writes.load(Ordering::Relaxed), 1);
        assert!(matches!(
            journal.telemetry_bindings.query_recovery(&request),
            RecoveryObservation::Unavailable {
                reason: BindingUnavailable::Storage
            }
        ));
        assert!(
            journal
                .telemetry_bindings
                .recover(&request, &owner.telemetry_recovery(&request).unwrap())
                .is_err()
        );
        drop(journal);
        let journal = Journal::open(root.path()).unwrap();
        let saved = resolved(journal.telemetry_bindings.query_recovery(&request));
        assert_eq!(saved.receipt.is_some(), renamed);
        assert_eq!(request.expects(&saved.binding), !renamed);
        assert!(journal.telemetry_bindings.ensure_legacy_allowed().is_err());
    }
}

#[test]
fn expired_or_disconnected_new_leases_never_write_and_competing_ids_only_close_once() {
    let root = tempfile::tempdir().unwrap();
    let request = fixture();
    let journal = retained(root.path(), &request, BindingOutcome::Prepared {});
    enable(&journal);
    let owner = scope(&request);
    let expired = owner.telemetry_recovery(&request).unwrap();
    expired.expire_for_test();
    let bytes = fs::read(&journal.telemetry_bindings.path).unwrap();
    assert_eq!(
        journal.telemetry_bindings.recover(&request, &expired),
        Err(Failure::Rejected(BindingRejection::Expired))
    );
    let lease = owner.telemetry_recovery(&request).unwrap();
    drop(owner);
    assert_eq!(
        journal.telemetry_bindings.recover(&request, &lease),
        Err(Failure::Rejected(BindingRejection::AuthorizationEnded))
    );
    assert_eq!(bytes, fs::read(&journal.telemetry_bindings.path).unwrap());
    let owner = scope(&request);
    journal
        .telemetry_bindings
        .recover(&request, &owner.telemetry_recovery(&request).unwrap())
        .unwrap();
    let saved = fs::read(&journal.telemetry_bindings.path).unwrap();
    let mut conflict = request.clone();
    conflict.resolution_id.push_str("-another");
    assert!(
        journal
            .telemetry_bindings
            .recover(&conflict, &owner.telemetry_recovery(&conflict).unwrap())
            .is_err()
    );
    conflict.resolution_id = request.resolution_id.clone();
    conflict.actor = crate::machine_protocol::telemetry_recovery::RecoveryActor::Admin {
        account: "different".into(),
    };
    assert_eq!(
        journal
            .telemetry_bindings
            .recover(&conflict, &owner.telemetry_recovery(&conflict).unwrap()),
        Err(Failure::Unavailable(BindingUnavailable::IdentityConflict))
    );
    assert_eq!(saved, fs::read(&journal.telemetry_bindings.path).unwrap());
}

#[test]
fn schema_two_audit_is_required_and_structural_tampering_fails_with_a_new_checksum() {
    let root = tempfile::tempdir().unwrap();
    let request = fixture();
    let journal = retained(root.path(), &request, BindingOutcome::Prepared {});
    enable(&journal);
    let owner = scope(&request);
    journal
        .telemetry_bindings
        .recover(&request, &owner.telemetry_recovery(&request).unwrap())
        .unwrap();
    let original = journal
        .telemetry_bindings
        .state
        .lock()
        .ledger
        .clone()
        .unwrap();
    for boundary in [
        "schema",
        "empty",
        "duplicate",
        "binding",
        "actor",
        "time",
        "digest",
    ] {
        let mut changed = original.clone();
        match boundary {
            "schema" => changed.schema = 1,
            "empty" => changed.resolutions.clear(),
            "duplicate" => changed.resolutions.push(changed.resolutions[0].clone()),
            "binding" => changed.resolutions[0].binding.outcome = BindingOutcome::Unknown {},
            "actor" => {
                changed.resolutions[0].request.actor =
                    crate::machine_protocol::telemetry_recovery::RecoveryActor::Product {
                        user_id: String::new(),
                    }
            }
            "time" => changed.resolutions[0].resolved_at_ms = request.expires_at_ms,
            _ => {
                changed.resolutions[0].request.expected_observation_digest =
                    binding_digest(b"different");
            }
        }
        save(&journal.telemetry_bindings.path, changed);
        assert!(
            Bindings::read(&journal.telemetry_bindings.path).is_err(),
            "{boundary}"
        );
    }
}
