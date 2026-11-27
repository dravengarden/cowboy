//! Versioned local IPC contract between `cowboy-machine` and workers.
//!
//! This protocol is deliberately independent from ACP. ACP is the downstream
//! worker-to-agent protocol; this is Cowboy's stable process boundary. Every
//! rolling deployment must retain at least one overlapping protocol version
//! until all workers using the older version have drained.

#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use std::io;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// Exact worker/turn identity captured before a user chooses delivery timing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryTarget {
    pub worker_epoch: String,
    pub turn_id: String,
}

/// Current runtime protocol. Version 1 is additive-only: new optional fields
/// must use serde defaults and existing meanings must not change in place.
/// Runtime protocol version that introduces `WorkerCommand::DrainCrashed`,
/// worker completion receipt `Frame::CrashedDrainCompleted`, and broker-only
/// retirement notice `Frame::CrashedWorkerDrained`.
pub const CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION: u16 = 10;
pub const PROTOCOL_VERSION: u16 = 10;
pub use crate::post_edit_inspect::{
    POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION, PostEditInspectOutcome, PostEditInspectReply,
    PostEditInspectRequest,
};
pub use crate::wrap_up::{MAX_WRAP_UP_HINT_BYTES, WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION};
/// Controllers that keep observed subagent transcripts. A Machine forwards
/// [`SUBAGENT_EVENT_UPDATE`] only to a Controller at this version or later and
/// retires it for an older one ([`retire_observation`]), so a Controller
/// rollback never stores observations in a parent timeline.
pub const SUBAGENT_OBSERVATION_PROTOCOL_VERSION: u16 = 7;
/// Named subscriptions must never be sent to peers that ignore their identity.
pub const PROVIDER_ACCOUNT_PROTOCOL_VERSION: u16 = 5;
/// Mandatory MCP launch plans cannot be ignored by retained workers.
pub const MCP_LAUNCH_PROTOCOL_VERSION: u16 = 6;
/// Private per-launch token; never a Provider credential or inherited setting.
pub const WORKER_LAUNCH_ENV: &str = "COWBOY_ACCOUNT_WORKER_LAUNCH";
/// Oldest runtime protocol this build accepts during a mixed-generation roll.
pub const MIN_PROTOCOL_VERSION: u16 = 1;
/// Upper bound for a single IPC frame. Large binary artifacts are referenced by
/// content hash elsewhere; accepting unbounded JSON here would turn a corrupt
/// peer into an allocation denial of service.
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerRole {
    Core,
    Worker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Starting,
    Running,
    Busy,
    Draining,
    Exited,
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct WorkerSnapshot {
    pub session_id: String,
    pub worker_epoch: String,
    pub generation: String,
    /// Concrete executable for this generation. Machine broker retains it so a failed
    /// new generation can be rolled back even after Machine broker itself restarted.
    #[serde(default)]
    pub executable: Option<String>,
    /// Full relaunch specification, self-reported by the worker so a freshly
    /// restarted Machine broker can reconstruct its session registry before the next
    /// generation handoff.
    #[serde(default)]
    pub launch: Option<StartSession>,
    pub state: WorkerState,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    /// Whether the Provider has durably materialized `agent_session_id`.
    /// Providers persist a native thread lazily: `session/new` returns an id
    /// before any transcript or rollout exists, so resuming it before the
    /// first prompt fails with "Resource not found". `None` comes from a
    /// worker that predates this fact and keeps treating the id as resumable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_thread_materialized: Option<bool>,
    #[serde(default)]
    pub current_turn_id: Option<String>,
    /// Observed from this worker's live ACP handshake, never from an installed
    /// Plugin version. Older workers cannot receive current-turn delivery.
    #[serde(default)]
    pub supports_current_turn: bool,
    #[serde(default)]
    pub delivery_controls: DeliveryControls,
    /// Live verified P2 Antigravity wrap-up steering capability.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wrap_up_steering: bool,
    /// The worker accepts [`WorkerCommand::CancelSubagent`]. Absent from older
    /// workers, which must never receive it: their frame decoder would reject
    /// the whole connection.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub supports_subagent_cancel: bool,
    #[serde(default)]
    pub last_runtime_seq: u64,
    #[serde(default)]
    pub pending_permissions: Vec<String>,
    #[serde(default)]
    pub config_options: Option<serde_json::Value>,
    #[serde(default)]
    pub context_used: Option<u64>,
    #[serde(default)]
    pub context_size: Option<u64>,
    /// Prompts accepted by Machine broker but intentionally held until a worker is
    /// ready (for example during generation drain).
    #[serde(default)]
    pub pending_prompt_count: u64,
    #[serde(default)]
    pub drain_requested: bool,
    /// Why Machine broker itself ended this worker, such as a heartbeat
    /// isolation. Workers report their own failures through runtime events and
    /// never set this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_detail: Option<String>,
    /// Live native background tasks that count as agent activity (see
    /// [`background_tasks_update`]). `None` comes from a worker that does not
    /// observe them. Restores the level after a Controller reconnect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_tasks: Option<u32>,
    /// The cycle the agent is running on its own, with no Cowboy turn (see
    /// [`native_cycle_update`]). Restores the level after a reconnect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_cycle_id: Option<String>,
    /// True while an adapter-reported native cycle is still active, even after
    /// a failed Cowboy turn clears steerable `native_cycle_id`. Cleared only on
    /// explicit adapter cycle end or confirmed worker exit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_cycle_active: bool,
    /// The Machine-minted durable Session lineage (see `incarnations`). Only a
    /// Machine whose incarnation writer was admitted stamps it, on every
    /// snapshot it sends; a worker never sets it and an older Machine omits it.
    /// It is storage identity for comparison, never authorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<String>,
    /// The account-sandbox unit running this worker's Provider process, for
    /// a Provider whose signed manifest requires `account_sandbox_v1`. The
    /// Machine confirms with the daemon that it is gone before it reports
    /// the worker stopped. Absent for every other Provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_unit: Option<AccountUnitHandle>,
    /// A target mutation or process start of this session lost its reply,
    /// so its effect is unknown (an account-sandboxed session's execution
    /// ledger records it durably). Absent when false and from older workers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertain_effect: Option<UncertainEffect>,
}

/// The latest target request of a session whose reply was lost after it may
/// have been admitted; its effect is unknown until the owner reviews it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UncertainEffect {
    pub method: String,
    /// How many requests of the session lost their reply. The Controller
    /// parks the queue until the owner acknowledged this sequence.
    #[serde(default)]
    pub sequence: u64,
}

/// One unit of the `cowboy-account-sandbox` daemon: its transient systemd
/// unit name and the exact invocation and boot that started it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountUnitHandle {
    pub unit: String,
    /// `None` only when the unit finished before its invocation was observed.
    pub invocation_id: Option<String>,
    pub boot_id: String,
}

/// Cowboy-owned `sessionUpdate` carrying the native background-task level.
///
/// A Provider can end its prompt turn while background work it will resume on
/// (a backgrounded shell, a Monitor) is still running. The level is carried as
/// an update rather than a new [`RuntimeEvent`] variant so peers that predate
/// it still decode the frame. The Controller projects it onto session
/// metadata and never stores it in the transcript.
pub const BACKGROUND_TASKS_UPDATE: &str = "cowboy_background_tasks";

/// Cowboy-owned `sessionUpdate` naming the cycle the agent is running on its
/// own, or `null` when it runs none.
///
/// Claude resumes by itself after a background task ends, while Cowboy holds
/// no turn for it. The worker names each such cycle once so current-turn
/// delivery can address exactly that cycle and never a later one. Like
/// [`BACKGROUND_TASKS_UPDATE`] it is session metadata carried as an update so
/// older peers still decode the frame; it is never a transcript row and never
/// a turn edge.
pub const NATIVE_CYCLE_UPDATE: &str = "cowboy_native_cycle";

#[must_use]
pub fn native_cycle_update(cycle_id: Option<&str>) -> serde_json::Value {
    serde_json::json!({ "sessionUpdate": NATIVE_CYCLE_UPDATE, "cycleId": cycle_id })
}

