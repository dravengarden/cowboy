//! Shared, versioned Plugin host contract.
//!
//! Capability payloads (`provider.json`, OIDC authentication, Zed) stay in the
//! Plugin SDK. This module is the business-agnostic host surface: slots, UI
//! renderer selection, and plugin-owned storage. A plugin may ship `host.json` next to its
//! generation tree; missing host metadata means "no host UI/storage", not an
//! error.

#![warn(clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::CliAuthRuleSet;

pub const PLUGIN_HOST_SCHEMA_VERSION: u16 = 1;
pub const PLUGIN_HOST_API_VERSION: &str = "1.0.0";
pub const PLUGIN_RENDERER_SCHEMA_VERSION: u16 = 1;
pub const PLUGIN_NATIVE_HOST_API_VERSION: &str = "1.0.0";
const MAX_MIGRATION_BYTES: usize = 1024 * 1024;
const MAX_MIGRATIONS: usize = 64;
const MAX_SQL_STATEMENTS: usize = 128;

/// Closed slot identifiers the Cowboy shell knows how to mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginSlotId {
    #[serde(rename = "login.method")]
    LoginMethod,
    #[serde(rename = "account.panel")]
    AccountPanel,
    #[serde(rename = "provider.card")]
    ProviderCard,
    #[serde(rename = "provider.setup")]
    ProviderSetup,
    #[serde(rename = "provider.settings")]
    ProviderSettings,
    #[serde(rename = "provider.usage")]
    ProviderUsage,
    #[serde(rename = "provider.empty")]
    ProviderEmpty,
    #[serde(rename = "code.intelligence")]
    CodeIntelligence,
}

impl PluginSlotId {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LoginMethod => "login.method",
            Self::AccountPanel => "account.panel",
            Self::ProviderCard => "provider.card",
            Self::ProviderSetup => "provider.setup",
            Self::ProviderSettings => "provider.settings",
            Self::ProviderUsage => "provider.usage",
            Self::ProviderEmpty => "provider.empty",
            Self::CodeIntelligence => "code.intelligence",
        }
    }

    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::LoginMethod,
            Self::AccountPanel,
            Self::ProviderCard,
            Self::ProviderSetup,
            Self::ProviderSettings,
            Self::ProviderUsage,
            Self::ProviderEmpty,
            Self::CodeIntelligence,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginHostSpec {
    pub schema_version: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<PluginSlotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<PluginUiSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<PluginStorageSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<PluginUsageSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loopback_origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loopback_detail: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub loopback_requires_catalog: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loopback_catalog: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loopback_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_slot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_executable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_executable_env: Option<String>,
    #[serde(default, skip_serializing_if = "is_none_cli_auth")]
    pub cli_auth: PluginCliAuthKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cli_auth_argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_auth_rules: Option<CliAuthRuleSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_fields: Option<PluginLoginFields>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rpc_argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual: Option<PluginVisualSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolated_home_env: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub isolated_home_settings: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub isolated_shell: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isolated_shell_env: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginVisualSpec {
    pub light: PluginVisualPair,
    pub dark: PluginVisualPair,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginVisualPair {
    pub primary: String,
    pub secondary: String,
}

/// Plugin-declared CLI authentication probe.
///
/// `none` and `exit` are host primitives. Other values remain opaque plugin
/// identifiers; their probe implementation is selected by the installed
/// plugin generation rather than by a closed host enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginCliAuthKind(String);

impl Default for PluginCliAuthKind {
    fn default() -> Self {
        Self("none".to_owned())
    }
}

impl PluginCliAuthKind {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_none(&self) -> bool {
        self.0 == "none"
    }

    #[must_use]
    pub fn is_exit(&self) -> bool {
        self.0 == "exit"
    }
}

fn is_none_cli_auth(kind: &PluginCliAuthKind) -> bool {
    kind.is_none()
}

/// Plugin-declared usage collector identifier.
///
/// `session` and `command` are the two host primitives. Any other value is
/// retained as an opaque plugin identifier so a new collector can be shipped
/// without changing Cowboy's enum or matching a new Provider id in core.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UsageCollectorKind {
    #[default]
    Session,
    Command,
    Named(String),
}

impl Serialize for UsageCollectorKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UsageCollectorKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "session" => Self::Session,
            "command" => Self::Command,
            _ => Self::Named(value),
        })
    }
}

impl UsageCollectorKind {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Session => "session",
            Self::Command => "command",
            Self::Named(value) => value,
        }
    }

    #[must_use]
    pub fn is_session(&self) -> bool {
        matches!(self, Self::Session)
    }

    #[must_use]
    pub fn is_command(&self) -> bool {
        matches!(self, Self::Command)
    }
}

macro_rules! plugin_string_kind {
    ($name:ident, $default:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl Default for $name {
            fn default() -> Self {
                Self($default.to_owned())
            }
        }

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[allow(dead_code)]
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn is_default(&self) -> bool {
                self.0 == $default
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }
    };
}

plugin_string_kind!(UsageLimitParserKind, "generic-buckets");
plugin_string_kind!(UsageErrorKind, "raw");
plugin_string_kind!(UsageWidgetKind, "none");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageWidgetShape {
    #[default]
    None,
    Percent,
    Balance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageResetClaim {
    #[default]
    AfterSuccess,
    BeforeAttempt,
}

plugin_string_kind!(UsageSessionOverlay, "none");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginUsageSpec {
    pub account: String,
    #[serde(default, skip_serializing_if = "is_session_collector")]
    pub collector: UsageCollectorKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collector_argv: Vec<String>,
    /// Optional plugin-owned reset command. It receives a JSON reset request
    /// on stdin and returns a JSON `ResetResult` on stdout.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reset_argv: Vec<String>,
    /// Exact Machine-local Provider sidecars required by this collector. The
    /// Machine resolves each adapter slot to an active signed generation and
    /// exposes only the declared loopback endpoint to the collector.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collector_sidecars: Vec<PluginUsageSidecar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, skip_serializing_if = "is_generic_parser")]
    pub parser: UsageLimitParserKind,
    #[serde(default, skip_serializing_if = "is_raw_error")]
    pub error: UsageErrorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_auth: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_config: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_fetch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_bar_windows: Vec<u32>,
    #[serde(default, skip_serializing_if = "is_none_widget")]
    pub widget: UsageWidgetKind,
    #[serde(default, skip_serializing_if = "is_default_value")]
    pub widget_shape: UsageWidgetShape,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_window: Option<u32>,
    #[serde(default, skip_serializing_if = "is_default_value")]
    pub reset_claim: UsageResetClaim,
    #[serde(default, skip_serializing_if = "is_none_session_overlay")]
    pub session_overlay: UsageSessionOverlay,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_rate_limits: Option<UsageSessionRateLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_status: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub omit_empty_limits: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_id_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limit_labels: Vec<UsageLimitLabel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_balance_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_spend_label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity_agents: Vec<UsageActivityAgent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity_models: Vec<UsageActivityAgent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_protection: Option<UsageCacheProtection>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub activity: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginUsageSidecar {
    /// Collector-specific lane identifier included in the injected target
    /// map. It normally matches one `activity_agents` id.
    pub id: String,
    pub adapter_slot: String,
    pub sidecar: String,
    /// Absolute HTTP path served by the declared loopback sidecar.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginLoginFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
}

/// Id/label row used by `activity_agents` and `activity_models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageActivityAgent {
    pub id: String,
    pub label: String,
}

/// Prompt-cache protection thresholds shown by the host usage and session UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageCacheProtection {
    pub min_hit_tokens: u32,
    pub min_hit_label: String,
    pub interval_ms: u32,
    pub interval_label: String,
    pub option_name: String,
    pub option_description: String,
    pub option_on: String,
    pub option_off: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageLimitLabel {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<u32>,
}

/// Generic projection from the latest session usage JSON into account limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageSessionRateLimits {
    pub pointer: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_number_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_string_fields: Vec<String>,
}

