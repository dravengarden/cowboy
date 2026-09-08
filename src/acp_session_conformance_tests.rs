//! Drive the production session handshake over real ACP byte streams, with a
//! deterministic in-memory peer. These are Cowboy protocol tests, not evidence
//! that a particular upstream CLI can resume its on-disk native history.

use super::*;
use crate::core::{Hub, SessionOrigin};
use serde_json::{Value, json};
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

const SESSION: &str = "reload-fixture";
const NATIVE: &str = "retained-native-thread";
const CWD: &str = "/fixture/workspace";

struct HubSink {
    hub: Hub,
    allocations: AtomicUsize,
    prompt_starts: Mutex<Vec<Option<String>>>,
    prompt_completions: Mutex<Vec<(Option<String>, String)>>,
}

impl AgentSink for HubSink {
    fn prompt_started(&self, _: &str, cmid: Option<&str>) {
        self.prompt_starts.lock().push(cmid.map(str::to_owned));
    }
    fn prompt_completed(&self, _: &str, cmid: Option<&str>, outcome: &str) {
        self.prompt_completions
            .lock()
            .push((cmid.map(str::to_owned), outcome.to_owned()));
    }
    fn set_status(&self, id: &str, status: Status, detail: Option<String>) {
        self.hub.set_status(id, status, detail);
    }
    fn push(&self, id: &str, event: Event) {
        self.hub.push(id, event);
    }
    fn push_tagged(&self, id: &str, event: Event, cmid: Option<String>) {
        self.hub.push_tagged(id, event, cmid);
    }
    fn set_config_options(&self, id: &str, options: Value) {
        self.hub.set_config_options(id, options);
    }
    fn set_agent_session_id(&self, id: &str, native: String) {
        self.allocations.fetch_add(1, Ordering::SeqCst);
        self.hub.set_agent_session_id(id, native);
    }
    fn set_session_usage(&self, id: &str, usage: SessionUsage) {
        self.hub.set_session_usage(id, usage);
    }
    fn schedule_wakeup(&self, _: &str, _: i64, _: String) {
        panic!("session startup must not schedule work");
    }
    fn session_is_system(&self, _: &str) -> bool {
        false
    }
    fn broadcast_error(&self, id: Option<String>, message: String) {
        self.hub.broadcast_error(id, message);
    }
    fn requeue_prompt(&self, _: &str, _: String, _: Vec<Value>, _: Option<String>) {
        panic!("session startup must not send or requeue a prompt");
    }
}

fn fixture_state(resume: bool) -> (Arc<ClientState>, Arc<HubSink>) {
    let hub = Hub::new();
    hub.create_local_session(
        SESSION.to_owned(),
        "fixture-provider".to_owned(),
        CWD.to_owned(),
        "keep title".to_owned(),
        SessionOrigin::Web,
        false,
    );
    if resume {
        hub.set_agent_session_id(SESSION, NATIVE.to_owned());
        for (kind, text) in [
            ("user_message_chunk", "saved user"),
            ("agent_message_chunk", "saved answer"),
        ] {
            hub.push(
                SESSION,
                Event::Update {
                    update: json!({
                        "sessionUpdate": kind, "content": {"type": "text", "text": text}
                    }),
                },
            );
        }
    }
    let sink = Arc::new(HubSink {
        hub,
        allocations: AtomicUsize::new(0),
        prompt_starts: Mutex::default(),
        prompt_completions: Mutex::default(),
    });
    let (prompt_cancellation, _) = watch::channel(0);
    let state = Arc::new(ClientState {
        sink: sink.clone(),
        session_id: SESSION.to_owned(),
        provider_id: "fixture-provider".to_owned(),
        service_auth_projected: true,
        pending: Mutex::new(HashMap::new()),
        prompt_lock: tokio::sync::Mutex::new(()),
        active_prompt: Mutex::new(None),
        prompt_cancellation,
        codex_full_access: AtomicBool::new(false),
        grok_permission_mode: Arc::new(Mutex::new(GrokPermissionMode::AlwaysApprove)),
        suppress_updates: AtomicBool::new(false),
        last_echoed_user_contents: Mutex::new(Vec::new()),
    });
    (state, sink)
}

async fn send_json(writer: &mut (impl AsyncWrite + Unpin), value: Value) {
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    writer.write_all(&bytes).await.unwrap();
    writer.flush().await.unwrap();
}

fn notification(native: &str, kind: &str, text: &str) -> Value {
    json!({"jsonrpc": "2.0", "method": "session/update", "params": {
        "sessionId": native, "update": {
            "sessionUpdate": kind, "content": {"type": "text", "text": text}
        }
    }})
}

