use super::*;
use crate::machine_plugins::PluginExecutionScope;
use crate::machine_protocol::telemetry_binding::{BindingInstallation, fixture};
use std::cell::Cell;
use std::os::unix::fs::MetadataExt as _;
use std::sync::Arc;

#[test]
fn namespace_presence_is_an_independent_cas_even_when_revision_is_still_zero() {
    use crate::machine_protocol::telemetry_binding::execution_fixture;
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let bindings = &journal.telemetry_bindings;
    enable(bindings);
    let initial = execution_fixture();
    let mut expects_managed = initial.clone();
    expects_managed.expected_namespace = Some(BindingNamespace::Managed);
    assert_eq!(
        commit(bindings, &expects_managed),
        Err(WriteError::Rejected(BindingRejection::TargetChanged))
    );
    assert!(!bindings.path.exists());
    let owner = scope(&initial);
    let lease = owner.telemetry_binding(&initial).unwrap();
    let persisted = Cell::new(false);
    let rejected = bindings
        .commit_with_io(
            &initial,
            &lease,
            &mut || {
                if persisted.get() {
                    Err(BindingRejection::AuthorizationEnded)
                } else {
                    Ok(())
                }
            },
            |bytes| {
                durable(&bindings.path, bytes)?;
                persisted.set(true);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(result(rejected).current, Some(BindingSnapshot::initial()));
    let mut next = initial.clone();
    next.operation_id = "namespace-next-operation".into();
    let before = fs::read(&bindings.path).unwrap();
    assert_eq!(
        commit(bindings, &next),
        Err(WriteError::Rejected(BindingRejection::TargetChanged))
    );
    assert_eq!(fs::read(&bindings.path).unwrap(), before);
    next.expected_namespace = Some(BindingNamespace::Managed);
    assert_eq!(
        result(commit(bindings, &next).unwrap()).current,
        Some(next.after().unwrap())
    );
    assert!(
        commit(bindings, &initial).unwrap().matches(&initial),
        "duplicate evidence never reexecutes namespace CAS"
    );
    let mut invalid = bindings.state.lock().ledger.clone().unwrap();
    let second = &mut invalid.receipts[1];
    second.step.expected_namespace = Some(BindingNamespace::Unmanaged);
    second.request_digest = second.step.request_digest().unwrap();
    assert!(
        invalid.validate().is_err(),
        "even a rechecksummed second initial namespace is invalid"
    );
    drop(journal);
    let reader = Journal::open(root.path()).unwrap();
    assert_eq!(
        result(reader.telemetry_bindings.query(&next)).current,
        Some(next.after().unwrap())
    );
}

fn scope(step: &BindingStep) -> PluginExecutionScope {
    PluginExecutionScope::new(Some(&step.service_id), &step.machine_id)
}

fn enable(bindings: &Bindings) {
    // Only hermetic fixtures can open this gate. No production setter/flag.
    bindings.state.lock().writer = true;
}

fn result(observation: BindingObservation) -> BindingObservationSnapshot {
    let BindingObservation::Observed { snapshot } = observation else {
        panic!("expected bounded observation: {observation:?}");
    };
    *snapshot
}

fn commit(bindings: &Bindings, step: &BindingStep) -> WriteResult<BindingObservation> {
    let owner = scope(step);
    bindings.commit(step, &owner.telemetry_binding(step).unwrap(), || Ok(()))
}

fn durable(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write(path, bytes, 0o600)?;
    fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}

fn next(previous: &BindingStep, index: usize) -> BindingStep {
    let mut step = fixture();
    step.operation_id = format!("binding-operation-{index:04}");
    step.expected = previous.after().unwrap();
    step.change = BindingChange::Revoke {
        policy_epoch: step.expected.policy_epoch.next().unwrap(),
    };
    step
}

fn restore(forward: &BindingStep, index: usize) -> BindingStep {
    let mut step = next(forward, index);
    step.change = BindingChange::Restore {
        forward_request_digest: forward.request_digest().unwrap(),
        selection: forward.expected.selection.clone(),
        policy_epoch: step.expected.policy_epoch.next().unwrap(),
    };
    step
}

#[test]
fn admission_stays_closed_and_open_query_or_denial_never_adopts_authority() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let bindings = &journal.telemetry_bindings;
    assert_eq!(commit(bindings, &fixture()), Err(WriteError::ReaderOnly));
    assert!(!bindings.path.exists());
    bindings.ensure_legacy_allowed().unwrap();
    assert!(result(bindings.query(&fixture())).current.is_none());
    assert!(
        Journal::open(root.path()).is_err(),
        "same exclusive journal owner"
    );
}

#[test]
fn finite_select_revoke_restore_flush_head_and_receipt_and_never_reexecute_duplicates() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let bindings = &journal.telemetry_bindings;
    enable(bindings);
    let select = fixture();
    let revoke = next(&select, 2);
    let restore = restore(&revoke, 3);
    for step in [&select, &revoke, &restore] {
        let observation = commit(bindings, step).unwrap();
        assert!(observation.matches(step));
        let snapshot = result(observation);
        assert_eq!(snapshot.current, Some(step.after().unwrap()));
        assert_eq!(
            snapshot.receipt.unwrap().outcome,
            BindingOutcome::Applied {
                after: step.after().unwrap()
            }
        );
        assert!(!snapshot.unresolved);
        assert_eq!(fs::metadata(&bindings.path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&journal.root).unwrap().mode() & 0o777, 0o700);
        assert!(bindings.ensure_legacy_allowed().is_err());
    }
    let path = bindings.path.clone();
    let before = fs::read(&path).unwrap();
    drop(journal);
    let reader = Journal::open(root.path()).unwrap();
    let bindings = &reader.telemetry_bindings;
    for step in [&select, &revoke, &restore] {
        let owner = scope(step);
        let lease = owner.telemetry_binding(step).unwrap();
        lease.expire_for_test();
        let observed = bindings
            .commit(step, &lease, || panic!("receipt is not replay authority"))
            .unwrap();
        let snapshot = result(observed);
        assert_eq!(snapshot.current, Some(restore.after().unwrap()));
        assert_eq!(snapshot.receipt.unwrap().step, *step);
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        commit(bindings, &next(&restore, 4)),
        Err(WriteError::ReaderOnly)
    );
}