fn is_session_collector(kind: &UsageCollectorKind) -> bool {
    kind.is_session()
}

fn is_generic_parser(kind: &UsageLimitParserKind) -> bool {
    kind.is_default()
}

fn is_raw_error(kind: &UsageErrorKind) -> bool {
    kind.is_default()
}

fn is_none_widget(kind: &UsageWidgetKind) -> bool {
    kind.is_default()
}

fn is_default_value<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

fn is_none_session_overlay(kind: &UsageSessionOverlay) -> bool {
    kind.is_default()
}

impl PluginUsageSpec {
    #[must_use]
    pub fn refreshable(&self) -> bool {
        !self.collector.is_session() || !self.collector_argv.is_empty()
    }

    #[must_use]
    pub fn product_label(&self) -> &str {
        self.product.as_deref().unwrap_or("Provider")
    }

    #[must_use]
    pub fn claims_reset_before_attempt(&self) -> bool {
        matches!(self.reset_claim, UsageResetClaim::BeforeAttempt)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginUiSpec {
    pub schema_version: u16,
    pub renderers: BTreeMap<PluginSlotId, PluginRendererId>,
}

/// Closed, Cowboy-owned renderers selectable by signed plugin data.
///
/// Plugins may choose one of these renderers for a compatible slot, but they
/// never supply JavaScript, React components, HTML, CSS, or DOM behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginRendererId {
    #[serde(rename = "login-password-v1")]
    LoginPasswordV1,
    #[serde(rename = "login-oidc-v1")]
    LoginOidcV1,
    #[serde(rename = "account-passkeys-v1")]
    AccountPasskeysV1,
    #[serde(rename = "provider-surface-v1")]
    ProviderSurfaceV1,
    #[serde(rename = "provider-usage-v1")]
    ProviderUsageV1,
    #[serde(rename = "provider-usage-activity-v1")]
    ProviderUsageActivityV1,
}

impl PluginRendererId {
    #[must_use]
    pub fn supports(self, slot: PluginSlotId) -> bool {
        match self {
            Self::LoginPasswordV1 | Self::LoginOidcV1 => {
                matches!(slot, PluginSlotId::LoginMethod)
            }
            Self::AccountPasskeysV1 => matches!(slot, PluginSlotId::AccountPanel),
            Self::ProviderSurfaceV1 => matches!(
                slot,
                PluginSlotId::ProviderCard
                    | PluginSlotId::ProviderSetup
                    | PluginSlotId::ProviderSettings
                    | PluginSlotId::ProviderEmpty
            ),
            Self::ProviderUsageV1 | Self::ProviderUsageActivityV1 => {
                matches!(slot, PluginSlotId::ProviderUsage)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginStorageSpec {
    pub postgres: PluginSqlMigrations,
    pub sqlite: PluginSqlMigrations,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSqlMigrations {
    pub migrations: Vec<PluginMigration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginMigration {
    pub version: String,
    pub sql: String,
}

impl PluginHostSpec {
    /// Parse and validate a host spec.
    ///
    /// # Errors
    /// Returns when schema, slots, renderer declarations, or storage migrations are invalid.
    pub fn from_json(value: &[u8]) -> Result<Self> {
        let spec: Self = serde_json::from_slice(value).context("parsing plugin host spec")?;
        spec.validate()?;
        Ok(spec)
    }

    /// Load `host.json` from a plugin generation or source directory.
    ///
    /// # Errors
    /// Returns when the file exists but is malformed or invalid.
    pub fn load_optional(plugin_root: &Path) -> Result<Option<Self>> {
        let path = plugin_root.join("host.json");
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .with_context(|| format!("reading plugin host spec {}", path.display()))?;
        Ok(Some(Self::from_json(&bytes)?))
    }

    /// # Errors
    /// Returns when any host field is outside the closed contract.
    #[allow(clippy::too_many_lines)] // One declarative schema validator keeps cross-field rules together.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == PLUGIN_HOST_SCHEMA_VERSION,
            "unsupported plugin host schema"
        );
        let mut slots = BTreeSet::new();
        for slot in &self.slots {
            ensure!(
                slots.insert(*slot),
                "duplicate plugin slot {}",
                slot.as_str()
            );
        }
        if let Some(ui) = &self.ui {
            ensure!(
                ui.schema_version == PLUGIN_RENDERER_SCHEMA_VERSION,
                "unsupported plugin renderer schema"
            );
            ensure!(
                !ui.renderers.is_empty(),
                "plugin renderer declaration is empty"
            );
            ensure!(
                ui.renderers.keys().copied().collect::<BTreeSet<_>>() == slots,
                "plugin renderers must exactly cover declared slots"
            );
            for (slot, renderer) in &ui.renderers {
                ensure!(
                    renderer.supports(*slot),
                    "plugin renderer is incompatible with slot {}",
                    slot.as_str()
                );
            }
        } else {
            ensure!(
                self.slots.is_empty(),
                "plugin slots require a data-only renderer declaration"
            );
        }
        ensure!(
            self.native_capabilities.len() <= 32,
            "too many plugin native capabilities"
        );
        let mut native_capabilities = BTreeSet::new();
        for capability in &self.native_capabilities {
            validate_native_capability(capability)?;
            ensure!(
                native_capabilities.insert(capability),
                "duplicate plugin native capability {capability}"
            );
        }
        if let Some(label) = &self.label {
            validate_host_label(label)?;
        }
        if let Some(origin) = &self.loopback_origin {
            validate_loopback_origin(origin)?;
        }
        if let Some(detail) = &self.loopback_detail {
            validate_host_label(detail).context("invalid loopback_detail")?;
        }
        if let Some(env) = &self.loopback_env {
            validate_isolated_home_env(env).context("invalid loopback_env")?;
        }
        if let Some(catalog) = &self.loopback_catalog {
            validate_loopback_catalog(catalog)?;
        }
        if self.loopback_requires_catalog {
            ensure!(
                self.loopback_catalog.is_some(),
                "loopback_requires_catalog needs loopback_catalog"
            );
        }
        if let Some(slot) = &self.adapter_slot {
            validate_plugin_id(slot)?;
        }
        if let Some(executable) = &self.cli_executable {
            validate_plugin_id(executable).context("invalid cli_executable")?;
        }
        if let Some(env) = &self.cli_executable_env {
            validate_isolated_home_env(env).context("invalid cli_executable_env")?;
        }
        validate_plugin_id(self.cli_auth.as_str()).context("invalid cli_auth identifier")?;
        if self.cli_auth.is_exit() {
            ensure!(
                !self.cli_auth_argv.is_empty(),
                "cli_auth exit needs cli_auth_argv"
            );
            validate_collector_argv(&self.cli_auth_argv)?;
            ensure!(
                self.cli_auth_rules.is_none(),
                "cli_auth exit cannot use cli_auth_rules"
            );
        } else {
            ensure!(
                self.cli_auth_argv.is_empty(),
                "cli_auth_argv is only valid for exit probes"
            );
            if let Some(rules) = &self.cli_auth_rules {
                ensure!(
                    !self.cli_auth.is_none(),
                    "cli_auth_rules needs a non-none cli_auth identifier"
                );
                rules.validate().context("invalid cli_auth_rules")?;
            }
        }
        validate_plugin_process_argv(&self.rpc_argv, "plugin rpc")?;
        if let Some(visual) = &self.visual {
            validate_visual(visual)?;
        }
        if let Some(env) = &self.isolated_home_env {
            validate_isolated_home_env(env)?;
        }
        ensure!(
            self.isolated_home_settings.is_empty() || self.isolated_home_env.is_some(),
            "isolated_home_settings needs isolated_home_env"
        );
        ensure!(
            serde_json::to_vec(&self.isolated_home_settings)?.len() <= 16 * 1024,
            "isolated_home_settings is too large"
        );
        for key in self.isolated_home_settings.keys() {
            ensure!(
                (1..=128).contains(&key.len())
                    && !key.contains('\0')
                    && key.chars().all(|ch| !ch.is_control()),
                "invalid isolated_home_settings key"
            );
        }
        for env in &self.isolated_shell_env {
            validate_isolated_home_env(env).context("invalid isolated_shell_env")?;
        }
        if let Some(fields) = &self.login_fields {
            if let Some(copy) = &fields.account {
                validate_usage_copy(copy, "login_fields.account")?;
            }
            if let Some(copy) = &fields.secret {
                validate_usage_copy(copy, "login_fields.secret")?;
            }
            if let Some(copy) = &fields.confirm {
                validate_usage_copy(copy, "login_fields.confirm")?;
            }
            if let Some(copy) = &fields.setup {
                validate_usage_copy(copy, "login_fields.setup")?;
            }
        }
        if let Some(usage) = &self.usage {
            validate_usage_account(&usage.account)?;
            for (value, field) in [
                (usage.collector.as_str(), "collector"),
                (usage.parser.as_str(), "parser"),
                (usage.error.as_str(), "error"),
                (usage.widget.as_str(), "widget"),
                (usage.session_overlay.as_str(), "session_overlay"),
            ] {
                validate_plugin_id(value)
                    .with_context(|| format!("invalid plugin usage {field} identifier"))?;
            }
            validate_plugin_process_argv(&usage.collector_argv, "plugin collector")?;
            validate_plugin_process_argv(&usage.reset_argv, "plugin reset")?;
            ensure!(
                usage.collector_sidecars.len() <= 16,
                "plugin usage declares too many collector sidecars"
            );
            let activity_agents = usage
                .activity_agents
                .iter()
                .map(|agent| agent.id.as_str())
                .collect::<BTreeSet<_>>();
            let mut sidecar_ids = BTreeSet::new();
            for target in &usage.collector_sidecars {
                validate_plugin_id(&target.id).context("invalid plugin usage sidecar id")?;
                validate_plugin_id(&target.adapter_slot)
                    .context("invalid plugin usage sidecar adapter slot")?;
                validate_plugin_id(&target.sidecar).context("invalid plugin usage sidecar name")?;
                ensure!(
                    activity_agents.contains(target.id.as_str()),
                    "plugin usage sidecar id is absent from activity_agents"
                );
                ensure!(
                    sidecar_ids.insert(target.id.as_str()),
                    "duplicate plugin usage sidecar id {}",
                    target.id
                );
                ensure!(
                    (1..=256).contains(&target.path.len())
                        && target.path.starts_with('/')
                        && !target.path.starts_with("//")
                        && target.path.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric()
                                || matches!(byte, b'/' | b'.' | b'_' | b'-' | b'~')
                        })
                        && !target.path.split('/').any(|part| part == ".."),
                    "invalid plugin usage sidecar path"
                );
            }
            ensure!(
                usage.collector_sidecars.is_empty()
                    || (!usage.collector_argv.is_empty() && usage.collector.is_command()),
                "plugin usage sidecars require a command collector"
            );
            if let Some(reset) = &usage.reset {
                validate_usage_account(reset)?;
            }
            validate_top_bar_windows(&usage.top_bar_windows)?;
            if let Some(window) = usage.widget_window {
                validate_top_bar_windows(&[window])?;
            }
            if let Some(empty) = &usage.empty {
                validate_usage_copy(empty, "empty")?;
            }
            if let Some(status) = &usage.available_status {
                validate_usage_copy(status, "available_status")?;
            }
            if let Some(copy) = &usage.error_auth {
                validate_usage_copy(copy, "error_auth")?;
            }
            if let Some(copy) = &usage.error_config {
                validate_usage_copy(copy, "error_config")?;
            }
            if let Some(copy) = &usage.error_fetch {
                validate_usage_copy(copy, "error_fetch")?;
            }
            if let Some(projection) = &usage.session_rate_limits {
                validate_session_rate_limits(projection)?;
            }
            if let Some(prefix) = &usage.limit_id_prefix {
                validate_limit_label_id(prefix, "limit_id_prefix")?;
            }
            validate_limit_labels(&usage.limit_labels)?;
            if let Some(copy) = &usage.widget_balance_label {
                validate_usage_copy(copy, "widget_balance_label")?;
            }
            if let Some(copy) = &usage.widget_spend_label {
                validate_usage_copy(copy, "widget_spend_label")?;
            }
            validate_activity_entries(&usage.activity_agents, "activity_agents")?;
            validate_activity_entries(&usage.activity_models, "activity_models")?;
            if let Some(protection) = &usage.cache_protection {
                validate_cache_protection(protection)?;
            }
        }
        if let Some(storage) = &self.storage {
            storage.postgres.validate("postgres")?;
            storage.sqlite.validate("sqlite")?;
        }
        Ok(())
    }

    /// Validate every generation-relative process entry against the exact
    /// signed host-bundle file set.
    ///
    /// # Errors
    /// Returns when a declared collector, reset, or RPC entry is absent from
    /// the bundle whose signature authorizes the command.
    pub fn validate_runtime_files(&self, files: &BTreeMap<String, String>) -> Result<()> {
        let process_argv = std::iter::once(&self.rpc_argv).chain(
            self.usage
                .iter()
                .flat_map(|usage| [&usage.collector_argv, &usage.reset_argv]),
        );
        for argv in process_argv {
            for entry in argv
                .iter()
                .filter_map(|argument| argument.strip_prefix("${PLUGIN_DIR}/"))
            {
                ensure!(
                    files.contains_key(entry),
                    "plugin host command references missing signed file {entry}"
                );
            }
        }
        Ok(())
    }
}

impl PluginSqlMigrations {
    /// Validate one dialect's ordered, bounded migration list.
    ///
    /// # Errors
    /// Returns when migration versions, SQL size, ordering, or statements are
    /// outside the Plugin storage contract.
    pub fn validate(&self, dialect: &str) -> Result<()> {
        ensure!(
            !self.migrations.is_empty(),
            "{dialect} plugin storage has no migrations"
        );
        ensure!(
            self.migrations.len() <= MAX_MIGRATIONS,
            "{dialect} plugin storage has too many migrations"
        );
        let mut versions = BTreeSet::new();
        let mut previous_version: Option<&str> = None;
        for migration in &self.migrations {
            ensure!(
                migration.version.len() == 4
                    && migration.version.bytes().all(|byte| byte.is_ascii_digit()),
                "{dialect} plugin migration version must be four digits"
            );
            ensure!(
                previous_version.is_none_or(|previous| previous < migration.version.as_str()),
                "{dialect} plugin migrations must be strictly ordered"
            );
            ensure!(
                versions.insert(migration.version.as_str()),
                "{dialect} plugin migration {} is duplicated",
                migration.version
            );
            ensure!(
                !migration.sql.is_empty() && migration.sql.len() <= MAX_MIGRATION_BYTES,
                "{dialect} plugin migration {} is empty or oversized",
                migration.version
            );
            ensure!(
                !migration.sql.contains('\0'),
                "{dialect} plugin migration {} contains NUL",
                migration.version
            );
            validate_plugin_sql(dialect, &migration.sql)?;
            previous_version = Some(&migration.version);
        }
        Ok(())
    }
}

fn validate_relative_entry(entry: &str) -> Result<()> {
    ensure!(
        !entry.is_empty()
            && !entry.starts_with('/')
            && !entry
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
            && entry
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric()
                    || matches!(byte, b'.' | b'_' | b'-' | b'/')),
        "invalid plugin relative entry"
    );
    Ok(())
}

fn validate_native_capability(value: &str) -> Result<()> {
    validate_plugin_id(value).with_context(|| format!("invalid plugin native capability {value}"))
}

const MAX_COLLECTOR_ARGV: usize = 32;

fn validate_collector_argv(argv: &[String]) -> Result<()> {
    ensure!(
        argv.len() <= MAX_COLLECTOR_ARGV,
        "plugin collector argv is too long"
    );
    for argument in argv {
        ensure!(
            (1..=256).contains(&argument.len())
                && !argument.contains('\0')
                && !argument.contains(".."),
            "invalid plugin collector argument"
        );
    }
    Ok(())
}

fn validate_plugin_process_argv(argv: &[String], label: &str) -> Result<()> {
    validate_collector_argv(argv)?;
    if argv.is_empty() {
        return Ok(());
    }
    ensure!(
        argv.first().is_some_and(|program| program == "@plugin-js"),
        "{label} must use the Cowboy Plugin JavaScript runtime"
    );
    ensure!(
        argv.get(1).is_some_and(|argument| argument == "run"),
        "{label} must use the Plugin JavaScript run command"
    );
    let mut signed_entry = None;
    for (index, argument) in argv.iter().enumerate() {
        let Some(relative) = argument.strip_prefix("${PLUGIN_DIR}/") else {
            ensure!(
                !argument.contains("${PLUGIN_DIR}"),
                "invalid {label} Plugin directory argument"
            );
            continue;
        };
        ensure!(
            signed_entry.is_none(),
            "{label} has multiple Plugin entries"
        );
        ensure!(
            relative.starts_with("collector/")
                && Path::new(relative)
                    .extension()
                    .and_then(std::ffi::OsStr::to_str)
                    == Some("js"),
            "{label} entry must be signed collector JavaScript"
        );
        validate_relative_entry(relative).with_context(|| format!("invalid {label} entry"))?;
        signed_entry = Some(index);
    }
    let signed_entry = signed_entry.context(format!("{label} needs a signed collector entry"))?;
    ensure!(
        signed_entry >= 2,
        "{label} signed collector entry precedes the runtime command"
    );
    let mut options = BTreeSet::new();
    for option in &argv[2..signed_entry] {
        let kind = validate_plugin_process_option(option, label)?;
        ensure!(
            options.insert(kind),
            "{label} repeats runtime option {kind}"
        );
    }
    validate_plugin_process_environment_links(&argv[2..signed_entry], label)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginProcessGrantKind {
    Read,
    Run,
    Net,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginProcessGrant<'a> {
    Literal(&'a str),
    Environment { name: &'a str, suffix: &'a str },
    EnvironmentUrlHosts { name: &'a str },
}

fn validate_plugin_process_option<'a>(option: &'a str, label: &str) -> Result<&'a str> {
    let kind = match option {
        "--quiet" => "quiet",
        "--no-prompt" => "no-prompt",
        "--no-config" => "no-config",
        "--no-remote" => "no-remote",
        "--no-npm" => "no-npm",
        _ => {
            if let Some(names) = option.strip_prefix("--allow-env=") {
                ensure!(!names.is_empty(), "{label} has an empty environment grant");
                for name in names.split(',') {
                    validate_isolated_home_env(name)
                        .with_context(|| format!("{label} has an invalid environment grant"))?;
                }
                "allow-env"
            } else if let Some(values) = option.strip_prefix("--allow-read=") {
                validate_plugin_process_grant(values, label, PluginProcessGrantKind::Read)?;
                "allow-read"
            } else if let Some(values) = option.strip_prefix("--allow-run=") {
                validate_plugin_process_grant(values, label, PluginProcessGrantKind::Run)?;
                "allow-run"
            } else if let Some(values) = option.strip_prefix("--allow-net=") {
                validate_plugin_process_grant(values, label, PluginProcessGrantKind::Net)?;
                "allow-net"
            } else {
                anyhow::bail!("{label} uses unsupported Plugin JavaScript option {option}");
            }
        }
    };
    Ok(kind)
}

fn validate_plugin_process_grant(
    values: &str,
    label: &str,
    kind: PluginProcessGrantKind,
) -> Result<()> {
    let values = values.split(',').collect::<Vec<_>>();
    ensure!(
        !values.is_empty() && values.len() <= 16,
        "{label} has an invalid Plugin JavaScript permission grant"
    );
    for value in values {
        parse_plugin_process_grant(value, kind).with_context(|| {
            format!("{label} has an invalid Plugin JavaScript permission grant")
        })?;
    }
    Ok(())
}

/// Parse one closed process capability grant.
///
/// # Errors
/// Returns when the literal or environment-bound grant is malformed or is
/// incompatible with the requested capability kind.
pub fn parse_plugin_process_grant(
    value: &str,
    kind: PluginProcessGrantKind,
) -> Result<PluginProcessGrant<'_>> {
    ensure!(
        (1..=256).contains(&value.len())
            && !value.contains('\0')
            && !value.bytes().any(|byte| byte.is_ascii_whitespace()),
        "permission grant is empty or contains whitespace"
    );
    if let Some(binding) = value.strip_prefix("${ENV_URL_HOSTS:") {
        ensure!(
            kind == PluginProcessGrantKind::Net && binding.ends_with('}'),
            "URL-host binding is only valid for network grants"
        );
        let name = &binding[..binding.len() - 1];
        validate_isolated_home_env(name)?;
        return Ok(PluginProcessGrant::EnvironmentUrlHosts { name });
    }
    if let Some(binding) = value.strip_prefix("${ENV:") {
        let close = binding
            .find('}')
            .context("environment binding is not closed")?;
        let name = &binding[..close];
        let suffix = &binding[close + 1..];
        validate_isolated_home_env(name)?;
        ensure!(
            kind != PluginProcessGrantKind::Net,
            "network grants must use URL-host bindings"
        );
        ensure!(
            suffix.is_empty()
                || (kind == PluginProcessGrantKind::Read
                    && suffix.starts_with('/')
                    && !suffix.split('/').any(|part| part == "..")
                    && suffix.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')
                    })),
            "invalid environment binding suffix"
        );
        return Ok(PluginProcessGrant::Environment { name, suffix });
    }
    ensure!(!value.contains("${"), "unknown dynamic permission binding");
    match kind {
        PluginProcessGrantKind::Read => ensure!(
            Path::new(value).is_absolute()
                && value != "/"
                && !value.split('/').any(|part| part == ".."),
            "read permission must name a bounded absolute path"
        ),
        PluginProcessGrantKind::Run => ensure!(
            value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')
            }) && value != "/"
                && !value.split('/').any(|part| part == ".."),
            "run permission must name one executable"
        ),
        PluginProcessGrantKind::Net => ensure!(
            value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'.' | b'-' | b'_' | b':' | b'[' | b']')
            }) && value.bytes().any(|byte| byte.is_ascii_alphanumeric()),
            "network permission must name one host"
        ),
    }
    Ok(PluginProcessGrant::Literal(value))
}