struct Observation {
    result: Result<(), String>,
    state: Arc<ClientState>,
    sink: Arc<HubSink>,
    requests: Vec<Value>,
}

impl Observation {
    fn methods(&self) -> Vec<&str> {
        self.requests
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect()
    }

    fn texts(&self) -> Vec<String> {
        self.sink
            .hub
            .snapshot(SESSION)
            .unwrap()
            .0
            .iter()
            .filter_map(|event| {
                let Event::Update { update } = &event.event else {
                    return None;
                };
                update
                    .pointer("/content/text")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect()
    }

    fn assert_retained_identity(&self) {
        let meta = self.sink.hub.session_info(SESSION).unwrap().meta;
        assert_eq!(meta.agent_session_id.as_deref(), Some(NATIVE));
        assert_eq!(meta.cwd, CWD);
        assert_eq!(meta.title, "keep title");
        assert_eq!(self.sink.allocations.load(Ordering::SeqCst), 0);
        assert!(!self.state.suppress_updates.load(Ordering::SeqCst));
        for request in self.requests.iter().skip(1) {
            assert_eq!(request["params"]["sessionId"], NATIVE);
            assert_eq!(request["params"]["cwd"], CWD);
        }
    }
}

async fn run_peer(
    peer_io: tokio::io::DuplexStream,
    mut phase: watch::Receiver<StartupPhase>,
    mut observed_rx: watch::Receiver<usize>,
    command_tx: mpsc::UnboundedSender<AgentCommand>,
    capabilities: Value,
    reject: bool,
) -> Vec<Value> {
    let (peer_read, mut peer_write) = tokio::io::split(peer_io);
    let mut requests = Vec::new();
    let mut lines = BufReader::new(peer_read).lines();
    let mut commands = Some(command_tx);
    while let Some(line) = lines.next_line().await.unwrap() {
        let request: Value = serde_json::from_str(&line).unwrap();
        let method = request["method"].as_str().unwrap();
        let id = request["id"].clone();
        requests.push(request.clone());
        if method == "initialize" {
            send_json(
                &mut peer_write,
                json!({"jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": 1, "agentCapabilities": capabilities
                }}),
            )
            .await;
            continue;
        }
        assert!(
            matches!(method, "session/resume" | "session/load" | "session/new"),
            "startup unexpectedly requested {method}"
        );
        let replay_count = if method == "session/load" {
            send_json(
                &mut peer_write,
                notification(NATIVE, "user_message_chunk", "saved user"),
            )
            .await;
            send_json(
                &mut peer_write,
                notification(NATIVE, "agent_message_chunk", "saved answer"),
            )
            .await;
            // A protocol barrier, not a timing assumption: load is still
            // pending while both replay callbacks have actually executed.
            observed_rx.wait_for(|count| *count >= 2).await.unwrap();
            2
        } else {
            0
        };
        if reject {
            send_json(
                &mut peer_write,
                json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": -32603, "message": "fixture native history is unavailable"
                }}),
            )
            .await;
            continue;
        }
        let result = if method == "session/new" {
            json!({"sessionId": "new-native-thread"})
        } else {
            json!({})
        };
        send_json(
            &mut peer_write,
            json!({"jsonrpc": "2.0", "id": id, "result": result}),
        )
        .await;
        phase
            .wait_for(|value| *value == StartupPhase::Ready)
            .await
            .unwrap();
        send_json(
            &mut peer_write,
            notification(
                if method == "session/new" {
                    "new-native-thread"
                } else {
                    NATIVE
                },
                "agent_message_chunk",
                "after resume",
            ),
        )
        .await;
        observed_rx
            .wait_for(|count| *count > replay_count)
            .await
            .unwrap();
        drop(commands.take());
    }
    requests
}

async fn exercise(resume: bool, capabilities: Value, reject: bool) -> Observation {
    let (state, sink) = fixture_state(resume);
    let (client_io, peer_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let (startup, phase) = watch::channel(StartupPhase::Initialize);
    let (observed_tx, observed_rx) = watch::channel(0_usize);
    let notifications = state.clone();
    let main_state = state.clone();
    let client = Client
        .builder()
        .name("cowboy-session-conformance")
        .on_receive_notification(
            async move |notification: SessionNotification,
                        _: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                handle_session_notification(&notifications, &notification);
                observed_tx.send_modify(|count| *count += 1);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(
            ByteStreams::new(client_write.compat_write(), client_read.compat()),
            async move |cx: ConnectionTo<Agent>| {
                run_session(
                    &main_state,
                    cx,
                    resume.then(|| NATIVE.to_owned()),
                    PathBuf::from(CWD),
                    &mut command_rx,
                    "fixture-provider",
                    &startup,
                )
                .await
            },
        );
    let peer = run_peer(
        peer_io,
        phase,
        observed_rx,
        command_tx,
        capabilities,
        reject,
    );
    let (result, requests) =
        tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(client, peer) })
            .await
            .expect("bounded ACP session fixture");
    Observation {
        result: result.map_err(|error| error.to_string()),
        state,
        sink,
        requests,
    }
}

