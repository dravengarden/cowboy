//! Product authentication configuration over signed, data-only Plugins.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

use crate::oidc::{OidcProvider, OidcProviderRuntimeDocument};
use crate::plugin_catalog::PluginCatalog;

const CONFIG_SCHEMA_V1: &str = "dravengarden.cowboy.authentication/v1";
const CONFIG_SCHEMA_V2: &str = "dravengarden.cowboy.authentication/v2";
const MAX_CONFIG_BYTES: u64 = 128 * 1_024;
pub(crate) const PASSWORD_LOGIN_METHOD: &str = "password";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapacityEnforcement {
    Observe,
    Enforce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionOverflowPolicy {
    RevokeOldestInactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActiveOverflowPolicy {
    WaitOrReclaimOwn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SingleSessionMode {
    Off,
    NewestWins,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct CapacityServerPolicy {
    pub enforcement: CapacityEnforcement,
    pub authorized_clients_per_user: u32,
    pub signed_in_sessions_per_user: u32,
    pub active_clients_per_user: u32,
    pub active_clients_service: u32,
    pub websocket_channels_per_client: u32,
    pub active_lease_ms: i64,
    pub heartbeat_ms: i64,
    pub reservation_ms: i64,
    pub session_overflow: SessionOverflowPolicy,
    pub active_overflow: ActiveOverflowPolicy,
    pub single_session_mode: SingleSessionMode,
}

impl Default for CapacityServerPolicy {
    fn default() -> Self {
        Self {
            enforcement: CapacityEnforcement::Enforce,
            authorized_clients_per_user: 12,
            signed_in_sessions_per_user: 10,
            active_clients_per_user: 4,
            active_clients_service: 32,
            websocket_channels_per_client: 8,
            active_lease_ms: 120_000,
            heartbeat_ms: 30_000,
            reservation_ms: 30_000,
            session_overflow: SessionOverflowPolicy::RevokeOldestInactive,
            active_overflow: ActiveOverflowPolicy::WaitOrReclaimOwn,
            single_session_mode: SingleSessionMode::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderLogoutPolicy {
    Never,
    Offer,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct LogoutServerPolicy {
    pub provider_logout: ProviderLogoutPolicy,
    pub backchannel_logout: bool,
}

impl Default for LogoutServerPolicy {
    fn default() -> Self {
        Self {
            provider_logout: ProviderLogoutPolicy::Offer,
            backchannel_logout: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct AutomationServerPolicy {
    pub enabled: bool,
    pub active_clients: u32,
    pub credential_max_age_ms: i64,
}

impl Default for AutomationServerPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            active_clients: 32,
            credential_max_age_ms: 10 * 60 * 1_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct PasskeyServerPolicy {
    pub enabled: bool,
    pub prompt_after_login: bool,
    pub session_refresh_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct SessionServerPolicy {
    pub activity_sliding_enabled: bool,
    pub idle_timeout_ms: i64,
    pub passkey_max_age_ms: i64,
    pub passkey_warning_ms: i64,
    pub primary_max_age_ms: i64,
    pub primary_warning_ms: i64,
}

impl Default for SessionServerPolicy {
    fn default() -> Self {
        Self {
            activity_sliding_enabled: true,
            idle_timeout_ms: 24 * 60 * 60 * 1_000,
            passkey_max_age_ms: 3 * 24 * 60 * 60 * 1_000,
            passkey_warning_ms: 30 * 60 * 1_000,
            primary_max_age_ms: 30 * 24 * 60 * 60 * 1_000,
            primary_warning_ms: 24 * 60 * 60 * 1_000,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ProductAuthentication {
    pub password_enabled: bool,
    pub passkeys: PasskeyServerPolicy,
    pub session: SessionServerPolicy,
    pub capacity: CapacityServerPolicy,
    pub logout: LogoutServerPolicy,
    pub automation: AutomationServerPolicy,
    login_method_order: Vec<String>,
    providers: BTreeMap<String, Arc<OidcProvider>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticationDocument {
    schema: String,
    #[serde(default)]
    password: LoginMethodPolicy,
    #[serde(default)]
    passkeys: PasskeyPolicyDocument,
    #[serde(default)]
    session: SessionPolicyDocument,
    #[serde(default)]
    capacity: Option<CapacityPolicyDocument>,
    #[serde(default)]
    logout: Option<LogoutPolicyDocument>,
    #[serde(default)]
    automation: Option<AutomationPolicyDocument>,
    #[serde(default)]
    login_method_order: Option<Vec<String>>,
    #[serde(default)]
    providers: Vec<AuthenticationProviderSelection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapacityPolicyDocument {
    #[serde(default = "default_capacity_enforcement")]
    enforcement: CapacityEnforcement,
    #[serde(default = "default_authorized_clients_per_user")]
    authorized_clients_per_user: u32,
    #[serde(default = "default_signed_in_sessions_per_user")]
    signed_in_sessions_per_user: u32,
    #[serde(default = "default_active_clients_per_user")]
    active_clients_per_user: u32,
    #[serde(default = "default_active_clients_service")]
    active_clients_service: u32,
    #[serde(default = "default_websocket_channels_per_client")]
    websocket_channels_per_client: u32,
    #[serde(default = "default_active_lease_ms")]
    active_lease_ms: i64,
    #[serde(default = "default_heartbeat_ms")]
    heartbeat_ms: i64,
    #[serde(default = "default_reservation_ms")]
    reservation_ms: i64,
    #[serde(default = "default_session_overflow")]
    session_overflow: SessionOverflowPolicy,
    #[serde(default = "default_active_overflow")]
    active_overflow: ActiveOverflowPolicy,
    #[serde(default = "default_single_session_mode")]
    single_session_mode: SingleSessionMode,
}

impl Default for CapacityPolicyDocument {
    fn default() -> Self {
        let policy = CapacityServerPolicy::default();
        Self {
            enforcement: policy.enforcement,
            authorized_clients_per_user: policy.authorized_clients_per_user,
            signed_in_sessions_per_user: policy.signed_in_sessions_per_user,
            active_clients_per_user: policy.active_clients_per_user,
            active_clients_service: policy.active_clients_service,
            websocket_channels_per_client: policy.websocket_channels_per_client,
            active_lease_ms: policy.active_lease_ms,
            heartbeat_ms: policy.heartbeat_ms,
            reservation_ms: policy.reservation_ms,
            session_overflow: policy.session_overflow,
            active_overflow: policy.active_overflow,
            single_session_mode: policy.single_session_mode,
        }
    }
}

impl CapacityPolicyDocument {
    fn validate(self) -> Result<CapacityServerPolicy> {
        const MINUTE_MS: i64 = 60 * 1_000;
        ensure!(
            (1..=10_000).contains(&self.authorized_clients_per_user),
            "authorized client limit must be between 1 and 10000"
        );
        ensure!(
            (1..=1_000).contains(&self.signed_in_sessions_per_user),
            "signed-in session limit must be between 1 and 1000"
        );
        ensure!(
            (1..=1_000).contains(&self.active_clients_per_user),
            "active client per-user limit must be between 1 and 1000"
        );
        ensure!(
            (1..=100_000).contains(&self.active_clients_service),
            "service active client limit must be between 1 and 100000"
        );
        ensure!(
            self.active_clients_per_user <= self.active_clients_service,
            "per-user active client limit cannot exceed the service limit"
        );
        ensure!(
            (1..=64).contains(&self.websocket_channels_per_client),
            "WebSocket channel limit must be between 1 and 64"
        );
        ensure!(
            (MINUTE_MS..=15 * MINUTE_MS).contains(&self.active_lease_ms),
            "active lease must be between 1 and 15 minutes"
        );
        ensure!(
            (5_000..=self.active_lease_ms / 2).contains(&self.heartbeat_ms),
            "capacity heartbeat must be at least 5 seconds and no more than half the lease"
        );
        ensure!(
            (5_000..=5 * MINUTE_MS).contains(&self.reservation_ms),
            "capacity reservation must be between 5 seconds and 5 minutes"
        );
        Ok(CapacityServerPolicy {
            enforcement: self.enforcement,
            authorized_clients_per_user: self.authorized_clients_per_user,
            signed_in_sessions_per_user: self.signed_in_sessions_per_user,
            active_clients_per_user: self.active_clients_per_user,
            active_clients_service: self.active_clients_service,
            websocket_channels_per_client: self.websocket_channels_per_client,
            active_lease_ms: self.active_lease_ms,
            heartbeat_ms: self.heartbeat_ms,
            reservation_ms: self.reservation_ms,
            session_overflow: self.session_overflow,
            active_overflow: self.active_overflow,
            single_session_mode: self.single_session_mode,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogoutPolicyDocument {
    #[serde(default = "default_provider_logout")]
    provider_logout: ProviderLogoutPolicy,
    #[serde(default = "default_true")]
    backchannel_logout: bool,
}

impl Default for LogoutPolicyDocument {
    fn default() -> Self {
        let policy = LogoutServerPolicy::default();
        Self {
            provider_logout: policy.provider_logout,
            backchannel_logout: policy.backchannel_logout,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomationPolicyDocument {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_automation_active_clients")]
    active_clients: u32,
    #[serde(default = "default_automation_credential_max_age_ms")]
    credential_max_age_ms: i64,
}

impl Default for AutomationPolicyDocument {
    fn default() -> Self {
        let policy = AutomationServerPolicy::default();
        Self {
            enabled: policy.enabled,
            active_clients: policy.active_clients,
            credential_max_age_ms: policy.credential_max_age_ms,
        }
    }
}

impl AutomationPolicyDocument {
    fn validate(self) -> Result<AutomationServerPolicy> {
        ensure!(
            (1..=10_000).contains(&self.active_clients),
            "automation active client limit must be between 1 and 10000"
        );
        ensure!(
            (60_000..=10 * 60 * 1_000).contains(&self.credential_max_age_ms),
            "automation credential maximum age must be between 1 and 10 minutes"
        );
        Ok(AutomationServerPolicy {
            enabled: self.enabled,
            active_clients: self.active_clients,
            credential_max_age_ms: self.credential_max_age_ms,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginMethodPolicy {
    #[serde(default = "default_true")]
    enabled: bool,
}

impl Default for LoginMethodPolicy {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyPolicyDocument {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_true")]
    prompt_after_login: bool,
    #[serde(default = "default_true")]
    session_refresh_enabled: bool,
}

impl Default for PasskeyPolicyDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            prompt_after_login: true,
            session_refresh_enabled: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionPolicyDocument {
    #[serde(default = "default_true")]
    activity_sliding_enabled: bool,
    #[serde(default = "default_idle_timeout_ms")]
    idle_timeout_ms: i64,
    #[serde(default = "default_passkey_max_age_ms")]
    passkey_max_age_ms: i64,
    #[serde(default = "default_passkey_warning_ms")]
    passkey_warning_ms: i64,
    #[serde(default = "default_primary_max_age_ms")]
    primary_max_age_ms: i64,
    #[serde(default = "default_primary_warning_ms")]
    primary_warning_ms: i64,
}

impl Default for SessionPolicyDocument {
    fn default() -> Self {
        let policy = SessionServerPolicy::default();
        Self {
            activity_sliding_enabled: policy.activity_sliding_enabled,
            idle_timeout_ms: policy.idle_timeout_ms,
            passkey_max_age_ms: policy.passkey_max_age_ms,
            passkey_warning_ms: policy.passkey_warning_ms,
            primary_max_age_ms: policy.primary_max_age_ms,
            primary_warning_ms: policy.primary_warning_ms,
        }
    }
}

impl SessionPolicyDocument {
    fn validate(self) -> Result<SessionServerPolicy> {
        const MINUTE_MS: i64 = 60 * 1_000;
        const HOUR_MS: i64 = 60 * MINUTE_MS;
        const DAY_MS: i64 = 24 * HOUR_MS;
        ensure!(
            (15 * MINUTE_MS..=DAY_MS).contains(&self.idle_timeout_ms),
            "session idle timeout must be between 15 minutes and 24 hours"
        );
        ensure!(
            (HOUR_MS..=3 * DAY_MS).contains(&self.passkey_max_age_ms)
                && crate::passkey::valid_reauth_interval(self.passkey_max_age_ms),
            "Passkey maximum age must be one of the supported verification intervals"
        );
        ensure!(
            (5 * MINUTE_MS..=2 * HOUR_MS).contains(&self.passkey_warning_ms)
                && self.passkey_warning_ms < self.passkey_max_age_ms,
            "Passkey warning must be between 5 minutes and 2 hours and shorter than its maximum age"
        );
        ensure!(
            (DAY_MS..=90 * DAY_MS).contains(&self.primary_max_age_ms),
            "primary login maximum age must be between 1 and 90 days"
        );
        ensure!(
            (HOUR_MS..=7 * DAY_MS).contains(&self.primary_warning_ms)
                && self.primary_warning_ms < self.primary_max_age_ms,
            "primary login warning must be between 1 hour and 7 days and shorter than its maximum age"
        );
        Ok(SessionServerPolicy {
            activity_sliding_enabled: self.activity_sliding_enabled,
            idle_timeout_ms: self.idle_timeout_ms,
            passkey_max_age_ms: self.passkey_max_age_ms,
            passkey_warning_ms: self.passkey_warning_ms,
            primary_max_age_ms: self.primary_max_age_ms,
            primary_warning_ms: self.primary_warning_ms,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticationProviderSelection {
    plugin_id: String,
    plugin_version: String,
    artifact_digest: String,
    oidc: OidcProviderRuntimeDocument,
}

impl ProductAuthentication {
    pub(crate) fn disabled() -> Self {
        Self {
            password_enabled: false,
            passkeys: PasskeyServerPolicy {
                enabled: false,
                prompt_after_login: false,
                session_refresh_enabled: false,
            },
            session: SessionServerPolicy::default(),
            capacity: CapacityServerPolicy {
                enforcement: CapacityEnforcement::Observe,
                ..CapacityServerPolicy::default()
            },
            logout: LogoutServerPolicy::default(),
            automation: AutomationServerPolicy::default(),
            login_method_order: Vec::new(),
            providers: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_default(provider: Option<Arc<OidcProvider>>) -> Self {
        let mut providers = BTreeMap::new();
        if let Some(provider) = provider {
            providers.insert(provider.id().to_owned(), provider);
        }
        let provider_ids = providers.keys().cloned().collect::<Vec<_>>();
        Self {
            password_enabled: true,
            passkeys: PasskeyServerPolicy {
                enabled: true,
                prompt_after_login: true,
                session_refresh_enabled: true,
            },
            session: SessionServerPolicy::default(),
            capacity: CapacityServerPolicy::default(),
            logout: LogoutServerPolicy::default(),
            automation: AutomationServerPolicy::default(),
            login_method_order: resolve_login_method_order(None, true, &provider_ids)
                .expect("default login method order"),
            providers,
        }
    }

    pub(crate) fn load(
        path: Option<&Path>,
        catalog: &PluginCatalog,
        legacy: Option<Arc<OidcProvider>>,
    ) -> Result<Self> {
        let document = path
            .map(read_document)
            .transpose()?
            .unwrap_or(AuthenticationDocument {
                schema: CONFIG_SCHEMA_V2.to_owned(),
                password: LoginMethodPolicy::default(),
                passkeys: PasskeyPolicyDocument::default(),
                session: SessionPolicyDocument::default(),
                capacity: None,
                logout: None,
                automation: None,
                login_method_order: None,
                providers: Vec::new(),
            });
        let legacy_schema = document.schema == CONFIG_SCHEMA_V1;
        ensure!(
            legacy_schema || document.schema == CONFIG_SCHEMA_V2,
            "unsupported authentication config"
        );
        ensure!(
            !legacy_schema
                || (document.capacity.is_none()
                    && document.logout.is_none()
                    && document.automation.is_none()),
            "authentication v2 policy fields require the v2 schema"
        );
        ensure!(
            document.providers.len() <= 16,
            "too many Authentication Providers"
        );
        let mut providers = BTreeMap::new();
        for selected in document.providers {
            let contract = catalog.resolve_authentication_provider(
                &selected.plugin_id,
                &selected.plugin_version,
                &selected.artifact_digest,
            )?;
            ensure!(
                contract.id == selected.plugin_id && contract.version == selected.plugin_version,
                "Authentication Provider identity mismatch"
            );
            let provider = Arc::new(OidcProvider::load_plugin(&contract, selected.oidc)?);
            ensure!(
                providers
                    .insert(provider.id().to_owned(), provider)
                    .is_none(),
                "duplicate Authentication Provider"
            );
        }
        if let Some(provider) = legacy {
            providers
                .entry(provider.id().to_owned())
                .or_insert(provider);
        }
        ensure!(
            document.password.enabled || !providers.is_empty(),
            "product authentication requires at least one login method"
        );
        ensure!(
            document.passkeys.enabled || !document.passkeys.prompt_after_login,
            "Passkey setup prompting requires Passkeys to be enabled"
        );
        ensure!(
            document.passkeys.enabled || !document.passkeys.session_refresh_enabled,
            "Passkey session refresh requires Passkeys to be enabled"
        );
        let provider_ids = providers.keys().cloned().collect::<Vec<_>>();
        let login_method_order = resolve_login_method_order(
            document.login_method_order,
            document.password.enabled,
            &provider_ids,
        )?;
        let session = document.session.validate()?;
        let mut capacity = document.capacity.unwrap_or_default().validate()?;
        if legacy_schema {
            // V1 deployments predate capacity accounting. Upgrade them into
            // shadow mode so a controller rollout cannot evict active users.
            capacity.enforcement = CapacityEnforcement::Observe;
        }
        let logout_document = document.logout.unwrap_or_default();
        let logout = LogoutServerPolicy {
            provider_logout: logout_document.provider_logout,
            backchannel_logout: logout_document.backchannel_logout,
        };
        let automation = document.automation.unwrap_or_default().validate()?;
        Ok(Self {
            password_enabled: document.password.enabled,
            passkeys: PasskeyServerPolicy {
                enabled: document.passkeys.enabled,
                prompt_after_login: document.passkeys.prompt_after_login,
                session_refresh_enabled: document.passkeys.session_refresh_enabled,
            },
            session,
            capacity,
            logout,
            automation,
            login_method_order,
            providers,
        })
    }

    pub(crate) fn provider(&self, id: &str) -> Option<&Arc<OidcProvider>> {
        self.providers.get(id)
    }

    pub(crate) fn public_providers(&self) -> Vec<crate::oidc::PublicProvider> {
        self.providers
            .values()
            .map(|provider| provider.public())
            .collect()
    }

    pub(crate) fn login_method_order(&self) -> &[String] {
        &self.login_method_order
    }
}

fn resolve_login_method_order(
    configured: Option<Vec<String>>,
    password_enabled: bool,
    provider_ids: &[String],
) -> Result<Vec<String>> {
    ensure!(
        provider_ids.iter().all(|id| id != PASSWORD_LOGIN_METHOD),
        "Authentication Provider ID password is reserved"
    );
    let mut default_order = Vec::with_capacity(provider_ids.len() + usize::from(password_enabled));
    if provider_ids.iter().any(|id| id == "cardea") {
        default_order.push("cardea".to_owned());
    }
    if password_enabled {
        default_order.push(PASSWORD_LOGIN_METHOD.to_owned());
    }
    default_order.extend(
        provider_ids
            .iter()
            .filter(|id| id.as_str() != "cardea")
            .cloned(),
    );

    let Some(configured) = configured else {
        return Ok(default_order);
    };
    let configured_set = configured.iter().cloned().collect::<BTreeSet<_>>();
    ensure!(
        configured_set.len() == configured.len(),
        "login method order contains a duplicate"
    );
    let available_set = default_order.iter().cloned().collect::<BTreeSet<_>>();
    ensure!(
        configured_set == available_set,
        "login method order must contain every enabled method exactly once"
    );
    Ok(configured)
}

fn default_true() -> bool {
    true
}

fn default_capacity_enforcement() -> CapacityEnforcement {
    CapacityServerPolicy::default().enforcement
}

fn default_authorized_clients_per_user() -> u32 {
    CapacityServerPolicy::default().authorized_clients_per_user
}

fn default_signed_in_sessions_per_user() -> u32 {
    CapacityServerPolicy::default().signed_in_sessions_per_user
}

fn default_active_clients_per_user() -> u32 {
    CapacityServerPolicy::default().active_clients_per_user
}

fn default_active_clients_service() -> u32 {
    CapacityServerPolicy::default().active_clients_service
}

fn default_websocket_channels_per_client() -> u32 {
    CapacityServerPolicy::default().websocket_channels_per_client
}

fn default_active_lease_ms() -> i64 {
    CapacityServerPolicy::default().active_lease_ms
}

fn default_heartbeat_ms() -> i64 {
    CapacityServerPolicy::default().heartbeat_ms
}

fn default_reservation_ms() -> i64 {
    CapacityServerPolicy::default().reservation_ms
}

fn default_session_overflow() -> SessionOverflowPolicy {
    CapacityServerPolicy::default().session_overflow
}

fn default_active_overflow() -> ActiveOverflowPolicy {
    CapacityServerPolicy::default().active_overflow
}

fn default_single_session_mode() -> SingleSessionMode {
    CapacityServerPolicy::default().single_session_mode
}

fn default_provider_logout() -> ProviderLogoutPolicy {
    LogoutServerPolicy::default().provider_logout
}

fn default_automation_active_clients() -> u32 {
    AutomationServerPolicy::default().active_clients
}

fn default_automation_credential_max_age_ms() -> i64 {
    AutomationServerPolicy::default().credential_max_age_ms
}

fn default_idle_timeout_ms() -> i64 {
    SessionServerPolicy::default().idle_timeout_ms
}

fn default_passkey_max_age_ms() -> i64 {
    SessionServerPolicy::default().passkey_max_age_ms
}

fn default_passkey_warning_ms() -> i64 {
    SessionServerPolicy::default().passkey_warning_ms
}

fn default_primary_max_age_ms() -> i64 {
    SessionServerPolicy::default().primary_max_age_ms
}

fn default_primary_warning_ms() -> i64 {
    SessionServerPolicy::default().primary_warning_ms
}

fn read_document(path: &Path) -> Result<AuthenticationDocument> {
    ensure!(
        path.is_absolute(),
        "authentication config path must be absolute"
    );
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("opening authentication config {}", path.display()))?;
    let metadata = file
        .metadata()
        .context("inspecting authentication config")?;
    ensure!(
        metadata.is_file(),
        "authentication config must be a regular file"
    );
    ensure!(
        metadata.mode() & 0o077 == 0,
        "authentication config permissions are too broad"
    );
    ensure!(
        metadata.len() <= MAX_CONFIG_BYTES,
        "authentication config is too large"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_CONFIG_BYTES,
        "authentication config is too large"
    );
    serde_json::from_slice(&bytes).context("decoding authentication config")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_keeps_password_and_passkeys_available() {
        let password = LoginMethodPolicy::default();
        let passkeys = PasskeyPolicyDocument::default();
        assert!(password.enabled);
        assert!(passkeys.enabled);
        assert!(passkeys.prompt_after_login);
        assert!(passkeys.session_refresh_enabled);
        assert_eq!(
            SessionServerPolicy::default().passkey_max_age_ms,
            3 * 24 * 60 * 60 * 1_000
        );
        assert_eq!(
            SessionServerPolicy::default().passkey_warning_ms,
            30 * 60 * 1_000
        );
        assert_eq!(
            SessionServerPolicy::default().primary_warning_ms,
            24 * 60 * 60 * 1_000
        );
    }

    #[test]
    fn session_policy_rejects_unsafe_or_confusing_windows() {
        let mut document = SessionPolicyDocument::default();
        document.passkey_warning_ms = document.passkey_max_age_ms;
        assert!(
            document
                .validate()
                .unwrap_err()
                .to_string()
                .contains("warning")
        );

        let document = SessionPolicyDocument {
            primary_max_age_ms: 91 * 24 * 60 * 60 * 1_000,
            ..SessionPolicyDocument::default()
        };
        assert!(
            document
                .validate()
                .unwrap_err()
                .to_string()
                .contains("maximum age")
        );
    }

    #[test]
    fn default_login_order_prefers_cardea_without_reordering_other_servers() {
        assert_eq!(
            resolve_login_method_order(None, true, &["cardea".to_owned(), "google".to_owned()],)
                .unwrap(),
            ["cardea", "password", "google"]
        );
        assert_eq!(
            resolve_login_method_order(None, true, &["google".to_owned()]).unwrap(),
            ["password", "google"]
        );
    }

    #[test]
    fn configured_login_order_is_an_exact_permutation() {
        let providers = ["cardea".to_owned(), "google".to_owned()];
        assert_eq!(
            resolve_login_method_order(
                Some(vec![
                    "google".to_owned(),
                    "cardea".to_owned(),
                    "password".to_owned(),
                ]),
                true,
                &providers,
            )
            .unwrap(),
            ["google", "cardea", "password"]
        );
        assert!(
            resolve_login_method_order(
                Some(vec!["cardea".to_owned(), "cardea".to_owned()]),
                false,
                &providers,
            )
            .unwrap_err()
            .to_string()
            .contains("duplicate")
        );
        assert!(
            resolve_login_method_order(
                Some(vec!["cardea".to_owned(), "password".to_owned()]),
                true,
                &providers,
            )
            .unwrap_err()
            .to_string()
            .contains("every enabled method")
        );
        assert!(
            resolve_login_method_order(None, true, &["password".to_owned()])
                .unwrap_err()
                .to_string()
                .contains("reserved")
        );
    }
}