fn validate_plugin_process_environment_links(options: &[String], label: &str) -> Result<()> {
    let allowed = options
        .iter()
        .filter_map(|option| option.strip_prefix("--allow-env="))
        .flat_map(|names| names.split(','))
        .collect::<BTreeSet<_>>();
    for option in options {
        let (kind, values) = if let Some(values) = option.strip_prefix("--allow-read=") {
            (PluginProcessGrantKind::Read, values)
        } else if let Some(values) = option.strip_prefix("--allow-run=") {
            (PluginProcessGrantKind::Run, values)
        } else if let Some(values) = option.strip_prefix("--allow-net=") {
            (PluginProcessGrantKind::Net, values)
        } else {
            continue;
        };
        for value in values.split(',') {
            let name = match parse_plugin_process_grant(value, kind)? {
                PluginProcessGrant::Environment { name, .. }
                | PluginProcessGrant::EnvironmentUrlHosts { name } => name,
                PluginProcessGrant::Literal(_) => continue,
            };
            ensure!(
                allowed.contains(name),
                "{label} permission binding reads undeclared environment variable {name}"
            );
        }
    }
    Ok(())
}

fn validate_session_rate_limits(projection: &UsageSessionRateLimits) -> Result<()> {
    ensure!(
        projection.pointer.starts_with('/') && projection.pointer.len() <= 256,
        "session rate-limit pointer must be a bounded JSON pointer"
    );
    ensure!(
        projection.target.len() <= 64
            && !projection.target.is_empty()
            && projection
                .target
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
        "invalid session rate-limit target"
    );
    ensure!(
        projection.required_number_fields.len() + projection.required_string_fields.len() <= 32,
        "too many session rate-limit required fields"
    );
    let mut fields = BTreeSet::new();
    for field in projection
        .required_number_fields
        .iter()
        .chain(&projection.required_string_fields)
    {
        ensure!(
            !field.is_empty()
                && field.len() <= 64
                && field
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
            "invalid session rate-limit required field"
        );
        ensure!(
            fields.insert(field),
            "duplicate session rate-limit required field"
        );
    }
    Ok(())
}

