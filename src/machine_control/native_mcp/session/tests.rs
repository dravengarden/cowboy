use super::*;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::mcp_binding::TransportRequest;
use serde_json::json;
use tokio::sync::mpsc;

fn fixture() -> (
    Arc<MachineControl>,
    ConnectionToken,
    mpsc::UnboundedReceiver<MachineCommand>,
    BindingV1,
    Vec<ProcessProfile>,
) {
    let root = tempfile::tempdir().unwrap();
    let control = Arc::new(MachineControl::new(
        crate::service_identity::load_or_create(root.path()).unwrap(),
    ));
    let (tx, rx) = mpsc::unbounded_channel();
    let connection = control.install(
        "hawk".into(),
        "epoch".into(),
        false,
        crate::machine_protocol::MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
        tx,
    );
    let binding = crate::execution_environment::fixture().decode().unwrap();
    let profile = serde_json::from_value(json!({"schema":1,"service_id":control.service.as_str(),"session_id":"parent","runtime_machine":"ovh","execution_machine":"hawk","workspace_id":"cowboy","epoch":"b".repeat(32),"binding":{"name":"suger","server":{"plugin_id":"suger","generation_digest":format!("sha256:{}", "c".repeat(64))},"tools":["fixture_read"]}})).unwrap();
    (control, connection, rx, binding, vec![profile])
}

fn ready(binding: &BindingV1) -> Response {
    Response::Call {
        response: KeeperResponse::Ready {
            scope: Scope::from_binding(binding),
            initialization: json!({"cowboyAgentCli":{"command":"/nix/store/fixture/bin/cowboy","sha256":"a".repeat(64),"version":"1.0.0"}}),
            event_cursor: 0,
        },
    }
}

#[test]
fn observed_identity_requires_exact_incarnation_and_valid_pin() {
    let binding = crate::execution_environment::fixture().decode().unwrap();
    assert!(observed_cli(ready(&binding), &binding).is_ok());
    for fault in ["scope", "missing", "null", "relative", "digest"] {
        let mut response = ready(&binding);
        let Response::Call {
            response:
                KeeperResponse::Ready {
                    scope,
                    initialization,
                    ..
                },
        } = &mut response
        else {
            panic!()
        };
        match fault {
            "scope" => scope.incarnation = "replacement".into(),
            "missing" => *initialization = json!({}),
            "null" => initialization[AGENT_CLI_INITIALIZATION_KEY] = json!(null),
            "relative" => initialization[AGENT_CLI_INITIALIZATION_KEY]["command"] = json!("cowboy"),
            "digest" => initialization[AGENT_CLI_INITIALIZATION_KEY]["sha256"] = json!("unknown"),
            _ => unreachable!(),
        }
        assert!(observed_cli(response, &binding).is_err(), "{fault}");
    }
    assert!(observed_cli(Response::Closed, &binding).is_err());
}

#[tokio::test]
async fn mismatched_policy_or_stale_connection_sends_nothing() {
    for fault in ["service", "runtime", "workspace", "managed", "stale"] {
        let (control, connection, mut commands, mut binding, mut profiles) = fixture();
        let mut replacement = None;
        match fault {
            "service" => profiles[0].service_id = format!("svc-{}", "d".repeat(32)),
            "runtime" => profiles[0].runtime_machine = "other".into(),
            "workspace" => profiles[0].workspace_id = "other".into(),
            "managed" => {
                binding.managed = Some(crate::execution_environment::ManagedBindingV1 {
                    parent_session_id: "parent".into(),
                    profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
                });
            }
            "stale" => {
                let (tx, rx) = mpsc::unbounded_channel();
                control.install(
                    "hawk".into(),
                    "epoch".into(),
                    false,
                    crate::machine_protocol::MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
                    tx,
                );
                replacement = Some(rx);
            }
            _ => unreachable!(),
        }
        assert!(
            control
                .prepare_session_native_mcp(&connection, binding, profiles)
                .await
                .is_err()
        );
        assert!(commands.try_recv().is_err());
        if let Some(mut rx) = replacement {
            assert!(rx.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn keeper_identity_precedes_descriptor_and_owner_retains_receipt() {
    let (control, connection, mut commands, binding, profiles) = fixture();
    let caller = Arc::clone(&control);
    let target = connection.clone();
    let expected = binding.clone();
    let task = tokio::spawn(async move {
        caller
            .prepare_session_native_mcp(&target, binding, profiles)
            .await
    });
    let MachineCommand::Execution {
        request_id,
        request,
    } = commands.recv().await.unwrap()
    else {
        panic!()
    };
    assert!(matches!(
        request.action,
        Action::Call {
            command: Command::Describe,
            ..
        }
    ));
    assert!(commands.try_recv().is_err());
    control.record_remote(
        &connection,
        MachineEvent::ExecutionResponse {
            request_id,
            response: Box::new(ready(&expected)),
        },
    );
    let MachineCommand::McpTransport {
        request_id,
        request: TransportRequest::PrepareNative { profile },
    } = commands.recv().await.unwrap()
    else {
        panic!()
    };
    let handle = format!("native-{}", "e".repeat(32));
    control.record_remote(&connection, MachineEvent::McpTransportResponse { request_id, response: json!({"handle":handle,"path":"/private/suger.json","descriptor":{"schema":1,"binding":profile.binding,"epoch":profile.epoch,"server_version":"1.0.0","tools":[{"name":"fixture_read","description":"Read","inputSchema":{"type":"object"}}]}}) });
    let MachineCommand::McpTransport {
        request_id,
        request: TransportRequest::ConfirmNative { .. },
    } = commands.recv().await.unwrap()
    else {
        panic!()
    };
    control.record_remote(
        &connection,
        MachineEvent::McpTransportResponse {
            request_id,
            response: json!({"confirmed":true}),
        },
    );
    let owner = task.await.unwrap().unwrap();
    assert_eq!(owner.binding(), &expected);
    assert!(owner.is_current(&control, &expected));
    let mut changed = expected.clone();
    changed.environment.incarnation = "replacement".into();
    assert!(!owner.is_current(&control, &changed));
    assert_eq!(owner.plan().command, owner.cli().command);
    assert!(commands.try_recv().is_err());
    drop(owner);
    assert!(
        matches!(commands.try_recv().unwrap(), MachineCommand::McpTransport { request: TransportRequest::ReleaseNative { handle: released }, .. } if released == handle)
    );
}

#[tokio::test]
async fn lost_keeper_observation_never_prepares_on_replacement() {
    for cancel in [false, true] {
        let (control, connection, mut commands, binding, profiles) = fixture();
        let expected = binding.clone();
        let caller = Arc::clone(&control);
        let target = connection.clone();
        let task = tokio::spawn(async move {
            caller
                .prepare_session_native_mcp(&target, binding, profiles)
                .await
        });
        let MachineCommand::Execution { request_id, .. } = commands.recv().await.unwrap() else {
            panic!()
        };
        let (tx, mut replacement) = mpsc::unbounded_channel();
        if cancel {
            task.abort();
        } else {
            control.install(
                "hawk".into(),
                "epoch".into(),
                false,
                crate::machine_protocol::MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
                tx,
            );
        }
        control.record_remote(
            &connection,
            MachineEvent::ExecutionResponse {
                request_id,
                response: Box::new(ready(&expected)),
            },
        );
        if cancel {
            assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        } else {
            assert!(task.await.unwrap().is_err());
        }
        assert!(commands.try_recv().is_err());
        assert!(replacement.try_recv().is_err());
    }
}
