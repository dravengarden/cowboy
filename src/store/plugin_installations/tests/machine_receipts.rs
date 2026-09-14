use super::*;
use crate::machine_protocol::plugin_install::{
    InstallPhase as MachinePhase, InstallRejection, InstallTarget, InstallUncertainty,
};
use crate::plugin_operation::installation::machine_fixture;

fn receipt(intent: &InstallIntent, outcome: InstallOutcome) -> InstallReceipt {
    let step = intent.machine_step().unwrap();
    InstallReceipt {
        request_digest: step.request_digest().unwrap(),
        step,
        outcome,
    }
}

fn applied() -> InstallOutcome {
    InstallOutcome::Applied {
        revision: format!("installation-{}", "d".repeat(64))
            .try_into()
            .unwrap(),
    }
}

async fn installing(store: &Store, intent: &InstallIntent) {
    store.begin_plugin_install(intent).await.unwrap();
    store
        .advance_plugin_install(
            intent,
            InstallPhase::Prepared,
            InstallPhase::Installing,
            None,
        )
        .await
        .unwrap();
}

async fn contract(store: &Store) {
    store.migrate().await.unwrap();
    for (index, outcome) in [
        applied(),
        InstallOutcome::Rejected {
            reason: InstallRejection::TargetChanged,
        },
        InstallOutcome::Pending {
            phase: MachinePhase::Staging,
        },
        InstallOutcome::Unknown {
            phase: MachinePhase::Activating,
            reason: InstallUncertainty::Interrupted,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let mut intent = machine_fixture(&format!("receipt-{index}"));
        intent.plugin_id = format!("receipt-plugin-{index}");
        installing(store, &intent).await;
        assert!(
            store
                .advance_plugin_install(
                    &intent,
                    InstallPhase::Installing,
                    InstallPhase::MachineAcknowledged,
                    None
                )
                .await
                .is_err(),
            "generic ACK cannot certify schema two"
        );
        assert!(
            store
                .advance_plugin_install(
                    &intent,
                    InstallPhase::Installing,
                    InstallPhase::Aborted,
                    Some(InstallProblem::MachineRejected)
                )
                .await
                .is_err(),
            "rejection needs a full receipt"
        );
        let receipt = receipt(&intent, outcome.clone());
        reject_changed_intent(store, &intent, &receipt).await;
        let saved = store
            .record_plugin_install_receipt(&intent, &receipt)
            .await
            .unwrap();
        assert_eq!(saved.machine_receipt, Some(receipt.clone()));
        let expected = match outcome {
            InstallOutcome::Applied { .. } => InstallPhase::MachineAcknowledged,
            InstallOutcome::Rejected { .. } => InstallPhase::Aborted,
            _ => InstallPhase::NeedsAttention,
        };
        assert_eq!(saved.phase, expected);
        assert!(
            store
                .record_plugin_install_receipt(&intent, &receipt)
                .await
                .is_err(),
            "observation is not a fresh writer"
        );
        assert_eq!(
            store
                .plugin_install_operation(&intent.operation_id)
                .await
                .unwrap()
                .unwrap(),
            saved
        );
        if expected == InstallPhase::MachineAcknowledged {
            store
                .advance_plugin_install(&intent, expected, InstallPhase::Completed, None)
                .await
                .unwrap();
        }
        let mut next = intent.clone();
        next.operation_id.push('n');
        next.request_id.push('n');
        assert_eq!(
            store.begin_plugin_install(&next).await.is_ok(),
            expected != InstallPhase::NeedsAttention
        );
    }
    let recovered = store.recover_plugin_installs("service-test").await.unwrap();
    assert_eq!(recovered.len(), 4);
    assert_eq!(
        store.recover_plugin_installs("service-test").await.unwrap(),
        recovered
    );
}

async fn reject_changed_intent(store: &Store, intent: &InstallIntent, receipt: &InstallReceipt) {
    for change in ["actor", "target", "envelope", "deadline", "operation"] {
        let mut changed = intent.clone();
        match change {
            "actor" => {
                changed.actor = crate::plugin_operation::Actor::Product {
                    user_id: "another-operator".into(),
                }
            }
            "target" => {
                changed.machine_target = Some(InstallTarget::Removed {
                    revision: format!("installation-{}", "e".repeat(64))
                        .try_into()
                        .unwrap(),
                });
            }
            "envelope" => changed.envelope_digest = format!("sha256:{}", "e".repeat(64)),
            "deadline" => changed.expires_at_ms += 1,
            _ => {
                changed.operation_id.push('x');
                changed.request_id.push('x');
            }
        }
        assert!(
            store
                .record_plugin_install_receipt(&changed, receipt)
                .await
                .is_err(),
            "{change}"
        );
    }
}

#[tokio::test]
async fn sqlite_machine_receipt_atomic_contract() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture: just test-postgres"]
async fn postgres_machine_receipt_atomic_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL fixture");
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
async fn applied_receipt_survives_repeated_cold_open_without_finishing_or_renewing_authority() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("receipt.sqlite").display());
    let intent = machine_fixture("reopen-receipt");
    let receipt = receipt(&intent, applied());
    let mut previous = None;
    for opening in 0..3 {
        let store = Store::connect(&url, root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        if opening == 0 {
            installing(&store, &intent).await;
            store
                .record_plugin_install_receipt(&intent, &receipt)
                .await
                .unwrap();
        } else {
            let rows = store.recover_plugin_installs("service-test").await.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].phase, InstallPhase::NeedsAttention);
            assert_eq!(
                rows[0].attention_from,
                Some(InstallPhase::MachineAcknowledged)
            );
            assert_eq!(rows[0].machine_receipt, Some(receipt.clone()));
            if let Some(before) = previous {
                assert_eq!(before, rows);
            }
            previous = Some(rows);
            assert!(
                store
                    .record_plugin_install_receipt(&intent, &receipt)
                    .await
                    .is_err()
            );
        }
        let StorageBackend::Sqlite(db) = &store.backend else {
            unreachable!()
        };
        db.pool.close().await;
    }
}

