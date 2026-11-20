//! Trusted process ledger of a sandboxed session's execution endpoint
//! (docs/antigravity-real-accounts-design.md, "Revision 5 simplification").
//!
//! The endpoint relays every request between the account unit and the target
//! keeper, so it, not the confined proxy, is where target effects are
//! admitted. Each `process/start` is recorded under the current Cowboy turn
//! before it is forwarded; process starts and filesystem mutations outside an
//! active turn are refused. The one exception is the projection's readiness
//! probe while the worker is starting its session: the worker's own state
//! opens that window, never a caller-chosen id. A turn is fenced when the
//! worker sees its cancellation, when it ends and at teardown: later effects
//! for it are refused, its admitted requests drain before its completion is
//! reported, and its recorded processes are terminated best-effort. Nothing a
//! dead worker left is inherited; the target keeper's unattended reap is the
//! backstop. A request whose reply was lost after it may have been admitted
//! makes the session's effect uncertain, durably, until the owner
//! acknowledges that exact sequence.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;

use super::Client;

pub use crate::account_sandbox::EXECUTION_LEDGER_ENV as LEDGER_ENV;
const SCHEMA: u16 = 1;
/// Live processes one session may hold; a start beyond it is refused.
pub const MAX_PROCESSES: usize = 256;
const MAX_LEDGER_BYTES: u64 = 256 * 1024;
const MAX_FENCED_TURNS: usize = 64;
/// The ledger turn the readiness probe is recorded under.
const READINESS_TURN: &str = "cowboy-readiness";
/// The ledger owner of the session's MCP servers on the target (Cowboy MCP
/// and the folder's bindings): the worker starts them itself, they outlive
/// turns and end at teardown. The unit can neither start nor reach them.
const SERVER_TURN: &str = "cowboy-session-servers";
/// Process ids of worker-run servers (and their probes), in every worker of
/// the session: the unit may never name one, also one a previous worker
/// left running.
pub(super) const SERVER_PREFIX: &str = "cowboy-worker-mcp-";
/// Whether an execution event concerns a worker-run server or probe.
pub(super) fn worker_event(message: &Value) -> bool {
    message
        .pointer("/params/processId")
        .and_then(Value::as_str)
        .is_some_and(|process| process.starts_with(SERVER_PREFIX))
}

/// Extension calls one session may have admitted at once.
const MAX_CALLS: usize = 8;
/// How long after a turn ends memory may still receive its evidence (the
/// memory client delivers in the background).
const OBSERVATION_GRACE: Duration = Duration::from_secs(120);
/// The endpoint's own request: the agent side reports an uncertain effect
/// it detected (`UNCERTAIN_REPORT` in the Antigravity facade). Never
/// forwarded to the target.
pub const UNCERTAIN_REPORT: &str = "cowboy/uncertainEffect";

/// The only command the readiness window admits, byte for byte the
/// projection's Git probe (`READINESS_PROBE` in the Antigravity proxy's
/// `projection.mjs`; a package test keeps both equal):
/// `/bin/sh -c READINESS_SCRIPT sh <absolute cwd>`.
pub const READINESS_SCRIPT: &str = r#"cd -- "$1" 2>/dev/null || exit 0
top=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
printf 'top\t%s\n' "$top"
printf 'branch\t%s\n' "$(git symbolic-ref --short -q HEAD || git rev-parse --short HEAD 2>/dev/null)"
git -c core.quotePath=true status --short 2>/dev/null | head -n 201 | sed 's/^/status\t/'
"#;

/// Whether `params` start exactly the readiness probe.
fn readiness_probe(params: &Value) -> bool {
    let Some(argv) = params.get("argv").and_then(Value::as_array) else {
        return false;
    };
    let argv: Vec<&str> = argv.iter().filter_map(Value::as_str).collect();
    // The unit names no environment, search path or interpreter of its own:
    // the probe runs with the keeper's environment, as the projection sends.
    let closed = params.get("env").is_none_or(|env| env == &json!({}))
        && params.get("arg0").is_none_or(Value::is_null)
        && params.get("tty").is_none_or(|tty| tty == &json!(false))
        && params.get("envPolicy").is_none_or(|policy| {
            policy.get("set").is_none_or(|set| set == &json!({}))
                && policy
                    .get("includeOnly")
                    .is_none_or(|only| only == &json!([]))
        });
    closed
        && matches!(
            argv.as_slice(),
            ["/bin/sh", "-c", script, "sh", cwd]
                if *script == READINESS_SCRIPT && cwd.starts_with('/')
        )
        && params.get("argv").and_then(Value::as_array).map(Vec::len) == Some(5)
}

