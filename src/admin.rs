//! Cowboy admin console policy: registration, permissions, and session limits.
//!
//! Product register consumes this policy. Public clients see
//! [`RegistrationPublicStatus`] only — never the invite table.

#![warn(clippy::pedantic)]
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
/// One-time Portainer-style proof that the caller can read the host data dir.
pub const ADMIN_SETUP_COOKIE: &str = "cowboy_admin_setup";
pub const ADMIN_SETUP_TOKEN_FILE: &str = "admin-setup.token";
pub const ADMIN_SETUP_TOKEN_PREFIX: &str = "cow_setup_";
/// Absolute admin session lifetime. Shorter than the product cookie.
pub const ADMIN_SESSION_TTL_SECS: i64 = 12 * 3_600;
const ADMIN_SESSION_TTL_MS: i64 = ADMIN_SESSION_TTL_SECS * 1_000;
/// Setup cookie only covers the create-admin step after the host token.
pub const ADMIN_SETUP_TTL_SECS: i64 = 10 * 60;
const ADMIN_SETUP_TOKEN_SECRET_BYTES: usize = 32;
/// Single-factor admin create/change floor. Product passwords stay separate.
/// 15 matches Chrome's default generator length.
pub const ADMIN_PASSWORD_MIN_LEN: usize = 15;
pub const ADMIN_PASSWORD_MAX_LEN: usize = 128;

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
    /// SHA-256 hex of the host setup token. Never the secret. Cleared after
    /// the first owner is created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_token_hash: Option<String>,
}

#[allow(dead_code, clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize)]
pub struct AdminAuthStatus {
    pub authenticated: bool,
    pub bootstrap_required: bool,
    /// Host setup token was proven in this browser; create-admin is next.
    #[serde(default)]
    pub setup_pending: bool,
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

#[derive(Debug, Deserialize)]
pub struct AdminSetupRequest {
    pub token: String,
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
                    setup_pending: false,
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
                setup_pending: false,
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
        self.setup_token_hash = None;
        self.login(request, now_ms)
    }

    #[must_use]
    pub fn setup_token_matches(&self, token: &str) -> bool {
        let candidate = hex_sha256(token.trim().as_bytes());
        if let Some(stored) = self.setup_token_hash.as_deref() {
            hashes_equal(stored, &candidate)
        } else {
            let _ = hashes_equal(dummy_setup_token_hash(), &candidate);
            false
        }
    }

