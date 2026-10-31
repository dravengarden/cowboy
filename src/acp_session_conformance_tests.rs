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
const FORKED: &str = "forked-native-thread";

struct HubSink {
    hub: Hub,
    managed_read_only: AtomicBool,
    allocations: AtomicUsize,
    prompt_starts: Mutex<Vec<Option<String>>>,
    prompt_completions: Mutex<Vec<(Option<String>, String)>>,
    native_cycles: Mutex<Vec<Option<String>>>,
}

impl AgentSink for HubSink {
    fn delivery_target_is_current(
        &self,
        session: &str,
        target: &crate::send_delivery::DeliveryTarget,
    ) -> bool {
        let named = self.native_cycles.lock().last().cloned().flatten();
        session == SESSION
            && target.worker_epoch == "fixture-worker"
            && (target.turn_id == "fixture-turn" || named.as_deref() == Some(&target.turn_id))
            && self.hub.status(session) == Some(Status::Busy)
    }
    fn set_native_cycle(&self, _: &str, cycle_id: Option<&str>) {
        self.native_cycles.lock().push(cycle_id.map(str::to_owned));
    }
    fn session_is_managed_read_only(&self, _: &str) -> bool {
        self.managed_read_only.load(Ordering::SeqCst)
    }
    fn managed_config_option_allowed(&self, _: &str, config_id: &str) -> bool {
        config_id == "model"
    }
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
    fn set_background_tasks(&self, id: &str, count: u32) {
        self.hub.set_background_tasks(id, count);
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
        managed_read_only: AtomicBool::new(false),
        allocations: AtomicUsize::new(0),
        prompt_starts: Mutex::default(),
        prompt_completions: Mutex::default(),
        native_cycles: Mutex::default(),
    });
    let (prompt_cancellation, _) = watch::channel(0);
    let state = Arc::new(ClientState {
        sink: sink.clone(),
        session_id: SESSION.to_owned(),
        provider_id: "fixture-provider".to_owned(),
        service_auth_projected: true,
        pending: Mutex::new(HashMap::new()),
        elicitations: Mutex::new(HashSet::new()),
        url_elicitations: Mutex::new(HashMap::new()),
        prompt_lock: tokio::sync::Mutex::new(()),
        steering_gate: tokio::sync::Mutex::new(()),
        active_prompt: Mutex::new(None),
        activity: Mutex::new(SessionActivity::default()),
        native_cycle: Mutex::new(None),
        prompt_cancellation,
        codex_full_access: AtomicBool::new(false),
        grok_permission_mode: Arc::new(Mutex::new(GrokPermissionMode::AlwaysApprove)),
        suppress_updates: AtomicBool::new(false),
        last_echoed_user_contents: Mutex::new(Vec::new()),
        last_progress: Mutex::new(std::time::Instant::now()),
        open_tools: Mutex::new(std::collections::HashSet::new()),
        cgroup: None,
        backoff: Mutex::new(None),
        model_label: Mutex::new(None),
        subagents: Mutex::new(subagents::Subagents::default()),
        model_usage: Mutex::new(model_usage::ModelUsage::default()),
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
    rejection: Option<Value>,
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
            matches!(
                method,
                "session/resume" | "session/load" | "session/new" | "session/fork"
            ),
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
        if let Some(error) = &rejection {
            send_json(
                &mut peer_write,
                json!({"jsonrpc": "2.0", "id": id, "error": error}),
            )
            .await;
            continue;
        }
        let result = match method {
            "session/new" => json!({"sessionId": "new-native-thread"}),
            "session/fork" => json!({"sessionId": FORKED}),
            _ => json!({}),
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
                match method {
                    "session/new" => "new-native-thread",
                    "session/fork" => FORKED,
                    _ => NATIVE,
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
    exercise_start(
        resume.then(|| NATIVE.to_owned()).into(),
        capabilities,
        reject,
    )
    .await
}

fn native_history_unavailable() -> Value {
    json!({"code": -32603, "message": "fixture native history is unavailable"})
}

async fn exercise_start(start: NativeStart, capabilities: Value, reject: bool) -> Observation {
    exercise_rejected(start, capabilities, reject.then(native_history_unavailable)).await
}

async fn exercise_rejected(
    start: NativeStart,
    capabilities: Value,
    rejection: Option<Value>,
) -> Observation {
    let (state, sink) = fixture_state(matches!(start, NativeStart::Resume(_)));
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
                    start,
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
        rejection,
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

fn native_fork(message_id: Option<&str>) -> NativeStart {
    NativeStart::Fork(crate::runtime_wire::NativeFork {
        agent_session_id: NATIVE.to_owned(),
        message_id: message_id.map(str::to_owned),
    })
}

#[tokio::test]
async fn a_fork_copies_the_source_thread_at_its_message_into_the_destination_cwd() {
    let observed = exercise_start(
        native_fork(Some("msg-1")),
        json!({"sessionCapabilities": {"resume": {}, "fork": {}}}),
        false,
    )
    .await;
    observed.result.as_ref().expect("fork starts the session");
    assert_eq!(observed.methods(), ["initialize", "session/fork"]);
    let fork = &observed.requests[1]["params"];
    assert_eq!(fork["sessionId"], NATIVE);
    assert_eq!(fork["cwd"], CWD);
    assert_eq!(
        fork["_meta"]["jetbrains"]["air"]["fork"],
        json!({"version": 1, "messageId": "msg-1"})
    );
    let meta = observed.sink.hub.session_info(SESSION).unwrap().meta;
    assert_eq!(meta.agent_session_id.as_deref(), Some(FORKED));
    assert_eq!(observed.texts(), ["after resume"]);
}

#[tokio::test]
async fn a_whole_fork_sends_no_fork_point() {
    let observed = exercise_start(
        native_fork(None),
        json!({"sessionCapabilities": {"fork": {}}}),
        false,
    )
    .await;
    observed.result.as_ref().expect("fork starts the session");
    assert!(
        observed.requests[1]["params"]["_meta"]
            .get("jetbrains")
            .is_none()
    );
}

#[tokio::test]
async fn a_fork_never_falls_back_to_an_empty_session() {
    let unsupported = exercise_start(native_fork(None), resume_and_load(), false).await;
    assert!(
        unsupported
            .result
            .as_ref()
            .unwrap_err()
            .contains("does not support session/fork"),
        "{:?}",
        unsupported.result
    );
    assert_eq!(unsupported.methods(), ["initialize"]);

    let rejected = exercise_start(
        native_fork(None),
        json!({"sessionCapabilities": {"fork": {}}}),
        true,
    )
    .await;
    assert!(rejected.result.is_err());
    assert_eq!(rejected.methods(), ["initialize", "session/fork"]);
    let meta = rejected.sink.hub.session_info(SESSION).unwrap().meta;
    assert_eq!(meta.agent_session_id, None);
}

fn fixture_options(model: &str, effort: &str) -> Value {
    json!([
        {"id":"model", "name":"Model", "type":"select", "currentValue":model,
         "options":[{"value":"old-model","name":"Old"},{"value":"saved-model","name":"Saved"}]},
        {"id":"reasoning_effort", "name":"Reasoning", "type":"select", "currentValue":effort,
         "options":[{"value":"low","name":"Low"},{"value":"high","name":"High"}]}
    ])
}

#[tokio::test]
async fn managed_read_only_never_sends_startup_or_replayed_permission_overrides() {
    for provider in ["codex", "claude-code"] {
        exercise_managed_read_only(provider).await;
    }
}

/// Exercise the actual ACP handshake and command loop. The peer advertises
/// ordinary full-access controls; neither startup nor a later caller can select
/// them. Native sandbox enforcement remains a separate Provider acceptance gate.
#[allow(clippy::too_many_lines)]
async fn exercise_managed_read_only(provider: &str) {
    let (state, sink) = fixture_state(false);
    sink.managed_read_only.store(true, Ordering::SeqCst);
    let (client_io, peer_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let (startup, mut phase) = watch::channel(StartupPhase::Initialize);
    let main_state = state.clone();
    let notifications = state.clone();
    let client = Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification,
                        _: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                handle_session_notification(&notifications, &notification);
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
                    None.into(),
                    PathBuf::from(CWD),
                    &mut command_rx,
                    provider,
                    &startup,
                )
                .await
            },
        );
    let peer = async {
        let (read, mut write) = tokio::io::split(peer_io);
        let mut lines = BufReader::new(read).lines();
        let options = json!([{
            "id":CODEX_FULL_ACCESS_CONFIG_ID,"name":"Approval","type":"select",
            "currentValue":"read-only","options":[
                {"value":"read-only","name":"Read only"},
                {"value":CODEX_FULL_ACCESS_CONFIG_VALUE,"name":"Full access"}
            ]
        }, {
            "id":"model","name":"Model","type":"select","currentValue":"base","options":[
                {"value":"base","name":"Base"},{"value":"strong","name":"Strong"}
            ]
        }]);
        for method in ["initialize", "session/new"] {
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], method);
            let result = if method == "initialize" {
                json!({"protocolVersion":1,"agentCapabilities":{}})
            } else {
                json!({"sessionId":NATIVE,"configOptions":options,"modes":{
                    "currentModeId":"default","availableModes":[
                        {"id":"default","name":"Default"},
                        {"id":"bypassPermissions","name":"Full access"}
                    ]
                }})
            };
            send_json(
                &mut write,
                json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
            )
            .await;
        }
        phase
            .wait_for(|phase| *phase == StartupPhase::Ready)
            .await
            .unwrap();
        assert_eq!(
            sink.hub.persisted_config_options(SESSION),
            Some(json!([options[1].clone()]))
        );
        send_json(&mut write, json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":NATIVE,"update":{"sessionUpdate":"config_option_update","configOptions":options}
        }})).await;
        command_tx
            .send(AgentCommand::SetConfigOption {
                config_id: CODEX_FULL_ACCESS_CONFIG_ID.into(),
                value: json!(CODEX_FULL_ACCESS_CONFIG_VALUE),
            })
            .unwrap();
        // The policy's preset (model, reasoning) is the one configuration a
        // managed child accepts, and it is applied before the prompt.
        command_tx
            .send(AgentCommand::SetConfigOption {
                config_id: "model".into(),
                value: json!("strong"),
            })
            .unwrap();
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["method"], "session/set_config_option");
        assert_eq!(request["params"]["configId"], "model");
        assert_eq!(request["params"]["value"], "strong");
        let mut confirmed = options.clone();
        confirmed[1]["currentValue"] = json!("strong");
        send_json(
            &mut write,
            json!({"jsonrpc":"2.0","id":request["id"],"result":{"configOptions":confirmed}}),
        )
        .await;
        let (done, completed) = oneshot::channel();
        command_tx
            .send(AgentCommand::Prompt(
                vec![serde_json::from_value(json!({"type":"text","text":"review"})).unwrap()],
                Some("managed-call-1".into()),
                Some(done),
            ))
            .unwrap();
        // An unexpected startup/config RPC appears here instead of the prompt.
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["method"], "session/prompt");
        let usage =
            agent_client_protocol::schema::v1::Usage::new(110, 80, 10).cached_read_tokens(20);
        send_json(
            &mut write,
            json!({"jsonrpc":"2.0","id":request["id"],"result":{"stopReason":"end_turn","usage":usage}}),
        )
        .await;
        assert!(completed.await.unwrap().is_ok());
        assert_eq!(
            sink.hub.managed_turn(SESSION, 0).unwrap().usage,
            crate::agent_model::TurnUsage::from_acp(&usage),
        );
        assert_eq!(
            sink.hub.persisted_config_options(SESSION),
            Some(json!([confirmed[1].clone()]))
        );
        drop(command_tx);
        assert!(lines.next_line().await.unwrap().is_none());
    };
    let (result, ()) =
        tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(client, peer) })
            .await
            .unwrap();
    assert!(result.is_ok(), "{result:?}");
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
                Some(NATIVE.to_owned()).into(),
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

