//! Post-edit delivery ledger: what Cowboy computed or delivered to a
//! session's model, kept in bounded Controller memory.
//!
//! Each delivery has an idempotent ID, its route (`adapter` computes a result a
//! Provider adapter appends, Claude's Mod today; `codex_steering` steers the edit's turn), the
//! files and a digest of the delivered JSON. States: `computed` (handed to an
//! adapter), `steering` (submitted to the turn), `appended` (injected),
//! `refused` (the turn ended or the worker declined; nothing reached the
//! model), `unknown` (no outcome observed) and `suppressed` (identical to the
//! previous delivery of the same context within the repetition window, so not
//! delivered again). `consumed` is not observable yet. A Controller restart
//! discards the ledger.

use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

const PER_SESSION: usize = 20;
const SESSIONS: usize = 256;
/// An identical result within this window is not delivered again.
const REPEAT_WINDOW: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    Adapter,
    CodexSteering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Computed,
    Steering,
    Appended,
    Refused,
    Unknown,
    Suppressed,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            Self::Computed => "computed",
            Self::Steering => "steering",
            Self::Appended => "appended",
            Self::Refused => "refused",
            Self::Unknown => "unknown",
            Self::Suppressed => "suppressed",
        }
    }

    /// Whether this delivery may have reached the model.
    fn delivered(self) -> bool {
        matches!(
            self,
            Self::Computed | Self::Steering | Self::Appended | Self::Unknown
        )
    }
}

#[derive(Clone, Debug)]
struct Delivery {
    id: String,
    route: Route,
    files: Vec<String>,
    digest: String,
    state: State,
    reason: Option<String>,
    at: Instant,
    at_ms: u64,
}

static LEDGER: LazyLock<Mutex<HashMap<String, VecDeque<Delivery>>>> = LazyLock::new(Mutex::default);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The identity of what a delivery would show: its files and diagnostics.
pub(super) fn digest(value: &Value) -> String {
    let shown = json!({"files":value["files"]});
    format!("{:x}", Sha256::digest(shown.to_string().as_bytes()))
}

/// Record a delivery about to be made. Returns its ID, or `None` when the
/// same content was already delivered to this session within the window
/// (recorded as `suppressed`).
pub(super) fn begin(
    session: &str,
    route: Route,
    files: Vec<String>,
    digest: String,
) -> Option<String> {
    let mut ledger = LEDGER.lock();
    if ledger.len() >= SESSIONS
        && !ledger.contains_key(session)
        && let Some(oldest) = ledger
            .iter()
            .min_by_key(|(_, entries)| entries.back().map(|entry| entry.at))
            .map(|(key, _)| key.clone())
    {
        ledger.remove(&oldest);
    }
    let entries = ledger.entry(session.to_owned()).or_default();
    let repeated = entries
        .iter()
        .rev()
        .find(|entry| entry.state != State::Suppressed)
        .is_some_and(|previous| {
            previous.state.delivered()
                && previous.digest == digest
                && previous.at.elapsed() < REPEAT_WINDOW
        });
    let id = uuid::Uuid::new_v4().simple().to_string();
    if entries.len() >= PER_SESSION {
        entries.pop_front();
    }
    entries.push_back(Delivery {
        id: id.clone(),
        route,
        files,
        digest,
        state: if repeated {
            State::Suppressed
        } else if route == Route::Adapter {
            State::Computed
        } else {
            State::Steering
        },
        reason: repeated.then(|| "repeat".to_owned()),
        at: Instant::now(),
        at_ms: now_ms(),
    });
    (!repeated).then_some(id)
}

/// Record a delivery's observed outcome.
pub(super) fn finish(session: &str, id: &str, state: State, reason: Option<&str>) {
    if let Some(entry) = LEDGER
        .lock()
        .get_mut(session)
        .and_then(|entries| entries.iter_mut().find(|entry| entry.id == id))
    {
        entry.state = state;
        entry.reason = reason.map(|value| value.chars().take(120).collect());
    }
}

/// This session's recent deliveries, newest last. Never repository text.
pub(super) fn list(session: &str) -> Value {
    let ledger = LEDGER.lock();
    let entries: Vec<Value> = ledger
        .get(session)
        .into_iter()
        .flatten()
        .map(|entry| {
            json!({
                "id": entry.id,
                "route": match entry.route { Route::Adapter => "adapter", Route::CodexSteering => "codex_steering" },
                "files": entry.files,
                "state": entry.state.name(),
                "reason": entry.reason,
                "at_ms": entry.at_ms,
            })
        })
        .collect();
    json!({"coverage":"controller_memory","deliveries":entries})
}

#[cfg(test)]
mod tests;
