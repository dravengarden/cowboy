//! The account-sandbox Unix endpoint and the trusted process ledger against
//! a scripted Machine/target relay.

use super::*;
use crate::execution_environment::{
    EnvironmentLocation, ExecutionAccess, RuntimeLocation, WorkspaceLocation,
};
use futures::SinkExt as _;
use std::collections::HashSet;
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
                    // Delivered to the scripted relay: one send attempt.
                    relay.sending(&frame);
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
        "fs/getMetadata" => Reply::Answer(Response::Refused {
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
    // A refused read had no effect (a refused effect may have had one).
    let refused = target
        .client
        .relay("fs/getMetadata".into(), json!({"path": "file:///target/a"}))
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
    // Leader exit alone keeps the process tree in live settlement until closed.
    assert_eq!(ledger.live(), ["p1", "p2"]);
    target
        .client
        .observe_event(&json!({"method": "process/closed", "params": {"processId": "p1"}}));
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
    let resent = target.client.frames(true);
    assert_eq!(resent.len(), 1);
    target.client.sending(&resent[0]);
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
async fn a_relay_counts_in_flight_until_its_reply_is_handled() {
    // Design L1c: a unit's stop waits for this count.
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/readFile" => Reply::Hold,
        _ => completed(json!({"result": {}})),
    }));
    let _ledger = ledger(&target.client, None);
    let quiet = Arc::clone(&target.client.relays);
    assert!(
        quiet.drained(Duration::from_millis(10)).await,
        "idle: immediate"
    );
    let mut relays = relay::Relays::new(Arc::clone(&target.client), 1);
    relays.spawn(
        json!(1),
        "fs/readFile".into(),
        json!({"path": "file:///target/a"}),
        quiet.try_enter("fs/readFile").unwrap(),
    );
    eventually(|| !target.held.lock().is_empty()).await;
    assert_eq!(quiet.in_flight(), 1);
    assert!(
        !quiet.drained(Duration::from_millis(50)).await,
        "held past the bound"
    );
    let waiting = {
        let quiet = Arc::clone(&quiet);
        tokio::spawn(async move { quiet.drained(Duration::from_secs(10)).await })
    };
    target.release(json!({"result": {"dataBase64": ""}}));
    // Finished but not yet handed to the unit: still owed.
    let done = relays.next().await;
    assert_eq!(quiet.in_flight(), 1);
    drop(done);
    assert!(waiting.await.unwrap(), "drained once the reply was handled");
    assert_eq!(quiet.in_flight(), 0);
}

#[tokio::test]
async fn a_relay_whose_socket_closed_counts_until_recorded_undelivered() {
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "process/terminate" => Reply::Hold,
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, None);
    let quiet = Arc::clone(&target.client.relays);
    let mut relays = relay::Relays::new(Arc::clone(&target.client), 1);
    relays.spawn(
        json!(1),
        "process/terminate".into(),
        json!({"processId": "p1"}),
        quiet.try_enter("process/terminate").unwrap(),
    );
    eventually(|| !target.held.lock().is_empty()).await;
    // The unit's connection ends with the stop in flight.
    drop(relays);
    assert_eq!(quiet.in_flight(), 1);
    target.release(json!({"result": {"running": false}}));
    assert!(quiet.drained(Duration::from_secs(10)).await);
    // Answered after the socket closed: recorded as undelivered (fail closed).
    assert_eq!(
        ledger.uncertain_effect().as_deref(),
        Some("process/terminate")
    );
}

#[test]
fn a_draining_unit_relays_only_cleanup() {
    let quiet = Arc::new(crate::account_sandbox::RelayQuiet::default());
    assert!(quiet.try_enter("fs/writeFile").is_some());
    assert_eq!(quiet.in_flight(), 0, "a dropped guard leaves no count");
    quiet.begin_drain();
    for method in [
        "process/read",
        "process/terminate",
        "cowboy/uncertainEffect",
    ] {
        assert!(quiet.try_enter(method).is_some(), "{method}");
    }
    for method in [
        "fs/writeFile",
        "process/start",
        "process/write",
        "process/signal",
        "fs/readFile",
    ] {
        assert!(quiet.try_enter(method).is_none(), "{method}");
    }
    assert_eq!(quiet.in_flight(), 0, "a refused entry leaves no count");
    // A replacement unit relays everything again.
    quiet.end_drain();
    assert!(quiet.try_enter("fs/writeFile").is_some());
}

fn recorded_quiet() -> (
    Arc<crate::account_sandbox::RelayQuiet>,
    Arc<Mutex<Vec<String>>>,
) {
    let quiet = Arc::new(crate::account_sandbox::RelayQuiet::default());
    let recorded = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::clone(&recorded);
    quiet.set_recorder(Box::new(move |effects| {
        sink.lock().extend_from_slice(effects)
    }));
    (quiet, recorded)
}

#[test]
fn only_the_units_acknowledgement_proves_a_drained_reply_was_read() {
    let (quiet, recorded) = recorded_quiet();
    // Before the drain the unit reads its replies as usual.
    quiet.written(&["process/read"]);
    // Acknowledged: nothing is recorded.
    quiet.begin_drain();
    quiet.written(&["process/terminate"]);
    quiet.acknowledge();
    assert!(recorded.lock().is_empty());
    // A reply after the acknowledgement is unread (the unit is closing).
    quiet.written(&["process/read"]);
    assert_eq!(*recorded.lock(), ["process/read"]);
    // Stopped without an acknowledgement: every drained reply, also one
    // written after the forced stop, is recorded.
    quiet.begin_drain();
    quiet.written(&["fs/writeFile"]);
    quiet.forced();
    quiet.written(&["tools/call"]);
    quiet.acknowledge();
    assert_eq!(
        *recorded.lock(),
        ["process/read", "fs/writeFile", "tools/call"]
    );
}

#[test]
fn a_replacement_socket_cannot_acknowledge_an_earlier_sockets_replies() {
    let (quiet, recorded) = recorded_quiet();
    quiet.begin_drain();
    quiet.written(&["process/terminate"]);
    // The socket ends before the unit read it; a new socket acknowledges.
    quiet.socket_ended();
    quiet.acknowledge();
    assert_eq!(*recorded.lock(), ["process/terminate"]);
}

#[test]
fn unproven_receipts_are_bounded() {
    let (quiet, recorded) = recorded_quiet();
    quiet.begin_drain();
    for _ in 0..300 {
        quiet.written(&["process/read"]);
    }
    // Beyond the bound they are recorded at once instead of kept.
    assert!(!recorded.lock().is_empty());
    quiet.acknowledge();
    assert!(recorded.lock().len() <= 300);
}

#[test]
fn an_unread_refusal_leaves_no_effect_and_an_answered_mutation_does() {
    use super::ledger::unread_effects;
    let refusal = json!({"error": {"code": -32000, "message": "refused"}});
    assert!(unread_effects("fs/writeFile", false, &refusal).is_empty());
    assert_eq!(
        unread_effects("fs/writeFile", false, &json!({"result": {}})),
        ["fs/writeFile"]
    );
    assert!(unread_effects("fs/readFile", false, &json!({"result": {}})).is_empty());
}

#[tokio::test]
async fn a_drain_acknowledgement_is_answered_by_the_endpoint_and_never_forwarded() {
    let target = Target::new(processes());
    let _ledger = ledger(&target.client, None);
    target.client.relays.begin_drain();
    let reply = target
        .client
        .relay(crate::account_sandbox::DRAINED.into(), json!({}))
        .await;
    assert!(reply.get("result").is_some());
    assert!(
        target
            .client
            .relays
            .wait_acknowledged(Duration::from_millis(10))
            .await
    );
    assert!(target.log().is_empty());
}

#[tokio::test]
async fn a_batch_entry_never_written_is_not_a_send() {
    // The second entry of a batch whose first write failed was never sent:
    // its later refusal on its first actual send is not an admission.
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/readFile" => Reply::Hold,
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    let client = Arc::clone(&target.client);
    let relay = tokio::spawn(async move {
        client
            .relay("fs/readFile".into(), json!({"path": "file:///target/a"}))
            .await
    });
    eventually(|| !target.held.lock().is_empty()).await;
    // A reconnect batch is built but its write fails before this entry.
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
    assert!(reply.pointer("/error/data").is_none());
    assert!(ledger.uncertain_effect().is_none());
}

