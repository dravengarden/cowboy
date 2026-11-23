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
                        Command::Observe { operation_id, .. } => {
                            relay_log.lock().push(format!("observe {operation_id}"));
                            // The keeper long-polls and still reports pending.
                            std::thread::sleep(Duration::from_millis(20));
                            let response = Response::Operation {
                                outcome: Outcome::Pending,
                            };
                            answer(&relay, request, response);
                        }
                        // Event polls stay open.
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

fn probe(id: &str) -> Value {
    json!({"processId": id, "argv": ["/bin/sh", "-c", super::ledger::READINESS_SCRIPT, "sh", "/target"]})
}

#[tokio::test]
async fn a_start_is_recorded_before_it_is_forwarded() {
    let recorded = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&recorded);
    let inner = processes();
    let holder: Arc<OnceLock<Arc<Ledger>>> = Arc::default();
    let reader = Arc::clone(&holder);
    let target = Target::new(Arc::new(move |invocation| {
        if invocation.method == "process/start" {
            *seen.lock() = reader.get().map(|ledger| ledger.live());
        }
        inner(invocation)
    }));
    let ledger = ledger(&target.client, None);
    let _ = holder.set(Arc::clone(&ledger));
    ledger.begin_turn("turn-1");
    let reply = target
        .client
        .relay("process/start".into(), start("p1"))
        .await;
    assert_eq!(reply["result"]["processId"], "p1");
    assert_eq!(recorded.lock().clone().unwrap(), ["p1".to_owned()]);
    assert_eq!(ledger.live(), ["p1"]);
}

#[tokio::test]
async fn starts_outside_a_turn_or_without_an_id_are_refused_unsent() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    ledger.set_readiness(false);
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
    // Filesystem mutations need a turn too, even in the readiness window.
    ledger.set_readiness(true);
    let write = target
        .client
        .relay(
            "fs/writeFile".into(),
            json!({"path": "file:///target/a", "dataBase64": ""}),
        )
        .await;
    assert!(
        write["error"]["message"]
            .as_str()
            .unwrap()
            .contains("filesystem change was refused")
    );
    ledger.set_readiness(false);
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
async fn a_dead_workers_processes_are_not_inherited() {
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
        // The worker dies here: nothing settles.
    }
    let target = Target::new(processes());
    let ledger = ledger(&target.client, Some(path));
    assert!(ledger.live().is_empty());
    ledger.begin_turn("turn-2");
    assert!(target.client.finish_turn("turn-2").is_none());
    assert!(target.log().is_empty());
}

#[tokio::test]
async fn lost_mutation_replies_mark_the_effect_uncertain_durably() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.json");
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/remove" => Reply::Answer(Response::Refused {
            reason: wire::Refusal::Capacity,
        }),
        "fs/writeFile" | "process/terminate" => Reply::Answer(Response::Operation {
            outcome: Outcome::Unknown,
        }),
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, Some(path.clone()));
    ledger.begin_turn("turn-1");
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
    let first = {
        let seen = observed.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "fs/writeFile");
        seen[0].1
    };
    assert!(first > 1, "time-based: never reused after a lost write");
    // A lost termination result is uncertain too.
    target
        .client
        .relay("process/terminate".into(), json!({"processId": "p9"}))
        .await;
    assert_eq!(
        ledger.uncertain_effect().as_deref(),
        Some("process/terminate")
    );
    drop(target);
    let reopened = Ledger::open("session", Some(path)).unwrap();
    assert_eq!(
        reopened.uncertain_effect().as_deref(),
        Some("process/terminate")
    );
    // Each lost reply advances the sequence the owner acknowledges.
    assert!(reopened.uncertain_sequence() > first);
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
        let (endpoint, bridge) = Endpoint::start_unix(Arc::clone(&target.client), None).unwrap();
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
async fn the_readiness_probe_runs_only_while_the_worker_starts() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    // While the session starts, only the exact projection probe may run
    // outside a turn, whatever its id; any other command is refused.
    let other = target
        .client
        .relay("process/start".into(), start("other"))
        .await;
    assert!(
        other["error"]["message"]
            .as_str()
            .unwrap()
            .contains("only the readiness probe")
    );
    let first = target
        .client
        .relay("process/start".into(), probe("any-id"))
        .await;
    assert_eq!(first["result"]["processId"], "any-id");
    // A second waits for the first.
    let second = target
        .client
        .relay("process/start".into(), probe("second"))
        .await;
    assert!(
        second["error"]["message"]
            .as_str()
            .unwrap()
            .contains("readiness")
    );
    ledger.exited("any-id");
    // Once the worker reports its session running, no probe id helps.
    ledger.set_readiness(false);
    let late = target
        .client
        .relay("process/start".into(), probe("cowboy-agy-readiness-1"))
        .await;
    assert!(
        late["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no Cowboy turn")
    );
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
    ledger.begin_turn("turn-1");
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

