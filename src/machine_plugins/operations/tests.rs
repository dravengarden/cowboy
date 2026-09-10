use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "full")]
mod recovery;

mod execution_lease;

impl Journal {
    // Existing durability fixtures still exercise the production lease path.
    // Lease-specific fixtures below retain and revoke their scope explicitly.
    fn fixture_execute(
        &self,
        step: &UninstallStep,
        precondition: impl FnOnce() -> bool,
        effect: impl FnOnce() -> Result<()>,
    ) -> StepLookup {
        let scope = lease::PluginExecutionScope::new(Some(&step.service_id), &step.machine_id);
        let lease = match scope.uninstall(step) {
            Ok(lease) => lease,
            Err(reason) => return unavailable(reason),
        };
        self.execute(step, &lease, precondition, effect)
    }
}

#[cfg(feature = "full")]
impl MachinePluginStore {
    async fn fixture_uninstall_step(
        &self,
        step: &UninstallStep,
        service: Option<&str>,
        machine: &str,
        admission: bool,
        query_only: bool,
    ) -> StepObservation {
        let scope = lease::PluginExecutionScope::new(service, machine);
        let lease = if query_only {
            None
        } else {
            match scope.uninstall(step) {
                Ok(lease) => Some(lease),
                Err(reason) => {
                    return StepObservation {
                        admission_enabled: false,
                        result: unavailable(reason),
                    };
                }
            }
        };
        self.uninstall_step(
            step,
            service,
            machine,
            admission,
            lease
                .as_ref()
                .map_or(UninstallAccess::Observe, UninstallAccess::Execute),
        )
        .await
    }
}

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
        installation_revision: None,
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
    let first = journal.fixture_execute(
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
        journal.fixture_execute(
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
    assert_eq!(
        journal.fixture_execute(&request, || panic!(), || panic!()),
        first
    );
}

#[test]
#[allow(clippy::used_underscore_binding)] // Inspect the private RAII guard, not production state.
fn graceful_owner_release_does_not_wait_for_an_inherited_file_description() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    // dup has the same open-file-description/flock lifetime as fork inheritance.
    let inherited = journal._owner.0.try_clone().unwrap();
    assert!(Journal::open(root.path()).is_err());
    drop(journal);
    let reopened = Journal::open(root.path()).unwrap();
    assert!(Journal::open(root.path()).is_err());
    drop(inherited);
    assert!(Journal::open(root.path()).is_err());
    drop(reopened);
    assert!(Journal::open(root.path()).is_ok());
}

#[test]
fn changed_parameters_cannot_reuse_a_receipt_or_change_its_target() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let original = step();
    let result = journal.fixture_execute(&original, || true, || Ok(()));
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
            journal.fixture_execute(&changed, || panic!(), || panic!()),
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
        journal.fixture_execute(
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
        journal.fixture_execute(&request, || panic!(), || panic!()),
        journal.query(&request)
    );
    assert!(journal.ensure_unfenced("victoria").is_err());
    assert!(journal.ensure_unfenced("unrelated").is_ok());
    let mut next = request;
    next.operation_id.push('2');
    assert_eq!(
        journal.fixture_execute(&next, || panic!(), || panic!()),
        unavailable(StepUnavailable::SlotFenced)
    );
}

#[test]
fn returned_effect_error_is_not_rejection_and_legacy_cannot_bypass_it() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path()).unwrap();
    let request = step();
    let result = journal.fixture_execute(
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
        receipt(journal.fixture_execute(&request, || panic!(), || panic!())).outcome,
        StepOutcome::Rejected {
            reason: StepRejection::Expired
        }
    );
    request.operation_id.push('2');
    request.expires_at_ms = chrono::Utc::now().timestamp_millis() + 60_000;
    assert_eq!(
        receipt(journal.fixture_execute(&request, || false, || panic!())).outcome,
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
        journal.fixture_execute(&request, || true, || Ok(()));
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
        journal.fixture_execute(&request, || panic!(), || panic!()),
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
    let result = journal.fixture_execute(
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
    let saved = journal.fixture_execute(&original, || true, || Ok(()));
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
        journal.fixture_execute(&next, || panic!(), || panic!()),
        unavailable(StepUnavailable::Capacity)
    );
    assert_eq!(
        journal.fixture_execute(&original, || panic!(), || panic!()),
        saved
    );
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
        .fixture_uninstall_step(&request, Some("service-a"), "machine-a", false, true)
        .await;
    assert!(!query.admission_enabled);
    assert_eq!(query.result, StepLookup::NotFound {});
    let denied = store
        .fixture_uninstall_step(&request, Some("service-a"), "machine-a", false, false)
        .await;
    assert_eq!(denied.result, unavailable(StepUnavailable::ReaderOnly));
    for (service, machine) in [
        (None, "machine-a"),
        (Some("other"), "machine-a"),
        (Some("service-a"), "other"),
    ] {
        assert_eq!(
            store
                .fixture_uninstall_step(&request, service, machine, true, false)
                .await
                .result,
            unavailable(StepUnavailable::WrongOwner)
        );
    }
    assert_eq!(store.inventory().unwrap().len(), 1);
    let applied = store
        .fixture_uninstall_step(&request, Some("service-a"), "machine-a", true, false)
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
            .fixture_uninstall_step(&request, Some("service-a"), "machine-a", true, false)
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
        .fixture_uninstall_step(&request, Some("service-a"), "machine-a", false, true)
        .await;
    assert_eq!(observed.result, applied.result);
    assert_eq!(store.inventory().unwrap().len(), 1);
}

