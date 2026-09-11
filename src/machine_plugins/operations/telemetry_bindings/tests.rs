use super::*;
use crate::machine_protocol::telemetry_binding::{BindingRejection, fixture};
use std::os::unix::fs::MetadataExt as _;

fn receipt(step: BindingStep) -> BindingReceipt {
    BindingReceipt {
        outcome: BindingOutcome::Applied {
            after: step.after().unwrap(),
        },
        request_digest: step.request_digest().unwrap(),
        step,
    }
}

fn ledger(receipts: Vec<BindingReceipt>) -> Ledger {
    let current = receipts
        .iter()
        .filter_map(|receipt| match &receipt.outcome {
            BindingOutcome::Applied { after } => Some(after.clone()),
            _ => None,
        })
        .next_back()
        .unwrap_or_else(BindingSnapshot::initial);
    Ledger {
        schema: 1,
        service_id: "service-test".into(),
        machine_id: "machine-test".into(),
        current,
        receipts,
    }
}

fn save(path: &Path, ledger: Ledger) {
    let record = LedgerFile {
        evidence_digest: binding_digest(&serde_json::to_vec(&ledger).unwrap()),
        ledger,
    };
    atomic_write(path, &serde_json::to_vec(&record).unwrap(), 0o600).unwrap();
}

fn chain() -> Vec<BindingReceipt> {
    let select = receipt(fixture());
    let mut revoke = fixture();
    revoke.operation_id = "binding-operation-0002".into();
    revoke.expected = select.step.after().unwrap();
    revoke.change = BindingChange::Revoke {
        policy_epoch: "2".to_owned().try_into().unwrap(),
    };
    let revoke = receipt(revoke);
    let mut restore = fixture();
    restore.operation_id = "binding-operation-0003".into();
    restore.expected = revoke.step.after().unwrap();
    restore.change = BindingChange::Restore {
        forward_request_digest: revoke.request_digest.clone(),
        selection: revoke.step.expected.selection.clone(),
        policy_epoch: "3".to_owned().try_into().unwrap(),
    };
    vec![select, revoke, receipt(restore)]
}

#[test]
fn absent_namespace_queries_do_not_adopt_or_write_authority() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE);
    let reader = Bindings::open(&path).unwrap();
    reader.ensure_legacy_allowed().unwrap();
    for _ in 0..3 {
        let observation = reader.query(&fixture());
        assert!(observation.matches(&fixture()));
        let BindingObservation::Observed { snapshot } = observation else {
            panic!()
        };
        assert!(snapshot.current.is_none() && snapshot.receipt.is_none() && !snapshot.unresolved);
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn retained_chain_reopens_with_historical_receipts_current_head_and_no_legacy_egress() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE);
    let receipts = chain();
    save(&path, ledger(receipts.clone()));
    let before = fs::read(&path).unwrap();
    let metadata = fs::metadata(&path).unwrap();
    for _ in 0..2 {
        let reader = Bindings::open(&path).unwrap();
        assert!(reader.ensure_legacy_allowed().is_err());
        for receipt in &receipts {
            let observation = reader.query(&receipt.step);
            assert!(observation.matches(&receipt.step));
            let BindingObservation::Observed { snapshot } = observation else {
                panic!()
            };
            assert_eq!(snapshot.receipt.as_deref(), Some(receipt));
            assert_eq!(snapshot.current, Some(receipts[2].step.after().unwrap()));
            assert!(!snapshot.unresolved);
        }
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::metadata(&path).unwrap().ctime_nsec(),
        metadata.ctime_nsec()
    );
}

#[test]
fn prepared_unknown_and_empty_managed_state_remain_fenced_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE);
    for outcome in [BindingOutcome::Prepared {}, BindingOutcome::Unknown {}] {
        let mut pending = receipt(fixture());
        pending.outcome = outcome;
        save(&path, ledger(vec![pending]));
        let reader = Bindings::open(&path).unwrap();
        let BindingObservation::Observed { snapshot } = reader.query(&fixture()) else {
            panic!()
        };
        assert!(snapshot.unresolved);
        assert_eq!(snapshot.current, Some(BindingSnapshot::initial()));
        assert!(reader.ensure_legacy_allowed().is_err());
    }
    save(&path, ledger(Vec::new()));
    assert!(
        Bindings::open(&path)
            .unwrap()
            .ensure_legacy_allowed()
            .is_err()
    );
}

