//! Closed MCP binding declarations. These records carry identity and policy,
//! never a command, endpoint, source credential or reusable execution grant.
use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use cowboy_plugin_sdk::McpServerSelection;
use serde::{Deserialize, Serialize};

pub const MAX_BINDINGS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub name: String,
    pub server: McpServerSelection,
    pub tools: Vec<String>,
}

impl Binding {
    pub fn validate(&self) -> Result<()> {
        ensure!(valid_name(&self.name), "invalid MCP binding name");
        self.server.validate()?;
        ensure!(
            (1..=16).contains(&self.tools.len())
                && self.tools.iter().collect::<BTreeSet<_>>().len() == self.tools.len()
                && self.tools.iter().all(|name| valid_name(name)),
            "invalid MCP binding tools"
        );
        Ok(())
    }
}

/// Controller-issued immutable process scope. The Machine still requires its
/// own live connection lease; a serialized profile grants no access by itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessProfile {
    pub schema: u16,
    pub service_id: String,
    pub session_id: String,
    pub runtime_machine: String,
    pub execution_machine: String,
    pub workspace_id: String,
    pub epoch: String,
    pub binding: Binding,
}

impl ProcessProfile {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1
                && crate::service_identity::valid_service_id(&self.service_id)
                && [
                    &self.session_id,
                    &self.runtime_machine,
                    &self.execution_machine,
                    &self.workspace_id
                ]
                .into_iter()
                .all(|id| crate::opaque_id::valid_id(id))
                && valid_epoch(&self.epoch),
            "invalid MCP process profile"
        );
        self.binding.validate()
    }
}

pub fn validate_bindings(bindings: &[Binding]) -> Result<()> {
    ensure!(bindings.len() <= MAX_BINDINGS, "too many MCP bindings");
    let mut names = BTreeSet::new();
    for binding in bindings {
        binding.validate()?;
        ensure!(names.insert(&binding.name), "duplicate MCP binding name");
    }
    Ok(())
}

pub fn valid_epoch(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn binding_declarations_are_bounded_and_do_not_accept_launch_configuration() {
        let value = json!({"name":"fixture","server":{"plugin_id":"fixture-mcp","generation_digest":format!("sha256:{}", "a".repeat(64))},"tools":["fixture_read"]});
        let binding: Binding = serde_json::from_value(value.clone()).unwrap();
        validate_bindings(std::slice::from_ref(&binding)).unwrap();
        assert!(validate_bindings(&[binding.clone(), binding.clone()]).is_err());
        for field in ["command", "env", "endpoint", "credentials", "workspace"] {
            let mut extra = value.clone();
            extra[field] = json!("not accepted");
            assert!(serde_json::from_value::<Binding>(extra).is_err());
        }
        for tools in [vec![], vec!["x".into(), "x".into()], vec!["x".repeat(65)]] {
            assert!(
                Binding {
                    tools,
                    ..binding.clone()
                }
                .validate()
                .is_err()
            );
        }
        for epoch in ["a".repeat(31), "A".repeat(32), "g".repeat(32)] {
            assert!(!valid_epoch(&epoch));
        }
    }
}

/// Only an authenticated Controller connection may deliver these commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransportRequest {
    ConfirmNative {
        handle: String,
    },
    ReleaseNative {
        handle: String,
    },
    PrepareNative {
        profile: Box<ProcessProfile>,
    },
    Renew {
        handle: String,
        profile: Box<ProcessProfile>,
    },
    Open {
        profile: Box<ProcessProfile>,
    },
    Request {
        handle: String,
        message: serde_json::Value,
    },
    Close {
        handle: String,
    },
}

impl TransportRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            serde_json::to_vec(self)?.len() <= 1024 * 1024,
            "MCP transport frame too large"
        );
        match self {
            Self::ReleaseNative { handle } | Self::ConfirmNative { handle } => {
                ensure!(
                    handle.strip_prefix("native-").is_some_and(valid_epoch),
                    "invalid native descriptor handle"
                );
                Ok(())
            }
            Self::Open { profile } | Self::PrepareNative { profile } => profile.validate(),
            Self::Request { handle, .. } | Self::Close { handle } | Self::Renew { handle, .. } => {
                ensure!(
                    handle.strip_prefix("mcp-").is_some_and(valid_epoch),
                    "invalid MCP handle"
                );
                if let Self::Renew { profile, .. } = self {
                    profile.validate()?;
                }
                Ok(())
            }
        }
    }
}

/// Agent input selects an admitted binding; it cannot supply a process profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayRequest {
    pub binding: String,
    pub epoch: String,
    pub action: RelayAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RelayAction {
    Open,
    Request {
        handle: String,
        message: serde_json::Value,
    },
    Close {
        handle: String,
    },
}