/// codex-acp answers ACP authRequired when the app-server needs a login and
/// no default auth request applies (an expired `ChatGPT` login). The
/// resume keeps the native thread and reports Cowboy's typed "reconnect the
/// same account" state, which the Controller marks as expired authorization
/// and the Machine refuses to retry on an older worker generation.
#[tokio::test]
async fn auth_required_resume_reports_reconnect_and_keeps_the_native_thread() {
    let observed = exercise_rejected(
        NativeStart::Resume(NATIVE.to_owned()),
        resume_and_load(),
        Some(json!({"code": -32000, "message": "Authentication required"})),
    )
    .await;
    let detail = observed.result.as_ref().unwrap_err();
    assert!(
        crate::provider_behavior::is_provider_auth_required_error(detail),
        "{detail}"
    );
    assert!(detail.contains("Reconnect the same"), "{detail}");
    assert_eq!(observed.methods(), ["initialize", "session/resume"]);
    observed.assert_retained_identity();
    assert_eq!(observed.texts(), ["saved user", "saved answer"]);
    assert_ne!(observed.sink.hub.status(SESSION), Some(Status::Running));
}

/// The pre-fix failure: an unconditional API-key default turned the same
/// expired login into an internal error, which is not an authorization state.
#[tokio::test]
async fn internal_api_key_error_at_resume_is_not_an_auth_state() {
    let observed = exercise_rejected(
        NativeStart::Resume(NATIVE.to_owned()),
        resume_and_load(),
        Some(json!({
            "code": -32603,
            "message": "Internal error",
            "data": "CODEX_API_KEY or OPENAI_API_KEY is not set"
        })),
    )
    .await;
    let detail = observed.result.as_ref().unwrap_err();
    assert!(
        !crate::provider_behavior::is_provider_auth_required_error(detail),
        "{detail}"
    );
    observed.assert_retained_identity();
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

fn native_activity(state: &str) -> ClaudeSdkMessageNotification {
    ClaudeSdkMessageNotification {
        session_id: NATIVE.to_owned(),
        message: json!({
            "type": "system", "subtype": "session_state_changed",
            "session_id": NATIVE, "state": state,
        }),
    }
}

fn activity_fixture(provider: &str) -> (Arc<ClientState>, Arc<HubSink>) {
    let (mut state, sink) = fixture_state(false);
    Arc::get_mut(&mut state).unwrap().provider_id = provider.to_owned();
    state.activity.lock().native_session_id = Some(NATIVE.to_owned());
    state.set_status(Status::Running, None);
    (state, sink)
}

fn observed_status(sink: &HubSink) -> Status {
    sink.hub.session_info(SESSION).unwrap().meta.status
}

#[test]
fn native_idle_never_finishes_a_pending_prompt_or_clears_a_failure() {
    let (state, sink) = activity_fixture("claude-code");
    state.set_status(Status::Busy, None);
    handle_native_activity(&state, &native_activity("running"));
    handle_native_activity(&state, &native_activity("idle"));
    assert_eq!(observed_status(&sink), Status::Busy);
    state.set_status(Status::Running, None);
    assert_eq!(observed_status(&sink), Status::Running);

    for status in [Status::Starting, Status::Crashed, Status::Exited] {
        state.set_status(status, None);
        handle_native_activity(&state, &native_activity("running"));
        handle_native_activity(&state, &native_activity("idle"));
        assert_eq!(observed_status(&sink), status);
    }
    // Recoverable prompt errors retain their detail on an otherwise live worker.
    state.set_status(Status::Running, Some("recoverable turn failure".to_owned()));
    let before = sink.hub.snapshot(SESSION).unwrap().0.len();
    handle_native_activity(&state, &native_activity("running"));
    handle_native_activity(&state, &native_activity("idle"));
    assert_eq!(sink.hub.snapshot(SESSION).unwrap().0.len(), before);
}

#[test]
fn native_activity_ignores_history_other_sessions_and_unrelated_messages() {
    let (state, sink) = activity_fixture("claude-deepseek");
    state.suppress_updates.store(true, Ordering::SeqCst);
    handle_native_activity(&state, &native_activity("running"));
    assert_eq!(observed_status(&sink), Status::Running);
    state.suppress_updates.store(false, Ordering::SeqCst);

    let mut wrong_session = native_activity("running");
    wrong_session.session_id = "other".to_owned();
    handle_native_activity(&state, &wrong_session);
    wrong_session.message["session_id"] = "other".into();
    handle_native_activity(&state, &wrong_session);
    let mut unrelated = native_activity("running");
    unrelated.message["subtype"] = "task_started".into();
    handle_native_activity(&state, &unrelated);
    handle_native_activity(&state, &native_activity("future-state"));
    assert_eq!(observed_status(&sink), Status::Running);

    handle_native_activity(&state, &native_activity("requires_action"));
    assert_eq!(observed_status(&sink), Status::Busy);
    handle_native_activity(&state, &native_activity("idle"));
    assert_eq!(observed_status(&sink), Status::Running);

    let (other, other_sink) = activity_fixture("codex");
    handle_native_activity(&other, &native_activity("running"));
    assert_eq!(observed_status(&other_sink), Status::Running);
}

fn native_api_error(retry_in_ms: u64) -> ClaudeSdkMessageNotification {
    ClaudeSdkMessageNotification {
        session_id: NATIVE.to_owned(),
        message: json!({
            "type": "system", "subtype": "api_error", "session_id": NATIVE,
            "error": {
                "status": 429,
                "rateLimits": {
                    "rateLimitType": "seven_day_overage_included",
                    "resetsAt": 1_789_884_000_i64
                }
            },
            "retryInMs": retry_in_ms, "retryAttempt": 1, "maxRetries": 300,
        }),
    }
}

/// A rate limit is the provider explaining its own silence, and the SDK states
/// it once and then says nothing for the whole wait. Cowboy has to hold that
/// explanation for exactly as long as it is true: from the frame that states it
/// until the turn proves the request finally landed.
#[test]
fn a_rate_limited_turn_holds_the_provider_reason_until_progress_resumes() {
    let (state, _sink) = activity_fixture("claude-code");
    *state.model_label.lock() = Some("Fable 5.1".to_owned());

    handle_native_activity(&state, &native_api_error(3_469_283));
    let reason = match backoff_verdict(state.backoff.lock().as_ref(), Instant::now()) {
        BackoffVerdict::Hopeless(reason) => reason,
        other => panic!("an hour-long wait must not read as a wedge or a short one: {other:?}"),
    };
    assert!(reason.contains("Fable 5.1"), "{reason}");
    assert!(reason.contains("seven_day_overage_included"), "{reason}");

    // Another native session's limit is not this turn's silence.
    state.backoff.lock().take();
    let mut elsewhere = native_api_error(3_469_283);
    elsewhere.message["session_id"] = "other".into();
    handle_native_activity(&state, &elsewhere);
    assert!(state.backoff.lock().is_none());

    // Streamed progress proves the request landed, so the wait is spent — a
    // stale reason would otherwise end the next quiet stretch as a rate limit.
    handle_native_activity(&state, &native_api_error(3_469_283));
    note_turn_progress(
        &state,
        &json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hi"}}),
    );
    assert!(state.backoff.lock().is_none());
}

fn native_background_tasks(tasks: &Value) -> ClaudeSdkMessageNotification {
    ClaudeSdkMessageNotification {
        session_id: NATIVE.to_owned(),
        message: json!({
            "type": "system", "subtype": "background_tasks_changed",
            "session_id": NATIVE, "tasks": tasks,
        }),
    }
}

fn observed_background_tasks(sink: &HubSink) -> u32 {
    sink.hub
        .session_info(SESSION)
        .unwrap()
        .meta
        .background_tasks
}

#[test]
fn native_background_tasks_project_activity_without_holding_the_prompt() {
    let (state, sink) = activity_fixture("claude-code");
    // The prompt returned while a Monitor and a backgrounded shell still run.
    handle_native_activity(
        &state,
        &native_background_tasks(&json!([
            {"task_id": "m1", "task_type": "local_bash", "description": "watch gate"},
            {"task_id": "b1", "task_type": "local_bash", "description": "build"},
            {"task_id": "d1", "task_type": "dream", "description": "memory", "ambient": true},
        ])),
    );
    assert_eq!(observed_background_tasks(&sink), 2);
    // Presentation only: the idle prompt remains dispatchable.
    assert_eq!(observed_status(&sink), Status::Running);
    let before = sink.hub.snapshot(SESSION).unwrap().0.len();

    // Another session's level, history replay, and malformed payloads are ignored.
    let mut other = native_background_tasks(&json!([]));
    other.session_id = "other".to_owned();
    other.message["session_id"] = "other".into();
    handle_native_activity(&state, &other);
    state.suppress_updates.store(true, Ordering::SeqCst);
    handle_native_activity(&state, &native_background_tasks(&json!([])));
    state.suppress_updates.store(false, Ordering::SeqCst);
    handle_native_activity(&state, &native_background_tasks(&json!(null)));
    assert_eq!(observed_background_tasks(&sink), 2);

    // REPLACE semantics: the level, not paired edges, drives the count.
    handle_native_activity(
        &state,
        &native_background_tasks(&json!([
            {"task_id": "m1", "task_type": "local_bash", "description": "watch gate"},
        ])),
    );
    assert_eq!(observed_background_tasks(&sink), 1);
    handle_native_activity(&state, &native_background_tasks(&json!([])));
    assert_eq!(observed_background_tasks(&sink), 0);
    // The level is session metadata, never a transcript row.
    assert_eq!(sink.hub.snapshot(SESSION).unwrap().0.len(), before);

    let (other_provider, other_sink) = activity_fixture("codex");
    handle_native_activity(
        &other_provider,
        &native_background_tasks(&json!([{"task_id": "x", "task_type": "local_bash"}])),
    );
    assert_eq!(observed_background_tasks(&other_sink), 0);
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep the complete prompt/native lifecycle in wire order.
async fn native_background_resume_restores_busy_after_prompt_response_over_acp() {
    let (state, sink) = activity_fixture("claude-code");
    let (client_io, peer_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let (startup, mut phase) = watch::channel(StartupPhase::Initialize);
    let (observed_tx, mut observed_rx) = watch::channel(0_usize);
    let notifications = state.clone();
    let main_state = state.clone();
    let client = Client
        .builder()
        .on_receive_notification(
            async move |notification: ClaudeSdkMessageNotification,
                        _: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                handle_native_activity(&notifications, &notification);
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
                    None.into(),
                    PathBuf::from(CWD),
                    &mut command_rx,
                    "claude-code",
                    &startup,
                )
                .await
            },
        );
    let peer = async {
        let (reader, mut writer) = tokio::io::split(peer_io);
        let mut lines = BufReader::new(reader).lines();
        let initialize: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(initialize["method"], "initialize");
        send_json(
            &mut writer,
            json!({
                "jsonrpc": "2.0", "id": initialize["id"],
                "result": {"protocolVersion": 1, "agentCapabilities": {}}
            }),
        )
        .await;
        let new: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(new["method"], "session/new");
        assert_eq!(
            new["params"]["_meta"]["claudeCode"]["emitRawSDKMessages"],
            json!([
                {"type": "system", "subtype": "session_state_changed"},
                {"type": "system", "subtype": "background_tasks_changed"},
                {"type": "system", "subtype": "api_error"}
            ])
        );
        send_json(
            &mut writer,
            json!({
                "jsonrpc": "2.0", "id": new["id"], "result": {"sessionId": NATIVE}
            }),
        )
        .await;
        phase
            .wait_for(|phase| *phase == StartupPhase::Ready)
            .await
            .unwrap();
        command_tx
            .send(AgentCommand::Prompt(
                vec![ContentBlock::Text(
                    agent_client_protocol::schema::v1::TextContent::new("build"),
                )],
                None,
                None,
            ))
            .unwrap();
        let prompt: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(prompt["method"], "session/prompt");
        send_json(&mut writer, json!({
            "jsonrpc": "2.0", "method": "_claude/sdkMessage", "params": native_activity("running")
        })).await;
        observed_rx.wait_for(|count| *count == 1).await.unwrap();
        send_json(
            &mut writer,
            json!({
                "jsonrpc": "2.0", "id": prompt["id"], "result": {"stopReason": "end_turn"}
            }),
        )
        .await;
        while sink.prompt_completions.lock().is_empty() {
            tokio::task::yield_now().await;
        }
        assert_eq!(state.activity.lock().prompt_status, Status::Running);
        assert_eq!(observed_status(&sink), Status::Busy);
        // The original prompt ends, then a shell completion wakes the SDK with
        // no new session/prompt request. Repeated native edges are idempotent.
        for (count, native, expected) in [
            (2, "idle", Status::Running),
            (3, "running", Status::Busy),
            (4, "running", Status::Busy),
            (5, "requires_action", Status::Busy),
            (6, "idle", Status::Running),
        ] {
            send_json(&mut writer, json!({
                "jsonrpc": "2.0", "method": "_claude/sdkMessage", "params": native_activity(native)
            })).await;
            observed_rx
                .wait_for(|observed| *observed == count)
                .await
                .unwrap();
            assert_eq!(observed_status(&sink), expected);
        }
        assert_eq!(sink.prompt_starts.lock().len(), 1);
        assert_eq!(sink.prompt_completions.lock().len(), 1);
        let events = sink.hub.snapshot(SESSION).unwrap().0;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.event, Event::TurnEnd { .. }))
                .count(),
            1
        );
        assert!(events.iter().all(|event| {
            !serde_json::to_string(event)
                .unwrap()
                .contains("session_state_changed")
        }));
        drop(command_tx);
        phase.wait_for(|_| false).await.unwrap_err();
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let (result, ()) = tokio::join!(client, peer);
        assert!(result.is_ok(), "{result:?}");
    })
    .await
    .expect("bounded native activity fixture");
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Exercise the complete concurrent native wire lifecycle.
async fn current_turn_injects_without_cancel_or_new_prompt_and_refuses_idle() {
    let (state, sink) = activity_fixture("claude-code");
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
                None.into(),
                PathBuf::from(CWD),
                &mut command_rx,
                "claude-code",
                &startup,
            )
            .await
        },
    );
    let peer = async {
        let (reader, mut writer) = tokio::io::split(peer_io);
        let mut lines = BufReader::new(reader).lines();
        let initialize: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        send_json(&mut writer, json!({"jsonrpc":"2.0","id":initialize["id"],"result":{
            "protocolVersion":1,"agentCapabilities":{},"_meta":{"steering":{"supported":true,"idleBehavior":"promptRequired","priorities":["now","next"]}}
        }})).await;
        let new: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        send_json(
            &mut writer,
            json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":NATIVE}}),
        )
        .await;
        phase
            .wait_for(|phase| *phase == StartupPhase::Ready)
            .await
            .unwrap();
        let blocks = || {
            vec![ContentBlock::Text(
                agent_client_protocol::schema::v1::TextContent::new("correction"),
            )]
        };
        command_tx
            .send(AgentCommand::Prompt(blocks(), Some("first".into()), None))
            .unwrap();
        let prompt: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(prompt["method"], "session/prompt");
        command_tx
            .send(AgentCommand::Steer {
                blocks: blocks(),
                cmid: "steered".into(),
                next_checkpoint: false,
                target: None,
            })
            .unwrap();
        let steer: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(steer["method"], "_session/steering");
        assert_eq!(
            steer["params"]["_meta"]["steering"]["idleBehavior"],
            "promptRequired"
        );
        send_json(
            &mut writer,
            json!({"jsonrpc":"2.0","id":steer["id"],"result":{"outcome":"injected"}}),
        )
        .await;
        loop {
            let events = sink.hub.snapshot(SESSION).unwrap().0;
            if events.iter().any(|entry| matches!(&entry.event, Event::Update { update } if update["cmid"] == "steered" && update["outcome"] == "injected")) { break; }
            tokio::task::yield_now().await;
        }
        assert_eq!(sink.prompt_starts.lock().len(), 1);
        assert!(sink.prompt_completions.lock().is_empty());
        command_tx
            .send(AgentCommand::Steer {
                blocks: blocks(),
                cmid: "checkpoint".into(),
                next_checkpoint: true,
                target: None,
            })
            .unwrap();
        let checkpoint: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(checkpoint["method"], "_session/steering");
        assert_eq!(
            checkpoint["params"]["_meta"]["steering"]["priority"],
            "next"
        );
        send_json(
            &mut writer,
            json!({"jsonrpc":"2.0","id":checkpoint["id"],"result":{"outcome":"injected"}}),
        )
        .await;
        command_tx
            .send(AgentCommand::CancelTurn {
                target: crate::send_delivery::DeliveryTarget {
                    worker_epoch: "stale-worker".into(),
                    turn_id: "stale-turn".into(),
                },
                cmid: "stale-cancel".into(),
            })
            .unwrap();
        loop {
            let events = sink.hub.snapshot(SESSION).unwrap().0;
            if events.iter().any(|entry| matches!(&entry.event, Event::Update { update } if update["cmid"] == "stale-cancel" && update["outcome"] == "not_delivered")) { break; }
            tokio::task::yield_now().await;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), lines.next_line())
                .await
                .is_err()
        );
        command_tx
            .send(AgentCommand::CancelTurn {
                target: crate::send_delivery::DeliveryTarget {
                    worker_epoch: "fixture-worker".into(),
                    turn_id: "fixture-turn".into(),
                },
                cmid: "matching-cancel".into(),
            })
            .unwrap();
        let cancellation: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(cancellation["method"], "session/cancel");
        assert_eq!(cancellation["params"]["sessionId"], NATIVE);
        send_json(
            &mut writer,
            json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}}),
        )
        .await;
        while sink.prompt_completions.lock().is_empty() {
            tokio::task::yield_now().await;
        }
        command_tx
            .send(AgentCommand::Steer {
                blocks: blocks(),
                cmid: "idle".into(),
                next_checkpoint: false,
                target: None,
            })
            .unwrap();
        loop {
            let events = sink.hub.snapshot(SESSION).unwrap().0;
            if events.iter().any(|entry| matches!(&entry.event, Event::Update { update } if update["cmid"] == "idle" && update["outcome"] == "not_delivered")) { break; }
            tokio::task::yield_now().await;
        }
        // The prompt watchdog may repeat the same cancellation when its watch
        // wakes. Idle steering must never emit another prompt or steering RPC.
        while let Ok(line) =
            tokio::time::timeout(Duration::from_millis(20), lines.next_line()).await
        {
            let frame: Value = serde_json::from_str(&line.unwrap().unwrap()).unwrap();
            assert_eq!(frame["method"], "session/cancel");
            assert_eq!(frame["params"]["sessionId"], NATIVE);
        }
        assert_eq!(sink.prompt_starts.lock().len(), 1);
        drop(command_tx);
        phase.wait_for(|_| false).await.unwrap_err();
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let (result, ()) = tokio::join!(client, peer);
        assert!(result.is_ok(), "{result:?}");
    })
    .await
    .expect("bounded steering fixture");
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // The two prompts must share one real ACP connection.
async fn partial_stream_failure_keeps_native_session_without_replaying_completed_work() {
    let (state, sink) = activity_fixture("claude-code");
    let (client_io, peer_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let (startup, mut phase) = watch::channel(StartupPhase::Initialize);
    let notifications = state.clone();
    let main_state = state.clone();
    let client = Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification,
                        _: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                handle_session_notification(&notifications, &notification);
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
                    None.into(),
                    PathBuf::from(CWD),
                    &mut command_rx,
                    "claude-code",
                    &startup,
                )
                .await
            },
        );
    let peer = async {
        let (reader, mut writer) = tokio::io::split(peer_io);
        let mut lines = BufReader::new(reader).lines();
        for (method, result) in [
            (
                "initialize",
                json!({"protocolVersion": 1, "agentCapabilities": {}}),
            ),
            ("session/new", json!({"sessionId": NATIVE})),
        ] {
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], method);
            send_json(
                &mut writer,
                json!({"jsonrpc":"2.0", "id":request["id"], "result":result}),
            )
            .await;
        }
        phase
            .wait_for(|phase| *phase == StartupPhase::Ready)
            .await
            .unwrap();
        let message =
            "API Error: Connection lost mid-response. The response above may be incomplete.";
        for (index, text) in ["write the file", "continue from the saved result"]
            .iter()
            .enumerate()
        {
            let (complete, completed) = oneshot::channel();
            command_tx
                .send(AgentCommand::Prompt(
                    vec![ContentBlock::Text(
                        agent_client_protocol::schema::v1::TextContent::new(*text),
                    )],
                    Some(format!("stream-prompt-{index}")),
                    Some(complete),
                ))
                .unwrap();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            // Any automatic replay or native-session reconstruction fails here.
            assert_eq!(request["method"], "session/prompt");
            assert_eq!(request["params"]["sessionId"], NATIVE);
            assert_eq!(request["params"]["prompt"][0]["text"], *text);
            if index == 0 {
                for update in [
                    json!({"sessionUpdate":"tool_call", "toolCallId":"write-once", "title":"Write file", "kind":"edit", "status":"completed"}),
                    json!({"sessionUpdate":"tool_call", "toolCallId":"incomplete", "title":"Terminal", "kind":"execute"}),
                ] {
                    send_json(&mut writer, json!({"jsonrpc":"2.0", "method":"session/update", "params":{"sessionId":NATIVE, "update":update}})).await;
                }
                send_json(
                    &mut writer,
                    notification(NATIVE, "agent_message_chunk", message),
                )
                .await;
                send_json(
                    &mut writer,
                    json!({"jsonrpc":"2.0", "id":request["id"], "error":{
                        "code":-32603, "message":message, "data":{"errorKind":"server_error"}
                    }}),
                )
                .await;
                assert!(completed.await.unwrap().unwrap_err().contains(message));
            } else {
                send_json(
                    &mut writer,
                    notification(
                        NATIVE,
                        "agent_message_chunk",
                        "Continued after the saved tool result.",
                    ),
                )
                .await;
                send_json(&mut writer, json!({"jsonrpc":"2.0", "id":request["id"], "result":{"stopReason":"end_turn"}})).await;
                assert_eq!(
                    completed.await.unwrap().unwrap(),
                    "Continued after the saved tool result."
                );
            }
            while observed_status(&sink) == Status::Busy {
                tokio::task::yield_now().await;
            }
            assert_eq!(observed_status(&sink), Status::Running);
            assert_eq!(
                sink.hub
                    .session_info(SESSION)
                    .unwrap()
                    .meta
                    .agent_session_id
                    .as_deref(),
                Some(NATIVE)
            );
        }
        assert_eq!(sink.allocations.load(Ordering::SeqCst), 1);
        assert_eq!(sink.prompt_starts.lock().len(), 2);
        assert_eq!(sink.prompt_completions.lock().len(), 2);
        let events = sink.hub.snapshot(SESSION).unwrap().0;
        let completed_tools = events.iter().filter(|event| matches!(&event.event,
            Event::Update { update } if update["toolCallId"] == "write-once" && update["status"] == "completed"
        )).count();
        assert_eq!(completed_tools, 1);
        let failures = events.iter().filter(|event| matches!(&event.event, Event::TurnEnd { stop_reason, .. } if stop_reason.starts_with("error:"))).count();
        assert_eq!(failures, 1);
        assert!(!events.iter().any(|event| matches!(
            event.event,
            Event::Lifecycle {
                status: Status::Crashed,
                ..
            }
        )));
        drop(command_tx);
        phase.wait_for(|_| false).await.unwrap_err();
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let (result, ()) = tokio::join!(client, peer);
        assert!(result.is_ok(), "{result:?}");
    })
    .await
    .expect("bounded partial-stream recovery fixture");
}