#[tokio::test]
async fn a_turns_completion_waits_for_its_admitted_writes() {
    let target = Target::new(Arc::new(|invocation| {
        if invocation.method == "fs/writeFile" {
            Reply::Hold
        } else {
            completed(json!({"result": {}}))
        }
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    let client = Arc::clone(&target.client);
    let writing = tokio::spawn(async move {
        client
            .relay(
                "fs/writeFile".into(),
                json!({"path": "file:///target/a", "dataBase64": ""}),
            )
            .await
    });
    eventually(|| !target.held.lock().is_empty()).await;
    let settled = tokio::spawn(target.client.finish_turn("turn-1").unwrap());
    // Fenced: a later write of the ended turn is refused unsent.
    let refused = target
        .client
        .relay("fs/remove".into(), json!({"path": "file:///target/a"}))
        .await;
    assert!(refused.get("error").is_some());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!settled.is_finished());
    target.release(json!({"result": {}}));
    writing.await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), settled)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target.log(), ["fs/writeFile "]);
}

#[tokio::test]
async fn a_lost_read_of_a_turn_process_after_its_exit_is_uncertain() {
    let fresh = Target::new(processes());
    let probes = ledger(&fresh.client, None);
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    target
        .client
        .relay("process/start".into(), start("publish"))
        .await;
    // The exit notification removes the process from the ledger first.
    ledger.exited("publish");
    let read = json!({"processId": "publish", "afterSeq": null, "maxBytes": 1, "waitMs": 0});
    assert!(target.client.carries_result("process/read", &read));
    target
        .client
        .undelivered("process/read", true, &json!({"result": {"exited": true}}));
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("process/read"));
    // A readiness probe's output carries no result the agent acts on.
    fresh
        .client
        .relay("process/start".into(), probe("probe-1"))
        .await;
    probes.exited("probe-1");
    let read = json!({"processId": "probe-1"});
    assert!(!fresh.client.carries_result("process/read", &read));
}

#[tokio::test]
async fn an_uncertain_effect_reported_by_the_agent_side_is_recorded_unforwarded() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    let reply = target
        .client
        .relay(
            super::ledger::UNCERTAIN_REPORT.into(),
            json!({"method": "file publish"}),
        )
        .await;
    assert!(reply.get("result").is_some());
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("file publish"));
    assert!(ledger.uncertain_sequence() > 0);
    assert!(target.log().is_empty(), "never forwarded to the target");
}

#[tokio::test]
async fn worker_servers_outlive_turns_and_stay_out_of_the_units_reach() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    ledger.set_readiness(false);
    ledger.record_server("server-1").unwrap();
    ledger.server_started("server-1", true);
    // The unit can neither write to, read from nor stop the worker's server.
    for method in ["process/write", "process/read", "process/terminate"] {
        let reply = target
            .client
            .relay(method.into(), json!({"processId": "server-1"}))
            .await;
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("belongs to the worker"),
            "{method}"
        );
    }
    // An extension call needs the active turn.
    assert!(ledger.admit_call().is_err());
    ledger.begin_turn("turn-1");
    let admitted = ledger.admit_call().unwrap();
    let settled = tokio::spawn(target.client.finish_turn("turn-1").unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!settled.is_finished(), "the turn drains its extension call");
    ledger.settle_call(&admitted);
    tokio::time::timeout(Duration::from_secs(10), settled)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ledger.live(), ["server-1"]);
    assert!(target.log().is_empty());
    // Teardown ends it.
    tokio::time::timeout(Duration::from_secs(10), target.client.settle_all())
        .await
        .unwrap();
    assert!(
        target
            .log()
            .iter()
            .any(|line| line == "process/terminate server-1")
    );
}