/// Bound of one process's best-effort termination.
const TERMINATE_BOUND: Duration = Duration::from_secs(5);
/// Requests that change the target: admitted only inside an active turn,
/// drained before the turn's completion, and uncertain when their reply is
/// lost.
const MUTATIONS: &[&str] = &[
    "fs/writeFile",
    "fs/remove",
    "fs/copy",
    "fs/createDirectory",
    "process/start",
];

/// The durable part of the ledger: the session's uncertain effect, so a
/// replacement worker keeps reporting it until the owner acknowledges it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    schema: u16,
    session_id: String,
    /// The latest request whose reply was lost after possible admission.
    uncertain_effect: Option<String>,
    /// How many requests lost their reply; the Controller parks the
    /// session's queue until the owner acknowledged this sequence.
    #[serde(default)]
    uncertain_sequence: u64,
}

#[derive(Default)]
struct State {
    record: Record,
    /// Recorded target processes and the turn that started each.
    processes: BTreeMap<String, String>,
    /// Starts not yet answered by the target.
    starting: HashSet<String>,
    /// Admitted requests still running, per turn.
    admitted: HashMap<String, usize>,
    current_turn: Option<String>,
    fenced: VecDeque<String>,
    /// The worker is starting its session: the readiness probe may run.
    readiness: bool,
    /// Processes started as readiness probes (bounded by the probes the
    /// readiness window admits): the only processes whose output carries no
    /// result the agent acts on. Kept after their exit.
    probes: HashSet<String>,
    /// Target processes the worker started as the session's MCP servers.
    servers: HashSet<String>,
    /// Extension calls admitted and not yet settled.
    calls: usize,
    /// When the last turn was fenced (its evidence may still be delivered).
    turn_ended: Option<std::time::Instant>,
    closed: bool,
}

impl State {
    fn fenced(&self, turn: &str) -> bool {
        self.closed || self.fenced.iter().any(|fenced| fenced == turn)
    }

    /// The turn a new target effect belongs to, if one is active.
    fn active_turn(&self) -> Option<String> {
        self.current_turn.clone().filter(|turn| !self.fenced(turn))
    }
}

/// Told the method and sequence of each request whose effect became unknown.
pub type Observer = Box<dyn Fn(&str, u64) + Send + Sync>;

pub struct Ledger {
    path: Option<PathBuf>,
    state: Mutex<State>,
    changed: watch::Sender<u64>,
    runtime: tokio::runtime::Handle,
    observer: OnceLock<Observer>,
}

impl std::fmt::Debug for Ledger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ledger").field("path", &self.path).finish()
    }
}

/// One admitted request: settled when its relay finished.
pub(super) struct Admitted {
    turn: String,
}

impl Ledger {
    /// Open the session's ledger on the current runtime. Only the uncertain
    /// effect survives a worker; processes are never inherited. The ledger
    /// starts in the readiness window of a starting worker.
    pub fn open(session_id: &str, path: Option<PathBuf>) -> Result<Arc<Self>> {
        let mut record = match &path {
            Some(path) => read(path)?.unwrap_or_default(),
            None => Record::default(),
        };
        ensure!(
            record.session_id.is_empty() || record.session_id == session_id,
            "execution ledger belongs to another session"
        );
        record.schema = SCHEMA;
        session_id.clone_into(&mut record.session_id);
        Ok(Arc::new(Self {
            path,
            state: Mutex::new(State {
                record,
                readiness: true,
                ..State::default()
            }),
            changed: watch::Sender::new(0),
            runtime: tokio::runtime::Handle::current(),
            observer: OnceLock::new(),
        }))
    }