#[test]
fn rejected_initial_requests_do_not_disable_the_existing_legacy_exporter() {
    for rejection in [
        BindingRejection::Expired,
        BindingRejection::AuthorizationEnded,
        BindingRejection::PolicyChanged,
        BindingRejection::TargetChanged,
    ] {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::open(root.path()).unwrap();
        let bindings = &journal.telemetry_bindings;
        enable(bindings);
        let step = fixture();
        let owner = scope(&step);
        let lease = owner.telemetry_binding(&step).unwrap();
        assert_eq!(
            bindings.commit(&step, &lease, || Err(rejection)),
            Err(WriteError::Rejected(rejection))
        );
        assert!(!bindings.path.exists());
        bindings.ensure_legacy_allowed().unwrap();
    }
}

#[test]
fn exact_identity_epochs_and_forward_provenance_prevent_stale_or_aba_restoration() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let bindings = &journal.telemetry_bindings;
    enable(bindings);
    let select = fixture();
    commit(bindings, &select).unwrap();
    let mut changed = select.clone();
    changed.plan_digest = binding_digest(b"another actor/intent");
    assert_eq!(
        commit(bindings, &changed),
        Err(WriteError::Unavailable(
            BindingUnavailable::IdentityConflict
        ))
    );
    changed.operation_id = "new-binding-operation".into();
    assert_eq!(
        commit(bindings, &changed),
        Err(WriteError::Rejected(BindingRejection::TargetChanged))
    );
    let before = fs::read(&bindings.path).unwrap();
    for epoch in ["1", "3", "18446744073709551615"] {
        let mut revoke = next(&select, 2);
        revoke.change = BindingChange::Revoke {
            policy_epoch: epoch.to_owned().try_into().unwrap(),
        };
        assert_eq!(
            commit(bindings, &revoke),
            Err(WriteError::Rejected(BindingRejection::PolicyChanged))
        );
        assert_eq!(fs::read(&bindings.path).unwrap(), before);
    }
    let revoke = next(&select, 2);
    commit(bindings, &revoke).unwrap();
    let restoration = restore(&revoke, 3);
    let mut bad = restoration.clone();
    if let BindingChange::Restore {
        forward_request_digest,
        ..
    } = &mut bad.change
    {
        *forward_request_digest = binding_digest(b"not an applied forward request");
    }
    assert_eq!(
        commit(bindings, &bad),
        Err(WriteError::Rejected(BindingRejection::TargetChanged))
    );
    commit(bindings, &restoration).unwrap();
    // Same selected release, newer revision/epoch: an old restoration cannot
    // overwrite it; historical duplicate evidence is not a current-state CAS.
    let mut stale = restore(&select, 4);
    assert_eq!(
        commit(bindings, &stale),
        Err(WriteError::Rejected(BindingRejection::TargetChanged))
    );
    stale.expected = restoration.after().unwrap();
    stale.change = BindingChange::Restore {
        forward_request_digest: select.request_digest().unwrap(),
        selection: None,
        policy_epoch: stale.expected.policy_epoch.next().unwrap(),
    };
    assert_eq!(
        commit(bindings, &stale),
        Err(WriteError::Rejected(BindingRejection::TargetChanged))
    );
}