/// The cycle carried by a [`NATIVE_CYCLE_UPDATE`]: `Some(None)` when it ended.
#[must_use]
pub fn native_cycle(update: &serde_json::Value) -> Option<Option<String>> {
    (update.get("sessionUpdate")?.as_str()? == NATIVE_CYCLE_UPDATE).then(|| {
        update
            .get("cycleId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    })
}

/// Cowboy-owned `sessionUpdate` presenting one ACP subagent (a child session
/// the agent announced with `subagent_update`) in its root session.
///
/// The worker never mixes a child's own messages and tool calls into the
/// parent transcript. It folds them into a bounded, self-contained snapshot of
/// the child, identified by `sessionId` and ordered by `revision`. Each
/// snapshot replaces the previous one for that child, so the Controller may
/// keep only the latest durably and a Client renders the highest revision. A
/// peer that predates it stores and ignores it like any unknown update.
pub const SUBAGENT_UPDATE: &str = "cowboy_subagent";

/// One of a subagent's own ACP updates, observed passively for its full
/// transcript (`docs/subagents.md#observed-transcripts`). The payload is
/// `{childSessionId, update, dropped?}`; `dropped` counts the child updates
/// the worker discarded before this one instead of waiting for delivery.
/// The Controller stores it outside the parent timeline; a peer that predates
/// it never receives it (see `observe_subagents`).
pub const SUBAGENT_EVENT_UPDATE: &str = "cowboy_subagent_event";

/// Replace an observed child update with an inert one for a Controller
/// before [`SUBAGENT_OBSERVATION_PROTOCOL_VERSION`]. The runtime sequence
/// stays contiguous; such a Controller consumes a steering result without a
/// `cmid` and never stores it.
pub fn retire_observation(event: &mut RuntimeEvent) {
    if let RuntimeEvent::Update { update, cmid } = event
        && update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            == Some(SUBAGENT_EVENT_UPDATE)
    {
        *update = serde_json::json!({ "sessionUpdate": STEERING_RESULT_UPDATE });
        *cmid = None;
    }
}

/// Approximate serialized JSON size of `value`, never below the real size
/// for text that needs no unusual escaping; walks without allocating.
#[must_use]
pub fn json_size(value: &serde_json::Value) -> usize {
    use serde_json::Value;
    match value {
        Value::Null => 4,
        Value::Bool(_) => 5,
        Value::Number(_) => 24,
        Value::String(text) => string_size(text),
        Value::Array(items) => items.iter().fold(2 + items.len(), |size, item| {
            size.saturating_add(json_size(item))
        }),
        Value::Object(fields) => fields.iter().fold(2 + fields.len(), |size, (key, item)| {
            size.saturating_add(string_size(key) + 1)
                .saturating_add(json_size(item))
        }),
    }
}

fn string_size(text: &str) -> usize {
    2 + text
        .bytes()
        .map(|byte| {
            if byte < 0x20 || byte == b'"' || byte == b'\\' {
                6
            } else {
                1
            }
        })
        .sum::<usize>()
}

fn truncate_text(text: &str, max: usize) -> String {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Bound an observed child update to about `max` serialized bytes, keeping
/// what identifies and summarizes it: long text first, then tool content and
/// raw input/output, and finally a minimal identity with short labels. A
/// trimmed update carries `_cowboy.truncated`.
#[must_use]
pub fn bound_observed_update(mut update: serde_json::Value, max: usize) -> serde_json::Value {
    use serde_json::{Value, json};
    if json_size(&update) <= max {
        return update;
    }
    if let Some(text) = update.pointer_mut("/content/text")
        && let Some(value) = text.as_str()
    {
        *text = Value::String(truncate_text(value, max / 8));
    }
    if let Some(object) = update.as_object_mut() {
        for field in ["content", "rawOutput", "rawInput", "locations"] {
            if json_size(&Value::Object(object.clone())) <= max {
                break;
            }
            object.remove(field);
        }
        object.insert("_cowboy".to_owned(), json!({ "truncated": true }));
    }
    if json_size(&update) <= max {
        return update;
    }
    let mut minimal = serde_json::Map::new();
    for key in [
        "sessionUpdate",
        "toolCallId",
        "messageId",
        "title",
        "status",
        "kind",
    ] {
        if let Some(Value::String(text)) = update.get(key) {
            minimal.insert(key.to_owned(), Value::String(truncate_text(text, 1024)));
        }
    }
    minimal.insert("_cowboy".to_owned(), json!({ "truncated": true }));
    Value::Object(minimal)
}

pub const CURRENT_TURN_CAPABILITIES_UPDATE: &str = "cowboy_delivery_capabilities";
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryControls {
    #[serde(default)]
    pub version: u16,
    #[serde(default)]
    pub next_checkpoint: bool,
}
pub const STEERING_RESULT_UPDATE: &str = "cowboy_steering_result";

/// Cowboy-owned `sessionUpdate` publishing a session's newest uncertain
/// effect as soon as the worker records it (worker metadata, never a
/// transcript row), so the Controller parks the queue without waiting for a
/// reconnect's snapshot. Applied monotonically by sequence.
pub const UNCERTAIN_EFFECT_UPDATE: &str = "cowboy_uncertain_effect";

#[must_use]
pub fn uncertain_effect_update(effect: &UncertainEffect) -> serde_json::Value {
    serde_json::json!({
        "sessionUpdate": UNCERTAIN_EFFECT_UPDATE,
        "method": effect.method,
        "sequence": effect.sequence,
    })
}

/// The effect carried by an [`UNCERTAIN_EFFECT_UPDATE`], if `update` is one.
#[must_use]
pub fn uncertain_effect(update: &serde_json::Value) -> Option<UncertainEffect> {
    (update.get("sessionUpdate")?.as_str()? == UNCERTAIN_EFFECT_UPDATE).then(|| UncertainEffect {
        method: update
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("request")
            .to_owned(),
        sequence: update
            .get("sequence")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
    })
}

/// Merge `observed` into `current`: the newest sequence wins and an older
/// observation never replaces it.
pub fn merge_uncertain_effect(current: &mut Option<UncertainEffect>, observed: UncertainEffect) {
    if current
        .as_ref()
        .is_none_or(|current| observed.sequence > current.sequence)
    {
        *current = Some(observed);
    }
}

#[must_use]
pub fn current_turn_support(update: &serde_json::Value) -> Option<bool> {
    (update.get("sessionUpdate")?.as_str()? == CURRENT_TURN_CAPABILITIES_UPDATE)
        .then(|| update.get("currentTurn")?.as_bool())
        .flatten()
}

#[must_use]
pub fn wrap_up_steering_support(update: &serde_json::Value) -> Option<bool> {
    (update.get("sessionUpdate")?.as_str()? == CURRENT_TURN_CAPABILITIES_UPDATE)
        .then(|| update.get("wrapUpSteering")?.as_bool())
        .flatten()
}

#[must_use]
pub fn background_tasks_update(count: u32) -> serde_json::Value {
    serde_json::json!({ "sessionUpdate": BACKGROUND_TASKS_UPDATE, "count": count })
}

/// The level carried by a [`BACKGROUND_TASKS_UPDATE`], if `update` is one.
#[must_use]
pub fn background_tasks_count(update: &serde_json::Value) -> Option<u32> {
    if update
        .get("sessionUpdate")
        .and_then(serde_json::Value::as_str)
        != Some(BACKGROUND_TASKS_UPDATE)
    {
        return None;
    }
    Some(
        update
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .map_or(0, |count| u32::try_from(count).unwrap_or(u32::MAX)),
    )
}

impl WorkerSnapshot {
    /// What current-turn delivery addresses: the Cowboy turn, or else the
    /// cycle the agent started on its own.
    #[must_use]
    pub fn delivery_turn_id(&self) -> Option<&str> {
        self.current_turn_id
            .as_deref()
            .or(self.native_cycle_id.as_deref())
    }

    /// The native thread a replacement worker may pass to `--resume`.
    ///
    /// Every Machine-initiated relaunch (generation cutover, fallback,
    /// Provider roll) must use this rather than `agent_session_id`: a thread
    /// that never received a prompt has nothing to resume, and a strict resume
    /// of it would crash a session whose context is intentionally empty. The
    /// Controller applies the same rule to its durable log through
    /// `Hub::agent_session_id_for_resume`.
    #[must_use]
    pub fn resumable_agent_session_id(&self) -> Option<String> {
        if self.native_thread_materialized == Some(false) {
            return None;
        }
        self.agent_session_id.clone()
    }

    /// Apply a runtime event's effect on native-thread identity. The worker
    /// and its Machine broker mirror both use this so their snapshots cannot
    /// disagree about resumability.
    pub fn observe_native_thread(&mut self, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::Ready {
                agent_session_id: Some(agent_session_id),
            } => {
                self.agent_session_id = Some(agent_session_id.clone());
            }
            RuntimeEvent::AgentSessionId {
                agent_session_id,
                materialized,
            } => {
                if *materialized {
                    // `session/fork` wrote the copied history already.
                    self.native_thread_materialized = Some(true);
                } else if self.agent_session_id.as_deref() != Some(agent_session_id.as_str()) {
                    // `session/new` announced a fresh thread; the Provider
                    // writes nothing for it until the first prompt.
                    self.native_thread_materialized = Some(false);
                }
                self.agent_session_id = Some(agent_session_id.clone());
            }
            RuntimeEvent::TurnStarted { .. } if self.agent_session_id.is_some() => {
                // Drain waits for the turn to end, and a Provider records the
                // user message before it answers.
                self.native_thread_materialized = Some(true);
            }
            _ => {}
        }
    }

    /// Whether this snapshot came from a connected worker rather than the
    /// Machine broker's launch-registry placeholder.
    ///
    /// Placeholder epochs use the reserved `broker-` prefix so an additive v1
    /// controller can distinguish "known session" from "live owner" without a
    /// wire-version flag. A placeholder must never prove that an in-flight turn
    /// survived a control-plane restart.
    #[must_use]
    pub fn has_connected_owner(&self) -> bool {
        !self.worker_epoch.starts_with("broker-")
    }

    /// Whether this live worker still owns the ACP prompt that a controller
    /// restart may have been trying to adopt. The worker process surviving is
    /// not enough: `session/prompt` is a Controller↔Machine RPC, so a reconnect
    /// can report Running with no turn id after the prompt already died.
    #[must_use]
    pub fn owns_in_flight_turn(&self) -> bool {
        self.current_turn_id.is_some()
            || self.pending_prompt_count > 0
            || self.state == WorkerState::Busy
            || !self.pending_permissions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartSession {
    pub session_id: String,
    pub provider: String,
    #[serde(default)]
    pub provider_account_id: crate::provider_account::ProviderAccountId,
    /// Immutable Provider package identity. These fields are independent from
    /// `generation`, which remains the Cowboy worker-binary generation.
    #[serde(default)]
    pub provider_version: String,
    #[serde(default)]
    pub provider_generation_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_auth_generation: Option<u64>,
    /// Signed, closed Cowboy host-integration interface. The Machine derives
    /// this again from the installed package before starting the worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_behavior: Option<cowboy_provider_sdk::ProviderBehaviorContract>,
    pub cwd: String,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub system: bool,
    /// Worker-local context budget selected for this provider/model. Older
    /// Machine generations ignore these optional fields during rolling deploys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_token_limit: Option<u64>,
    /// Cowboy-owned `DeepSeek` cache policy. Older launch snapshots omit this
    /// additive field; current Machines interpret omission as the safe `auto`
    /// default for `DeepSeek` providers and preserve explicit `false` as off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_protection: Option<bool>,
    pub generation: String,
    /// Desired generation this session is temporarily falling back from.
    #[serde(default)]
    pub fallback_for: Option<String>,
    /// Rebuild Machine broker's launch registry without spawning a missing worker.
    /// Cowboy sends this on broker reconnect while surviving workers are still
    /// converging; a later real `EnsureSession` may start one if it never returns.
    #[serde(default)]
    pub adopt_only: bool,
    /// Presence, including unknown or malformed records, always disables a
    /// legacy local launch. Bound workers require runtime protocol two.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::execution_environment::deserialize_optional_binding"
    )]
    pub execution_binding: Option<crate::execution_environment::ExecutionBinding>,
    /// Controller policy for the runtime Machine's installed session-extension
    /// Plugins. Older Machines ignore this additive field and keep their host
    /// enrollment; current ones write the session's private descriptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<SessionExtensions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_plan: Option<Box<crate::mcp_binding::NativeLaunchPlan>>,
    /// Create this session's first native thread with ACP `session/fork` of
    /// another session's thread instead of `session/new`. Mutually exclusive
    /// with `agent_session_id`; only Machines at
    /// `NATIVE_FORK_PROTOCOL_VERSION` or later receive it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_fork: Option<Box<NativeFork>>,
}

/// Source of a native fork. Both identities belong to the same account and
/// runtime Machine as the session being started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeFork {
    /// Native session ID of the source thread.
    pub agent_session_id: String,
    /// Provider message ID of a turn-final assistant message. The fork keeps
    /// the conversation through this message; `None` forks all of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

pub const SESSION_EXTENSIONS_SCHEMA: u16 = 1;

impl CoreCommand {
    /// The worker's form of a Controller prompt, with the session it targets.
    #[must_use]
    pub fn into_worker_prompt(self) -> Option<(String, WorkerCommand)> {
        let Self::Prompt {
            session_id,
            command_id,
            turn_id,
            content,
            cmid,
            trace,
            echo_artifacts,
            observe_subagents,
            expected_worker_epoch,
            acknowledged_uncertain_sequence,
        } = self
        else {
            return None;
        };
        Some((
            session_id,
            WorkerCommand::Prompt {
                command_id,
                turn_id,
                content,
                cmid,
                trace,
                echo_artifacts,
                observe_subagents,
                expected_worker_epoch,
                acknowledged_uncertain_sequence,
            },
        ))
    }
}

impl StartSession {
    #[must_use]
    pub fn minimum_protocol(&self) -> u16 {
        if self.mcp_plan.is_some() {
            MCP_LAUNCH_PROTOCOL_VERSION
        } else if !self.provider_account_id.is_default() {
            PROVIDER_ACCOUNT_PROTOCOL_VERSION
        } else if self.execution_binding.is_some() {
            2
        } else {
            MIN_PROTOCOL_VERSION
        }
    }
}

/// Per-session switches for session-extension Plugins. Which memory backend
/// and call profiles serve them is resolved by the runtime Machine from its
/// own installed, active Plugins; no endpoint or credential travels here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionExtensions {
    pub schema: u16,
    pub memory_tools: bool,
    pub memory_recall: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum CoreCommand {
    CancelTurn {
        session_id: String,
        command_id: String,
        target: DeliveryTarget,
        cmid: String,
    },
    Steer {
        session_id: String,
        command_id: String,
        worker_epoch: String,
        turn_id: String,
        content: Vec<serde_json::Value>,
        cmid: String,
        #[serde(default)]
        next_checkpoint: bool,
    },
    /// Protocol-27 host-authorized replacement of a remote execution binding.
    /// Ordinary `EnsureSession` must never perform this transition.
    RecoverExecutionSession {
        session: Box<StartSession>,
        intent: Box<crate::execution_environment::RecoveryV1>,
        command_id: String,
    },
    EnsureSession {
        session: StartSession,
    },
    Prompt {
        session_id: String,
        command_id: String,
        turn_id: String,
        content: Vec<serde_json::Value>,
        #[serde(default)]
        cmid: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "crate::runtime_trace::deserialize_carrier"
        )]
        trace: Option<crate::runtime_trace::TraceCarrier>,
        /// Cowboy stored this prompt's large images before dispatch, so the
        /// worker may echo them by content digest instead of returning the
        /// bytes. Absent from older Controllers, which keeps the inline echo.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        echo_artifacts: bool,
        /// The Controller keeps observed subagent transcripts, so the worker
        /// may forward each child's own updates as [`SUBAGENT_EVENT_UPDATE`].
        /// Absent from older Controllers and dropped by older Machines, which
        /// keeps the bounded snapshot only. Presentation only: it never
        /// changes what the agent runs or sees.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        observe_subagents: bool,
        /// Optional worker epoch bound by the Controller at dispatch time; a
        /// replacement worker with a different epoch refuses admission before ACP.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_worker_epoch: Option<String>,
        /// Highest uncertain-effect sequence acknowledged by the owner when this
        /// prompt was admitted; workers with a higher sequence refuse before ACP.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        acknowledged_uncertain_sequence: Option<u64>,
    },
    Cancel {
        session_id: String,
        command_id: String,
    },
    /// Stop one ACP subagent (a child session of this session's agent) without
    /// cancelling the session's turn. Sent only to Machines at
    /// [`crate::machine_protocol::SUBAGENT_CANCEL_PROTOCOL_VERSION`]; the
    /// Machine forwards it only to a worker whose snapshot reports
    /// `supports_subagent_cancel` and refuses it otherwise.
    CancelSubagent {
        session_id: String,
        command_id: String,
        subagent_session_id: String,
    },
    /// Best-effort, payload-free hint asking a verified P2 Antigravity turn to
    /// wrap up its current step so a front-queued Owner prompt runs next.
    WrapUpHint {
        session_id: String,
        target: DeliveryTarget,
    },
    Permission {
        session_id: String,
        command_id: String,
        request_id: String,
        #[serde(default)]
        option_id: Option<String>,
    },
    SetConfigOption {
        session_id: String,
        command_id: String,
        config_id: String,
        value: serde_json::Value,
    },
    DrainSession {
        session_id: String,
        generation: String,
    },
    StopSession {
        session_id: String,
        command_id: String,
    },
    /// Stop only an idle session's live worker, retaining its worktree,
    /// transcript and resumable native id; the next ensure resumes it.
    /// Sent only to Machines at
    /// [`crate::machine_protocol::SESSION_HIBERNATION_PROTOCOL_VERSION`].
    HibernateSession {
        session_id: String,
        command_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_worker_epoch: Option<String>,
    },
    /// Ask an idle worker in the `Crashed` state to flush its outbox and exit
    /// cleanly without disturbing the session's `Crashed` status or held queue.
    /// Bound to `worker_epoch` so a replacement worker is never affected.
    /// Sent only to Machines at
    /// [`crate::machine_protocol::CRASHED_WORKER_DRAIN_PROTOCOL_VERSION`].
    DrainCrashedWorker {
        session_id: String,
        command_id: String,
        worker_epoch: String,
    },
    /// Stop a named account's worker without deleting its session. This is
    /// process-stop evidence only, not account-wide logout or credential cleanup.
    QuiesceAccountSession {
        session_id: String,
        command_id: String,
        account_id: crate::provider_account::ProviderAccountId,
    },
    ReadmitAccountSession {
        session: Box<StartSession>,
        binding: Box<crate::provider_account::PreparedBinding>,
        command_id: String,
    },
    SetDesiredGeneration {
        generation: String,
        /// Concrete worker executable for this generation. Optional keeps the
        /// v1 wire format compatible with older cores during a rolling update.
        #[serde(default)]
        worker_command: Option<String>,
    },
    /// Gracefully recycle the live workers for one provider after an external
    /// CLI/adapter update. Busy turns drain before their worker is replaced.
    RollProvider {
        provider: String,
        /// None rolls the installed Provider; Some rolls only this credential scope.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<crate::provider_account::ProviderAccountId>,
    },
}

impl CoreCommand {
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::EnsureSession { session } => Some(&session.session_id),
            Self::RecoverExecutionSession { session, .. }
            | Self::ReadmitAccountSession { session, .. } => Some(&session.session_id),
            Self::CancelTurn { session_id, .. }
            | Self::Steer { session_id, .. }
            | Self::Prompt { session_id, .. }
            | Self::Cancel { session_id, .. }
            | Self::CancelSubagent { session_id, .. }
            | Self::WrapUpHint { session_id, .. }
            | Self::Permission { session_id, .. }
            | Self::SetConfigOption { session_id, .. }
            | Self::DrainSession { session_id, .. }
            | Self::StopSession { session_id, .. }
            | Self::QuiesceAccountSession { session_id, .. }
            | Self::HibernateSession { session_id, .. }
            | Self::DrainCrashedWorker { session_id, .. } => Some(session_id),
            Self::SetDesiredGeneration { .. } | Self::RollProvider { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WorkerCommand {
    CancelTurn {
        command_id: String,
        target: DeliveryTarget,
        cmid: String,
    },
    Steer {
        command_id: String,
        worker_epoch: String,
        turn_id: String,
        content: Vec<serde_json::Value>,
        cmid: String,
        #[serde(default)]
        next_checkpoint: bool,
    },
    Prompt {
        command_id: String,
        turn_id: String,
        content: Vec<serde_json::Value>,
        #[serde(default)]
        cmid: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "crate::runtime_trace::deserialize_carrier"
        )]
        trace: Option<crate::runtime_trace::TraceCarrier>,
        /// Cowboy stored this prompt's large images before dispatch, so the
        /// worker may echo them by content digest instead of returning the
        /// bytes. Absent from older Controllers, which keeps the inline echo.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        echo_artifacts: bool,
        /// The Controller keeps observed subagent transcripts, so the worker
        /// may forward each child's own updates as [`SUBAGENT_EVENT_UPDATE`].
        /// Absent from older Controllers and dropped by older Machines, which
        /// keeps the bounded snapshot only. Presentation only: it never
        /// changes what the agent runs or sees.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        observe_subagents: bool,
        /// Optional worker epoch bound by the Controller at dispatch time; a
        /// replacement worker with a different epoch refuses admission before ACP.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_worker_epoch: Option<String>,
        /// Highest uncertain-effect sequence acknowledged by the owner when this
        /// prompt was admitted; workers with a higher sequence refuse before ACP.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        acknowledged_uncertain_sequence: Option<u64>,
    },
    Cancel {
        command_id: String,
    },
    /// See [`CoreCommand::CancelSubagent`].
    CancelSubagent {
        command_id: String,
        subagent_session_id: String,
    },
    /// See [`CoreCommand::WrapUpHint`].
    WrapUpHint {
        session_id: String,
        target: DeliveryTarget,
    },
    Permission {
        command_id: String,
        request_id: String,
        #[serde(default)]
        option_id: Option<String>,
    },
    SetConfigOption {
        command_id: String,
        config_id: String,
        value: serde_json::Value,
    },
    Drain,
    /// See [`CoreCommand::DrainCrashedWorker`]. Routed only to workers speaking
    /// [`CRASHED_WORKER_DRAIN_RUNTIME_PROTOCOL_VERSION`] or newer.
    DrainCrashed {
        command_id: String,
        worker_epoch: String,
    },
    Stop {
        command_id: String,
    },
}

/// Native usage for exactly one prompt, retained with its turn-end event.
/// Input includes cache reads; reasoning is already included in output.
/// Missing cache evidence stays unknown rather than becoming zero.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnUsage {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: Option<u64>,
    pub uncached_input_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

/// First-party providers whose ACP/gateway surfaces do not meter cache writes
/// (or default absent write counts to zero). Third-party ACP plugins and
/// `claude-code` are NOT listed here, so measured zero write counts are preserved.
pub const UNMETERED_ZERO_CACHE_WRITE_PROVIDERS: &[&str] = &[
    "codex",
    "antigravity",
    "claude-deepseek",
    "codex-deepseek",
    "deepseek-harness",
    "grok",
];

/// Whether `provider` genuinely reports `cache_write_tokens == 0` as a measured
/// zero (as Anthropic Messages and third-party metered ACP plugins do) rather
/// than defaulting an unsupported/absent write counter to zero.
#[must_use]
pub fn provider_reports_zero_cache_write_meaningfully(provider: &str) -> bool {
    !UNMETERED_ZERO_CACHE_WRITE_PROVIDERS.contains(&provider)
}

impl TurnUsage {
    /// ACP adapters report either disjoint fresh/read/write input counts or
    /// inclusive input. Require the reported totals to reconcile before
    /// projecting a common representation; never count reasoning twice.
    #[must_use]
    pub fn from_acp(usage: &agent_client_protocol::schema::v1::Usage) -> Option<Self> {
        let total = usage.total_tokens;
        let output = usage.output_tokens;
        let input = total.checked_sub(output)?;
        let read = usage.cached_read_tokens;
        let write = usage.cached_write_tokens;
        let disjoint = usage
            .input_tokens
            .checked_add(read.unwrap_or(0))?
            .checked_add(write.unwrap_or(0))?;
        if (usage.input_tokens != input && disjoint != input)
            || read.is_some_and(|tokens| tokens > input)
            || write.is_some_and(|tokens| tokens > input.saturating_sub(read.unwrap_or(0)))
            || usage.thought_tokens.is_some_and(|tokens| tokens > output)
            || total > 9_007_199_254_740_991
        {
            return None;
        }
        Some(Self {
            total_tokens: total,
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: read,
            uncached_input_tokens: read.map(|tokens| input - tokens),
            cache_write_tokens: write,
            reasoning_tokens: usage.thought_tokens,
        })
    }

    /// Provider-aware ACP usage projection: preserves positive `cache_write_tokens`
    /// for all providers, keeps `Some(0)` for providers with genuine write metering
    /// (`claude-code`), and normalizes unsupported zero-defaulted `cache_write_tokens`
    /// to `None` so downstream coverage metrics stay honest.
    #[must_use]
    pub fn from_acp_for_provider(
        provider: &str,
        usage: &agent_client_protocol::schema::v1::Usage,
    ) -> Option<Self> {
        let mut projected = Self::from_acp(usage)?;
        if projected.cache_write_tokens == Some(0)
            && !provider_reports_zero_cache_write_meaningfully(provider)
        {
            projected.cache_write_tokens = None;
        }
        Some(projected)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuntimeEvent {
    Ready {
        #[serde(default)]
        agent_session_id: Option<String>,
    },
    Status {
        state: WorkerState,
        #[serde(default)]
        detail: Option<String>,
    },
    Update {
        update: serde_json::Value,
        #[serde(default)]
        cmid: Option<String>,
    },
    ConfigOptions {
        options: serde_json::Value,
    },
    ContextUsage {
        used: u64,
        size: u64,
        #[serde(default)]
        raw: serde_json::Value,
        #[serde(default)]
        observed_at_ms: i64,
    },
    PermissionRequest {
        request_id: String,
        tool_call: serde_json::Value,
        options: serde_json::Value,
    },
    PermissionResolved {
        request_id: String,
        #[serde(default)]
        option_id: Option<String>,
    },
    TurnStarted {
        turn_id: String,
        command_id: String,
    },
    TurnEnded {
        turn_id: String,
        stop_reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<TurnUsage>,
    },
    AgentSessionId {
        agent_session_id: String,
        /// The thread already holds history, as a forked one does. A fresh
        /// `session/new` thread is not materialized until its first prompt.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        materialized: bool,
    },
    ScheduleWakeup {
        delay_seconds: i64,
        prompt: String,
    },
    UndeliveredPrompt {
        command_id: String,
        #[serde(default)]
        cmid: Option<String>,
        content: Vec<serde_json::Value>,
        error: String,
    },
    CommandRejected {
        command_id: String,
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    ExecutionRequest {
        request: Box<crate::execution_protocol::RuntimeRequest>,
    },
    ExecutionReply {
        reply: Box<crate::execution_protocol::RuntimeReply>,
    },
    /// Unsequenced, non-retrying local diagnostic inspection pull (Runtime 9).
    PostEditInspectRequest {
        request: Box<PostEditInspectRequest>,
    },
    /// Unsequenced, non-retrying local diagnostic inspection reply (Runtime 9).
    PostEditInspectReply {
        reply: Box<PostEditInspectReply>,
    },
    Hello {
        role: PeerRole,
        min_protocol: u16,
        max_protocol: u16,
        build: String,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        worker_epoch: Option<String>,
        #[serde(default)]
        generation: Option<String>,
        #[serde(default)]
        executable: Option<String>,
        #[serde(default)]
        fallback_for: Option<String>,
        /// Outer Machine WebSocket protocol version negotiated with the
        /// Controller (`None` from older peers, failing closed as `< 53`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outer_machine_protocol: Option<u16>,
        /// True when `machine_cli` negotiated Controller support for the
        /// independent HTTPS post-edit inspection route (`POST /api/machine/post-edit-inspect`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        post_edit_http: bool,
    },
    Welcome {
        protocol: u16,
        controller_epoch: u64,
        #[serde(default)]
        workers: Vec<WorkerSnapshot>,
    },
    Reject {
        reason: String,
    },
    CoreCommand {
        command: CoreCommand,
    },
    WorkerCommand {
        session_id: String,
        command: WorkerCommand,
    },
    WorkerEvent {
        session_id: String,
        worker_epoch: String,
        runtime_seq: u64,
        event: RuntimeEvent,
        #[serde(
            default,
            skip_serializing_if = "Vec::is_empty",
            deserialize_with = "crate::runtime_trace::deserialize_spans"
        )]
        diagnostics: Vec<crate::runtime_trace::SpanRecord>,
    },
    Ack {
        session_id: String,
        worker_epoch: String,
        runtime_seq: u64,
    },
    /// Ask a worker to replay every still-unacknowledged event after this
    /// sequence. Used whenever a new Cowboy controller takes the lease while
    /// the worker-to-Machine broker connection itself stayed up.
    Replay {
        session_id: String,
        worker_epoch: String,
        after_runtime_seq: u64,
    },
    CommandAck {
        session_id: String,
        command_id: String,
        accepted: bool,
        #[serde(default)]
        reason: Option<String>,
    },
    Snapshot {
        // Keep the frequently queued wire envelope small. Serde still emits
        // the same object shape, so this is not a protocol change.
        worker: Box<WorkerSnapshot>,
    },
    /// Worker-issued receipt sent on Runtime 10+ only after `DrainCrashed`
    /// ended the ACP loop and the Controller acknowledged every outbox event.
    CrashedDrainCompleted {
        session_id: String,
        worker_epoch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_id: Option<String>,
    },
    /// Broker-only maintenance retirement notice sent on Machine 55+ / Runtime
    /// 10+ after a crashed worker confirmed completion, disconnected, and (for
    /// sandboxed Providers) had its account unit confirmed gone.
    CrashedWorkerDrained {
        session_id: String,
        worker_epoch: String,
    },
    Heartbeat,
}

#[must_use]
pub fn negotiate(local_min: u16, local_max: u16, peer_min: u16, peer_max: u16) -> Option<u16> {
    let low = local_min.max(peer_min);
    let high = local_max.min(peer_max);
    (low <= high).then_some(high)
}

/// Write one length-prefixed JSON frame.
///
/// # Errors
///
/// Returns an error when serialization, frame-size validation, or writing to
/// the transport fails.
#[derive(Deserialize)]
struct FrameTypePeek {
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictPostEditRequestFrame {
    #[serde(rename = "type")]
    _kind: String,
    request: Box<PostEditInspectRequest>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictPostEditReplyFrame {
    #[serde(rename = "type")]
    _kind: String,
    reply: Box<PostEditInspectReply>,
}

fn max_encoded_bytes_for_frame(frame: &Frame) -> usize {
    match frame {
        Frame::PostEditInspectRequest { .. } => {
            crate::post_edit_inspect::MAX_POST_EDIT_REQUEST_BYTES
        }
        Frame::PostEditInspectReply { .. } => crate::post_edit_inspect::MAX_POST_EDIT_REPLY_BYTES,
        _ => MAX_FRAME_BYTES,
    }
}

/// Decode one length-prefixed runtime frame payload using strict
/// `deny_unknown_fields` and raw-byte caps for post-edit diagnostic frames.
///
/// # Errors
///
/// Returns `InvalidData` when the payload exceeds its variant cap, contains
/// unknown diagnostic envelope fields, or is invalid JSON.
pub fn decode_runtime_frame(payload: &[u8]) -> io::Result<Frame> {
    if let Ok(peek) = serde_json::from_slice::<FrameTypePeek>(payload) {
        match peek.kind.as_deref() {
            Some("post_edit_inspect_request") => {
                if payload.len() > crate::post_edit_inspect::MAX_POST_EDIT_REQUEST_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "post_edit_inspect_request frame too large: {} bytes",
                            payload.len()
                        ),
                    ));
                }
                let strict: StrictPostEditRequestFrame = serde_json::from_slice(payload)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                return Ok(Frame::PostEditInspectRequest {
                    request: strict.request,
                });
            }
            Some("post_edit_inspect_reply") => {
                if payload.len() > crate::post_edit_inspect::MAX_POST_EDIT_REPLY_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "post_edit_inspect_reply frame too large: {} bytes",
                            payload.len()
                        ),
                    ));
                }
                let strict: StrictPostEditReplyFrame = serde_json::from_slice(payload)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                return Ok(Frame::PostEditInspectReply {
                    reply: strict.reply,
                });
            }
            _ => {}
        }
    }
    let frame: Frame = serde_json::from_slice(payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if matches!(
        frame,
        Frame::PostEditInspectRequest { .. } | Frame::PostEditInspectReply { .. }
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "post-edit diagnostic frame bypassed strict envelope decoder",
        ));
    }
    Ok(frame)
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &Frame) -> io::Result<()> {
    let payload = serde_json::to_vec(frame)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let limit = max_encoded_bytes_for_frame(frame);
    if payload.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "runtime frame too large: {} bytes (limit {limit})",
                payload.len()
            ),
        ));
    }
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runtime frame length overflow"))?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await
}

