use super::*;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, tungstenite::client::IntoClientRequest as _,
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(super) async fn connect(endpoint: &Endpoint) -> Socket {
    let descriptor: Value =
        serde_json::from_slice(&std::fs::read(endpoint.descriptor()).unwrap()).unwrap();
    let mut request = descriptor["endpoint"]
        .as_str()
        .unwrap()
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", descriptor["bearer_token"].as_str().unwrap())
            .parse()
            .unwrap(),
    );
    tokio_tungstenite::connect_async(request).await.unwrap().0
}

pub(super) async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .unwrap();
}

pub(super) async fn receive(socket: &mut Socket) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(60), socket.next())
        .await
        .expect("endpoint timed out")
        .expect("endpoint closed")
        .expect("endpoint reset");
    assert!(
        matches!(frame, Message::Text(_)),
        "unexpected control frame"
    );
    serde_json::from_slice(&frame.into_data()).unwrap()
}

struct Fixture {
    _root: tempfile::TempDir,
    endpoint: Endpoint,
    requests: mpsc::UnboundedReceiver<RuntimeRequest>,
    relay: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.relay.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let (notify, mut notified) = mpsc::unbounded_channel();
        let client = Client::new("session".into(), "worker".into(), binding(), notify);
        let endpoint = Endpoint::start(Arc::clone(&client), root.path())
            .await
            .unwrap();
        let (sender, requests) = mpsc::unbounded_channel();
        let relay = tokio::spawn(async move {
            while notified.recv().await.is_some() {
                for frame in client.frames(false) {
                    if let Frame::ExecutionRequest { request } = frame {
                        sender.send(*request).unwrap();
                    }
                }
            }
        });
        Self {
            _root: root,
            endpoint,
            requests,
            relay,
        }
    }

    async fn request(&mut self) -> RuntimeRequest {
        tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }

    fn answer(&self, request: RuntimeRequest, response: Response) {
        self.endpoint.client.complete(RuntimeReply {
            session_id: request.session_id,
            worker_epoch: request.worker_epoch,
            request_id: request.request_id,
            scope: Scope::from_binding(&request.binding),
            response: MachineResponse::Call { response },
        });
    }

    async fn initialize(&mut self, cursor: u64, resume: bool) -> Socket {
        let mut socket = connect(&self.endpoint).await;
        send(
            &mut socket,
            json!({"id":0,"method":"initialize","params":{
                "resumeSessionId": if resume { json!("original-session") } else { Value::Null }
            }}),
        )
        .await;
        let request = self.request().await;
        assert!(matches!(request.command, Command::Describe));
        self.answer(
            request,
            Response::Ready {
                scope: Scope::from_binding(&binding()),
                initialization: json!({"sessionId":"original-session"}),
                event_cursor: cursor,
            },
        );
        assert_eq!(
            receive(&mut socket).await["result"]["sessionId"],
            "original-session"
        );
        socket
    }
}

#[tokio::test]
async fn legacy_event_gap_closes_cleanly_and_next_connection_uses_fresh_cursor() {
    let mut fixture = Fixture::new().await;
    let mut first = fixture.initialize(7, false).await;
    let poll = fixture.request().await;
    assert!(matches!(poll.command, Command::Events { after: 7, .. }));
    fixture.answer(
        poll,
        Response::Refused {
            reason: wire::Refusal::CursorExpired,
        },
    );
    let close = tokio::time::timeout(Duration::from_secs(5), first.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        close,
        Message::Close(Some(CloseFrame {
            code: CloseCode::Restart,
            ..
        }))
    ));
    drop(first);

    let mut second = fixture.initialize(99, true).await;
    let poll = fixture.request().await;
    assert!(matches!(poll.command, Command::Events { after: 99, .. }));
    fixture.answer(poll, Response::Events {
        events: vec![wire::Event { sequence: 100, message: json!({"method":"process/closed","params":{"processId":"original-process","seq":2}}) }],
        through: 100,
    });
    assert_eq!(
        receive(&mut second).await["params"]["processId"],
        "original-process"
    );
    let poll = fixture.request().await;
    assert!(matches!(poll.command, Command::Events { after: 100, .. }));
    send(&mut second, json!({"id":1,"method":"process/read","params":{"processId":"original-process","afterSeq":1}})).await;
    let recovery = fixture.request().await;
    assert!(
        matches!(&recovery.command, Command::Invoke { invocation, .. } if invocation.method == "process/read")
    );
    fixture.answer(
        recovery,
        Response::Operation {
            outcome: Outcome::Completed {
                reply: json!({"result":{"closed":true}}),
            },
        },
    );
    assert_eq!(
        receive(&mut second).await,
        json!({"id":1,"result":{"closed":true}})
    );
}

