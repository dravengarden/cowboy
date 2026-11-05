#[test]
fn old_session_metadata_defaults_to_the_local_machine() {
    let meta: crate::core::SessionMeta = serde_json::from_value(serde_json::json!({
        "id": "sess-old",
        "provider": "codex",
        "cwd": "/tmp",
        "title": "old",
        "status": "running"
    }))
    .expect("old metadata remains readable");
    assert_eq!(meta.machine_id, "local");
}
