//! Stable control contract between Cowboy and a `cowboy-machine` host.
//!
//! ACP worker traffic remains defined by [`crate::runtime_wire`]. Zed's raw
//! protocol remains isolated in the GPL adapter. This module owns only machine
//! identity, inventory, component lifecycle, and multiplexing.

#![warn(clippy::pedantic)]

use serde::{Deserialize, Serialize};

pub const MACHINE_PROTOCOL_VERSION: u16 = 5;
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
    /// X25519 public key used only to seal Service-scoped Provider credential
    /// replicas to this Machine. It is enrolled and challenge-bound separately
    /// from the Ed25519 signing identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_public_key: Option<String>,
    #[serde(default)]
    pub components: Vec<ComponentInventory>,
    /// Product-level Plugin installations. Internal components remain a
    /// developer diagnostic and never define ordinary scheduling.
    #[serde(default, alias = "providers")]
    pub plugins: Vec<PluginInventory>,
    /// Signed capability inventory used by the Controller to select only
    /// Agent Plugin bindings this exact Machine can decode and activate. An absent
    /// inventory is a legacy Machine and fails closed for install/upgrade.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_provider_contracts"
    )]
    pub provider_contracts: Option<cowboy_provider_sdk::ProviderContractInventory>,
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

fn deserialize_provider_contracts<'de, D>(
    deserializer: D,
) -> Result<Option<cowboy_provider_sdk::ProviderContractInventory>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Some(mut value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    if let Some(object) = value.as_object_mut() {
        for (legacy, current) in [
            ("min_release_schema", "min_runtime_binding_schema"),
            ("max_release_schema", "max_runtime_binding_schema"),
        ] {
            if let Some(legacy_value) = object.remove(legacy) {
                if object.contains_key(current) {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate Provider contract field {current}"
                    )));
                }
                object.insert(current.to_owned(), legacy_value);
            }
        }
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(serde::de::Error::custom)
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

/// Browser-facing projection of one enrolled Machine. The HTTP registry route
/// and the product WebSocket share this exact type so periodic browser polling
/// cannot drift from the pushed control-plane state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSummary {
    pub id: String,
    pub display_name: String,
    pub platform: String,
    pub architecture: String,
    pub status: String,
    pub local: bool,
    pub connected: bool,
    pub schedulable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<MachineWorkspace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_revision: Option<String>,
    #[serde(default)]
    pub components: Vec<ComponentInventory>,
    #[serde(default)]
    pub plugins: Vec<PluginInventory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_contracts: Option<cowboy_provider_sdk::ProviderContractInventory>,
    #[serde(default)]
    pub capacity: MachineCapacity,
    #[serde(default)]
    pub active_sessions: u32,
    #[serde(default)]
    pub pending_updates: Vec<ComponentId>,
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

