use super::*;
use serde_json::json;

fn request() -> Request {
    serde_json::from_value(json!({"service_id":format!("svc-{}", "a".repeat(32)), "machine_id":"hawk",
        "action":{"kind":"query","navigation":{"instance":"b".repeat(32),"id":"navigation:0000000000000001"}}})).unwrap()
}

#[tokio::test(start_paused = true)]
async fn navigation_admission_budget_includes_scheduling_time() {
    let request = request();
    let scope = PluginExecutionScope::new(Some(&request.service_id), "hawk");
    let invocation = scope.code_navigation(request).unwrap();
    invocation.remaining().unwrap();
    tokio::time::advance(COMMAND_BUDGET).await;
    assert!(invocation.remaining().is_err());
}

#[test]
fn navigation_requires_the_actual_site_and_original_connection() {
    let request = request();
    for scope in [
        PluginExecutionScope::new(None, "hawk"),
        PluginExecutionScope::new(Some(&request.service_id), "other"),
        PluginExecutionScope::new(Some("other"), "hawk"),
    ] {
        assert!(scope.code_navigation(request.clone()).is_err());
    }
    let scope = PluginExecutionScope::new(Some(&request.service_id), "hawk");
    let invocation = scope.code_navigation(request.clone()).unwrap();
    let owner = invocation.owner().unwrap();
    drop(scope);
    assert!(invocation.remaining().is_err());
    let next = PluginExecutionScope::new(Some(&request.service_id), "hawk");
    assert!(
        next.code_navigation(request)
            .unwrap()
            .check_owner(&owner)
            .is_err()
    );
}
