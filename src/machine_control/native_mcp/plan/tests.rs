use super::*;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::mcp_binding::TransportRequest;
use serde_json::json;
use tokio::sync::mpsc;

fn fixture() -> (
    Arc<MachineControl>,
    ConnectionToken,
    mpsc::UnboundedReceiver<MachineCommand>,
    crate::execution_protocol::Executor,
    Vec<ProcessProfile>,
) {
    let control = Arc::new(MachineControl::default());
    let (tx, rx) = mpsc::unbounded_channel();
    let connection = control.install(
        "hawk".into(),
        "epoch".into(),
        false,
        crate::machine_protocol::MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
        tx,
    );
    let cli = crate::execution_protocol::Executor {
        command: "/nix/store/fixture/bin/cowboy".into(),
        sha256: "a".repeat(64),
        version: "1.0.0".into(),
    };
    let profiles = ["first", "second"].map(|name| serde_json::from_value(json!({
        "schema":1,"service_id":format!("svc-{}", "a".repeat(32)),
        "session_id":"parent","runtime_machine":"ovh","execution_machine":"hawk",
        "workspace_id":"workspace","epoch":"b".repeat(32),
        "binding":{"name":name,"server":{"plugin_id":"fixture-mcp","generation_digest":format!("sha256:{}", "c".repeat(64))},"tools":["fixture_read"]}
    })).unwrap()).to_vec();
    (control, connection, rx, cli, profiles)
}

#[tokio::test]
async fn plan_rejects_mixed_scope_and_duplicate_bindings_before_sending() {
    for fault in [
        "empty",
        "capacity",
        "duplicate",
        "service",
        "session",
        "runtime",
        "execution",
        "workspace",
        "epoch",
        "cli",
    ] {
        let (control, connection, mut commands, mut cli, mut profiles) = fixture();
        match fault {
            "empty" => profiles.clear(),
            "capacity" => profiles = vec![profiles[0].clone(); 9],
            "duplicate" => profiles[1].binding = profiles[0].binding.clone(),
            "service" => profiles[1].service_id = format!("svc-{}", "e".repeat(32)),
            "session" => profiles[1].session_id = "other".into(),
            "runtime" => profiles[1].runtime_machine = "other".into(),
            "execution" => profiles[1].execution_machine = "other".into(),
            "workspace" => profiles[1].workspace_id = "other".into(),
            "epoch" => profiles[1].epoch = "e".repeat(32),
            "cli" => cli.command = "cowboy".into(),
            _ => unreachable!(),
        }
        assert!(
            control
                .prepare_native_mcp_plan(&connection, &cli, profiles)
                .await
                .is_err(),
            "{fault}"
        );
        assert!(commands.try_recv().is_err(), "{fault}");
    }
}

#[tokio::test]
async fn parallel_plan_is_ordered_and_releases_all_known_receipts_on_failure_or_cancel() {
    for mode in ["success", "invalid", "cancel"] {
        let (control, connection, mut commands, cli, profiles) = fixture();
        let task_control = Arc::clone(&control);
        let task_connection = connection.clone();
        let task = tokio::spawn(async move {
            task_control
                .prepare_native_mcp_plan(&task_connection, &cli, profiles)
                .await
        });
        // Both requests must arrive before either receives a response.
        let mut requests = Vec::new();
        for _ in 0..2 {
            let command = tokio::time::timeout(std::time::Duration::from_secs(5), commands.recv())
                .await
                .unwrap()
                .unwrap();
            let MachineCommand::McpTransport {
                request_id,
                request: TransportRequest::PrepareNative { profile },
            } = command
            else {
                panic!()
            };
            requests.push((request_id, profile));
        }
        requests.reverse();
        let mut known = Vec::new();
        for (index, (request_id, profile)) in requests.into_iter().enumerate() {
            if mode == "cancel" && index == 1 {
                break;
            }
            let handle = format!("native-{:032x}", index + 1);
            known.push(handle.clone());
            let invalid = mode == "invalid" && index == 1;
            control.record_remote(&connection, MachineEvent::McpTransportResponse {
                request_id,
                response: json!({"handle":handle,"path":if invalid { "relative".into() } else { format!("/private/{}.json", profile.binding.name) },
                    "descriptor":{"schema":1,"binding":profile.binding,"epoch":profile.epoch,"server_version":"1.0.0",
                        "tools":[{"name":"fixture_read","description":"Read fixture","inputSchema":{"type":"object"}}]}}),
            });
            if !invalid {
                let MachineCommand::McpTransport {
                    request_id,
                    request: TransportRequest::ConfirmNative { handle: confirmed },
                } = commands.recv().await.unwrap()
                else {
                    panic!()
                };
                assert_eq!(confirmed, handle);
                control.record_remote(
                    &connection,
                    MachineEvent::McpTransportResponse {
                        request_id,
                        response: json!({"confirmed":true}),
                    },
                );
            }
        }
        if mode == "cancel" {
            task.abort();
            assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        } else {
            let result = task.await.unwrap();
            if mode == "success" {
                let owner = result.unwrap();
                assert_eq!(
                    serde_json::to_value(owner.plan()).unwrap(),
                    json!({"schema":1,"command":"/nix/store/fixture/bin/cowboy","bindings":[
                        {"name":"first","descriptor":"/private/first.json"},{"name":"second","descriptor":"/private/second.json"}
                    ]})
                );
                assert!(commands.try_recv().is_err());
                drop(owner);
            } else {
                assert!(result.is_err());
            }
        }
        let mut released = Vec::new();
        for _ in 0..known.len() {
            let MachineCommand::McpTransport {
                request: TransportRequest::ReleaseNative { handle },
                ..
            } = commands.recv().await.unwrap()
            else {
                panic!()
            };
            released.push(handle);
        }
        released.sort();
        known.sort();
        assert_eq!(released, known);
        assert!(commands.try_recv().is_err());
    }
}
