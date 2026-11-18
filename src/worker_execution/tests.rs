//! The account-sandbox Unix endpoint and the trusted process ledger against
//! a scripted Machine/target relay.

use super::*;
use crate::execution_environment::{
    EnvironmentLocation, ExecutionAccess, RuntimeLocation, WorkspaceLocation,
};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

fn binding() -> BindingV1 {
    BindingV1 {
        schema: 1,
        id: "binding".into(),
        revision: 1,
        runtime: RuntimeLocation {
            machine_id: "runtime".into(),
            cwd: "/runtime".into(),
        },
        environment: EnvironmentLocation {
            machine_id: "target".into(),
            id: "environment".into(),
            incarnation: "incarnation".into(),
            executor_digest: format!("sha256:{}", "a".repeat(64)),
            protocol: 1,
        },
        workspace: WorkspaceLocation {
            id: "project".into(),
            worktree_id: "session".into(),
            source_path: "/source".into(),
            cwd: "/target".into(),
        },
        access: ExecutionAccess::Project,
        managed: None,
    }
}

/// What the scripted target does with one invocation.
enum Reply {
    Answer(Response),
    Hold,
}

type Handler = Arc<dyn Fn(&Invocation) -> Reply + Send + Sync>;

/// A Machine relay and target keeper answering every request of `client`.
struct Target {
    client: Arc<Client>,
    log: Arc<Mutex<Vec<String>>>,
    held: Arc<Mutex<Vec<RuntimeRequest>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Target {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn completed(reply: Value) -> Reply {
    Reply::Answer(Response::Operation {
        outcome: Outcome::Completed { reply },
    })
}

fn answer(client: &Client, request: RuntimeRequest, response: Response) {
    client.complete(RuntimeReply {
        session_id: request.session_id,
        worker_epoch: request.worker_epoch,
        request_id: request.request_id,
        scope: Scope::from_binding(&request.binding),
        response: MachineResponse::Call { response },
    });
}

impl Target {
    fn new(handler: Handler) -> Self {
        let (notify, mut notified) = mpsc::unbounded_channel();
        let client = Client::new("session".into(), "worker".into(), binding(), notify);
        let log = Arc::<Mutex<Vec<String>>>::default();
        let held = Arc::<Mutex<Vec<RuntimeRequest>>>::default();
        let (relay, relay_log, relay_held) =
            (Arc::clone(&client), Arc::clone(&log), Arc::clone(&held));
        let task = tokio::spawn(async move {
            while notified.recv().await.is_some() {
                for frame in relay.frames(false) {
                    let Frame::ExecutionRequest { request } = frame else {
                        continue;
                    };
                    let request = *request;
                    match &request.command {
                        Command::Describe => {
                            let response = Response::Ready {
                                scope: Scope::from_binding(&binding()),
                                initialization: json!({"sessionId": "native"}),
                                event_cursor: 0,
                            };
                            answer(&relay, request, response);
                        }
                        Command::Invoke { invocation, .. } => {
                            let id = invocation.params["processId"].as_str().unwrap_or("");
                            relay_log.lock().push(format!("{} {id}", invocation.method));
                            match handler(invocation) {
                                Reply::Answer(response) => answer(&relay, request, response),
                                Reply::Hold => relay_held.lock().push(request),
                            }
                        }
                        // Event polls and observations stay open.
                        _ => {}
                    }
                }
            }
        });
        Self {
            client,
            log,
            held,
            task,
        }
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().clone()
    }

    fn release(&self, reply: Value) {
        for request in std::mem::take(&mut *self.held.lock()) {
            answer(
                &self.client,
                request,
                Response::Operation {
                    outcome: Outcome::Completed {
                        reply: reply.clone(),
                    },
                },
            );
        }
    }
}

/// A target whose processes run until terminated.
fn processes() -> Handler {
    let terminated = Arc::new(Mutex::new(HashSet::<String>::new()));
    Arc::new(move |invocation| {
        let id = invocation.params["processId"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        match invocation.method.as_str() {
            "process/start" => completed(json!({"result": {"processId": id}})),
            "process/terminate" => {
                terminated.lock().insert(id);
                completed(json!({"result": {}}))
            }
            "process/read" => {
                let exited = terminated.lock().contains(&id);
                completed(json!({"result": {"chunks": [], "exited": exited, "closed": exited}}))
            }
            _ => completed(json!({"result": {}})),
        }
    })
}

fn ledger(client: &Arc<Client>, path: Option<PathBuf>) -> Arc<Ledger> {
    let ledger = Ledger::open("session", path).unwrap();
    client.set_ledger(Arc::clone(&ledger));
    ledger
}

async fn eventually(mut done: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition never held");
}

fn start(id: &str) -> Value {
    json!({"processId": id, "argv": ["sleep", "600"]})
}

#[tokio::test]
async fn a_start_is_durable_before_it_is_forwarded() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.json");
    let observed = Arc::new(Mutex::new(None));
    let (file, seen) = (path.clone(), Arc::clone(&observed));
    let inner = processes();
    let target = Target::new(Arc::new(move |invocation| {
        if invocation.method == "process/start" {
            *seen.lock() = Some(std::fs::read_to_string(&file).unwrap_or_default());
        }
        inner(invocation)
    }));
    let ledger = ledger(&target.client, Some(path.clone()));
    ledger.begin_turn("turn-1");
    let reply = target
        .client
        .relay("process/start".into(), start("p1"))
        .await;
    assert_eq!(reply["result"]["processId"], "p1");
    let recorded = observed.lock().clone().unwrap();
    assert!(recorded.contains("\"p1\"") && recorded.contains("turn-1"));
    assert_eq!(ledger.live(), ["p1"]);
}

#[tokio::test]
async fn starts_outside_a_turn_or_without_an_id_are_refused_unsent() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    let reply = target
        .client
        .relay("process/start".into(), start("p1"))
        .await;
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no Cowboy turn")
    );
    ledger.begin_turn("turn-1");
    let reply = target
        .client
        .relay("process/start".into(), json!({"argv": ["true"]}))
        .await;
    assert!(reply.get("error").is_some());
    assert!(target.log().is_empty());
}

