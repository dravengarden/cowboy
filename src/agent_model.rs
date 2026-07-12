//! Types and markers shared by the restartable control plane and detached ACP
//! workers. Keeping this boundary small lets worker-generation hashing ignore
//! unrelated Hub/API edits without relying on a hand-waved compatibility rule.

use serde::{Deserialize, Serialize};

/// Provider/session status as shown in the session list.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Agent subprocess spawning / ACP handshake in flight.
    Starting,
    /// Session established; idle, ready for a prompt.
    Running,
    /// A prompt turn is currently being processed.
    Busy,
    /// Agent exited cleanly (or was stopped).
    Exited,
    /// Agent crashed / the ACP connection failed.
    Crashed,
    /// A turn had no surviving worker during control-plane restore.
    Interrupted,
}

/// Latest provider-reported usage for one ACP session. `raw` preserves optional
/// standard fields (`cost`) and provider extensions (`_meta`) without teaching
/// the worker protocol every provider's private schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionUsage {
    pub used: u64,
    pub size: u64,
    #[serde(default)]
    pub raw: serde_json::Value,
    #[serde(default)]
    pub observed_at_ms: i64,
}

/// A normalized session event fanned out to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A serialized ACP `SessionUpdate`.
    Update {
        update: serde_json::Value,
    },
    PermissionRequest {
        request_id: String,
        tool_call: serde_json::Value,
        options: serde_json::Value,
    },
    PermissionResolved {
        request_id: String,
        option_id: Option<String>,
    },
    Lifecycle {
        status: Status,
        detail: Option<String>,
    },
    TurnEnd {
        stop_reason: String,
    },
}

/// Optimistic-message markers interpreted by the ACP echo boundary.
pub(crate) const AUTO_CONTINUE_PREFIX: &str = "__cont__";
pub(crate) const SCHED_PREFIX: &str = "__sched__";
pub const WAKEUP_PREFIX: &str = "__wake__";
