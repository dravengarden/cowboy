//! Trusted process ledger of a sandboxed session's execution endpoint
//! (docs/antigravity-real-accounts-design.md, amendments R3-5 and R3-6).
//!
//! The endpoint relays every request between the account unit and the target
//! keeper, so it, not the confined proxy, is where process starts are
//! accounted. Each `process/start` is recorded under the current Cowboy turn,
//! durably, before it is forwarded. A turn is fenced when the worker sees its
//! cancellation, when it ends, when the unit is lost and at teardown: later
//! starts for it are refused and every recorded process is terminated, with
//! completion observed through `process/read` (an exit notification may
//! already have been consumed by the agent). A process counts as gone only on
//! the target's exit evidence, or when the target reports its id unknown while
//! no start of it is in flight. Entries a dead worker left behind are
//! terminated by the next worker of the same binding.

use std::collections::{BTreeMap, HashSet, VecDeque};
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
/// Process ids of the facade's bounded readiness probes (the projection's Git
/// state). They run outside any turn, one at a time, and are recorded and
/// ended like every other process.
pub const READINESS_PREFIX: &str = "cowboy-agy-readiness-";
/// The ledger turn readiness probes are recorded under.
const READINESS_TURN: &str = "cowboy-readiness";
const RETRY_INITIAL: Duration = Duration::from_millis(250);
const RETRY_MAX: Duration = Duration::from_secs(15);
/// Requests whose lost reply leaves an unknown effect on the target.
const MUTATIONS: &[&str] = &[
    "fs/writeFile",
    "fs/remove",
    "fs/copy",
    "fs/createDirectory",
    "process/start",
];

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u16,
    session_id: String,
    /// The latest mutation whose reply was lost after possible admission.
    uncertain_effect: Option<String>,
    /// How many mutations lost their reply; the Controller parks the
    /// session's queue until the owner acknowledged this sequence.
    #[serde(default)]
    uncertain_sequence: u64,
    processes: BTreeMap<String, Recorded>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recorded {
    turn: String,
    /// Written by a previous worker: every turn of this one waits for it.
    #[serde(default)]
    recovered: bool,
}

#[derive(Default)]
struct State {
    record: Record,
    in_flight: HashSet<String>,
    terminating: HashSet<String>,
    current_turn: Option<String>,
    fenced: VecDeque<String>,
    closed: bool,
}

impl State {
    fn fenced(&self, turn: &str) -> bool {
        self.closed || self.fenced.iter().any(|fenced| fenced == turn)
    }
}

/// Told the method of the first mutation whose effect became unknown.
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