/// Version-two handshake proof additionally binds the credential-replica key.
/// A protocol-three Machine must use this proof so an enrolled signing key
/// cannot advertise a substituted X25519 recipient.
#[must_use]
pub fn challenge_proof_v2(
    challenge_id: &str,
    nonce: &str,
    expires_at_ms: i64,
    hello: &MachineHello,
) -> Vec<u8> {
    let mut proof = challenge_proof_v1(challenge_id, nonce, expires_at_ms, hello);
    proof.extend_from_slice(b"cowboy-machine-proof-v2\n");
    let encryption_key = hello.encryption_public_key.as_deref().unwrap_or_default();
    proof.extend_from_slice(encryption_key.len().to_string().as_bytes());
    proof.push(b':');
    proof.extend_from_slice(encryption_key.as_bytes());
    proof.push(b'\n');
    if let Some(provider_contracts) = &hello.provider_contracts {
        proof.extend_from_slice(b"provider-contracts:");
        proof.extend_from_slice(
            provider_contracts
                .provider_sdk_version
                .len()
                .to_string()
                .as_bytes(),
        );
        proof.push(b':');
        proof.extend_from_slice(provider_contracts.provider_sdk_version.as_bytes());
        for value in [
            provider_contracts.min_package_schema,
            provider_contracts.max_package_schema,
            provider_contracts.min_runtime_binding_schema,
            provider_contracts.max_runtime_binding_schema,
            provider_contracts.min_ui_schema,
            provider_contracts.max_ui_schema,
            provider_contracts.min_host_schema,
            provider_contracts.max_host_schema,
            provider_contracts.machine_contract,
        ] {
            proof.push(b':');
            proof.extend_from_slice(value.to_string().as_bytes());
        }
        proof.push(b'\n');
    }
    proof
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginInstallationState {
    Missing,
    Installing,
    Active,
    Uninstalling,
    Incompatible,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReplicaState {
    Absent,
    Pending,
    Storing,
    Current,
    Failed,
    Revoking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMaterializationState {
    NotInstalled,
    Applying,
    Current,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInventory {
    #[serde(alias = "provider_id")]
    pub plugin_id: String,
    #[serde(alias = "provider_version")]
    pub plugin_version: String,
    #[serde(default = "default_agent_plugin_kind")]
    pub plugin_kind: cowboy_plugin_sdk::PluginKind,
    pub generation_digest: String,
    pub contract_fingerprint: String,
    pub state: PluginInstallationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_generation_digest: Option<String>,
    #[serde(default)]
    pub active_session_leases: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_generation: Option<u64>,
    #[serde(default = "default_provider_replica_state")]
    pub replica_state: ProviderReplicaState,
    #[serde(default = "default_provider_materialization_state")]
    pub materialization_state: ProviderMaterializationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

const fn default_agent_plugin_kind() -> cowboy_plugin_sdk::PluginKind {
    cowboy_plugin_sdk::PluginKind::AgentProvider
}

const fn default_provider_replica_state() -> ProviderReplicaState {
    ProviderReplicaState::Absent
}

const fn default_provider_materialization_state() -> ProviderMaterializationState {
    ProviderMaterializationState::NotInstalled
}

/// Immutable Catalog-selected payload. The browser submits only Plugin id,
/// version, and optional digest; the Controller fills this complete envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredPlugin {
    pub release: cowboy_plugin_sdk::PluginRelease,
    pub package_base64: String,
    pub publisher_public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthAction {
    Apply,
    Wipe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableCredentialBundle {
    pub portable_schema: String,
    pub method_id: String,
    /// Opaque values encoded with standard base64. Bundle keys are declared by
    /// the Provider authentication contract and never appear in logs.
    pub values: std::collections::BTreeMap<String, String>,
}

/// Service-auth bundle encrypted to one enrolled Machine. The signature binds
/// every routing and cryptographic field; plaintext never enters this contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedProviderAuth {
    pub envelope_schema: u16,
    pub provider_id: String,
    pub auth_generation: u64,
    pub auth_contract_fingerprint: String,
    pub projection_schema: String,
    pub action: ProviderAuthAction,
    pub ephemeral_public_key: String,
    pub nonce: String,
    pub ciphertext: String,
    pub service_public_key: String,
    pub signature: String,
}

impl SealedProviderAuth {
    #[must_use]
    pub fn proof(&self) -> Vec<u8> {
        let action = match self.action {
            ProviderAuthAction::Apply => "apply",
            ProviderAuthAction::Wipe => "wipe",
        };
        let fields = [
            self.envelope_schema.to_string(),
            self.provider_id.clone(),
            self.auth_generation.to_string(),
            self.auth_contract_fingerprint.clone(),
            self.projection_schema.clone(),
            action.to_owned(),
            self.ephemeral_public_key.clone(),
            self.nonce.clone(),
            self.ciphertext.clone(),
        ];
        let mut proof = b"cowboy-provider-auth-envelope-v1\n".to_vec();
        for field in fields {
            proof.extend_from_slice(field.len().to_string().as_bytes());
            proof.push(b':');
            proof.extend_from_slice(field.as_bytes());
            proof.push(b'\n');
        }
        proof
    }
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
    InstallPlugin {
        request_id: String,
        plugin: Box<DesiredPlugin>,
    },
    UninstallPlugin {
        request_id: String,
        plugin_id: String,
        generation_digest: String,
    },
    /// Compensate a Controller uninstall saga whose durable session commit
    /// failed after the Machine removed its active link. Only retained,
    /// previously verified generation bytes may be re-activated.
    ReactivatePlugin {
        request_id: String,
        plugin_id: String,
        generation_digest: String,
    },
    ApplyProviderAuth {
        request_id: String,
        envelope: Box<SealedProviderAuth>,
    },
    FinalizeProviderAuthCandidate {
        request_id: String,
        provider_id: String,
        auth_method: String,
        candidate_request_id: String,
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

impl MachineCommand {
    /// Old Machines may still negotiate an earlier wire version. Keep the
    /// capability check beside the tagged command so the Controller never
    /// sends a Service-auth or Provider-lifecycle message an older peer could
    /// deserialize incorrectly.
    #[must_use]
    pub const fn minimum_protocol(&self) -> u16 {
        match self {
            // Uninstall is destructive and its Controller saga relies on
            // exact-generation reactivation for compensation. Never let a
            // protocol-three Machine begin removal that it cannot undo.
            Self::InstallPlugin { .. }
            | Self::UninstallPlugin { .. }
            | Self::ReactivatePlugin { .. } => 5,
            Self::BeginLogin { .. }
            | Self::CancelLogin { .. }
            | Self::SubmitLoginCode { .. }
            | Self::ApplyProviderAuth { .. }
            | Self::FinalizeProviderAuthCandidate { .. } => 3,
            Self::Reconcile { .. }
            | Self::UpdateNpmComponent { .. }
            | Self::RefreshInventory { .. }
            | Self::AdapterRequest { .. }
            | Self::ProviderUsageAck { .. } => 1,
        }
    }
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
    #[serde(alias = "provider_inventory")]
    PluginInventory {
        #[serde(alias = "providers")]
        plugins: Vec<PluginInventory>,
        observed_at_ms: i64,
    },
    ProviderAuthReceipt {
        request_id: String,
        provider_id: String,
        auth_generation: u64,
        replica_state: ProviderReplicaState,
        materialization_state: ProviderMaterializationState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// A temporary login executor returns a portable typed bundle to the
    /// Service. Values are base64 so JSON never guesses text/binary encoding.
    ServiceAuthCandidate {
        request_id: String,
        provider_id: String,
        auth_method: String,
        provider_version: String,
        generation_digest: String,
        auth_contract_fingerprint: String,
        portable_schema: String,
        bundle: std::collections::BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_label: Option<String>,
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
        #[serde(default = "default_machine_proof_version")]
        proof_version: u16,
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

const fn default_machine_proof_version() -> u16 {
    1
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
    fn old_provider_inventory_event_maps_to_plugin_inventory() {
        let event: MachineEvent = serde_json::from_value(serde_json::json!({
            "event": "provider_inventory",
            "providers": [{
                "provider_id": "codex",
                "provider_version": "1.1.2",
                "generation_digest": "sha256:0123456789abcdef",
                "contract_fingerprint": "legacy-contract",
                "state": "active",
                "active_session_leases": 2,
                "replica_state": "absent",
                "materialization_state": "not_installed"
            }],
            "observed_at_ms": 1
        }))
        .unwrap();
        assert!(matches!(
            event,
            MachineEvent::PluginInventory { plugins, .. }
                if plugins.len() == 1
                    && plugins[0].plugin_id == "codex"
                    && plugins[0].plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
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
        assert!(hello.provider_contracts.is_none());
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
            encryption_public_key: None,
            components: Vec::new(),
            plugins: Vec::new(),
            provider_contracts: None,
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
    fn challenge_proof_v2_binds_provider_contract_inventory_when_present() {
        let mut hello: MachineHello = serde_json::from_value(serde_json::json!({
            "machine_id": "falcon",
            "display_name": "Falcon",
            "platform": "linux",
            "arch": "x86_64",
            "connection_mode": "outbound_tls",
            "min_protocol": 3,
            "max_protocol": 4,
            "min_runtime_protocol": 1,
            "max_runtime_protocol": 3,
            "host_build": "test",
            "encryption_public_key": "machine-key"
        }))
        .unwrap();
        hello.provider_contracts =
            Some(cowboy_provider_sdk::ProviderContractInventory::current_machine());
        let proof = challenge_proof_v2("id", "nonce", 42, &hello);

        let mut downgraded = hello.clone();
        downgraded
            .provider_contracts
            .as_mut()
            .unwrap()
            .max_host_schema = 1;
        assert_ne!(proof, challenge_proof_v2("id", "nonce", 42, &downgraded));

        let mut omitted = hello;
        omitted.provider_contracts = None;
        assert_ne!(proof, challenge_proof_v2("id", "nonce", 42, &omitted));
    }

    #[test]
    fn machine_hello_accepts_legacy_provider_contract_inventory() {
        let hello: MachineHello = serde_json::from_value(serde_json::json!({
            "machine_id": "hawk",
            "display_name": "Hawk",
            "platform": "linux",
            "arch": "x86_64",
            "connection_mode": "local_uds",
            "min_protocol": 3,
            "max_protocol": 4,
            "min_runtime_protocol": 1,
            "max_runtime_protocol": 3,
            "host_build": "legacy-machine",
            "provider_contracts": {
                "provider_sdk_version": "3.1.0",
                "min_package_schema": 2,
                "max_package_schema": 2,
                "min_release_schema": 2,
                "max_release_schema": 2,
                "min_ui_schema": 1,
                "max_ui_schema": 2,
                "min_host_schema": 1,
                "max_host_schema": 2,
                "machine_contract": 4
            }
        }))
        .unwrap();
        let contracts = hello.provider_contracts.clone().unwrap();
        assert_eq!(contracts.min_runtime_binding_schema, 2);
        assert_eq!(contracts.max_runtime_binding_schema, 2);

        let legacy_proof = challenge_proof_v2("id", "nonce", 42, &hello);
        let mut current = hello;
        current.provider_contracts = Some(contracts);
        assert_eq!(
            legacy_proof,
            challenge_proof_v2("id", "nonce", 42, &current)
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
                encryption_public_key: None,
                components: Vec::new(),
                plugins: Vec::new(),
                provider_contracts: None,
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
