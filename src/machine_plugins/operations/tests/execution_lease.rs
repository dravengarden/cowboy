use super::*;
use lease::PluginExecutionScope;

fn scope(request: &UninstallStep) -> PluginExecutionScope {
    PluginExecutionScope::new(Some(&request.service_id), &request.machine_id)
}

#[test]
fn wrong_request_or_disconnected_lease_cannot_create_an_intent() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let request = step();
    let owner = scope(&request);
    let lease = owner.uninstall(&request).unwrap();
    let mut changed = request.clone();
    changed.plan_digest = digest(b"other actor or impact");
    assert_eq!(
        journal.execute(&changed, &lease, || panic!(), || panic!()),
        unavailable(StepUnavailable::InvalidRequest)
    );
    drop(owner);
    assert_eq!(
        journal.execute(&request, &lease, || panic!(), || panic!()),
        unavailable(StepUnavailable::WrongOwner)
    );
    assert!(journal.state.lock().receipts.is_empty());
    assert_eq!(
        fs::read_dir(&journal.root).unwrap().count(),
        1,
        "only owner.lock"
    );
}

#[test]
fn disconnect_during_verification_keeps_durable_unknown_without_running_effect() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let request = step();
    let owner = scope(&request);
    let lease = owner.uninstall(&request).unwrap();
    let result = journal.execute(
        &request,
        &lease,
        || {
            drop(owner);
            true
        },
        || panic!("disconnected effect"),
    );
    assert_eq!(result, unavailable(StepUnavailable::WrongOwner));
    drop(journal);
    let journal = Journal::open(root.path()).unwrap();
    assert_eq!(
        receipt(journal.query(&request)).outcome,
        StepOutcome::Unknown {
            reason: StepUncertainty::Interrupted
        }
    );
    assert!(journal.ensure_unfenced(&request.plugin_id).is_err());
    let fresh = scope(&request);
    let renewed = fresh.uninstall(&request).unwrap();
    assert_eq!(
        journal.execute(&request, &renewed, || panic!(), || panic!()),
        journal.query(&request)
    );
}

#[test]
fn lease_expiry_after_verification_records_a_definitive_no_effect_result() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let request = step();
    let owner = scope(&request);
    let lease = owner.uninstall(&request).unwrap();
    let result = journal.execute(
        &request,
        &lease,
        || {
            lease.expire_for_test();
            true
        },
        || panic!("expired effect"),
    );
    assert_eq!(
        receipt(result).outcome,
        StepOutcome::Rejected {
            reason: StepRejection::Expired
        }
    );
    assert!(journal.ensure_unfenced(&request.plugin_id).is_ok());
}

#[test]
fn an_effect_already_started_finishes_its_receipt_after_observer_loss() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let request = step();
    let owner = scope(&request);
    let lease = owner.uninstall(&request).unwrap();
    let calls = AtomicUsize::new(0);
    let result = journal.execute(
        &request,
        &lease,
        || true,
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            drop(owner);
            lease.expire_for_test();
            Ok(())
        },
    );
    assert_eq!(receipt(result.clone()).outcome, StepOutcome::Applied {});
    assert_eq!(
        journal.execute(&request, &lease, || panic!(), || panic!()),
        result
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(journal);
    assert_eq!(Journal::open(root.path()).unwrap().query(&request), result);
}

#[cfg(feature = "full")]
#[tokio::test]
async fn a_queued_uninstall_cannot_outlive_its_original_connection_or_budget() {
    for disconnected in [false, true] {
        let (_root, store, _desired, request) = recovery::installed().await;
        let before = store.inventory().unwrap();
        let installation_bytes =
            recovery::journal_bytes(&store.operations.root.join(installations::DIRECTORY));
        let owner = scope(&request);
        let lease = owner.uninstall(&request).unwrap();
        let lock = store.lifecycle.lock().await;
        let mut pending = Box::pin(store.uninstall_step(
            &request,
            Some("service-a"),
            "machine-a",
            true,
            UninstallAccess::Execute(&lease),
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut pending)
                .await
                .is_err()
        );
        if disconnected {
            drop(owner);
        } else {
            lease.expire_for_test();
        }
        drop(lock);
        let result = pending.await.result;
        if disconnected {
            assert_eq!(result, unavailable(StepUnavailable::WrongOwner));
            assert_eq!(store.operations.query(&request), StepLookup::NotFound {});
        } else {
            assert_eq!(
                receipt(result).outcome,
                StepOutcome::Rejected {
                    reason: StepRejection::Expired
                }
            );
        }
        assert_eq!(store.inventory().unwrap(), before);
        assert_eq!(
            recovery::journal_bytes(&store.operations.root.join(installations::DIRECTORY)),
            installation_bytes
        );
    }
}