/// What a ledger's observer was told: each published method and sequence.
type Published = Arc<Mutex<Vec<(String, u64)>>>;

/// A ledger whose next durable write fails: a directory now occupies its
/// path, so the atomic rename cannot replace it (whatever the privileges).
fn unwritable_ledger(target: &Target, root: &std::path::Path) -> (Arc<Ledger>, Published) {
    let path = root.join("session.json");
    let ledger = ledger(&target.client, Some(path.clone()));
    std::fs::create_dir(&path).unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&observed);
    ledger.set_observer(Box::new(move |method, sequence| {
        seen.lock().push((method.to_owned(), sequence));
    }));
    (ledger, observed)
}

#[tokio::test]
async fn an_unwritable_ledger_publishes_then_refuses_every_effect_but_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/writeFile" => Reply::Answer(Response::Operation {
            outcome: Outcome::Unknown,
        }),
        _ => completed(json!({"result": {}})),
    }));
    let (ledger, observed) = unwritable_ledger(&target, root.path());
    ledger.begin_turn("turn-1");
    target
        .client
        .relay(
            "fs/writeFile".into(),
            json!({"path": "file:///target/a", "dataBase64": ""}),
        )
        .await;
    // Published although its write failed.
    {
        let seen = observed.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "fs/writeFile");
        assert_eq!(seen[0].1, ledger.uncertain_sequence());
    }
    assert!(ledger.unwritable());
    for (method, params) in [
        ("fs/createDirectory", json!({"path": "file:///target/b"})),
        ("process/write", json!({"processId": "p1"})),
        ("process/signal", json!({"processId": "p1"})),
    ] {
        let refused = target.client.relay(method.into(), params).await;
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("cannot be written"),
            "{method}"
        );
    }
    assert!(ledger.admit_call().is_err());
    assert!(ledger.record_server("cowboy-worker-mcp-x").is_err());
    // Observation and stopping remain.
    for method in ["process/read", "process/terminate"] {
        let reply = target
            .client
            .relay(method.into(), json!({"processId": "p1"}))
            .await;
        assert!(reply.get("result").is_some(), "{method}");
    }
}

#[tokio::test]
async fn a_refused_effect_is_possibly_admitted_but_a_refused_read_is_not() {
    // The Machine may resend below the worker (a bulk-lane retry), so a
    // refusal of an effectful request cannot prove it had no effect.
    let target = Target::new(Arc::new(|_| {
        Reply::Answer(Response::Refused {
            reason: wire::Refusal::WorkspaceChanged,
        })
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    let read = target
        .client
        .relay("fs/readFile".into(), json!({"path": "file:///target/a"}))
        .await;
    assert!(read.get("error").is_some());
    assert!(read.pointer("/error/data").is_none());
    assert!(ledger.uncertain_effect().is_none());
    let write = target
        .client
        .relay(
            "fs/writeFile".into(),
            json!({"path": "file:///target/a", "dataBase64": ""}),
        )
        .await;
    assert_eq!(
        write.pointer("/error/data/outcome"),
        Some(&json!("unknown"))
    );
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("fs/writeFile"));
}

#[tokio::test]
async fn an_unknown_process_write_or_signal_parks_the_session() {
    for method in ["process/write", "process/signal"] {
        let target = Target::new(Arc::new(|_| {
            Reply::Answer(Response::Operation {
                outcome: Outcome::Unknown,
            })
        }));
        let ledger = ledger(&target.client, None);
        ledger.begin_turn("turn-1");
        let reply = target
            .client
            .relay(method.into(), json!({"processId": "p1"}))
            .await;
        assert_eq!(
            reply.pointer("/error/data/outcome"),
            Some(&json!("unknown"))
        );
        assert_eq!(ledger.uncertain_effect().as_deref(), Some(method));
    }
}

#[test]
fn an_answered_write_or_signal_the_unit_never_read_is_unknown() {
    use super::ledger::unread_effects;
    for method in ["process/write", "process/signal"] {
        assert_eq!(
            unread_effects(method, false, &json!({"result": {}})),
            [method]
        );
    }
}

#[tokio::test]
async fn unconfirmed_process_termination_in_finish_turn_parks_durably_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.json");
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "process/start" => {
            let id = invocation.params["processId"].as_str().unwrap_or("");
            completed(json!({"result": {"processId": id}}))
        }
        "process/terminate" => completed(json!({"result": {}})),
        "process/read" => {
            completed(json!({"result": {"chunks": [], "exited": false, "closed": false}}))
        }
        _ => completed(json!({"result": {}})),
    }));
    let l = ledger(&target.client, Some(path.clone()));
    l.begin_turn("turn-1");
    let started = target
        .client
        .relay("process/start".into(), start("stuck-1"))
        .await;
    assert!(started.get("result").is_some());
    let finish = target
        .client
        .finish_turn("turn-1")
        .expect("running process triggers barrier");
    finish.await;
    assert_eq!(l.uncertain_effect().as_deref(), Some("process/terminate"));

    let reopened = Ledger::open("session", Some(path)).unwrap();
    assert_eq!(
        reopened.uncertain_effect().as_deref(),
        Some("process/terminate")
    );
    assert!(!reopened.unwritable());
}

#[tokio::test]
async fn unconfirmed_process_termination_in_settle_all_parks_durably_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.json");
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "process/start" => {
            let id = invocation.params["processId"].as_str().unwrap_or("");
            completed(json!({"result": {"processId": id}}))
        }
        "process/terminate" => completed(json!({"result": {}})),
        "process/read" => {
            completed(json!({"result": {"chunks": [], "exited": false, "closed": false}}))
        }
        _ => completed(json!({"result": {}})),
    }));
    let l = ledger(&target.client, Some(path.clone()));
    l.begin_turn("turn-1");
    let started = target
        .client
        .relay("process/start".into(), start("stuck-2"))
        .await;
    assert!(started.get("result").is_some());
    target.client.settle_all().await;
    assert_eq!(l.uncertain_effect().as_deref(), Some("process/terminate"));

    let reopened = Ledger::open("session", Some(path)).unwrap();
    assert_eq!(
        reopened.uncertain_effect().as_deref(),
        Some("process/terminate")
    );
}

#[tokio::test]
async fn background_cancel_turn_does_not_park_on_unconfirmed_termination() {
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "process/start" => {
            let id = invocation.params["processId"].as_str().unwrap_or("");
            completed(json!({"result": {"processId": id}}))
        }
        "process/terminate" => completed(json!({"result": {}})),
        "process/read" => {
            completed(json!({"result": {"chunks": [], "exited": false, "closed": false}}))
        }
        _ => completed(json!({"result": {}})),
    }));
    let l = ledger(&target.client, None);
    l.begin_turn("turn-1");
    let _ = target
        .client
        .relay("process/start".into(), start("slow-bg"))
        .await;
    target.client.cancel_turn();
    tokio::time::sleep(Duration::from_millis(5300)).await;
    assert!(l.uncertain_effect().is_none());
}

#[tokio::test]
async fn abrupt_worker_exit_with_live_process_parks_replacement_ledger_durably() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.json");
    let target = Target::new(processes());
    let l = ledger(&target.client, Some(path.clone()));
    l.begin_turn("turn-1");
    let started = target
        .client
        .relay("process/start".into(), start("live-before-sigkill"))
        .await;
    assert!(started.get("result").is_some());
    // Worker dies abruptly without running finish_turn/settle_all (SIGTERM/SIGKILL).
    drop(l);

    // Replacement worker reopening the same ledger sees the unconfirmed process obligation
    // and immediately latches a durable `process/terminate` uncertain_effect hold.
    let reopened = Ledger::open("session", Some(path.clone())).unwrap();
    assert_eq!(
        reopened.uncertain_effect().as_deref(),
        Some("process/terminate")
    );
    assert!(reopened.uncertain_sequence() > 0);
}

