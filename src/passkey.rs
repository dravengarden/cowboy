//! Product-plane WebAuthn / Passkey registration and step-up.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Webauthn, WebauthnBuilder,
};

use crate::product_auth::{new_session_token, new_user_id};

/// Default interval between product Passkey refresh assertions.
pub const DEFAULT_PASSKEY_REAUTH_AFTER_MS: i64 = 24 * 60 * 60 * 1_000;
/// Closed set exposed by the Web and native settings surfaces.
pub const PASSKEY_REAUTH_INTERVALS_MS: [i64; 9] = [
    60 * 60 * 1_000,
    2 * 60 * 60 * 1_000,
    3 * 60 * 60 * 1_000,
    4 * 60 * 60 * 1_000,
    6 * 60 * 60 * 1_000,
    12 * 60 * 60 * 1_000,
    DEFAULT_PASSKEY_REAUTH_AFTER_MS,
    2 * 24 * 60 * 60 * 1_000,
    3 * 24 * 60 * 60 * 1_000,
];
/// Admin console idle lock. Shorter because the console is break-glass.
pub const ADMIN_PASSKEY_REAUTH_AFTER_MS: i64 = 5 * 60 * 1_000;
const CEREMONY_TTL: Duration = Duration::from_secs(300);
pub const EXTERNAL_CEREMONY_TTL_SECS: u64 = 120;
const EXTERNAL_CEREMONY_TTL: Duration = Duration::from_secs(EXTERNAL_CEREMONY_TTL_SECS);
const MAX_PASSKEYS_PER_USER: usize = 8;
const MAX_EXTERNAL_CEREMONIES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PasskeyView {
    pub id: String,
    pub nickname: String,
    pub created_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct UserPasskey {
    pub id: String,
    pub user_id: String,
    pub credential_id: String,
    pub nickname: String,
    pub passkey_json: String,
    pub created_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasskeyPolicy {
    pub enabled: bool,
    pub reauth_after_ms: i64,
    pub last_step_up_at_ms: Option<i64>,
    pub passkey_count: u32,
}

impl PasskeyPolicy {
    /// Whether a client can require a fresh Passkey assertion.
    #[must_use]
    pub fn reauth_eligible(&self) -> bool {
        self.enabled && self.passkey_count > 0
    }
}

#[must_use]
pub fn valid_reauth_interval(value: i64) -> bool {
    PASSKEY_REAUTH_INTERVALS_MS.contains(&value)
}

pub fn normalize_nickname(nickname: &str) -> Result<String> {
    let nickname = nickname.trim();
    anyhow::ensure!(
        (1..=64).contains(&nickname.len()),
        "passkey name must be 1-64 characters"
    );
    Ok(nickname.to_owned())
}

pub fn webauthn_for_request(headers: &HeaderMap) -> Result<Webauthn> {
    let origin = request_webauthn_origin(headers).context("passkey origin is required")?;
    let rp_id = origin
        .host_str()
        .context("passkey origin is missing a host")?
        .to_owned();
    WebauthnBuilder::new(&rp_id, &origin)
        .context("building WebAuthn relying party")?
        .rp_name("Cowboy")
        .build()
        .context("building WebAuthn")
}

fn request_webauthn_origin(headers: &HeaderMap) -> Option<Url> {
    let raw = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())?;
    Url::parse(raw).ok()
}

#[derive(Debug)]
struct StoredRegistration {
    user_id: String,
    nickname: String,
    state: PasskeyRegistration,
    expires: std::time::Instant,
}

#[derive(Debug)]
struct StoredAssertion {
    user_id: String,
    state: PasskeyAuthentication,
    expires: std::time::Instant,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalPasskeyAction {
    Register,
    Assert,
}

#[derive(Debug, Clone, Copy)]
pub struct ExternalPasskeyBinding<'a> {
    pub user_id: &'a str,
    pub session_token_hash: &'a str,
    pub code_challenge: &'a str,
}

#[derive(Debug)]
enum ExternalPasskeyState {
    RegistrationReady {
        nickname: String,
        state: PasskeyRegistration,
        public_key: serde_json::Value,
    },
    AssertionReady {
        state: PasskeyAuthentication,
        public_key: serde_json::Value,
    },
    RegistrationVerified {
        nickname: String,
        credential_id: String,
        passkey_json: String,
    },
    AssertionVerified {
        passkey_id: String,
        passkey_json: String,
    },
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalPasskeyEvent {
    Pending,
    Complete,
    Failed,
}

#[derive(Debug)]
struct StoredExternalPasskey {
    user_id: String,
    session_token_hash: String,
    code_challenge: String,
    state: ExternalPasskeyState,
    events: tokio::sync::watch::Sender<ExternalPasskeyEvent>,
    expires: Instant,
    created: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExternalBrowserState {
    Ready {
        action: ExternalPasskeyAction,
        public_key: serde_json::Value,
    },
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalFinalizeResult {
    Pending,
    Failed,
    Registration {
        nickname: String,
        credential_id: String,
        passkey_json: String,
    },
    Assertion {
        passkey_id: String,
        passkey_json: String,
    },
}

#[derive(Debug, Default)]
pub struct PasskeyCeremonies {
    registrations: parking_lot::Mutex<HashMap<String, StoredRegistration>>,
    assertions: parking_lot::Mutex<HashMap<String, StoredAssertion>>,
    external: parking_lot::Mutex<HashMap<String, StoredExternalPasskey>>,
}

impl PasskeyCeremonies {
    pub fn start_registration(
        &self,
        user_id: &str,
        username: &str,
        nickname: String,
        existing: &[UserPasskey],
        webauthn: &Webauthn,
    ) -> Result<(String, serde_json::Value)> {
        anyhow::ensure!(
            existing.len() < MAX_PASSKEYS_PER_USER,
            "at most {MAX_PASSKEYS_PER_USER} passkeys"
        );
        let uuid = subject_uuid(user_id);
        let exclude = existing
            .iter()
            .filter_map(|row| decode_passkey(&row.passkey_json).ok())
            .map(|passkey| passkey.cred_id().clone())
            .collect::<Vec<_>>();
        let (ccr, state) = webauthn
            .start_passkey_registration(uuid, username, username, Some(exclude))
            .context("starting passkey registration")?;
        let challenge_id = new_user_id()?;
        self.gc();
        self.registrations.lock().insert(
            challenge_id.clone(),
            StoredRegistration {
                user_id: user_id.to_owned(),
                nickname,
                state,
                expires: std::time::Instant::now() + CEREMONY_TTL,
            },
        );
        Ok((
            challenge_id,
            serde_json::to_value(ccr.public_key).context("encoding registration options")?,
        ))
    }

    pub fn finish_registration(
        &self,
        user_id: &str,
        challenge_id: &str,
        credential: RegisterPublicKeyCredential,
        webauthn: &Webauthn,
    ) -> Result<(String, String, String)> {
        let stored = self
            .registrations
            .lock()
            .remove(challenge_id)
            .context("passkey registration expired")?;
        anyhow::ensure!(stored.user_id == user_id, "passkey registration expired");
        anyhow::ensure!(
            stored.expires > std::time::Instant::now(),
            "passkey registration expired"
        );
        let passkey = webauthn
            .finish_passkey_registration(&credential, &stored.state)
            .context("passkey registration rejected")?;
        let credential_id = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            passkey.cred_id(),
        );
        let passkey_json = serde_json::to_string(&passkey).context("encoding passkey")?;
        Ok((stored.nickname, credential_id, passkey_json))
    }

    pub fn start_assertion(
        &self,
        user_id: &str,
        existing: &[UserPasskey],
        webauthn: &Webauthn,
    ) -> Result<(String, serde_json::Value)> {
        anyhow::ensure!(!existing.is_empty(), "no passkey registered");
        let passkeys = existing
            .iter()
            .filter_map(|row| decode_passkey(&row.passkey_json).ok())
            .collect::<Vec<_>>();
        anyhow::ensure!(!passkeys.is_empty(), "no passkey registered");
        let (rcr, state) = webauthn
            .start_passkey_authentication(&passkeys)
            .context("starting passkey assertion")?;
        let challenge_id = new_user_id()?;
        self.gc();
        self.assertions.lock().insert(
            challenge_id.clone(),
            StoredAssertion {
                user_id: user_id.to_owned(),
                state,
                expires: std::time::Instant::now() + CEREMONY_TTL,
            },
        );
        Ok((
            challenge_id,
            serde_json::to_value(rcr.public_key).context("encoding assertion options")?,
        ))
    }

    pub fn finish_assertion(
        &self,
        user_id: &str,
        challenge_id: &str,
        credential: PublicKeyCredential,
        existing: &[UserPasskey],
        webauthn: &Webauthn,
    ) -> Result<(String, String)> {
        let stored = self
            .assertions
            .lock()
            .remove(challenge_id)
            .context("passkey assertion expired")?;
        anyhow::ensure!(stored.user_id == user_id, "passkey assertion expired");
        anyhow::ensure!(
            stored.expires > std::time::Instant::now(),
            "passkey assertion expired"
        );
        let result = webauthn
            .finish_passkey_authentication(&credential, &stored.state)
            .context("passkey assertion rejected")?;
        let cred_id = result.cred_id();
        let Some(row) = existing.iter().find(|row| {
            decode_passkey(&row.passkey_json).is_ok_and(|passkey| passkey.cred_id() == cred_id)
        }) else {
            bail!("passkey assertion rejected");
        };
        let mut passkey = decode_passkey(&row.passkey_json)?;
        passkey.update_credential(&result);
        let passkey_json = serde_json::to_string(&passkey).context("encoding passkey")?;
        Ok((row.id.clone(), passkey_json))
    }

    pub fn start_external_registration(
        &self,
        binding: ExternalPasskeyBinding<'_>,
        username: &str,
        nickname: String,
        existing: &[UserPasskey],
        webauthn: &Webauthn,
    ) -> Result<String> {
        anyhow::ensure!(
            existing.len() < MAX_PASSKEYS_PER_USER,
            "at most {MAX_PASSKEYS_PER_USER} passkeys"
        );
        validate_external_binding(binding.session_token_hash, binding.code_challenge)?;
        let uuid = subject_uuid(binding.user_id);
        let exclude = existing
            .iter()
            .filter_map(|row| decode_passkey(&row.passkey_json).ok())
            .map(|passkey| passkey.cred_id().clone())
            .collect::<Vec<_>>();
        let (ccr, state) = webauthn
            .start_passkey_registration(uuid, username, username, Some(exclude))
            .context("starting external passkey registration")?;
        let public_key = serde_json::to_value(ccr.public_key)
            .context("encoding external registration options")?;
        self.issue_external(
            binding.user_id,
            binding.session_token_hash,
            binding.code_challenge,
            ExternalPasskeyState::RegistrationReady {
                nickname,
                state,
                public_key,
            },
        )
    }

    pub fn start_external_assertion(
        &self,
        binding: ExternalPasskeyBinding<'_>,
        existing: &[UserPasskey],
        webauthn: &Webauthn,
    ) -> Result<String> {
        validate_external_binding(binding.session_token_hash, binding.code_challenge)?;
        anyhow::ensure!(!existing.is_empty(), "no passkey registered");
        let passkeys = existing
            .iter()
            .filter_map(|row| decode_passkey(&row.passkey_json).ok())
            .collect::<Vec<_>>();
        anyhow::ensure!(!passkeys.is_empty(), "no passkey registered");
        let (rcr, state) = webauthn
            .start_passkey_authentication(&passkeys)
            .context("starting external passkey assertion")?;
        let public_key =
            serde_json::to_value(rcr.public_key).context("encoding external assertion options")?;
        self.issue_external(
            binding.user_id,
            binding.session_token_hash,
            binding.code_challenge,
            ExternalPasskeyState::AssertionReady { state, public_key },
        )
    }

    pub fn external_browser_state(&self, transaction_id: &str) -> Result<ExternalBrowserState> {
        let key = external_transaction_key(transaction_id)?;
        self.gc_external();
        let entries = self.external.lock();
        let stored = entries
            .get(&key)
            .context("external passkey ceremony expired")?;
        Ok(match &stored.state {
            ExternalPasskeyState::RegistrationReady { public_key, .. } => {
                ExternalBrowserState::Ready {
                    action: ExternalPasskeyAction::Register,
                    public_key: public_key.clone(),
                }
            }
            ExternalPasskeyState::AssertionReady { public_key, .. } => {
                ExternalBrowserState::Ready {
                    action: ExternalPasskeyAction::Assert,
                    public_key: public_key.clone(),
                }
            }
            ExternalPasskeyState::RegistrationVerified { .. }
            | ExternalPasskeyState::AssertionVerified { .. } => ExternalBrowserState::Complete,
            ExternalPasskeyState::Failed => ExternalBrowserState::Failed,
        })
    }

    pub fn external_subject(
        &self,
        transaction_id: &str,
    ) -> Result<(ExternalPasskeyAction, String)> {
        let key = external_transaction_key(transaction_id)?;
        self.gc_external();
        let entries = self.external.lock();
        let stored = entries
            .get(&key)
            .context("external passkey ceremony expired")?;
        let action = match stored.state {
            ExternalPasskeyState::RegistrationReady { .. } => ExternalPasskeyAction::Register,
            ExternalPasskeyState::AssertionReady { .. } => ExternalPasskeyAction::Assert,
            _ => bail!("external passkey ceremony is unavailable"),
        };
        Ok((action, stored.user_id.clone()))
    }

    pub fn complete_external_registration(
        &self,
        transaction_id: &str,
        credential: RegisterPublicKeyCredential,
        webauthn: &Webauthn,
    ) -> Result<()> {
        let (key, mut stored) = self.take_external_ready(transaction_id)?;
        let ready = std::mem::replace(&mut stored.state, ExternalPasskeyState::Failed);
        let verified = (|| -> Result<ExternalPasskeyState> {
            let ExternalPasskeyState::RegistrationReady {
                nickname, state, ..
            } = ready
            else {
                bail!("external passkey ceremony type mismatch");
            };
            let passkey = webauthn
                .finish_passkey_registration(&credential, &state)
                .context("external passkey registration rejected")?;
            let credential_id = base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                passkey.cred_id(),
            );
            let passkey_json = serde_json::to_string(&passkey).context("encoding passkey")?;
            Ok(ExternalPasskeyState::RegistrationVerified {
                nickname,
                credential_id,
                passkey_json,
            })
        })();
        let result = verified.map(|state| stored.state = state);
        let event = if result.is_ok() {
            ExternalPasskeyEvent::Complete
        } else {
            ExternalPasskeyEvent::Failed
        };
        let events = stored.events.clone();
        self.external.lock().insert(key, stored);
        events.send_replace(event);
        result
    }

    pub fn complete_external_assertion(
        &self,
        transaction_id: &str,
        credential: PublicKeyCredential,
        existing: &[UserPasskey],
        webauthn: &Webauthn,
    ) -> Result<()> {
        let (key, mut stored) = self.take_external_ready(transaction_id)?;
        let ready = std::mem::replace(&mut stored.state, ExternalPasskeyState::Failed);
        let verified = (|| -> Result<ExternalPasskeyState> {
            let ExternalPasskeyState::AssertionReady { state, .. } = ready else {
                bail!("external passkey ceremony type mismatch");
            };
            let result = webauthn
                .finish_passkey_authentication(&credential, &state)
                .context("external passkey assertion rejected")?;
            let cred_id = result.cred_id();
            let Some(row) = existing.iter().find(|row| {
                decode_passkey(&row.passkey_json).is_ok_and(|passkey| passkey.cred_id() == cred_id)
            }) else {
                bail!("external passkey assertion rejected");
            };
            let mut passkey = decode_passkey(&row.passkey_json)?;
            passkey.update_credential(&result);
            let passkey_json = serde_json::to_string(&passkey).context("encoding passkey")?;
            Ok(ExternalPasskeyState::AssertionVerified {
                passkey_id: row.id.clone(),
                passkey_json,
            })
        })();
        let result = verified.map(|state| stored.state = state);
        let event = if result.is_ok() {
            ExternalPasskeyEvent::Complete
        } else {
            ExternalPasskeyEvent::Failed
        };
        let events = stored.events.clone();
        self.external.lock().insert(key, stored);
        events.send_replace(event);
        result
    }

    pub fn fail_external(&self, transaction_id: &str) -> Result<()> {
        let key = external_transaction_key(transaction_id)?;
        self.gc_external();
        let mut entries = self.external.lock();
        let stored = entries
            .get_mut(&key)
            .context("external passkey ceremony expired")?;
        if matches!(
            stored.state,
            ExternalPasskeyState::RegistrationReady { .. }
                | ExternalPasskeyState::AssertionReady { .. }
        ) {
            stored.state = ExternalPasskeyState::Failed;
            stored.events.send_replace(ExternalPasskeyEvent::Failed);
        }
        Ok(())
    }

    pub fn subscribe_external(
        &self,
        transaction_id: &str,
        user_id: &str,
        session_token_hash: &str,
        code_verifier: &str,
    ) -> Result<tokio::sync::watch::Receiver<ExternalPasskeyEvent>> {
        let key = external_transaction_key(transaction_id)?;
        let challenge = pkce_challenge(code_verifier)?;
        self.gc_external();
        let entries = self.external.lock();
        let stored = entries
            .get(&key)
            .context("external passkey ceremony expired")?;
        anyhow::ensure!(
            stored.user_id == user_id,
            "external passkey ceremony expired"
        );
        anyhow::ensure!(
            constant_time_equal(
                stored.session_token_hash.as_bytes(),
                session_token_hash.as_bytes()
            ),
            "external passkey session mismatch"
        );
        anyhow::ensure!(
            constant_time_equal(challenge.as_bytes(), stored.code_challenge.as_bytes()),
            "external passkey PKCE mismatch"
        );
        Ok(stored.events.subscribe())
    }

    pub fn finalize_external(
        &self,
        transaction_id: &str,
        user_id: &str,
        session_token_hash: &str,
        code_verifier: &str,
    ) -> Result<ExternalFinalizeResult> {
        let key = external_transaction_key(transaction_id)?;
        let challenge = pkce_challenge(code_verifier)?;
        self.gc_external();
        let mut entries = self.external.lock();
        let stored = entries
            .get(&key)
            .context("external passkey ceremony expired")?;
        anyhow::ensure!(
            stored.user_id == user_id,
            "external passkey ceremony expired"
        );
        anyhow::ensure!(
            constant_time_equal(
                stored.session_token_hash.as_bytes(),
                session_token_hash.as_bytes()
            ),
            "external passkey session mismatch"
        );
        anyhow::ensure!(
            constant_time_equal(challenge.as_bytes(), stored.code_challenge.as_bytes()),
            "external passkey PKCE mismatch"
        );
        if matches!(
            stored.state,
            ExternalPasskeyState::RegistrationReady { .. }
                | ExternalPasskeyState::AssertionReady { .. }
        ) {
            return Ok(ExternalFinalizeResult::Pending);
        }
        let stored = entries
            .remove(&key)
            .context("external passkey ceremony expired")?;
        Ok(match stored.state {
            ExternalPasskeyState::RegistrationVerified {
                nickname,
                credential_id,
                passkey_json,
            } => ExternalFinalizeResult::Registration {
                nickname,
                credential_id,
                passkey_json,
            },
            ExternalPasskeyState::AssertionVerified {
                passkey_id,
                passkey_json,
            } => ExternalFinalizeResult::Assertion {
                passkey_id,
                passkey_json,
            },
            ExternalPasskeyState::Failed => ExternalFinalizeResult::Failed,
            ExternalPasskeyState::RegistrationReady { .. }
            | ExternalPasskeyState::AssertionReady { .. } => {
                unreachable!("pending external ceremonies return before removal")
            }
        })
    }

    fn issue_external(
        &self,
        user_id: &str,
        session_token_hash: &str,
        code_challenge: &str,
        state: ExternalPasskeyState,
    ) -> Result<String> {
        let transaction_id = new_session_token()?;
        let now = Instant::now();
        let (events, _initial_receiver) =
            tokio::sync::watch::channel(ExternalPasskeyEvent::Pending);
        let mut entries = self.external.lock();
        entries.retain(|_, row| row.expires > now);
        if entries.len() >= MAX_EXTERNAL_CEREMONIES
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, row)| row.created)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(
            crate::admin::hex_sha256(transaction_id.as_bytes()),
            StoredExternalPasskey {
                user_id: user_id.to_owned(),
                session_token_hash: session_token_hash.to_owned(),
                code_challenge: code_challenge.to_owned(),
                state,
                events,
                expires: now + EXTERNAL_CEREMONY_TTL,
                created: now,
            },
        );
        Ok(transaction_id)
    }

    fn take_external_ready(&self, transaction_id: &str) -> Result<(String, StoredExternalPasskey)> {
        let key = external_transaction_key(transaction_id)?;
        self.gc_external();
        let stored = self
            .external
            .lock()
            .remove(&key)
            .context("external passkey ceremony expired")?;
        anyhow::ensure!(
            matches!(
                stored.state,
                ExternalPasskeyState::RegistrationReady { .. }
                    | ExternalPasskeyState::AssertionReady { .. }
            ),
            "external passkey ceremony is unavailable"
        );
        Ok((key, stored))
    }

    fn gc_external(&self) {
        let now = Instant::now();
        self.external.lock().retain(|_, row| row.expires > now);
    }

    fn gc(&self) {
        let now = std::time::Instant::now();
        self.registrations.lock().retain(|_, row| row.expires > now);
        self.assertions.lock().retain(|_, row| row.expires > now);
    }
}