#[test]
fn query_identity_and_owner_cannot_reuse_another_operation_receipt() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE);
    save(&path, ledger(chain()));
    let reader = Bindings::open(&path).unwrap();
    let mut changed = fixture();
    changed.plan_digest = binding_digest(b"other actor or intent");
    assert_eq!(
        reader.query(&changed),
        unavailable(BindingUnavailable::IdentityConflict)
    );
    changed.service_id = "another-service".into();
    assert_eq!(
        reader.query(&changed),
        unavailable(BindingUnavailable::WrongOwner)
    );
    changed = fixture();
    changed.machine_id = "another-machine".into();
    assert_eq!(
        reader.query(&changed),
        unavailable(BindingUnavailable::WrongOwner)
    );
    changed = fixture();
    changed.operation_id = "unknown-operation-0001".into();
    let observation = reader.query(&changed);
    assert!(observation.matches(&changed));
    let BindingObservation::Observed { snapshot } = observation else {
        panic!()
    };
    assert!(snapshot.receipt.is_none() && snapshot.current.is_some());
}

#[test]
fn rechecksummed_forgery_cannot_erase_history_reuse_ids_or_restore_over_later_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE);
    for mutation in 0..8 {
        let mut evidence = ledger(chain());
        match mutation {
            0 => evidence.receipts.swap(0, 1),
            1 => {
                evidence.receipts[1].step.operation_id =
                    evidence.receipts[0].step.operation_id.clone();
            }
            2 => evidence.current = BindingSnapshot::initial(),
            3 => evidence.receipts[0].outcome = BindingOutcome::Unknown {},
            4 => evidence.receipts[1].step.expected = BindingSnapshot::initial(),
            5 => {
                let BindingChange::Restore {
                    forward_request_digest,
                    ..
                } = &mut evidence.receipts[2].step.change
                else {
                    panic!()
                };
                *forward_request_digest = binding_digest(b"missing source");
            }
            6 => {
                let BindingChange::Restore { selection, .. } =
                    &mut evidence.receipts[2].step.change
                else {
                    panic!()
                };
                *selection = None;
            }
            7 => {
                let first = evidence.receipts[0].request_digest.clone();
                let BindingChange::Restore {
                    forward_request_digest,
                    ..
                } = &mut evidence.receipts[2].step.change
                else {
                    panic!()
                };
                *forward_request_digest = first;
            }
            _ => unreachable!(),
        }
        // Recompute hashes and projected after-states too: the chain checks,
        // not just a checksum mismatch, must reject semantically forged data.
        for receipt in &mut evidence.receipts {
            receipt.request_digest = receipt.step.request_digest().unwrap();
            if matches!(receipt.outcome, BindingOutcome::Applied { .. }) {
                receipt.outcome = BindingOutcome::Applied {
                    after: receipt.step.after().unwrap(),
                };
            }
        }
        save(&path, evidence);
        assert!(Bindings::open(&path).is_err(), "mutation {mutation}");
    }
}

