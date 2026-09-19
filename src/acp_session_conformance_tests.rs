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
        activity: Mutex::new(SessionActivity::default()),
        prompt_cancellation,
        codex_full_access: AtomicBool::new(false),
        grok_permission_mode: Arc::new(Mutex::new(GrokPermissionMode::AlwaysApprove)),
        suppress_updates: AtomicBool::new(false),
        last_echoed_user_contents: Mutex::new(Vec::new()),
        last_progress: Mutex::new(std::time::Instant::now()),
        open_tools: Mutex::new(std::collections::HashSet::new()),
        cgroup: None,
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
                    None,
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
                {"type": "system", "subtype": "background_tasks_changed"}
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
                    None,
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
        let failures = events.iter().filter(|event| matches!(&event.event, Event::TurnEnd { stop_reason } if stop_reason.starts_with("error:"))).count();
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
