//! Enrolled Machine execution control. Local keeper capabilities stay on the
//! target; a Controller routes only session-bound commands and exact identities.

use crate::execution_environment::{BindingV1, PreparationV1, RuntimeLocation};
use crate::execution_protocol::{Command, Response as KeeperResponse};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub service_id: String,
    pub machine_id: String,
    pub action: Action,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Inventory,
    Recover {
        session_id: String,
        intent: Box<crate::execution_environment::RecoveryV1>,
    },
    PrepareRuntime {
        session_id: String,
    },
    Prepare {
        session_id: String,
        workspace_id: String,
        runtime: RuntimeLocation,
    },
    Call {
        session_id: String,
        binding: Box<BindingV1>,
        command: Command,
    },
    Close {
        session_id: String,
        binding: Box<BindingV1>,
    },
    AbandonPreparation {
        session_id: String,
        preparation: Box<PreparationV1>,
    },
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.action {
            Action::Inventory => "ExecutionInventory",
            Action::Recover { .. } => "ExecutionRecovery",
            Action::PrepareRuntime { .. } => "ExecutionRuntimePrepare",
            Action::Prepare { .. } => "ExecutionPrepare",
            Action::Call { .. } => "ExecutionCall",
            Action::Close { .. } => "ExecutionClose",
            Action::AbandonPreparation { .. } => "ExecutionAbandonPreparation",
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Inventory { executor: Option<ExecutorInventory> },
    Prepared { binding: BindingV1 },
    RuntimePrepared { runtime: RuntimeLocation },
    Closed,
    Call { response: KeeperResponse },
    Refused { reason: Refusal },
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Inventory { .. } => "ExecutionInventory",
            Self::Prepared { .. } => "ExecutionPrepared",
            Self::RuntimePrepared { .. } => "ExecutionRuntimePrepared",
            Self::Closed => "ExecutionClosed",
            Self::Call { .. } => "ExecutionResult",
            Self::Refused { .. } => "ExecutionRefused",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorInventory {
    pub protocol: u16,
    pub digest: String,
    pub version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    Unavailable,
    InvalidRequest,
    IdentityMismatch,
    WorkspaceUnavailable,
    EnvironmentLost,
    PreparationFailed,
    Capacity,
}
