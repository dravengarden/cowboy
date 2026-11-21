use super::*;
use tokio::io::{AsyncWriteExt as _, BufReader};

fn fixture() -> NativeDescriptor {
    serde_json::from_value(json!({"schema":1,"binding":{"name":"suger","server":{"plugin_id":"suger","generation_digest":format!("sha256:{}", "a".repeat(64))},"tools":["suger_read"]},"epoch":"a".repeat(32),"server_version":"1.0.0","tools":[{"name":"suger_read","description":"Read fixture","inputSchema":{"type":"object","properties":{}}}]})).unwrap()
}

fn mock(block: bool) -> (Gateway, Arc<parking_lot::Mutex<Vec<RelayAction>>>) {
    let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let gateway: Gateway = Arc::new(move |action| {
        let calls = Arc::clone(&observed);
        Box::pin(async move {
            calls.lock().push(action.clone());
            Ok(match action {
                RelayAction::Open => json!({"handle":format!("relay-{}", "b".repeat(32))}),
                RelayAction::Close { .. } => json!({"closed":true}),
                RelayAction::Request { message, .. } => {
                    if message["method"] == "tools/call" && block {
                        std::future::pending::<()>().await;
                    }
                    if message["method"] == "notifications/initialized" {
                        json!({"response":null})
                    } else if message["method"] == "initialize" {
                        json!({"response":{"jsonrpc":"2.0","id":message["id"],"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1.0.0"}}}})
                    } else {
                        json!({"response":{"jsonrpc":"2.0","id":message["id"],"result":{"content":[{"type":"text","text":"ok"}]}}})
                    }
                }
            })
        })
    });
    (gateway, calls)
}

async fn send(output: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, message: Value) {
    output
        .write_all(format!("{message}\n").as_bytes())
        .await
        .unwrap();
}

async fn read(input: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>) -> Value {
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        input.read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
async fn discovery_is_offline_and_first_call_initializes_remote_then_eof_closes() {
    let (gateway, calls) = mock(false);
    let (server, client) = tokio::io::duplex(32768);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(serve_io(input, output, fixture(), gateway, false));
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    send(&mut output, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).await;
    assert_eq!(
        read(&mut input).await["result"]["protocolVersion"],
        "2025-06-18"
    );
    send(
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    assert_eq!(
        read(&mut input).await["result"]["tools"][0]["name"],
        "suger_read"
    );
    assert!(calls.lock().is_empty());
    send(
        &mut output,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"unsigned"}}),
    )
    .await;
    assert!(read(&mut input).await.get("error").is_some());
    assert!(calls.lock().is_empty());
    send(&mut output, json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"suger_read","arguments":{}}})).await;
    assert_eq!(read(&mut input).await["result"]["content"][0]["text"], "ok");
    assert_eq!(calls.lock().len(), 4);
    output.shutdown().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        calls.lock().last(),
        Some(RelayAction::Close { .. })
    ));
}

#[tokio::test]
async fn cancellation_and_eof_interrupt_an_active_remote_read() {
    for eof in [false, true] {
        let (gateway, calls) = mock(true);
        let (server, client) = tokio::io::duplex(32768);
        let (input, output) = tokio::io::split(server);
        let task = tokio::spawn(serve_io(input, output, fixture(), gateway, false));
        let (input, mut output) = tokio::io::split(client);
        let mut input = BufReader::new(input);
        send(
            &mut output,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        )
        .await;
        read(&mut input).await;
        send(
            &mut output,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"suger_read"}}),
        )
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while calls.lock().len() < 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if eof {
            output.shutdown().await.unwrap();
        } else {
            send(&mut output, json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}})).await;
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            calls
                .lock()
                .iter()
                .filter(|action| matches!(action, RelayAction::Close { .. }))
                .count(),
            1
        );
    }
}

#[test]
fn descriptor_must_be_private_and_cannot_use_a_symlink() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("binding.json");
    std::fs::write(&path, serde_json::to_vec(&fixture()).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    descriptor(&path).unwrap();
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(descriptor(&link).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(descriptor(&path).is_err());
}

#[tokio::test]
async fn eof_interrupts_blocked_discovery_output() {
    let (gateway, calls) = mock(false);
    let (server, client) = tokio::io::duplex(1);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(serve_io(input, output, fixture(), gateway, false));
    let (_input, mut output) = tokio::io::split(client);
    send(
        &mut output,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
    )
    .await;
    output.shutdown().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(calls.lock().is_empty());
}

#[tokio::test]
async fn oversized_frame_never_executes_its_valid_suffix() {
    let (gateway, calls) = mock(false);
    let (server, client) = tokio::io::duplex(4096);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(serve_io(input, output, fixture(), gateway, false));
    let (_input, mut output) = tokio::io::split(client);
    let bytes = format!(
        "{}\n{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"suger_read\"}}}}\n",
        " ".repeat(LIMIT + 1)
    );
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        output.write_all(bytes.as_bytes()),
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(calls.lock().is_empty());
}
