//! Stable, data-only authoring contract for independently built Cowboy Providers.
//!
//! Provider source may use these Rust types directly or generate equivalent
//! JSON from another language. Cowboy validates the canonical artifact again at
//! Catalog ingestion and on the target Machine; compile-time types are never
//! treated as a trust boundary.

#![warn(clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const PACKAGE_SCHEMA_VERSION: u16 = 2;
pub const PROVIDER_SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const UI_SCHEMA_VERSION: u16 = 1;
pub const LOGIC_SCHEMA_VERSION: u16 = 1;
pub const AUTH_SCHEMA_VERSION: u16 = 1;
pub const CONTROLLER_CONTRACT_VERSION: u16 = 2;
pub const MACHINE_CONTRACT_VERSION: u16 = 4;
pub const RELEASE_SCHEMA_VERSION: u16 = 2;
pub const PROVIDER_RELEASE_SIGNATURE_NAMESPACE: &str = "cowboy-provider-release-v1";

const REQUIRED_SURFACES: [SurfaceSlot; 8] = [
    SurfaceSlot::Card,
    SurfaceSlot::Setup,
    SurfaceSlot::Settings,
    SurfaceSlot::Information,
    SurfaceSlot::Empty,
    SurfaceSlot::Loading,
    SurfaceSlot::Error,
    SurfaceSlot::Session,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPackage {
    pub package_schema: u16,
    pub manifest: ProviderManifest,
    pub contract_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderManifest {
    pub id: String,
    pub version: String,
    pub publisher: String,
    pub sdk_version: String,
    pub display: ProviderDisplay,
    pub ui: UiContract,
    pub logic: LogicContract,
    pub configuration: ConfigurationUiContract,
    pub host: HostIntegrationContract,
    pub runtime: RuntimeContract,
    pub authentication: AuthenticationContract,
    pub compatibility: CompatibilityClaim,
}

/// Public, data-only projection consumed by Cowboy UI. Runtime transports,
/// dependency pins, credential paths, and authentication executors stay inside
/// the signed Provider package and never cross the ordinary UI API boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUiManifest {
    pub id: String,
    pub version: String,
    pub publisher: String,
    pub sdk_version: String,
    pub display: ProviderDisplay,
    pub ui: UiContract,
    pub logic: LogicContract,
    pub configuration: ConfigurationUiContract,
    pub host: HostIntegrationContract,
    pub authentication: ProviderUiAuthenticationContract,
    pub compatibility: CompatibilityClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUiAuthenticationContract {
    pub schema_version: u16,
    pub required: bool,
    pub methods: Vec<ProviderUiAuthMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUiAuthMethod {
    pub id: String,
    pub label: String,
    pub flow: AuthFlow,
}

/// Concise typed authoring profile for ordinary coding-agent Providers. It
/// compiles to the same complete UI IR as a hand-authored [`ProviderManifest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandardProviderSource {
    pub authoring_schema: u16,
    pub id: String,
    pub version: String,
    pub publisher: String,
    pub sdk_version: String,
    pub display: StandardProviderDisplay,
    pub card_layout: StandardCardLayout,
    #[serde(default)]
    pub configuration_presets: Vec<ConfigurationPreset>,
    #[serde(default)]
    pub configuration_options: Vec<ConfigurationOptionPresentation>,
    pub host: HostIntegrationContract,
    pub runtime: RuntimeContract,
    pub authentication: AuthenticationContract,
    pub compatibility: CompatibilityClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandardProviderDisplay {
    pub name: String,
    pub vendor: String,
    pub summary: String,
    pub accent: String,
    pub secondary_accent: String,
    pub mark_view_box: String,
    pub mark_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark_fill: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardCardLayout {
    MarkLeading,
    MarkAbove,
    SplitStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDisplay {
    pub name: String,
    pub vendor: String,
    pub summary: String,
    pub accent: String,
    pub secondary_accent: String,
    pub logo_asset: String,
    pub icon_asset: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationUiContract {
    pub schema_version: u16,
    #[serde(default)]
    pub presets: Vec<ConfigurationPreset>,
    #[serde(default)]
    pub options: Vec<ConfigurationOptionPresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationPreset {
    pub id: String,
    pub name: String,
    pub detail: String,
    #[serde(default)]
    pub is_default: bool,
    pub values: BTreeMap<String, String>,
}

/// Provider-owned presentation and lifecycle policy for a runtime-advertised
/// configuration option. Cowboy never branches on the option id itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationOptionPresentation {
    pub id: String,
    pub order: u16,
    #[serde(default)]
    pub layout: ConfigurationOptionLayout,
    #[serde(default)]
    pub availability: ConfigurationOptionAvailability,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationOptionLayout {
    #[default]
    Standard,
    FullWidth,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationOptionAvailability {
    /// Mutable while the session is live, including while it is starting or busy.
    #[default]
    LiveSession,
    /// Mutable only while the session is idle or stopped; the next start applies it.
    IdleOrStopped,
}

/// Safe host capabilities consumed by Cowboy's shell. These declarations are
/// identity-free: a new Provider composes existing contracts without adding a
/// Provider id branch to Web, Controller, or Machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostIntegrationContract {
    pub schema_version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_compaction: Option<CommandDiscoveryContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_usage: Option<AccountUsageContract>,
    #[serde(default)]
    pub features: BTreeSet<HostFeature>,
    #[serde(default)]
    pub tool_presentations: Vec<ToolPresentationContract>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDiscoveryContract {
    pub aliases: BTreeSet<String>,
    pub fallback_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountUsageContract {
    pub provider: AccountUsageProvider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageProvider {
    Openai,
    Anthropic,
    Deepseek,
    Gemini,
    Xai,
}

impl AccountUsageProvider {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Deepseek => "deepseek",
            Self::Gemini => "gemini",
            Self::Xai => "xai",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostFeature {
    CacheProtectionV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPresentationContract {
    pub tool_name: String,
    pub renderer: ToolRenderer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRenderer {
    TodoListV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiContract {
    pub schema_version: u16,
    pub assets: Vec<UiAsset>,
    pub surfaces: BTreeMap<SurfaceSlot, UiNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceSlot {
    Card,
    Setup,
    Settings,
    Information,
    Empty,
    Loading,
    Error,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiAsset {
    pub id: String,
    pub role: AssetRole,
    pub media_type: String,
    pub digest: String,
    pub accessible_label: String,
    pub content: AssetContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetRole {
    Logo,
    Icon,
    Loading,
    Illustration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssetContent {
    VectorPath {
        view_box: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<String>,
    },
    Inline {
        base64: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "component", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiNode {
    Stack {
        direction: StackDirection,
        gap: SpacingToken,
        #[serde(default)]
        children: Vec<Self>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visible_when: Option<BoolExpression>,
    },
    Text {
        variant: TextVariant,
        value: TextValue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
    },
    Asset {
        asset: String,
        size: AssetSize,
    },
    Badge {
        label: TextValue,
        tone: Tone,
    },
    Progress {
        label: TextValue,
    },
    Alert {
        tone: Tone,
        title: TextValue,
        body: TextValue,
    },
    Divider,
    Button {
        label: TextValue,
        style: ButtonStyle,
        emit: MessageEmission,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled_when: Option<BoolExpression>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StackDirection {
    Row,
    Column,
    Responsive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpacingToken {
    Xs,
    Sm,
    Md,
    Lg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextVariant {
    Title,
    Body,
    Caption,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Neutral,
    Primary,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetSize {
    Sm,
    Md,
    Lg,
    Fill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextValue {
    Literal { value: String },
    State { field: String },
    Host { field: HostTextField },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostTextField {
    ProviderVersion,
    InstallationState,
    AuthenticationState,
    DistributionState,
    MachineName,
    ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoolExpression {
    StateEquals { field: String, value: LiteralValue },
    HostEquals { field: HostBoolField, value: bool },
    All { values: Vec<Self> },
    Any { values: Vec<Self> },
    Not { value: Box<Self> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostBoolField {
    Installed,
    AuthReady,
    AuthRequired,
    MachineOnline,
    UpgradeAvailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LiteralValue {
    String(String),
    Bool(bool),
    Integer(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageEmission {
    pub message: String,
    #[serde(default)]
    pub payload: BTreeMap<String, LiteralValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicContract {
    pub schema_version: u16,
    #[serde(default)]
    pub state: Vec<StateField>,
    #[serde(default)]
    pub messages: Vec<MessageSchema>,
    #[serde(default)]
    pub reducers: Vec<ReducerRule>,
    #[serde(default)]
    pub effects: Vec<EffectSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateField {
    pub id: String,
    pub value_type: ValueType,
    pub initial: LiteralValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    String,
    Bool,
    Integer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSchema {
    pub id: String,
    #[serde(default)]
    pub payload: BTreeMap<String, ValueType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReducerRule {
    pub message: String,
    #[serde(default)]
    pub assignments: Vec<StateAssignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateAssignment {
    pub field: String,
    pub value: AssignmentValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssignmentValue {
    Literal { value: LiteralValue },
    State { field: String },
    Message { field: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectSchema {
    pub id: String,
    pub capability: EffectCapability,
    #[serde(default)]
    pub request: BTreeMap<String, ValueType>,
    pub success_message: String,
    pub failure_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectCapability {
    BeginServiceAuthentication,
    LogoutServiceAuthentication,
    InstallOnMachine,
    UpgradeOnMachine,
    RequestUninstallPlan,
    OpenExternalDocumentation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContract {
    pub driver_schema: u16,
    pub protocol: String,
    pub entrypoint: String,
    /// Closed host-integration profiles selected by the Provider. These are
    /// versioned Cowboy SDK interfaces, not Provider ids; an older host cannot
    /// silently guess how to implement an unknown profile.
    pub behavior: ProviderBehaviorContract,
    #[serde(default)]
    pub arguments: Vec<RuntimeValue>,
    /// Provider-owned, non-secret process environment. Credential values are
    /// supplied only through the separate authentication projection contract.
    #[serde(default)]
    pub environment: BTreeMap<String, RuntimeValue>,
    /// Session-owned auxiliary processes. Cowboy allocates a distinct
    /// loopback endpoint for every worker so exact Provider generations and
    /// authentication generations can drain alongside their replacements.
    #[serde(default)]
    pub sidecars: Vec<RuntimeSidecar>,
    /// Exact inherited variables and prefixes that must not cross into this
    /// Provider runtime. This makes isolation part of the signed package.
    #[serde(default)]
    pub remove_environment: BTreeSet<String>,
    #[serde(default)]
    pub remove_environment_prefixes: BTreeSet<String>,
    #[serde(default)]
    pub dependencies: Vec<ExactDependency>,
    pub platforms: Vec<PlatformPayload>,
    #[serde(default)]
    pub required_capabilities: BTreeSet<RuntimeCapability>,
}

/// A closed runtime value DSL. Plain strings remain ergonomic literals while
/// host bindings are explicitly tagged, linked, and validated at package build
/// and Machine installation time. No shell or Provider code is evaluated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RuntimeValue {
    Literal(String),
    Binding(RuntimeBinding),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeBinding {
    ComponentCommand {
        component: AuthComponent,
        #[serde(default)]
        prefix: String,
        #[serde(default)]
        suffix: String,
    },
    SidecarUrl {
        sidecar: String,
        #[serde(default)]
        prefix: String,
        #[serde(default)]
        suffix: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSidecar {
    pub id: String,
    pub component: AuthComponent,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Authentication projection variables forwarded only to this sidecar.
    /// Declaring them here prevents gateway credentials from leaking into the
    /// main ACP adapter process.
    #[serde(default)]
    pub auth_environment: BTreeSet<String>,
    pub transport: RuntimeSidecarTransport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeSidecarTransport {
    LoopbackHttpV1 {
        listen_argument: String,
        health_path: String,
        timeout_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderBehaviorContract {
    pub schema_version: u16,
    pub permission: PermissionBehavior,
    pub session: SessionBehavior,
    pub turn_end: TurnEndBehavior,
    pub configuration: ConfigurationBehavior,
    #[serde(default)]
    pub default_preferences: BTreeMap<String, LiteralValue>,
    #[serde(default)]
    pub error_rules: Vec<RuntimeErrorRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeErrorRule {
    pub when: TextMatchExpression,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(default)]
    pub keep_worker_alive: bool,
    #[serde(default)]
    pub retry_once_without_visible_update: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextMatchExpression {
    Contains { value: String },
    All { values: Vec<TextMatchExpression> },
    Any { values: Vec<TextMatchExpression> },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RuntimeCapability {
    #[serde(rename = "provider.runtime.v1")]
    ProviderRuntimeV1,
    #[serde(rename = "provider.gateway.v1")]
    ProviderGatewayV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionBehavior {
    PortableV1,
    AcpConfigFullAccessV1,
    AcpSessionModeBypassPermissionsV1,
    AcpSessionModeYoloV1,
    XaiSessionV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionBehavior {
    PortableV1,
    StablePresetSystemPromptV1,
    XaiSessionV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEndBehavior {
    PortableV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationBehavior {
    PortableV1,
    AcpConfigOptionsV1,
    XaiSessionV1,
    AnthropicGatewayV1,
    OpenaiGatewayV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactDependency {
    pub id: String,
    pub version: String,
    pub source: String,
    pub integrity: String,
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformPayload {
    pub os: OperatingSystem,
    pub architecture: Architecture,
    pub payload_digest: String,
    pub launch_command: String,
    pub private_components: Vec<PrivateComponentRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateComponentRequirement {
    pub kind: PrivateComponentKind,
    pub slot: String,
    pub dependency: String,
    /// Stable command name exported by this component inside the Provider's
    /// exact, content-addressed generation. It is never resolved from a
    /// Machine-global Provider PATH for release schema v2.
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateComponentKind {
    ProviderCli,
    ProviderAdapter,
    ProviderGateway,
    AcpRuntime,
}

impl PrivateComponentKind {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ProviderCli => "provider_cli",
            Self::ProviderAdapter => "provider_adapter",
            Self::ProviderGateway => "provider_gateway",
            Self::AcpRuntime => "acp_runtime",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingSystem {
    Linux,
    Macos,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    X86_64,
    Aarch64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticationContract {
    pub schema_version: u16,
    pub required: bool,
    pub portable_schema: String,
    pub projection_schema: String,
    pub refresh: RefreshOwnership,
    pub methods: Vec<AuthMethod>,
    #[serde(default)]
    pub credential_files: Vec<CredentialFile>,
    #[serde(default)]
    pub environment_projection: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshOwnership {
    Service,
    CompareAndSwap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthMethod {
    pub id: String,
    pub label: String,
    pub flow: AuthFlow,
    pub executor: AuthExecutor,
    pub required_bundle_keys: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthFlow {
    DeviceCode,
    BrowserCode,
    SecretInput,
    ServiceBroker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthExecutor {
    SecretInputV1 {
        bundle_key: String,
        verification_url: String,
    },
    CommandV1 {
        component: AuthComponent,
        #[serde(default)]
        arguments: Vec<String>,
        terminal: AuthTerminal,
        challenge: AuthChallenge,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        #[serde(default)]
        preflight: Vec<AuthPreflight>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthComponent {
    pub kind: PrivateComponentKind,
    pub slot: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthTerminal {
    Pipes,
    Pty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthChallenge {
    DeviceCode,
    BrowserCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthPreflight {
    JsonStringSetV1 {
        relative_path: String,
        path: Vec<String>,
        value: String,
    },
    EnvFileKeyRequiredV1 {
        relative_path: String,
        key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialFile {
    pub bundle_key: String,
    pub relative_path: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityClaim {
    pub min_controller_contract: u16,
    pub max_controller_contract: u16,
    pub min_machine_contract: u16,
    pub max_machine_contract: u16,
    pub ui_component_fingerprint: String,
    pub auth_contract_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRelease {
    pub release_schema: u16,
    pub provider_id: String,
    pub provider_version: String,
    /// Digest of the canonical data-only `.cowboy-provider` package.
    pub package_digest: String,
    /// Composite identity of the package and every platform runtime artifact.
    /// Catalog selection, Machine generations, and sessions pin this value.
    pub artifact_digest: String,
    pub artifact_url: String,
    pub publisher: String,
    pub contract_fingerprint: String,
    pub signature: String,
    pub supported_platforms: Vec<PlatformTarget>,
    /// Signed, platform-specific runtime artifacts. The Provider package owns
    /// logical components and exact dependency pins; the release binds those
    /// requirements to immutable executable bytes without exposing them as
    /// independently installable Machine products.
    pub runtime_artifacts: Vec<PlatformRuntimeArtifacts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformRuntimeArtifacts {
    pub os: OperatingSystem,
    pub architecture: Architecture,
    pub components: Vec<ReleasedPrivateComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasedPrivateComponent {
    pub kind: PrivateComponentKind,
    pub slot: String,
    pub dependency: String,
    pub version: String,
    pub command: String,
    pub artifact_url: String,
    pub artifact_digest: String,
    pub artifact_format: ProviderArtifactFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    pub probe: ProviderArtifactProbe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderArtifactFormat {
    Raw,
    TarGz,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderArtifactProbe {
    #[serde(default)]
    pub args: Vec<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformTarget {
    pub os: OperatingSystem,
    pub architecture: Architecture,
}

impl ProviderPackage {
    /// Parse and validate one canonical Provider artifact.
    ///
    /// # Errors
    /// Returns an error when the bytes are not a closed Provider package or
    /// when any linked contract, digest, or compatibility constraint fails.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let package: Self = serde_json::from_slice(bytes).context("decoding Provider package")?;
        package.validate()?;
        Ok(package)
    }

    /// Validate structural types, linked UI logic, pins, and compatibility.
    ///
    /// # Errors
    /// Returns an error for a schema, type, link, digest, pin, or host-contract
    /// mismatch.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.package_schema == PACKAGE_SCHEMA_VERSION,
            "unsupported Provider package schema {}",
            self.package_schema
        );
        self.manifest.validate()?;
        let computed = contract_fingerprint(&self.manifest)?;
        ensure!(
            self.contract_fingerprint == computed,
            "Provider contract fingerprint mismatch"
        );
        Ok(())
    }

    /// Encode a stable package. Struct field order and ordered maps make the
    /// resulting bytes deterministic for identical source.
    ///
    /// # Errors
    /// Returns an error when the package is invalid or cannot be serialized.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        serde_json::to_vec(self).context("encoding canonical Provider package")
    }

    #[must_use]
    pub fn artifact_digest(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }
}

impl ProviderManifest {
    /// Produce the only manifest shape that may be returned to ordinary
    /// Cowboy UI clients.
    #[must_use]
    pub fn ui_projection(&self) -> ProviderUiManifest {
        ProviderUiManifest {
            id: self.id.clone(),
            version: self.version.clone(),
            publisher: self.publisher.clone(),
            sdk_version: self.sdk_version.clone(),
            display: self.display.clone(),
            ui: self.ui.clone(),
            logic: self.logic.clone(),
            configuration: self.configuration.clone(),
            host: self.host.clone(),
            authentication: ProviderUiAuthenticationContract {
                schema_version: self.authentication.schema_version,
                required: self.authentication.required,
                methods: self
                    .authentication
                    .methods
                    .iter()
                    .map(|method| ProviderUiAuthMethod {
                        id: method.id.clone(),
                        label: method.label.clone(),
                        flow: method.flow.clone(),
                    })
                    .collect(),
            },
            compatibility: self.compatibility.clone(),
        }
    }

    /// Validate the complete runtime, UI, logic, authentication, and host
    /// contract graph against this SDK and Cowboy's current host contracts.
    ///
    /// # Errors
    /// Returns an error when any contract is malformed, internally unlinked,
    /// or incompatible with the current SDK, Controller, or Machine.
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id, "Provider id")?;
        validate_semantic_version(&self.version, "Provider version")?;
        validate_id(&self.publisher, "Provider publisher")?;
        validate_provider_sdk_version(&self.sdk_version)?;
        validate_display(&self.display)?;
        self.logic.validate()?;
        self.ui.validate(&self.logic)?;
        let logo = self
            .ui
            .assets
            .iter()
            .find(|asset| asset.id == self.display.logo_asset)
            .context("display logo asset does not exist")?;
        ensure!(
            logo.role == AssetRole::Logo,
            "display logo has the wrong role"
        );
        let icon = self
            .ui
            .assets
            .iter()
            .find(|asset| asset.id == self.display.icon_asset)
            .context("display icon asset does not exist")?;
        ensure!(
            icon.role == AssetRole::Icon,
            "display icon has the wrong role"
        );
        self.configuration.validate()?;
        self.host.validate()?;
        self.runtime.validate()?;
        self.authentication.validate()?;
        validate_auth_runtime_link(&self.authentication, &self.runtime)?;
        ensure!(
            self.compatibility.min_controller_contract <= CONTROLLER_CONTRACT_VERSION
                && self.compatibility.max_controller_contract >= CONTROLLER_CONTRACT_VERSION,
            "Provider does not support Controller contract {CONTROLLER_CONTRACT_VERSION}"
        );
        ensure!(
            self.compatibility.min_machine_contract <= MACHINE_CONTRACT_VERSION
                && self.compatibility.max_machine_contract >= MACHINE_CONTRACT_VERSION,
            "Provider does not support Machine contract {MACHINE_CONTRACT_VERSION}"
        );
        ensure!(
            self.compatibility.auth_contract_fingerprint == fingerprint_json(&self.authentication)?,
            "authentication contract fingerprint mismatch"
        );
        ensure!(
            self.compatibility.ui_component_fingerprint
                == fingerprint_json(&(&self.ui, &self.logic, &self.configuration, &self.host))?,
            "UI contract fingerprint mismatch"
        );
        Ok(())
    }
}

impl ConfigurationUiContract {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported configuration UI schema"
        );
        ensure!(self.presets.len() <= 32, "too many configuration presets");
        let mut ids = BTreeSet::new();
        let mut default_count = 0_usize;
        for preset in &self.presets {
            validate_id(&preset.id, "configuration preset")?;
            ensure!(
                ids.insert(preset.id.as_str()),
                "duplicate configuration preset"
            );
            ensure!(
                !preset.name.trim().is_empty() && preset.name.len() <= 128,
                "invalid configuration preset name"
            );
            ensure!(
                preset.detail.len() <= 512,
                "configuration preset detail is too long"
            );
            ensure!(
                !preset.values.is_empty() && preset.values.len() <= 32,
                "configuration preset has invalid values"
            );
            for (option, value) in &preset.values {
                validate_id(option, "configuration option")?;
                ensure!(
                    !value.is_empty() && !value.contains('\0') && value.len() <= 4_096,
                    "invalid configuration preset value"
                );
            }
            default_count += usize::from(preset.is_default);
        }
        ensure!(default_count <= 1, "multiple default configuration presets");
        ensure!(
            self.options.len() <= 64,
            "too many configuration option presentations"
        );
        let mut option_ids = BTreeSet::new();
        for option in &self.options {
            validate_id(&option.id, "configuration option presentation")?;
            ensure!(
                option_ids.insert(option.id.as_str()),
                "duplicate configuration option presentation"
            );
        }
        Ok(())
    }
}

impl HostIntegrationContract {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported host integration schema"
        );
        if let Some(compaction) = &self.conversation_compaction {
            ensure!(
                !compaction.aliases.is_empty() && compaction.aliases.len() <= 16,
                "invalid conversation compaction aliases"
            );
            for alias in &compaction.aliases {
                validate_id(alias, "conversation compaction alias")?;
            }
            validate_id(
                &compaction.fallback_command,
                "conversation compaction fallback command",
            )?;
            ensure!(
                compaction.aliases.contains(&compaction.fallback_command),
                "conversation compaction fallback must be one of its aliases"
            );
        }
        ensure!(
            self.tool_presentations.len() <= 64,
            "too many tool presentations"
        );
        let mut tool_names = BTreeSet::new();
        for presentation in &self.tool_presentations {
            ensure!(
                !presentation.tool_name.trim().is_empty()
                    && presentation.tool_name.trim() == presentation.tool_name
                    && presentation.tool_name.len() <= 128
                    && presentation.tool_name.is_ascii()
                    && !presentation
                        .tool_name
                        .chars()
                        .any(|character| character.is_ascii_control()),
                "invalid tool presentation name"
            );
            ensure!(
                tool_names.insert(presentation.tool_name.as_str()),
                "duplicate tool presentation"
            );
        }
        Ok(())
    }
}

impl UiContract {
    fn validate(&self, logic: &LogicContract) -> Result<()> {
        ensure!(
            self.schema_version == UI_SCHEMA_VERSION,
            "unsupported UI schema"
        );
        ensure!(self.assets.len() <= 64, "too many Provider UI assets");
        let mut assets = BTreeSet::new();
        for asset in &self.assets {
            validate_id(&asset.id, "asset id")?;
            ensure!(
                assets.insert(asset.id.as_str()),
                "duplicate asset id {}",
                asset.id
            );
            ensure!(
                !asset.accessible_label.trim().is_empty() && asset.accessible_label.len() <= 512,
                "asset label is empty or too long"
            );
            validate_digest(&asset.digest, "asset digest")?;
            ensure!(
                asset.digest == fingerprint_json(&asset.content)?,
                "asset digest mismatch"
            );
            match &asset.content {
                AssetContent::VectorPath {
                    view_box,
                    path,
                    fill,
                } => {
                    ensure!(
                        asset.media_type == "image/svg+xml",
                        "vector asset has the wrong media type"
                    );
                    validate_view_box(view_box)?;
                    validate_vector_path(path)?;
                    if let Some(fill) = fill {
                        validate_color(fill, "vector asset fill")?;
                    }
                }
                AssetContent::Inline { base64 } => {
                    ensure!(
                        matches!(
                            asset.media_type.as_str(),
                            "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
                        ),
                        "inline Provider asset media type is not allowed"
                    );
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(base64)
                        .context("invalid inline asset base64")?;
                    ensure!(decoded.len() <= 1_048_576, "inline asset exceeds 1 MiB");
                }
            }
        }
        for role in [AssetRole::Logo, AssetRole::Icon, AssetRole::Loading] {
            self_manifest_asset(&self.assets, role)?;
        }
        for slot in REQUIRED_SURFACES {
            ensure!(
                self.surfaces.contains_key(&slot),
                "missing {slot:?} UI surface"
            );
        }
        let state = logic
            .state
            .iter()
            .map(|field| (field.id.as_str(), &field.value_type))
            .collect();
        let messages = logic
            .messages
            .iter()
            .map(|message| (&*message.id, message))
            .collect();
        let mut nodes = 0_usize;
        for (slot, surface) in &self.surfaces {
            validate_node(surface, &assets, &state, &messages, 0, &mut nodes)?;
            validate_surface_effects(slot, surface, logic)?;
        }
        ensure!(nodes <= 2_048, "Provider UI has too many nodes");
        Ok(())
    }
}

fn validate_surface_effects(
    slot: &SurfaceSlot,
    node: &UiNode,
    logic: &LogicContract,
) -> Result<()> {
    match node {
        UiNode::Stack { children, .. } => {
            for child in children {
                validate_surface_effects(slot, child, logic)?;
            }
        }
        UiNode::Button { emit, .. } => {
            for effect_id in logic
                .reducers
                .iter()
                .filter(|rule| rule.message == emit.message)
                .filter_map(|rule| rule.effect.as_deref())
            {
                let effect = logic
                    .effects
                    .iter()
                    .find(|effect| effect.id == effect_id)
                    .context("surface button references an unknown effect")?;
                let valid_slot = match effect.capability {
                    EffectCapability::BeginServiceAuthentication
                    | EffectCapability::LogoutServiceAuthentication => *slot == SurfaceSlot::Setup,
                    EffectCapability::InstallOnMachine => *slot == SurfaceSlot::Empty,
                    EffectCapability::UpgradeOnMachine | EffectCapability::RequestUninstallPlan => {
                        *slot == SurfaceSlot::Settings
                    }
                    EffectCapability::OpenExternalDocumentation => true,
                };
                ensure!(
                    valid_slot,
                    "Provider effect {:?} is not allowed on {slot:?} surface",
                    effect.capability
                );
            }
        }
        UiNode::Text { .. }
        | UiNode::Asset { .. }
        | UiNode::Badge { .. }
        | UiNode::Progress { .. }
        | UiNode::Alert { .. }
        | UiNode::Divider => {}
    }
    Ok(())
}

fn self_manifest_asset(assets: &[UiAsset], role: AssetRole) -> Result<&str> {
    assets
        .iter()
        .find(|asset| asset.role == role)
        .map(|asset| asset.id.as_str())
        .context("required UI asset role is missing")
}

impl LogicContract {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == LOGIC_SCHEMA_VERSION,
            "unsupported logic schema"
        );
        ensure!(self.state.len() <= 64, "too many Provider state fields");
        ensure!(self.messages.len() <= 128, "too many Provider messages");
        ensure!(self.reducers.len() <= 256, "too many Provider reducers");
        ensure!(self.effects.len() <= 64, "too many Provider effects");
        let mut state = BTreeMap::new();
        for field in &self.state {
            validate_id(&field.id, "state field")?;
            ensure!(
                literal_matches(&field.initial, &field.value_type),
                "state initial value type mismatch"
            );
            ensure!(
                state.insert(field.id.as_str(), &field.value_type).is_none(),
                "duplicate state field"
            );
        }
        let mut messages = BTreeMap::new();
        for message in &self.messages {
            validate_id(&message.id, "message id")?;
            ensure!(message.payload.len() <= 64, "message payload is too wide");
            for field in message.payload.keys() {
                validate_id(field, "message payload field")?;
            }
            ensure!(
                messages.insert(message.id.as_str(), message).is_none(),
                "duplicate message id"
            );
        }
        let mut effects = BTreeSet::new();
        for effect in &self.effects {
            validate_id(&effect.id, "effect id")?;
            ensure!(effects.insert(effect.id.as_str()), "duplicate effect id");
            ensure!(effect.request.len() <= 64, "effect request is too wide");
            for field in effect.request.keys() {
                validate_id(field, "effect request field")?;
            }
            ensure!(
                messages.contains_key(effect.success_message.as_str()),
                "unknown effect success message"
            );
            ensure!(
                messages.contains_key(effect.failure_message.as_str()),
                "unknown effect failure message"
            );
        }
        let mut effect_messages = BTreeSet::new();
        for reducer in &self.reducers {
            ensure!(
                messages.contains_key(reducer.message.as_str()),
                "reducer handles unknown message"
            );
            if let Some(effect) = &reducer.effect {
                ensure!(
                    effects.contains(effect.as_str()),
                    "reducer invokes unknown effect"
                );
                ensure!(
                    effect_messages.insert(reducer.message.as_str()),
                    "a Provider message cannot invoke multiple effects"
                );
            }
            ensure!(
                reducer.assignments.len() <= 64,
                "reducer has too many assignments"
            );
            for assignment in &reducer.assignments {
                let target_type = state
                    .get(assignment.field.as_str())
                    .context("assignment targets unknown state")?;
                let source_type = match &assignment.value {
                    AssignmentValue::Literal { value } => {
                        ensure!(
                            literal_matches(value, target_type),
                            "assignment literal type mismatch"
                        );
                        *target_type
                    }
                    AssignmentValue::State { field } => state
                        .get(field.as_str())
                        .copied()
                        .context("assignment reads unknown state")?,
                    AssignmentValue::Message { field } => messages
                        .get(reducer.message.as_str())
                        .and_then(|message| message.payload.get(field))
                        .context("assignment reads unknown message payload")?,
                };
                ensure!(
                    source_type == *target_type,
                    "assignment value type mismatch"
                );
            }
        }
        Ok(())
    }
}

impl ProviderBehaviorContract {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported Provider behavior schema"
        );
        ensure!(
            self.default_preferences.len() <= 64,
            "too many default Provider preferences"
        );
        for option in self.default_preferences.keys() {
            validate_id(option, "default Provider preference")?;
        }
        ensure!(
            self.error_rules.len() <= 32,
            "too many Provider error rules"
        );
        let mut expression_nodes = 0_usize;
        for rule in &self.error_rules {
            rule.when.validate(0, &mut expression_nodes)?;
            if let Some(detail) = &rule.user_detail {
                ensure!(
                    !detail.trim().is_empty() && detail.len() <= 2_048,
                    "invalid Provider error presentation"
                );
            }
            if let Some(classification) = &rule.classification {
                validate_id(classification, "Provider error classification")?;
            }
            ensure!(
                !rule.retry_once_without_visible_update || rule.keep_worker_alive,
                "a retryable Provider error must keep the worker alive"
            );
        }
        ensure!(
            expression_nodes <= 256,
            "Provider error matcher is too complex"
        );
        Ok(())
    }

    #[must_use]
    pub fn matching_error_rule(&self, detail: &str) -> Option<&RuntimeErrorRule> {
        self.error_rules
            .iter()
            .find(|rule| rule.when.matches(detail))
    }
}

impl TextMatchExpression {
    fn validate(&self, depth: usize, nodes: &mut usize) -> Result<()> {
        ensure!(depth <= 8, "Provider error matcher is too deeply nested");
        *nodes = nodes.saturating_add(1);
        match self {
            Self::Contains { value } => ensure!(
                !value.trim().is_empty() && value.len() <= 512,
                "invalid Provider error substring"
            ),
            Self::All { values } | Self::Any { values } => {
                ensure!(
                    !values.is_empty() && values.len() <= 16,
                    "invalid Provider error matcher branch"
                );
                for value in values {
                    value.validate(depth.saturating_add(1), nodes)?;
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn matches(&self, detail: &str) -> bool {
        self.matches_normalized(&detail.to_ascii_lowercase())
    }

    fn matches_normalized(&self, detail: &str) -> bool {
        match self {
            Self::Contains { value } => detail.contains(&value.to_ascii_lowercase()),
            Self::All { values } => values.iter().all(|value| value.matches_normalized(detail)),
            Self::Any { values } => values.iter().any(|value| value.matches_normalized(detail)),
        }
    }
}

impl RuntimeContract {
    // Runtime graph validation intentionally stays in one cross-field pass so
    // platform components, capabilities, sidecars, auth links, and value
    // bindings cannot drift between partially independent validators.
    #[allow(clippy::too_many_lines)]
    fn validate(&self) -> Result<()> {
        ensure!(self.driver_schema == 1, "unsupported runtime driver schema");
        self.behavior.validate()?;
        ensure!(
            !self.protocol.trim().is_empty(),
            "runtime protocol is empty"
        );
        validate_id(&self.entrypoint, "runtime entrypoint")?;
        for argument in &self.arguments {
            argument.validate()?;
        }
        for (name, value) in &self.environment {
            ensure!(
                valid_environment_name(name),
                "invalid runtime environment name"
            );
            value.validate()?;
        }
        for name in &self.remove_environment {
            ensure!(
                valid_environment_name(name),
                "invalid runtime environment name"
            );
        }
        for prefix in &self.remove_environment_prefixes {
            ensure!(
                !prefix.is_empty()
                    && prefix.len() <= 64
                    && prefix.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    }),
                "invalid runtime environment prefix"
            );
        }
        let uses_gateway = matches!(
            self.behavior.configuration,
            ConfigurationBehavior::AnthropicGatewayV1 | ConfigurationBehavior::OpenaiGatewayV1
        );
        let declares_gateway = !self.sidecars.is_empty();
        ensure!(
            uses_gateway == declares_gateway,
            "gateway-backed behavior and session sidecars must be declared together"
        );
        ensure!(
            self.required_capabilities
                .contains(&RuntimeCapability::ProviderGatewayV1)
                == declares_gateway,
            "provider.gateway.v1 does not match the session sidecar contract"
        );
        ensure!(
            self.required_capabilities
                .contains(&RuntimeCapability::ProviderRuntimeV1),
            "Provider runtime is missing provider.runtime.v1"
        );
        let mut dependencies = BTreeSet::new();
        for dependency in &self.dependencies {
            validate_id(&dependency.id, "dependency id")?;
            validate_exact_version(&dependency.version, "dependency version")?;
            ensure!(
                dependencies.insert(dependency.id.as_str()),
                "duplicate dependency"
            );
            validate_dependency_integrity(&dependency.integrity)?;
            ensure!(
                !dependency.source.contains("latest"),
                "dependency source uses latest"
            );
        }
        ensure!(
            !self.platforms.is_empty(),
            "Provider has no platform payload"
        );
        let mut targets = BTreeSet::new();
        for payload in &self.platforms {
            ensure!(
                targets.insert((&payload.os, &payload.architecture)),
                "duplicate platform payload"
            );
            validate_digest(&payload.payload_digest, "platform payload digest")?;
            ensure!(
                payload.payload_digest
                    == platform_payload_fingerprint(payload, &self.dependencies)?,
                "platform payload fingerprint mismatch"
            );
            validate_id(&payload.launch_command, "platform launch command")?;
            ensure!(
                !payload.private_components.is_empty(),
                "platform payload has no private components"
            );
            let mut slots = BTreeSet::new();
            let mut commands = BTreeSet::new();
            for component in &payload.private_components {
                validate_id(&component.slot, "private component slot")?;
                validate_id(&component.dependency, "private component dependency")?;
                validate_id(&component.command, "private component command")?;
                ensure!(
                    dependencies.contains(component.dependency.as_str()),
                    "private component references an undeclared dependency"
                );
                ensure!(
                    slots.insert((&component.kind, component.slot.as_str())),
                    "duplicate private component slot"
                );
                ensure!(
                    commands.insert(component.command.as_str()),
                    "duplicate private component command"
                );
            }
            ensure!(
                commands.contains(payload.launch_command.as_str()),
                "platform launch command is not exported by a private component"
            );
        }
        let mut sidecar_ids = BTreeSet::new();
        let mut sidecar_components = BTreeSet::new();
        for sidecar in &self.sidecars {
            validate_id(&sidecar.id, "runtime sidecar")?;
            ensure!(
                sidecar_ids.insert(sidecar.id.as_str()),
                "duplicate runtime sidecar"
            );
            ensure!(
                sidecar.component.kind == PrivateComponentKind::ProviderGateway,
                "runtime sidecar component is not a Provider gateway"
            );
            ensure!(
                sidecar_components.insert(&sidecar.component),
                "duplicate runtime sidecar component"
            );
            ensure!(
                self.platforms.iter().all(|platform| {
                    platform.private_components.iter().any(|candidate| {
                        candidate.kind == sidecar.component.kind
                            && candidate.slot == sidecar.component.slot
                    })
                }),
                "runtime sidecar component is unavailable on a supported platform"
            );
            ensure!(
                sidecar.arguments.len() <= 64
                    && sidecar
                        .arguments
                        .iter()
                        .all(|argument| argument.len() <= 4_096 && !argument.contains('\0')),
                "invalid runtime sidecar arguments"
            );
            for (name, value) in &sidecar.environment {
                ensure!(
                    valid_environment_name(name) && value.len() <= 32_768 && !value.contains('\0'),
                    "invalid runtime sidecar environment"
                );
            }
            for name in &sidecar.auth_environment {
                ensure!(
                    valid_environment_name(name),
                    "invalid runtime sidecar auth environment"
                );
            }
            sidecar.transport.validate()?;
        }
        for value in self.arguments.iter().chain(self.environment.values()) {
            match value {
                RuntimeValue::Literal(_) => {}
                RuntimeValue::Binding(RuntimeBinding::ComponentCommand { component, .. }) => {
                    ensure!(
                        self.platforms.iter().all(|platform| {
                            platform.private_components.iter().any(|candidate| {
                                candidate.kind == component.kind && candidate.slot == component.slot
                            })
                        }),
                        "runtime value component is unavailable on a supported platform"
                    );
                }
                RuntimeValue::Binding(RuntimeBinding::SidecarUrl { sidecar, .. }) => {
                    ensure!(
                        sidecar_ids.contains(sidecar.as_str()),
                        "runtime value references an unknown sidecar"
                    );
                }
            }
        }
        for platform in &self.platforms {
            for component in &platform.private_components {
                if component.kind == PrivateComponentKind::ProviderGateway {
                    ensure!(
                        sidecar_components.iter().any(|reference| {
                            reference.kind == component.kind && reference.slot == component.slot
                        }),
                        "Provider gateway component has no runtime sidecar"
                    );
                }
            }
        }
        Ok(())
    }
}

impl RuntimeValue {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Literal(value) => ensure!(
                value.len() <= 32_768 && !value.contains('\0'),
                "invalid runtime literal"
            ),
            Self::Binding(binding) => binding.validate()?,
        }
        Ok(())
    }
}

impl RuntimeBinding {
    fn validate(&self) -> Result<()> {
        let (prefix, suffix) = match self {
            Self::ComponentCommand {
                component,
                prefix,
                suffix,
            } => {
                validate_id(&component.slot, "runtime component binding")?;
                (prefix, suffix)
            }
            Self::SidecarUrl {
                sidecar,
                prefix,
                suffix,
            } => {
                validate_id(sidecar, "runtime sidecar binding")?;
                (prefix, suffix)
            }
        };
        ensure!(
            prefix.len() <= 4_096
                && suffix.len() <= 4_096
                && !prefix.contains('\0')
                && !suffix.contains('\0'),
            "invalid runtime binding decoration"
        );
        Ok(())
    }
}

impl RuntimeSidecarTransport {
    fn validate(&self) -> Result<()> {
        match self {
            Self::LoopbackHttpV1 {
                listen_argument,
                health_path,
                timeout_ms,
            } => {
                ensure!(
                    (3..=64).contains(&listen_argument.len())
                        && listen_argument.starts_with("--")
                        && listen_argument[2..].bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                        }),
                    "invalid runtime sidecar listen argument"
                );
                ensure!(
                    health_path.starts_with('/')
                        && health_path.len() <= 256
                        && !health_path
                            .chars()
                            .any(|character| matches!(character, '?' | '#' | '\0'))
                        && !health_path.starts_with("//"),
                    "invalid runtime sidecar health path"
                );
                ensure!(
                    (100..=120_000).contains(timeout_ms),
                    "invalid runtime sidecar readiness timeout"
                );
            }
        }
        Ok(())
    }
}

impl AuthenticationContract {
    // This is one closed cross-field validator; splitting it would obscure the
    // uniqueness and projection invariants it checks in one pass.
    #[allow(clippy::too_many_lines)]
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == AUTH_SCHEMA_VERSION,
            "unsupported authentication schema"
        );
        validate_id(&self.portable_schema, "portable credential schema")?;
        validate_id(&self.projection_schema, "Machine projection schema")?;
        ensure!(
            !self.required || !self.methods.is_empty(),
            "authenticated Provider has no login method"
        );
        let mut methods = BTreeSet::new();
        for method in &self.methods {
            validate_id(&method.id, "authentication method")?;
            ensure!(
                !method.label.trim().is_empty(),
                "authentication method label is empty"
            );
            ensure!(
                methods.insert(method.id.as_str()),
                "duplicate authentication method"
            );
            match &method.executor {
                AuthExecutor::SecretInputV1 {
                    bundle_key,
                    verification_url,
                } => {
                    ensure!(
                        method.flow == AuthFlow::SecretInput,
                        "secret executor must use secret_input flow"
                    );
                    validate_id(bundle_key, "secret bundle key")?;
                    validate_https_url(verification_url, "secret verification URL")?;
                }
                AuthExecutor::CommandV1 {
                    component,
                    arguments,
                    environment,
                    preflight,
                    ..
                } => {
                    ensure!(
                        method.flow != AuthFlow::SecretInput,
                        "command executor cannot use secret_input flow"
                    );
                    validate_id(&component.slot, "authentication component slot")?;
                    ensure!(arguments.len() <= 64, "too many authentication arguments");
                    ensure!(
                        arguments
                            .iter()
                            .all(|value| !value.contains('\0') && value.len() <= 4_096),
                        "invalid authentication argument"
                    );
                    for (name, value) in environment {
                        ensure!(
                            valid_environment_name(name),
                            "invalid authentication environment name"
                        );
                        ensure!(
                            !value.contains('\0') && value.len() <= 16_384,
                            "invalid authentication environment value"
                        );
                    }
                    ensure!(
                        preflight.len() <= 16,
                        "too many authentication preflight steps"
                    );
                    for step in preflight {
                        match step {
                            AuthPreflight::JsonStringSetV1 {
                                relative_path,
                                path,
                                value,
                            } => {
                                validate_relative_path(relative_path)?;
                                ensure!(
                                    !path.is_empty()
                                        && path.len() <= 16
                                        && path.iter().all(|segment| {
                                            !segment.is_empty()
                                                && segment.len() <= 128
                                                && !segment.contains(['\0', '/', '~'])
                                        }),
                                    "invalid authentication JSON path"
                                );
                                ensure!(
                                    !value.contains('\0') && value.len() <= 4_096,
                                    "invalid authentication JSON value"
                                );
                            }
                            AuthPreflight::EnvFileKeyRequiredV1 { relative_path, key } => {
                                validate_relative_path(relative_path)?;
                                ensure!(
                                    valid_environment_name(key),
                                    "invalid required environment key"
                                );
                            }
                        }
                    }
                }
            }
        }
        let mut paths = BTreeSet::new();
        for file in &self.credential_files {
            validate_id(&file.bundle_key, "credential bundle key")?;
            validate_relative_path(&file.relative_path)?;
            ensure!(
                paths.insert(file.relative_path.as_str()),
                "duplicate credential path"
            );
        }
        for (name, bundle_key) in &self.environment_projection {
            ensure!(
                valid_environment_name(name),
                "invalid environment projection name"
            );
            validate_id(bundle_key, "environment projection bundle key")?;
        }
        let allowed: BTreeSet<_> = self
            .credential_files
            .iter()
            .map(|file| file.bundle_key.as_str())
            .chain(self.environment_projection.values().map(String::as_str))
            .collect();
        for method in &self.methods {
            ensure!(
                !method.required_bundle_keys.is_empty(),
                "authentication method has no required bundle value"
            );
            ensure!(
                method
                    .required_bundle_keys
                    .iter()
                    .all(|key| allowed.contains(key.as_str())),
                "authentication method requires an undeclared bundle value"
            );
            if let AuthExecutor::SecretInputV1 { bundle_key, .. } = &method.executor {
                ensure!(
                    method.required_bundle_keys.len() == 1
                        && method.required_bundle_keys.contains(bundle_key),
                    "secret executor bundle key does not match its method contract"
                );
            } else {
                let file_keys: BTreeSet<_> = self
                    .credential_files
                    .iter()
                    .map(|file| file.bundle_key.as_str())
                    .collect();
                ensure!(
                    method
                        .required_bundle_keys
                        .iter()
                        .all(|key| file_keys.contains(key.as_str())),
                    "command executor credentials must be declared files"
                );
            }
        }
        ensure!(
            !self.required
                || !self.credential_files.is_empty()
                || !self.environment_projection.is_empty(),
            "authenticated Provider has no credential projection"
        );
        Ok(())
    }
}

fn validate_auth_runtime_link(
    authentication: &AuthenticationContract,
    runtime: &RuntimeContract,
) -> Result<()> {
    for method in &authentication.methods {
        let AuthExecutor::CommandV1 { component, .. } = &method.executor else {
            continue;
        };
        ensure!(
            runtime.platforms.iter().all(|platform| {
                platform.private_components.iter().any(|candidate| {
                    candidate.kind == component.kind && candidate.slot == component.slot
                })
            }),
            "authentication executor component is unavailable on a supported platform"
        );
    }
    let mut sidecar_auth_environment = BTreeSet::new();
    for sidecar in &runtime.sidecars {
        for name in &sidecar.auth_environment {
            ensure!(
                authentication.environment_projection.contains_key(name),
                "runtime sidecar references an undeclared auth environment projection"
            );
            ensure!(
                sidecar_auth_environment.insert(name.as_str()),
                "auth environment projection is forwarded to multiple sidecars"
            );
        }
    }
    Ok(())
}

impl ProviderRelease {
    /// Validate the signed release envelope and bind it to the exact artifact
    /// bytes, not merely to a Provider-authored compatibility claim.
    ///
    /// # Errors
    /// Returns an error when package parsing or any release identity, digest,
    /// platform, or runtime binding check fails.
    pub fn validate_bytes(&self, bytes: &[u8]) -> Result<ProviderPackage> {
        let package = ProviderPackage::from_bytes(bytes)?;
        self.validate_for(&package)?;
        ensure!(
            self.package_digest == ProviderPackage::artifact_digest(bytes),
            "release package digest mismatch"
        );
        Ok(package)
    }

    /// Validate this release against an already validated Provider package.
    ///
    /// # Errors
    /// Returns an error when the semantic identity, compatibility matrix,
    /// runtime artifacts, signature presence, or composite digest mismatches.
    pub fn validate_for(&self, package: &ProviderPackage) -> Result<()> {
        ensure!(
            self.release_schema == RELEASE_SCHEMA_VERSION,
            "unsupported release schema"
        );
        ensure!(
            self.provider_id == package.manifest.id,
            "release Provider id mismatch"
        );
        ensure!(
            self.provider_version == package.manifest.version,
            "release Provider version mismatch"
        );
        ensure!(
            self.publisher == package.manifest.publisher,
            "release publisher mismatch"
        );
        ensure!(
            self.contract_fingerprint == package.contract_fingerprint,
            "release contract mismatch"
        );
        validate_digest(&self.package_digest, "release package digest")?;
        validate_digest(&self.artifact_digest, "release artifact digest")?;
        validate_runtime_artifact_url(&self.artifact_url)
            .context("invalid Provider package artifact URL")?;
        ensure!(
            !self.signature.trim().is_empty(),
            "Provider release is unsigned"
        );
        let expected: BTreeSet<_> = package
            .manifest
            .runtime
            .platforms
            .iter()
            .map(|payload| PlatformTarget {
                os: payload.os.clone(),
                architecture: payload.architecture.clone(),
            })
            .collect();
        let supported = self
            .supported_platforms
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        ensure!(
            supported.len() == self.supported_platforms.len() && supported == expected,
            "release platform matrix mismatch"
        );
        validate_runtime_artifacts(self, package, &expected)?;
        ensure!(
            self.artifact_digest == self.computed_artifact_digest()?,
            "release composite artifact digest mismatch"
        );
        Ok(())
    }

    /// Compute the content identity of the package plus its exact runtime
    /// artifact matrix.
    ///
    /// # Errors
    /// Returns an error when the release identity cannot be serialized.
    pub fn computed_artifact_digest(&self) -> Result<String> {
        fingerprint_json(&serde_json::json!({
            "release_schema": self.release_schema,
            "provider_id": self.provider_id,
            "provider_version": self.provider_version,
            "package_digest": self.package_digest,
            "publisher": self.publisher,
            "contract_fingerprint": self.contract_fingerprint,
            "supported_platforms": self.supported_platforms,
            "runtime_artifacts": self.runtime_artifacts,
        }))
    }

    /// Produce the namespaced, length-delimited release signature payload.
    #[must_use]
    pub fn proof(&self) -> Vec<u8> {
        let runtime_artifacts = runtime_artifacts_fingerprint(self);
        let fields = [
            self.provider_id.as_str(),
            self.provider_version.as_str(),
            self.package_digest.as_str(),
            self.artifact_digest.as_str(),
            self.publisher.as_str(),
            self.contract_fingerprint.as_str(),
            runtime_artifacts.as_str(),
        ];
        let mut proof = b"cowboy-provider-release-v1\n".to_vec();
        for field in fields {
            proof.extend_from_slice(field.len().to_string().as_bytes());
            proof.push(b':');
            proof.extend_from_slice(field.as_bytes());
            proof.push(b'\n');
        }
        proof
    }
}

fn runtime_artifacts_fingerprint(release: &ProviderRelease) -> String {
    fingerprint_json(&(&release.supported_platforms, &release.runtime_artifacts))
        .expect("Provider release runtime artifacts serialize")
}

// The runtime matrix is validated in one pass so target and component
// uniqueness cannot drift between independent helpers.
#[allow(clippy::too_many_lines)]
fn validate_runtime_artifacts(
    release: &ProviderRelease,
    package: &ProviderPackage,
    expected_targets: &BTreeSet<PlatformTarget>,
) -> Result<()> {
    let mut targets = BTreeSet::new();
    for artifacts in &release.runtime_artifacts {
        let target = PlatformTarget {
            os: artifacts.os.clone(),
            architecture: artifacts.architecture.clone(),
        };
        ensure!(
            targets.insert(target.clone()),
            "duplicate release runtime target"
        );
        ensure!(
            expected_targets.contains(&target),
            "release runtime artifact targets an undeclared platform"
        );
        let payload = package
            .manifest
            .runtime
            .platforms
            .iter()
            .find(|payload| {
                payload.os == artifacts.os && payload.architecture == artifacts.architecture
            })
            .context("release runtime target has no package payload")?;
        ensure!(
            artifacts.components.len() == payload.private_components.len(),
            "release runtime component set is incomplete"
        );
        let mut components = BTreeSet::new();
        for artifact in &artifacts.components {
            validate_id(&artifact.slot, "released private component slot")?;
            validate_id(
                &artifact.dependency,
                "released private component dependency",
            )?;
            validate_exact_version(&artifact.version, "released private component version")?;
            validate_id(&artifact.command, "released private component command")?;
            validate_runtime_artifact_url(&artifact.artifact_url)?;
            validate_digest(
                &artifact.artifact_digest,
                "released private component digest",
            )?;
            ensure!(
                artifact.probe.args.len() <= 64
                    && artifact
                        .probe
                        .args
                        .iter()
                        .all(|argument| !argument.contains('\0') && argument.len() <= 4_096),
                "invalid released private component probe arguments"
            );
            ensure!(
                (100..=120_000).contains(&artifact.probe.timeout_ms),
                "invalid released private component probe timeout"
            );
            match artifact.artifact_format {
                ProviderArtifactFormat::Raw => ensure!(
                    artifact.entrypoint.is_none(),
                    "raw released component cannot declare an archive entrypoint"
                ),
                ProviderArtifactFormat::TarGz => {
                    validate_relative_path(
                        artifact
                            .entrypoint
                            .as_deref()
                            .context("archive released component requires an entrypoint")?,
                    )?;
                }
            }
            let key = (&artifact.kind, artifact.slot.as_str());
            ensure!(
                components.insert(key),
                "duplicate released private component"
            );
            let requirement = payload
                .private_components
                .iter()
                .find(|requirement| {
                    requirement.kind == artifact.kind && requirement.slot == artifact.slot
                })
                .context("release contains an undeclared private component")?;
            ensure!(
                requirement.dependency == artifact.dependency
                    && requirement.command == artifact.command,
                "released private component binding mismatch"
            );
            let dependency = package
                .manifest
                .runtime
                .dependencies
                .iter()
                .find(|dependency| dependency.id == artifact.dependency)
                .context("released private component dependency disappeared")?;
            ensure!(
                dependency.version == artifact.version,
                "released private component version does not match its exact dependency pin"
            );
        }
        ensure!(
            artifacts
                .components
                .iter()
                .any(|artifact| artifact.command == payload.launch_command),
            "release runtime does not export the platform launch command"
        );
    }
    ensure!(
        targets == *expected_targets,
        "release runtime artifact platform matrix mismatch"
    );
    Ok(())
}

/// Finalize computed fingerprints and build one canonical Provider package.
///
/// # Errors
/// Returns an error when computed fields cannot be serialized or the complete
/// manifest fails validation.
pub fn build_package(mut manifest: ProviderManifest) -> Result<ProviderPackage> {
    for asset in &mut manifest.ui.assets {
        if asset.digest.is_empty() {
            asset.digest = fingerprint_json(&asset.content)?;
        }
    }
    let dependencies = &manifest.runtime.dependencies;
    for payload in &mut manifest.runtime.platforms {
        if payload.payload_digest.is_empty() {
            payload.payload_digest = platform_payload_fingerprint(payload, dependencies)?;
        }
    }
    if manifest.compatibility.ui_component_fingerprint.is_empty() {
        manifest.compatibility.ui_component_fingerprint = fingerprint_json(&(
            &manifest.ui,
            &manifest.logic,
            &manifest.configuration,
            &manifest.host,
        ))?;
    }
    if manifest.compatibility.auth_contract_fingerprint.is_empty() {
        manifest.compatibility.auth_contract_fingerprint =
            fingerprint_json(&manifest.authentication)?;
    }
    manifest.validate()?;
    let contract_fingerprint = contract_fingerprint(&manifest)?;
    Ok(ProviderPackage {
        package_schema: PACKAGE_SCHEMA_VERSION,
        manifest,
        contract_fingerprint,
    })
}

/// Build one independently authored Provider source into an atomic package
/// file and return its validated package plus digest.
///
/// # Errors
/// Returns an error when the source cannot be read, compiled, validated, or
/// atomically written.
pub fn build_package_file(source: &Path, output: &Path) -> Result<(ProviderPackage, String)> {
    let bytes = std::fs::read(source).with_context(|| format!("reading {}", source.display()))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("decoding {}", source.display()))?;
    let manifest = if value.get("authoring_schema").is_some() {
        serde_json::from_value::<StandardProviderSource>(value)
            .context("decoding standard Provider authoring source")?
            .compile()?
    } else {
        serde_json::from_value::<ProviderManifest>(value)
            .context("decoding complete Provider manifest")?
    };
    let package = build_package(manifest)?;
    let bytes = package.canonical_bytes()?;
    let digest = ProviderPackage::artifact_digest(&bytes);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = output.with_extension("cowboy-provider.partial");
    std::fs::write(&temporary, &bytes)?;
    std::fs::rename(temporary, output)?;
    Ok((package, digest))
}

impl StandardProviderSource {
    /// Compile the standard profile into closed UI, message, reducer, effect,
    /// asset, runtime, and authentication contracts.
    ///
    /// # Errors
    /// Returns an error when authoring fields are invalid or cannot be linked
    /// into a complete Provider manifest.
    #[allow(clippy::too_many_lines)]
    pub fn compile(self) -> Result<ProviderManifest> {
        ensure!(self.authoring_schema == 2, "unsupported authoring schema");
        let logo_asset = format!("{}-logo", self.id);
        let icon_asset = format!("{}-icon", self.id);
        let loading_asset = format!("{}-loading", self.id);
        let asset = |id: String, role: AssetRole| UiAsset {
            id,
            role,
            media_type: "image/svg+xml".to_owned(),
            digest: String::new(),
            accessible_label: self.display.name.clone(),
            content: AssetContent::VectorPath {
                view_box: self.display.mark_view_box.clone(),
                path: self.display.mark_path.clone(),
                fill: self.display.mark_fill.clone(),
            },
        };
        let literal = |value: &str| TextValue::Literal {
            value: value.to_owned(),
        };
        let icon = || UiNode::Asset {
            asset: icon_asset.clone(),
            size: AssetSize::Md,
        };
        let identity = || UiNode::Stack {
            direction: StackDirection::Column,
            gap: SpacingToken::Xs,
            children: vec![
                UiNode::Text {
                    variant: TextVariant::Title,
                    value: literal(&self.display.name),
                    tone: None,
                },
                UiNode::Text {
                    variant: TextVariant::Caption,
                    value: literal(&self.display.summary),
                    tone: Some(Tone::Neutral),
                },
            ],
            visible_when: None,
        };
        let status = || UiNode::Badge {
            label: TextValue::Host {
                field: HostTextField::InstallationState,
            },
            tone: Tone::Primary,
        };
        let card_children = match self.card_layout {
            StandardCardLayout::MarkLeading => vec![icon(), identity(), status()],
            StandardCardLayout::MarkAbove => vec![icon(), status(), identity()],
            StandardCardLayout::SplitStatus => vec![identity(), status(), icon()],
        };
        let button = |label: &str, message: &str, style: ButtonStyle| UiNode::Button {
            label: literal(label),
            style,
            emit: MessageEmission {
                message: message.to_owned(),
                payload: BTreeMap::new(),
            },
            enabled_when: Some(BoolExpression::All {
                values: vec![
                    BoolExpression::HostEquals {
                        field: HostBoolField::MachineOnline,
                        value: true,
                    },
                    BoolExpression::StateEquals {
                        field: "operation_pending".to_owned(),
                        value: LiteralValue::Bool(false),
                    },
                ],
            }),
        };
        let stack = |children: Vec<UiNode>| UiNode::Stack {
            direction: StackDirection::Responsive,
            gap: SpacingToken::Sm,
            children,
            visible_when: None,
        };
        let mut surfaces = BTreeMap::new();
        surfaces.insert(SurfaceSlot::Card, stack(card_children));
        surfaces.insert(
            SurfaceSlot::Setup,
            stack(vec![
                UiNode::Text {
                    variant: TextVariant::Body,
                    value: literal(
                        "Authentication belongs to this Cowboy Service and syncs to every Machine.",
                    ),
                    tone: None,
                },
                UiNode::Button {
                    label: literal("Sign in"),
                    style: ButtonStyle::Primary,
                    emit: MessageEmission {
                        message: "authenticate".to_owned(),
                        payload: BTreeMap::new(),
                    },
                    enabled_when: Some(BoolExpression::All {
                        values: vec![
                            BoolExpression::Not {
                                value: Box::new(BoolExpression::HostEquals {
                                    field: HostBoolField::AuthReady,
                                    value: true,
                                }),
                            },
                            BoolExpression::StateEquals {
                                field: "operation_pending".to_owned(),
                                value: LiteralValue::Bool(false),
                            },
                        ],
                    }),
                },
                UiNode::Button {
                    label: literal("Sign out"),
                    style: ButtonStyle::Secondary,
                    emit: MessageEmission {
                        message: "logout".to_owned(),
                        payload: BTreeMap::new(),
                    },
                    enabled_when: Some(BoolExpression::All {
                        values: vec![
                            BoolExpression::HostEquals {
                                field: HostBoolField::AuthReady,
                                value: true,
                            },
                            BoolExpression::StateEquals {
                                field: "operation_pending".to_owned(),
                                value: LiteralValue::Bool(false),
                            },
                        ],
                    }),
                },
            ]),
        );
        surfaces.insert(
            SurfaceSlot::Settings,
            stack(vec![
                UiNode::Text {
                    variant: TextVariant::Title,
                    value: literal("Provider settings"),
                    tone: None,
                },
                UiNode::Text {
                    variant: TextVariant::Body,
                    value: literal(
                        "Runtime options are validated against the active Provider generation.",
                    ),
                    tone: None,
                },
                UiNode::Button {
                    label: literal("Upgrade"),
                    style: ButtonStyle::Secondary,
                    emit: MessageEmission {
                        message: "upgrade".to_owned(),
                        payload: BTreeMap::new(),
                    },
                    enabled_when: Some(BoolExpression::All {
                        values: vec![
                            BoolExpression::HostEquals {
                                field: HostBoolField::MachineOnline,
                                value: true,
                            },
                            BoolExpression::HostEquals {
                                field: HostBoolField::UpgradeAvailable,
                                value: true,
                            },
                            BoolExpression::StateEquals {
                                field: "operation_pending".to_owned(),
                                value: LiteralValue::Bool(false),
                            },
                        ],
                    }),
                },
                button("Uninstall", "request_uninstall", ButtonStyle::Destructive),
            ]),
        );
        surfaces.insert(
            SurfaceSlot::Information,
            stack(vec![
                icon(),
                identity(),
                UiNode::Text {
                    variant: TextVariant::Code,
                    value: TextValue::Host {
                        field: HostTextField::ProviderVersion,
                    },
                    tone: None,
                },
            ]),
        );
        surfaces.insert(
            SurfaceSlot::Empty,
            stack(vec![
                icon(),
                UiNode::Text {
                    variant: TextVariant::Body,
                    value: literal("Install this Provider on a Machine to start a session."),
                    tone: None,
                },
                button("Install", "install", ButtonStyle::Primary),
            ]),
        );
        surfaces.insert(
            SurfaceSlot::Loading,
            stack(vec![
                UiNode::Asset {
                    asset: loading_asset.clone(),
                    size: AssetSize::Md,
                },
                UiNode::Progress {
                    label: literal("Working"),
                },
            ]),
        );
        surfaces.insert(
            SurfaceSlot::Error,
            UiNode::Alert {
                tone: Tone::Error,
                title: literal("Provider unavailable"),
                body: TextValue::Host {
                    field: HostTextField::ErrorDetail,
                },
            },
        );
        surfaces.insert(
            SurfaceSlot::Session,
            stack(vec![
                icon(),
                identity(),
                UiNode::Badge {
                    label: TextValue::Host {
                        field: HostTextField::AuthenticationState,
                    },
                    tone: Tone::Neutral,
                },
            ]),
        );
        let messages = [
            ("authenticate", BTreeMap::new()),
            ("logout", BTreeMap::new()),
            ("install", BTreeMap::new()),
            ("upgrade", BTreeMap::new()),
            ("request_uninstall", BTreeMap::new()),
            ("operation_succeeded", BTreeMap::new()),
            (
                "operation_failed",
                BTreeMap::from([("detail".to_owned(), ValueType::String)]),
            ),
        ]
        .into_iter()
        .map(|(id, payload)| MessageSchema {
            id: id.to_owned(),
            payload,
        })
        .collect();
        let effects = [
            ("authenticate", EffectCapability::BeginServiceAuthentication),
            ("logout", EffectCapability::LogoutServiceAuthentication),
            ("install", EffectCapability::InstallOnMachine),
            ("upgrade", EffectCapability::UpgradeOnMachine),
            ("request_uninstall", EffectCapability::RequestUninstallPlan),
        ]
        .into_iter()
        .map(|(id, capability)| EffectSchema {
            id: id.to_owned(),
            capability,
            request: BTreeMap::new(),
            success_message: "operation_succeeded".to_owned(),
            failure_message: "operation_failed".to_owned(),
        })
        .collect::<Vec<_>>();
        let mut reducers = effects
            .iter()
            .map(|effect| ReducerRule {
                message: effect.id.clone(),
                assignments: vec![
                    StateAssignment {
                        field: "operation_pending".to_owned(),
                        value: AssignmentValue::Literal {
                            value: LiteralValue::Bool(true),
                        },
                    },
                    StateAssignment {
                        field: "last_error".to_owned(),
                        value: AssignmentValue::Literal {
                            value: LiteralValue::String(String::new()),
                        },
                    },
                ],
                effect: Some(effect.id.clone()),
            })
            .collect::<Vec<_>>();
        reducers.extend([
            ReducerRule {
                message: "operation_succeeded".to_owned(),
                assignments: vec![StateAssignment {
                    field: "operation_pending".to_owned(),
                    value: AssignmentValue::Literal {
                        value: LiteralValue::Bool(false),
                    },
                }],
                effect: None,
            },
            ReducerRule {
                message: "operation_failed".to_owned(),
                assignments: vec![
                    StateAssignment {
                        field: "operation_pending".to_owned(),
                        value: AssignmentValue::Literal {
                            value: LiteralValue::Bool(false),
                        },
                    },
                    StateAssignment {
                        field: "last_error".to_owned(),
                        value: AssignmentValue::Message {
                            field: "detail".to_owned(),
                        },
                    },
                ],
                effect: None,
            },
        ]);
        let assets = vec![
            asset(logo_asset.clone(), AssetRole::Logo),
            asset(icon_asset.clone(), AssetRole::Icon),
            asset(loading_asset, AssetRole::Loading),
        ];
        Ok(ProviderManifest {
            id: self.id,
            version: self.version,
            publisher: self.publisher,
            sdk_version: self.sdk_version,
            display: ProviderDisplay {
                name: self.display.name,
                vendor: self.display.vendor,
                summary: self.display.summary,
                accent: self.display.accent,
                secondary_accent: self.display.secondary_accent,
                logo_asset: logo_asset.clone(),
                icon_asset: icon_asset.clone(),
            },
            ui: UiContract {
                schema_version: UI_SCHEMA_VERSION,
                assets,
                surfaces,
            },
            logic: LogicContract {
                schema_version: LOGIC_SCHEMA_VERSION,
                state: vec![
                    StateField {
                        id: "last_error".to_owned(),
                        value_type: ValueType::String,
                        initial: LiteralValue::String(String::new()),
                    },
                    StateField {
                        id: "operation_pending".to_owned(),
                        value_type: ValueType::Bool,
                        initial: LiteralValue::Bool(false),
                    },
                ],
                messages,
                reducers,
                effects,
            },
            configuration: ConfigurationUiContract {
                schema_version: 1,
                presets: self.configuration_presets,
                options: self.configuration_options,
            },
            host: self.host,
            runtime: self.runtime,
            authentication: self.authentication,
            compatibility: self.compatibility,
        })
    }
}

/// Fingerprint every interface surface whose drift changes host behavior.
///
/// # Errors
/// Returns an error when the contract graph cannot be serialized.
pub fn contract_fingerprint(manifest: &ProviderManifest) -> Result<String> {
    fingerprint_json(&serde_json::json!({
        "sdk_version": manifest.sdk_version,
        "ui": manifest.ui,
        "logic": manifest.logic,
        "configuration": manifest.configuration,
        "host": manifest.host,
        "runtime": manifest.runtime,
        "authentication": manifest.authentication,
    }))
}

/// Serialize a typed value and return its SHA-256 content identity.
///
/// # Errors
/// Returns an error when the value cannot be serialized.
pub fn fingerprint_json(value: &impl Serialize) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

fn validate_display(display: &ProviderDisplay) -> Result<()> {
    for (name, value) in [
        ("display name", &display.name),
        ("vendor", &display.vendor),
        ("summary", &display.summary),
    ] {
        ensure!(
            !value.trim().is_empty() && value.len() <= 512,
            "invalid {name}"
        );
    }
    validate_color(&display.accent, "accent color")?;
    validate_color(&display.secondary_accent, "secondary accent color")?;
    validate_id(&display.logo_asset, "logo asset")?;
    validate_id(&display.icon_asset, "icon asset")
}

fn validate_node(
    node: &UiNode,
    assets: &BTreeSet<&str>,
    state: &BTreeMap<&str, &ValueType>,
    messages: &BTreeMap<&str, &MessageSchema>,
    depth: usize,
    nodes: &mut usize,
) -> Result<()> {
    ensure!(depth <= 32, "Provider UI is too deeply nested");
    *nodes = nodes.saturating_add(1);
    match node {
        UiNode::Stack {
            children,
            visible_when,
            ..
        } => {
            ensure!(children.len() <= 64, "UI stack exceeds 64 children");
            if let Some(expression) = visible_when {
                validate_expression(expression, state, 0)?;
            }
            for child in children {
                validate_node(
                    child,
                    assets,
                    state,
                    messages,
                    depth.saturating_add(1),
                    nodes,
                )?;
            }
        }
        UiNode::Text { value, .. }
        | UiNode::Badge { label: value, .. }
        | UiNode::Progress { label: value } => validate_text(value, state)?,
        UiNode::Asset { asset, .. } => ensure!(
            assets.contains(asset.as_str()),
            "UI references unknown asset {asset}"
        ),
        UiNode::Alert { title, body, .. } => {
            validate_text(title, state)?;
            validate_text(body, state)?;
        }
        UiNode::Divider => {}
        UiNode::Button {
            label,
            emit,
            enabled_when,
            ..
        } => {
            validate_text(label, state)?;
            if let Some(expression) = enabled_when {
                validate_expression(expression, state, 0)?;
            }
            let schema = messages
                .get(emit.message.as_str())
                .context("button emits unknown message")?;
            ensure!(
                schema.payload.len() == emit.payload.len(),
                "button message payload shape mismatch"
            );
            for (field, value_type) in &schema.payload {
                let value = emit
                    .payload
                    .get(field)
                    .context("button message payload field is missing")?;
                ensure!(
                    literal_matches(value, value_type),
                    "button message payload type mismatch"
                );
            }
        }
    }
    Ok(())
}

fn validate_text(value: &TextValue, state: &BTreeMap<&str, &ValueType>) -> Result<()> {
    match value {
        TextValue::Literal { value } => ensure!(value.len() <= 4_096, "UI text is too long"),
        TextValue::State { field } => ensure!(
            state.contains_key(field.as_str()),
            "text references unknown state"
        ),
        TextValue::Host { .. } => {}
    }
    Ok(())
}

fn validate_expression(
    expression: &BoolExpression,
    state: &BTreeMap<&str, &ValueType>,
    depth: usize,
) -> Result<()> {
    ensure!(depth <= 16, "Provider UI expression is too deeply nested");
    match expression {
        BoolExpression::StateEquals { field, value } => {
            let value_type = state
                .get(field.as_str())
                .context("expression references unknown state")?;
            ensure!(
                literal_matches(value, value_type),
                "expression state comparison type mismatch"
            );
        }
        BoolExpression::HostEquals { .. } => {}
        BoolExpression::All { values } | BoolExpression::Any { values } => {
            ensure!(
                !values.is_empty() && values.len() <= 32,
                "invalid boolean expression width"
            );
            for value in values {
                validate_expression(value, state, depth.saturating_add(1))?;
            }
        }
        BoolExpression::Not { value } => {
            validate_expression(value, state, depth.saturating_add(1))?;
        }
    }
    Ok(())
}

fn validate_view_box(value: &str) -> Result<()> {
    let values: Vec<_> = value
        .split_ascii_whitespace()
        .map(str::parse::<f64>)
        .collect::<std::result::Result<_, _>>()
        .context("vector asset viewBox is invalid")?;
    ensure!(
        values.len() == 4
            && values.iter().all(|value| value.is_finite())
            && values[2] > 0.0
            && values[3] > 0.0,
        "vector asset viewBox is invalid"
    );
    Ok(())
}

fn validate_vector_path(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty()
            && value.len() <= 65_536
            && value.bytes().all(|byte| {
                byte.is_ascii_digit()
                    || byte.is_ascii_whitespace()
                    || matches!(
                        byte,
                        b'M' | b'm'
                            | b'Z'
                            | b'z'
                            | b'L'
                            | b'l'
                            | b'H'
                            | b'h'
                            | b'V'
                            | b'v'
                            | b'C'
                            | b'c'
                            | b'S'
                            | b's'
                            | b'Q'
                            | b'q'
                            | b'T'
                            | b't'
                            | b'A'
                            | b'a'
                            | b'E'
                            | b'e'
                            | b'+'
                            | b'-'
                            | b'.'
                            | b','
                    )
            }),
        "vector asset path is invalid"
    );
    Ok(())
}

fn validate_color(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid {label}"
    );
    Ok(())
}

fn validate_id(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 128 && value != "." && value != "..",
        "{label} is empty or too long"
    );
    ensure!(
        value.bytes().all(|byte| byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'-' | b'_' | b'.')),
        "{label} contains invalid characters"
    );
    Ok(())
}

fn validate_exact_version(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 128,
        "{label} is empty or too long"
    );
    ensure!(
        !value.contains(['*', '^', '~', '>', '<', '=']) && !value.eq_ignore_ascii_case("latest"),
        "{label} is not an exact pin"
    );
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+')),
        "{label} contains invalid characters"
    );
    Ok(())
}

fn validate_semantic_version(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() <= 128 && semver::Version::parse(value).is_ok(),
        "{label} is not an exact semantic version"
    );
    Ok(())
}

fn validate_provider_sdk_version(value: &str) -> Result<()> {
    validate_semantic_version(value, "Provider SDK version")?;
    let provider = semver::Version::parse(value).context("parsing Provider SDK version")?;
    let supported = semver::Version::parse(PROVIDER_SDK_VERSION)
        .context("parsing Cowboy Provider SDK version")?;
    ensure!(
        provider.major == supported.major && provider <= supported,
        "Provider SDK version {provider} is incompatible with Cowboy Provider SDK {supported}"
    );
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        bail!("{label} must use sha256");
    };
    ensure!(
        hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{label} is malformed"
    );
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    ensure!(
        !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "credential path is unsafe"
    );
    Ok(())
}

fn validate_https_url(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.starts_with("https://")
            && value.len() <= 2_048
            && !value
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte == 0),
        "invalid {label}"
    );
    Ok(())
}

fn validate_runtime_artifact_url(value: &str) -> Result<()> {
    let loopback = value.starts_with("http://127.0.0.1/")
        || value.starts_with("http://127.0.0.1:")
        || value.starts_with("http://localhost/")
        || value.starts_with("http://localhost:")
        || value.starts_with("http://[::1]/")
        || value.starts_with("http://[::1]:");
    ensure!(
        (value.starts_with("https://") || loopback)
            && value.len() <= 2_048
            && !value.contains("latest")
            && !value
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte == 0),
        "invalid released private component artifact URL"
    );
    Ok(())
}

fn valid_environment_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_uppercase())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn literal_matches(value: &LiteralValue, value_type: &ValueType) -> bool {
    matches!(
        (value, value_type),
        (LiteralValue::String(_), ValueType::String)
            | (LiteralValue::Bool(_), ValueType::Bool)
            | (LiteralValue::Integer(_), ValueType::Integer)
    )
}

fn platform_payload_fingerprint(
    payload: &PlatformPayload,
    dependencies: &[ExactDependency],
) -> Result<String> {
    fingerprint_json(&serde_json::json!({
        "os": payload.os,
        "architecture": payload.architecture,
        "launch_command": payload.launch_command,
        "private_components": payload.private_components,
        "dependencies": dependencies,
    }))
}

fn validate_dependency_integrity(value: &str) -> Result<()> {
    if value.starts_with("sha256:") {
        return validate_digest(value, "dependency integrity");
    }
    let encoded = value
        .strip_prefix("sha512-")
        .context("dependency integrity is not immutable")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("dependency integrity is invalid base64")?;
    ensure!(
        decoded.len() == 64,
        "dependency SHA-512 integrity has the wrong length"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ranges_are_rejected() {
        assert!(validate_exact_version("^1.2.3", "dependency").is_err());
        assert!(validate_exact_version("latest", "dependency").is_err());
        assert!(validate_exact_version("1.2.3", "dependency").is_ok());
        assert!(validate_id(".", "Provider id").is_err());
        assert!(validate_id("..", "Provider id").is_err());
    }

    #[test]
    fn provider_sdk_versions_are_compatibility_checked() {
        assert!(validate_provider_sdk_version(PROVIDER_SDK_VERSION).is_ok());
        assert!(validate_provider_sdk_version("1.99.0").is_err());
        assert!(validate_provider_sdk_version("2.0.1").is_ok());
        assert!(validate_provider_sdk_version("2.1.1").is_err());
        assert!(validate_provider_sdk_version("3.0.0").is_err());
    }

    #[test]
    fn release_proof_binds_the_immutable_identity() {
        let release = ProviderRelease {
            release_schema: RELEASE_SCHEMA_VERSION,
            provider_id: "codex".to_owned(),
            provider_version: "1.0.0".to_owned(),
            package_digest: format!("sha256:{}", "c".repeat(64)),
            artifact_digest: format!("sha256:{}", "a".repeat(64)),
            artifact_url: "https://example.invalid/codex".to_owned(),
            publisher: "cowboy".to_owned(),
            contract_fingerprint: format!("sha256:{}", "b".repeat(64)),
            signature: "signature".to_owned(),
            supported_platforms: Vec::new(),
            runtime_artifacts: Vec::new(),
        };
        let mut changed = release.clone();
        changed.provider_version = "1.0.1".to_owned();
        assert_ne!(release.proof(), changed.proof());
        assert!(
            !String::from_utf8(release.proof())
                .unwrap()
                .contains(&release.artifact_url)
        );
    }

    #[test]
    fn every_first_party_provider_builds_independently_and_deterministically() {
        let sources = [
            include_str!("../../../providers/claude-code/provider.json"),
            include_str!("../../../providers/codex/provider.json"),
            include_str!("../../../providers/gemini/provider.json"),
            include_str!("../../../providers/grok/provider.json"),
            include_str!("../../../providers/claude-deepseek/provider.json"),
            include_str!("../../../providers/codex-deepseek/provider.json"),
        ];
        let mut ids = BTreeSet::new();
        for source in sources {
            let source: StandardProviderSource = serde_json::from_str(source).unwrap();
            let first = build_package(source.clone().compile().unwrap()).unwrap();
            let second = build_package(source.compile().unwrap()).unwrap();
            assert_eq!(first.manifest.sdk_version, PROVIDER_SDK_VERSION);
            assert_eq!(
                first.canonical_bytes().unwrap(),
                second.canonical_bytes().unwrap()
            );
            assert!(ids.insert(first.manifest.id.clone()));
            assert_eq!(first.manifest.ui.surfaces.len(), REQUIRED_SURFACES.len());
            assert!(
                !first
                    .manifest
                    .compatibility
                    .ui_component_fingerprint
                    .is_empty()
            );
            assert!(
                !first
                    .manifest
                    .compatibility
                    .auth_contract_fingerprint
                    .is_empty()
            );
        }
        assert_eq!(ids.len(), 6);
        assert!(ids.contains("gemini"));
    }

    #[test]
    fn provider_owns_configuration_and_tool_ui_policy() {
        let source: StandardProviderSource = serde_json::from_str(include_str!(
            "../../../providers/claude-deepseek/provider.json"
        ))
        .unwrap();
        let mut manifest = source.clone().compile().unwrap();
        let context = manifest
            .configuration
            .options
            .iter()
            .find(|option| option.id == "deepseek_context")
            .cloned()
            .unwrap();
        assert_eq!(context.layout, ConfigurationOptionLayout::FullWidth);
        assert_eq!(
            context.availability,
            ConfigurationOptionAvailability::IdleOrStopped
        );

        manifest.configuration.options.push(context);
        assert!(build_package(manifest).is_err());

        let mut manifest = source.compile().unwrap();
        let tool = manifest.host.tool_presentations[0].clone();
        assert_eq!(tool.tool_name, "TodoWrite");
        assert_eq!(tool.renderer, ToolRenderer::TodoListV1);
        manifest.host.tool_presentations.push(tool);
        assert!(build_package(manifest).is_err());
    }

    #[test]
    fn gateway_runtime_links_are_closed_and_platform_complete() {
        let source: StandardProviderSource = serde_json::from_str(include_str!(
            "../../../providers/codex-deepseek/provider.json"
        ))
        .unwrap();
        let mut manifest = source.clone().compile().unwrap();
        let sidecar = &manifest.runtime.sidecars[0];
        assert_eq!(sidecar.id, "deepseek-gateway");
        assert_eq!(
            sidecar.component.kind,
            PrivateComponentKind::ProviderGateway
        );
        assert!(sidecar.auth_environment.contains("DEEPSEEK_API_KEY"));
        assert!(manifest.runtime.arguments.iter().any(|value| matches!(
            value,
            RuntimeValue::Binding(RuntimeBinding::SidecarUrl { sidecar, .. })
                if sidecar == "deepseek-gateway"
        )));
        let runtime_json = serde_json::to_string(&manifest.runtime).unwrap();
        assert!(!runtime_json.contains("127.0.0.1:61137"));
        assert!(!runtime_json.contains("/nix/"));
        build_package(manifest.clone()).unwrap();

        if let RuntimeValue::Binding(RuntimeBinding::SidecarUrl { sidecar, .. }) = manifest
            .runtime
            .arguments
            .iter_mut()
            .find(|value| {
                matches!(
                    value,
                    RuntimeValue::Binding(RuntimeBinding::SidecarUrl { .. })
                )
            })
            .unwrap()
        {
            *sidecar = "missing-gateway".to_owned();
        }
        assert!(build_package(manifest).is_err());

        let mut missing_auth = source.compile().unwrap();
        missing_auth.authentication.environment_projection.clear();
        assert!(build_package(missing_auth).is_err());

        let claude: StandardProviderSource = serde_json::from_str(include_str!(
            "../../../providers/claude-deepseek/provider.json"
        ))
        .unwrap();
        let claude = build_package(claude.compile().unwrap()).unwrap();
        assert!(claude.manifest.runtime.environment.values().any(|value| {
            matches!(
                value,
                RuntimeValue::Binding(RuntimeBinding::ComponentCommand { component, .. })
                    if component.kind == PrivateComponentKind::ProviderCli
                        && component.slot == "claude"
            )
        }));
        assert!(
            claude
                .manifest
                .authentication
                .credential_files
                .iter()
                .any(|file| {
                    file.bundle_key == "api_key"
                        && file.relative_path == ".config/credentials/deepseek-claude-api-key"
                })
        );
        assert!(
            claude
                .manifest
                .authentication
                .environment_projection
                .is_empty()
        );
    }

    #[test]
    fn downloaded_artifact_rejects_contract_and_ui_link_tampering() {
        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../../../providers/gemini/provider.json")).unwrap();
        let package = build_package(source.compile().unwrap()).unwrap();
        let mut bad_contract = package.clone();
        bad_contract.contract_fingerprint = format!("sha256:{}", "0".repeat(64));
        assert!(ProviderPackage::from_bytes(&serde_json::to_vec(&bad_contract).unwrap()).is_err());

        let mut bad_ui = package;
        bad_ui.manifest.ui.surfaces.insert(
            SurfaceSlot::Card,
            UiNode::Asset {
                asset: "undeclared".to_owned(),
                size: AssetSize::Md,
            },
        );
        assert!(ProviderPackage::from_bytes(&serde_json::to_vec(&bad_ui).unwrap()).is_err());
    }

    #[test]
    fn release_requires_a_complete_exact_runtime_binding() {
        let package = first_party_package("../../../providers/gemini/provider.json");
        let bytes = package.canonical_bytes().unwrap();
        let release = complete_release(&package, &bytes);
        release.validate_bytes(&bytes).unwrap();

        let mut missing_target = release.clone();
        missing_target.runtime_artifacts.pop();
        missing_target.artifact_digest = missing_target.computed_artifact_digest().unwrap();
        assert!(missing_target.validate_bytes(&bytes).is_err());

        let mut wrong_version = release.clone();
        wrong_version.runtime_artifacts[0].components[0].version = "999.0.0".to_owned();
        wrong_version.artifact_digest = wrong_version.computed_artifact_digest().unwrap();
        assert!(wrong_version.validate_bytes(&bytes).is_err());

        let mut changed_runtime = release.clone();
        changed_runtime.runtime_artifacts[0].components[0].artifact_digest =
            format!("sha256:{}", "d".repeat(64));
        changed_runtime.artifact_digest = changed_runtime.computed_artifact_digest().unwrap();
        assert_ne!(release.artifact_digest, changed_runtime.artifact_digest);
        assert_ne!(release.proof(), changed_runtime.proof());
    }

    #[test]
    fn authoring_rejects_ambiguous_defaults_and_unsafe_error_retries() {
        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../../../providers/codex/provider.json")).unwrap();
        let mut ambiguous = source.clone();
        let mut duplicate = ambiguous.configuration_presets[0].clone();
        duplicate.id = "also-default".to_owned();
        ambiguous.configuration_presets.push(duplicate);
        assert!(build_package(ambiguous.compile().unwrap()).is_err());

        let mut unsafe_retry = source;
        unsafe_retry
            .runtime
            .behavior
            .error_rules
            .push(RuntimeErrorRule {
                when: TextMatchExpression::Contains {
                    value: "retry me".to_owned(),
                },
                user_detail: None,
                classification: None,
                keep_worker_alive: false,
                retry_once_without_visible_update: true,
            });
        assert!(build_package(unsafe_retry.compile().unwrap()).is_err());
    }

    #[test]
    fn lifecycle_effects_cannot_escape_their_typed_ui_surface() {
        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../../../providers/gemini/provider.json")).unwrap();
        let mut manifest = source.compile().unwrap();
        let service_auth = manifest
            .ui
            .surfaces
            .get(&SurfaceSlot::Setup)
            .unwrap()
            .clone();
        manifest
            .ui
            .surfaces
            .insert(SurfaceSlot::Settings, service_auth);
        assert!(
            build_package(manifest)
                .unwrap_err()
                .to_string()
                .contains("not allowed on Settings surface")
        );
    }

    #[test]
    fn authoring_rejects_auth_and_host_contract_drift() {
        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../../../providers/claude-code/provider.json"))
                .unwrap();
        let mut missing_file = source.clone();
        missing_file.authentication.credential_files.clear();
        assert!(build_package(missing_file.compile().unwrap()).is_err());

        let mut incompatible = source.clone();
        incompatible.compatibility.min_machine_contract = MACHINE_CONTRACT_VERSION + 1;
        incompatible.compatibility.max_machine_contract = MACHINE_CONTRACT_VERSION + 1;
        assert!(build_package(incompatible.compile().unwrap()).is_err());

        let mut unknown_profile = serde_json::to_value(source).unwrap();
        unknown_profile["runtime"]["behavior"]["permission"] =
            serde_json::Value::String("provider_specific_magic".to_owned());
        assert!(serde_json::from_value::<StandardProviderSource>(unknown_profile).is_err());
    }

    fn first_party_package(path: &str) -> ProviderPackage {
        let source = match path {
            "../../../providers/gemini/provider.json" => {
                include_str!("../../../providers/gemini/provider.json")
            }
            other => panic!("unknown test Provider source {other}"),
        };
        let source: StandardProviderSource = serde_json::from_str(source).unwrap();
        build_package(source.compile().unwrap()).unwrap()
    }

    fn complete_release(package: &ProviderPackage, bytes: &[u8]) -> ProviderRelease {
        let runtime_artifacts = package
            .manifest
            .runtime
            .platforms
            .iter()
            .map(|payload| PlatformRuntimeArtifacts {
                os: payload.os.clone(),
                architecture: payload.architecture.clone(),
                components: payload
                    .private_components
                    .iter()
                    .map(|requirement| {
                        let dependency = package
                            .manifest
                            .runtime
                            .dependencies
                            .iter()
                            .find(|dependency| dependency.id == requirement.dependency)
                            .unwrap();
                        ReleasedPrivateComponent {
                            kind: requirement.kind.clone(),
                            slot: requirement.slot.clone(),
                            dependency: requirement.dependency.clone(),
                            version: dependency.version.clone(),
                            command: requirement.command.clone(),
                            artifact_url: format!(
                                "https://example.invalid/{}/{}",
                                requirement.dependency, dependency.version
                            ),
                            artifact_digest: format!("sha256:{}", "a".repeat(64)),
                            artifact_format: ProviderArtifactFormat::Raw,
                            entrypoint: None,
                            probe: ProviderArtifactProbe {
                                args: vec!["--version".to_owned()],
                                timeout_ms: 1_000,
                            },
                        }
                    })
                    .collect(),
            })
            .collect();
        let mut release = ProviderRelease {
            release_schema: RELEASE_SCHEMA_VERSION,
            provider_id: package.manifest.id.clone(),
            provider_version: package.manifest.version.clone(),
            package_digest: ProviderPackage::artifact_digest(bytes),
            artifact_digest: String::new(),
            artifact_url: "https://example.invalid/provider.cowboy-provider".to_owned(),
            publisher: package.manifest.publisher.clone(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            signature: "fixture-signature".to_owned(),
            supported_platforms: package
                .manifest
                .runtime
                .platforms
                .iter()
                .map(|payload| PlatformTarget {
                    os: payload.os.clone(),
                    architecture: payload.architecture.clone(),
                })
                .collect(),
            runtime_artifacts,
        };
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        release
    }
}