const MAX_TOP_BAR_WINDOWS: usize = 8;
const MAX_TOP_BAR_WINDOW_MINUTES: u32 = 525_600;

fn validate_top_bar_windows(windows: &[u32]) -> Result<()> {
    ensure!(
        windows.len() <= MAX_TOP_BAR_WINDOWS,
        "plugin usage top_bar_windows is too long"
    );
    ensure!(
        windows
            .iter()
            .all(|minutes| *minutes > 0 && *minutes <= MAX_TOP_BAR_WINDOW_MINUTES),
        "plugin usage top_bar_windows must be positive minute windows"
    );
    Ok(())
}

const MAX_USAGE_COPY: usize = 256;
const MAX_USAGE_DESCRIPTION: usize = 512;

const MAX_HOST_LABEL: usize = 64;

fn validate_loopback_origin(value: &str) -> Result<()> {
    let port = value.strip_prefix("http://127.0.0.1:");
    ensure!(
        port.is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port > 0)
        }),
        "plugin loopback_origin must be http://127.0.0.1:<port>"
    );
    Ok(())
}

fn validate_loopback_catalog(value: &str) -> Result<()> {
    ensure!(
        (1..=512).contains(&value.len())
            && value.starts_with('/')
            && !value.contains('\0')
            && !value.contains("//")
            && !value.split('/').any(|component| component == "..")
            && value
                .chars()
                .all(|ch| { ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.') }),
        "invalid loopback_catalog"
    );
    Ok(())
}

fn validate_visual(visual: &PluginVisualSpec) -> Result<()> {
    validate_visual_pair(&visual.light, "visual.light")?;
    validate_visual_pair(&visual.dark, "visual.dark")
}

fn validate_visual_pair(pair: &PluginVisualPair, field: &str) -> Result<()> {
    validate_hex_color(&pair.primary, &format!("{field}.primary"))?;
    validate_hex_color(&pair.secondary, &format!("{field}.secondary"))
}

fn validate_hex_color(value: &str, field: &str) -> Result<()> {
    ensure!(
        value.len() == 7
            && value.starts_with('#')
            && value.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit()),
        "{field} must be a #RRGGBB color"
    );
    Ok(())
}

fn validate_isolated_home_env(value: &str) -> Result<()> {
    ensure!(
        (1..=64).contains(&value.len())
            && value
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_uppercase() || byte == b'_')
            && value
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
        "invalid isolated_home_env"
    );
    Ok(())
}

