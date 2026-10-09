//! Session-scoped execution IPC. Transport identities never select a new target.
//!
//! The Machine owns the launch contract and its private capability. An Agent
//! receives a local transport through its worker, never these credentials or an
//! arbitrary destination. Backend JSON-RPC is opaque to Controller routing.

use crate::execution_environment::BindingV1;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: u16 = 1;
// Leave envelope headroom below the enrolled WebSocket's 16 MiB frame bound.
// This carries ordinary multi-megabyte screenshots without another tool turn.
pub const MAX_FRAME_BYTES: usize = 14 * 1024 * 1024;
pub const MAX_WAIT_MS: u64 = 20_000;

/// Private worker-to-Service request. Only the enrolled runtime Machine may
/// forward it, and both brokers validate the original session and worker epoch.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequest {
    pub session_id: String,
    pub worker_epoch: String,
    pub request_id: String,
    pub binding: BindingV1,
    pub command: Command,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeReply {
    pub session_id: String,
    pub worker_epoch: String,
    pub request_id: String,
    pub scope: Scope,
    pub response: crate::machine_protocol::execution::Response,
}

impl std::fmt::Debug for RuntimeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExecutionRequest([private])")
    }
}

impl std::fmt::Debug for RuntimeReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExecutionReply([private])")
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchContract {
    pub schema: u16,
    pub session_id: String,
    pub binding: BindingV1,
    pub executor: Executor,
    pub capability: String,
    /// Closed target-owned environment; no Agent credentials are inherited.
    pub environment: BTreeMap<String, String>,
    /// Machine-owned managed-call context path for this session. The keeper
    /// adds it to native process starts after admission; it is never part of
    /// the operator environment or an admitted invocation's parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_context: Option<String>,
}

/// Name of the private variable that locates a session's call context.
pub const CALL_CONTEXT_ENV: &str = "COWBOY_CALL_CONTEXT";

/// Add the session call context to one native process start. Applied to the
/// outbound native frame only, so ledger replay compares the original params.
#[must_use]
pub fn with_call_context(
    method: &str,
    params: &serde_json::Value,
    context: Option<&str>,
) -> serde_json::Value {
    let mut params = params.clone();
    if method == "process/start"
        && let Some(context) = context
        && let Some(env) = params
            .as_object_mut()
            .and_then(|params| params.get_mut("env"))
            .and_then(serde_json::Value::as_object_mut)
    {
        env.insert(
            CALL_CONTEXT_ENV.to_owned(),
            serde_json::Value::String(context.to_owned()),
        );
    }
    params
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Executor {
    pub command: String,
    pub sha256: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub binding_id: String,
    pub revision: u64,
    pub incarnation: String,
}

impl Scope {
    pub fn from_binding(binding: &BindingV1) -> Self {
        Self {
            binding_id: binding.id.clone(),
            revision: binding.revision,
            incarnation: binding.environment.incarnation.clone(),
        }
    }
}

// Deliberately no Debug: this local request includes a capability and may carry
// private source or command output. Logs must contain only bounded reason codes.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u16,
    pub scope: Scope,
    pub capability: String,
    pub command: Command,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Describe,
    Shutdown,
    Invoke {
        invocation: Invocation,
        wait_ms: u64,
    },
    Observe {
        operation_id: String,
        wait_ms: u64,
    },
    Events {
        after: u64,
        wait_ms: u64,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    /// Caller retains this identity through disconnects. Transport correlation
    /// IDs and native JSON-RPC IDs are not effect identities.
    pub operation_id: String,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Closed,
    Ready {
        scope: Scope,
        initialization: serde_json::Value,
        event_cursor: u64,
    },
    Operation {
        outcome: Outcome,
    },
    Events {
        events: Vec<Event>,
        through: u64,
    },
    Refused {
        reason: Refusal,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Pending,
    /// Backend result or error, with its private correlation ID removed.
    Completed {
        reply: serde_json::Value,
    },
    /// The keeper admitted the call, but cannot prove its final effect. Never
    /// replay it. Loss of retained bytes is distinct from a missing operation.
    Unknown,
    ResultExpired,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub sequence: u64,
    pub message: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    Unauthorized,
    InvalidRequest,
    EnvironmentLost,
    WorkspaceChanged,
    OperationConflict,
    Capacity,
    CursorExpired,
}

pub fn valid_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

impl Invocation {
    pub fn validate(&self) -> bool {
        valid_operation_id(&self.operation_id)
            && !self.method.is_empty()
            && self.method.len() <= 128
            && self
                .method
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/_".contains(&byte))
            && !matches!(self.method.as_str(), "initialize" | "initialized")
            && self.params.is_object()
            && serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= MAX_FRAME_BYTES / 2)
    }
}
