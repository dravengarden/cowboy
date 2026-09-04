//! Controller-side plugin host contract.
//!
//! Capability payloads (`provider.json`, OIDC authentication, Zed) stay in the
//! Plugin SDK. This module is the business-agnostic host surface: slots, UI
//! entry, and plugin-owned storage. A plugin may ship `host.json` next to its
//! generation tree; missing host metadata means "no host UI/storage", not an
//! error.

#![warn(clippy::pedantic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

pub const PLUGIN_HOST_SCHEMA_VERSION: u16 = 1;
pub const PLUGIN_HOST_API_VERSION: &str = "1.0.0";
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
    pub login_fields: Option<PluginLoginFields>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rpc_argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual: Option<PluginVisualSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolated_home_env: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginCliAuthKind {
    #[default]
    None,
    Exit,
    GeminiEnv,
    GrokJson,
    #[serde(other)]
    Unknown,
}

fn is_none_cli_auth(kind: &PluginCliAuthKind) -> bool {
    matches!(kind, PluginCliAuthKind::None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageCollectorKind {
    #[default]
    Session,
    OpenaiAppserver,
    XaiBilling,
    DeepseekStore,
    Command,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageLimitParserKind {
    #[default]
    GenericBuckets,
    XaiCredits,
    AnthropicUtilization,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageErrorKind {
    #[default]
    Raw,
    OpenaiAuth,
    XaiBilling,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageWidgetKind {
    #[default]
    None,
    OpenaiWeekly,
    XaiIncluded,
    DeepseekBalance,
    #[serde(other)]
    Unknown,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageSessionOverlay {
    #[default]
    None,
    AnthropicRateLimit,
    GeminiSessionOnly,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginUsageSpec {
    pub account: String,
    #[serde(default, skip_serializing_if = "is_session_collector")]
    pub collector: UsageCollectorKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collector_argv: Vec<String>,
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
    #[serde(default, skip_serializing_if = "is_none_widget_shape")]
    pub widget_shape: UsageWidgetShape,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_window: Option<u32>,
    #[serde(default, skip_serializing_if = "is_after_success_claim")]
    pub reset_claim: UsageResetClaim,
    #[serde(default, skip_serializing_if = "is_none_session_overlay")]
    pub session_overlay: UsageSessionOverlay,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_status: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub activity: bool,
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

fn is_session_collector(kind: &UsageCollectorKind) -> bool {
    matches!(kind, UsageCollectorKind::Session)
}

fn is_generic_parser(kind: &UsageLimitParserKind) -> bool {
    matches!(kind, UsageLimitParserKind::GenericBuckets)
}

fn is_raw_error(kind: &UsageErrorKind) -> bool {
    matches!(kind, UsageErrorKind::Raw)
}

fn is_none_widget(kind: &UsageWidgetKind) -> bool {
    matches!(kind, UsageWidgetKind::None)
}

fn is_none_widget_shape(kind: &UsageWidgetShape) -> bool {
    matches!(kind, UsageWidgetShape::None)
}

fn is_after_success_claim(kind: &UsageResetClaim) -> bool {
    matches!(kind, UsageResetClaim::AfterSuccess)
}

fn is_none_session_overlay(kind: &UsageSessionOverlay) -> bool {
    matches!(kind, UsageSessionOverlay::None)
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl PluginUsageSpec {
    #[must_use]
    pub fn refreshable(&self) -> bool {
        !self.collector_argv.is_empty()
            || matches!(
                self.collector,
                UsageCollectorKind::OpenaiAppserver
                    | UsageCollectorKind::XaiBilling
                    | UsageCollectorKind::DeepseekStore
                    | UsageCollectorKind::Command
            )
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
    pub entry: String,
    pub host_api: String,
    pub ui_kit: String,
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
    /// Returns when schema, slots, UI paths, or storage migrations are invalid.
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
            validate_relative_entry(&ui.entry)?;
            validate_exact_semver(&ui.host_api, "plugin host API")?;
            validate_exact_semver(&ui.ui_kit, "plugin UI kit")?;
        }
        for capability in &self.native_capabilities {
            validate_native_capability(capability)?;
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
        match self.cli_auth {
            PluginCliAuthKind::Exit => {
                ensure!(
                    !self.cli_auth_argv.is_empty(),
                    "cli_auth exit needs cli_auth_argv"
                );
                validate_collector_argv(&self.cli_auth_argv)?;
            }
            PluginCliAuthKind::None
            | PluginCliAuthKind::GeminiEnv
            | PluginCliAuthKind::GrokJson
            | PluginCliAuthKind::Unknown => {
                ensure!(
                    self.cli_auth_argv.is_empty(),
                    "cli_auth_argv is only valid for exit probes"
                );
            }
        }
        validate_collector_argv(&self.rpc_argv)?;
        if let Some(visual) = &self.visual {
            validate_visual(visual)?;
        }
        if let Some(env) = &self.isolated_home_env {
            validate_isolated_home_env(env)?;
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
            validate_collector_argv(&usage.collector_argv)?;
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
}

impl PluginSqlMigrations {
    pub(crate) fn validate(&self, dialect: &str) -> Result<()> {
        ensure!(
            !self.migrations.is_empty(),
            "{dialect} plugin storage has no migrations"
        );
        ensure!(
            self.migrations.len() <= MAX_MIGRATIONS,
            "{dialect} plugin storage has too many migrations"
        );
        let mut versions = BTreeSet::new();
        for migration in &self.migrations {
            ensure!(
                migration.version.len() == 4
                    && migration.version.bytes().all(|byte| byte.is_ascii_digit()),
                "{dialect} plugin migration version must be four digits"
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
        }
        Ok(())
    }
}

fn validate_relative_entry(entry: &str) -> Result<()> {
    ensure!(
        !entry.is_empty()
            && !entry.starts_with('/')
            && !entry.split('/').any(|part| part.is_empty() || part == "..")
            && entry
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric()
                    || matches!(byte, b'.' | b'_' | b'-' | b'/')),
        "invalid plugin UI entry"
    );
    Ok(())
}

fn validate_exact_semver(value: &str, label: &str) -> Result<()> {
    let version = semver::Version::parse(value).with_context(|| format!("parsing {label}"))?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "{label} must be exact stable SemVer"
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

const MAX_ACTIVITY_ENTRIES: usize = 8;

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

pub(crate) fn validate_plugin_sql(dialect: &str, sql: &str) -> Result<()> {
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

pub(crate) fn split_sql_statements(sql: &str) -> Result<Vec<String>> {
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
                    current.push(chars.next().expect("escaped quote"));
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

/// PostgreSQL schema name derived from a plugin id. Hyphens become underscores.
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
            cli_auth: PluginCliAuthKind::None,
            cli_auth_argv: Vec::new(),
            login_fields: None,
            rpc_argv: Vec::new(),
            visual: None,
            isolated_home_env: None,
            isolated_shell: false,
            isolated_shell_env: Vec::new(),
        };
        assert!(spec.validate().is_err());
    }

    #[test]
    fn host_spec_accepts_usage_account() {
        let spec = PluginHostSpec::from_json(
            br#"{"schema_version":1,"slots":["provider.usage"],"usage":{"account":"xai","collector":"xai-billing","reset":"xai","reset_claim":"before-attempt","product":"xAI","parser":"xai-credits","error":"xai-billing","error_auth":"Sign in to Grok Build in Machines, then refresh xAI usage.","error_config":"Grok Build usage is not configured on this Machine.","error_fetch":"Grok Build could not fetch xAI usage.","order":1,"widget":"xai-included","widget_shape":"percent"}}"#,
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
                UsageLimitParserKind::XaiCredits,
                UsageErrorKind::XaiBilling,
                Some("Sign in to Grok Build in Machines, then refresh xAI usage.".to_owned()),
                Some(1),
                UsageWidgetKind::XaiIncluded,
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
                br#"{"schema_version":1,"label":"Password","slots":["login.method"]}"#,
            )
            .unwrap()
            .label
            .as_deref(),
            Some("Password")
        );
        assert!(PluginHostSpec::from_json(br#"{"schema_version":1,"label":""}"#).is_err());
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
                br#"{"schema_version":1,"usage":{"account":"future","collector":"future-bill","collector_argv":["/bin/true"]}}"#,
            )
            .unwrap()
            .usage
            .as_ref()
            .map(|usage| (usage.collector, usage.collector_argv.len())),
            Some((UsageCollectorKind::Unknown, 1))
        );
        assert_eq!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"rpc_argv":["/bin/true"]}"#,)
                .unwrap()
                .rpc_argv,
            vec!["/bin/true".to_owned()]
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
        assert_eq!(auth.cli_auth, PluginCliAuthKind::Exit);
        assert_eq!(
            auth.cli_auth_argv,
            ["login".to_owned(), "status".to_owned()]
        );
        assert!(PluginHostSpec::from_json(br#"{"schema_version":1,"cli_auth":"exit"}"#).is_err());
        assert_eq!(
            PluginHostSpec::from_json(br#"{"schema_version":1,"cli_auth":"gemini-env"}"#)
                .unwrap()
                .cli_auth,
            PluginCliAuthKind::GeminiEnv
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