/// Read one length-prefixed JSON frame. EOF before a new header is a clean
/// disconnect; EOF inside a frame is reported as corruption.
///
/// # Errors
///
/// Returns an error when the frame is truncated, oversized, invalid JSON, or
/// the transport read fails.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Option<Frame>> {
    let mut header = [0_u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("runtime frame too large: {len} bytes"),
        ));
    }
    let mut payload = vec![0; len];
    reader.read_exact(&mut payload).await?;
    decode_runtime_frame(&payload).map(Some)
}

/// Stateful frame decoder for reads that participate in `tokio::select!` or a
/// timeout. `read_exact` is not cancellation-safe: dropping it after it consumed
/// part of a frame loses that progress and makes the next header start inside the
/// JSON payload. This decoder retains every completed partial read in the struct,
/// so cancelling `next()` and polling it again resumes at the exact byte offset.
pub struct FrameReader<R> {
    reader: R,
    header: [u8; 4],
    header_read: usize,
    payload: Vec<u8>,
    payload_read: usize,
}

impl<R> FrameReader<R> {
    #[must_use]
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            header: [0; 4],
            header_read: 0,
            payload: Vec::new(),
            payload_read: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    /// Reads the next frame while preserving partial input between calls.
    ///
    /// # Errors
    ///
    /// Returns an error when the stream contains a truncated, oversized, or
    /// invalid frame, or when the transport read fails.
    pub async fn next(&mut self) -> io::Result<Option<Frame>> {
        while self.header_read < self.header.len() {
            let read = self
                .reader
                .read(&mut self.header[self.header_read..])
                .await?;
            if read == 0 {
                if self.header_read == 0 {
                    return Ok(None);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "runtime frame ended inside header",
                ));
            }
            self.header_read += read;
        }