#[tokio::test]
async fn durable_process_obligation_survives_exited_until_closed_and_mints_fresh_sequence_on_second_crash()
 {
    let dir = tempfile::tempdir().unwrap();
    let ledger_path = dir.path().join("ledger.json");

    // Crash 1: leader exited=true / closed=false + process/exited MUST retain obligation!
    {
        let target = Target::new(Arc::new(|inv| match inv.method.as_str() {
            "process/start" => completed(json!({"result": {"processId": "p-tree"}})),
            "process/read" => {
                completed(json!({"result": {"chunks": [], "exited": true, "closed": false}}))
            }
            _ => completed(json!({"result": {}})),
        }));
        let l = ledger(&target.client, Some(ledger_path.clone()));
        l.begin_turn("turn-1");
        target
            .client
            .relay("process/start".into(), start("p-tree"))
            .await;
        target
            .client
            .relay(
                "process/read".into(),
                json!({"processId": "p-tree", "afterSeq": null, "maxBytes": 10, "waitMs": 0}),
            )
            .await;
        target
            .client
            .observe_event(&json!({"method": "process/exited", "params": {"processId": "p-tree"}}));
        // Abrupt crash without process/closed or closed=true!
    }

    let first_reopen = Ledger::open("session", Some(ledger_path.clone())).unwrap();
    assert_eq!(
        first_reopen.uncertain_effect().as_deref(),
        Some("process/terminate"),
        "exited=true/closed=false must NOT erase durable process obligation"
    );
    let seq1 = first_reopen.uncertain_sequence();
    assert!(seq1 > 0);
    drop(first_reopen);

    // Crash 2 (after owner acknowledged seq1 in Controller): starting a second process
    // and crashing again MUST mint a strictly greater sequence (seq2 > seq1)!
    {
        let target = Target::new(Arc::new(|inv| match inv.method.as_str() {
            "process/start" => completed(json!({"result": {"processId": "p-second"}})),
            _ => completed(json!({"result": {}})),
        }));
        let l = ledger(&target.client, Some(ledger_path.clone()));
        l.begin_turn("turn-2");
        target
            .client
            .relay("process/start".into(), start("p-second"))
            .await;
        // Abrupt crash 2!
    }
    let second_reopen = Ledger::open("session", Some(ledger_path.clone())).unwrap();
    assert_eq!(
        second_reopen.uncertain_effect().as_deref(),
        Some("process/terminate")
    );
    let seq2 = second_reopen.uncertain_sequence();
    assert!(
        seq2 > seq1,
        "second crash must mint a fresh sequence ({seq2} > {seq1}) even when method is unchanged"
    );
    drop(second_reopen);

    // Verified process/closed clears the durable obligation cleanly (no sequence bump on reopen).
    {
        let target = Target::new(Arc::new(|inv| match inv.method.as_str() {
            "process/start" => completed(json!({"result": {"processId": "p-clean"}})),
            _ => completed(json!({"result": {}})),
        }));
        let l = ledger(&target.client, Some(ledger_path.clone()));
        l.begin_turn("turn-3");
        target
            .client
            .relay("process/start".into(), start("p-clean"))
            .await;
        target.client.observe_event(
            &json!({"method": "process/closed", "params": {"processId": "p-clean"}}),
        );
    }
    let third_reopen = Ledger::open("session", Some(ledger_path)).unwrap();
    assert_eq!(
        third_reopen.uncertain_sequence(),
        seq2,
        "process/closed cleared obligation so clean reopen does not bump sequence"
    );
}

#[tokio::test]
async fn forgotten_process_read_never_erases_in_flight_start_obligation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.json");
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "process/start" => Reply::Hold,
        "process/read" => completed(json!({
            "error": {"code": -32001, "message": "unknown process id p-race"}
        })),
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, Some(path.clone()));
    ledger.begin_turn("turn-1");

    let client = Arc::clone(&target.client);
    let start_task =
        tokio::spawn(async move { client.relay("process/start".into(), start("p-race")).await });
    eventually(|| !target.held.lock().is_empty()).await;

    // Concurrent process/read arrives before process/start and gets genuine unknown-id.
    let read_reply = target
        .client
        .relay(
            "process/read".into(),
            json!({"processId": "p-race", "afterSeq": null, "maxBytes": 1, "waitMs": 0}),
        )
        .await;
    assert_eq!(read_reply["error"]["message"], "unknown process id p-race");
    // In-flight start MUST remain tracked in BOTH live map and durable file.
    assert_eq!(ledger.live(), ["p-race"]);

    // Now the start completes on the target.
    target.release(json!({"result": {"processId": "p-race"}}));
    let start_reply = start_task.await.unwrap();
    assert_eq!(start_reply["result"]["processId"], "p-race");
    assert_eq!(ledger.live(), ["p-race"]);

    // Reopening after crash proves the durable active_processes entry survived.
    let reopened = Ledger::open("session", Some(path)).unwrap();
    assert_eq!(
        reopened.uncertain_effect().as_deref(),
        Some("process/terminate")
    );
    assert!(reopened.uncertain_sequence() > 0);
}

#[tokio::test]
async fn leader_exit_keeps_tree_in_finish_turn_until_confirmed_closed() {
    let terminated = Arc::new(Mutex::new(false));
    let terminated_flag = Arc::clone(&terminated);
    let target = Target::new(Arc::new(move |invocation| {
        match invocation.method.as_str() {
            "process/start" => completed(json!({"result": {"processId": "p-desc"}})),
            "process/terminate" => {
                *terminated_flag.lock() = true;
                completed(json!({"result": {}}))
            }
            "process/read" => {
                let done = *terminated_flag.lock();
                completed(json!({"result": {"chunks": [], "exited": true, "closed": done}}))
            }
            _ => completed(json!({"result": {}})),
        }
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    target
        .client
        .relay("process/start".into(), start("p-desc"))
        .await;
    target
        .client
        .relay(
            "process/read".into(),
            json!({"processId": "p-desc", "afterSeq": null, "maxBytes": 1, "waitMs": 0}),
        )
        .await;
    // Leader exited (exited=true, closed=false), descendants still alive!
    assert_eq!(ledger.live(), ["p-desc"]);
    let settle = target
        .client
        .finish_turn("turn-1")
        .expect("finish_turn must settle unclosed process tree");
    tokio::time::timeout(Duration::from_secs(5), settle)
        .await
        .unwrap();
    assert!(*terminated.lock());
    assert!(ledger.live().is_empty());
}

#[tokio::test]
async fn process_closed_event_wakes_terminate_wait_without_recording_uncertainty() {
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "process/start" => completed(json!({"result": {"processId": "p-evt"}})),
        "process/terminate" => completed(json!({"result": {}})),
        "process/read" => Reply::Hold,
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    target
        .client
        .relay("process/start".into(), start("p-evt"))
        .await;

    let settle = target
        .client
        .finish_turn("turn-1")
        .expect("finish_turn barrier");
    let settle_task = tokio::spawn(settle);
    eventually(|| !target.held.lock().is_empty()).await;

    // Executor delivers process/closed while process/read is stalled.
    target
        .client
        .observe_event(&json!({"method": "process/closed", "params": {"processId": "p-evt"}}));
    tokio::time::timeout(Duration::from_secs(2), settle_task)
        .await
        .expect("process/closed must wake terminate immediately")
        .unwrap();
    assert!(ledger.live().is_empty());
    assert_eq!(ledger.uncertain_effect(), None);
}

#[tokio::test]
async fn reusing_closed_process_id_is_refused_so_stale_closure_cannot_erase_replacement() {
    let target = Target::new(processes());
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    let first = target
        .client
        .relay("process/start".into(), start("p-unique"))
        .await;
    assert_eq!(first["result"]["processId"], "p-unique");
    target
        .client
        .observe_event(&json!({"method": "process/closed", "params": {"processId": "p-unique"}}));
    assert!(ledger.live().is_empty());

    // Reusing the closed process ID in the same worker MUST be refused.
    let reused = target
        .client
        .relay("process/start".into(), start("p-unique"))
        .await;
    assert!(reused.get("error").is_some());
    assert!(ledger.live().is_empty());
}

#[tokio::test]
async fn sigterm_cutoff_persists_pending_filesystem_mutation_for_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.json");
    {
        let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
            "fs/writeFile" => Reply::Hold,
            _ => completed(json!({"result": {}})),
        }));
        let ledger = ledger(&target.client, Some(path.clone()));
        ledger.begin_turn("turn-1");
        let client = Arc::clone(&target.client);
        let _write_task = tokio::spawn(async move {
            client
                .relay(
                    "fs/writeFile".into(),
                    json!({"path": "/tmp/a", "contentBase64": ""}),
                )
                .await
        });
        eventually(|| !target.held.lock().is_empty()).await;
        // Disconnected SIGTERM cutoff with NO active process, only an in-flight fs/writeFile!
        assert!(ledger.mark_unresolved_effects_uncertain());
    }
    let reopened = Ledger::open("session", Some(path)).unwrap();
    assert_eq!(reopened.uncertain_effect().as_deref(), Some("request"));
    assert!(reopened.uncertain_sequence() > 0);
}

