//! Cowboy admin console policy: registration, permissions, and session limits.
//!
//! Product register consumes this policy. Public clients see
//! [`RegistrationPublicStatus`] only — never the invite table.

#![warn(clippy::pedantic)]
#![cfg_attr(not(test), allow(dead_code))]

use std::io::Read as _;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub const REGISTRATION_SETTING: &str = "cowboy.registration";
pub const PERMISSIONS_SETTING: &str = "cowboy.permissions";
#[allow(dead_code)]
pub const SESSION_LIMITS_SETTING: &str = "cowboy.session_limits";
pub const ADMIN_IDENTITIES_SETTING: &str = "cowboy.admin.identities";

/// Keys that persist on the Hub but must never appear on product `/ws`.
#[must_use]
pub fn is_admin_setting_key(key: &str) -> bool {
    matches!(
        key,
        REGISTRATION_SETTING
            | PERMISSIONS_SETTING
            | SESSION_LIMITS_SETTING
            | ADMIN_IDENTITIES_SETTING
    )
}

pub const ADMIN_SESSION_COOKIE: &str = "cowboy_admin";
/// Absolute admin session lifetime. Shorter than the product cookie.
pub const ADMIN_SESSION_TTL_SECS: i64 = 12 * 3_600;
const ADMIN_SESSION_TTL_MS: i64 = ADMIN_SESSION_TTL_SECS * 1_000;

/// Synapse-shaped registration switch. The service, not the client, decides.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationMode {
    #[default]
    Disabled,
    Token,
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationToken {
    pub id: String,
    pub name: String,
    pub token_prefix: String,
    pub token_hash: String,
    pub uses_allowed: Option<u32>,
    pub uses_count: u32,
    pub expires_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationPolicy {
    pub enabled: bool,
    pub mode: RegistrationMode,
    #[serde(default)]
    pub tokens: Vec<RegistrationToken>,
}

impl Default for RegistrationPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: RegistrationMode::Disabled,
            tokens: Vec::new(),
        }
    }
}

