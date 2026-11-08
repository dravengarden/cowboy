use super::*;
use crate::machine_protocol::MachineEvent;
use serde_json::json;
use tokio::sync::mpsc;

fn connect(control: &MachineControl) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        control.install("hawk".into(), "epoch".into(), false, 43, tx),
        rx,
    )
}

fn profile() -> ProcessProfile {
    serde_json::from_value(json!({
        "schema":1,"service_id":format!("svc-{}", "a".repeat(32)),
        "session_id":"parent","runtime_machine":"ovh","execution_machine":"hawk",
        "workspace_id":"workspace","epoch":"b".repeat(32),
        "binding":{"name":"suger","server":{"plugin_id":"fixture-mcp","generation_digest":format!("sha256:{}", "c".repeat(64))},"tools":["fixture_read"]}
    })).unwrap()
}

fn response(profile: &ProcessProfile) -> serde_json::Value {
    json!({
        "handle":format!("native-{}", "d".repeat(32)), "path":"/private/binding.json",
        "descriptor":{"schema":1,"binding":profile.binding,"epoch":profile.epoch,"server_version":"1.0.0",
            "tools":[{"name":"fixture_read","description":"Read fixture","inputSchema":{"type":"object"}}]}
    })
}

#[tokio::test]
async fn native_preparation_owns_cleanup_and_refuses_mismatched_metadata() {
    for fault in ["none", "epoch", "path", "extra", "binding", "confirm"] {
        let control = Arc::new(MachineControl::default());
        let (connection, mut commands) = connect(&control);
        let profile = profile();
        let mut value = response(&profile);
        match fault {
            "epoch" => value["descriptor"]["epoch"] = json!("e".repeat(32)),
            "path" => value["path"] = json!("relative"),
            "extra" => value["command"] = json!("/unexpected"),
            "binding" => value["descriptor"]["binding"]["name"] = json!("other"),
            _ => {}
        }
        let (result, ()) = tokio::join!(
            control.prepare_native_mcp(&connection, profile.clone()),
            async {
                let MachineCommand::McpTransport {
                    request_id,
                    request,
                } = commands.recv().await.unwrap()
                else {
                    panic!()
                };
                assert_eq!(
                    request,
                    TransportRequest::PrepareNative {
                        profile: Box::new(profile.clone())
                    }
                );
                control.record_remote(
                    &connection,
                    MachineEvent::McpTransportResponse {
                        request_id,
                        response: value,
                    },
                );
                if fault == "none" || fault == "confirm" {
                    confirm(&control, &connection, &mut commands, fault == "none").await;
                }
            }
        );
        if fault == "none" {
            let prepared = result.unwrap();
            assert_eq!(prepared.path(), Path::new("/private/binding.json"));
            assert_eq!(prepared.descriptor.binding, profile.binding);
            assert!(commands.try_recv().is_err());
            drop(prepared);
        } else {
            assert!(result.is_err(), "{fault}");
        }
        assert!(
            matches!(commands.try_recv().unwrap(), MachineCommand::McpTransport {
            request:TransportRequest::ReleaseNative {handle}, ..
        } if handle == format!("native-{}", "d".repeat(32)))
        );
        assert!(commands.try_recv().is_err());
    }
}

#[tokio::test]
async fn native_receipt_cleanup_never_targets_a_replacement_connection() {
    let control = Arc::new(MachineControl::default());
    let (connection, mut commands) = connect(&control);
    let profile = profile();
    let (result, ()) = tokio::join!(
        control.prepare_native_mcp(&connection, profile.clone()),
        async {
            let MachineCommand::McpTransport { request_id, .. } = commands.recv().await.unwrap()
            else {
                panic!()
            };
            control.record_remote(
                &connection,
                MachineEvent::McpTransportResponse {
                    request_id,
                    response: response(&profile),
                },
            );
            confirm(&control, &connection, &mut commands, true).await;
        }
    );
    let prepared = result.unwrap();
    let (_, mut replacement) = connect(&control);
    drop(prepared);
    assert!(replacement.try_recv().is_err());
}

async fn confirm(
    control: &MachineControl,
    connection: &ConnectionToken,
    commands: &mut mpsc::UnboundedReceiver<MachineCommand>,
    accepted: bool,
) {
    let MachineCommand::McpTransport {
        request_id,
        request: TransportRequest::ConfirmNative { handle },
    } = commands.recv().await.unwrap()
    else {
        panic!()
    };
    assert_eq!(handle, format!("native-{}", "d".repeat(32)));
    control.record_remote(
        connection,
        MachineEvent::McpTransportResponse {
            request_id,
            response: json!({"confirmed":accepted}),
        },
    );
}

#[tokio::test]
async fn native_preparation_refuses_an_unconfirmed_protocol_without_sending() {
    let control = Arc::new(MachineControl::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let connection = control.install("hawk".into(), "epoch".into(), false, 42, tx);
    assert!(
        control
            .prepare_native_mcp(&connection, profile())
            .await
            .is_err()
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn cancellation_while_confirming_releases_the_known_receipt() {
    let control = Arc::new(MachineControl::default());
    let (connection, mut commands) = connect(&control);
    let task_control = Arc::clone(&control);
    let task_connection = connection.clone();
    let task = tokio::spawn(async move {
        task_control
            .prepare_native_mcp(&task_connection, profile())
            .await
    });
    let MachineCommand::McpTransport {
        request_id,
        request: TransportRequest::PrepareNative { .. },
    } = commands.recv().await.unwrap()
    else {
        panic!()
    };
    control.record_remote(
        &connection,
        MachineEvent::McpTransportResponse {
            request_id,
            response: response(&profile()),
        },
    );
    assert!(matches!(
        commands.recv().await.unwrap(),
        MachineCommand::McpTransport {
            request: TransportRequest::ConfirmNative { .. },
            ..
        }
    ));
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert!(
        matches!(commands.recv().await.unwrap(), MachineCommand::McpTransport {
        request: TransportRequest::ReleaseNative {handle}, ..
    } if handle == format!("native-{}", "d".repeat(32)))
    );
    assert!(commands.try_recv().is_err());
}