    pub fn install_setup_token(&mut self, token: &str) {
        self.setup_token_hash = Some(hex_sha256(token.as_bytes()));
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

    /// Issue an admin session only after an external identity has already been
    /// verified and mapped to this exact, pre-existing Cowboy admin account.
    pub(crate) fn login_federated(&mut self, account: &str, now_ms: i64) -> Result<String> {
        let account = normalize_admin_account(account)?;
        anyhow::ensure!(
            self.accounts
                .iter()
                .any(|existing| existing.account == account),
            "federated admin account not found"
        );
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
            reauth_after_ms: crate::passkey::ADMIN_PASSKEY_REAUTH_AFTER_MS,
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

pub(crate) fn ensure_admin_password(password: &str, account: &str) -> Result<()> {
    let chars = password.chars().count();
    anyhow::ensure!(
        (ADMIN_PASSWORD_MIN_LEN..=ADMIN_PASSWORD_MAX_LEN).contains(&chars),
        "admin password must be 15-128 characters"
    );
    anyhow::ensure!(password != account, "admin password cannot be the account");
    anyhow::ensure!(
        admin_password_has_required_classes(password)
            || looks_like_password_manager_secret(password),
        "admin password needs uppercase, lowercase, and a digit — or use a Chrome / Apple generated password"
    );
    Ok(())
}

fn admin_password_has_required_classes(password: &str) -> bool {
    let mut lower = false;
    let mut upper = false;
    let mut digit = false;
    for char in password.chars() {
        lower |= char.is_ascii_lowercase();
        upper |= char.is_ascii_uppercase();
        digit |= char.is_ascii_digit();
        if lower && upper && digit {
            return true;
        }
    }
    false
}

/// Chrome (`xxxx-xxxx-xxxx`) and Apple Keychain (`xxxxxx-xxxxxx-xxxxxx`)
/// generated secrets. Apple's default is lowercase-only, so class rules
/// would reject it.
fn looks_like_password_manager_secret(password: &str) -> bool {
    let groups: Vec<&str> = password.split('-').collect();
    groups.len() >= 3
        && groups.iter().all(|group| {
            (3..=8).contains(&group.len()) && group.chars().all(|char| char.is_ascii_alphanumeric())
        })
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

#[must_use]
pub fn setup_cookie_token(headers: &axum::http::HeaderMap) -> Option<String> {
    crate::product_auth::cookie_value(headers, ADMIN_SETUP_COOKIE)
}

#[must_use]
pub fn setup_session_cookie(token: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{ADMIN_SETUP_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ADMIN_SETUP_TTL_SECS}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[must_use]
pub fn clear_setup_cookie(secure: bool) -> String {
    let mut cookie = format!("{ADMIN_SETUP_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// In-memory tickets issued after a valid host setup token.
#[derive(Debug, Default)]
pub struct AdminSetupTickets {
    tickets: parking_lot::Mutex<HashMap<String, Instant>>,
}

impl AdminSetupTickets {
    pub fn issue(&self) -> Result<String> {
        let token = new_setup_ticket()?;
        self.tickets.lock().insert(
            hex_sha256(token.as_bytes()),
            Instant::now() + Duration::from_secs(ADMIN_SETUP_TTL_SECS as u64),
        );
        Ok(token)
    }

    #[must_use]
    pub fn is_valid(&self, token: &str) -> bool {
        let hash = hex_sha256(token.as_bytes());
        let mut tickets = self.tickets.lock();
        tickets.retain(|_, expires| Instant::now() < *expires);
        tickets.contains_key(&hash)
    }

    pub fn clear(&self) {
        self.tickets.lock().clear();
    }
}

/// Host-side setup token file plus in-memory create-admin tickets.
#[derive(Debug)]
pub struct AdminSetupState {
    pub data_dir: PathBuf,
    pub tickets: AdminSetupTickets,
}

impl AdminSetupState {
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            tickets: AdminSetupTickets::default(),
        }
    }

    #[must_use]
    pub fn token_path(&self) -> PathBuf {
        self.data_dir.join(ADMIN_SETUP_TOKEN_FILE)
    }
}

/// Create or keep the one-time setup token while no admin owner exists.
///
/// Returns the plaintext only when a **new** token is written. Restarts that
/// still have a matching file do not print it again.
///
/// # Errors
/// Returns when randomness or the data-dir file cannot be written.
pub fn ensure_admin_setup_token(
    data_dir: &Path,
    identities: &mut AdminIdentities,
    needed: bool,
) -> Result<Option<String>> {
    let path = data_dir.join(ADMIN_SETUP_TOKEN_FILE);
    if !needed {
        identities.setup_token_hash = None;
        if path.exists() {
            std::fs::remove_file(&path).context("removing consumed admin setup token")?;
        }
        return Ok(None);
    }
    if let Some(existing) = read_setup_token_file(&path)?
        && identities.setup_token_matches(&existing)
    {
        return Ok(None);
    }
    std::fs::create_dir_all(data_dir).context("creating admin setup token directory")?;
    let token = new_setup_token()?;
    write_setup_token_file(&path, &token)?;
    identities.install_setup_token(&token);
    Ok(Some(token))
}

/// Delete the host setup file after the first owner exists.
///
/// # Errors
/// Returns when the file exists and cannot be removed.
pub fn consume_admin_setup_token_file(data_dir: &Path) -> Result<()> {
    let path = data_dir.join(ADMIN_SETUP_TOKEN_FILE);
    if path.exists() {
        std::fs::remove_file(&path).context("removing consumed admin setup token")?;
    }
    Ok(())
}

fn new_setup_token() -> Result<String> {
    Ok(format!(
        "{ADMIN_SETUP_TOKEN_PREFIX}{}",
        random_hex(ADMIN_SETUP_TOKEN_SECRET_BYTES)?
    ))
}

fn new_setup_ticket() -> Result<String> {
    random_hex(ADMIN_SETUP_TOKEN_SECRET_BYTES)
}

fn random_hex(byte_len: usize) -> Result<String> {
    let mut bytes = vec![0_u8; byte_len];
    std::fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut bytes)
        .context("reading OS randomness")?;
    Ok(hex_bytes(&bytes))
}

fn write_setup_token_file(path: &Path, token: &str) -> Result<()> {
    let tmp = path.with_extension("token.tmp");
    std::fs::write(&tmp, format!("{token}\n")).context("writing admin setup token")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .context("restricting admin setup token permissions")?;
    }
    std::fs::rename(&tmp, path).context("publishing admin setup token")?;
    Ok(())
}

fn read_setup_token_file(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path).context("reading admin setup token")?;
    let token = raw.trim();
    if token.is_empty() {
        return Ok(None);
    }
    Ok(Some(token.to_owned()))
}

const DUMMY_SETUP_TOKEN_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

fn dummy_setup_token_hash() -> &'static str {
    DUMMY_SETUP_TOKEN_HASH
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
            password: "Correct-horse-bat1".to_owned(),
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
    fn setup_token_is_required_before_first_owner_and_never_serialized() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-admin-setup-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut identities = AdminIdentities::default();
        let first = ensure_admin_setup_token(&root, &mut identities, true)
            .unwrap()
            .expect("new setup token");
        assert!(first.starts_with(ADMIN_SETUP_TOKEN_PREFIX));
        assert!(identities.setup_token_matches(&first));
        assert!(!identities.setup_token_matches("cow_setup_wrong"));
        let json = serde_json::to_value(&identities).unwrap();
        assert!(json.get("setup_token_hash").is_some());
        assert!(!json.to_string().contains(&first));
        assert_eq!(
            ensure_admin_setup_token(&root, &mut identities, true).unwrap(),
            None
        );
        let credentials = AdminCredentials {
            account: "owner".to_owned(),
            password: "Correct-horse-bat1".to_owned(),
        };
        identities.bootstrap(&credentials, 1).unwrap();
        ensure_admin_setup_token(&root, &mut identities, false).unwrap();
        assert!(identities.setup_token_hash.is_none());
        assert!(!root.join(ADMIN_SETUP_TOKEN_FILE).exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn admin_password_accepts_chrome_and_apple_generated_secrets() {
        assert!(ensure_admin_password("short-pass", "owner").is_err());
        assert!(ensure_admin_password("ownerrrrrrrrrrr", "ownerrrrrrrrrrr").is_err());
        assert!(ensure_admin_password("alllowercaseword", "owner").is_err());
        assert!(ensure_admin_password("ALLUPPERCASEWORD", "owner").is_err());
        assert!(ensure_admin_password("NoDigitsInHere!!", "owner").is_err());
        assert!(ensure_admin_password("Correct-horse-bat1", "owner").is_ok());
        assert!(ensure_admin_password("kL9mNp2qRs4tUv7", "owner").is_ok());
        assert!(ensure_admin_password("Wq3p-Lm8n-Ks2xY", "owner").is_ok());
        assert!(ensure_admin_password("xidneh-bintun-zygfew", "owner").is_ok());
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
            setup_token_hash: None,
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