impl RegistrationPolicy {
    pub fn from_setting(value: Option<&serde_json::Value>) -> Self {
        value
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn public_view(&self) -> RegistrationPolicyView {
        RegistrationPolicyView {
            enabled: self.enabled,
            mode: self.mode,
            accepts_registration: self.accepts_registration(),
            tokens: self
                .tokens
                .iter()
                .map(RegistrationTokenView::from)
                .collect(),
        }
    }

    /// Three-field public view. Never serialize tokens here.
    #[must_use]
    pub fn public_status(&self) -> RegistrationPublicStatus {
        RegistrationPublicStatus {
            enabled: self.enabled,
            mode: self.mode,
            accepts_registration: self.accepts_registration(),
        }
    }

    #[must_use]
    pub fn accepts_registration(&self) -> bool {
        self.enabled && !matches!(self.mode, RegistrationMode::Disabled)
    }
}

/// Product-visible registration flags. No invite table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationPublicStatus {
    pub enabled: bool,
    pub mode: RegistrationMode,
    pub accepts_registration: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistrationPolicyView {
    pub enabled: bool,
    pub mode: RegistrationMode,
    pub accepts_registration: bool,
    pub tokens: Vec<RegistrationTokenView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistrationTokenView {
    pub id: String,
    pub name: String,
    pub token_prefix: String,
    pub uses_allowed: Option<u32>,
    pub uses_count: u32,
    pub expires_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub disabled: bool,
}

impl From<&RegistrationToken> for RegistrationTokenView {
    fn from(token: &RegistrationToken) -> Self {
        Self {
            id: token.id.clone(),
            name: token.name.clone(),
            token_prefix: token.token_prefix.clone(),
            uses_allowed: token.uses_allowed,
            uses_count: token.uses_count,
            expires_at_ms: token.expires_at_ms,
            created_at_ms: token.created_at_ms,
            disabled: token.disabled,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RegistrationPolicyPatch {
    pub enabled: bool,
    pub mode: RegistrationMode,
}

#[derive(Debug, Deserialize)]
pub struct CreateRegistrationToken {
    pub name: String,
    pub uses_allowed: Option<u32>,
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreatedRegistrationToken {
    pub token: String,
    pub record: RegistrationTokenView,
}

/// Why [`consume_registration_token`] refused the attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumeRegistrationError {
    Disabled,
    InvalidToken,
}

impl std::fmt::Display for ConsumeRegistrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(formatter, "registration is disabled by the service"),
            Self::InvalidToken => write!(formatter, "invalid registration token"),
        }
    }
}

/// Pure mutation of a policy value. Callers persist the result under the
/// settings lock. Does not I/O or take the Hub mutex.
///
/// # Errors
/// Returns [`ConsumeRegistrationError::Disabled`] when the switch is off, or
/// [`ConsumeRegistrationError::InvalidToken`] in token mode when the token is
/// missing, unknown, expired, disabled, or exhausted.
pub fn consume_registration_token(
    policy: &mut RegistrationPolicy,
    plaintext: Option<&str>,
    now_ms: i64,
) -> Result<(), ConsumeRegistrationError> {
    if !policy.accepts_registration() {
        return Err(ConsumeRegistrationError::Disabled);
    }
    if policy.mode == RegistrationMode::Open {
        return Ok(());
    }
    let Some(plaintext) = plaintext.filter(|token| !token.is_empty()) else {
        return Err(ConsumeRegistrationError::InvalidToken);
    };
    let token_hash = hex_sha256(plaintext.as_bytes());
    let token = policy.tokens.iter_mut().find(|token| {
        token.token_hash == token_hash
            && !token.disabled
            && token.expires_at_ms.is_none_or(|expires| expires > now_ms)
            && token
                .uses_allowed
                .is_none_or(|allowed| token.uses_count < allowed)
    });
    let Some(token) = token else {
        return Err(ConsumeRegistrationError::InvalidToken);
    };
    token.uses_count = token.uses_count.saturating_add(1);
    Ok(())
}

pub fn apply_policy_patch(policy: &mut RegistrationPolicy, patch: &RegistrationPolicyPatch) {
    policy.enabled = patch.enabled;
    policy.mode = if patch.enabled {
        patch.mode
    } else {
        RegistrationMode::Disabled
    };
}

pub fn issue_registration_token(
    policy: &mut RegistrationPolicy,
    request: &CreateRegistrationToken,
    now_ms: i64,
) -> Result<CreatedRegistrationToken> {
    anyhow::ensure!(policy.enabled, "registration is disabled by the service");
    anyhow::ensure!(
        policy.mode == RegistrationMode::Token,
        "registration tokens are only issued in token mode"
    );
    let name = request.name.trim();
    anyhow::ensure!(!name.is_empty(), "token name cannot be empty");
    anyhow::ensure!(name.len() <= 64, "token name is too long");
    if let Some(uses) = request.uses_allowed {
        anyhow::ensure!((1..=10_000).contains(&uses), "uses_allowed must be 1-10000");
    }
    if let Some(ttl) = request.ttl_seconds {
        anyhow::ensure!((60..=365 * 86_400).contains(&ttl), "ttl must be 60s-365d");
    }
    let mut random = [0_u8; 24];
    std::fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut random)
        .context("reading OS randomness")?;
    let token = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, random);
    let token_hash = hex_sha256(token.as_bytes());
    let id = token_hash[..16].to_owned();
    let record = RegistrationToken {
        id: id.clone(),
        name: name.to_owned(),
        token_prefix: token.chars().take(6).collect(),
        token_hash,
        uses_allowed: request.uses_allowed,
        uses_count: 0,
        expires_at_ms: request
            .ttl_seconds
            .map(|ttl| now_ms.saturating_add(ttl.saturating_mul(1_000))),
        created_at_ms: now_ms,
        disabled: false,
    };
    policy.tokens.insert(0, record.clone());
    Ok(CreatedRegistrationToken {
        token,
        record: RegistrationTokenView::from(&record),
    })
}

pub fn disable_registration_token(policy: &mut RegistrationPolicy, token_id: &str) -> Result<()> {
    let token = policy
        .tokens
        .iter_mut()
        .find(|token| token.id == token_id)
        .context("unknown registration token")?;
    token.disabled = true;
    Ok(())
}