#[test]
fn loss_after_durable_intent_finishes_rejected_without_restoring_legacy_admission() {
    for reason in [
        BindingRejection::Expired,
        BindingRejection::AuthorizationEnded,
        BindingRejection::PolicyChanged,
        BindingRejection::TargetChanged,
    ] {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::open(root.path()).unwrap();
        let bindings = &journal.telemetry_bindings;
        enable(bindings);
        let step = fixture();
        let owner = scope(&step);
        let lease = owner.telemetry_binding(&step).unwrap();
        let persisted = Cell::new(false);
        let observation = bindings
            .commit_with_io(
                &step,
                &lease,
                &mut || if persisted.get() { Err(reason) } else { Ok(()) },
                |bytes| {
                    durable(&bindings.path, bytes)?;
                    persisted.set(true);
                    Ok(())
                },
            )
            .unwrap();
        let snapshot = result(observation);
        assert_eq!(snapshot.current, Some(BindingSnapshot::initial()));
        assert_eq!(
            snapshot.receipt.unwrap().outcome,
            BindingOutcome::Rejected { reason }
        );
        assert!(!snapshot.unresolved);
        assert!(bindings.ensure_legacy_allowed().is_err());
        // Rejection did not consume epoch 1. A NEW independently leased
        // operation can select; the old rejected operation cannot revive.
        let mut fresh = fixture();
        fresh.operation_id = "binding-operation-fresh".into();
        commit(bindings, &fresh).unwrap();
        assert!(matches!(
            result(commit(bindings, &step).unwrap())
                .receipt
                .unwrap()
                .outcome,
            BindingOutcome::Rejected { .. }
        ));
    }
}

#[test]
fn storage_failure_before_or_after_each_rename_poisons_until_validated_reopen() {
    for failure in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::open(root.path()).unwrap();
        let bindings = &journal.telemetry_bindings;
        enable(bindings);
        let step = fixture();
        let owner = scope(&step);
        let lease = owner.telemetry_binding(&step).unwrap();
        let mut point = 0;
        let outcome = bindings.commit_with_io(&step, &lease, &mut || Ok(()), |bytes| {
            if point == failure {
                bail!("fixture before rename");
            }
            point += 1;
            atomic_write(&bindings.path, bytes, 0o600)?;
            if point == failure {
                bail!("fixture directory sync uncertainty");
            }
            point += 1;
            fs::File::open(&journal.root)?.sync_all()?;
            Ok(())
        });
        assert_eq!(
            outcome,
            Err(WriteError::Unavailable(BindingUnavailable::Storage))
        );
        assert_eq!(
            bindings.query(&step),
            unavailable(BindingUnavailable::Storage)
        );
        assert!(
            bindings.ensure_legacy_allowed().is_err(),
            "never fall through poisoned initial adoption"
        );
        assert_eq!(
            commit(bindings, &step),
            Err(WriteError::Unavailable(BindingUnavailable::Storage))
        );
        let path = bindings.path.clone();
        drop(journal);
        let reopened = Journal::open(root.path()).unwrap();
        let bindings = &reopened.telemetry_bindings;
        let snapshot = result(bindings.query(&step));
        if failure == 0 {
            assert!(snapshot.current.is_none() && snapshot.receipt.is_none());
            bindings.ensure_legacy_allowed().unwrap();
        } else {
            let before = fs::read(&path).unwrap();
            if failure == 3 {
                assert_eq!(
                    snapshot.receipt.unwrap().outcome,
                    BindingOutcome::Applied {
                        after: step.after().unwrap()
                    }
                );
                assert!(!snapshot.unresolved);
            } else {
                assert_eq!(
                    snapshot.receipt.unwrap().outcome,
                    BindingOutcome::Prepared {}
                );
                assert!(snapshot.unresolved);
                enable(bindings);
                let mut fresh = fixture();
                fresh.operation_id = "different-operation-0001".into();
                assert_eq!(commit(bindings, &fresh), Err(WriteError::Fenced));
            }
            commit(bindings, &step).unwrap(); // evidence only, never resume.
            assert_eq!(fs::read(&path).unwrap(), before);
            assert!(bindings.ensure_legacy_allowed().is_err());
        }
    }
}

