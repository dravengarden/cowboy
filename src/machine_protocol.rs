//! Stable control contract between Cowboy and a `cowboy-machine` host.
//!
//! ACP worker traffic remains defined by [`crate::runtime_wire`]. Zed's raw
//! protocol remains isolated in the GPL adapter. This module owns only machine
//! identity, inventory, component lifecycle, and multiplexing.

#![warn(clippy::pedantic)]

use serde::{Deserialize, Serialize};

pub const MACHINE_PROTOCOL_VERSION: u16 = 1;
pub const MIN_MACHINE_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Linux,
    Macos,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    LocalUds,
    OutboundTls,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    MachineHost,
    AcpRuntime,
    ProviderAdapter,
    ProviderCli,
    ZedAdapter,
    ZedServer,
    ManagedNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    Missing,
    Downloading,
    Staged,
    Active,
    Draining,
    RollingBack,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Unsupported,
    SignedOut,
    Pending,
    SignedIn,
    Expired,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentId {
    pub kind: ComponentKind,
    /// Provider id, Zed adapter ABI, or Zed client version. Empty only for
    /// singleton components such as the Machine host and managed Node runtime.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub slot: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentInventory {
    pub id: ComponentId,
    pub state: ComponentState,
    pub version: String,
    pub generation: String,
    pub digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_generation: Option<String>,
    #[serde(default)]
    pub active_leases: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineHello {
    pub machine_id: String,
    pub display_name: String,
    pub platform: Platform,
    pub arch: String,
    pub connection_mode: ConnectionMode,
    pub min_protocol: u16,
    pub max_protocol: u16,
    pub min_runtime_protocol: u16,
    pub max_runtime_protocol: u16,
    pub host_build: String,
    /// Fresh value signed by the enrolled machine credential at the transport
    /// handshake. Local UDS connections leave both fields absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_signature: Option<String>,
    #[serde(default)]
    pub components: Vec<ComponentInventory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredComponent {
    pub id: ComponentId,
    pub version: String,
    pub generation: String,
    pub artifact_url: String,
    pub digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default)]
    pub automatic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum MachineCommand {
    Reconcile {
        request_id: String,
        components: Vec<DesiredComponent>,
    },
    BeginLogin {
        request_id: String,
        provider: String,
    },
    CancelLogin {
        request_id: String,
    },
    DrainComponent {
        request_id: String,
        component: ComponentId,
    },
    RefreshInventory {
        request_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum MachineEvent {
    Inventory {
        components: Vec<ComponentInventory>,
        observed_at_ms: i64,
    },
    CommandResult {
        request_id: String,
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    LoginChallenge {
        request_id: String,
        provider: String,
        verification_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_code: Option<String>,
        expires_at_ms: i64,
    },
    LoginState {
        request_id: String,
        provider: String,
        state: AuthState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MachineFrame {
    Challenge {
        challenge_id: String,
        nonce: String,
        expires_at_ms: i64,
    },
    Hello {
        hello: MachineHello,
    },
    Welcome {
        protocol: u16,
        controller_epoch: u64,
        heartbeat_interval_ms: u64,
        #[serde(default)]
        desired_components: Vec<DesiredComponent>,
    },
    Reject {
        reason: String,
    },
    Command {
        command: MachineCommand,
    },
    Event {
        event: MachineEvent,
    },
    Runtime {
        frame: crate::runtime_wire::Frame,
    },
    Heartbeat {
        sent_at_ms: i64,
    },
}

#[must_use]
pub fn negotiate(local_min: u16, local_max: u16, peer_min: u16, peer_max: u16) -> Option<u16> {
    let low = local_min.max(peer_min);
    let high = local_max.min(peer_max);
    (low <= high).then_some(high)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_protocol_negotiates_overlap_only() {
        assert_eq!(negotiate(1, 2, 2, 3), Some(2));
        assert_eq!(negotiate(1, 1, 2, 2), None);
    }

    #[test]
    fn hello_contract_omits_remote_auth_for_local_uds() {
        let frame = MachineFrame::Hello {
            hello: MachineHello {
                machine_id: "local".to_owned(),
                display_name: "Hawk".to_owned(),
                platform: Platform::Linux,
                arch: "x86_64".to_owned(),
                connection_mode: ConnectionMode::LocalUds,
                min_protocol: 1,
                max_protocol: 1,
                min_runtime_protocol: 1,
                max_runtime_protocol: 1,
                host_build: "test".to_owned(),
                challenge_id: None,
                challenge_signature: None,
                components: Vec::new(),
            },
        };
        let value = serde_json::to_value(frame).expect("serialize machine hello");
        assert_eq!(value["type"], "hello");
        assert_eq!(value["hello"]["connection_mode"], "local_uds");
        assert!(value["hello"].get("challenge_id").is_none());
    }

    #[test]
    fn zed_server_slots_allow_exact_versions_to_coexist() {
        let inventory = ["1.13.0", "1.14.0"]
            .into_iter()
            .map(|version| ComponentInventory {
                id: ComponentId {
                    kind: ComponentKind::ZedServer,
                    slot: version.to_owned(),
                },
                state: ComponentState::Active,
                version: version.to_owned(),
                generation: format!("zed-{version}"),
                digest: format!("sha256:{version}"),
                rollback_generation: None,
                active_leases: 1,
                auth: None,
                detail: None,
            })
            .collect::<Vec<_>>();
        let json = serde_json::to_value(inventory).expect("serialize inventory");
        assert_eq!(json[0]["id"]["slot"], "1.13.0");
        assert_eq!(json[1]["id"]["slot"], "1.14.0");
    }

    #[test]
    #[cfg(feature = "full")]
    fn old_session_metadata_defaults_to_the_local_machine() {
        let meta: crate::core::SessionMeta = serde_json::from_value(serde_json::json!({
            "id": "sess-old",
            "provider": "codex",
            "cwd": "/tmp",
            "title": "old",
            "status": "running"
        }))
        .expect("old metadata remains readable");
        assert_eq!(meta.machine_id, "local");
    }
}
