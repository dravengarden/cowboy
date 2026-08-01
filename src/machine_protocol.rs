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
    CodeAdapter,
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
    /// Provider id or Zed compatibility key (normally the exact client/server
    /// version). Empty only for
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
    /// Latest release observed from the component's authoritative update
    /// channel. Absent means the Machine could not establish a trustworthy
    /// comparison; it must not be interpreted as "up to date".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<ComponentUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentUpdate {
    pub latest_version: String,
    pub available: bool,
    pub source: String,
    pub checked_at_ms: i64,
    /// True only when Cowboy has a signed artifact it can reconcile. Release
    /// discovery alone never grants the browser authority to mutate a host.
    #[serde(default)]
    pub installable: bool,
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
    /// Explicit launch roots exported by the Machine. Cowboy never sends an
    /// arbitrary controller-side path to a remote host.
    #[serde(default)]
    pub workspaces: Vec<MachineWorkspace>,
    /// Scheduling envelope declared by the stable host. Active usage is
    /// controller-derived so a reconnect cannot under-report existing
    /// session leases.
    #[serde(default)]
    pub capacity: MachineCapacity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineCapacity {
    #[serde(default = "default_max_sessions")]
    pub max_sessions: u32,
    #[serde(default)]
    pub draining: bool,
}

impl Default for MachineCapacity {
    fn default() -> Self {
        Self {
            max_sessions: default_max_sessions(),
            draining: false,
        }
    }
}

fn default_max_sessions() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineWorkspace {
    pub id: String,
    pub display_name: String,
    pub canonical_path: String,
}

/// Build the exact version-one proof signed during a remote Machine handshake.
/// Length-prefixed fields avoid delimiter ambiguity while keeping the proof
/// independent from later additions to the wire `Hello` object.
#[must_use]
pub fn challenge_proof_v1(
    challenge_id: &str,
    nonce: &str,
    expires_at_ms: i64,
    hello: &MachineHello,
) -> Vec<u8> {
    let platform = match hello.platform {
        Platform::Linux => "linux",
        Platform::Macos => "macos",
    };
    let connection_mode = match hello.connection_mode {
        ConnectionMode::LocalUds => "local_uds",
        ConnectionMode::OutboundTls => "outbound_tls",
    };
    let fields = [
        challenge_id.to_owned(),
        nonce.to_owned(),
        expires_at_ms.to_string(),
        hello.machine_id.clone(),
        platform.to_owned(),
        hello.arch.clone(),
        connection_mode.to_owned(),
        hello.min_protocol.to_string(),
        hello.max_protocol.to_string(),
        hello.min_runtime_protocol.to_string(),
        hello.max_runtime_protocol.to_string(),
        hello.host_build.clone(),
    ];
    let mut proof = b"cowboy-machine-proof-v1\n".to_vec();
    for field in fields {
        proof.extend_from_slice(field.len().to_string().as_bytes());
        proof.push(b':');
        proof.extend_from_slice(field.as_bytes());
        proof.push(b'\n');
    }
    proof
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredComponent {
    pub id: ComponentId,
    pub version: String,
    pub generation: String,
    pub artifact_url: String,
    pub digest: String,
    #[serde(default)]
    pub artifact_format: ArtifactFormat,
    /// Relative executable path within an archive. Raw artifacts always use
    /// `bin` and leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Optional executable readiness check run against the staged generation
    /// before any active symlink changes. Automatic activation requires one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<ComponentProbe>,
    #[serde(default)]
    pub automatic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentProbe {
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_probe_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_probe_timeout_ms() -> u64 {
    10_000
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    #[default]
    Raw,
    TarGz,
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
    RefreshInventory {
        request_id: String,
    },
    /// Forward one stable Cowboy product request to a versioned adapter on
    /// the Machine. Raw Zed protobuf never crosses this boundary.
    AdapterRequest {
        request_id: String,
        adapter: String,
        payload: serde_json::Value,
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
    AdapterResponse {
        request_id: String,
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payload: Option<serde_json::Value>,
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
    fn old_raw_component_manifest_keeps_its_additive_defaults() {
        let component: DesiredComponent = serde_json::from_value(serde_json::json!({
            "id": { "kind": "provider_cli", "slot": "codex" },
            "version": "1",
            "generation": "1",
            "artifact_url": "https://example.invalid/codex",
            "digest": "00",
            "automatic": false
        }))
        .unwrap();
        assert_eq!(component.artifact_format, ArtifactFormat::Raw);
        assert_eq!(component.entrypoint, None);
        assert_eq!(component.probe, None);
    }

    #[test]
    fn old_component_inventory_does_not_claim_release_freshness() {
        let component: ComponentInventory = serde_json::from_value(serde_json::json!({
            "id": { "kind": "provider_cli", "slot": "codex" },
            "state": "active",
            "version": "0.145.0",
            "generation": "",
            "digest": "",
            "active_leases": 0
        }))
        .unwrap();
        assert_eq!(component.update, None);
    }

    #[test]
    fn old_machine_hello_defaults_to_a_safe_single_session_capacity() {
        let hello: MachineHello = serde_json::from_value(serde_json::json!({
            "machine_id": "old",
            "display_name": "Old Machine",
            "platform": "linux",
            "arch": "x86_64",
            "connection_mode": "outbound_tls",
            "min_protocol": 1,
            "max_protocol": 1,
            "min_runtime_protocol": 1,
            "max_runtime_protocol": 1,
            "host_build": "old"
        }))
        .unwrap();
        assert_eq!(hello.capacity.max_sessions, 1);
        assert!(!hello.capacity.draining);
    }

    #[test]
    fn challenge_proof_binds_machine_and_protocol_without_display_metadata() {
        let hello = MachineHello {
            machine_id: "falcon".to_owned(),
            display_name: "Falcon".to_owned(),
            platform: Platform::Linux,
            arch: "x86_64".to_owned(),
            connection_mode: ConnectionMode::OutboundTls,
            min_protocol: 1,
            max_protocol: 1,
            min_runtime_protocol: 1,
            max_runtime_protocol: 2,
            host_build: "build-a".to_owned(),
            challenge_id: None,
            challenge_signature: None,
            components: Vec::new(),
            workspaces: Vec::new(),
            capacity: MachineCapacity::default(),
        };
        let proof = challenge_proof_v1("id", "nonce", 42, &hello);
        let mut renamed = hello.clone();
        renamed.display_name = "Renamed Falcon".to_owned();
        assert_eq!(proof, challenge_proof_v1("id", "nonce", 42, &renamed));
        let mut other_machine = hello.clone();
        other_machine.machine_id = "macbook-air".to_owned();
        assert_ne!(proof, challenge_proof_v1("id", "nonce", 42, &other_machine));
        let mut other_protocol = hello;
        other_protocol.max_runtime_protocol = 3;
        assert_ne!(
            proof,
            challenge_proof_v1("id", "nonce", 42, &other_protocol)
        );
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
                workspaces: Vec::new(),
                capacity: MachineCapacity::default(),
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
                update: None,
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