#[tokio::test]
async fn a_cancel_fences_the_turn_and_terminates_until_read_proves_exit() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    target
        .client
        .relay("process/start".into(), start("p1"))
        .await;
    target.client.cancel_turn();
    eventually(|| ledger.live().is_empty()).await;
    let refused = target
        .client
        .relay("process/start".into(), start("p2"))
        .await;
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cancelled")
    );
    let log = target.log();
    assert_eq!(
        log,
        [
            "process/start p1",
            "process/terminate p1",
            "process/read p1"
        ]
    );
    // A new turn starts again.
    ledger.begin_turn("turn-2");
    let reply = target
        .client
        .relay("process/start".into(), start("p3"))
        .await;
    assert_eq!(reply["result"]["processId"], "p3");
}

#[tokio::test]
async fn turn_end_waits_for_an_in_flight_start_before_terminating() {
    let inner = processes();
    let target = Target::new(Arc::new(move |invocation| {
        if invocation.method == "process/start" {
            Reply::Hold
        } else {
            inner(invocation)
        }
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    let client = Arc::clone(&target.client);
    let starting =
        tokio::spawn(async move { client.relay("process/start".into(), start("p1")).await });
    eventually(|| !target.held.lock().is_empty()).await;
    let settled = tokio::spawn(target.client.finish_turn("turn-1").unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!settled.is_finished());
    assert_eq!(target.log(), ["process/start p1"]);
    target.release(json!({"result": {"processId": "p1"}}));
    starting.await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), settled)
        .await
        .unwrap()
        .unwrap();
    assert!(ledger.live().is_empty());
    assert_eq!(
        target.log(),
        [
            "process/start p1",
            "process/terminate p1",
            "process/read p1"
        ]
    );
    assert!(target.client.finish_turn("turn-1").is_none());
}

#[tokio::test]
async fn an_unknown_id_counts_as_gone_only_once_its_start_settled() {
    let unknown = json!({"error": {"code": -32600, "message": "unknown process id p1"}});
    let reads = Arc::new(Mutex::new(0));
    let counted = Arc::clone(&reads);
    let target = Target::new(Arc::new(move |invocation| {
        match invocation.method.as_str() {
            "process/start" => completed(json!({"result": {"processId": "p1"}})),
            "process/read" => {
                *counted.lock() += 1;
                completed(unknown.clone())
            }
            _ => completed(unknown.clone()),
        }
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    target
        .client
        .relay("process/start".into(), start("p1"))
        .await;
    target.client.finish_turn("turn-1").unwrap().await;
    assert!(ledger.live().is_empty());
    assert_eq!(*reads.lock(), 1);
}

#[tokio::test]
async fn a_dead_workers_ledger_is_terminated_by_the_next_worker() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.json");
    {
        let target = Target::new(processes());
        let ledger = ledger(&target.client, Some(path.clone()));
        ledger.begin_turn("turn-1");
        target
            .client
            .relay("process/start".into(), start("p1"))
            .await;
        target
            .client
            .relay(
                "fs/writeFile".into(),
                json!({"path": "file:///target/a", "dataBase64": ""}),
            )
            .await;
        // The worker dies here: nothing settles.
    }
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["processes"]["p1"]["turn"], "turn-1");
    let target = Target::new(processes());
    let ledger = ledger(&target.client, Some(path.clone()));
    assert_eq!(ledger.live(), ["p1"]);
    // The next turn's completion waits for the recovered process too.
    ledger.begin_turn("turn-2");
    let settled = target.client.finish_turn("turn-2").unwrap();
    target.client.settle_recovered();
    settled.await;
    assert!(ledger.live().is_empty());
    assert!(!path.exists(), "an empty ledger leaves no file");
    assert_eq!(target.log(), ["process/terminate p1", "process/read p1"]);
}

#[tokio::test]
async fn lost_mutation_replies_mark_the_effect_uncertain_durably() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.json");
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/remove" => Reply::Answer(Response::Refused {
            reason: wire::Refusal::Capacity,
        }),
        "fs/writeFile" => Reply::Answer(Response::Operation {
            outcome: Outcome::Unknown,
        }),
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, Some(path.clone()));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&observed);
    ledger.set_observer(Box::new(move |method, sequence| {
        seen.lock().push((method.to_owned(), sequence));
    }));
    // A refusal before admission had no effect.
    let refused = target
        .client
        .relay("fs/remove".into(), json!({"path": "file:///target/a"}))
        .await;
    assert!(refused.get("error").is_some());
    assert!(ledger.uncertain_effect().is_none());
    // A read is not a mutation.
    target
        .client
        .relay("fs/readFile".into(), json!({"path": "file:///target/a"}))
        .await;
    assert!(ledger.uncertain_effect().is_none());
    let lost = target
        .client
        .relay(
            "fs/writeFile".into(),
            json!({"path": "file:///target/a", "dataBase64": ""}),
        )
        .await;
    // The unknown outcome is typed for the agent side, unlike a refusal.
    assert_eq!(lost.pointer("/error/data/outcome"), Some(&json!("unknown")));
    assert!(refused.pointer("/error/data").is_none());
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("fs/writeFile"));
    assert_eq!(*observed.lock(), [("fs/writeFile".to_owned(), 1)]);
    drop(target);
    let reopened = Ledger::open("session", Some(path)).unwrap();
    assert_eq!(reopened.uncertain_effect().as_deref(), Some("fs/writeFile"));
    // Each lost reply advances the sequence the owner acknowledges.
    assert_eq!(reopened.uncertain_sequence(), 1);
    assert!(Ledger::open("other", Some(root.path().join("session.json"))).is_err());
}