#[tokio::test]
async fn memory_requests_need_the_sessions_bridge() {
    let target = Target::new(processes());
    let _ledger = ledger(&target.client, None);
    let reply = target
        .client
        .relay("cowboy/memory".into(), json!({"path": "/v1/context"}))
        .await;
    let call = target
        .client
        .relay(
            "cowboy/extensionCall".into(),
            json!({"name": "cowboy_read"}),
        )
        .await;
    assert!(call.get("error").is_some());
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("memory is not available")
    );
    assert!(target.log().is_empty());
}

/// A target running one stdio MCP server per started process: it answers
/// `initialize`, `tools/list` and `tools/call` on its stdout.
fn mcp_target() -> Handler {
    use base64::Engine as _;
    let output = Arc::new(Mutex::new(BTreeMap::<String, Vec<Value>>::new()));
    Arc::new(move |invocation| {
        let id = invocation.params["processId"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        match invocation.method.as_str() {
            "process/start" => {
                assert_eq!(invocation.params["pipeStdin"], true);
                output.lock().insert(id.clone(), Vec::new());
                completed(json!({"result": {"processId": id}}))
            }
            "process/write" => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(invocation.params["chunk"].as_str().unwrap())
                    .unwrap();
                let message: Value = serde_json::from_slice(&bytes).unwrap();
                let result = match message["method"].as_str().unwrap_or("") {
                    "initialize" => Some(json!({"instructions": "Prefer Cowboy MCP."})),
                    "tools/list" => Some(json!({"tools": [
                        {"name": "cowboy_read", "inputSchema": {"type": "object"}},
                        {"name": "ReadFile"},
                    ]})),
                    "tools/call" => Some(json!({"content": [{"type": "text",
                        "text": message["params"]["name"]}]})),
                    _ => None,
                };
                if let (Some(result), Some(rpc)) = (result, message.get("id")) {
                    // A server-initiated request with the same id comes first.
                    let mut lines = format!(
                        "{}\n{}\n",
                        json!({"jsonrpc": "2.0", "id": rpc, "method": "ping"}),
                        json!({"jsonrpc": "2.0", "id": rpc, "result": result})
                    )
                    .into_bytes();
                    let mut chunks = output.lock();
                    let list = chunks.get_mut(&id).unwrap();
                    // Split inside the reply to exercise reassembly.
                    let tail = lines.split_off(lines.len() / 2);
                    for part in [lines, tail] {
                        let seq = list.len() as u64 + 1;
                        list.push(json!({"seq": seq, "stream": "stdout",
                            "chunk": base64::engine::general_purpose::STANDARD.encode(part)}));
                    }
                }
                completed(json!({"result": {"status": "accepted"}}))
            }
            "process/read" => {
                let after = invocation.params["afterSeq"].as_u64().unwrap_or(0);
                let chunks: Vec<Value> = output
                    .lock()
                    .get(&id)
                    .map(|list| {
                        list.iter()
                            .filter(|chunk| chunk["seq"].as_u64().unwrap() > after)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                completed(json!({"result": {"chunks": chunks, "exited": false, "closed": false}}))
            }
            _ => completed(json!({"result": {}})),
        }
    })
}

#[tokio::test]
async fn the_broker_runs_target_servers_for_the_unit_inside_turns() {
    let target = Target::new(mcp_target());
    let ledger = ledger(&target.client, None);
    ledger.set_readiness(false);
    let broker = super::ExtensionBroker::start(
        &target.client,
        super::ExtensionPlan {
            servers: vec![super::ServerPlan {
                name: "cowboy".into(),
                argv: vec!["cowboy".into(), "mcp".into()],
                required: false,
            }],
            memory_tools: Vec::new(),
            memory_server: "matrix".into(),
            guidance: vec!["Prefer {{peer}}.".into()],
        },
        None,
    );
    target.client.set_extensions(broker);
    let listed = target
        .client
        .relay("cowboy/extensions".into(), json!({}))
        .await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    // A facade name takes its server's prefix.
    assert_eq!(names, ["cowboy_read", "cowboy_ReadFile"]);
    assert_eq!(listed["result"]["guidance"], json!(["Prefer {{peer}}."]));
    assert!(
        listed["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("Prefer Cowboy MCP.")
    );
    let outside = target
        .client
        .relay(
            "cowboy/extensionCall".into(),
            json!({"name": "cowboy_read", "arguments": {}}),
        )
        .await;
    assert!(
        outside["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no Cowboy turn")
    );
    ledger.begin_turn("turn-1");
    let called = target
        .client
        .relay(
            "cowboy/extensionCall".into(),
            json!({"name": "cowboy_ReadFile", "arguments": {}}),
        )
        .await;
    assert_eq!(called["result"]["content"][0]["text"], "ReadFile");
    let unknown = target
        .client
        .relay("cowboy/extensionCall".into(), json!({"name": "nope"}))
        .await;
    assert!(unknown.get("error").is_some());
}

#[tokio::test]
async fn a_refusal_after_a_resend_does_not_hide_a_possible_admission() {
    // F7: the first send's reply is lost; the resend is refused. The keeper
    // may have admitted the first send, so the effect is uncertain.
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/writeFile" => Reply::Hold,
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    let client = Arc::clone(&target.client);
    let relay = tokio::spawn(async move {
        client
            .relay(
                "fs/writeFile".into(),
                json!({"path": "file:///target/a", "dataBase64": ""}),
            )
            .await
    });
    eventually(|| !target.held.lock().is_empty()).await;
    // A reconnect resends the same request under its identity.
    assert_eq!(target.client.frames(true).len(), 1);
    let request = target.held.lock().pop().unwrap();
    answer(
        &target.client,
        request,
        Response::Refused {
            reason: wire::Refusal::WorkspaceChanged,
        },
    );
    let reply = relay.await.unwrap();
    assert_eq!(
        reply.pointer("/error/data/outcome"),
        Some(&json!("unknown"))
    );
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("fs/writeFile"));
}

#[tokio::test]
async fn a_refusal_of_the_only_send_is_not_an_admission() {
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/writeFile" => Reply::Answer(Response::Refused {
            reason: wire::Refusal::WorkspaceChanged,
        }),
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    let reply = target
        .client
        .relay(
            "fs/writeFile".into(),
            json!({"path": "file:///target/a", "dataBase64": ""}),
        )
        .await;
    assert!(reply.get("error").is_some());
    assert!(reply.pointer("/error/data").is_none());
    assert!(ledger.uncertain_effect().is_none());
}