fn validate_host_label(value: &str) -> Result<()> {
    ensure!(
        (1..=MAX_HOST_LABEL).contains(&value.len())
            && !value.contains('\0')
            && value.chars().all(|ch| !ch.is_control()),
        "invalid plugin host label"
    );
    Ok(())
}

const MAX_LIMIT_LABELS: usize = 16;

fn validate_limit_label_id(value: &str, field: &str) -> Result<()> {
    ensure!(
        (1..=64).contains(&value.len())
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            })
            && !value.starts_with('-')
            && !value.ends_with('-')
            && !value.starts_with('_')
            && !value.ends_with('_')
            && !value.contains("--")
            && !value.contains("__"),
        "invalid plugin usage {field}"
    );
    Ok(())
}

const MAX_CACHE_MIN_HIT_TOKENS: u32 = 10_000_000;
const MAX_CACHE_INTERVAL_MS: u32 = 7 * 24 * 60 * 60 * 1_000;

fn validate_cache_protection(protection: &UsageCacheProtection) -> Result<()> {
    ensure!(
        (1..=MAX_CACHE_MIN_HIT_TOKENS).contains(&protection.min_hit_tokens),
        "plugin usage cache_protection.min_hit_tokens is out of range"
    );
    ensure!(
        (1..=MAX_CACHE_INTERVAL_MS).contains(&protection.interval_ms),
        "plugin usage cache_protection.interval_ms is out of range"
    );
    validate_usage_copy(&protection.min_hit_label, "cache_protection.min_hit_label")?;
    validate_usage_copy(
        &protection.interval_label,
        "cache_protection.interval_label",
    )?;
    validate_usage_copy(&protection.option_name, "cache_protection.option_name")?;
    validate_usage_description(
        &protection.option_description,
        "cache_protection.option_description",
    )?;
    validate_usage_copy(&protection.option_on, "cache_protection.option_on")?;
    validate_usage_copy(&protection.option_off, "cache_protection.option_off")?;
    Ok(())
}

pub const MAX_ACTIVITY_ENTRIES: usize = 8;

fn validate_activity_entries(entries: &[UsageActivityAgent], field: &str) -> Result<()> {
    ensure!(
        entries.len() <= MAX_ACTIVITY_ENTRIES,
        "plugin usage {field} is too long"
    );
    let mut ids = BTreeSet::new();
    for entry in entries {
        validate_plugin_id(&entry.id)?;
        ensure!(
            ids.insert(entry.id.as_str()),
            "duplicate plugin usage {field} id"
        );
        let label_field = format!("{field}.label");
        validate_usage_copy(&entry.label, &label_field)?;
    }
    Ok(())
}

fn validate_limit_labels(labels: &[UsageLimitLabel]) -> Result<()> {
    ensure!(
        labels.len() <= MAX_LIMIT_LABELS,
        "plugin usage limit_labels is too long"
    );
    let mut ids = BTreeSet::new();
    for label in labels {
        validate_limit_label_id(&label.id, "limit_labels.id")?;
        ensure!(
            ids.insert(label.id.as_str()),
            "duplicate plugin usage limit id"
        );
        validate_usage_copy(&label.label, "limit_labels.label")?;
        if let Some(window) = label.window_minutes {
            validate_top_bar_windows(&[window])?;
        }
    }
    Ok(())
}

fn validate_usage_copy(value: &str, field: &str) -> Result<()> {
    validate_usage_text(value, field, MAX_USAGE_COPY)
}

fn validate_usage_description(value: &str, field: &str) -> Result<()> {
    validate_usage_text(value, field, MAX_USAGE_DESCRIPTION)
}

fn validate_usage_text(value: &str, field: &str, max: usize) -> Result<()> {
    ensure!(
        (1..=max).contains(&value.len())
            && !value.contains('\0')
            && value.chars().all(|ch| !ch.is_control()),
        "invalid plugin usage {field}"
    );
    Ok(())
}

