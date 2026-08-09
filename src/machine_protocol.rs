//! Stable control contract between Cowboy and a `cowboy-machine` host.
//!
//! ACP worker traffic remains defined by [`crate::runtime_wire`]. Zed's raw
//! protocol remains isolated in the GPL adapter. This module owns only machine
//! identity, inventory, component lifecycle, and multiplexing.

#![warn(clippy::pedantic)]

use serde::{Deserialize, Serialize};

pub const MACHINE_PROTOCOL_VERSION: u16 = 2;
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
    /// Immutable host-configuration revision that produced `workspaces`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_revision: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_method: Option<String>,
    },
    CancelLogin {
        request_id: String,
    },
    SubmitLoginCode {
        request_id: String,
        code: String,
    },
    UpdateNpmComponent {
        request_id: String,
        component: ComponentId,
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
    ProviderUsageAck {
        producer_id: String,
        sequence: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsageEvent {
    #[serde(default = "default_provider_usage_schema_version")]
    pub schema_version: u16,
    pub producer_id: String,
    pub sequence: u64,
    pub occurred_at_ms: i64,
    pub account_fingerprint: String,
    pub provider: String,
    pub agent: String,
    pub model: String,
    #[serde(default = "unknown_provider_usage_dimension")]
    pub model_family: String,
    #[serde(default)]
    pub resolved_model: Option<String>,
    #[serde(default)]
    pub model_revision: Option<String>,
    #[serde(default = "unknown_provider_usage_dimension")]
    pub request_role: String,
    pub status: u16,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cache_hit_tokens: Option<u64>,
    pub cache_miss_tokens: Option<u64>,
    #[serde(default = "legacy_provider_usage_dimension")]
    pub operation: String,
    #[serde(default = "legacy_provider_usage_dimension")]
    pub protocol: String,
    #[serde(default = "legacy_provider_usage_dimension")]
    pub client_protocol: String,
    #[serde(default = "legacy_provider_usage_dimension")]
    pub upstream_protocol: String,
    #[serde(default = "legacy_provider_usage_dimension")]
    pub translation_mode: String,
    #[serde(default = "unknown_provider_usage_dimension")]
    pub thinking_mode: String,
    #[serde(default = "unknown_provider_usage_dimension")]
    pub reasoning_effort: String,
    #[serde(default)]
    pub session_fingerprint: Option<String>,
    #[serde(default = "unattributed_provider_usage_dimension")]
    pub session_attribution: String,
    #[serde(default = "unattributed_provider_usage_dimension")]
    pub traffic_source: String,
    #[serde(default)]
    pub static_prefix_fingerprint: Option<String>,
    #[serde(default)]
    pub request_prefix_fingerprint: Option<String>,
    #[serde(default)]
    pub gateway_build: Option<String>,
    #[serde(default)]
    pub gateway_boot_id: Option<String>,
    #[serde(default = "legacy_provider_usage_dimension")]
    pub cache_observation: String,
    #[serde(default)]
    pub usage_observed: Option<bool>,
    #[serde(default)]
    pub completed: Option<bool>,
    #[serde(default)]
    pub streaming: Option<bool>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub request_bytes: Option<u64>,
    #[serde(default)]
    pub input_item_count: Option<u64>,
    #[serde(default)]
    pub tool_count: Option<u64>,
    #[serde(default)]
    pub system_block_count: Option<u64>,
    #[serde(default)]
    pub has_previous_response_id: Option<bool>,
    #[serde(default)]
    pub compatibility_fixes: Option<u64>,
    #[serde(default = "interactive_provider_usage_dimension")]
    pub request_purpose: String,
    #[serde(default = "not_applicable_provider_usage_dimension")]
    pub cache_keepalive_outcome: String,
    #[serde(default)]
    pub cache_keepalive_algorithm: Option<String>,
    #[serde(default)]
    pub cache_keepalive_attempt: Option<u64>,
    #[serde(default)]
    pub cache_keepalive_interval_ms: Option<u64>,
    #[serde(default)]
    pub cache_keepalive_source_age_ms: Option<u64>,
    #[serde(default)]
    pub source_request_prefix_fingerprint: Option<String>,
}

// These bounds are deliberately above every supported model/request limit.
// They keep a compromised same-user producer from poisoning long-lived SQL
// aggregates while preserving headroom for future DeepSeek models.
pub(crate) const PROVIDER_USAGE_MAX_TOKENS: u64 = 10_000_000;
pub(crate) const PROVIDER_USAGE_MAX_DURATION_MS: u64 = 86_400_000;
pub(crate) const PROVIDER_USAGE_MAX_REQUEST_BYTES: u64 = 64 << 20;
pub(crate) const PROVIDER_USAGE_MAX_SHAPE_COUNT: u64 = 1_000_000;
pub(crate) const PROVIDER_USAGE_MAX_KEEPALIVE_MS: u64 = 604_800_000;

const fn default_provider_usage_schema_version() -> u16 {
    1
}

fn legacy_provider_usage_dimension() -> String {
    "legacy".to_owned()
}

fn unknown_provider_usage_dimension() -> String {
    "unknown".to_owned()
}

fn unattributed_provider_usage_dimension() -> String {
    "unattributed".to_owned()
}

fn interactive_provider_usage_dimension() -> String {
    "interactive".to_owned()
}

fn not_applicable_provider_usage_dimension() -> String {
    "not_applicable".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum MachineEvent {
    Inventory {
        components: Vec<ComponentInventory>,
        /// Present only when the Machine has reloaded its workspace contract.
        /// Older Machines omit both workspace fields, so component-only
        /// refreshes must preserve the controller's current workspace set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspaces: Option<Vec<MachineWorkspace>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_revision: Option<String>,
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
        #[serde(default)]
        input_required: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_label: Option<String>,
        #[serde(default)]
        secret_input: bool,
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
    ProviderUsageBatch {
        producer_id: String,
        first_sequence: u64,
        last_sequence: u64,
        events: Vec<ProviderUsageEvent>,
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
    fn old_inventory_event_preserves_the_existing_workspace_contract() {
        let event: MachineEvent = serde_json::from_value(serde_json::json!({
            "event": "inventory",
            "components": [],
            "observed_at_ms": 1
        }))
        .unwrap();
        assert!(matches!(
            event,
            MachineEvent::Inventory {
                workspaces: None,
                workspace_revision: None,
                ..
            }
        ));
    }

    #[test]
    fn old_login_challenge_defaults_to_browser_only_completion() {
        let event: MachineEvent = serde_json::from_value(serde_json::json!({
            "event": "login_challenge",
            "request_id": "login-1",
            "provider": "codex",
            "verification_url": "https://example.invalid/device",
            "user_code": "ABCD-EFGH",
            "expires_at_ms": 1
        }))
        .unwrap();
        assert!(matches!(
            event,
            MachineEvent::LoginChallenge {
                input_required: false,
                ..
            }
        ));
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
            workspace_revision: None,
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
                workspace_revision: None,
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