#[tokio::test]
async fn lost_mutation_records_uncertainty_before_finish_turn_wakes() {
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/writeFile" => Reply::Hold,
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, None);
    let observed = Arc::new(Mutex::new(Vec::<(String, u64)>::new()));
    let observed_ref = Arc::clone(&observed);
    ledger.set_observer(Box::new(move |method, seq| {
        observed_ref.lock().push((method.to_owned(), seq));
    }));
    ledger.begin_turn("turn-1");
    let client = Arc::clone(&target.client);
    let write_task = tokio::spawn(async move {
        client
            .relay(
                "fs/writeFile".into(),
                json!({"path": "/tmp/x", "contentBase64": ""}),
            )
            .await
    });
    eventually(|| !target.held.lock().is_empty()).await;
    let settle = target
        .client
        .finish_turn("turn-1")
        .expect("finish_turn must wait for admitted fs/writeFile");

    // Simulate lost reply after admission (`fail_pending` / `RuntimeReply` error).
    let req = target.held.lock().pop().unwrap();
    target.client.complete(RuntimeReply {
        session_id: req.session_id,
        worker_epoch: req.worker_epoch,
        request_id: req.request_id,
        scope: Scope::from_binding(&req.binding),
        response: MachineResponse::Closed,
    });
    settle.await;
    // The instant `finish_turn` resolves, uncertainty MUST already be recorded & observed!
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("fs/writeFile"));
    assert!(ledger.uncertain_sequence() > 0);
    assert_eq!(observed.lock().len(), 1);
    let _ = write_task.await.unwrap();
}

#[tokio::test]
async fn finish_turn_waits_for_in_flight_result_carrying_process_read_after_process_closed() {
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "process/start" => completed(json!({"result": {"processId": "p-read"}})),
        "process/read" => Reply::Hold,
        _ => completed(json!({"result": {}})),
    }));
    let ledger = ledger(&target.client, None);
    ledger.begin_turn("turn-1");
    target
        .client
        .relay("process/start".into(), start("p-read"))
        .await;
    let client = Arc::clone(&target.client);
    let read_task = tokio::spawn(async move {
        client
            .relay(
                "process/read".into(),
                json!({"processId": "p-read", "afterSeq": null, "maxBytes": 64, "waitMs": 0}),
            )
            .await
    });
    eventually(|| !target.held.lock().is_empty()).await;
    target
        .client
        .observe_event(&json!({"method": "process/closed", "params": {"processId": "p-read"}}));
    let settle = target
        .client
        .finish_turn("turn-1")
        .expect("finish_turn must wait for in-flight result-bearing process/read");
    let req = target.held.lock().pop().unwrap();
    target.client.complete(RuntimeReply {
        session_id: req.session_id,
        worker_epoch: req.worker_epoch,
        request_id: req.request_id,
        scope: Scope::from_binding(&req.binding),
        response: MachineResponse::Closed,
    });
    settle.await;
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("process/read"));
    assert!(ledger.uncertain_sequence() > 0);
    let _ = read_task.await.unwrap();
}

/// A target whose processes never confirm their end.
fn unconfirmed_ends() -> Handler {
    Arc::new(|invocation| match invocation.method.as_str() {
        "process/start" => {
            let id = invocation.params["processId"].as_str().unwrap_or("");
            completed(json!({"result": {"processId": id}}))
        }
        "process/read" => {
            completed(json!({"result": {"chunks": [], "exited": false, "closed": false}}))
        }
        _ => completed(json!({"result": {}})),
    })
}

#[tokio::test]
async fn an_unconfirmed_server_end_in_teardown_does_not_park_the_session() {
    // 2026-11-25 (OVH): every worker SIGTERM whose MCP server end was not
    // confirmed within the bound parked its session with `process/terminate`:
    // the server was ended again, parking, with the turn processes.
    let target = Target::new(unconfirmed_ends());
    let l = ledger(&target.client, None);
    l.record_server("cowboy-worker-mcp-1").unwrap();
    l.server_started("cowboy-worker-mcp-1", true);
    target.client.settle_all().await;
    assert!(l.uncertain_effect().is_none());
    // Its end is still requested a second time, without parking.
    let terminations = target
        .log()
        .iter()
        .filter(|entry| entry.starts_with("process/terminate "))
        .count();
    assert_eq!(terminations, 2);
    // The SIGTERM cutoff ignores it too.
    assert!(!l.mark_unresolved_effects_uncertain());
    assert!(l.uncertain_effect().is_none());
}

#[tokio::test]
async fn an_unconfirmed_turn_process_beside_a_server_still_parks_teardown() {
    let target = Target::new(unconfirmed_ends());
    let l = ledger(&target.client, None);
    l.record_server("cowboy-worker-mcp-1").unwrap();
    l.server_started("cowboy-worker-mcp-1", true);
    l.begin_turn("turn-1");
    let started = target
        .client
        .relay("process/start".into(), start("stuck-turn-process"))
        .await;
    assert!(started.get("result").is_some());
    target.client.settle_all().await;
    assert_eq!(l.uncertain_effect().as_deref(), Some("process/terminate"));
}

#[tokio::test]
async fn ledger_grants_no_out_of_turn_filesystem_removal_bypass() {
    let target = Target::new(Arc::new(|invocation| match invocation.method.as_str() {
        "fs/writeFile" => completed(json!({"result": {}})),
        "fs/remove" => completed(json!({"result": {}})),
        _ => completed(json!({"result": {}})),
    }));
    let l = ledger(&target.client, None);
    l.begin_turn("turn-1");
    let staged = format!(
        "file:///home/draven/.cache/cowboy/file-buffers/payload-{}.json",
        "a".repeat(32)
    );
    let wrote = target
        .client
        .relay(
            "fs/writeFile".into(),
            json!({"path": staged, "dataBase64": "e30="}),
        )
        .await;
    assert!(wrote.get("result").is_some());
    // Once turn-1 is fenced, ordinary fs/remove is refused and no bypass exists (H1).
    target.client.cancel_turn();
    let removed = target
        .client
        .relay("fs/remove".into(), json!({"path": staged, "force": true}))
        .await;
    assert!(removed.get("error").is_some());
    assert!(l.uncertain_effect().is_none());
}

#[tokio::test]
async fn rotating_active_turn_nonce_invalidates_prior_attempt_post_edit_waiters() {
    let target = Target::new(Arc::new(|_| completed(json!({"result": {}}))));
    let (conn_id, mut rx) = target.client.attach_post_edit_broker(9);
    target
        .client
        .set_active_turn_nonce(Some(("turn-1".into(), "nonce-attempt-1".into())));
    let client = Arc::clone(&target.client);
    let inspect_task = tokio::spawn(async move {
        client
            .inspect_post_edit(
                0,
                &json!({
                    "path": "src/lib.rs",
                    "expectedLfSha256": "a".repeat(64),
                    "turnNonce": "nonce-attempt-1",
                    "timeoutMs": 3000,
                }),
            )
            .await
    });
    let mut queued = rx.recv().await.expect("queued post-edit request");
    // Rotate nonce for an ACP retry on the exact same turn_id (M-Worker-5).
    target
        .client
        .set_active_turn_nonce(Some(("turn-1".into(), "nonce-attempt-2".into())));
    // Earlier attempt's queued request must be dropped at prepare_post_edit_send!
    assert!(!target.client.prepare_post_edit_send(&mut queued, conn_id));
    let outcome = inspect_task.await.unwrap();
    assert_eq!(outcome["state"], "refused");
}