    /// Called with the request's method and sequence whenever the session
    /// holds a newer uncertain effect.
    pub fn set_observer(&self, observer: Observer) {
        let _ = self.observer.set(observer);
    }

    /// The worker's session state: the readiness probe may run only while
    /// the worker is starting its session.
    pub fn set_readiness(&self, starting: bool) {
        self.state.lock().readiness = starting;
    }

    /// Record a session MCP server the worker started: ended at teardown,
    /// never by a turn, and out of the unit's reach.
    pub(super) fn record_server(&self, process: &str) -> Result<(), &'static str> {
        let mut state = self.state.lock();
        if state.closed {
            return Err("the session is ending");
        }
        if state.processes.len() >= MAX_PROCESSES {
            return Err("session process limit reached; process start refused");
        }
        state.servers.insert(process.to_owned());
        state
            .processes
            .insert(process.to_owned(), SERVER_TURN.to_owned());
        // Starting until the target answers, so teardown waits for it.
        state.starting.insert(process.to_owned());
        Ok(())
    }

    /// The target answered a server's start (`started`: it runs).
    pub(super) fn server_started(&self, process: &str, started: bool) {
        let mut state = self.state.lock();
        state.starting.remove(process);
        if !started {
            state.processes.remove(process);
        }
        drop(state);
        self.changed.send_modify(|value| *value += 1);
    }

    /// The worker's servers and probes (teardown ends them first).
    fn server_processes(&self) -> Vec<String> {
        let state = self.state.lock();
        state
            .processes
            .iter()
            .filter(|(_, owner)| *owner == SERVER_TURN)
            .map(|(process, _)| process.clone())
            .collect()
    }

    /// Whether `turn` is still the active turn (an admitted call queued
    /// behind another may not reach the target once it ended).
    pub(super) fn turn_open(&self, turn: &str) -> bool {
        self.state.lock().active_turn().as_deref() == Some(turn)
    }

    /// Memory may receive evidence during a turn and shortly after it.
    pub(super) fn observation_open(&self) -> bool {
        let state = self.state.lock();
        !state.closed
            && (state.active_turn().is_some()
                || state
                    .turn_ended
                    .is_some_and(|ended| ended.elapsed() < OBSERVATION_GRACE))
    }

    /// Admit one extension tool call under the active turn: refused outside
    /// a turn, drained before the turn's completion.
    pub(super) fn admit_call(&self) -> Result<Admitted, &'static str> {
        let mut state = self.state.lock();
        let turn = state
            .active_turn()
            .ok_or("no Cowboy turn is running; the tool call was refused")?;
        if state.calls >= MAX_CALLS {
            return Err("too many extension calls are running; the tool call was refused");
        }
        state.calls += 1;
        *state.admitted.entry(turn.clone()).or_default() += 1;
        Ok(Admitted { turn })
    }

    /// An admitted extension call finished.
    pub(super) fn settle_call(&self, admitted: &Admitted) {
        self.state.lock().calls -= 1;
        self.settled(admitted, None);
    }

    pub(super) fn admitted_turn(admitted: &Admitted) -> &str {
        &admitted.turn
    }

    pub fn begin_turn(&self, turn: &str) {
        let mut state = self.state.lock();
        state.current_turn = Some(turn.to_owned());
        state.readiness = false;
    }

    #[must_use]
    pub fn current_turn(&self) -> Option<String> {
        self.state.lock().current_turn.clone()
    }

    /// The method of the latest request whose effect is unknown.
    #[must_use]
    pub fn uncertain_effect(&self) -> Option<String> {
        self.state.lock().record.uncertain_effect.clone()
    }

    /// How many requests of this session lost their reply.
    #[must_use]
    pub fn uncertain_sequence(&self) -> u64 {
        self.state.lock().record.uncertain_sequence
    }

    #[must_use]
    pub fn live(&self) -> Vec<String> {
        self.state.lock().processes.keys().cloned().collect()
    }

    /// Admit one request before it is forwarded, recording a start, or say
    /// why it is refused. Requests that change nothing pass unrecorded.
    pub(super) fn admit(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<Option<Admitted>, &'static str> {
        let mut state = self.state.lock();
        // The worker's own MCP servers are not the unit's processes, also
        // those an earlier worker of the session started.
        if process_id(params).is_some_and(|process| {
            state.servers.contains(&process) || process.starts_with(SERVER_PREFIX)
        }) {
            return Err("that process belongs to the worker; request refused");
        }
        if !MUTATIONS.contains(&method) {
            return Ok(None);
        }
        if method != "process/start" {
            let turn = state
                .active_turn()
                .ok_or("no Cowboy turn is running; the filesystem change was refused")?;
            *state.admitted.entry(turn.clone()).or_default() += 1;
            return Ok(Some(Admitted { turn }));
        }
        let process = process_id(params)
            .filter(|process| !process.is_empty() && process.len() <= 256)
            .ok_or("process start needs a valid processId")?;
        let turn = match state.active_turn() {
            Some(turn) => turn,
            None if state.readiness && !state.closed => {
                // The readiness probe: exactly the projection's Git probe,
                // one at a time, never during a turn.
                if !readiness_probe(params) {
                    return Err(
                        "only the readiness probe may start before a turn; process start refused",
                    );
                }
                let busy = state.processes.values().any(|turn| turn == READINESS_TURN)
                    || state.admitted.contains_key(READINESS_TURN);
                if busy {
                    return Err("a readiness probe is already running; process start refused");
                }
                READINESS_TURN.to_owned()
            }
            None if state.current_turn.is_some() || state.closed => {
                return Err("the turn was cancelled or ended; process start refused");
            }
            None => return Err("no Cowboy turn is running; process start refused"),
        };
        if state.processes.contains_key(&process) || state.starting.contains(&process) {
            return Err("process id is already recorded; process start refused");
        }
        if state.processes.len() >= MAX_PROCESSES {
            return Err("session process limit reached; process start refused");
        }
        if turn == READINESS_TURN {
            state.probes.insert(process.clone());
        }
        state.processes.insert(process.clone(), turn.clone());
        state.starting.insert(process);
        *state.admitted.entry(turn.clone()).or_default() += 1;
        Ok(Some(Admitted { turn }))
    }

    /// An admitted request finished (answered, refused or lost).
    pub(super) fn settled(&self, admitted: &Admitted, start: Option<&str>) {
        let mut state = self.state.lock();
        if let Some(process) = start {
            state.starting.remove(process);
        }
        if let Some(count) = state.admitted.get_mut(&admitted.turn) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.admitted.remove(&admitted.turn);
            }
        }
        drop(state);
        self.changed.send_modify(|value| *value += 1);
    }

    /// The target's evidence that `process` is gone.
    pub fn exited(&self, process: &str) {
        if self.state.lock().processes.remove(process).is_some() {
            self.changed.send_modify(|value| *value += 1);
        }
    }

    /// Whether `process` was not a readiness probe: its output may carry a
    /// result the agent acts on (a publish's outcome), also after its exit
    /// was observed. Unknown processes count too (fail closed).
    fn result_process(&self, process: &str) -> bool {
        !self.state.lock().probes.contains(process)
    }

    pub(super) fn mark_uncertain(&self, method: &str) {
        let mut state = self.state.lock();
        state.record.uncertain_effect = Some(method.to_owned());
        state.record.uncertain_sequence = state.record.uncertain_sequence.saturating_add(1);
        let sequence = state.record.uncertain_sequence;
        if let Err(error) = self.persist(&state.record) {
            tracing::warn!(%error, "execution ledger update failed");
        }
        drop(state);
        if let Some(observer) = self.observer.get() {
            observer(method, sequence);
        }
    }

    /// Refuse every later effect of `turn` (every turn when `None`);
    /// returns the processes to end.
    fn fence(&self, turn: Option<&str>) -> Vec<String> {
        let mut state = self.state.lock();
        match turn {
            Some(turn) => {
                state.turn_ended = Some(std::time::Instant::now());
                if !state.fenced.iter().any(|fenced| fenced == turn) {
                    if state.fenced.len() >= MAX_FENCED_TURNS {
                        state.fenced.pop_front();
                    }
                    state.fenced.push_back(turn.to_owned());
                }
            }
            None => {
                state.closed = true;
                state.readiness = false;
            }
        }
        state
            .processes
            .iter()
            .filter(|(_, owner)| turn.is_none_or(|turn| *owner == turn))
            .map(|(process, _)| process.clone())
            .collect()
    }

    fn drained(&self, turn: Option<&str>) -> bool {
        let state = self.state.lock();
        match turn {
            Some(turn) => !state.admitted.contains_key(turn),
            None => state.admitted.is_empty(),
        }
    }

    fn starting(&self, process: &str) -> bool {
        self.state.lock().starting.contains(process)
    }

    /// Wait until `ready` holds or the ledger changes.
    async fn wait_until(&self, ready: impl Fn(&Self) -> bool) {
        let mut changed = self.changed.subscribe();
        while !ready(self) {
            if changed.changed().await.is_err() {
                return;
            }
        }
    }

    fn persist(&self, record: &Record) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(record)?;
        let parent = path.parent().context("execution ledger has no directory")?;
        let temporary = parent.join(format!(
            ".{}.{:016x}.partial",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("ledger"),
            rand::random::<u64>()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}

fn read(path: &Path) -> Result<Option<Record>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.len() <= MAX_LEDGER_BYTES,
        "execution ledger exceeds its size limit"
    );
    let record: Record = serde_json::from_reader(file).context("invalid execution ledger")?;
    ensure!(
        record.schema == SCHEMA,
        "unsupported execution ledger schema"
    );
    Ok(Some(record))
}