        if self.payload.is_empty() {
            let len = u32::from_be_bytes(self.header) as usize;
            if len > MAX_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("runtime frame too large: {len} bytes"),
                ));
            }
            self.payload.resize(len, 0);
        }
        while self.payload_read < self.payload.len() {
            let read = self
                .reader
                .read(&mut self.payload[self.payload_read..])
                .await?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "runtime frame ended inside payload",
                ));
            }
            self.payload_read += read;
        }

        let payload = std::mem::take(&mut self.payload);
        self.header_read = 0;
        self.payload_read = 0;
        decode_runtime_frame(&payload).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_snapshot(resume: Option<&str>, materialized: Option<bool>) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: "s".to_owned(),
            worker_epoch: "epoch".to_owned(),
            generation: "gen".to_owned(),
            executable: None,
            launch: None,
            state: WorkerState::Running,
            agent_session_id: resume.map(str::to_owned),
            native_thread_materialized: materialized,
            current_turn_id: None,
            supports_current_turn: false,
            delivery_controls: DeliveryControls::default(),
            wrap_up_steering: false,
            supports_subagent_cancel: false,
            last_runtime_seq: 0,
            pending_permissions: Vec::new(),
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
            exit_detail: None,
            background_tasks: None,
            native_cycle_id: None,
            native_cycle_active: false,
            incarnation: None,
            account_unit: None,
            uncertain_effect: None,
        }
    }

    #[test]
    fn native_cycles_round_trip_and_older_snapshots_still_decode() {
        assert_eq!(
            native_cycle(&native_cycle_update(Some("native-a"))),
            Some(Some("native-a".to_owned()))
        );
        assert_eq!(native_cycle(&native_cycle_update(None)), Some(None));
        assert_eq!(native_cycle(&background_tasks_update(1)), None);
        let mut snapshot = native_snapshot(None, None);
        let older = serde_json::to_value(&snapshot).unwrap();
        assert!(
            older.get("native_cycle_id").is_none(),
            "omitted when absent"
        );
        assert_eq!(
            serde_json::from_value::<WorkerSnapshot>(older).unwrap(),
            snapshot
        );
        assert_eq!(snapshot.delivery_turn_id(), None);
        snapshot.native_cycle_id = Some("native-a".to_owned());
        assert_eq!(snapshot.delivery_turn_id(), Some("native-a"));
        snapshot.current_turn_id = Some("turn-1".to_owned());
        assert_eq!(snapshot.delivery_turn_id(), Some("turn-1"));
    }

    #[test]
    fn a_native_thread_is_resumable_only_after_a_prompt_or_a_resume() {
        let mut fresh = native_snapshot(None, Some(false));
        fresh.observe_native_thread(&RuntimeEvent::AgentSessionId {
            agent_session_id: "thread-new".to_owned(),
            materialized: false,
        });
        assert_eq!(fresh.agent_session_id.as_deref(), Some("thread-new"));
        assert_eq!(fresh.resumable_agent_session_id(), None);

        fresh.observe_native_thread(&RuntimeEvent::TurnStarted {
            turn_id: "turn-1".to_owned(),
            command_id: "command-1".to_owned(),
        });
        assert_eq!(
            fresh.resumable_agent_session_id().as_deref(),
            Some("thread-new")
        );

        let mut resumed = native_snapshot(Some("thread-old"), Some(true));
        resumed.observe_native_thread(&RuntimeEvent::AgentSessionId {
            agent_session_id: "thread-old".to_owned(),
            materialized: false,
        });
        assert_eq!(
            resumed.resumable_agent_session_id().as_deref(),
            Some("thread-old")
        );
        resumed.observe_native_thread(&RuntimeEvent::AgentSessionId {
            agent_session_id: "thread-cleared".to_owned(),
            materialized: false,
        });
        assert_eq!(resumed.resumable_agent_session_id(), None);
    }

    #[test]
    fn a_forked_thread_is_resumable_before_its_first_prompt() {
        let mut forked = native_snapshot(None, Some(false));
        let event = RuntimeEvent::AgentSessionId {
            agent_session_id: "thread-fork".to_owned(),
            materialized: true,
        };
        forked.observe_native_thread(&event);
        assert_eq!(
            forked.resumable_agent_session_id().as_deref(),
            Some("thread-fork")
        );
        // The flag is additive: an older peer decodes the event unchanged and
        // a fresh thread does not serialize it.
        let wire = serde_json::to_value(&event).unwrap();
        assert_eq!(wire["materialized"], true);
        let fresh = serde_json::to_value(RuntimeEvent::AgentSessionId {
            agent_session_id: "thread-new".to_owned(),
            materialized: false,
        })
        .unwrap();
        assert!(fresh.get("materialized").is_none());
        assert_eq!(
            serde_json::from_value::<RuntimeEvent>(fresh).unwrap(),
            RuntimeEvent::AgentSessionId {
                agent_session_id: "thread-new".to_owned(),
                materialized: false,
            }
        );
    }

    #[test]
    fn the_lineage_is_additive_on_the_wire_in_both_directions() {
        let mut snapshot = native_snapshot(None, None);
        // An older Machine omits it and a Machine without a writer sends none.
        let absent = serde_json::to_value(&snapshot).unwrap();
        assert!(absent.get("incarnation").is_none());
        assert_eq!(
            serde_json::from_value::<WorkerSnapshot>(absent)
                .unwrap()
                .incarnation,
            None
        );
        snapshot.incarnation = Some("0123456789abcdef0123456789abcdef".to_owned());
        let present = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(present["incarnation"], "0123456789abcdef0123456789abcdef");
        assert_eq!(
            serde_json::from_value::<WorkerSnapshot>(present.clone()).unwrap(),
            snapshot
        );
        // A Controller that predates the field decodes a stamped snapshot by
        // ignoring it: the snapshot type does not deny unknown fields.
        let mut legacy = present;
        legacy["unknown_future_field"] = serde_json::json!(true);
        assert!(serde_json::from_value::<WorkerSnapshot>(legacy).is_ok());
    }

    #[test]
    fn subagent_cancel_is_a_separate_command_older_snapshots_never_advertise() {
        let mut snapshot = native_snapshot(None, None);
        let legacy = serde_json::to_value(&snapshot).unwrap();
        assert!(legacy.get("supports_subagent_cancel").is_none());
        assert!(
            !serde_json::from_value::<WorkerSnapshot>(legacy)
                .unwrap()
                .supports_subagent_cancel
        );
        snapshot.supports_subagent_cancel = true;
        let current = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(current["supports_subagent_cancel"], true);
        assert_eq!(
            serde_json::from_value::<WorkerSnapshot>(current).unwrap(),
            snapshot
        );
        // Its own tag: an older peer refuses it rather than reading it as
        // `cancel`, which would stop the whole turn.
        let command = serde_json::to_value(CoreCommand::CancelSubagent {
            session_id: "s".into(),
            command_id: "c".into(),
            subagent_session_id: "child".into(),
        })
        .unwrap();
        assert_eq!(command["command"], "cancel_subagent");
        let worker = serde_json::to_value(WorkerCommand::CancelSubagent {
            command_id: "c".into(),
            subagent_session_id: "child".into(),
        })
        .unwrap();
        assert_eq!(
            worker,
            serde_json::json!({"command": "cancel_subagent", "command_id": "c", "subagent_session_id": "child"})
        );
        assert!(serde_json::from_value::<WorkerCommand>(worker).is_ok());
    }

    #[test]
    fn a_legacy_snapshot_keeps_its_native_thread_resumable() {
        let legacy = serde_json::to_value(native_snapshot(Some("thread-1"), None)).unwrap();
        assert!(legacy.get("native_thread_materialized").is_none());
        let decoded: WorkerSnapshot = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.native_thread_materialized, None);
        assert_eq!(
            decoded.resumable_agent_session_id().as_deref(),
            Some("thread-1")
        );
    }

    #[test]
    fn echo_artifacts_is_an_additive_prompt_opt_in() {
        let legacy = serde_json::json!({"command":"prompt","command_id":"c","turn_id":"t","content":[],"cmid":"m"});
        let decoded: WorkerCommand = serde_json::from_value(legacy.clone()).unwrap();
        assert!(matches!(
            decoded,
            WorkerCommand::Prompt {
                echo_artifacts: false,
                observe_subagents: false,
                ..
            }
        ));
        assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
        let mut opted = legacy;
        opted["echo_artifacts"] = serde_json::json!(true);
        let decoded: WorkerCommand = serde_json::from_value(opted).unwrap();
        assert!(matches!(
            decoded,
            WorkerCommand::Prompt {
                echo_artifacts: true,
                observe_subagents: false,
                ..
            }
        ));
    }

    #[test]
    fn telemetry_metadata_is_optional_bounded_and_cannot_reject_a_valid_command() {
        #[derive(Deserialize)]
        struct LegacyEvent {
            event: RuntimeEvent,
        }
        let legacy = serde_json::json!({"command":"prompt","session_id":"s","command_id":"c","turn_id":"t","content":[],"cmid":"m"});
        let decoded: CoreCommand = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
        for malformed in [
            serde_json::json!(null),
            serde_json::json!("private value"),
            serde_json::json!({"context":{"traceparent":"bad"}}),
        ] {
            let mut value = legacy.clone();
            value["trace"] = malformed;
            let decoded: CoreCommand = serde_json::from_value(value).unwrap();
            assert!(matches!(decoded, CoreCommand::Prompt { trace: None, .. }));
        }
        let context = crate::runtime_trace::TraceContext {
            traceparent: "00-11111111111111111111111111111111-2222222222222222-01".into(),
            correlation_id: "33333333333333333333333333333333".into(),
        };
        let record = crate::runtime_trace::SpanTimer::start(
            &context,
            crate::runtime_trace::Stage::WorkerPrompt,
        )
        .unwrap()
        .finish(crate::runtime_trace::Outcome::Ok)
        .unwrap();
        let mut event = serde_json::json!({"type":"worker_event","session_id":"s","worker_epoch":"e","runtime_seq":1,"event":{"event":"turn_ended","turn_id":"t","stop_reason":"end_turn"}});
        for malformed in [
            serde_json::json!(["private value"]),
            serde_json::json!(vec![record.clone(); 5]),
            serde_json::json!({"arbitrary":"object"}),
        ] {
            event["diagnostics"] = malformed;
            let decoded: Frame = serde_json::from_value(event.clone()).unwrap();
            assert!(
                matches!(decoded, Frame::WorkerEvent { diagnostics, .. } if diagnostics.is_empty())
            );
        }
        event["diagnostics"] = serde_json::json!([record]);
        let decoded: Frame = serde_json::from_value(event.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), event);
        assert!(event["event"].get("diagnostics").is_none());
        // A v1 consumer of the same event shape ignores additive diagnostics.
        assert!(matches!(
            serde_json::from_value::<LegacyEvent>(event).unwrap().event,
            RuntimeEvent::TurnEnded { .. }
        ));
    }

    #[test]
    fn protocol_negotiation_requires_overlap_and_selects_newest() {
        assert_eq!(negotiate(1, 2, 2, 3), Some(2));
        assert_eq!(negotiate(1, 1, 2, 2), None);
        assert_eq!(negotiate(2, 4, 1, 3), Some(3));
    }

    #[tokio::test]
    async fn framed_json_round_trips_fragment_safe() {
        let (mut left, mut right) = tokio::io::duplex(128);
        let frame = Frame::WorkerEvent {
            diagnostics: Vec::new(),
            session_id: "sess-1".to_owned(),
            worker_epoch: "epoch-1".to_owned(),
            runtime_seq: 9,
            event: RuntimeEvent::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "hello\nworld"},
                }),
                cmid: None,
            },
        };
        let expected = frame.clone();
        let send = tokio::spawn(async move { write_frame(&mut left, &frame).await });
        let received = read_frame(&mut right).await.expect("read frame");
        send.await.expect("writer task").expect("write frame");
        assert_eq!(received, Some(expected));
    }

    #[tokio::test]
    async fn stateful_reader_resumes_after_repeated_select_cancellation() {
        let (mut writer, reader) = tokio::io::duplex(128);
        let payload = serde_json::to_vec(&Frame::Heartbeat).expect("serialize heartbeat");
        let len = u32::try_from(payload.len())
            .expect("small frame")
            .to_be_bytes();
        writer
            .write_all(&len[..2])
            .await
            .expect("write partial header");

        let mut reader = FrameReader::new(reader);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), reader.next())
                .await
                .is_err(),
            "partial header must still be waiting"
        );

        writer.write_all(&len[2..]).await.expect("finish header");
        writer
            .write_all(&payload[..2])
            .await
            .expect("write partial payload");

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), reader.next())
                .await
                .is_err(),
            "partial frame must still be waiting"
        );

        writer
            .write_all(&payload[2..])
            .await
            .expect("finish payload");
        assert!(matches!(
            reader.next().await.expect("resume frame"),
            Some(Frame::Heartbeat)
        ));
    }

    #[test]
    fn old_additive_frames_ignore_future_fields() {
        let raw = serde_json::json!({
            "type": "heartbeat",
            "futureOptionalField": true
        });
        let parsed: Frame = serde_json::from_value(raw).expect("additive field must be ignored");
        assert_eq!(parsed, Frame::Heartbeat);
    }

    #[test]
    fn old_worker_snapshot_defaults_new_recovery_fields() {
        let raw = serde_json::json!({
            "type": "snapshot",
            "worker": {
                "session_id": "sess-1",
                "worker_epoch": "epoch-1",
                "generation": "gen-1",
                "state": "busy"
            }
        });
        let Frame::Snapshot { worker } =
            serde_json::from_value(raw).expect("old snapshot remains compatible")
        else {
            panic!("snapshot frame")
        };
        assert!(worker.launch.is_none());
        assert!(worker.config_options.is_none());
        assert_eq!(worker.pending_prompt_count, 0);
    }

    #[test]
    fn old_ensure_session_defaults_to_real_spawn_semantics() {
        let raw = serde_json::json!({
            "command": "ensure_session",
            "session": {
                "session_id": "sess-1",
                "provider": "codex",
                "cwd": "/work",
                "generation": "gen-1"
            }
        });
        let CoreCommand::EnsureSession { session } =
            serde_json::from_value(raw).expect("old ensure remains compatible")
        else {
            panic!("ensure command")
        };
        assert!(!session.adopt_only);
        assert!(session.provider_account_id.is_default());
        assert_eq!(session.context_window, None);
        assert_eq!(session.auto_compact_token_limit, None);
    }

    #[test]
    fn account_identity_cannot_be_dropped_or_coerced_during_launch_decode() {
        let mut raw = serde_json::json!({"session_id":"s", "provider":"claude-code",
            "cwd":"/work", "generation":"g", "provider_account_id":"11111111-1111-4111-8111-111111111111"});
        let launch: StartSession = serde_json::from_value(raw.clone()).unwrap();
        assert!(!launch.provider_account_id.is_default());
        assert_eq!(
            serde_json::to_value(&launch).unwrap()["provider_account_id"],
            raw["provider_account_id"]
        );
        for id in [
            serde_json::Value::Null,
            serde_json::json!("../default"),
            serde_json::json!(""),
        ] {
            raw["provider_account_id"] = id;
            assert!(serde_json::from_value::<StartSession>(raw.clone()).is_err());
        }
    }

    #[test]
    fn post_edit_inspect_frames_round_trip_and_hello_defaults_post_edit_http() {
        let req = Frame::PostEditInspectRequest {
            request: Box::new(PostEditInspectRequest {
                session_id: "sess-1".into(),
                worker_epoch: "epoch-1".into(),
                request_id: "req-1".into(),
                turn_id: "turn-1".into(),
                scope: crate::post_edit_inspect::PostEditScope {
                    binding_id: "bind-1".into(),
                    revision: 1,
                    incarnation: "inc-1".into(),
                },
                path: "src/main.rs".into(),
                expected_lf_sha256: "b".repeat(64),
                remaining_ms: 4_000,
                canary_inspect_id: Some("canary-1".into()),
            }),
        };
        let encoded = serde_json::to_vec(&req).unwrap();
        assert_eq!(serde_json::from_slice::<Frame>(&encoded).unwrap(), req);

        let reply = Frame::PostEditInspectReply {
            reply: Box::new(PostEditInspectReply::refused(
                "sess-1".into(),
                "epoch-1".into(),
                "req-1".into(),
                crate::post_edit_inspect::PostEditScope {
                    binding_id: "bind-1".into(),
                    revision: 1,
                    incarnation: "inc-1".into(),
                },
                "unsupported",
            )),
        };
        let encoded_reply = serde_json::to_vec(&reply).unwrap();
        assert_eq!(
            serde_json::from_slice::<Frame>(&encoded_reply).unwrap(),
            reply
        );

        let legacy_hello = serde_json::json!({
            "type": "hello",
            "role": "core",
            "min_protocol": 1,
            "max_protocol": 8,
            "build": "legacy"
        });
        let decoded: Frame = serde_json::from_value(legacy_hello).unwrap();
        assert!(matches!(
            decoded,
            Frame::Hello {
                post_edit_http: false,
                outer_machine_protocol: None,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn post_edit_envelopes_reject_unknown_sibling_fields_and_oversized_frames() {
        let req = PostEditInspectRequest {
            session_id: "sess-1".into(),
            worker_epoch: "epoch-1".into(),
            request_id: "req-1".into(),
            turn_id: "turn-1".into(),
            scope: crate::post_edit_inspect::PostEditScope {
                binding_id: "bind-1".into(),
                revision: 1,
                incarnation: "inc-1".into(),
            },
            path: "src/main.rs".into(),
            expected_lf_sha256: "b".repeat(64),
            remaining_ms: 4_000,
            canary_inspect_id: None,
        };
        // Unknown sibling field at Frame envelope level must be rejected (M-Wire-1).
        let with_extra = serde_json::json!({
            "type": "post_edit_inspect_request",
            "request": req,
            "unexpected_sibling": true
        });
        let raw = serde_json::to_vec(&with_extra).unwrap();
        assert!(decode_runtime_frame(&raw).is_err());

        // Oversized raw request envelope (> 8 KiB) must be rejected on read and write.
        let mut oversized_req = req.clone();
        oversized_req.path = "a".repeat(crate::post_edit_inspect::MAX_POST_EDIT_REQUEST_BYTES);
        let mut sink = Vec::new();
        assert!(
            write_frame(
                &mut sink,
                &Frame::PostEditInspectRequest {
                    request: Box::new(oversized_req.clone()),
                },
            )
            .await
            .is_err()
        );
        let oversized_raw = serde_json::to_vec(&Frame::PostEditInspectRequest {
            request: Box::new(oversized_req),
        })
        .unwrap();
        assert!(decode_runtime_frame(&oversized_raw).is_err());

        // Oversized refusal reply (> 16 KiB) must be rejected on write.
        let oversized_reply = Frame::PostEditInspectReply {
            reply: Box::new(PostEditInspectReply::refused(
                "s".repeat(crate::post_edit_inspect::MAX_POST_EDIT_REPLY_BYTES),
                "epoch-1".into(),
                "req-1".into(),
                crate::post_edit_inspect::PostEditScope {
                    binding_id: "bind-1".into(),
                    revision: 1,
                    incarnation: "inc-1".into(),
                },
                "unavailable",
            )),
        };
        assert!(write_frame(&mut sink, &oversized_reply).await.is_err());

        // M2: Escaped JSON discriminator string must still hit strict validation & raw size cap.
        let escaped_unknown = format!(
            r#"{{"type":"post_edit\u005finspect_request","extra":1,"request":{}}}"#,
            serde_json::to_string(&req).unwrap()
        );
        assert!(decode_runtime_frame(escaped_unknown.as_bytes()).is_err());
        let escaped_oversized = format!(
            r#"{{"type":"post_edit\u005finspect_request","request":{}}}"#,
            " ".repeat(crate::post_edit_inspect::MAX_POST_EDIT_REQUEST_BYTES + 32)
        );
        assert!(decode_runtime_frame(escaped_oversized.as_bytes()).is_err());
    }

    #[test]
    fn turn_usage_for_provider_nulls_unmetered_zero_cache_write_and_keeps_positive_and_claude_zero()
    {
        let zero_write = agent_client_protocol::schema::v1::Usage::new(100, 90, 10)
            .cached_read_tokens(50)
            .cached_write_tokens(0);
        assert_eq!(
            TurnUsage::from_acp_for_provider("claude-code", &zero_write)
                .unwrap()
                .cache_write_tokens,
            Some(0)
        );
        assert_eq!(
            TurnUsage::from_acp_for_provider("third-party-acp", &zero_write)
                .unwrap()
                .cache_write_tokens,
            Some(0)
        );
        for provider in [
            "codex",
            "antigravity",
            "claude-deepseek",
            "codex-deepseek",
            "deepseek-harness",
            "grok",
        ] {
            let projected = TurnUsage::from_acp_for_provider(provider, &zero_write).unwrap();
            assert_eq!(projected.cached_input_tokens, Some(50), "{provider}");
            assert_eq!(projected.uncached_input_tokens, Some(40), "{provider}");
            assert_eq!(projected.cache_write_tokens, None, "{provider}");
        }
        let positive_write = agent_client_protocol::schema::v1::Usage::new(100, 90, 10)
            .cached_read_tokens(50)
            .cached_write_tokens(20);
        for provider in ["claude-code", "codex", "antigravity", "grok"] {
            assert_eq!(
                TurnUsage::from_acp_for_provider(provider, &positive_write)
                    .unwrap()
                    .cache_write_tokens,
                Some(20),
                "{provider}"
            );
        }
    }

    #[test]
    fn drain_crashed_worker_wire_round_trip() {
        let core = CoreCommand::DrainCrashedWorker {
            session_id: "sess-1".into(),
            command_id: "drain-crashed-1".into(),
            worker_epoch: "42-99".into(),
        };
        assert_eq!(core.session_id(), Some("sess-1"));
        let core_json = serde_json::to_value(&core).unwrap();
        assert_eq!(core_json["command"], "drain_crashed_worker");
        assert_eq!(core_json["worker_epoch"], "42-99");
        assert_eq!(
            serde_json::from_value::<CoreCommand>(core_json).unwrap(),
            core
        );

        let worker = WorkerCommand::DrainCrashed {
            command_id: "drain-crashed-1".into(),
            worker_epoch: "42-99".into(),
        };
        let worker_json = serde_json::to_value(&worker).unwrap();
        assert_eq!(worker_json["command"], "drain_crashed");
        assert_eq!(worker_json["worker_epoch"], "42-99");
        assert_eq!(
            serde_json::from_value::<WorkerCommand>(worker_json).unwrap(),
            worker
        );

        let completed = Frame::CrashedDrainCompleted {
            session_id: "sess-1".into(),
            worker_epoch: "42-99".into(),
            command_id: Some("dc-1".into()),
        };
        let completed_json = serde_json::to_value(&completed).unwrap();
        assert_eq!(completed_json["type"], "crashed_drain_completed");
        assert_eq!(
            serde_json::from_value::<Frame>(completed_json).unwrap(),
            completed
        );

        let drained = Frame::CrashedWorkerDrained {
            session_id: "sess-1".into(),
            worker_epoch: "42-99".into(),
        };
        let drained_json = serde_json::to_value(&drained).unwrap();
        assert_eq!(drained_json["type"], "crashed_worker_drained");
        assert_eq!(
            serde_json::from_value::<Frame>(drained_json).unwrap(),
            drained
        );
    }
}