#[tokio::test]
async fn a_process_gets_one_stop_from_the_unit_and_the_worker_together() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    target
        .client
        .relay("process/start".into(), start("p1"))
        .await;
    target
        .client
        .relay("process/terminate".into(), json!({"processId": "p1"}))
        .await;
    // A repeated stop from the unit is not forwarded.
    let again = target
        .client
        .relay("process/terminate".into(), json!({"processId": "p1"}))
        .await;
    assert!(again.get("result").is_some());
    // The worker's own cancellation only observes.
    target.client.cancel_turn();
    eventually(|| ledger.live().is_empty()).await;
    let stops = target
        .log()
        .iter()
        .filter(|entry| entry.starts_with("process/terminate"))
        .count();
    assert_eq!(stops, 1);
}

#[tokio::test]
async fn an_unwritable_ledger_refuses_further_effects() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("ledger");
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("session.json");
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/writeFile" => Reply::Answer(Response::Operation {
            outcome: Outcome::Unknown,
        }),
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, Some(path));
    ledger.begin_turn("turn-1");
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
    target
        .client
        .relay(
            "fs/writeFile".into(),
            json!({"path": "file:///target/a", "dataBase64": ""}),
        )
        .await;
    // Published although not durable, then no effect is admitted.
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("fs/writeFile"));
    let refused = target
        .client
        .relay(
            "fs/createDirectory".into(),
            json!({"path": "file:///target/b"}),
        )
        .await;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot be written")
    );
}