#[tokio::test]
async fn relayed_exit_evidence_retires_a_process() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    for id in ["p1", "p2"] {
        target.client.relay("process/start".into(), start(id)).await;
    }
    target
        .client
        .observe_event(&json!({"method": "process/exited", "params": {"processId": "p1"}}));
    assert_eq!(ledger.live(), ["p2"]);
    target
        .client
        .relay("process/terminate".into(), json!({"processId": "p2"}))
        .await;
    target
        .client
        .relay(
            "process/read".into(),
            json!({"processId": "p2", "afterSeq": null, "maxBytes": 10, "waitMs": 0}),
        )
        .await;
    assert!(ledger.live().is_empty());
    assert!(target.client.finish_turn("turn-1").is_none());
}

#[tokio::test]
async fn teardown_settles_every_turn_and_refuses_further_starts() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    target
        .client
        .relay("process/start".into(), start("p1"))
        .await;
    target.client.settle_all().await;
    assert!(ledger.live().is_empty());
    ledger.begin_turn("turn-2");
    let refused = target
        .client
        .relay("process/start".into(), start("p2"))
        .await;
    assert!(refused.get("error").is_some());
}

type UnixSocket = tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>;

struct UnixFixture {
    _root: tempfile::TempDir,
    socket: PathBuf,
    descriptor: Value,
    _endpoint: Endpoint,
    target: Target,
}

impl UnixFixture {
    fn new() -> Self {
        Self::with(processes())
    }