#[test]
fn partial_stream_policy_preserves_worker_but_never_replays_a_prompt() {
    let source: cowboy_provider_sdk::StandardProviderSource =
        serde_json::from_str(include_str!("../plugins/claude-code/provider.json")).unwrap();
    let behavior = source.compile().unwrap().runtime.behavior;
    for reason in [
        "Connection lost mid-response",
        "Server error mid-response",
        "The response stopped arriving",
        "Your computer went to sleep mid-response",
    ] {
        let detail = format!(
            "Internal error: API Error: {reason}. The response above may be incomplete.: {{\"errorKind\":\"server_error\"}}"
        );
        assert!(crate::provider::keeps_worker_alive_for_behavior(
            &behavior, &detail
        ));
        for visible in [false, true] {
            assert!(!crate::provider::should_retry_without_visible_update(
                &behavior, &detail, visible, 0
            ));
        }
    }
    for detail in [
        "Internal error: API Error: authentication failed",
        "Internal error: API Error: permission denied",
        "Internal error: connection closed",
        "API Error: Connection lost mid-response.",
        "The response above may be incomplete.",
    ] {
        assert!(!crate::provider::keeps_worker_alive_for_behavior(
            &behavior, detail
        ));
    }
}

fn native_cycle(cycle: Option<&str>) -> ClaudeSdkMessageNotification {
    ClaudeSdkMessageNotification {
        session_id: NATIVE.to_owned(),
        message: json!({
            "type": "system", "subtype": "cowboy_native_cycle",
            "session_id": NATIVE, "cycle_id": cycle,
        }),
    }
}