pub(crate) fn hex_sha256(value: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(value)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            use std::fmt::Write as _;
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminRole {
    Owner,
    Operator,
    #[default]
    Viewer,
}

impl AdminRole {
    #[must_use]
    pub fn at_least(self, minimum: Self) -> bool {
        self.rank() >= minimum.rank()
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Viewer => 0,
            Self::Operator => 1,
            Self::Owner => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionGrant {
    pub account: String,
    pub role: AdminRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionPolicy {
    pub default_role: AdminRole,
    #[serde(default)]
    pub grants: Vec<PermissionGrant>,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self {
            default_role: AdminRole::Viewer,
            grants: Vec::new(),
        }
    }
}

impl PermissionPolicy {
    pub fn from_setting(value: Option<&serde_json::Value>) -> Self {
        value
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    pub fn apply_patch(&mut self, patch: PermissionPolicy) -> Result<()> {
        anyhow::ensure!(
            matches!(
                patch.default_role,
                AdminRole::Owner | AdminRole::Operator | AdminRole::Viewer
            ),
            "invalid default role"
        );
        for grant in &patch.grants {
            let account = grant.account.trim();
            anyhow::ensure!(!account.is_empty(), "grant account cannot be empty");
            anyhow::ensure!(account.len() <= 64, "grant account is too long");
        }
        self.default_role = patch.default_role;
        self.grants = patch
            .grants
            .into_iter()
            .map(|grant| PermissionGrant {
                account: grant.account.trim().to_ascii_lowercase(),
                role: grant.role,
            })
            .collect();
        Ok(())
    }

    pub fn upsert_grant(&mut self, account: &str, role: AdminRole) {
        let account = account.trim().to_ascii_lowercase();
        self.grants.retain(|grant| grant.account != account);
        self.grants.push(PermissionGrant { account, role });
    }

    #[must_use]
    pub fn role_for(&self, account: &str) -> AdminRole {
        let account = account.trim().to_ascii_lowercase();
        self.grants
            .iter()
            .find(|grant| grant.account == account)
            .map_or(self.default_role, |grant| grant.role)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminAccount {
    pub account: String,
    pub role: AdminRole,
    /// Empty when `password_hash` is an argon2id PHC string.
    #[serde(default)]
    pub password_salt: String,
    pub password_hash: String,
    pub created_at_ms: i64,
    #[serde(default = "default_true")]
    pub passkey_reauth_enabled: bool,
    #[serde(default)]
    pub last_step_up_at_ms: Option<i64>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminSessionRecord {
    pub token_hash: String,
    pub account: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminIdentities {
    #[serde(default)]
    pub accounts: Vec<AdminAccount>,
    #[serde(default)]
    pub sessions: Vec<AdminSessionRecord>,
}

#[allow(dead_code, clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize)]
pub struct AdminAuthStatus {
    pub authenticated: bool,
    pub bootstrap_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<AdminRole>,
    #[serde(default)]
    pub passkey_count: u32,
    #[serde(default = "default_true")]
    pub passkey_reauth_enabled: bool,
    #[serde(default)]
    pub passkey_reauth_required: bool,
}

#[derive(Debug, Deserialize)]
pub struct AdminCredentials {
    pub account: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct AdminPrincipal {
    pub account: String,
    pub role: AdminRole,
}

impl AdminIdentities {
    pub fn from_setting(value: Option<&serde_json::Value>) -> Self {
        value
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn bootstrap_required(&self) -> bool {
        self.accounts.is_empty()
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn status(&self, token: Option<&str>, now_ms: i64) -> AdminAuthStatus {
        match token.and_then(|token| self.principal(token, now_ms)) {
            Some(principal) => {
                let account = self
                    .accounts
                    .iter()
                    .find(|account| account.account == principal.account);
                AdminAuthStatus {
                    authenticated: true,
                    bootstrap_required: false,
                    account: Some(principal.account),
                    role: Some(principal.role),
                    passkey_count: 0,
                    passkey_reauth_enabled: account
                        .is_none_or(|account| account.passkey_reauth_enabled),
                    passkey_reauth_required: false,
                }
            }
            None => AdminAuthStatus {
                authenticated: false,
                bootstrap_required: self.bootstrap_required(),
                account: None,
                role: None,
                passkey_count: 0,
                passkey_reauth_enabled: true,
                passkey_reauth_required: false,
            },
        }
    }

    #[must_use]
    pub fn principal(&self, token: &str, now_ms: i64) -> Option<AdminPrincipal> {
        let token_hash = hex_sha256(token.as_bytes());
        let session = self
            .sessions
            .iter()
            .find(|session| session.token_hash == token_hash && session.expires_at_ms > now_ms)?;
        let account = self
            .accounts
            .iter()
            .find(|account| account.account == session.account)?;
        Some(AdminPrincipal {
            account: account.account.clone(),
            role: account.role,
        })
    }

    pub fn bootstrap(&mut self, request: &AdminCredentials, now_ms: i64) -> Result<String> {
        anyhow::ensure!(self.bootstrap_required(), "admin owner already exists");
        self.create_account(request, AdminRole::Owner, now_ms)?;
        self.login(request, now_ms)
    }

    pub fn create_account(
        &mut self,
        request: &AdminCredentials,
        role: AdminRole,
        now_ms: i64,
    ) -> Result<()> {
        let account = normalize_admin_account(&request.account)?;
        ensure_admin_password(&request.password, &account)?;
        anyhow::ensure!(
            !self
                .accounts
                .iter()
                .any(|existing| existing.account == account),
            "admin account already exists"
        );
        let password_hash = crate::product_auth::hash_password(&request.password)
            .context("hashing admin password")?;
        self.accounts.push(AdminAccount {
            account,
            role,
            password_salt: String::new(),
            password_hash,
            created_at_ms: now_ms,
            passkey_reauth_enabled: true,
            last_step_up_at_ms: Some(now_ms),
        });
        Ok(())
    }

    pub fn login(&mut self, request: &AdminCredentials, now_ms: i64) -> Result<String> {
        let Ok(account) = normalize_admin_account(&request.account) else {
            let _ = crate::product_auth::verify_unknown_user_password(&request.password);
            anyhow::bail!("invalid admin credentials");
        };
        let Some(index) = self
            .accounts
            .iter()
            .position(|existing| existing.account == account)
        else {
            let _ = crate::product_auth::verify_unknown_user_password(&request.password);
            anyhow::bail!("invalid admin credentials");
        };
        if !verify_stored_admin_password(&self.accounts[index], &request.password) {
            anyhow::bail!("invalid admin credentials");
        }
        if !is_argon2id_phc(&self.accounts[index].password_hash)
            && let Ok(hash) = crate::product_auth::hash_password(&request.password)
        {
            self.accounts[index].password_salt.clear();
            self.accounts[index].password_hash = hash;
        }
        self.issue_session(&account, now_ms)
    }

    fn issue_session(&mut self, account: &str, now_ms: i64) -> Result<String> {
        let mut token_bytes = [0_u8; 32];
        std::fs::File::open("/dev/urandom")
            .context("opening OS randomness")?
            .read_exact(&mut token_bytes)
            .context("reading OS randomness")?;
        let token = hex_bytes(&token_bytes);
        self.sessions
            .retain(|session| session.expires_at_ms > now_ms && session.account != account);
        self.sessions.push(AdminSessionRecord {
            token_hash: hex_sha256(token.as_bytes()),
            account: account.to_owned(),
            expires_at_ms: now_ms.saturating_add(ADMIN_SESSION_TTL_MS),
        });
        self.touch_last_step_up(account, now_ms);
        Ok(token)
    }

    pub fn touch_last_step_up(&mut self, account: &str, now_ms: i64) {
        if let Some(stored) = self
            .accounts
            .iter_mut()
            .find(|existing| existing.account == account)
        {
            stored.last_step_up_at_ms = Some(now_ms);
        }
    }

    pub fn set_passkey_reauth(&mut self, account: &str, enabled: bool) -> Result<()> {
        let stored = self
            .accounts
            .iter_mut()
            .find(|existing| existing.account == account)
            .context("admin account not found")?;
        stored.passkey_reauth_enabled = enabled;
        Ok(())
    }

    #[must_use]
    pub fn passkey_policy(
        &self,
        account: &str,
        passkey_count: u32,
    ) -> crate::passkey::PasskeyPolicy {
        let stored = self
            .accounts
            .iter()
            .find(|existing| existing.account == account);
        crate::passkey::PasskeyPolicy {
            enabled: stored.is_none_or(|account| account.passkey_reauth_enabled),
            last_step_up_at_ms: stored.and_then(|account| account.last_step_up_at_ms),
            passkey_count,
        }
    }

    #[allow(dead_code)]
    pub fn logout(&mut self, token: &str) {
        let token_hash = hex_sha256(token.as_bytes());
        self.sessions
            .retain(|session| session.token_hash != token_hash);
    }
}

fn normalize_admin_account(account: &str) -> Result<String> {
    let account = account.trim().to_ascii_lowercase();
    anyhow::ensure!(
        (1..=64).contains(&account.len())
            && account.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'-'
                || byte == b'_'
                || byte == b'.'),
        "admin account must be 1-64 lowercase letters, digits, '.', '_' or '-'"
    );
    Ok(account)
}

fn ensure_admin_password(password: &str, account: &str) -> Result<()> {
    anyhow::ensure!(
        (12..=128).contains(&password.len()),
        "admin password must be 12-128 characters"
    );
    anyhow::ensure!(password != account, "admin password cannot be the account");
    Ok(())
}

fn is_argon2id_phc(password_hash: &str) -> bool {
    password_hash.starts_with("$argon2id$")
}

fn verify_stored_admin_password(account: &AdminAccount, password: &str) -> bool {
    if is_argon2id_phc(&account.password_hash) {
        return crate::product_auth::verify_password(password, &account.password_hash);
    }
    let Ok(salt) = decode_hex(&account.password_salt) else {
        let _ = crate::product_auth::verify_unknown_user_password(password);
        return false;
    };
    hashes_equal(
        &hash_admin_password(&salt, password),
        &account.password_hash,
    )
}

fn hashes_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn hash_admin_password(salt: &[u8], password: &str) -> String {
    use sha2::Digest as _;
    let mut digest = sha2::Sha256::digest([salt, password.as_bytes()].concat());
    for _ in 0..100_000 {
        digest = sha2::Sha256::digest([digest.as_slice(), salt].concat());
    }
    hex_bytes(&digest)
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            use std::fmt::Write as _;
            write!(out, "{byte:02x}").expect("writing to a String cannot fail");
            out
        })
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(value.len().is_multiple_of(2), "invalid hex");
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).context("invalid hex"))
        .collect()
}

pub fn cookie_token(headers: &axum::http::HeaderMap) -> Option<String> {
    crate::product_auth::cookie_value(headers, ADMIN_SESSION_COOKIE)
}

#[must_use]
pub fn session_cookie(token: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{ADMIN_SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ADMIN_SESSION_TTL_SECS}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[must_use]
pub fn clear_session_cookie(secure: bool) -> String {
    let mut cookie =
        format!("{ADMIN_SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Event/session retention. `last_n` and `last_time_hours` are an OR:
/// an event is kept if it is in the newest N **or** newer than the time window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLimits {
    pub max_sessions: Option<u32>,
    pub max_retention_days: Option<u32>,
    pub last_n: Option<u32>,
    pub last_time_hours: Option<u32>,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_sessions: None,
            max_retention_days: None,
            last_n: Some(u32::try_from(crate::core::HOT_TAIL).unwrap_or(1_000)),
            last_time_hours: None,
        }
    }
}

#[allow(dead_code)]
impl SessionLimits {
    pub fn from_setting(value: Option<&serde_json::Value>) -> Self {
        value
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    pub fn apply_patch(&mut self, patch: SessionLimits) -> Result<()> {
        if let Some(max_sessions) = patch.max_sessions {
            anyhow::ensure!(
                (1..=10_000).contains(&max_sessions),
                "max_sessions must be 1-10000"
            );
        }
        if let Some(days) = patch.max_retention_days {
            anyhow::ensure!(
                (1..=3650).contains(&days),
                "max_retention_days must be 1-3650"
            );
        }
        if let Some(last_n) = patch.last_n {
            anyhow::ensure!((1..=100_000).contains(&last_n), "last_n must be 1-100000");
        }
        if let Some(hours) = patch.last_time_hours {
            anyhow::ensure!(
                (1..=24 * 365 * 10).contains(&hours),
                "last_time_hours must be 1 hour-10 years"
            );
        }
        *self = patch;
        Ok(())
    }

    #[must_use]
    pub fn keeps_event(&self, age_from_newest: u32, age_hours: u32) -> bool {
        let by_n = self.last_n.is_none_or(|limit| age_from_newest < limit);
        let by_time = self.last_time_hours.is_none_or(|limit| age_hours <= limit);
        match (self.last_n, self.last_time_hours) {
            (None, None) => true,
            (Some(_), None) => by_n,
            (None, Some(_)) => by_time,
            (Some(_), Some(_)) => by_n || by_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_rejects_registration() {
        let policy = RegistrationPolicy::default();
        assert!(!policy.enabled);
        assert_eq!(policy.mode, RegistrationMode::Disabled);
        assert!(!policy.accepts_registration());
        let status = policy.public_status();
        assert!(!status.enabled);
        assert!(!status.accepts_registration);
        let json = serde_json::to_value(&status).expect("status serializes");
        let object = json.as_object().expect("object");
        assert_eq!(object.len(), 3);
        assert!(object.get("tokens").is_none());
    }

    #[test]
    fn consume_fails_closed_when_disabled() {
        let mut policy = RegistrationPolicy::default();
        assert_eq!(
            consume_registration_token(&mut policy, Some("invite"), 1),
            Err(ConsumeRegistrationError::Disabled)
        );
    }

    #[test]
    fn open_mode_ignores_token() {
        let mut policy = RegistrationPolicy {
            enabled: true,
            mode: RegistrationMode::Open,
            tokens: Vec::new(),
        };
        consume_registration_token(&mut policy, Some("ignored"), 1).unwrap();
        consume_registration_token(&mut policy, None, 1).unwrap();
        assert!(policy.tokens.is_empty());
    }

    #[test]
    fn token_mode_increments_until_exhausted() {
        let mut policy = RegistrationPolicy {
            enabled: true,
            mode: RegistrationMode::Token,
            tokens: Vec::new(),
        };
        let created = issue_registration_token(
            &mut policy,
            &CreateRegistrationToken {
                name: "invite".to_owned(),
                uses_allowed: Some(1),
                ttl_seconds: Some(600),
            },
            1_900_000_000_000,
        )
        .unwrap();
        consume_registration_token(&mut policy, Some(&created.token), 1_900_000_000_001).unwrap();
        assert_eq!(policy.tokens[0].uses_count, 1);
        assert_eq!(
            consume_registration_token(&mut policy, Some(&created.token), 1_900_000_000_002),
            Err(ConsumeRegistrationError::InvalidToken)
        );
        assert_eq!(
            consume_registration_token(&mut policy, None, 1_900_000_000_003),
            Err(ConsumeRegistrationError::InvalidToken)
        );
    }

    #[test]
    fn disabled_switch_overrides_open_mode() {
        let mut policy = RegistrationPolicy {
            enabled: true,
            mode: RegistrationMode::Open,
            tokens: Vec::new(),
        };
        apply_policy_patch(
            &mut policy,
            &RegistrationPolicyPatch {
                enabled: false,
                mode: RegistrationMode::Open,
            },
        );
        assert!(!policy.accepts_registration());
        assert_eq!(policy.mode, RegistrationMode::Disabled);
    }

    #[test]
    fn token_mode_can_issue_and_disable() {
        let mut policy = RegistrationPolicy {
            enabled: true,
            mode: RegistrationMode::Token,
            tokens: Vec::new(),
        };
        let created = issue_registration_token(
            &mut policy,
            &CreateRegistrationToken {
                name: "invite".to_owned(),
                uses_allowed: Some(3),
                ttl_seconds: Some(600),
            },
            1_900_000_000_000,
        )
        .unwrap();
        assert_eq!(created.record.name, "invite");
        assert_eq!(created.record.uses_allowed, Some(3));
        assert!(created.token.starts_with(&created.record.token_prefix));
        assert!(!policy.public_view().tokens[0].id.is_empty());
        disable_registration_token(&mut policy, &created.record.id).unwrap();
        assert!(policy.tokens[0].disabled);
    }

    #[test]
    fn session_limits_keep_last_n_or_last_time() {
        let limits = SessionLimits {
            max_sessions: Some(20),
            max_retention_days: Some(30),
            last_n: Some(10),
            last_time_hours: Some(24),
        };
        assert!(limits.keeps_event(0, 100));
        assert!(limits.keeps_event(50, 1));
        assert!(!limits.keeps_event(50, 48));
    }

    #[test]
    fn permission_grants_override_default_role() {
        let mut policy = PermissionPolicy::default();
        policy
            .apply_patch(PermissionPolicy {
                default_role: AdminRole::Viewer,
                grants: vec![PermissionGrant {
                    account: " Draven ".to_owned(),
                    role: AdminRole::Owner,
                }],
            })
            .unwrap();
        assert_eq!(policy.grants[0].account, "draven");
        assert_eq!(policy.role_for("draven"), AdminRole::Owner);
        assert_eq!(policy.role_for("DRAVEN"), AdminRole::Owner);
        assert_eq!(policy.role_for("guest"), AdminRole::Viewer);
    }

    #[test]
    fn admin_created_grant_defaults_to_operator() {
        let mut policy = PermissionPolicy::default();
        policy.upsert_grant("Draven", AdminRole::Operator);
        assert_eq!(policy.role_for("draven"), AdminRole::Operator);
    }

    #[test]
    fn first_admin_must_bootstrap_then_login() {
        let mut identities = AdminIdentities::default();
        assert!(identities.bootstrap_required());
        let credentials = AdminCredentials {
            account: "Owner".to_owned(),
            password: "correct-horse".to_owned(),
        };
        let token = identities
            .bootstrap(&credentials, 1_900_000_000_000)
            .unwrap();
        assert!(!identities.bootstrap_required());
        let principal = identities.principal(&token, 1_900_000_000_001).unwrap();
        assert_eq!(principal.account, "owner");
        assert_eq!(principal.role, AdminRole::Owner);
        assert!(
            identities
                .login(
                    &AdminCredentials {
                        account: "owner".to_owned(),
                        password: "wrong-pass".to_owned(),
                    },
                    1_900_000_000_002,
                )
                .is_err()
        );
        assert!(
            identities.accounts[0]
                .password_hash
                .starts_with("$argon2id$")
        );
        assert!(identities.accounts[0].password_salt.is_empty());
    }

    #[test]
    fn admin_password_rejects_short_and_account_name() {
        assert!(ensure_admin_password("short-pass", "owner").is_err());
        assert!(ensure_admin_password("ownerrrrrrrr", "ownerrrrrrrr").is_err());
        assert!(ensure_admin_password("correct-horse", "owner").is_ok());
    }

    #[test]
    fn legacy_sha256_admin_login_upgrades_to_argon2id_and_replaces_sessions() {
        let mut salt = [0_u8; 16];
        salt[0] = 7;
        let mut identities = AdminIdentities {
            accounts: vec![AdminAccount {
                account: "owner".to_owned(),
                role: AdminRole::Owner,
                password_salt: hex_bytes(&salt),
                password_hash: hash_admin_password(&salt, "correct-horse"),
                created_at_ms: 1,
                passkey_reauth_enabled: true,
                last_step_up_at_ms: None,
            }],
            sessions: vec![AdminSessionRecord {
                token_hash: hex_sha256(b"old-session"),
                account: "owner".to_owned(),
                expires_at_ms: 1_900_000_100_000,
            }],
        };
        assert!(!is_argon2id_phc(&identities.accounts[0].password_hash));
        let token = identities
            .login(
                &AdminCredentials {
                    account: "owner".to_owned(),
                    password: "correct-horse".to_owned(),
                },
                1_900_000_000_000,
            )
            .unwrap();
        assert!(
            identities.accounts[0]
                .password_hash
                .starts_with("$argon2id$")
        );
        assert!(identities.accounts[0].password_salt.is_empty());
        assert_eq!(identities.sessions.len(), 1);
        assert!(identities.principal(&token, 1_900_000_000_001).is_some());
        assert!(
            identities
                .principal("old-session", 1_900_000_000_001)
                .is_none()
        );
    }

    #[test]
    fn unknown_admin_login_is_generic() {
        let mut identities = AdminIdentities::default();
        let error = identities
            .login(
                &AdminCredentials {
                    account: "nobody".to_owned(),
                    password: "correct-horse".to_owned(),
                },
                1,
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "invalid admin credentials");
    }
}