impl RelayRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            valid_name(&self.binding) && valid_epoch(&self.epoch),
            "invalid MCP selection"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= 256 * 1024,
            "MCP gateway request too large"
        );
        match &self.action {
            RelayAction::Open => {}
            RelayAction::Request { handle, .. } | RelayAction::Close { handle } => {
                ensure!(
                    handle.strip_prefix("relay-").is_some_and(valid_epoch),
                    "invalid MCP relay handle"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod relay_tests {
    use super::*;
    use crate::machine_protocol::{MachineCommand, execution};
    use crate::managed_calls::protocol::Grant;
    use serde_json::json;

    #[test]
    fn relay_inputs_cannot_supply_profiles_or_unbounded_messages() {
        let input = json!({"binding":"suger","epoch":"a".repeat(32),"action":{"kind":"open"}});
        serde_json::from_value::<RelayRequest>(input.clone())
            .unwrap()
            .validate()
            .unwrap();
        let mut injected = input;
        injected["profile"] = json!({"execution_machine":"other"});
        assert!(serde_json::from_value::<RelayRequest>(injected).is_err());
        let request = RelayRequest {
            binding: "suger".into(),
            epoch: "a".repeat(32),
            action: RelayAction::Request {
                handle: format!("relay-{}", "a".repeat(32)),
                message: json!({"data":"x".repeat(256 * 1024)}),
            },
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn enabled_mcp_grant_requires_new_machine_protocol() {
        let mut grant = Grant {
            grant_id: "grant".into(),
            parent_session_id: "parent".into(),
            agent_api: true,
            agent_actions: true,
            mcp: false,
        };
        assert!(serde_json::to_value(&grant).unwrap().get("mcp").is_none());
        grant.mcp = true;
        let command = MachineCommand::Execution {
            request_id: "install".into(),
            request: Box::new(execution::Request {
                service_id: "service-test".into(),
                machine_id: "hawk".into(),
                action: execution::Action::InstallCallGateway { grant },
            }),
        };
        assert_eq!(command.minimum_protocol(), 42);
    }
}

/// Private launcher-written startup metadata. It is not an execution grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDescriptor {
    pub schema: u16,
    pub binding: Binding,
    pub epoch: String,
    pub server_version: String,
    pub tools: Vec<cowboy_plugin_sdk::McpToolDefinition>,
}

impl NativeDescriptor {
    pub fn validate(&self) -> Result<()> {
        self.binding.validate()?;
        ensure!(
            self.schema == 1
                && valid_epoch(&self.epoch)
                && !self.server_version.is_empty()
                && self.server_version.len() <= 64
                && !self.server_version.chars().any(char::is_control)
                && serde_json::to_vec(self)?.len() <= 64 * 1024,
            "invalid native MCP descriptor"
        );
        cowboy_plugin_sdk::validate_mcp_discovery(&self.tools, &self.binding.tools)
    }
}

/// Reported by the exact executor-side CLI; Machine version is insufficient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBridgeCapabilities {
    pub schema: u16,
    pub descriptor_schema: u16,
    pub relay_schema: u16,
    pub grant_mcp: bool,
    pub transport: String,
}

impl NativeBridgeCapabilities {
    pub fn current() -> Self {
        Self {
            schema: 1,
            descriptor_schema: 1,
            relay_schema: 1,
            grant_mcp: true,
            transport: "stdio_tool_only_v1".into(),
        }
    }

    pub fn compatible(&self) -> bool {
        *self == Self::current()
    }
}

#[cfg(test)]
mod native_capability_tests {
    use super::*;

    #[test]
    fn native_bridge_capabilities_require_the_exact_closed_contract() {
        let current = NativeBridgeCapabilities::current();
        assert!(current.compatible());
        for field in [
            "schema",
            "descriptor_schema",
            "relay_schema",
            "grant_mcp",
            "transport",
        ] {
            let mut value = serde_json::to_value(&current).unwrap();
            value.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<NativeBridgeCapabilities>(value).is_err());
        }
        for field in ["schema", "descriptor_schema", "relay_schema"] {
            let mut value = serde_json::to_value(&current).unwrap();
            value[field] = serde_json::json!(2);
            assert!(
                !serde_json::from_value::<NativeBridgeCapabilities>(value)
                    .unwrap()
                    .compatible()
            );
        }
        let mut old = current.clone();
        old.grant_mcp = false;
        assert!(!old.compatible());
        old = current;
        old.transport = "http".into();
        assert!(!old.compatible());
        let mut value = serde_json::to_value(old).unwrap();
        value["command"] = serde_json::json!("untrusted");
        assert!(serde_json::from_value::<NativeBridgeCapabilities>(value).is_err());
    }
}

/// Immutable startup metadata; possession never grants execution authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLaunchPlan {
    pub schema: u16,
    pub command: String,
    pub bindings: Vec<NativeLaunchBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLaunchBinding {
    pub name: String,
    pub descriptor: String,
}

impl NativeLaunchPlan {
    pub fn validate(&self) -> Result<()> {
        let absolute =
            |path: &str| path.starts_with('/') && path.len() <= 4096 && !path.contains('\0');
        ensure!(
            self.schema == 1
                && absolute(&self.command)
                && (1..=MAX_BINDINGS).contains(&self.bindings.len()),
            "invalid MCP launch plan"
        );
        let mut names = BTreeSet::new();
        for binding in &self.bindings {
            ensure!(
                valid_name(&binding.name)
                    && names.insert(&binding.name)
                    && absolute(&binding.descriptor),
                "invalid MCP launch binding"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod launch_plan_tests {
    use super::*;

    #[test]
    fn launch_plan_is_closed_bounded_and_unambiguous() {
        let plan = NativeLaunchPlan {
            schema: 1,
            command: "/bin/cowboy".into(),
            bindings: vec![NativeLaunchBinding {
                name: "suger".into(),
                descriptor: "/private/suger.json".into(),
            }],
        };
        assert!(plan.validate().is_ok());
        let mut changed = plan.clone();
        changed.bindings.push(changed.bindings[0].clone());
        assert!(changed.validate().is_err());
        for path in ["relative", "/bad\0path"] {
            let mut changed = plan.clone();
            changed.bindings[0].descriptor = path.into();
            assert!(changed.validate().is_err());
        }
        let mut changed = plan.clone();
        changed.bindings.clear();
        assert!(changed.validate().is_err());
        let mut changed = plan.clone();
        changed.schema = 2;
        assert!(changed.validate().is_err());
        let mut value = serde_json::to_value(plan).unwrap();
        value["authority"] = serde_json::json!(true);
        assert!(serde_json::from_value::<NativeLaunchPlan>(value).is_err());
    }
}