#[tokio::test]
async fn invalid_receipts_fail_closed_without_rewriting_any_progress() {
    for corruption in [
        "checksum",
        "unknown-field",
        "actor",
        "phase",
        "missing",
        "legacy",
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let mut intent = machine_fixture("corrupt-receipt");
        installing(&store, &intent).await;
        let saved_receipt = receipt(&intent, applied());
        store
            .record_plugin_install_receipt(&intent, &saved_receipt)
            .await
            .unwrap();
        let StorageBackend::Sqlite(db) = &store.backend else {
            unreachable!()
        };
        match corruption {
            "missing" => {
                sqlx::query("UPDATE plugin_install_operations SET machine_receipt = NULL, machine_receipt_sha256 = NULL").execute(&db.pool).await.unwrap();
            }
            "phase" => {
                sqlx::query("UPDATE plugin_install_operations SET phase = 'prepared'")
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
            "legacy" => {
                intent.schema = 1;
                intent.machine_target = None;
                let doc = serde_json::to_string(&intent).unwrap();
                sqlx::query("UPDATE plugin_install_operations SET intent = $1, intent_sha256 = $2")
                    .bind(&doc)
                    .bind(format!("{:x}", Sha256::digest(doc.as_bytes())))
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
            _ => {
                let mut doc = serde_json::to_value(&saved_receipt).unwrap();
                if corruption == "unknown-field" {
                    doc["retry"] = true.into();
                }
                if corruption == "actor" {
                    doc["step"]["plan_digest"] = format!("sha256:{}", "f".repeat(64)).into();
                }
                let doc = serde_json::to_string(&doc).unwrap();
                let checksum = if corruption == "checksum" {
                    "f".repeat(64)
                } else {
                    format!("{:x}", Sha256::digest(doc.as_bytes()))
                };
                sqlx::query("UPDATE plugin_install_operations SET machine_receipt = $1, machine_receipt_sha256 = $2").bind(doc).bind(checksum).execute(&db.pool).await.unwrap();
            }
        }
        assert!(
            store.recover_plugin_installs("service-test").await.is_err(),
            "{corruption}"
        );
        assert!(
            store
                .plugin_install_operation(&intent.operation_id)
                .await
                .is_err()
        );
        let problem: Option<String> =
            sqlx::query_scalar("SELECT problem FROM plugin_install_operations")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(
            problem.is_none(),
            "corruption must not be rewritten as an interruption"
        );
    }
}

#[tokio::test]
async fn concurrent_receipt_writers_cannot_change_the_first_terminal_observation() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect(
        &format!("sqlite://{}", root.path().join("race.sqlite").display()),
        root.path().join("artifacts"),
    )
    .await
    .unwrap();
    store.migrate().await.unwrap();
    let intent = machine_fixture("concurrent-receipt");
    installing(&store, &intent).await;
    let left = receipt(&intent, applied());
    let right = receipt(
        &intent,
        InstallOutcome::Rejected {
            reason: InstallRejection::Expired,
        },
    );
    let (left_result, right_result) = tokio::join!(
        store.record_plugin_install_receipt(&intent, &left),
        store.record_plugin_install_receipt(&intent, &right)
    );
    assert_ne!(left_result.is_ok(), right_result.is_ok());
    let saved = store
        .plugin_install_operation(&intent.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved, left_result.or(right_result).unwrap());
}

#[test]
fn schema_one_serialization_is_unchanged_and_cannot_be_promoted_without_a_target() {
    let legacy = fixture("retained");
    let doc = serde_json::to_string(&legacy).unwrap();
    assert!(!doc.contains("machine_target"));
    assert_eq!(serde_json::from_str::<InstallIntent>(&doc).unwrap(), legacy);
    assert!(legacy.machine_step().is_err());
    let mut changed = legacy.clone();
    changed.schema = 2;
    assert!(changed.validate().is_err());
    changed = legacy;
    changed.machine_target = Some(InstallTarget::Vacant {});
    assert!(changed.validate().is_err());
}