fn validate_usage_account(account: &str) -> Result<()> {
    ensure!(
        (1..=64).contains(&account.len())
            && account
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !account.starts_with('-')
            && !account.ends_with('-')
            && !account.contains("--"),
        "invalid plugin usage account id"
    );
    Ok(())
}

/// Validate Plugin migration SQL against the selected dialect's containment rules.
///
/// # Errors
/// Returns when the SQL is empty, oversized, malformed, or references a
/// forbidden host namespace or operation.
pub fn validate_plugin_sql(dialect: &str, sql: &str) -> Result<()> {
    let statements = split_sql_statements(sql)?;
    ensure!(
        !statements.is_empty(),
        "{dialect} plugin SQL has no statements"
    );
    ensure!(
        statements.len() <= MAX_SQL_STATEMENTS,
        "{dialect} plugin SQL has too many statements"
    );
    for statement in &statements {
        let normalized = normalize_sql_for_guard(statement);
        for needle in forbidden_sql(dialect) {
            ensure!(
                !normalized.contains(needle),
                "{dialect} plugin SQL is not allowed to use {needle}"
            );
        }
    }
    Ok(())
}

/// Split migration SQL while preserving quoted semicolons and discarding comments.
///
/// # Errors
/// Returns when a quoted string or block comment is unterminated.
pub fn split_sql_statements(sql: &str) -> Result<Vec<String>> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
                current.push('\n');
            }
            continue;
        }
        if in_block_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if !in_single && ch == '-' && chars.peek() == Some(&'-') {
            chars.next();
            in_line_comment = true;
            continue;
        }
        if !in_single && ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_block_comment = true;
            continue;
        }
        if ch == '\'' {
            current.push(ch);
            if in_single {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    current.push('\'');
                } else {
                    in_single = false;
                }
            } else {
                in_single = true;
            }
            continue;
        }
        if ch == ';' && !in_single {
            push_statement(&mut statements, &mut current);
            continue;
        }
        current.push(ch);
    }
    ensure!(!in_single, "unterminated string in plugin SQL");
    ensure!(
        !in_block_comment,
        "unterminated block comment in plugin SQL"
    );
    push_statement(&mut statements, &mut current);
    Ok(statements)
}

fn push_statement(statements: &mut Vec<String>, current: &mut String) {
    let statement = current.trim();
    if !statement.is_empty() {
        statements.push(statement.to_owned());
    }
    current.clear();
}

fn normalize_sql_for_guard(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn forbidden_sql(dialect: &str) -> &'static [&'static str] {
    match dialect {
        "postgres" => &[
            "public.",
            "pg_catalog.",
            "information_schema.",
            "set search_path",
            "set local search_path",
            "create schema",
            "drop schema",
            "alter schema",
            "create database",
            "alter database",
            "drop database",
            "create extension",
            "create server",
            "create foreign",
            "alter system",
            "copy ",
            "lo_import",
            "pg_read_file",
            "dblink",
        ],
        "sqlite" => &[
            "attach ",
            "detach ",
            "load_extension",
            "vacuum into",
            "pragma ",
        ],
        _ => &[],
    }
}

/// `PostgreSQL` schema name derived from a plugin id. Hyphens become underscores.
///
/// # Errors
/// Returns when the plugin id is not a closed slug.
pub fn postgres_schema_name(plugin_id: &str) -> Result<String> {
    validate_plugin_id(plugin_id)?;
    let schema = format!("plugin_{}", plugin_id.replace('-', "_"));
    ensure!(
        schema.len() <= 63
            && schema
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "plugin PostgreSQL schema name is invalid"
    );
    Ok(schema)
}