fn validate_external_binding(session_token_hash: &str, code_challenge: &str) -> Result<()> {
    anyhow::ensure!(
        session_token_hash.len() == 64
            && session_token_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "invalid external passkey session"
    );
    anyhow::ensure!(
        valid_pkce_challenge(code_challenge),
        "invalid Passkey PKCE challenge"
    );
    Ok(())
}

fn external_transaction_key(transaction_id: &str) -> Result<String> {
    anyhow::ensure!(
        transaction_id.len() == 64
            && transaction_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
        "invalid external passkey ceremony"
    );
    Ok(crate::admin::hex_sha256(transaction_id.as_bytes()))
}

fn valid_pkce_challenge(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_pkce_verifier(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-'))
}

fn pkce_challenge(verifier: &str) -> Result<String> {
    use sha2::Digest as _;
    anyhow::ensure!(
        valid_pkce_verifier(verifier),
        "invalid Passkey PKCE verifier"
    );
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        sha2::Sha256::digest(verifier.as_bytes()),
    ))
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

fn decode_passkey(passkey_json: &str) -> Result<Passkey> {
    serde_json::from_str(passkey_json).context("stored passkey is invalid")
}

fn subject_uuid(subject: &str) -> Uuid {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(subject.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

#[derive(Debug, Deserialize)]
pub struct RegisterStartRequest {
    pub nickname: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterCompleteRequest {
    pub challenge_id: String,
    pub credential: RegisterPublicKeyCredential,
}

#[derive(Debug, Deserialize)]
pub struct AssertCompleteRequest {
    pub challenge_id: String,
    pub credential: PublicKeyCredential,
}

#[derive(Debug, Deserialize)]
pub struct ReauthSettingRequest {
    pub enabled: bool,
    pub reauth_after_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalStartRequest {
    pub action: ExternalPasskeyAction,
    pub nickname: Option<String>,
    pub code_challenge: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalTransactionRequest {
    pub transaction_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalCompleteRequest {
    pub transaction_id: String,
    pub credential: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalFinalizeRequest {
    pub transaction_id: String,
    pub code_verifier: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header};

    #[test]
    fn reauth_is_eligible_only_with_a_passkey_and_the_setting_on() {
        assert!(
            !PasskeyPolicy {
                enabled: true,
                reauth_after_ms: DEFAULT_PASSKEY_REAUTH_AFTER_MS,
                last_step_up_at_ms: Some(1),
                passkey_count: 0,
            }
            .reauth_eligible()
        );
        assert!(
            PasskeyPolicy {
                enabled: true,
                reauth_after_ms: DEFAULT_PASSKEY_REAUTH_AFTER_MS,
                last_step_up_at_ms: Some(1),
                passkey_count: 1,
            }
            .reauth_eligible()
        );
        assert!(
            !PasskeyPolicy {
                enabled: false,
                reauth_after_ms: DEFAULT_PASSKEY_REAUTH_AFTER_MS,
                last_step_up_at_ms: Some(1),
                passkey_count: 1,
            }
            .reauth_eligible()
        );
    }

    #[test]
    fn product_reauth_intervals_are_closed() {
        assert_eq!(DEFAULT_PASSKEY_REAUTH_AFTER_MS, 24 * 60 * 60 * 1_000);
        assert!(valid_reauth_interval(60 * 60 * 1_000));
        assert!(valid_reauth_interval(2 * 60 * 60 * 1_000));
        assert!(valid_reauth_interval(3 * 60 * 60 * 1_000));
        assert!(valid_reauth_interval(4 * 60 * 60 * 1_000));
        assert!(valid_reauth_interval(6 * 60 * 60 * 1_000));
        assert!(valid_reauth_interval(12 * 60 * 60 * 1_000));
        assert!(valid_reauth_interval(24 * 60 * 60 * 1_000));
        assert!(valid_reauth_interval(2 * 24 * 60 * 60 * 1_000));
        assert!(valid_reauth_interval(3 * 24 * 60 * 60 * 1_000));
        assert!(!valid_reauth_interval(8 * 60 * 60 * 1_000));
        assert!(!valid_reauth_interval(7 * 24 * 60 * 60 * 1_000));
        assert!(!valid_reauth_interval(14 * 24 * 60 * 60 * 1_000));
    }

    #[test]
    fn external_registration_is_session_bound_pkce_and_single_use() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://cowboy.example"),
        );
        let webauthn = webauthn_for_request(&headers).unwrap();
        let verifier = "a".repeat(64);
        let challenge = pkce_challenge(&verifier).unwrap();
        let session_hash = "b".repeat(64);
        let ceremonies = PasskeyCeremonies::default();
        let transaction_id = ceremonies
            .start_external_registration(
                ExternalPasskeyBinding {
                    user_id: "user-1",
                    session_token_hash: &session_hash,
                    code_challenge: &challenge,
                },
                "draven",
                "Travel phone".to_owned(),
                &[],
                &webauthn,
            )
            .unwrap();

        assert!(matches!(
            ceremonies.external_browser_state(&transaction_id).unwrap(),
            ExternalBrowserState::Ready {
                action: ExternalPasskeyAction::Register,
                ..
            }
        ));
        assert!(
            ceremonies
                .finalize_external(&transaction_id, "user-1", &"c".repeat(64), &verifier)
                .is_err()
        );
        assert!(
            ceremonies
                .finalize_external(&transaction_id, "user-1", &session_hash, &"d".repeat(64))
                .is_err()
        );
        assert_eq!(
            ceremonies
                .finalize_external(&transaction_id, "user-1", &session_hash, &verifier)
                .unwrap(),
            ExternalFinalizeResult::Pending
        );

        let events = ceremonies
            .subscribe_external(&transaction_id, "user-1", &session_hash, &verifier)
            .unwrap();
        assert_eq!(*events.borrow(), ExternalPasskeyEvent::Pending);
        assert!(
            ceremonies
                .subscribe_external(&transaction_id, "user-1", &session_hash, &"e".repeat(64))
                .is_err()
        );
        ceremonies.fail_external(&transaction_id).unwrap();
        assert_eq!(*events.borrow(), ExternalPasskeyEvent::Failed);
        assert_eq!(
            ceremonies
                .finalize_external(&transaction_id, "user-1", &session_hash, &verifier)
                .unwrap(),
            ExternalFinalizeResult::Failed
        );
        assert!(
            ceremonies
                .finalize_external(&transaction_id, "user-1", &session_hash, &verifier)
                .is_err()
        );
    }

    #[test]
    fn external_binding_rejects_malformed_pkce_and_transaction_values() {
        assert!(pkce_challenge("short").is_err());
        assert!(external_transaction_key("not-a-transaction").is_err());
        assert!(validate_external_binding(&"a".repeat(64), &"b".repeat(42)).is_err());
    }
}