#[cfg(feature = "full")]
#[tokio::test]
async fn lease_loss_after_slot_intent_preserves_pending_installation_and_active_bytes() {
    let (_root, store, _desired, request) = recovery::installed().await;
    let owner = scope(&request);
    let lease = owner.uninstall(&request).unwrap();
    let active = fs::read_link(store.plugin_root(&request.plugin_id).join("active")).unwrap();
    let result = store.operations.execute(
        &request,
        &lease,
        || true,
        || {
            let _pending = store.operations.installations.begin(
                &request.plugin_id,
                Some(&request.generation_digest),
                None,
                installations::Effect::Uninstall,
                Some(request.request_digest()?),
            )?;
            lease.expire_for_test();
            lease.before_effect()?;
            panic!("no Plugin mutation after lease expiry");
        },
    );
    assert_eq!(
        receipt(result).outcome,
        StepOutcome::Unknown {
            reason: StepUncertainty::EffectFailure
        }
    );
    assert!(
        store
            .operations
            .ensure_unfenced(&request.plugin_id)
            .is_err()
    );
    assert_eq!(
        fs::read_link(store.plugin_root(&request.plugin_id).join("active")).unwrap(),
        active
    );
    let observation = store
        .uninstall_recovery_observation(&request, Some("service-a"), "machine-a")
        .await;
    assert!(
        matches!(observation, crate::machine_protocol::plugin_recovery::RecoveryObservation::Observed { snapshot }
        if matches!(snapshot.installation, crate::machine_protocol::plugin_recovery::InstallationEvidence::Pending { .. }) && snapshot.slot_fenced)
    );
}

impl MachinePluginStore {
    // Also used by the signed Agent/Zed lifecycle fixtures. Agent inventory
    // takes the uncached signature/runtime/auth-validation path.
    pub(in crate::machine_plugins) fn assert_final_uninstall_lease_check(
        &self,
        request: &UninstallStep,
    ) {
        for disconnected in [false, true] {
            let active =
                fs::read_link(self.plugin_root(&request.plugin_id).join("active")).unwrap();
            let inventory = self.inventory().unwrap();
            let owner = scope(request);
            let lease = owner.uninstall(request).unwrap();
            assert!(
                self.uninstall_inner(&request.plugin_id, &digest(b"wrong release"), || panic!(
                    "validation precedes final admission"
                ))
                .is_err()
            );
            let called = AtomicUsize::new(0);
            let result =
                self.uninstall_inner(&request.plugin_id, &request.generation_digest, || {
                    called.fetch_add(1, Ordering::SeqCst);
                    if disconnected {
                        drop(owner);
                    } else {
                        lease.expire_for_test();
                    }
                    lease.before_effect()
                });
            assert_eq!(
                called.load(Ordering::SeqCst),
                1,
                "inventory was validated before the final checkpoint"
            );
            assert!(result.is_err());
            assert_eq!(
                fs::read_link(self.plugin_root(&request.plugin_id).join("active")).unwrap(),
                active
            );
            assert_eq!(self.inventory().unwrap(), inventory);
        }
    }
}

#[cfg(feature = "full")]
#[tokio::test]
async fn final_lease_check_follows_inventory_validation_before_removing_active() {
    let (_root, store, _desired, request) = recovery::installed().await;
    store.assert_final_uninstall_lease_check(&request);
}