/// # Errors
/// Returns when the id is not a lowercase slug.
pub fn validate_plugin_id(plugin_id: &str) -> Result<()> {
    ensure!(
        (1..=64).contains(&plugin_id.len())
            && plugin_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !plugin_id.starts_with('-')
            && !plugin_id.ends_with('-')
            && !plugin_id.contains("--"),
        "invalid plugin id"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_spec_requires_both_sql_dialects() {
        let spec = serde_json::json!({
            "schema_version": 1,
            "storage": {
                "postgres": {
                    "migrations": [{ "version": "0001", "sql": "CREATE TABLE notes (id TEXT PRIMARY KEY);" }]
                }
            }
        });
        assert!(PluginHostSpec::from_json(spec.to_string().as_bytes()).is_err());

        let out_of_order = serde_json::json!({
            "schema_version": 1,
            "storage": {
                "postgres": {
                    "migrations": [
                        { "version": "0002", "sql": "CREATE TABLE later (id TEXT PRIMARY KEY);" },
                        { "version": "0001", "sql": "CREATE TABLE earlier (id TEXT PRIMARY KEY);" }
                    ]
                },
                "sqlite": {
                    "migrations": [
                        { "version": "0002", "sql": "CREATE TABLE later (id TEXT PRIMARY KEY);" },
                        { "version": "0001", "sql": "CREATE TABLE earlier (id TEXT PRIMARY KEY);" }
                    ]
                }
            }
        });
        assert!(PluginHostSpec::from_json(out_of_order.to_string().as_bytes()).is_err());
    }

    #[test]
    fn host_spec_rejects_core_schema_escape() {
        let spec = PluginHostSpec {
            schema_version: 1,
            slots: vec![PluginSlotId::ProviderUsage],
            ui: None,
            storage: Some(PluginStorageSpec {
                postgres: PluginSqlMigrations {
                    migrations: vec![PluginMigration {
                        version: "0001".to_owned(),
                        sql:
                            "CREATE TABLE notes (id TEXT PRIMARY KEY); SELECT * FROM public.users;"
                                .to_owned(),
                    }],
                },
                sqlite: PluginSqlMigrations {
                    migrations: vec![PluginMigration {
                        version: "0001".to_owned(),
                        sql: "CREATE TABLE notes (id TEXT PRIMARY KEY);".to_owned(),
                    }],
                },
            }),
            native_capabilities: Vec::new(),
            usage: None,
            label: None,
            loopback_origin: None,
            loopback_detail: None,
            loopback_requires_catalog: false,
            loopback_catalog: None,
            loopback_env: None,
            adapter_slot: None,
            cli_executable: None,
            cli_executable_env: None,
            cli_auth: PluginCliAuthKind::default(),
            cli_auth_argv: Vec::new(),
            cli_auth_rules: None,
            login_fields: None,
            rpc_argv: Vec::new(),
            visual: None,
            isolated_home_env: None,
            isolated_home_settings: BTreeMap::new(),
            isolated_shell: false,
            isolated_shell_env: Vec::new(),
        };
        assert!(spec.validate().is_err());
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One fixture exercises the complete host schema surface.
    fn host_spec_accepts_usage_account() {
        let spec = PluginHostSpec::from_json(
            br#"{"schema_version":1,"slots":["provider.usage"],"ui":{"schema_version":1,"renderers":{"provider.usage":"provider-usage-v1"}},"usage":{"account":"xai","collector":"xai-billing","reset":"xai","reset_claim":"before-attempt","product":"xAI","parser":"xai-credits","error":"xai-billing","error_auth":"Sign in to Grok Build in Machines, then refresh xAI usage.","error_config":"Grok Build usage is not configured on this Machine.","error_fetch":"Grok Build could not fetch xAI usage.","order":1,"widget":"xai-included","widget_shape":"percent"}}"#,
        )
        .unwrap();
        assert_eq!(
            spec.usage.as_ref().map(|usage| usage.account.as_str()),
            Some("xai")
        );
        assert!(
            spec.usage
                .as_ref()
                .is_some_and(PluginUsageSpec::refreshable)
        );
        assert_eq!(
            spec.usage.as_ref().and_then(|usage| usage.reset.as_deref()),
            Some("xai")
        );
        assert_eq!(
            spec.usage.map(|usage| {
                (
                    usage.parser,
                    usage.error,
                    usage.error_auth,
                    usage.order,
                    usage.widget,
                    usage.widget_shape,
                    usage.reset_claim,
                )
            }),
            Some((
                UsageLimitParserKind::new("xai-credits"),
                UsageErrorKind::new("xai-billing"),
                Some("Sign in to Grok Build in Machines, then refresh xAI usage.".to_owned()),
                Some(1),
                UsageWidgetKind::new("xai-included"),
                UsageWidgetShape::Percent,
                UsageResetClaim::BeforeAttempt,
            ))
        );
        assert!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"usage":{"account":"OpenAI"}}"#)
                .is_err()
        );
        assert_eq!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"label":"Password","slots":["login.method"],"ui":{"schema_version":1,"renderers":{"login.method":"login-password-v1"}}}"#,
            )
            .unwrap()
            .label
            .as_deref(),
            Some("Password")
        );
        assert!(PluginHostSpec::from_json(br#"{"schema_version":1,"label":""}"#).is_err());
        assert!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"slots":["login.method"],"ui":{"schema_version":2,"renderers":{"login.method":"login-password-v1"}}}"#,
            )
            .is_err()
        );
        assert!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"slots":["provider.usage"],"ui":{"schema_version":1,"renderers":{"provider.usage":"login-password-v1"}}}"#,
            )
            .is_err()
        );
        assert!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"slots":["login.method"],"ui":{"schema_version":1,"renderers":{"login.method":"login-password-v1"},"entry":"ui/index.js"}}"#,
            )
            .is_err()
        );
        assert!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"native_capabilities":["webauthn","webauthn"]}"#,
            )
            .is_err()
        );
        assert_eq!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"loopback_origin":"http://127.0.0.1:61137"}"#,
            )
            .unwrap()
            .loopback_origin
            .as_deref(),
            Some("http://127.0.0.1:61137")
        );
        assert!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"loopback_origin":"https://example.com"}"#,
            )
            .is_err()
        );
        assert_eq!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"adapter_slot":"claude"}"#,)
                .unwrap()
                .adapter_slot
                .as_deref(),
            Some("claude")
        );
        assert!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"adapter_slot":"Claude"}"#).is_err()
        );
        let claude = PluginHostSpec::from_json(
            br#"{"schema_version":1,"usage":{"account":"anthropic","limit_id_prefix":"claude","limit_labels":[{"id":"five_hour","label":"5h","window_minutes":300}]}}"#,
        )
        .unwrap();
        assert_eq!(
            claude
                .usage
                .as_ref()
                .and_then(|usage| usage.limit_id_prefix.as_deref()),
            Some("claude")
        );
        assert_eq!(
            claude
                .usage
                .as_ref()
                .and_then(|usage| usage.limit_labels.first().map(|label| label.id.as_str())),
            Some("five_hour")
        );
        assert_eq!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"usage":{"account":"deepseek","widget_balance_label":"Balance","widget_spend_label":"24h spend"}}"#,
            )
            .unwrap()
            .usage
            .as_ref()
            .and_then(|usage| usage.widget_balance_label.as_deref()),
            Some("Balance")
        );
        assert_eq!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"usage":{"account":"deepseek","activity_agents":[{"id":"codex","label":"Codex"},{"id":"claude","label":"Claude Code"}],"activity_models":[{"id":"flash","label":"Flash"},{"id":"pro","label":"Pro"}]}}"#,
            )
            .unwrap()
            .usage
            .as_ref()
            .map(|usage| (usage.activity_agents.len(), usage.activity_models.len())),
            Some((2, 2))
        );
        assert_eq!(
            PluginHostSpec::from_json(
                r#"{"schema_version":1,"usage":{"account":"deepseek","cache_protection":{"min_hit_tokens":64000,"min_hit_label":"64K","interval_ms":28800000,"interval_label":"8h","option_name":"Cache protection","option_description":"Automatically protects DeepSeek prompt caches after at least 64K verified hit tokens.","option_on":"Auto · recommended","option_off":"Off"}}}"#
                    .as_bytes(),
            )
            .unwrap()
            .usage
            .as_ref()
            .and_then(|usage| usage.cache_protection.as_ref())
            .map(|protection| {
                (
                    protection.min_hit_tokens,
                    protection.min_hit_label.as_str(),
                    protection.interval_ms,
                    protection.interval_label.as_str(),
                    protection.option_name.as_str(),
                    protection.option_on.as_str(),
                    protection.option_off.as_str(),
                )
            }),
            Some((
                64_000,
                "64K",
                28_800_000,
                "8h",
                "Cache protection",
                "Auto · recommended",
                "Off",
            ))
        );
        let deepseek = PluginHostSpec::from_json(
            br#"{"schema_version":1,"usage":{"account":"deepseek","available_status":"API","omit_empty_limits":true}}"#,
        )
        .unwrap();
        assert_eq!(
            deepseek
                .usage
                .as_ref()
                .and_then(|usage| usage.available_status.as_deref()),
            Some("API")
        );
        assert!(deepseek.usage.is_some_and(|usage| usage.omit_empty_limits));
        assert_eq!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"usage":{"account":"future","collector":"future-bill","collector_argv":["@plugin-js","run","${PLUGIN_DIR}/collector/index.js"]}}"#,
            )
            .unwrap()
            .usage
            .as_ref()
            .map(|usage| (usage.collector.as_str(), usage.collector_argv.len())),
            Some(("future-bill", 3))
        );
        assert_eq!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"rpc_argv":["@plugin-js","run","${PLUGIN_DIR}/collector/rpc.js"]}"#,
            )
            .unwrap()
            .rpc_argv,
            vec![
                "@plugin-js".to_owned(),
                "run".to_owned(),
                "${PLUGIN_DIR}/collector/rpc.js".to_owned(),
            ]
        );
        assert!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"rpc_argv":["/bin/true"]}"#,)
                .is_err()
        );
        for forbidden in [
            "--allow-all",
            "-A",
            "--allow-read",
            "--allow-run",
            "--allow-net",
            "--allow-write",
            "--allow-env",
            "--allow-import=example.com",
            "--config=config.json",
            "--allow-net=*",
        ] {
            let host = serde_json::json!({
                "schema_version": 1,
                "rpc_argv": [
                    "@plugin-js",
                    "run",
                    forbidden,
                    "${PLUGIN_DIR}/collector/rpc.js"
                ]
            });
            assert!(
                PluginHostSpec::from_json(&serde_json::to_vec(&host).unwrap()).is_err(),
                "accepted over-capable runtime option {forbidden}"
            );
        }
        PluginHostSpec::from_json(
            br#"{"schema_version":1,"rpc_argv":["@plugin-js","run","--no-remote","--allow-env=FUTURE_TOKEN,FUTURE_HOME","--allow-net=api.example.com","${PLUGIN_DIR}/collector/rpc.js"]}"#,
        )
        .expect("closed Plugin JavaScript options");
        PluginHostSpec::from_json(
            br#"{"schema_version":1,"rpc_argv":["@plugin-js","run","--allow-env=FUTURE_URLS,FUTURE_HOME,FUTURE_COMMAND","--allow-read=${ENV:FUTURE_HOME}/auth.json","--allow-run=future,${ENV:FUTURE_COMMAND}","--allow-net=${ENV_URL_HOSTS:FUTURE_URLS}","${PLUGIN_DIR}/collector/rpc.js"]}"#,
        )
        .expect("closed dynamic Plugin JavaScript options");
        assert!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"rpc_argv":["@plugin-js","run","--allow-read=${ENV:UNDECLARED_HOME}/auth.json","${PLUGIN_DIR}/collector/rpc.js"]}"#,
            )
            .is_err()
        );
        assert!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"rpc_argv":["@plugin-js","run","${PLUGIN_DIR}/collector/../rpc.js"]}"#,
            )
            .is_err()
        );
        assert_eq!(
            PluginHostSpec::from_json(
                r##"{"schema_version":1,"visual":{"light":{"primary":"#C65D3A","secondary":"#9A4A30"},"dark":{"primary":"#E08A6A","secondary":"#D97757"}}}"##
                    .as_bytes(),
            )
            .unwrap()
            .visual
            .map(|visual| visual.dark.primary),
            Some("#E08A6A".to_owned())
        );
        let isolated = PluginHostSpec::from_json(
            br#"{"schema_version":1,"isolated_home_env":"CLAUDE_CONFIG_DIR","isolated_shell":true}"#,
        )
        .unwrap();
        assert_eq!(
            isolated.isolated_home_env.as_deref(),
            Some("CLAUDE_CONFIG_DIR")
        );
        assert!(isolated.isolated_shell);
        let isolated = PluginHostSpec::from_json(
            br#"{"schema_version":1,"isolated_home_env":"CLAUDE_CONFIG_DIR","isolated_home_settings":{"autoCompactEnabled":true}}"#,
        )
        .unwrap();
        assert_eq!(
            isolated.isolated_home_settings.get("autoCompactEnabled"),
            Some(&serde_json::Value::Bool(true))
        );
        assert!(
            PluginHostSpec::from_json(
                br#"{"schema_version":1,"isolated_home_settings":{"autoCompactEnabled":true}}"#,
            )
            .is_err()
        );
        assert!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"isolated_home_env":"claude"}"#)
                .is_err()
        );
        assert!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"isolated_home_env":""}"#).is_err()
        );
        let loopback = PluginHostSpec::from_json(
            br#"{"schema_version":1,"loopback_origin":"http://127.0.0.1:61137","loopback_detail":"loopback Responses gateway","loopback_requires_catalog":true,"loopback_catalog":"/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json"}"#,
        )
        .unwrap();
        assert_eq!(
            loopback.loopback_detail.as_deref(),
            Some("loopback Responses gateway")
        );
        assert!(loopback.loopback_requires_catalog);
        assert_eq!(
            loopback.loopback_catalog.as_deref(),
            Some(
                "/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json"
            )
        );
        assert!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"loopback_requires_catalog":true}"#,)
                .is_err()
        );
        assert_eq!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"cli_executable":"claude"}"#)
                .unwrap()
                .cli_executable
                .as_deref(),
            Some("claude")
        );
        assert!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"cli_executable":"Claude"}"#)
                .is_err()
        );
        let auth = PluginHostSpec::from_json(
            br#"{"schema_version":1,"cli_auth":"exit","cli_auth_argv":["login","status"]}"#,
        )
        .unwrap();
        assert_eq!(auth.cli_auth, PluginCliAuthKind::new("exit"));
        assert_eq!(
            auth.cli_auth_argv,
            ["login".to_owned(), "status".to_owned()]
        );
        assert!(PluginHostSpec::from_json(br#"{"schema_version":1,"cli_auth":"exit"}"#).is_err());
        assert_eq!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"cli_auth":"future-probe"}"#)
                .unwrap()
                .cli_auth,
            PluginCliAuthKind::new("future-probe")
        );
        let launch = PluginHostSpec::from_json(
            br#"{"schema_version":1,"loopback_env":"ANTHROPIC_BASE_URL","cli_executable_env":"CLAUDE_CODE_EXECUTABLE","isolated_shell":true,"isolated_shell_env":["CLAUDE_CODE_SHELL","SHELL"]}"#,
        )
        .unwrap();
        assert_eq!(launch.loopback_env.as_deref(), Some("ANTHROPIC_BASE_URL"));
        assert_eq!(
            launch.cli_executable_env.as_deref(),
            Some("CLAUDE_CODE_EXECUTABLE")
        );
        assert_eq!(
            launch.isolated_shell_env,
            ["CLAUDE_CODE_SHELL".to_owned(), "SHELL".to_owned()]
        );
    }

    #[test]
    fn opaque_host_strategy_identifiers_are_bounded_slugs() {
        for field in ["collector", "parser", "error", "widget", "session_overlay"] {
            let mut source = serde_json::json!({
                "schema_version": 1,
                "usage": {
                    "account": "future"
                }
            });
            source["usage"][field] = serde_json::Value::String("Future Strategy".to_owned());
            assert!(
                PluginHostSpec::from_json(source.to_string().as_bytes()).is_err(),
                "accepted invalid usage strategy {field}"
            );
        }
        assert!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"cli_auth":"Future Strategy"}"#,)
                .is_err()
        );
        assert!(
            PluginHostSpec::from_json(
                format!(
                    r#"{{"schema_version":1,"usage":{{"account":"future","widget":"{}"}}}}"#,
                    "a".repeat(65)
                )
                .as_bytes(),
            )
            .is_err()
        );
        PluginHostSpec::from_json(
            br#"{"schema_version":1,"cli_auth":"future-probe","usage":{"account":"future","collector":"future-collector","parser":"future-parser","error":"future-error","widget":"future-widget","session_overlay":"future-overlay"}}"#,
        )
        .expect("bounded future strategy identifiers");
    }

    #[test]
    fn usage_sidecar_targets_are_explicit_bounded_loopback_paths() {
        let valid = PluginHostSpec::from_json(
            br#"{"schema_version":1,"usage":{"account":"deepseek","collector":"command","collector_argv":["@plugin-js","run","${PLUGIN_DIR}/collector/index.js"],"activity_agents":[{"id":"codex","label":"Codex"}],"collector_sidecars":[{"id":"codex","adapter_slot":"codex","sidecar":"deepseek-gateway","path":"/provider-info"}]}}"#,
        )
        .unwrap();
        assert_eq!(
            valid.usage.unwrap().collector_sidecars[0].adapter_slot,
            "codex"
        );
        for path in ["//example.invalid/x", "/../x", "/x?redirect=evil"] {
            let source = format!(
                r#"{{"schema_version":1,"usage":{{"account":"deepseek","collector":"command","collector_argv":["@plugin-js","run","${{PLUGIN_DIR}}/collector/index.js"],"activity_agents":[{{"id":"codex","label":"Codex"}}],"collector_sidecars":[{{"id":"codex","adapter_slot":"codex","sidecar":"deepseek-gateway","path":"{path}"}}]}}}}"#
            );
            assert!(PluginHostSpec::from_json(source.as_bytes()).is_err());
        }
    }

    #[test]
    fn postgres_schema_tracks_plugin_id() {
        assert_eq!(
            postgres_schema_name("claude-code").unwrap(),
            "plugin_claude_code"
        );
        assert!(postgres_schema_name("../users").is_err());
    }

    #[test]
    fn load_optional_reads_host_json() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-plugin-host-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(PluginHostSpec::load_optional(&root).unwrap().is_none());
        std::fs::write(
            root.join("host.json"),
            r#"{
              "schema_version": 1,
              "slots": ["login.method"],
              "ui": {
                "schema_version": 1,
                "renderers": { "login.method": "login-password-v1" }
              },
              "storage": {
                "postgres": {
                  "migrations": [{ "version": "0001", "sql": "CREATE TABLE notes (id TEXT PRIMARY KEY);" }]
                },
                "sqlite": {
                  "migrations": [{ "version": "0001", "sql": "CREATE TABLE notes (id TEXT PRIMARY KEY);" }]
                }
              }
            }"#,
        )
        .unwrap();
        let spec = PluginHostSpec::load_optional(&root).unwrap().unwrap();
        assert_eq!(spec.slots, vec![PluginSlotId::LoginMethod]);
        let _ = std::fs::remove_dir_all(root);
    }
}
