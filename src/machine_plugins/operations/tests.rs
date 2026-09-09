use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn step() -> UninstallStep {
    UninstallStep {
        schema: 1,
        operation_id: "operation-fixture-0001".into(),
        service_id: "service-a".into(),
        machine_id: "machine-a".into(),
        plan_digest: digest(b"approved actor and impact"),
        plugin_id: "victoria".into(),
        plugin_version: "1.0.0".into(),
        generation_digest: digest(b"release"),
        contract_fingerprint: digest(b"contract"),
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 60_000,
    }
}

fn receipt(result: StepLookup) -> Box<StepReceipt> {
    let StepLookup::Found { receipt } = result else {
        panic!("expected receipt, got {result:?}");
    };
    receipt
}

#[test]
fn durable_duplicate_and_reopen_never_repeat_the_effect() {
    let root = tempfile::tempdir().unwrap();
    let request = step();
    let count = AtomicUsize::new(0);
    let journal = Journal::open(root.path()).unwrap();
    let first = journal.execute(
        &request,
        || true,
        || {
            // The intent is on disk BEFORE entering the effect.
            let record: Record = serde_json::from_slice(
                &fs::read(
                    journal
                        .root
                        .join(format!("{}.json", request.key().unwrap())),
                )
                .unwrap(),
            )
            .unwrap();
            assert!(matches!(
                record.receipt.outcome,
                StepOutcome::Unknown { .. }
            ));
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    );
    assert_eq!(receipt(first.clone()).outcome, StepOutcome::Applied {});
    assert_eq!(
        journal.execute(
            &request,
            || panic!("duplicate precondition"),
            || panic!("duplicate effect")
        ),
        first
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    drop(journal);
    let journal = Journal::open(root.path()).unwrap();
    assert_eq!(journal.query(&request), first);
    assert_eq!(journal.execute(&request, || panic!(), || panic!()), first);
}

#[test]
fn changed_parameters_cannot_reuse_a_receipt_or_change_its_target() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let original = step();
    let result = journal.execute(&original, || true, || Ok(()));
    for field in [
        "plugin_id",
        "machine_id",
        "plan_digest",
        "generation_digest",
        "contract_fingerprint",
        "plugin_version",
        "expires_at_ms",
    ] {
        let mut encoded = serde_json::to_value(&original).unwrap();
        encoded[field] = if field == "expires_at_ms" {
            serde_json::json!(original.expires_at_ms + 1)
        } else if field == "plugin_version" {
            serde_json::json!("1.0.1")
        } else if field.ends_with("digest") || field == "contract_fingerprint" {
            serde_json::json!(digest(b"different"))
        } else {
            serde_json::json!("different")
        };
        let changed: UninstallStep = serde_json::from_value(encoded).unwrap();
        assert_eq!(
            journal.execute(&changed, || panic!(), || panic!()),
            unavailable(StepUnavailable::IdentityConflict),
            "{field}"
        );
    }
    assert_eq!(journal.query(&original), result);
}

#[test]
fn interrupted_effect_reopens_fenced_and_unknown_without_replay() {
    let root = tempfile::tempdir().unwrap();
    let request = step();
    let journal = Journal::open(root.path()).unwrap();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        journal.execute(
            &request,
            || true,
            || panic!("simulated process interruption"),
        )
    }));
    assert!(panicked.is_err());
    drop(journal);
    let journal = Journal::open(root.path()).unwrap();
    assert_eq!(
        receipt(journal.query(&request)).outcome,
        StepOutcome::Unknown {
            reason: StepUncertainty::Interrupted
        }
    );
    assert_eq!(
        journal.execute(&request, || panic!(), || panic!()),
        journal.query(&request)
    );
    assert!(journal.ensure_unfenced("victoria").is_err());
    assert!(journal.ensure_unfenced("unrelated").is_ok());
    let mut next = request;
    next.operation_id.push('2');
    assert_eq!(
        journal.execute(&next, || panic!(), || panic!()),
        unavailable(StepUnavailable::SlotFenced)
    );
}

