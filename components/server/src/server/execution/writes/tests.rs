use super::*;
use serde_json::json;

fn command(value: serde_json::Value) -> Command {
    serde_json::from_value(value).unwrap()
}

fn invoke(operation: &str, path: &str, text: &str) -> Command {
    let data = base64::engine::general_purpose::STANDARD.encode(text);
    command(
        json!({"kind":"invoke","wait_ms":0,"invocation":{"operation_id":operation,
        "method":"fs/writeFile","params":{"path":path,"dataBase64":data}}}),
    )
}

fn reply(outcome: serde_json::Value) -> Response {
    serde_json::from_value(json!({"kind":"call","response":{"kind":"operation","outcome":outcome}}))
        .unwrap()
}

fn sha(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[test]
fn completed_writes_record_the_mirror_text_identity() {
    let session = "writes-completed";
    observe(
        session,
        &invoke("op-1", "/w/a.go", "package a\r\n"),
        &reply(json!({"state":"completed","reply":{}})),
    );
    let first = latest(session, "/w/a.go").unwrap();
    assert_eq!(first.sha256, sha("package a\n"));
    assert_eq!(first.utf8_bytes, 10);
    observe(
        session,
        &invoke("op-2", "/w/a.go", "package b\n"),
        &reply(json!({"state":"completed","reply":{}})),
    );
    let second = latest(session, "/w/a.go").unwrap();
    assert_eq!(second.sha256, sha("package b\n"));
    assert!(second.sequence > first.sequence);
    assert_eq!(latest("another-session", "/w/a.go"), None);
}

#[test]
fn failed_unknown_and_late_completions_are_distinguished() {
    let session = "writes-late";
    observe(
        session,
        &invoke("op-err", "/w/err.go", "x"),
        &reply(json!({"state":"completed","reply":{"error":{"message":"denied"}}})),
    );
    assert_eq!(latest(session, "/w/err.go"), None);
    observe(
        session,
        &invoke("op-unknown", "/w/unknown.go", "x"),
        &reply(json!({"state":"unknown"})),
    );
    assert_eq!(latest(session, "/w/unknown.go"), None);
    observe(
        session,
        &invoke("op-late", "/w/late.go", "late\n"),
        &reply(json!({"state":"pending"})),
    );
    assert_eq!(latest(session, "/w/late.go"), None);
    let observe_late = command(json!({"kind":"observe","operation_id":"op-late","wait_ms":0}));
    observe(session, &observe_late, &reply(json!({"state":"pending"})));
    assert_eq!(latest(session, "/w/late.go"), None);
    observe(
        session,
        &observe_late,
        &reply(json!({"state":"completed","reply":{}})),
    );
    assert_eq!(latest(session, "/w/late.go").unwrap().sha256, sha("late\n"));
}

#[test]
fn other_methods_relative_paths_and_binary_bytes_are_not_writes() {
    let session = "writes-ignored";
    let done = reply(json!({"state":"completed","reply":{}}));
    observe(
        session,
        &command(
            json!({"kind":"invoke","wait_ms":0,"invocation":{"operation_id":"r",
            "method":"fs/readFile","params":{"path":"/w/a.go"}}}),
        ),
        &done,
    );
    observe(session, &invoke("rel", "a.go", "x"), &done);
    let binary = base64::engine::general_purpose::STANDARD.encode([0xff, 0xfe, 0x00]);
    observe(
        session,
        &command(
            json!({"kind":"invoke","wait_ms":0,"invocation":{"operation_id":"bin",
            "method":"fs/writeFile","params":{"path":"/w/bin","dataBase64":binary}}}),
        ),
        &done,
    );
    for path in ["/w/a.go", "a.go", "/w/bin"] {
        assert_eq!(latest(session, path), None, "{path}");
    }
}

#[test]
fn native_file_urls_name_the_same_target_path() {
    let session = "writes-urls";
    let done = reply(json!({"state":"completed","reply":{}}));
    observe(
        session,
        &invoke("url", "file:///w/my%20dir/a.go", "x\n"),
        &done,
    );
    assert_eq!(
        latest(session, "/w/my dir/a.go").unwrap().sha256,
        sha("x\n")
    );
    for (operation, location) in [
        ("remote", "file://host/w/b.go"),
        ("http", "https://x/w/c.go"),
    ] {
        observe(session, &invoke(operation, location, "x"), &done);
    }
    assert_eq!(latest(session, "/w/b.go"), None);
    assert_eq!(latest(session, "/w/c.go"), None);
}