fn resume_and_load() -> Value {
    json!({"loadSession": true, "sessionCapabilities": {"resume": {}}})
}

fn fixture_options(model: &str, effort: &str) -> Value {
    json!([
        {"id":"model", "name":"Model", "type":"select", "currentValue":model,
         "options":[{"value":"old-model","name":"Old"},{"value":"saved-model","name":"Saved"}]},
        {"id":"reasoning_effort", "name":"Reasoning", "type":"select", "currentValue":effort,
         "options":[{"value":"low","name":"Low"},{"value":"high","name":"High"}]}
    ])
}

/// The peer withholds real ACP replies while a restored prompt is already
/// queued. A command ACK from the Machine must not let that prompt bypass the
/// provider's authoritative model + reasoning snapshot.
#[allow(clippy::too_many_lines)] // Keep the held-reply protocol transcript in one place.
async fn exercise_configured_resume(reject: bool, cancel: bool) {
    let (state, sink) = fixture_state(true);
    let (client_io, peer_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let (startup, mut phase) = watch::channel(StartupPhase::Initialize);
    let main_state = state.clone();
    let client = Client.builder().connect_with(
        ByteStreams::new(client_write.compat_write(), client_read.compat()),
        async move |cx: ConnectionTo<Agent>| {
            run_session(
                &main_state,
                cx,
                Some(NATIVE.to_owned()),
                PathBuf::from(CWD),
                &mut command_rx,
                "fixture-provider",
                &startup,
            )
            .await
        },
    );
    let peer = async {
        let (read, mut write) = tokio::io::split(peer_io);
        let mut lines = BufReader::new(read).lines();
        for method in ["initialize", "session/resume"] {
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], method);
            let result = if method == "initialize" {
                json!({"protocolVersion":1, "agentCapabilities":resume_and_load()})
            } else {
                assert_eq!(request["params"]["sessionId"], NATIVE);
                assert_eq!(request["params"]["cwd"], CWD);
                json!({"configOptions":fixture_options("old-model", "low")})
            };
            send_json(
                &mut write,
                json!({"jsonrpc":"2.0", "id":request["id"], "result":result}),
            )
            .await;
        }
        phase
            .wait_for(|phase| *phase == StartupPhase::Ready)
            .await
            .unwrap();
        for (id, value) in [("model", "saved-model"), ("reasoning_effort", "high")] {
            command_tx
                .send(AgentCommand::SetConfigOption {
                    config_id: id.to_owned(),
                    value: json!(value),
                })
                .unwrap();
        }
        let (done, mut completed) = oneshot::channel();
        command_tx
            .send(AgentCommand::Prompt(
                vec![
                    serde_json::from_value(json!({"type":"text", "text":"queued after reload"}))
                        .unwrap(),
                ],
                Some("restored-first-prompt".to_owned()),
                Some(done),
            ))
            .unwrap();
        for (index, id) in ["model", "reasoning_effort"].into_iter().enumerate() {
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "session/set_config_option");
            assert_eq!(request["params"]["sessionId"], NATIVE);
            assert_eq!(request["params"]["configId"], id);
            // Hold the authoritative reply. Neither the next mutation nor the
            // first prompt may appear on the wire during this window.
            assert!(
                sink.prompt_starts.lock().is_empty(),
                "telemetry must not count configuration wait as ACP execution"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(50), lines.next_line())
                    .await
                    .is_err()
            );
            if cancel && index == 0 {
                command_tx.send(AgentCommand::Cancel).unwrap();
                let cancellation: Value =
                    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(cancellation["method"], "session/cancel");
            }
            let response = if reject && index == 0 {
                json!({"jsonrpc":"2.0", "id":request["id"], "error":{
                    "code":-32603, "message":"fixture rejects restored model"
                }})
            } else {
                json!({"jsonrpc":"2.0", "id":request["id"], "result":{
                    "configOptions":fixture_options("saved-model", if index == 0 {"low"} else {"high"})
                }})
            };
            send_json(&mut write, response).await;
        }
        if reject || cancel {
            let error = (&mut completed).await.unwrap().unwrap_err();
            assert!(
                error.contains(if cancel {
                    "cancelled"
                } else {
                    "fixture rejects"
                }),
                "{error}"
            );
        } else {
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "session/prompt");
            assert_eq!(request["params"]["sessionId"], NATIVE);
            assert_eq!(
                *sink.prompt_starts.lock(),
                vec![Some("restored-first-prompt".to_owned())]
            );
            assert!(request["params"].get("traceparent").is_none());
            assert!(request["params"].get("trace").is_none());
            assert_eq!(
                sink.hub.persisted_config_options(SESSION),
                Some(fixture_options("saved-model", "high"))
            );
            send_json(
                &mut write,
                json!({"jsonrpc":"2.0", "id":request["id"], "result":{"stopReason":"end_turn"}}),
            )
            .await;
            assert!(completed.await.unwrap().is_ok());
        }
        drop(command_tx);
        // Closing the command loop must not release a cancelled/rejected turn.
        assert!(lines.next_line().await.unwrap().is_none());
    };
    let (result, ()) =
        tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(client, peer) })
            .await
            .unwrap();
    assert!(result.is_ok(), "{result:?}");
    let observation = Observation {
        result: Ok(()),
        state,
        sink,
        requests: vec![],
    };
    observation.assert_retained_identity();
    assert_eq!(&observation.texts()[..2], ["saved user", "saved answer"]);
    assert_eq!(
        observation.sink.prompt_starts.lock().len(),
        usize::from(!reject && !cancel)
    );
    assert_eq!(
        *observation.sink.prompt_completions.lock(),
        vec![(
            Some("restored-first-prompt".to_owned()),
            if cancel {
                "Cancelled"
            } else if reject {
                "Error"
            } else {
                "EndTurn"
            }
            .to_owned()
        )]
    );
}