#[tokio::test]
async fn capacity_refuses_only_unadmitted_request_and_retains_other_results() {
    let mut fixture = Fixture::new().await;
    let mut socket = fixture.initialize(0, false).await;
    let poll = fixture.request().await;
    assert!(matches!(poll.command, Command::Events { .. }));
    let mut admitted = Vec::new();
    for id in 1..=16 {
        send(
            &mut socket,
            json!({"id":id,"method":"fs/getMetadata","params":{"path":"file:///target/file"}}),
        )
        .await;
        let request = fixture.request().await;
        assert!(matches!(request.command, Command::Invoke { .. }));
        admitted.push((id, request));
    }
    send(&mut socket, json!({"id":17,"method":"fs/writeFile","params":{"path":"file:///target/file","dataBase64":"YQ=="}})).await;
    let refused = receive(&mut socket).await;
    assert_eq!(refused["id"], 17);
    assert_eq!(refused["error"]["code"], -32000);
    assert!(
        fixture.requests.try_recv().is_err(),
        "refused write must not reach target"
    );
    for (id, request) in admitted {
        fixture.answer(
            request,
            Response::Operation {
                outcome: Outcome::Completed {
                    reply: json!({"result":{"receipt":id}}),
                },
            },
        );
    }
    let mut results = HashSet::new();
    for _ in 1..=16 {
        let reply = receive(&mut socket).await;
        assert_eq!(reply["id"], reply["result"]["receipt"]);
        results.insert(reply["id"].as_i64().unwrap());
    }
    assert_eq!(results.len(), 16);
    send(
        &mut socket,
        json!({"id":18,"method":"fs/getMetadata","params":{"path":"file:///target/file"}}),
    )
    .await;
    let request = fixture.request().await;
    fixture.answer(
        request,
        Response::Operation {
            outcome: Outcome::Completed {
                reply: json!({"result":{}}),
            },
        },
    );
    assert_eq!(receive(&mut socket).await, json!({"id":18,"result":{}}));
}

#[tokio::test]
async fn bulk_input_cannot_take_the_next_event_polls_byte_budget() {
    let mut fixture = Fixture::new().await;
    let client = Arc::clone(&fixture.endpoint.client);
    let caller = Arc::clone(&client);
    let poll = tokio::spawn(async move {
        caller
            .call(Command::Events {
                after: 0,
                wait_ms: 0,
            })
            .await
    });
    let request = fixture.request().await;
    fixture.answer(
        request,
        Response::Events {
            events: vec![],
            through: 0,
        },
    );
    poll.await.unwrap().unwrap();

    // Two valid bulk writes fill most of the budget after the completed poll
    // released its slot. Keep their receipts pending throughout the next poll.
    let params = |size| json!({"path":"file:///target/file","dataBase64":"a".repeat(size)});
    let mut writes = Vec::new();
    for _ in 0..2 {
        let caller = Arc::clone(&client);
        let value = params(6 * 1024 * 1024);
        let task = tokio::spawn(async move { caller.invoke("fs/writeFile".into(), value).await });
        let request = fixture.request().await;
        writes.push((request, task));
    }
    let mut template = writes[0].0.clone();
    let Command::Invoke { invocation, .. } = &mut template.command else {
        panic!("write invocation")
    };
    invocation.params = params(0);
    let overhead = serde_json::to_vec(&template).unwrap().len();
    let used: usize = client
        .pending
        .lock()
        .values()
        .map(|entry| entry.bytes)
        .sum();
    // Without the reserve this valid third write exactly fills the shared
    // budget, leaving no room for the subsequent event request's envelope.
    let third = client.invoke(
        "fs/writeFile".into(),
        params(MAX_PENDING_BYTES - used - overhead),
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(2), third)
            .await
            .expect("bulk capacity must refuse before admission")
            .is_err()
    );
    assert!(fixture.requests.try_recv().is_err());

    let caller = Arc::clone(&client);
    let poll = tokio::spawn(async move {
        caller
            .call(Command::Events {
                after: 0,
                wait_ms: 0,
            })
            .await
    });
    let request = fixture.request().await;
    assert!(matches!(request.command, Command::Events { .. }));
    fixture.answer(
        request,
        Response::Events {
            events: vec![],
            through: 0,
        },
    );
    assert!(matches!(
        poll.await.unwrap().unwrap(),
        Response::Events { .. }
    ));
    let retries = client.frames(true);
    for (request, task) in writes {
        assert!(retries.iter().any(|frame| matches!(frame, Frame::ExecutionRequest { request: retry } if **retry == request)));
        fixture.answer(
            request,
            Response::Operation {
                outcome: Outcome::Completed {
                    reply: json!({"result":{}}),
                },
            },
        );
        assert_eq!(task.await.unwrap().unwrap(), json!({"result":{}}));
    }
}