#[tokio::test]
async fn rotating_active_turn_nonce_discards_settled_reply_before_projection() {
    let target = Target::new(Arc::new(|_| completed(json!({"result": {}}))));
    let (conn_id, mut rx) = target.client.attach_post_edit_broker(9);
    target
        .client
        .set_active_turn_nonce(Some(("turn-1".into(), "nonce-1".into())));
    let client = Arc::clone(&target.client);
    let inspect_task = tokio::spawn(async move {
        client
            .inspect_post_edit(
                0,
                &json!({
                    "path": "src/lib.rs",
                    "expectedLfSha256": "a".repeat(64),
                    "turnNonce": "nonce-1",
                    "timeoutMs": 3000,
                }),
            )
            .await
    });
    let queued = rx.recv().await.expect("queued post-edit request");
    let crate::runtime_wire::Frame::PostEditInspectRequest { request } = queued.frame else {
        panic!("expected post edit request");
    };
    // Deliver reply and immediately rotate nonce before inspect_task can project it (M6).
    target.client.complete_post_edit_reply(
        crate::post_edit_inspect::PostEditInspectReply {
            session_id: request.session_id,
            worker_epoch: request.worker_epoch,
            request_id: request.request_id,
            scope: request.scope,
            outcome: crate::post_edit_inspect::PostEditInspectOutcome::Projected {
                payload: json!({"deliver": true, "files": [], "truncated": false}),
                matches_expected: true,
            },
        },
        conn_id,
    );
    target
        .client
        .set_active_turn_nonce(Some(("turn-1".into(), "nonce-2".into())));
    let outcome = inspect_task.await.unwrap();
    assert_eq!(outcome["state"], "refused");
    assert_eq!(outcome["reason"], "unavailable");
}

#[test]
fn classify_request_line_parses_uri_path_and_rejects_absolute_form_diagnostics() {
    assert_eq!(
        classify_request_line(b"GET / HTTP/1.1\r\n"),
        RequestLineRoute::Mutation
    );
    assert_eq!(
        classify_request_line(b"GET /?v=1 HTTP/1.1\r\n"),
        RequestLineRoute::Mutation
    );
    assert_eq!(
        classify_request_line(b"GET /diagnostics HTTP/1.1\r\n"),
        RequestLineRoute::Diagnostics
    );
    assert_eq!(
        classify_request_line(b"GET http://127.0.0.1/diagnostics HTTP/1.1\r\n"),
        RequestLineRoute::Diagnostics
    );
    assert_eq!(
        classify_request_line(b"GET http://127.0.0.1/diagnostics?foo=bar HTTP/1.1\r\n"),
        RequestLineRoute::Diagnostics
    );
    assert_eq!(
        classify_request_line(b"GET\t/diagnostics\tHTTP/1.1\r\n"),
        RequestLineRoute::Invalid
    );
    assert_eq!(
        classify_request_line(b"POST / HTTP/1.1\r\n"),
        RequestLineRoute::Invalid
    );
    assert_eq!(
        classify_request_line(b"GET /bad target HTTP/1.1\r\n"),
        RequestLineRoute::Invalid
    );
    assert_eq!(
        classify_request_line(b"GET / HTTP/1.0\r\n"),
        RequestLineRoute::Invalid
    );
    assert_eq!(
        classify_request_line(b"GET / HTTP/1.9\r\n"),
        RequestLineRoute::Invalid
    );
    assert_eq!(
        classify_request_line(b"GET / HTTP/1.foo\r\n"),
        RequestLineRoute::Invalid
    );
    assert_eq!(
        classify_request_line(b"GET / HTTP/2.0\r\n"),
        RequestLineRoute::Invalid
    );
}