/// Claude's self-started follow-up has no prompt. A steer addressed to the
/// adapter's exact cycle name reaches the adapter with that name; any other
/// cycle, or one that ended, is refused here without a native request.
#[tokio::test]
#[allow(clippy::too_many_lines)] // One real ACP connection across the cycle's lifetime.
async fn a_self_started_cycle_is_steerable_only_by_its_exact_name() {
    let (state, sink) = activity_fixture("claude-code");
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
                None.into(),
                PathBuf::from(CWD),
                &mut command_rx,
                "claude-code",
                &startup,
            )
            .await
        },
    );
    let peer = async {
        let (reader, mut writer) = tokio::io::split(peer_io);
        let mut lines = BufReader::new(reader).lines();
        let initialize: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        send_json(&mut writer, json!({"jsonrpc":"2.0","id":initialize["id"],"result":{
            "protocolVersion":1,"agentCapabilities":{},"_meta":{"steering":{"supported":true,"idleBehavior":"promptRequired","priorities":["now","next"]}}
        }})).await;
        let new: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        send_json(
            &mut writer,
            json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":NATIVE}}),
        )
        .await;
        phase
            .wait_for(|phase| *phase == StartupPhase::Ready)
            .await
            .unwrap();
        sink.hub.set_status(SESSION, Status::Busy, None);
        // The production connection routes `_claude/sdkMessage` here.
        handle_native_activity(&state, &native_cycle(Some("native-a")));
        assert_eq!(
            *sink.native_cycles.lock(),
            vec![Some("native-a".to_owned())]
        );
        handle_native_activity(&state, &native_cycle(Some("native-a")));
        assert_eq!(
            sink.native_cycles.lock().len(),
            1,
            "an unchanged name is not re-announced"
        );
        let blocks = || {
            vec![ContentBlock::Text(
                agent_client_protocol::schema::v1::TextContent::new("correction"),
            )]
        };
        let steer = |cmid: &str, cycle: &str, next_checkpoint: bool| AgentCommand::Steer {
            blocks: blocks(),
            cmid: cmid.into(),
            next_checkpoint,
            target: Some(crate::send_delivery::DeliveryTarget {
                worker_epoch: "fixture-worker".into(),
                turn_id: cycle.into(),
            }),
        };
        let outcome = |cmid: &'static str, expected: &'static str| {
            let hub = sink.hub.clone();
            async move {
                loop {
                    let events = hub.snapshot(SESSION).unwrap().0;
                    if events.iter().any(|entry| matches!(&entry.event, Event::Update { update } if update["cmid"] == cmid && update["outcome"] == expected)) { break; }
                    tokio::task::yield_now().await;
                }
            }
        };
        command_tx.send(steer("join", "native-a", true)).unwrap();
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["method"], "_session/steering");
        assert_eq!(
            request["params"]["_meta"]["steering"]["nativeCycle"],
            "native-a"
        );
        assert_eq!(request["params"]["_meta"]["steering"]["priority"], "next");
        assert_eq!(
            request["params"]["_meta"]["steering"]["idleBehavior"],
            "promptRequired"
        );
        send_json(
            &mut writer,
            json!({"jsonrpc":"2.0","id":request["id"],"result":{"outcome":"injected"}}),
        )
        .await;
        outcome("join", "injected").await;
        assert!(
            sink.hub
                .snapshot(SESSION)
                .unwrap()
                .0
                .iter()
                .any(|entry| matches!(
                    &entry.event,
                    Event::Update { update } if update["sessionUpdate"] == "user_message_chunk"
                        && update["_meta"]["cowboyDelivery"] == "current_turn"
                )),
            "the joined message is echoed once as current-turn delivery"
        );

        // A different (earlier or later) cycle is never joined.
        command_tx.send(steer("other", "native-b", false)).unwrap();
        outcome("other", "not_delivered").await;
        // The cycle ended: its name no longer addresses anything.
        handle_native_activity(&state, &native_cycle(None));
        assert_eq!(sink.native_cycles.lock().last(), Some(&None));
        assert_eq!(*state.native_cycle.lock(), None);
        command_tx.send(steer("ended", "native-a", false)).unwrap();
        outcome("ended", "not_delivered").await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), lines.next_line())
                .await
                .is_err(),
            "a refused cycle steer sends no native request"
        );
        assert!(
            sink.prompt_starts.lock().is_empty(),
            "joining never sends session/prompt"
        );
        drop(command_tx);
        phase.wait_for(|_| false).await.unwrap_err();
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let (result, ()) = tokio::join!(client, peer);
        assert!(result.is_ok(), "{result:?}");
    })
    .await
    .expect("bounded native-cycle fixture");
}

