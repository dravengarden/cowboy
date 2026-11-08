use super::*;
use crate::machine_control::{ConnectionToken, MachineControl};
use crate::machine_plugins::mcp_conformance::{self, artifact_processes, process_live};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

struct Link {
    control: Arc<MachineControl>,
    connection: ConnectionToken,
    scope: parking_lot::Mutex<Option<Arc<PluginExecutionScope>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Link {
    fn new(store: MachinePluginStore, service: &str) -> Self {
        let control = Arc::new(MachineControl::default());
        let (commands, mut incoming) = mpsc::unbounded_channel();
        let connection = control.install(
            "execution".into(),
            "fixture".into(),
            false,
            crate::machine_protocol::MCP_NATIVE_CONFIRMATION_PROTOCOL_VERSION,
            commands,
        );
        let scope = Arc::new(PluginExecutionScope::new(Some(service), "execution"));
        let link = McpLink::new(&scope);
        let store = Arc::new(store);
        let (events, mut outgoing) = mpsc::unbounded_channel();
        let recipient = Arc::clone(&control);
        let token = connection.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(command) = incoming.recv() => {
                        let command = serde_json::from_slice::<MachineCommand>(&serde_json::to_vec(&command).unwrap()).unwrap();
                        let MachineCommand::McpTransport { request_id, request } = command else {
                            panic!("unexpected command")
                        };
                        link.dispatch(request_id, request, Arc::clone(&store), events.clone());
                    }
                    Some(event) = outgoing.recv() => {
                        let event = serde_json::from_slice::<MachineEvent>(&serde_json::to_vec(&event).unwrap()).unwrap();
                        recipient.record_remote(&token, event);
                    }
                    else => return,
                }
            }
        });
        Self {
            control,
            connection,
            scope: parking_lot::Mutex::new(Some(scope)),
            task,
        }
    }
    fn disconnect(&self) {
        self.control.remove_if_current(&self.connection);
        self.scope.lock().take();
    }
    async fn request(&self, request: TransportRequest) -> Value {
        timeout(
            Duration::from_secs(5),
            self.control
                .mcp_request_on_connection(&self.connection, request),
        )
        .await
        .unwrap()
        .unwrap()
    }
    async fn message(&self, handle: &str, message: Value) -> Value {
        self.request(TransportRequest::Request {
            handle: handle.into(),
            message,
        })
        .await
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        self.control.remove_if_current(&self.connection);
        self.task.abort();
    }
}

#[tokio::test]
#[ignore = "requires COWBOY_MCP_CONFORMANCE_CASE with cancel_arguments"]
async fn signed_codeact_transport_close_stops_active_execution() {
    transport_cancellation(false).await;
}

#[tokio::test]
#[ignore = "requires COWBOY_MCP_CONFORMANCE_CASE with cancel_arguments"]
async fn signed_codeact_transport_disconnect_stops_active_execution() {
    transport_cancellation(true).await;
}

async fn transport_cancellation(disconnect: bool) {
    let f = mcp_conformance::install().await;
    let link = Link::new(f.store, &f.profile.service_id);
    let opened = link
        .request(TransportRequest::Open {
            profile: Box::new(f.profile),
        })
        .await;
    let handle = opened["handle"].as_str().expect("open failed");
    let initialized = link
        .message(
            handle,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        )
        .await;
    assert!(
        initialized["response"]["result"].is_object(),
        "{initialized}"
    );
    link.message(
        handle,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let listed = link
        .message(
            handle,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .await;
    let tools = listed["response"]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], f.case["tool"]);
    let call = |id, arguments| {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
        "params":{"name":f.case["tool"],"arguments":arguments}})
    };
    let result = link
        .message(handle, call(3, f.case["arguments"].clone()))
        .await;
    let content: Value = serde_json::from_str(
        result["response"]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(content["state"], "ok", "{content}");
    assert_eq!(content["value"], f.case["expected"]);
    let arguments = f
        .case
        .get("cancel_arguments")
        .expect("cancellation program required")
        .clone();
    let mut pending = Box::pin(link.control.mcp_request_on_connection(
        &link.connection,
        TransportRequest::Request {
            handle: handle.into(),
            message: call(4, arguments),
        },
    ));
    let observe = async {
        loop {
            let pids = artifact_processes(f.root.path());
            if pids.len() >= 4 {
                break pids;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let pids = tokio::select! {
        reply = &mut pending => panic!("execution ended early: {reply:?}"),
        pids = timeout(Duration::from_secs(3), observe) => pids.unwrap(),
    };
    if disconnect {
        link.disconnect();
        assert!(
            timeout(Duration::from_secs(2), &mut pending)
                .await
                .unwrap()
                .is_err()
        );
    } else {
        let closed = link
            .request(TransportRequest::Close {
                handle: handle.into(),
            })
            .await;
        assert_eq!(closed, json!({"closed":true}));
        let result = timeout(Duration::from_secs(2), &mut pending)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["error"], "mcp_unavailable");
        assert_eq!(
            link.message(handle, call(5, f.case["arguments"].clone()))
                .await["error"],
            "mcp_unavailable"
        );
    }
    timeout(Duration::from_secs(2), async {
        while pids.iter().any(|pid| process_live(*pid)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("processes survived transport close");
}

mod native;
