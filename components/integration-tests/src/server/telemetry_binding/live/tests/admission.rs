//! Actual private host policies, signed Machine store, SQL and HTTP surfaces.
//! No fixture writer switch is used by these tests.
use super::*;
use crate::server::telemetry_binding::resolution::surface::tests::Fixture as Http;
use crate::telemetry_plugin::writer_admission::{MACHINE_POLICY_FILE, tests::policy};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

const PLAN: &str = "/api/telemetry/binding/plan";
const CONFIRM: &str = "/api/telemetry/binding/confirm";

async fn http(machine: &Fixture, purposes: [bool; 3]) -> Http {
    Http::with_binding(
        false,
        false,
        machine.control.clone(),
        machine.catalog.clone(),
        machine.fences.clone(),
    )
    .await
    .with_admission(policy(
        &machine.root.path().join("service-writer.json"),
        "service-test",
        "machine-test",
        purposes,
    ))
    .await
}

fn selection(machine: &Fixture) -> Value {
    let BindingChange::Select { installation, .. } = machine.select().change else {
        unreachable!()
    };
    json!({"action":"select", "target":{"machine_id":"machine-test", "installation":installation}})
}

fn confirmation(plan: &Value) -> Value {
    json!({"plan_id":plan["plan_id"], "action":plan["action"]})
}