#[test]
fn returned_effect_error_is_not_rejection_and_legacy_cannot_bypass_it() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let request = step();
    let result = journal.execute(
        &request,
        || true,
        || bail!("private error must never be stored"),
    );
    assert_eq!(
        receipt(result).outcome,
        StepOutcome::Unknown {
            reason: StepUncertainty::EffectFailure
        }
    );
    assert!(journal.ensure_legacy_allowed("victoria").is_err());
    let bytes = fs::read(
        journal
            .root
            .join(format!("{}.json", request.key().unwrap())),
    )
    .unwrap();
    assert!(!String::from_utf8(bytes).unwrap().contains("private error"));
}

#[test]
fn expired_and_changed_targets_get_definitive_no_effect_receipts() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let mut request = step();
    request.expires_at_ms = 1;
    assert_eq!(
        receipt(journal.execute(&request, || panic!(), || panic!())).outcome,
        StepOutcome::Rejected {
            reason: StepRejection::Expired
        }
    );
    request.operation_id.push('2');
    request.expires_at_ms = chrono::Utc::now().timestamp_millis() + 60_000;
    assert_eq!(
        receipt(journal.execute(&request, || false, || panic!())).outcome,
        StepOutcome::Rejected {
            reason: StepRejection::TargetChanged
        }
    );
    assert!(journal.ensure_unfenced("victoria").is_ok());
}

#[test]
fn corruption_unknown_schema_and_oversize_fail_closed_on_open() {
    for corruption in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::open(root.path()).unwrap();
        let request = step();
        journal.execute(&request, || true, || Ok(()));
        let path = journal
            .root
            .join(format!("{}.json", request.key().unwrap()));
        drop(journal);
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match corruption {
            0 => record["schema"] = 2.into(),
            1 => record["receipt"]["outcome"]["state"] = "invented".into(),
            2 => record["receipt"]["step"]["plugin_id"] = "changed".into(),
            _ => record["unexpected"] = "x".repeat(9000).into(),
        }
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(
            Journal::open(root.path()).is_err(),
            "corruption {corruption}"
        );
    }
}

#[test]
fn storage_failure_runs_no_effect_and_fences_the_process() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let request = step();
    fs::create_dir(
        journal
            .root
            .join(format!("{}.json", request.key().unwrap())),
    )
    .unwrap();
    assert_eq!(
        journal.execute(&request, || panic!(), || panic!()),
        unavailable(StepUnavailable::Storage)
    );
    assert_eq!(
        journal.query(&request),
        unavailable(StepUnavailable::Storage)
    );
    assert!(journal.ensure_unfenced("unrelated").is_err());
}

#[test]
fn writer_is_single_owner_and_query_does_not_claim_an_operation() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    assert!(Journal::open(root.path()).is_err());
    assert_eq!(journal.query(&step()), StepLookup::NotFound {});
    assert!(journal.state.lock().receipts.is_empty());
    assert_eq!(
        fs::metadata(&journal.root).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn receipt_flush_failure_never_claims_applied() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let request = step();
    let path = journal
        .root
        .join(format!("{}.json", request.key().unwrap()));
    let saved = root.path().join("saved-intent");
    let result = journal.execute(
        &request,
        || true,
        || {
            // Force the post-effect receipt rename to fail; preserve the original
            // intent so reopen exercises exactly the unresolved crash window.
            fs::rename(&path, &saved)?;
            fs::create_dir(&path)?;
            Ok(())
        },
    );
    assert_eq!(result, unavailable(StepUnavailable::Storage));
    assert!(journal.ensure_unfenced("victoria").is_err());
    fs::remove_dir(&path).unwrap();
    fs::rename(&saved, &path).unwrap();
    drop(journal);
    let journal = Journal::open(root.path()).unwrap();
    assert_eq!(
        receipt(journal.query(&request)).outcome,
        StepOutcome::Unknown {
            reason: StepUncertainty::Interrupted
        }
    );
}