#[tokio::test]
async fn resumed_prompt_waits_for_authoritative_config_replies() {
    exercise_configured_resume(false, false).await;
}

#[tokio::test]
async fn rejected_restore_blocks_the_queued_prompt() {
    exercise_configured_resume(true, false).await;
}

#[tokio::test]
async fn cancel_stays_responsive_while_configuration_is_pending() {
    exercise_configured_resume(false, true).await;
}

#[tokio::test]
async fn resume_uses_native_identity_without_load_or_new() {
    let observed = exercise(true, resume_and_load(), false).await;
    assert!(observed.result.is_ok(), "{:?}", observed.result);
    assert_eq!(observed.methods(), ["initialize", "session/resume"]);
    observed.assert_retained_identity();
    assert_eq!(
        observed.texts(),
        ["saved user", "saved answer", "after resume"]
    );
    assert_eq!(observed.sink.hub.status(SESSION), Some(Status::Running));
}

#[tokio::test]
async fn load_drops_replayed_history_but_keeps_new_notifications() {
    let observed = exercise(true, json!({"loadSession": true}), false).await;
    assert!(observed.result.is_ok(), "{:?}", observed.result);
    assert_eq!(observed.methods(), ["initialize", "session/load"]);
    observed.assert_retained_identity();
    assert_eq!(
        observed.texts(),
        ["saved user", "saved answer", "after resume"]
    );
}

#[tokio::test]
async fn rejected_resume_never_tries_load_or_creates_a_blank_session() {
    let observed = exercise(true, resume_and_load(), true).await;
    assert!(observed.result.is_err());
    assert_eq!(observed.methods(), ["initialize", "session/resume"]);
    observed.assert_retained_identity();
    assert_eq!(observed.texts(), ["saved user", "saved answer"]);
    assert_ne!(observed.sink.hub.status(SESSION), Some(Status::Running));
}

#[tokio::test]
async fn rejected_load_preserves_history_and_restores_notification_handling() {
    let observed = exercise(true, json!({"loadSession": true}), true).await;
    assert!(observed.result.is_err());
    assert_eq!(observed.methods(), ["initialize", "session/load"]);
    observed.assert_retained_identity();
    assert_eq!(observed.texts(), ["saved user", "saved answer"]);
    assert_ne!(observed.sink.hub.status(SESSION), Some(Status::Running));
}

#[tokio::test]
async fn unsupported_native_resume_fails_before_session_creation() {
    let observed = exercise(true, json!({}), false).await;
    assert!(observed.result.is_err());
    assert_eq!(observed.methods(), ["initialize"]);
    observed.assert_retained_identity();
    assert_eq!(observed.texts(), ["saved user", "saved answer"]);
}

#[tokio::test]
async fn genuinely_new_sessions_can_still_allocate_native_identity() {
    let observed = exercise(false, json!({}), false).await;
    assert!(observed.result.is_ok(), "{:?}", observed.result);
    assert_eq!(observed.methods(), ["initialize", "session/new"]);
    assert_eq!(observed.sink.allocations.load(Ordering::SeqCst), 1);
    assert_eq!(
        observed
            .sink
            .hub
            .session_info(SESSION)
            .unwrap()
            .meta
            .agent_session_id
            .as_deref(),
        Some("new-native-thread")
    );
}