const CHILD: &str = "child-session";

fn update_of(session: &str, update: Value) -> Value {
    let mut notification = json!({"jsonrpc": "2.0", "method": "session/update", "params": {
        "sessionId": session
    }});
    notification["params"]["update"] = update;
    notification
}

fn timeline_updates(sink: &HubSink) -> Vec<Value> {
    sink.hub
        .snapshot(SESSION)
        .unwrap()
        .0
        .into_iter()
        .filter_map(|entry| match entry.event {
            Event::Update { update } => Some(update),
            _ => None,
        })
        .collect()
}

fn subagent_row(sink: &HubSink, child: &str) -> Option<Value> {
    timeline_updates(sink).into_iter().rev().find(|update| {
        update["sessionUpdate"] == subagents::SUBAGENT_UPDATE && update["sessionId"] == child
    })
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One subagent's complete wire lifecycle, in order.
async fn a_child_session_is_one_live_row_and_stops_alone() {
    let (state, sink) = activity_fixture("fixture-provider");
    let (client_io, peer_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let (startup, mut phase) = watch::channel(StartupPhase::Initialize);
    let (observed_tx, mut observed_rx) = watch::channel(0_usize);
    let notifications = state.clone();
    let main_state = state.clone();
    let client = Client
        .builder()
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
                    None.into(),
                    PathBuf::from(CWD),
                    &mut command_rx,
                    "fixture-provider",
                    &startup,
                )
                .await
            },
        );
    let peer = async {
        let (reader, mut writer) = tokio::io::split(peer_io);
        let mut lines = BufReader::new(reader).lines();
        let mut next = async || -> Value {
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
        };
        let initialize = next().await;
        assert_eq!(initialize["method"], "initialize");
        assert_eq!(
            initialize["params"]["clientCapabilities"]["subagents"],
            json!({})
        );
        send_json(
            &mut writer,
            json!({"jsonrpc":"2.0","id":initialize["id"],"result":{
                "protocolVersion":1,"agentCapabilities":{}
            }}),
        )
        .await;
        let new = next().await;
        assert_eq!(new["method"], "session/new");
        send_json(
            &mut writer,
            json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":NATIVE}}),
        )
        .await;
        phase
            .wait_for(|phase| *phase == StartupPhase::Ready)
            .await
            .unwrap();
        command_tx
            .send(AgentCommand::Prompt(
                vec![ContentBlock::Text(
                    agent_client_protocol::schema::v1::TextContent::new("delegate"),
                )],
                None,
                None,
            ))
            .unwrap();
        let prompt = next().await;
        assert_eq!(prompt["method"], "session/prompt");
        let baseline = *observed_rx.borrow();
        // The scripted adapter's RFD form: announce, delegate, then the
        // child's own traffic on its own session id.
        let wire = [
            update_of(
                NATIVE,
                json!({
                    "sessionUpdate": "subagent_update", "sessionId": CHILD,
                    "title": "Explore", "description": "Explore",
                    "capabilities": {"cancel": {}}, "state": {"state": "running"},
                }),
            ),
            update_of(
                NATIVE,
                json!({
                    "sessionUpdate": "session_message", "messageId": "toolu_1",
                    "senderSessionId": NATIVE, "recipientSessionId": CHILD,
                    "recipientMessageId": "toolu_1",
                    "content": [{"type": "text", "text": "Find the flaky test"}],
                }),
            ),
            update_of(
                CHILD,
                json!({
                    "sessionUpdate": "config_option_update",
                    "configOptions": [{"id": "model", "name": "Model", "type": "select",
                        "currentValue": "child-model", "options": [{"value": "child-model", "name": "Child"}]}],
                }),
            ),
            update_of(
                CHILD,
                json!({
                    "sessionUpdate": "tool_call", "toolCallId": "child-tool",
                    "title": "Read tests.rs", "kind": "read", "status": "in_progress",
                }),
            ),
            update_of(
                CHILD,
                json!({
                    "sessionUpdate": "agent_message_chunk", "messageId": "child-message",
                    "content": {"type": "text", "text": "Looking"},
                }),
            ),
            update_of(
                NATIVE,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "Parent waits."},
                }),
            ),
        ];
        for (index, notification) in wire.into_iter().enumerate() {
            send_json(&mut writer, notification).await;
            observed_rx
                .wait_for(|count| *count > baseline + index)
                .await
                .unwrap();
        }
        let updates = timeline_updates(&sink);
        assert!(
            updates
                .iter()
                .all(|update| update["toolCallId"] != "child-tool"
                    && update["content"]["text"] != "Looking"
                    && update["sessionUpdate"] != "session_message"),
            "child traffic never enters the parent timeline: {updates:?}"
        );
        assert!(
            !sink
                .hub
                .config_options(SESSION)
                .unwrap_or_default()
                .to_string()
                .contains("child-model"),
            "a child's configuration is not the parent's"
        );
        assert_eq!(
            updates
                .iter()
                .filter(|update| update["sessionUpdate"] == subagents::SUBAGENT_UPDATE)
                .count(),
            1,
            "one durable row per child: {updates:?}"
        );
        let row = subagent_row(&sink, CHILD).expect("subagent row");
        assert_eq!(row["title"], "Explore");
        assert_eq!(row["state"], "running");
        assert_eq!(row["cancellable"], true);
        assert_eq!(row["prompt"], "Find the flaky test");
        assert_eq!(row["toolCalls"], 1);
        assert_eq!(row["activity"][0]["title"], "Read tests.rs");
        assert_eq!(row["activity"][1]["text"], "Looking");
        assert!(
            updates
                .iter()
                .any(|update| update["content"]["text"] == "Parent waits.")
        );
        assert_eq!(
            sink.hub
                .session_info(SESSION)
                .unwrap()
                .meta
                .background_tasks,
            1,
            "a working subagent is activity"
        );

        command_tx
            .send(AgentCommand::CancelSubagent {
                subagent_session_id: CHILD.to_owned(),
            })
            .unwrap();
        let cancel = next().await;
        assert_eq!(cancel["method"], "session/cancel");
        assert_eq!(
            cancel["params"]["sessionId"], CHILD,
            "only the child is cancelled"
        );
        let baseline = *observed_rx.borrow();
        for (index, notification) in [
            update_of(
                NATIVE,
                json!({
                    "sessionUpdate": "subagent_update", "sessionId": CHILD,
                    "state": {"state": "idle", "stopReason": "cancelled"},
                }),
            ),
            update_of(
                NATIVE,
                json!({
                    "sessionUpdate": "subagent_update", "sessionId": CHILD,
                    "capabilities": {"cancel": null},
                }),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            send_json(&mut writer, notification).await;
            observed_rx
                .wait_for(|count| *count > baseline + index)
                .await
                .unwrap();
        }
        let row = subagent_row(&sink, CHILD).expect("subagent row");
        assert_eq!(row["state"], "idle");
        assert_eq!(row["stopReason"], "cancelled");
        assert_eq!(row["cancellable"], false);
        assert_eq!(
            sink.hub
                .session_info(SESSION)
                .unwrap()
                .meta
                .background_tasks,
            0
        );
        // A stop for a child that no longer works never reaches the adapter.
        command_tx
            .send(AgentCommand::CancelSubagent {
                subagent_session_id: CHILD.to_owned(),
            })
            .unwrap();
        command_tx
            .send(AgentCommand::CancelSubagent {
                subagent_session_id: NATIVE.to_owned(),
            })
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), lines.next_line())
                .await
                .is_err(),
            "no session/cancel for an idle or unknown subagent"
        );
        send_json(
            &mut writer,
            json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}}),
        )
        .await;
        while sink.prompt_completions.lock().is_empty() {
            tokio::task::yield_now().await;
        }
        drop(command_tx);
        phase.wait_for(|_| false).await.unwrap_err();
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let (result, ()) = tokio::join!(client, peer);
        assert!(result.is_ok(), "{result:?}");
    })
    .await
    .expect("bounded subagent fixture");
}

