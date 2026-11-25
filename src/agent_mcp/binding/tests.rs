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

#[tokio::test]
async fn pre_dispatch_init_failure_closes_handle_and_recovers_on_next_call() {
    let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let init_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let attempts = Arc::clone(&init_attempts);
    let gateway: Gateway = Arc::new(move |action| {
        let calls = Arc::clone(&observed);
        let attempts = Arc::clone(&attempts);
        Box::pin(async move {
            calls.lock().push(action.clone());
            match action {
                RelayAction::Open => {
                    let n = attempts.load(std::sync::atomic::Ordering::SeqCst);
                    let suffix = if n == 0 { "b" } else { "c" };
                    Ok(json!({"handle": format!("relay-{}", suffix.repeat(32))}))
                }
                RelayAction::Close { .. } => Ok(json!({"closed": true})),
                RelayAction::Request { message, .. } => {
                    if message["method"] == "initialize" {
                        if attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                            return Err(GatewayError::rejected(
                                Some("mcp_unavailable".into()),
                                "MCP binding unavailable",
                            ));
                        }
                        Ok(
                            json!({"response":{"jsonrpc":"2.0","id":message["id"],"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1.0.0"}}}}),
                        )
                    } else if message["method"] == "notifications/initialized" {
                        Ok(json!({"response": null}))
                    } else {
                        Ok(
                            json!({"response":{"jsonrpc":"2.0","id":message["id"],"result":{"content":[{"type":"text","text":"recovered"}]}}}),
                        )
                    }
                }
            }
        })
    });
    let (server, client) = tokio::io::duplex(32768);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(serve_io(input, output, fixture(), gateway, false));
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    send(&mut output, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).await;
    read(&mut input).await;

    send(&mut output, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"suger_read","arguments":{}}})).await;
    let first = read(&mut input).await;
    assert_eq!(first["error"]["code"], -32000);
    assert_eq!(first["error"]["message"], "MCP binding unavailable");
    assert_eq!(
        calls
            .lock()
            .iter()
            .filter(|a| matches!(a, RelayAction::Close { .. }))
            .count(),
        1,
        "failed initialize must close its opened handle to avoid leaking a Machine slot"
    );

    // Bridge stays alive and reopens cleanly on the next tool call.
    send(&mut output, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"suger_read","arguments":{}}})).await;
    let second = read(&mut input).await;
    assert_eq!(second["result"]["content"][0]["text"], "recovered");

    output.shutdown().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn post_dispatch_failure_closes_handle_reports_uncertain_outcome_and_reopens_next_call() {
    let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let tool_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let attempts = Arc::clone(&tool_attempts);
    let gateway: Gateway = Arc::new(move |action| {
        let calls = Arc::clone(&observed);
        let attempts = Arc::clone(&attempts);
        Box::pin(async move {
            calls.lock().push(action.clone());
            match action {
                RelayAction::Open => {
                    let n = attempts.load(std::sync::atomic::Ordering::SeqCst);
                    let suffix = if n == 0 { "b" } else { "c" };
                    Ok(json!({"handle": format!("relay-{}", suffix.repeat(32))}))
                }
                RelayAction::Close { .. } => Ok(json!({"closed": true})),
                RelayAction::Request { message, .. } => {
                    if message["method"] == "initialize" {
                        Ok(
                            json!({"response":{"jsonrpc":"2.0","id":message["id"],"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1.0.0"}}}}),
                        )
                    } else if message["method"] == "notifications/initialized" {
                        Ok(json!({"response": null}))
                    } else if attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        // Connection dropped after tools/call was sent (e.g. Controller restart or Machine reconnect).
                        Err(GatewayError::rejected(
                            Some("mcp_authority_ended".into()),
                            "MCP binding unavailable",
                        ))
                    } else {
                        Ok(
                            json!({"response":{"jsonrpc":"2.0","id":message["id"],"result":{"content":[{"type":"text","text":"after-reconnect"}]}}}),
                        )
                    }
                }
            }
        })
    });
    let (server, client) = tokio::io::duplex(32768);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(serve_io(input, output, fixture(), gateway, false));
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    send(&mut output, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).await;
    read(&mut input).await;

    send(&mut output, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"suger_read","arguments":{}}})).await;
    let first = read(&mut input).await;
    assert_eq!(first["error"]["code"], -32000);
    assert!(
        first["error"]["message"]
            .as_str()
            .unwrap()
            .contains("outcome unknown after dispatch"),
        "{first}"
    );
    // Must not auto-replay the dispatched call.
    assert_eq!(tool_attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        calls
            .lock()
            .iter()
            .filter(|a| matches!(a, RelayAction::Close { .. }))
            .count(),
        1
    );

    // Subsequent call opens a fresh handle and succeeds.
    send(&mut output, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"suger_read","arguments":{}}})).await;
    let second = read(&mut input).await;
    assert_eq!(second["result"]["content"][0]["text"], "after-reconnect");

    output.shutdown().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn definitive_authority_end_stops_reopening_without_further_gateway_calls() {
    let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let gateway: Gateway = Arc::new(move |action| {
        let calls = Arc::clone(&observed);
        Box::pin(async move {
            calls.lock().push(action.clone());
            Err(GatewayError::rejected(
                Some("epoch_changed".into()),
                "MCP binding unavailable",
            ))
        })
    });
    let (server, client) = tokio::io::duplex(32768);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(serve_io(input, output, fixture(), gateway, false));
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    send(&mut output, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).await;
    read(&mut input).await;

    send(&mut output, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"suger_read","arguments":{}}})).await;
    let first = read(&mut input).await;
    assert_eq!(first["error"]["message"], "MCP binding authority ended");
    assert_eq!(calls.lock().len(), 1);

    // Second call refuses deterministically without touching the gateway again.
    send(&mut output, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"suger_read","arguments":{}}})).await;
    let second = read(&mut input).await;
    assert_eq!(second["error"]["message"], "MCP binding authority ended");
    assert_eq!(calls.lock().len(), 1);

    output.shutdown().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn ordinary_tool_error_keeps_remote_handle_open_for_subsequent_calls() {
    let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let gateway: Gateway = Arc::new(move |action| {
        let calls = Arc::clone(&observed);
        Box::pin(async move {
            calls.lock().push(action.clone());
            Ok(match action {
                RelayAction::Open => json!({"handle": format!("relay-{}", "b".repeat(32))}),
                RelayAction::Close { .. } => json!({"closed": true}),
                RelayAction::Request { message, .. } => {
                    if message["method"] == "initialize" {
                        json!({"response":{"jsonrpc":"2.0","id":message["id"],"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1.0.0"}}}})
                    } else if message["method"] == "notifications/initialized" {
                        json!({"response": null})
                    } else if message["id"] == 2 {
                        json!({"response":{"jsonrpc":"2.0","id":2,"result":{"isError":true,"content":[{"type":"text","text":"bad args"}]}}})
                    } else {
                        json!({"response":{"jsonrpc":"2.0","id":message["id"],"result":{"content":[{"type":"text","text":"ok"}]}}})
                    }
                }
            })
        })
    });
    let (server, client) = tokio::io::duplex(32768);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(serve_io(input, output, fixture(), gateway, false));
    let (input, mut output) = tokio::io::split(client);
    let mut input = BufReader::new(input);
    send(&mut output, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).await;
    read(&mut input).await;

    send(&mut output, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"suger_read","arguments":{}}})).await;
    assert_eq!(read(&mut input).await["result"]["isError"], true);
    send(&mut output, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"suger_read","arguments":{}}})).await;
    assert_eq!(read(&mut input).await["result"]["content"][0]["text"], "ok");

    // Only one Open and zero Close calls while the client is still connected.
    assert_eq!(
        calls
            .lock()
            .iter()
            .filter(|a| matches!(a, RelayAction::Open))
            .count(),
        1
    );
    assert_eq!(
        calls
            .lock()
            .iter()
            .filter(|a| matches!(a, RelayAction::Close { .. }))
            .count(),
        0
    );

    output.shutdown().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