#[tokio::test]
#[cfg(feature = "full")]
#[allow(clippy::too_many_lines)] // One signed ABA, tombstone and rollback-reader lifecycle.
async fn same_release_reinstall_rejects_stale_preview_and_retains_exact_receipt() {
    let root = tempfile::tempdir().unwrap();
    let publisher =
        crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
            .unwrap();
    let desired = super::super::tests::telemetry_release(&publisher, "1.0.0");
    let path = root.path().join("machine");
    let store = MachinePluginStore::new(&path, Platform::Linux, "x86_64".into()).unwrap();
    store.install(&desired).await.unwrap();
    assert!(
        store.inventory().unwrap()[0]
            .installation_revision
            .is_none()
    );
    assert!(!path.join("plugin-operations/installations-v1").exists());
    store.enable_installation_tracking().await.unwrap();
    let first = store.inventory().unwrap().remove(0);
    assert!(first.installation_revision.is_some());
    let mut request = step();
    request.schema = 2;
    request.generation_digest = first.generation_digest.clone();
    request.contract_fingerprint = first.contract_fingerprint.clone();
    request.installation_revision = first.installation_revision.clone();
    // An old Controller's new digest-only request is never admitted after cutover.
    let mut legacy = request.clone();
    legacy.schema = 1;
    legacy.installation_revision = None;
    let preflight = store
        .fixture_uninstall_step(&legacy, Some("service-a"), "machine-a", true, true)
        .await;
    assert!(!preflight.admission_enabled);
    assert_eq!(
        store
            .fixture_uninstall_step(&legacy, Some("service-a"), "machine-a", true, false)
            .await
            .result,
        unavailable(StepUnavailable::InvalidRequest)
    );
    let second = store.install(&desired).await.unwrap();
    assert_eq!(first.generation_digest, second.generation_digest);
    assert_ne!(first.installation_revision, second.installation_revision);
    assert_eq!(
        store
            .fixture_uninstall_step(&request, Some("service-a"), "machine-a", true, true)
            .await
            .result,
        unavailable(StepUnavailable::InvalidRequest)
    );
    let rejected = store
        .fixture_uninstall_step(&request, Some("service-a"), "machine-a", true, false)
        .await;
    assert_eq!(
        receipt(rejected.result).outcome,
        StepOutcome::Rejected {
            reason: StepRejection::TargetChanged
        }
    );
    assert_eq!(
        store.inventory().unwrap()[0].installation_revision,
        second.installation_revision
    );
    // Reusing the rejected identity with a fresh incarnation is a different request.
    request.installation_revision = second.installation_revision.clone();
    assert_eq!(
        store
            .fixture_uninstall_step(&request, Some("service-a"), "machine-a", true, false)
            .await
            .result,
        unavailable(StepUnavailable::IdentityConflict)
    );
    request.operation_id.push('2');
    let applied = store
        .fixture_uninstall_step(&request, Some("service-a"), "machine-a", true, false)
        .await;
    assert_eq!(
        receipt(applied.result.clone()).outcome,
        StepOutcome::Applied {}
    );
    assert!(store.inventory().unwrap().is_empty());
    let tombstone =
        fs::read(path.join("plugin-operations/installations-v1/victoria.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&tombstone).unwrap();
    assert!(value["transition"]["generation_digest"].is_null());
    assert_eq!(
        value["transition"]["operation_digest"],
        request.request_digest().unwrap()
    );
    assert_eq!(
        value["transition"]["previous_revision"],
        serde_json::to_value(second.installation_revision).unwrap()
    );
    drop(store);
    // Real reader-only reopen: both schemas remain readable, with no adoption,
    // replay, legacy fallback, normal install or accidental authority deletion.
    let reader = MachinePluginStore::new(&path, Platform::Linux, "x86_64".into()).unwrap();
    let observed = reader
        .fixture_uninstall_step(&request, Some("service-a"), "machine-a", true, true)
        .await;
    assert!(!observed.admission_enabled);
    assert_eq!(observed.result, applied.result);
    assert!(reader.install(&desired).await.is_err());
    assert!(
        reader
            .reactivate("victoria", &first.generation_digest)
            .await
            .is_err()
    );
    assert!(
        reader
            .uninstall("victoria", &first.generation_digest)
            .await
            .is_err()
    );
    assert_eq!(
        fs::read(path.join("plugin-operations/installations-v1/victoria.json")).unwrap(),
        tombstone
    );
    reader.enable_installation_tracking().await.unwrap();
    let third = reader.install(&desired).await.unwrap();
    assert_ne!(third.installation_revision, first.installation_revision);
    assert_ne!(third.installation_revision, request.installation_revision);
    assert_eq!(
        reader
            .fixture_uninstall_step(&request, Some("service-a"), "machine-a", true, false)
            .await
            .result,
        applied.result
    );
    assert_eq!(
        reader.inventory().unwrap()[0].installation_revision,
        third.installation_revision
    );
    let mut interrupted = request.clone();
    interrupted.operation_id.push('3');
    interrupted.installation_revision = third.installation_revision;
    reader
        .operations
        .fixture_execute(&interrupted, || true, || bail!("fixture partial effect"));
    interrupted.operation_id.push('4');
    assert_eq!(
        reader
            .fixture_uninstall_step(&interrupted, Some("service-a"), "machine-a", true, true)
            .await
            .result,
        unavailable(StepUnavailable::SlotFenced)
    );
}