#[tokio::test]
async fn explicit_pair_policies_allow_one_confirmed_write_and_revocation_leaves_durable_reads() {
    let machine = Fixture::setup_admission(false, false, false, Some([true, false, false])).await;
    let f = http(&machine, [true, false, false]).await;
    assert!(f.state.ledger().await.unwrap().is_none());
    assert!(f.state.legacy_fence.allows_legacy());
    let (status, plan) = f
        .request(Method::POST, PLAN, Some(selection(&machine)))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plan["confirmation_available"], true);
    assert!(
        f.state.ledger().await.unwrap().is_none(),
        "preview creates no namespace"
    );
    let (status, receipt) = f
        .request(Method::POST, CONFIRM, Some(confirmation(&plan)))
        .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["operation"]["phase"], "completed");
    assert_eq!(machine.sends.load(Ordering::Relaxed), 1);
    fs::remove_file(machine.root.path().join("service-writer.json")).unwrap();
    fs::remove_file(
        machine
            .root
            .path()
            .join("machine")
            .join(MACHINE_POLICY_FILE),
    )
    .unwrap();
    let (status, _) = f
        .request(Method::POST, CONFIRM, Some(confirmation(&plan)))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, history) = f
        .request(
            Method::GET,
            &format!(
                "/api/telemetry/binding/operations/{}/receipt",
                plan["plan_id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(history, receipt);
    assert!(!f.state.legacy_fence.allows_legacy());
    let ledger = f.state.ledger().await.unwrap().unwrap();
    let step = ledger.operations[0].intent.machine_step().unwrap();
    assert!(
        machine
            .machine
            .telemetry_binding_observation(&step, Some("service-test"), "machine-test")
            .await
            .matches(&step)
    );
    assert_eq!(machine.sends.load(Ordering::Relaxed), 1);
    f.stop().await;
}

#[tokio::test]
async fn service_and_machine_revocation_between_preview_and_confirm_are_independent() {
    for side in ["service", "machine"] {
        let machine =
            Fixture::setup_admission(false, false, false, Some([true, false, false])).await;
        let f = http(&machine, [true, false, false]).await;
        let (_, plan) = f
            .request(Method::POST, PLAN, Some(selection(&machine)))
            .await;
        assert_eq!(plan["confirmation_available"], true);
        let path = if side == "service" {
            machine.root.path().join("service-writer.json")
        } else {
            machine
                .root
                .path()
                .join("machine")
                .join(MACHINE_POLICY_FILE)
        };
        fs::remove_file(path).unwrap();
        let (status, receipt) = f
            .request(Method::POST, CONFIRM, Some(confirmation(&plan)))
            .await;
        if side == "service" {
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(receipt["error"], "binding_admission_closed");
            assert!(f.state.ledger().await.unwrap().is_none());
            assert!(f.state.legacy_fence.allows_legacy());
            assert_eq!(machine.sends.load(Ordering::Relaxed), 0);
        } else {
            // A remote refusal cannot undo the already durable Service intent.
            assert_ne!(receipt["operation"]["phase"], "completed");
            assert!(f.state.ledger().await.unwrap().is_some());
            assert!(!f.state.legacy_fence.allows_legacy());
            assert_eq!(machine.sends.load(Ordering::Relaxed), 1);
        }
        assert!(
            !machine
                .root
                .path()
                .join("machine/plugin-operations/telemetry-bindings-v1.json")
                .exists()
        );
        f.stop().await;
    }
}

#[tokio::test]
async fn wrong_target_and_other_purposes_do_not_admit_service_binding() {
    for (target, purposes) in [
        ("foreign-machine", [true, true, true]),
        ("machine-test", [false, true, true]),
    ] {
        let machine =
            Fixture::setup_admission(false, false, false, Some([true, false, false])).await;
        let f = Http::with_binding(
            false,
            false,
            machine.control.clone(),
            machine.catalog.clone(),
            machine.fences.clone(),
        )
        .await
        .with_admission(policy(
            &machine.root.path().join("service-writer.json"),
            "service-test",
            target,
            purposes,
        ))
        .await;
        let (status, plan) = f
            .request(Method::POST, PLAN, Some(selection(&machine)))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(plan["confirmation_available"], false);
        assert_eq!(
            f.request(Method::POST, CONFIRM, Some(confirmation(&plan)))
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert!(f.state.ledger().await.unwrap().is_none());
        assert_eq!(machine.sends.load(Ordering::Relaxed), 0);
        f.stop().await;
    }
}

#[tokio::test]
async fn recovery_only_host_policies_require_a_separate_service_resolution_activation_and_confirmation()
 {
    let machine = Fixture::setup_admission(false, false, true, Some([false, true, false])).await;
    let f = http(&machine, [false, true, false]).await;
    let intent = machine.pending.as_ref().unwrap();
    let store = f.state.store.as_ref().unwrap();
    let before = store
        .change_telemetry_binding(&Change::Begin(intent), &|| true)
        .await
        .unwrap()
        .operation;
    let before = super::super::super::advance(store, &before, Progress::Dispatching)
        .await
        .unwrap();
    let before = super::super::super::advance(
        store,
        &before,
        Progress::NeedsAttention {
            reason: crate::telemetry_binding::Attention::Uncertain,
            observation: None,
        },
    )
    .await
    .unwrap();
    let base = format!("/api/telemetry/binding/operations/{}", intent.operation_id);
    let original = f.state.ledger().await.unwrap().unwrap();
    let (status, plan) = f
        .request(
            Method::POST,
            &format!("{base}/machine-recovery-plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    assert_eq!(plan["confirmation_available"], true);
    let (status, receipt) = f
        .request(
            Method::POST,
            &format!("{base}/recover-machine"),
            Some(confirmation(&plan)),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(
        f.state.ledger().await.unwrap().unwrap(),
        original,
        "Machine recovery is not Service resolution"
    );
    let (_, resolution) = f
        .request(
            Method::POST,
            &format!("{base}/resolution-plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(resolution["confirmation_available"], false);
    assert_eq!(
        f.request(
            Method::POST,
            &format!("{base}/resolve"),
            Some(confirmation(&resolution))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let f = f
        .with_admission(policy(
            &machine.root.path().join("service-writer.json"),
            "service-test",
            "machine-test",
            [false, false, true],
        ))
        .await;
    // Explicit host reload invalidates all in-memory previews; new Operator
    // confirmation must adopt the definite result, without another Machine write.
    assert_eq!(
        f.request(
            Method::POST,
            &format!("{base}/resolve"),
            Some(confirmation(&resolution))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, resolution) = f
        .request(
            Method::POST,
            &format!("{base}/resolution-plan"),
            Some(json!({})),
        )
        .await;
    assert_eq!(resolution["confirmation_available"], true);
    let (status, receipt) = f
        .request(
            Method::POST,
            &format!("{base}/resolve"),
            Some(confirmation(&resolution)),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["phase"], "rejected");
    assert_eq!(
        f.state.ledger().await.unwrap().unwrap().resolutions.len(),
        1
    );
    assert_eq!(machine.sends.load(Ordering::Relaxed), 1);
    assert!(
        !LegacyFence::recover(Some(store_for(&f)), &before.intent.service_id)
            .await
            .unwrap()
            .allows_legacy()
    );
    f.stop().await;
}

fn store_for(f: &Http) -> &Store {
    f.state.store.as_ref().unwrap()
}
