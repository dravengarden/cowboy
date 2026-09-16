use super::*;
use serde_json::json;

fn request() -> Request {
    serde_json::from_value(
        json!({"service_id":format!("svc-{}", "a".repeat(32)),"machine_id":"hawk",
        "action":{"kind":"query","operation":{"instance":"b".repeat(32),"id":"0000000000000001"}}}),
    )
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn scheduling_time_is_part_of_the_nonrenewable_budget() {
    let request = request();
    let scope = PluginExecutionScope::new(Some(&request.service_id), "hawk");
    let invocation = scope.code_buffer_sync(request).unwrap();
    invocation.remaining().unwrap();
    tokio::time::advance(COMMAND_BUDGET).await;
    assert!(invocation.remaining().is_err());
}

#[test]
fn a_replacement_connection_cannot_inherit_original_operations() {
    let request = request();
    let scope = PluginExecutionScope::new(Some(&request.service_id), "hawk");
    let invocation = scope.code_buffer_sync(request.clone()).unwrap();
    let owner = invocation.owner().unwrap();
    drop(scope);
    assert!(invocation.remaining().is_err());
    let next = PluginExecutionScope::new(Some(&request.service_id), "hawk");
    let replacement = next.code_buffer_sync(request).unwrap();
    assert!(replacement.check_owner(&owner).is_err());
}

#[test]
fn a_site_declaration_does_not_manufacture_connection_authority() {
    let request = request();
    for scope in [
        PluginExecutionScope::new(None, "hawk"),
        PluginExecutionScope::new(Some(&request.service_id), "other"),
        PluginExecutionScope::new(Some("other"), "hawk"),
    ] {
        assert!(scope.code_buffer_sync(request.clone()).is_err());
    }
}