#[test]
fn expiry_disconnect_and_retargeting_cannot_mint_or_renew_a_binding_lease() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let bindings = &journal.telemetry_bindings;
    enable(bindings);
    let step = fixture();
    for owner in [
        PluginExecutionScope::new(None, "machine-test"),
        PluginExecutionScope::new(Some("foreign-service"), "machine-test"),
        PluginExecutionScope::new(Some("service-test"), "foreign-machine"),
    ] {
        assert!(matches!(
            owner.telemetry_binding(&step),
            Err(BindingUnavailable::WrongOwner)
        ));
    }
    let owner = scope(&step);
    let lease = owner.telemetry_binding(&step).unwrap();
    let mut other = step.clone();
    other.plan_digest = binding_digest(b"other");
    assert_eq!(
        bindings.commit(&other, &lease, || panic!()),
        Err(WriteError::Unavailable(BindingUnavailable::InvalidRequest))
    );
    lease.expire_for_test();
    assert_eq!(
        bindings.commit(&step, &lease, || panic!()),
        Err(WriteError::Rejected(BindingRejection::Expired))
    );
    let lease = owner.telemetry_binding(&step).unwrap();
    drop(owner);
    let _replacement = scope(&step);
    assert_eq!(
        bindings.commit(&step, &lease, || panic!()),
        Err(WriteError::Rejected(BindingRejection::AuthorizationEnded))
    );
    assert!(!bindings.path.exists());
}

#[test]
fn out_of_band_evidence_cannot_be_clobbered_by_a_stale_cached_head() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let bindings = &journal.telemetry_bindings;
    enable(bindings);
    let step = fixture();
    commit(bindings, &step).unwrap();
    atomic_write(&bindings.path, b"fixture-corrupt-private-evidence", 0o600).unwrap();
    assert_eq!(
        commit(bindings, &next(&step, 2)),
        Err(WriteError::Unavailable(BindingUnavailable::Storage))
    );
    assert_eq!(
        fs::read(&bindings.path).unwrap(),
        b"fixture-corrupt-private-evidence"
    );
}

#[test]
fn concurrent_same_predecessor_commits_have_exactly_one_winner() {
    let root = tempfile::tempdir().unwrap();
    let journal = Arc::new(Journal::open(root.path()).unwrap());
    enable(&journal.telemetry_bindings);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let tasks = (0..2)
        .map(|index| {
            let journal = Arc::clone(&journal);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let mut step = fixture();
                step.operation_id = format!("concurrent-operation-{index}");
                barrier.wait();
                commit(&journal.telemetry_bindings, &step)
            })
        })
        .collect::<Vec<_>>();
    let results = tasks
        .into_iter()
        .map(|task| task.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == Err(WriteError::Rejected(BindingRejection::TargetChanged)))
            .count(),
        1
    );
    assert_eq!(
        journal
            .telemetry_bindings
            .state
            .lock()
            .ledger
            .as_ref()
            .unwrap()
            .receipts
            .len(),
        1
    );
}

#[cfg(feature = "full")]
mod signed;

#[test]
fn capacity_preserves_receipts_and_epoch_exhaustion_does_not_wrap() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let path = journal.telemetry_bindings.path.clone();
    drop(journal);
    let receipts = (0..MAX_BINDING_RECORDS)
        .map(|index| {
            let mut step = fixture();
            step.operation_id = format!("capacity-operation-{index:04}");
            BindingReceipt {
                request_digest: step.request_digest().unwrap(),
                step,
                outcome: BindingOutcome::Rejected {
                    reason: BindingRejection::Expired,
                },
            }
        })
        .collect();
    let ledger = Ledger {
        schema: 1,
        service_id: "service-test".into(),
        machine_id: "machine-test".into(),
        current: BindingSnapshot::initial(),
        receipts,
    };
    let before = ledger.encode().unwrap();
    durable(&path, &before).unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let bindings = &journal.telemetry_bindings;
    enable(bindings);
    assert_eq!(commit(bindings, &fixture()), Err(WriteError::Capacity));
    assert_eq!(fs::read(&path).unwrap(), before);
    let epoch: crate::machine_protocol::telemetry_binding::PolicyEpoch =
        "18446744073709551615".to_owned().try_into().unwrap();
    assert!(epoch.next().is_err());
    let exact: crate::machine_protocol::telemetry_binding::PolicyEpoch =
        "9007199254740992".to_owned().try_into().unwrap();
    assert_eq!(String::from(exact.next().unwrap()), "9007199254740993");
}

#[test]
fn original_connection_and_budget_are_checked_again_after_the_intent_flush() {
    for disconnect in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::open(root.path()).unwrap();
        let bindings = &journal.telemetry_bindings;
        enable(bindings);
        let step = fixture();
        let mut owner = Some(scope(&step));
        let lease = owner.as_ref().unwrap().telemetry_binding(&step).unwrap();
        let observed = bindings
            .commit_with_io(&step, &lease, &mut || Ok(()), |bytes| {
                durable(&bindings.path, bytes)?;
                if disconnect {
                    owner.take();
                } else {
                    lease.expire_for_test();
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(
            result(observed).receipt.unwrap().outcome,
            BindingOutcome::Rejected {
                reason: if disconnect {
                    BindingRejection::AuthorizationEnded
                } else {
                    BindingRejection::Expired
                }
            }
        );
    }
}
