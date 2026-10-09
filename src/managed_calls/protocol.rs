//! Closed gateway messages. Validation is repeated at every trust boundary;
//! the CLI parser is convenience, not a server authorization boundary.

use serde::{Deserialize, Serialize};

use super::{InputError, Request, valid_id};

pub const MAX_WAIT_MS: u64 = 60_000;
pub const MAX_FRAME_BYTES: usize = super::MAX_REQUEST_BYTES + 4096;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Capabilities {},
    Start {
        provider: String,
        request: Request,
        wait_ms: u64,
    },
    Inspect {
        call_id: String,
    },
    Wait {
        call_id: String,
        timeout_ms: u64,
    },
    Result {
        call_id: String,
    },
    Cancel {
        call_id: String,
    },
    Observe {
        request_id: String,
    },
}

impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedCallAction([private])")
    }
}

impl Action {
    pub fn validate(&self) -> Result<(), InputError> {
        match self {
            Self::Capabilities {} => Ok(()),
            Self::Start {
                provider,
                request,
                wait_ms,
            } => {
                if !matches!(provider.as_str(), "codex" | "claude-code") || *wait_ms > MAX_WAIT_MS {
                    return Err(InputError::InvalidContract);
                }
                request.validate()?;
                if serde_json::to_vec(request)
                    .map_or(true, |bytes| bytes.len() > super::MAX_REQUEST_BYTES)
                {
                    return Err(InputError::RequestTooLarge);
                }
                Ok(())
            }
            Self::Wait {
                call_id,
                timeout_ms,
            } => {
                if valid_id(call_id) && *timeout_ms <= MAX_WAIT_MS {
                    Ok(())
                } else {
                    Err(InputError::InvalidContract)
                }
            }
            Self::Inspect { call_id } | Self::Result { call_id } | Self::Cancel { call_id } => {
                if valid_id(call_id) {
                    Ok(())
                } else {
                    Err(InputError::InvalidContract)
                }
            }
            Self::Observe { request_id } => {
                if valid_id(request_id) {
                    Ok(())
                } else {
                    Err(InputError::InvalidRequestId)
                }
            }
        }
    }

    pub fn wait_ms(&self) -> u64 {
        match self {
            Self::Start { wait_ms, .. } => *wait_ms,
            Self::Wait { timeout_ms, .. } => *timeout_ms,
            _ => 0,
        }
    }
}

impl Action {
    /// Starting a call is the only action whose lost reply leaves admission
    /// uncertain; every other action is an observation or an idempotent stop.
    pub fn submits(&self) -> bool {
        matches!(self, Self::Start { .. })
    }
}

/// Controller-issued parent grant installed on the execution Machine. The id
/// is an opaque revocation handle; the Controller re-derives authority from
/// the live parent worker for every action, so possession grants nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub grant_id: String,
    pub parent_session_id: String,
}

impl Grant {
    pub fn validate(&self) -> bool {
        valid_id(&self.grant_id) && valid_id(&self.parent_session_id)
    }
}

/// One target-owned managed child round: an immutable input snapshot plus the
/// native turn constraint the child worker forwards for exactly this call.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildRound {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub call_id: String,
    pub workspace_id: String,
    /// The parent's execution directory on this Machine. The Machine checks
    /// it against its own worktree roots and registered workspaces.
    pub source_cwd: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    pub profile: cowboy_provider_sdk::ManagedRuntimeProfile,
}

impl std::fmt::Debug for ChildRound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedChildRound([private])")
    }
}

impl ChildRound {
    pub fn validate(&self) -> bool {
        [
            &self.child_session_id,
            &self.parent_session_id,
            &self.call_id,
            &self.workspace_id,
        ]
        .into_iter()
        .all(|id| valid_id(id))
            && self.child_session_id != self.parent_session_id
            && self.source_cwd.starts_with('/')
            && self.source_cwd.len() <= 4096
            && !self.source_cwd.contains('\0')
            && self.files.len() <= 32
            && self.files.iter().all(|file| super::relative_file(file))
            && self.output_schema.as_ref().is_none_or(|schema| {
                super::OutputFormat::JsonSchema {
                    schema: schema.clone(),
                }
                .validate()
            })
    }
}

/// Target receipt for a prepared round. `cwd` is stable for the child; the
/// input revision identifies the exact captured content of this round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildPrepared {
    pub cwd: String,
    pub input_revision: String,
    pub head: String,
    pub index_tree: String,
    pub worktree_tree: String,
}

// Never derive Debug: this envelope contains a private local capability.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalRequest {
    pub schema: u16,
    pub capability: String,
    pub action: Action,
}

impl LocalRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, InputError> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(InputError::RequestTooLarge);
        }
        let request: Self =
            serde_json::from_slice(bytes).map_err(|_| InputError::InvalidContract)?;
        if request.schema != 1 {
            return Err(InputError::UnsupportedSchema);
        }
        if !valid_capability(&request.capability) {
            return Err(InputError::InvalidContract);
        }
        request.action.validate()?;
        Ok(request)
    }
}

pub fn valid_capability(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn raw_gateway_clients_cannot_bypass_cli_limits_or_inject_identity() {
        let valid = json!({"schema":1,"capability":"a".repeat(64),"action":{"kind":"wait","call_id":"call-1","timeout_ms":60000}});
        assert!(LocalRequest::parse(&serde_json::to_vec(&valid).unwrap()).is_ok());
        for action in [
            json!({"kind":"wait","call_id":"call-1","timeout_ms":60001}),
            json!({"kind":"inspect","call_id":"../another-parent"}),
            json!({"kind":"capabilities","parent_session_id":"victim"}),
        ] {
            let mut value = valid.clone();
            value["action"] = action;
            assert!(LocalRequest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        let mut value = valid.clone();
        value["parent_session_id"] = json!("victim");
        assert!(LocalRequest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value = valid;
        value["capability"] = json!("a".repeat(63));
        assert!(LocalRequest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
