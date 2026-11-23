use super::super::resolution::surface::tests::Fixture;
use super::*;
use axum::http::Method;
use serde_json::{Value, json};

const PLAN: &str = "/api/telemetry/binding/plan";
const CONFIRM: &str = "/api/telemetry/binding/confirm";
const CHOICES: &str = "/api/telemetry/binding/choices";

#[tokio::test]
async fn closed_gate_and_closed_requests_do_not_create_a_namespace() {
    let f = Fixture::new(false).await;
    for body in [
        json!({}),
        json!({"action":"select"}),
        json!({"action":"recover"}),
        json!({"action":"revoke","actor":"operator"}),
        json!({"action":"restore","operation_id":"old","force":true}),
    ] {
        let (status, value) = f.request(Method::POST, PLAN, Some(body)).await;
        assert!(status.is_client_error());
        assert_eq!(value["error"], "invalid_request");
    }
    let (status, value) = f
        .request(
            Method::POST,
            CONFIRM,
            Some(json!({"plan_id":"unknown-plan-id-123", "action":"select"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value["error"], "binding_admission_closed");
    let (_, choices) = f.request(Method::GET, CHOICES, None).await;
    assert_eq!(choices["targets"], json!([]));
    assert_eq!(choices["revoke_available"], false);
    assert!(f.state.ledger().await.unwrap().is_none());
    assert!(f.state.legacy_fence.allows_legacy());
    f.stop().await;
}

#[test]
fn public_contract_has_no_serialized_authority() {
    let mut intent = crate::telemetry_binding::fixture("binding-surface-fixture");
    intent.schema = 2;
    intent.expires_at_ms = 2_300_000_000_000;
    let plan = PlanView::new(&intent, false, None).unwrap();
    let receipt = ReceiptView::new(&Operation {
        intent,
        progress: Progress::Aborted,
    })
    .unwrap();
    let actual = json!({"plan":plan,"receipt":receipt});
    // This fixture is also decoded by the independent Web implementation.
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/telemetry-binding-surface.json"
    ))
    .unwrap();
    assert_eq!(expected, actual);
}