impl Ledger {
    /// Open the session's ledger on the current runtime. Entries a previous
    /// worker left are marked recovered; the caller terminates them.
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
        for entry in record.processes.values_mut() {
            entry.recovered = true;
        }
        Ok(Arc::new(Self {
            path,
            state: Mutex::new(State {
                record,
                ..State::default()
            }),
            changed: watch::Sender::new(0),
            runtime: tokio::runtime::Handle::current(),
            observer: OnceLock::new(),
        }))
    }

    /// Called once, with the request's method, when the session first holds
    /// an uncertain effect.
    pub fn set_observer(&self, observer: Observer) {
        let _ = self.observer.set(observer);
    }

    pub fn begin_turn(&self, turn: &str) {
        self.state.lock().current_turn = Some(turn.to_owned());
    }

    #[must_use]
    pub fn current_turn(&self) -> Option<String> {
        self.state.lock().current_turn.clone()
    }

    /// The method of the latest mutation whose effect is unknown.
    #[must_use]
    pub fn uncertain_effect(&self) -> Option<String> {
        self.state.lock().record.uncertain_effect.clone()
    }

    /// How many mutations of this session lost their reply.
    #[must_use]
    pub fn uncertain_sequence(&self) -> u64 {
        self.state.lock().record.uncertain_sequence
    }

    #[must_use]
    pub fn live(&self) -> Vec<String> {
        self.state.lock().record.processes.keys().cloned().collect()
    }

    /// Record a start before it is forwarded, or say why it is refused.
    pub fn admit_start(&self, process: &str) -> Result<(), &'static str> {
        let mut state = self.state.lock();
        let readiness = process.starts_with(READINESS_PREFIX);
        let turn = if readiness {
            // Independently tracked: never part of a turn, one at a time.
            if state
                .record
                .processes
                .values()
                .any(|entry| entry.turn == READINESS_TURN)
            {
                return Err("a readiness probe is already running; process start refused");
            }
            READINESS_TURN.to_owned()
        } else {
            let Some(turn) = state.current_turn.clone() else {
                return Err("no Cowboy turn is running; process start refused");
            };
            turn
        };
        if state.fenced(&turn) {
            return Err("the turn was cancelled or ended; process start refused");
        }
        if process.is_empty() || process.len() > 256 {
            return Err("process start needs a valid processId");
        }
        if state.record.processes.contains_key(process) || state.in_flight.contains(process) {
            return Err("process id is already recorded; process start refused");
        }
        if state.record.processes.len() >= MAX_PROCESSES {
            return Err("session process limit reached; process start refused");
        }
        state.record.processes.insert(
            process.to_owned(),
            Recorded {
                turn,
                recovered: false,
            },
        );
        if self.persist(&state).is_err() {
            state.record.processes.remove(process);
            return Err("the process ledger is unavailable; process start refused");
        }
        state.in_flight.insert(process.to_owned());
        Ok(())
    }

    fn start_settled(&self, process: &str) {
        self.state.lock().in_flight.remove(process);
        self.changed.send_modify(|value| *value += 1);
    }

    /// The target's evidence that `process` is gone.
    pub fn exited(&self, process: &str) {
        let mut state = self.state.lock();
        if state.record.processes.remove(process).is_some() {
            if let Err(error) = self.persist(&state) {
                tracing::warn!(%error, "execution ledger update failed");
            }
            drop(state);
            self.changed.send_modify(|value| *value += 1);
        }
    }

    fn mark_uncertain(&self, method: &str) {
        let mut state = self.state.lock();
        state.record.uncertain_effect = Some(method.to_owned());
        state.record.uncertain_sequence = state.record.uncertain_sequence.saturating_add(1);
        let sequence = state.record.uncertain_sequence;
        if let Err(error) = self.persist(&state) {
            tracing::warn!(%error, "execution ledger update failed");
        }
        drop(state);
        if let Some(observer) = self.observer.get() {
            observer(method, sequence);
        }
    }

    /// Refuse every later start of `turn`; returns the processes to end.
    fn fence(&self, turn: Option<&str>) -> Vec<String> {
        let mut state = self.state.lock();
        match turn {
            Some(turn) => {
                if !state.fenced.iter().any(|fenced| fenced == turn) {
                    if state.fenced.len() >= MAX_FENCED_TURNS {
                        state.fenced.pop_front();
                    }
                    state.fenced.push_back(turn.to_owned());
                }
            }
            None => state.closed = true,
        }
        state
            .record
            .processes
            .iter()
            .filter(|(_, entry)| turn.is_none_or(|turn| entry.turn == turn || entry.recovered))
            .map(|(process, _)| process.clone())
            .collect()
    }

    fn recovered(&self) -> Vec<String> {
        let state = self.state.lock();
        state
            .record
            .processes
            .iter()
            .filter(|(_, entry)| entry.recovered)
            .map(|(process, _)| process.clone())
            .collect()
    }

    fn tracked(&self, process: &str) -> bool {
        self.state.lock().record.processes.contains_key(process)
    }

    fn in_flight(&self, process: &str) -> bool {
        self.state.lock().in_flight.contains(process)
    }

    /// Whether this caller owns the termination of `process`.
    fn claim(&self, process: &str) -> bool {
        self.state.lock().terminating.insert(process.to_owned())
    }

    fn release(&self, process: &str) {
        self.state.lock().terminating.remove(process);
        self.changed.send_modify(|value| *value += 1);
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

    fn persist(&self, state: &State) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if state.record.processes.is_empty() && state.record.uncertain_effect.is_none() {
            return match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(()),
            };
        }
        let bytes = serde_json::to_vec(&state.record)?;
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
    ensure!(
        record.processes.len() <= MAX_PROCESSES,
        "execution ledger exceeds its process limit"
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

    /// Relay one native request from the agent side. Without a ledger this
    /// is the plain invocation; with one, starts are admitted and recorded
    /// first and lost mutation replies mark the session's effect uncertain.
    pub async fn relay(self: &Arc<Self>, method: String, params: Value) -> Value {
        let Some(ledger) = self.ledger().cloned() else {
            return self
                .invoke(method, params)
                .await
                .unwrap_or_else(|_| unavailable());
        };
        let start = (method == "process/start").then(|| process_id(&params));
        if let Some(process) = &start {
            let admitted = process
                .as_deref()
                .ok_or("process start needs a processId")
                .and_then(|process| ledger.admit_start(process));
            if let Err(reason) = admitted {
                return refused(reason);
            }
        }
        let read = (method == "process/read")
            .then(|| process_id(&params))
            .flatten();
        let mutation = MUTATIONS.contains(&method.as_str()).then(|| method.clone());
        let result = self.invoke_classified(method, params).await;
        if let Some(Some(process)) = &start {
            ledger.start_settled(process);
        }
        match result {
            Ok(reply) => {
                if let Some(process) = read.filter(|_| exited(&reply)) {
                    ledger.exited(&process);
                }
                reply
            }
            Err(admitted) => {
                if !admitted {
                    // Refused before it was sent: nothing can have happened.
                    return refused("Execution unavailable; the request was not sent");
                }
                if let Some(method) = mutation {
                    ledger.mark_uncertain(&method);
                }
                unavailable()
            }
        }
    }

    /// The reply of `method` never reached the agent side. A mutation the
    /// target answered (or whose outcome is already unknown) then has an
    /// effect the agent never learned: the session's effect is uncertain.
    pub(super) fn undelivered(&self, method: &str, reply: &Value) {
        let Some(ledger) = self.ledger() else {
            return;
        };
        let answered = reply.get("result").is_some()
            || reply.pointer("/error/data/outcome") == Some(&json!("unknown"));
        if MUTATIONS.contains(&method) && answered {
            ledger.mark_uncertain(method);
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
    /// in the background.
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

    /// Fence an ended turn. When it still holds processes, the returned
    /// future resolves once the target proved every one of them gone; the
    /// turn's completion must wait for it.
    pub fn finish_turn(
        self: &Arc<Self>,
        turn: &str,
    ) -> Option<impl std::future::Future<Output = ()> + Send + 'static> {
        let ledger = self.ledger()?;
        let processes = ledger.fence(Some(turn));
        if processes.is_empty() {
            return None;
        }
        tracing::info!(event_name = "cowboy.execution.turn_fenced", session = %self.session, %turn, processes = processes.len(), reason = "turn_end", "execution turn fenced");
        let client = Arc::clone(self);
        Some(async move { client.terminate_all(processes).await })
    }

    /// Fence every turn and end every recorded process: unit loss and
    /// teardown. Entries left unresolved stay durable for the next worker.
    pub async fn settle_all(self: &Arc<Self>) {
        if let Some(ledger) = self.ledger() {
            let processes = ledger.fence(None);
            self.terminate_all(processes).await;
        }
    }

    /// End what a previous worker of this session left recorded.
    pub fn settle_recovered(self: &Arc<Self>) {
        let Some(ledger) = self.ledger() else {
            return;
        };
        let processes = ledger.recovered();
        if processes.is_empty() {
            return;
        }
        tracing::warn!(event_name = "cowboy.execution.ledger_recovered", session = %self.session, processes = processes.len(), "terminating processes a previous worker recorded");
        let client = Arc::clone(self);
        ledger.runtime.spawn(async move {
            client.terminate_all(processes).await;
        });
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

    /// Terminate `process` until the target proves it gone.
    async fn terminate(self: &Arc<Self>, process: String) {
        let Some(ledger) = self.ledger().cloned() else {
            return;
        };
        if !ledger.claim(&process) {
            // Another settlement owns it; wait for its evidence.
            ledger.wait_until(|ledger| !ledger.tracked(&process)).await;
            return;
        }
        let mut delay = RETRY_INITIAL;
        while ledger.tracked(&process) {
            ledger
                .wait_until(|ledger| !ledger.in_flight(&process))
                .await;
            let _ = self
                .invoke(
                    "process/terminate".to_owned(),
                    json!({"processId": process}),
                )
                .await;
            let read = self
                .invoke(
                    "process/read".to_owned(),
                    json!({"processId": process, "afterSeq": null, "maxBytes": 1, "waitMs": 1000}),
                )
                .await;
            match read {
                Ok(reply)
                    if exited(&reply)
                        || (unknown_process(&reply) && !ledger.in_flight(&process)) =>
                {
                    ledger.exited(&process);
                }
                _ => {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(RETRY_MAX);
                }
            }
        }
        ledger.release(&process);
    }
}