#[tokio::test]
async fn malformed_http_version_cannot_occupy_mutation_reserve_slot() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let fixture = UnixFixture::new();
    let sock_path = &fixture.socket;
    // Occupy all 3 general pre-auth slots with silent stalled peers.
    let mut stalled = Vec::new();
    for _ in 0..3 {
        stalled.push(tokio::net::UnixStream::connect(sock_path).await.unwrap());
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    // A 4th peer sending a malformed HTTP version must receive 400 and never
    // hold the 4th mutation-only pre-auth reserve slot (R5-M3).
    let mut bad = tokio::net::UnixStream::connect(sock_path).await.unwrap();
    bad.write_all(b"GET / HTTP/1.foo\r\n").await.unwrap();
    let mut reply = vec![0_u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(2), bad.read(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert!(
        std::str::from_utf8(&reply[..n])
            .unwrap_or("")
            .starts_with("HTTP/1.1 400"),
        "expected 400 response, got {:?}",
        String::from_utf8_lossy(&reply[..n])
    );
    // A real mutation client must still succeed through the 4th reserve slot.
    let _valid = tokio::time::timeout(Duration::from_secs(3), fixture.initialized())
        .await
        .expect("mutation client must not be starved by malformed HTTP version");
    drop(stalled);
}

#[tokio::test]
async fn uncertain_effect_control_request_succeeds_under_16_slot_saturation() {
    let inner = processes();
    let fixture = UnixFixture::with(Arc::new(move |invocation| {
        if invocation.method == "fs/getMetadata" {
            Reply::Hold
        } else {
            inner(invocation)
        }
    }));
    let ledger_dir = tempfile::tempdir().unwrap();
    let ledger_path = ledger_dir.path().join("ledger.json");
    let ledger = ledger(&fixture.target.client, Some(ledger_path.clone()));
    let seq_before = ledger.uncertain_sequence();
    ledger.begin_turn("turn-1");
    let mut socket = fixture.initialized().await;
    // Saturate all 16 forwarded request slots.
    for id in 1..=16 {
        socket
            .send(Message::Text(
                json!({"id": id, "method": "fs/getMetadata", "params": {"path": "file:///target/a"}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    }
    eventually(|| fixture.target.held.lock().len() == 16).await;

    // A 17th forwarded request must be refused at capacity.
    socket
        .send(Message::Text(
            json!({"id": 17, "method": "fs/getMetadata", "params": {"path": "file:///target/a"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let refused_raw = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let refused: Value = serde_json::from_str(refused_raw.to_text().unwrap()).unwrap();
    assert_eq!(refused["id"], json!(17));
    assert_eq!(refused["error"]["code"], json!(-32000));

    // A worker-local uncertainty report must still be admitted through the independent
    // control lane and durably fence the ledger while all 16 forwarded slots stay full (R20-M3).
    socket
        .send(Message::Text(
            json!({"id": 18, "method": "cowboy/uncertainEffect", "params": {"method": "file publish"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let ack_raw = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ack: Value = serde_json::from_str(ack_raw.to_text().unwrap()).unwrap();
    assert_eq!(ack["id"], json!(18));
    assert!(
        ack.get("error").is_none(),
        "control request failed under saturation: {ack}"
    );
    assert_eq!(ledger.uncertain_effect().as_deref(), Some("file publish"));
    // Reopen from disk while all 16 forwarded requests remain held to prove
    // durable persistence across worker restarts (R21-L1).
    assert_eq!(fixture.target.held.lock().len(), 16);
    let reopened = Ledger::open("session", Some(ledger_path)).unwrap();
    assert_eq!(reopened.uncertain_effect().as_deref(), Some("file publish"));
    assert!(
        reopened.uncertain_sequence() > seq_before,
        "durable uncertainty sequence must advance"
    );

    fixture.target.release(json!({"result": {}}));
}

#[tokio::test]
async fn drain_acknowledgement_without_ledger_is_local_under_16_slot_saturation() {
    let inner = processes();
    let fixture = UnixFixture::with(Arc::new(move |invocation| {
        if invocation.method == "fs/getMetadata" {
            Reply::Hold
        } else {
            inner(invocation)
        }
    }));
    // Deliberately attach NO ledger on this connection (R21-M1).
    let mut socket = fixture.initialized().await;
    for id in 1..=16 {
        socket
            .send(Message::Text(
                json!({"id": id, "method": "fs/getMetadata", "params": {"path": "file:///target/a"}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    }
    eventually(|| fixture.target.held.lock().len() == 16).await;

    // Sending cowboy/drained while saturated and ledgerless must be answered locally
    // without forwarding a 17th request to the target.
    socket
        .send(Message::Text(
            json!({"id": 17, "method": "cowboy/drained", "params": {}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let ack_raw = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ack: Value = serde_json::from_str(ack_raw.to_text().unwrap()).unwrap();
    assert_eq!(ack["id"], json!(17));
    assert_eq!(ack["result"], json!({}));
    assert_eq!(fixture.target.held.lock().len(), 16);
    assert!(
        !fixture
            .target
            .log()
            .iter()
            .any(|entry| entry.contains("cowboy/drained")),
        "cowboy/drained must never be forwarded to target"
    );

    fixture.target.release(json!({"result": {}}));
}

impl Target {
    fn new_with_init(initialization: Value, handler: Handler) -> Self {
        let (notify, mut notified) = mpsc::unbounded_channel();
        let client = Client::new("session".into(), "worker".into(), binding(), notify);
        let log = Arc::<Mutex<Vec<String>>>::default();
        let held = Arc::<Mutex<Vec<RuntimeRequest>>>::default();
        let (relay, relay_log, relay_held) =
            (Arc::clone(&client), Arc::clone(&log), Arc::clone(&held));
        let task = tokio::spawn(async move {
            while notified.recv().await.is_some() {
                for frame in relay.frames(false) {
                    relay.sending(&frame);
                    let Frame::ExecutionRequest { request } = frame else {
                        continue;
                    };
                    let request = *request;
                    match &request.command {
                        Command::Describe => {
                            let response = Response::Ready {
                                scope: Scope::from_binding(&binding()),
                                initialization: initialization.clone(),
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
                            std::thread::sleep(Duration::from_millis(20));
                            let response = Response::Operation {
                                outcome: Outcome::Pending,
                            };
                            answer(&relay, request, response);
                        }
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
}

fn user_mcp_init() -> Value {
    json!({"sessionId": "native", USER_MCP_INITIALIZATION_KEY: {"schema": 1}})
}

fn push_mcp_line(output: &Mutex<BTreeMap<String, Vec<Value>>>, id: &str, msg: Value) {
    use base64::Engine as _;
    let mut map = output.lock();
    let list = map.entry(id.to_owned()).or_default();
    let seq = (list.len() as u64) + 1;
    let line = format!("{msg}\n");
    list.push(json!({
        "seq": seq,
        "stream": "stdout",
        "chunk": base64::engine::general_purpose::STANDARD.encode(line.as_bytes()),
    }));
}

#[tokio::test]
async fn r2_unit_isolation_blocks_keeper_methods_preserves_memory_and_hides_user_prefix() {
    let target = Target::new_with_init(
        user_mcp_init(),
        Arc::new(|_| completed(json!({"result": {}}))),
    );
    let ledger = ledger(&target.client, None);
    ledger.set_readiness(false);
    ledger.begin_turn("t1");

    // 1. Unit cannot invoke cowboy/listUserMcp, startUserMcp, stopUserMcp, or unknown cowboy/*.
    for method in [
        USER_MCP_LIST_METHOD,
        USER_MCP_START_METHOD,
        USER_MCP_STOP_METHOD,
        "cowboy/unknownInternal",
    ] {
        let reply = target.client.relay(method.into(), json!({})).await;
        assert_eq!(
            reply.pointer("/error/message").and_then(Value::as_str),
            Some("reserved cowboy method"),
            "{method} must be refused locally"
        );
    }
    assert!(
        target.log().is_empty(),
        "reserved cowboy/* methods must never reach the target"
    );

    // 2. cowboy/memory STILL reaches the local memory handler (not blocked by catch-all!).
    let mem = target
        .client
        .relay(
            memory_bridge::MEMORY_REQUEST.into(),
            json!({"path": "/v1/context"}),
        )
        .await;
    assert_eq!(
        mem.pointer("/error/message").and_then(Value::as_str),
        Some("memory is not available in this session"),
        "cowboy/memory must hit its local branch, not the reserved cowboy/* catch-all"
    );

    // 3. cowboy-worker-mcp-user-* inherits SERVER_PREFIX isolation in worker_event() and admit().
    let user_pid = format!("{USER_MCP_PROCESS_PREFIX}0123456789abcdef0123456789abcdef");
    assert!(ledger::worker_event(&json!({
        "method": "process/output",
        "params": {"processId": user_pid}
    })));
    for op in ["process/read", "process/write", "process/terminate"] {
        let reply = target
            .client
            .relay(op.into(), json!({"processId": user_pid}))
            .await;
        assert!(
            reply.get("error").is_some(),
            "unit {op} on user server must be refused"
        );
    }
    assert!(target.log().is_empty());

    // 4. cowboy/startUserMcp is classified effectful().
    assert!(ledger::effectful(USER_MCP_START_METHOD));
}

#[tokio::test]
async fn r2_phase2_is_demand_triggered_filters_collisions_and_handles_hanging_server() {
    use base64::Engine as _;
    let output: Arc<Mutex<BTreeMap<String, Vec<Value>>>> = Arc::default();
    let output_h = Arc::clone(&output);
    let started_names: Arc<Mutex<std::collections::HashMap<String, String>>> = Arc::default();
    let started_h = Arc::clone(&started_names);

    let target = Target::new_with_init(
        user_mcp_init(),
        Arc::new(move |inv| match inv.method.as_str() {
            USER_MCP_STOP_METHOD => completed(json!({"result": {"stopped": 0, "failed": 0}})),
            USER_MCP_LIST_METHOD => completed(json!({
                "result": {
                    "servers": [
                        {"name": "chrome-dev", "scope": "user", "fingerprint": "a".repeat(64)},
                        {"name": "hanging-srv", "scope": "user", "fingerprint": "b".repeat(64)},
                        {"name": "suger", "scope": "user", "fingerprint": "c".repeat(64)},
                        {"name": "matrix", "scope": "user", "fingerprint": "d".repeat(64)},
                        {"name": "cowboy-evil", "scope": "user", "fingerprint": "e".repeat(64)}
                    ],
                    "omitted": [
                        {"name": "bad-env", "reason": "unresolved_variable"}
                    ]
                }
            })),
            USER_MCP_START_METHOD => {
                let name = inv.params["name"].as_str().unwrap().to_owned();
                let pid = inv.params["processId"].as_str().unwrap().to_owned();
                started_h.lock().insert(pid.clone(), name);
                completed(json!({"result": {"processId": pid}}))
            }
            "process/write" => {
                let pid = inv.params["processId"].as_str().unwrap().to_owned();
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(inv.params["chunk"].as_str().unwrap())
                    .unwrap();
                let msg: Value = serde_json::from_slice(&bytes).unwrap();
                let server_name = started_h.lock().get(&pid).cloned().unwrap_or_default();
                if server_name == "hanging-srv" {
                    // Never responds to initialize -> triggers Phase 2 timeout & cleanup!
                    return completed(json!({"result": {"status": "accepted"}}));
                }
                if let Some(req_id) = msg.get("id").and_then(Value::as_u64) {
                    let result = match msg["method"].as_str() {
                        Some("initialize") => json!({"instructions": "i".repeat(2000)}),
                        Some("tools/list") => {
                            json!({"tools": [{"name": "snap", "description": "s"}]})
                        }
                        _ => json!({}),
                    };
                    push_mcp_line(
                        &output_h,
                        &pid,
                        json!({"jsonrpc": "2.0", "id": req_id, "result": result}),
                    );
                }
                completed(json!({"result": {"status": "accepted"}}))
            }
            "process/read" => {
                let pid = inv.params["processId"].as_str().unwrap().to_owned();
                let after = inv.params["afterSeq"].as_u64().unwrap_or(0);
                let chunks: Vec<Value> = output_h
                    .lock()
                    .get(&pid)
                    .map(|l| {
                        l.iter()
                            .filter(|c| c["seq"].as_u64().unwrap() > after)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                completed(json!({"result": {"chunks": chunks, "exited": false, "closed": false}}))
            }
            _ => completed(json!({"result": {}})),
        }),
    );
    let ledger = ledger(&target.client, None);
    ledger.set_readiness(false);
    let broker = ExtensionBroker::with_test_timeouts(
        &target.client,
        ExtensionPlan {
            servers: vec![ServerPlan {
                name: "suger".into(),
                argv: vec!["suger".into()],
                required: false,
            }],
            memory_tools: Vec::new(),
            memory_server: "matrix".into(),
            guidance: Vec::new(),
        },
        Some(Duration::from_millis(200)),
        Some(Duration::from_millis(350)),
    );
    target.client.set_extensions(broker);

    // 1. Legacy 0.2.39 proxy (ackProtocol omitted): MUST NOT trigger Phase 2!
    let legacy = target
        .client
        .relay("cowboy/extensions".into(), json!({}))
        .await;
    assert!(legacy.get("result").is_some());
    assert!(
        !target
            .log()
            .iter()
            .any(|l| l.starts_with(USER_MCP_LIST_METHOD)),
        "0.2.39 without ackProtocol must skip Phase 2 completely"
    );

    // 2. 0.2.40 proxy (ackProtocol: 1): triggers Phase 2 once, returns chrome-dev_snap
    //    despite hanging-srv timing out, filters suger/matrix/cowboy-evil collisions,
    //    caps instructions to <=1024 chars, and cleans up hanging-srv via stopUserMcp.
    let v2 = target
        .client
        .relay("cowboy/extensions".into(), json!({"ackProtocol": 1}))
        .await;
    let tools = v2["result"]["tools"].as_array().unwrap();
    assert!(
        tools
            .iter()
            .any(|t| t["name"] == "chrome-dev_snap" && t["tool"] == "snap"),
        "expected mandatory <server>_ prefixed tool, got {tools:?}"
    );
    let instructions = v2["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("reserved_name_collision"));
    assert!(instructions.contains("unresolved_variable"));
    assert!(instructions.contains("init_timeout"));
    assert!(
        !instructions.contains(&"i".repeat(1200)),
        "instructions must be capped at 1024 chars"
    );

    tokio::time::sleep(Duration::from_millis(50)).await;
    let stop_count = target
        .log()
        .iter()
        .filter(|l| l.starts_with(USER_MCP_STOP_METHOD))
        .count();
    // 1 unconditional startup sweep + at least 1 straggler stop for hanging-srv
    assert!(
        stop_count >= 2,
        "expected startup sweep + hanging straggler stop, got {stop_count}"
    );
}

#[tokio::test]
async fn r2_pending_ack_happy_path_timeout_closed_socket_high1_and_notsent() {
    use base64::Engine as _;
    let temp = tempfile::tempdir().unwrap();
    let ledger_path = temp.path().join("ledger.json");
    let output: Arc<Mutex<BTreeMap<String, Vec<Value>>>> = Arc::default();
    let output_h = Arc::clone(&output);
    let mode = Arc::new(Mutex::new("ok".to_owned()));
    let mode_h = Arc::clone(&mode);
    let hold_call_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>> = Arc::default();
    let hold_call_h = Arc::clone(&hold_call_tx);

    let target = Target::new_with_init(
        user_mcp_init(),
        Arc::new(move |inv| match inv.method.as_str() {
            USER_MCP_STOP_METHOD => completed(json!({"result": {"stopped": 0, "failed": 0}})),
            USER_MCP_LIST_METHOD => completed(json!({
                "result": {
                    "servers": [{"name": "demo", "scope": "user", "fingerprint": "a".repeat(64)}],
                    "omitted": []
                }
            })),
            USER_MCP_START_METHOD => {
                completed(json!({"result": {"processId": inv.params["processId"]}}))
            }
            "process/write" => {
                let pid = inv.params["processId"].as_str().unwrap().to_owned();
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(inv.params["chunk"].as_str().unwrap())
                    .unwrap();
                let msg: Value = serde_json::from_slice(&bytes).unwrap();
                let cur_mode = mode_h.lock().clone();
                if msg["method"] == "tools/call" && cur_mode == "not_sent" {
                    return completed(json!({"result": {"status": "closed"}}));
                }
                if let Some(req_id) = msg.get("id").and_then(Value::as_u64) {
                    match msg["method"].as_str() {
                        Some("initialize") => push_mcp_line(
                            &output_h,
                            &pid,
                            json!({"jsonrpc": "2.0", "id": req_id, "result": {}}),
                        ),
                        Some("tools/list") => push_mcp_line(
                            &output_h,
                            &pid,
                            json!({"jsonrpc": "2.0", "id": req_id, "result": {"tools": [{"name": "act"}]}}),
                        ),
                        Some("tools/call") => {
                            if cur_mode == "hold_then_reply" {
                                let (tx, rx) = tokio::sync::oneshot::channel();
                                *hold_call_h.lock() = Some(tx);
                                let out_async = Arc::clone(&output_h);
                                let pid_async = pid.clone();
                                tokio::spawn(async move {
                                    let _ = rx.await;
                                    push_mcp_line(
                                        &out_async,
                                        &pid_async,
                                        json!({"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": "late-ok"}]}}),
                                    );
                                });
                            } else if cur_mode == "tool_error" {
                                push_mcp_line(
                                    &output_h,
                                    &pid,
                                    json!({"jsonrpc": "2.0", "id": req_id, "error": {"message": "bad args"}}),
                                );
                            } else {
                                push_mcp_line(
                                    &output_h,
                                    &pid,
                                    json!({"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": "ok"}]}}),
                                );
                            }
                        }
                        _ => {}
                    }
                }
                completed(json!({"result": {"status": "accepted"}}))
            }
            "process/read" => {
                let pid = inv.params["processId"].as_str().unwrap().to_owned();
                let after = inv.params["afterSeq"].as_u64().unwrap_or(0);
                let chunks: Vec<Value> = output_h
                    .lock()
                    .get(&pid)
                    .map(|l| {
                        l.iter()
                            .filter(|c| c["seq"].as_u64().unwrap() > after)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                completed(json!({"result": {"chunks": chunks, "exited": false, "closed": false}}))
            }
            _ => completed(json!({"result": {}})),
        }),
    );
    let ledger = Ledger::open("session", Some(ledger_path.clone())).unwrap();
    let observed = Arc::new(Mutex::new(Vec::<(String, u64)>::new()));
    let obs_ref = Arc::clone(&observed);
    ledger.set_observer(Box::new(move |method, seq| {
        obs_ref.lock().push((method.to_owned(), seq));
    }));
    ledger.set_readiness(false);
    target.client.set_ledger(Arc::clone(&ledger));

    let broker = ExtensionBroker::with_test_timeouts(
        &target.client,
        ExtensionPlan::default(),
        Some(Duration::from_millis(120)),
        Some(Duration::from_secs(2)),
    );
    target.client.set_extensions(Arc::clone(&broker));

    let listed = target
        .client
        .relay("cowboy/extensions".into(), json!({"ackProtocol": 1}))
        .await;
    assert_eq!(listed["result"]["tools"][0]["name"], "demo_act");

    // 0. Calling user-MCP tool without ackProtocol is refused.
    ledger.begin_turn("t1");
    let no_ack = target
        .client
        .relay(
            "cowboy/extensionCall".into(),
            json!({"name": "demo_act", "arguments": {}}),
        )
        .await;
    assert!(no_ack.get("error").is_some());

    // 1. Happy path: call returns Pending(call_id), sentinel persisted, ack clears sentinel & releases guard.
    let (reply, guard, ack) = target
        .client
        .relay_guarded(
            "cowboy/extensionCall".into(),
            json!({"name": "demo_act", "arguments": {}, "ackProtocol": 1}),
            10,
        )
        .await;
    assert!(reply.get("result").is_some());
    assert!(
        guard.is_none(),
        "SettlementGuard must be owned by PendingAck"
    );
    let relay::AckState::Pending(call_id) = ack else {
        panic!("expected Pending ack, got {ack:?}");
    };
    let on_disk: Value = serde_json::from_slice(&std::fs::read(&ledger_path).unwrap()).unwrap();
    assert!(
        on_disk["active_processes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some(&format!("cowboy-pending-call:{call_id}"))),
        "Schema-1 sentinel must be persisted while awaiting ack"
    );
    let ack_reply = target
        .client
        .relay(
            crate::account_sandbox::EXTENSION_CALL_ACK.into(),
            json!({"callId": call_id}),
        )
        .await;
    assert_eq!(ack_reply["result"]["acknowledged"], true);
    let on_disk: Value = serde_json::from_slice(&std::fs::read(&ledger_path).unwrap()).unwrap();
    assert!(on_disk.get("active_processes").is_none());
    assert!(observed.lock().is_empty());

    // 2. Failure::NotSent clears sentinel cleanly (Round 3 Medium 5).
    *mode.lock() = "not_sent".into();
    let (ns_reply, ns_guard, ns_ack) = target
        .client
        .relay_guarded(
            "cowboy/extensionCall".into(),
            json!({"name": "demo_act", "arguments": {}, "ackProtocol": 1}),
            10,
        )
        .await;
    assert!(ns_reply.get("error").is_some());
    assert_eq!(ns_ack, relay::AckState::None);
    drop(ns_guard);
    let on_disk: Value = serde_json::from_slice(&std::fs::read(&ledger_path).unwrap()).unwrap();
    assert!(
        on_disk.get("active_processes").is_none(),
        "NotSent must clear sentinel"
    );
    assert!(observed.lock().is_empty());

    // 3. Refused user-server tool error ALSO returns Pending(call_id); timeout converts once.
    *mode.lock() = "tool_error".into();
    let (err_reply, _, err_ack) = target
        .client
        .relay_guarded(
            "cowboy/extensionCall".into(),
            json!({"name": "demo_act", "arguments": {}, "ackProtocol": 1}),
            10,
        )
        .await;
    assert!(err_reply.get("error").is_some());
    let relay::AckState::Pending(err_call_id) = err_ack else {
        panic!("refused tool error must require ack");
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(observed.lock().len(), 1);
    assert_eq!(observed.lock()[0].0, "tools/call");
    // Late ack after timeout returns acknowledged: false.
    let late_ack = target
        .client
        .relay(
            crate::account_sandbox::EXTENSION_CALL_ACK.into(),
            json!({"callId": err_call_id}),
        )
        .await;
    assert_eq!(late_ack["result"]["acknowledged"], false);

    // 4. HIGH-1 REGRESSION: socket closes WHILE user-MCP tools/call is in flight!
    //    - Must NOT hang finish_turn
    //    - Must NOT deadlock on SettlementGuard drop
    //    - Must record EXACTLY ONE uncertain_effect ("tools/call") even through Relays::drop!
    observed.lock().clear();
    let seq_before = ledger.uncertain_sequence();
    ledger.try_begin_turn("t2", Some(seq_before)).unwrap();
    *mode.lock() = "hold_then_reply".into();

    let mut relays = relay::Relays::new(Arc::clone(&target.client), 42);
    let in_flight = target
        .client
        .relays
        .try_enter("cowboy/extensionCall")
        .unwrap();
    relays.spawn(
        json!(99),
        "cowboy/extensionCall".into(),
        json!({"name": "demo_act", "arguments": {}, "ackProtocol": 1}),
        in_flight,
    );
    // Wait until the call is in flight inside tools/call.
    for _ in 0..50 {
        if hold_call_tx.lock().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(hold_call_tx.lock().is_some(), "call should be in flight");

    // Socket closes BEFORE the server replies! Drop BrokerConnectionGuard order:
    // connection_closed(42) runs FIRST, then Relays drops!
    broker.connection_closed(42);
    drop(relays);

    // Now release the server reply so call_guarded completes after socket close.
    hold_call_tx.lock().take().unwrap().send(()).unwrap();

    // finish_turn MUST complete boundedly (no hang, no deadlock!).
    if let Some(finishing) = target.client.finish_turn("t2") {
        tokio::time::timeout(Duration::from_secs(2), finishing)
            .await
            .expect("finish_turn must terminate boundedly after mid-call socket close");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        observed.lock().len(),
        1,
        "High-1: must record EXACTLY ONE uncertain_effect, got {:?}",
        *observed.lock()
    );
    assert_eq!(observed.lock()[0].0, "tools/call");

    // 4b. Correlated cowboy/uncertainEffect({ callId }) settles PendingAck ONCE without double bump.
    observed.lock().clear();
    let seq_before_rep = ledger.uncertain_sequence();
    ledger.try_begin_turn("t3", Some(seq_before_rep)).unwrap();
    *mode.lock() = "ok".into();
    let (_, _, rep_ack) = target
        .client
        .relay_guarded(
            "cowboy/extensionCall".into(),
            json!({"name": "demo_act", "arguments": {}, "ackProtocol": 1}),
            11,
        )
        .await;
    let relay::AckState::Pending(rep_call_id) = rep_ack else {
        panic!("expected Pending ack");
    };
    let rep_res = target
        .client
        .relay(
            ledger::UNCERTAIN_REPORT.into(),
            json!({"method": "tools/call", "callId": rep_call_id}),
        )
        .await;
    assert_eq!(rep_res["result"], json!({}));
    assert_eq!(
        observed.lock().len(),
        1,
        "uncertainEffect(callId) must bump sequence ONCE"
    );
    assert_eq!(observed.lock()[0].0, "tools/call");

    // 4c. observe_event(process/closed) marks user server closed so next call fails locally without sentinel.
    let started_pid = target
        .log()
        .iter()
        .find_map(|entry| entry.strip_prefix(&format!("{USER_MCP_START_METHOD} ")))
        .unwrap()
        .to_owned();
    target.client.observe_event(&json!({
        "method": "process/closed",
        "params": {"processId": started_pid}
    }));
    let seq_closed = ledger.uncertain_sequence();
    ledger.try_begin_turn("t4", Some(seq_closed)).unwrap();
    let (closed_reply, closed_guard, closed_ack) = target
        .client
        .relay_guarded(
            "cowboy/extensionCall".into(),
            json!({"name": "demo_act", "arguments": {}, "ackProtocol": 1}),
            11,
        )
        .await;
    assert!(closed_reply.get("error").is_some());
    assert_eq!(closed_ack, relay::AckState::None);
    drop(closed_guard);
    let on_disk_closed: Value =
        serde_json::from_slice(&std::fs::read(&ledger_path).unwrap()).unwrap();
    assert!(on_disk_closed.get("active_processes").is_none());

    // 5. MutationInflightIds MAX_ACK = 8 lane is independent of control (4) and forwarded (16).
    let mut ids = MutationInflightIds::new();
    for i in 0..16 {
        assert!(
            ids.try_admit(&json!(format!("f{i}")), "process/read")
                .unwrap()
        );
    }
    for i in 0..4 {
        assert!(
            ids.try_admit(&json!(format!("c{i}")), "cowboy/drained")
                .unwrap()
        );
    }
    for i in 0..8 {
        assert!(
            ids.try_admit(
                &json!(format!("a{i}")),
                crate::account_sandbox::EXTENSION_CALL_ACK
            )
            .unwrap()
        );
    }
    assert!(
        !ids.try_admit(&json!("a8"), crate::account_sandbox::EXTENSION_CALL_ACK)
            .unwrap()
    );
}

#[tokio::test]
async fn r2_schema1_sentinel_reopen_twice_and_pre_floor_counterexample() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ledger.json");

    // Write a Schema-1 ledger populated with a pending user-MCP call sentinel.
    let populated = json!({
        "schema": 1,
        "session_id": "sess-r2",
        "uncertain_effect": null,
        "uncertain_sequence": 5,
        "active_processes": ["cowboy-pending-call:0123456789abcdef0123456789abcdef"]
    });
    std::fs::write(&path, serde_json::to_vec(&populated).unwrap()).unwrap();

    // 1. First reopen on R2 (>= f2aa86175 floor): consumes sentinel, records tools/call, bumps sequence.
    let first = Ledger::open("sess-r2", Some(path.clone())).unwrap();
    let effect1 = first
        .uncertain_effect()
        .expect("reopen must record uncertainty");
    let seq1 = first.uncertain_sequence();
    assert_eq!(effect1, "tools/call");
    assert!(seq1 > 5);
    assert!(
        first.try_begin_turn("t1", Some(5)).is_err(),
        "stale ack must stay fenced"
    );

    // 2. Second reopen (R4 §7.1 durability check): retains incremented sequence & fence.
    let second = Ledger::open("sess-r2", Some(path.clone())).unwrap();
    let effect2 = second
        .uncertain_effect()
        .expect("second reopen must keep uncertainty");
    let seq2 = second.uncertain_sequence();
    assert_eq!(effect2, "tools/call");
    assert_eq!(seq2, seq1);
    assert!(second.try_begin_turn("t2", Some(5)).is_err());

    // 3. Counterexample (High-2): a pre-d3b59b8ca worker Record (lacking `active_processes`)
    //    silently drops the sentinel on deserialize + re-serialize, proving why the
    //    `>= f2aa86175` rollback worker floor is mandatory before enabling R2.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct PreD3b59b8caRecord {
        schema: u16,
        session_id: String,
        uncertain_effect: Option<String>,
        #[serde(default)]
        uncertain_sequence: u64,
    }
    let legacy: PreD3b59b8caRecord = serde_json::from_value(populated).unwrap();
    let rewritten = serde_json::to_value(&legacy).unwrap();
    assert!(rewritten.get("active_processes").is_none());
    assert_eq!(rewritten["uncertain_sequence"], 5);
}
