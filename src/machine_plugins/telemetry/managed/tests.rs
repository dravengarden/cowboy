use super::*;

#[tokio::test]
async fn managed_export_lifecycle_queue_consumes_original_budget_and_never_renews() {
    let root = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
    let owner = PluginExecutionScope::new(Some("service-test"), "machine-test");
    let mut request = crate::machine_protocol::telemetry_export::fixture();
    request.expires_at_ms = chrono::Utc::now().timestamp_millis() + 40;
    let invocation = owner.telemetry_export(&request).unwrap();
    let _guard = store.lifecycle.lock().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), invocation.lock(&store))
            .await
            .unwrap()
            .is_err()
    );
    assert!(invocation.check().is_err());
    let _replacement = PluginExecutionScope::new(Some("service-test"), "machine-test");
    assert!(
        invocation.check().is_err(),
        "a later owner cannot reset an old monotonic deadline"
    );
}