#[test]
fn a_child_request_names_its_subagent_and_a_root_request_does_not() {
    let (state, _sink) = activity_fixture("fixture-provider");
    state.subagents.lock().route(
        NATIVE,
        &json!({"sessionUpdate": "subagent_update", "sessionId": CHILD, "title": "Reviewer"}),
        std::time::Instant::now(),
        1,
    );
    let mut child = json!({"toolCallId": "t", "title": "Run tests"});
    tag_subagent_request(&state, CHILD, &mut child);
    assert_eq!(
        child["_cowboy"]["subagent"],
        json!({"sessionId": CHILD, "title": "Reviewer"})
    );
    let mut elicitation = json!({"title": "Pick", "_cowboy": {"elicitation": {"mode": "form"}}});
    tag_subagent_request(&state, CHILD, &mut elicitation);
    assert_eq!(elicitation["_cowboy"]["elicitation"]["mode"], "form");
    assert_eq!(elicitation["_cowboy"]["subagent"]["title"], "Reviewer");
    let mut root = json!({"toolCallId": "t", "title": "Run tests"});
    tag_subagent_request(&state, NATIVE, &mut root);
    assert!(root.get("_cowboy").is_none());
}

#[test]
fn an_agent_completed_url_elicitation_is_accepted_and_resolved_once() {
    let (state, sink) = fixture_state(false);
    let (tx, mut rx) = oneshot::channel::<Option<String>>();
    state.pending.lock().insert("7".to_owned(), tx);
    state.elicitations.lock().insert("7".to_owned());
    state
        .url_elicitations
        .lock()
        .insert("login-1".to_owned(), "7".to_owned());

    assert!(!complete_url_elicitation(&state, "unknown"));
    assert!(complete_url_elicitation(&state, "login-1"));
    assert_eq!(rx.try_recv().unwrap(), Some(ELICITATION_ACCEPT.to_owned()));
    assert!(state.pending.lock().is_empty());
    assert!(state.elicitations.lock().is_empty());
    assert!(state.url_elicitations.lock().is_empty());
    // A repeated notification, or one after the user answered, is a no-op.
    assert!(!complete_url_elicitation(&state, "login-1"));

    let resolved: Vec<_> = sink
        .hub
        .snapshot(SESSION)
        .unwrap()
        .0
        .into_iter()
        .filter_map(|envelope| match envelope.event {
            Event::PermissionResolved {
                request_id,
                option_id,
            } => Some((request_id, option_id)),
            _ => None,
        })
        .collect();
    assert_eq!(
        resolved,
        vec![("7".to_owned(), Some(ELICITATION_ACCEPT.to_owned()))]
    );
}

#[test]
fn an_answered_url_elicitation_forgets_its_completion_id() {
    let (state, _sink) = fixture_state(false);
    state
        .url_elicitations
        .lock()
        .insert("login-2".to_owned(), "9".to_owned());
    forget_url_elicitation(&state, "9");
    assert!(state.url_elicitations.lock().is_empty());
}