#[test]
fn rejection_never_advances_binding_and_capacity_rejects_without_pruning() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE);
    let mut rejected = receipt(fixture());
    rejected.outcome = BindingOutcome::Rejected {
        reason: BindingRejection::TargetChanged,
    };
    let receipts = (0..MAX_BINDING_RECORDS)
        .map(|index| {
            let mut record = rejected.clone();
            record.step.operation_id = format!("rejected-operation-{index:04}");
            record.request_digest = record.step.request_digest().unwrap();
            record
        })
        .collect::<Vec<_>>();
    save(&path, ledger(receipts.clone()));
    let reader = Bindings::open(&path).unwrap();
    assert_eq!(
        reader.ledger.as_ref().unwrap().current,
        BindingSnapshot::initial()
    );
    let mut too_many = receipts;
    too_many.push(rejected);
    save(&path, ledger(too_many));
    let before = fs::read(&path).unwrap();
    assert!(Bindings::open(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn corrupt_unknown_oversized_public_linked_and_special_evidence_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE);
    for contents in [b"fixture-private-value".as_slice(), b"{}", b"null"] {
        atomic_write(&path, contents, 0o600).unwrap();
        let error = Bindings::open(&path).err().unwrap();
        assert!(!error.to_string().contains("fixture-private-value"));
    }
    let mut unknown = ledger(chain());
    unknown.schema = 2;
    save(&path, unknown);
    assert!(Bindings::open(&path).is_err());
    save(&path, ledger(chain()));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Bindings::open(&path).is_err());
    save(&path, ledger(chain()));
    let linked = root.path().join("linked");
    fs::hard_link(&path, &linked).unwrap();
    assert!(Bindings::open(&linked).is_err());
    let symlink = root.path().join("symlink");
    std::os::unix::fs::symlink(&path, &symlink).unwrap();
    assert!(Bindings::open(&symlink).is_err());
    assert!(Bindings::open(root.path()).is_err());
    let huge = root.path().join("huge");
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&huge)
        .unwrap();
    file.set_len(MAX_BINDING_BYTES + 1).unwrap();
    assert!(Bindings::open(&huge).is_err());
    let fifo = root.path().join("fifo");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        0,
    )
    .unwrap();
    assert!(Bindings::open(&fifo).is_err());
}

#[tokio::test]
#[cfg(feature = "full")]
async fn signed_legacy_and_otlp_exports_cannot_bypass_managed_binding_after_reopen() {
    use crate::machine_plugins::{PluginExecutionScope, PluginHostRequest};
    let _ = rustls::crypto::ring::default_provider().install_default();
    for version in ["1.0.0", "1.1.0"] {
        let root = tempfile::tempdir().unwrap();
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
                .unwrap();
        let desired = crate::machine_plugins::tests::telemetry_release(&publisher, version);
        let state = root.path().join("machine");
        let store = MachinePluginStore::new(&state, Platform::Linux, "x86_64".into()).unwrap();
        let installed = store.install(&desired).await.unwrap();
        let untracked = store
            .telemetry_binding_observation(&fixture(), Some("service-test"), "machine-test")
            .await;
        assert!(untracked.matches(&fixture()));
        let path = state.join("plugin-operations").join(FILE);
        assert!(!path.exists());
        drop(store);
        save(&path, ledger(chain()));
        let before = fs::read(&path).unwrap();
        let store = MachinePluginStore::new(&state, Platform::Linux, "x86_64".into()).unwrap();
        let observation = store
            .telemetry_binding_observation(&fixture(), Some("service-test"), "machine-test")
            .await;
        assert!(observation.matches(&fixture()));
        assert_eq!(
            store
                .telemetry_binding_observation(&fixture(), None, "machine-test")
                .await,
            unavailable(BindingUnavailable::WrongOwner)
        );
        for operation in [
            PluginHostOperation::ExportTelemetry,
            PluginHostOperation::ExportOtlp,
        ] {
            let scope = PluginExecutionScope::new(Some("service-test"), "machine-test");
            let invocation = scope.host(PluginHostRequest {
                plugin_id: installed.plugin_id.clone(),
                plugin_version: installed.plugin_version.clone(),
                generation_digest: installed.generation_digest.clone(),
                auth_generation: None,
                operation,
                payload: serde_json::json!({}),
            });
            let error = store.invoke_host(invocation).await.unwrap_err();
            assert!(!error.started);
            assert!(error.error.to_string().contains("reader-only"));
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            store
                .inventory_one("victoria")
                .unwrap()
                .unwrap()
                .generation_digest,
            installed.generation_digest
        );
        // The old journal schema cannot silently treat a managed binding as
        // an ordinary uninstall receipt. It is below the accepted reader floor.
        assert!(serde_json::from_slice::<super::super::Record>(&before).is_err());
    }
}