/// A refused native request; nothing was forwarded.
fn refused(message: &str) -> Value {
    json!({"error": {"code": -32000, "message": message}})
}

/// The request may have been admitted before its reply was lost: its effect
/// is unknown, typed so the agent side never reads it as a refusal.
fn unavailable() -> Value {
    json!({"error": {
        "code": -32000,
        "message": "Execution unavailable or result unknown; no local fallback and no replay",
        "data": {"outcome": "unknown"},
    }})
}

fn exited(reply: &Value) -> bool {
    reply.pointer("/result/exited") == Some(&Value::Bool(true))
}

fn unknown_process(reply: &Value) -> bool {
    reply.get("error").is_some_and(|error| {
        let text = error.to_string().to_ascii_lowercase();
        ["unknown process", "not found", "no such process"]
            .iter()
            .any(|needle| text.contains(needle))
    })
}

fn process_id(params: &Value) -> Option<String> {
    params
        .get("processId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl Client {
    pub fn set_ledger(&self, ledger: Arc<Ledger>) {
        let _ = self.ledger.set(ledger);
    }

    #[must_use]
    pub fn ledger(&self) -> Option<&Arc<Ledger>> {
        self.ledger.get()
    }

    /// Bridge memory requests of the sandboxed unit to the session's relay.
    pub fn set_memory(&self, memory: super::memory_bridge::MemoryBridge) {
        let _ = self.memory.set(memory);
    }

    /// Serve the session's extension servers to the sandboxed unit.
    pub fn set_extensions(&self, broker: Arc<super::extension_broker::Broker>) {
        let _ = self.extensions.set(broker);
    }

    /// Relay one native request from the agent side. Without a ledger this
    /// is the plain invocation; with one, effects are admitted (and starts
    /// recorded) first and lost replies mark the session's effect uncertain.
    pub async fn relay(self: &Arc<Self>, method: String, params: Value) -> Value {
        let Some(ledger) = self.ledger().cloned() else {
            if method == UNCERTAIN_REPORT {
                return refused("no execution ledger");
            }
            return self
                .invoke(method, params)
                .await
                .unwrap_or_else(|_| unavailable());
        };
        if method == super::memory_bridge::MEMORY_REQUEST {
            if params.get("path").and_then(Value::as_str) == Some("/v1/observe")
                && !ledger.observation_open()
            {
                return refused("memory evidence is accepted only for a turn");
            }
            return match self.memory.get() {
                Some(memory) => memory.forward(&params).await,
                None => refused("memory is not available in this session"),
            };
        }
        if method == super::extension_broker::EXTENSIONS_REQUEST {
            return match self.extensions.get() {
                Some(broker) => broker.list().await,
                None => json!({"result": {"tools": [], "instructions": ""}}),
            };
        }
        if method == super::extension_broker::EXTENSION_CALL {
            return match self.extensions.get().cloned() {
                Some(broker) => broker.call(self, &params).await,
                None => refused("this session has no extensions"),
            };
        }
        if method == UNCERTAIN_REPORT {
            // The facade detected an unknown outcome every transport call
            // of which was answered (a timed-out publish): recorded like a
            // lost reply. Reporting one only parks the session (fail
            // closed), so the agent side may.
            let reported = params
                .get("method")
                .and_then(Value::as_str)
                .filter(|reported| {
                    !reported.is_empty()
                        && reported.len() <= 64
                        && reported
                            .bytes()
                            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
                })
                .unwrap_or("request");
            ledger.mark_uncertain(reported);
            return json!({"result": {}});
        }
        let admitted = match ledger.admit(&method, &params) {
            Ok(admitted) => admitted,
            Err(reason) => return refused(reason),
        };
        let process = process_id(&params);
        let start = (method == "process/start")
            .then(|| process.clone())
            .flatten();
        let result_carrying = self.carries_result(&method, &params);
        let result = self.invoke_classified(method.clone(), params).await;
        if let Some(admitted) = &admitted {
            ledger.settled(admitted, start.as_deref());
        }
        match result {
            Ok(reply) => {
                if let Some(process) = &process
                    && ((method == "process/read" && exited(&reply))
                        || (start.is_some() && reply.get("error").is_some()))
                {
                    // An exit, or a refused start that started nothing.
                    ledger.exited(process);
                }
                reply
            }
            Err(admitted) => {
                if !admitted {
                    // Refused before it was sent: nothing can have happened.
                    if let Some(process) = &start {
                        ledger.exited(process);
                    }
                    return refused("Execution unavailable; the request was not sent");
                }
                if MUTATIONS.contains(&method.as_str()) || result_carrying {
                    ledger.mark_uncertain(&method);
                }
                unavailable()
            }
        }
    }

    /// Whether a lost reply of this request hides a result the agent acts
    /// on: a termination, or a read of any process but a readiness probe
    /// (a publish's outcome; also the final read after its exit).
    pub(super) fn carries_result(&self, method: &str, params: &Value) -> bool {
        let Some(ledger) = self.ledger() else {
            return false;
        };
        method == "process/terminate"
            || (method == "process/read"
                && process_id(params).is_none_or(|process| ledger.result_process(&process)))
    }

    /// The reply of `method` never reached the agent side. A mutation, or a
    /// result-carrying request ([`Self::carries_result`]), the target
    /// answered (or whose outcome is already unknown) then has an effect
    /// the agent never learned: the session's effect is uncertain.
    pub(super) fn undelivered(&self, method: &str, carries_result: bool, reply: &Value) {
        let Some(ledger) = self.ledger() else {
            return;
        };
        let answered = reply.get("result").is_some()
            || reply.pointer("/error/data/outcome") == Some(&json!("unknown"));
        if (MUTATIONS.contains(&method) || carries_result) && answered {
            ledger.mark_uncertain(method);
        }
        // An extension call that ran but whose result never reached the
        // unit (an unknown outcome was already recorded by the broker).
        if method == super::extension_broker::EXTENSION_CALL && reply.get("result").is_some() {
            ledger.mark_uncertain("tools/call");
        }
    }

    /// Exit notifications the agent side receives are evidence too.
    pub fn observe_event(&self, message: &Value) {
        if let Some(ledger) = self.ledger()
            && message.get("method").and_then(Value::as_str) == Some("process/exited")
            && let Some(process) = message.get("params").and_then(process_id)
        {
            ledger.exited(&process);
        }
    }

    /// Fence the current turn after its cancellation and end its processes
    /// in the background, best-effort.
    pub fn cancel_turn(self: &Arc<Self>) {
        let Some(ledger) = self.ledger() else {
            return;
        };
        let Some(turn) = ledger.current_turn() else {
            return;
        };
        let processes = ledger.fence(Some(&turn));
        tracing::info!(event_name = "cowboy.execution.turn_fenced", session = %self.session, %turn, processes = processes.len(), reason = "cancelled", "execution turn fenced");
        let client = Arc::clone(self);
        ledger.runtime.spawn(async move {
            client.terminate_all(processes).await;
        });
    }

    /// Fence an ended turn. When it still has admitted requests or recorded
    /// processes, the returned future resolves once its admitted requests
    /// drained and its processes were terminated best-effort; the turn's
    /// completion is reported after it.
    pub fn finish_turn(
        self: &Arc<Self>,
        turn: &str,
    ) -> Option<impl std::future::Future<Output = ()> + Send + 'static> {
        let ledger = Arc::clone(self.ledger()?);
        let processes = ledger.fence(Some(turn));
        if processes.is_empty() && ledger.drained(Some(turn)) {
            return None;
        }
        tracing::info!(event_name = "cowboy.execution.turn_fenced", session = %self.session, %turn, processes = processes.len(), reason = "turn_end", "execution turn fenced");
        let (client, turn) = (Arc::clone(self), turn.to_owned());
        Some(async move {
            ledger
                .wait_until(|ledger| ledger.drained(Some(&turn)))
                .await;
            // Starts admitted before the fence was raised are recorded too.
            let processes = ledger.fence(Some(&turn));
            client.terminate_all(processes).await;
        })
    }

    /// Fence every turn, drain and end every recorded process best-effort:
    /// teardown.
    pub async fn settle_all(self: &Arc<Self>) {
        if let Some(ledger) = self.ledger().cloned() {
            ledger.fence(None);
            // The worker's servers end first: a call still running on one
            // then ends too (its outcome is unknown) instead of holding the
            // shutdown past its deadline. A start in flight is waited for.
            ledger
                .wait_until(|ledger| {
                    let state = ledger.state.lock();
                    !state
                        .starting
                        .iter()
                        .any(|process| state.servers.contains(process))
                })
                .await;
            self.terminate_all(ledger.server_processes()).await;
            ledger.wait_until(|ledger| ledger.drained(None)).await;
            let processes = ledger.fence(None);
            self.terminate_all(processes).await;
        }
    }

    /// Resolve the spawned settlement of `future` on the ledger's runtime.
    pub fn spawn_settled(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        if let Some(ledger) = self.ledger() {
            ledger.runtime.spawn(future);
        }
    }

    async fn terminate_all(self: &Arc<Self>, processes: Vec<String>) {
        futures::future::join_all(processes.into_iter().map(|process| self.terminate(process)))
            .await;
    }

    /// Ask the target to terminate `process` and observe its end, within a
    /// bound. A process that does not end in time stays recorded; the
    /// target keeper's unattended reap is the backstop.
    async fn terminate(self: &Arc<Self>, process: String) {
        let Some(ledger) = self.ledger().cloned() else {
            return;
        };
        let attempt = async {
            ledger.wait_until(|ledger| !ledger.starting(&process)).await;
            let _ = self
                .invoke(
                    "process/terminate".to_owned(),
                    json!({"processId": process}),
                )
                .await;
            loop {
                let read = self
                    .invoke(
                        "process/read".to_owned(),
                        json!({"processId": process, "afterSeq": null, "maxBytes": 1, "waitMs": 1000}),
                    )
                    .await;
                match read {
                    Ok(reply) if exited(&reply) || unknown_process(&reply) => {
                        ledger.exited(&process);
                        return;
                    }
                    Ok(_) => {}
                    Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
                }
            }
        };
        if tokio::time::timeout(TERMINATE_BOUND, attempt)
            .await
            .is_err()
        {
            tracing::warn!(event_name = "cowboy.execution.terminate_unconfirmed", session = %self.session, %process, "target process end unconfirmed; the keeper's reap is the backstop");
        }
    }
}