#[test]
fn capacity_preserves_duplicate_queries_and_unresolved_evidence() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let original = step();
    let saved = journal.execute(&original, || true, || Ok(()));
    {
        let mut state = journal.state.lock();
        for n in 1..MAX_RECORDS {
            let mut step = original.clone();
            step.operation_id = format!("capacity-operation-{n:08}");
            state.receipts.insert(
                step.key().unwrap(),
                StepReceipt {
                    request_digest: step.request_digest().unwrap(),
                    step,
                    outcome: StepOutcome::Applied {},
                },
            );
        }
    }
    let mut next = original.clone();
    next.operation_id.push('2');
    assert_eq!(
        journal.execute(&next, || panic!(), || panic!()),
        unavailable(StepUnavailable::Capacity)
    );
    assert_eq!(journal.execute(&original, || panic!(), || panic!()), saved);
}

#[test]
fn journal_cannot_follow_symlinks_to_external_evidence() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let path = journal.root.clone();
    drop(journal);
    let external = root.path().join("external");
    fs::write(&external, b"not a receipt").unwrap();
    symlink(&external, path.join("record.json")).unwrap();
    assert!(Journal::open(root.path()).is_err());
    assert_eq!(fs::read(external).unwrap(), b"not a receipt");
}

#[tokio::test]
#[cfg(feature = "full")]
async fn signed_lifecycle_and_reader_bridge_preserve_receipts_without_reactivation() {
    let root = tempfile::tempdir().unwrap();
    let publisher =
        crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
            .unwrap();
    let desired = super::super::tests::telemetry_release(&publisher, "1.0.0");
    let path = root.path().join("machine");
    let store = MachinePluginStore::new(&path, Platform::Linux, "x86_64".into()).unwrap();
    let installed = store.install(&desired).await.unwrap();
    let mut request = step();
    request.generation_digest = installed.generation_digest;
    request.contract_fingerprint = installed.contract_fingerprint;
    let query = store
        .uninstall_step(&request, Some("service-a"), "machine-a", false, true)
        .await;
    assert!(!query.admission_enabled);
    assert_eq!(query.result, StepLookup::NotFound {});
    let denied = store
        .uninstall_step(&request, Some("service-a"), "machine-a", false, false)
        .await;
    assert_eq!(denied.result, unavailable(StepUnavailable::ReaderOnly));
    for (service, machine) in [
        (None, "machine-a"),
        (Some("other"), "machine-a"),
        (Some("service-a"), "other"),
    ] {
        assert_eq!(
            store
                .uninstall_step(&request, service, machine, true, false)
                .await
                .result,
            unavailable(StepUnavailable::WrongOwner)
        );
    }
    assert_eq!(store.inventory().unwrap().len(), 1);
    let applied = store
        .uninstall_step(&request, Some("service-a"), "machine-a", true, false)
        .await;
    assert_eq!(
        receipt(applied.result.clone()).outcome,
        StepOutcome::Applied {}
    );
    assert!(store.inventory().unwrap().is_empty());
    // New installation is an independent explicit request; replay of the old
    // uninstall must return its receipt, never remove this later installation.
    store.install(&desired).await.unwrap();
    assert_eq!(
        store
            .uninstall_step(&request, Some("service-a"), "machine-a", true, false)
            .await,
        applied
    );
    assert_eq!(store.inventory().unwrap().len(), 1);
    assert!(
        store
            .uninstall("victoria", &request.generation_digest)
            .await
            .is_err()
    );
    assert!(
        store
            .reactivate("victoria", &request.generation_digest)
            .await
            .is_err()
    );
    drop(store);
    let store = MachinePluginStore::new(&path, Platform::Linux, "x86_64".into()).unwrap();
    let observed = store
        .uninstall_step(&request, Some("service-a"), "machine-a", false, true)
        .await;
    assert_eq!(observed.result, applied.result);
    assert_eq!(store.inventory().unwrap().len(), 1);
}