    fn with(handler: Handler) -> Self {
        let target = Target::new(handler);
        let root = tempfile::tempdir().unwrap();
        let (endpoint, bridge) = Endpoint::start_unix(Arc::clone(&target.client)).unwrap();
        assert!(endpoint.descriptor().is_none());
        let socket = root.path().join("exec.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        bridge.listeners.send(OwnedFd::from(listener)).unwrap();
        let descriptor = serde_json::from_slice(&bridge.descriptor).unwrap();
        Self {
            _root: root,
            socket,
            descriptor,
            _endpoint: endpoint,
            target,
        }
    }

    async fn connect(&self, token: &str) -> Result<UnixSocket> {
        let stream = tokio::net::UnixStream::connect(&self.socket).await?;
        let mut request = "ws://localhost/".into_client_request()?;
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse()?);
        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::client_async(request, stream),
        )
        .await??;
        Ok(socket)
    }

    async fn initialized(&self) -> UnixSocket {
        let token = self.descriptor["bearer_token"].as_str().unwrap();
        let mut socket = self.connect(token).await.unwrap();
        socket
            .send(Message::Text(
                json!({"id": 0, "method": "initialize", "params": {}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let reply = receive(&mut socket).await;
        assert_eq!(reply["result"]["sessionId"], "native");
        socket
    }
}

async fn receive(socket: &mut UnixSocket) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_slice(&frame.into_data()).unwrap()
}

#[tokio::test]
async fn the_unix_descriptor_names_the_unit_socket() {
    let fixture = UnixFixture::new();
    assert_eq!(fixture.descriptor["endpoint"], "unix:/run/cowboy-exec.sock");
    assert_eq!(fixture.descriptor["schema"], 1);
    assert_eq!(
        fixture.descriptor["bearer_token"].as_str().unwrap().len(),
        64
    );
    assert_eq!(
        serde_json::from_value::<BindingV1>(fixture.descriptor["binding"].clone()).unwrap(),
        binding()
    );
    assert!(fixture.descriptor.get("file_buffers").is_none());
}

#[tokio::test]
async fn the_unix_endpoint_authenticates_and_relays() {
    let fixture = UnixFixture::new();
    assert!(fixture.connect("wrong").await.is_err());
    let mut socket = fixture.initialized().await;
    socket
        .send(Message::Text(
            json!({"id": 1, "method": "fs/readFile", "params": {"path": "file:///target/a"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let reply = receive(&mut socket).await;
    assert_eq!(reply["id"], 1);
    assert_eq!(fixture.target.log(), ["fs/readFile "]);
}

#[tokio::test]
async fn the_unix_endpoint_admits_one_connection() {
    let fixture = UnixFixture::new();
    let token = fixture.descriptor["bearer_token"].as_str().unwrap();
    let _first = fixture.initialized().await;
    assert!(fixture.connect(token).await.is_err());
}

#[tokio::test]
async fn the_unix_endpoint_bounds_frames() {
    let fixture = UnixFixture::new();
    let mut socket = fixture.initialized().await;
    let oversized = "x".repeat(wire::MAX_FRAME_BYTES + 1);
    let _ = socket.send(Message::Text(oversized.into())).await;
    let closed = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .unwrap();
    assert!(!matches!(closed, Some(Ok(Message::Text(_)))));
    // The connection slot is free again.
    let _again = fixture.initialized().await;
}

#[tokio::test]
async fn readiness_probes_run_outside_turns_one_at_a_time() {
    let target = Target::new(Arc::new(|_| completed(json!({"result": {}}))));
    let ledger = ledger(&target.client, None);
    // No turn runs while a session is created; only a readiness probe starts.
    assert!(ledger.admit_start("cowboy-agy-plain").is_err());
    let first = format!("{}1", super::ledger::READINESS_PREFIX);
    let second = format!("{}2", super::ledger::READINESS_PREFIX);
    ledger.admit_start(&first).unwrap();
    assert!(ledger.admit_start(&second).is_err());
    // It is recorded like every process and ends with the target's evidence.
    assert_eq!(ledger.live(), std::slice::from_ref(&first));
    ledger.exited(&first);
    ledger.admit_start(&second).unwrap();
}

#[tokio::test]
async fn a_closed_socket_neither_aborts_an_admitted_relay_nor_drops_its_outcome() {
    let inner = processes();
    let fixture = UnixFixture::with(Arc::new(move |invocation| {
        if invocation.method == "fs/writeFile" {
            Reply::Hold
        } else {
            inner(invocation)
        }
    }));
    let ledger = ledger(&fixture.target.client, None);
    let mut socket = fixture.initialized().await;
    socket
        .send(Message::Text(
            json!({"id": 1, "method": "fs/writeFile",
                   "params": {"path": "file:///target/a", "dataBase64": ""}})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    eventually(|| fixture.target.held.lock().len() == 1).await;
    // The agent side goes away while the target holds the admitted write.
    drop(socket);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(ledger.uncertain_effect().is_none());
    // The relay still finishes; its reply reaches nobody, so the effect the
    // agent never learned is recorded as uncertain.
    fixture.target.release(json!({"result": {}}));
    eventually(|| ledger.uncertain_effect().is_some()).await;
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("fs/writeFile"));
}
