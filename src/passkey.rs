//! Product-plane WebAuthn / Passkey registration and step-up.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Webauthn, WebauthnBuilder,
};

use crate::product_auth::new_user_id;

pub const PASSKEY_REAUTH_AFTER_MS: i64 = 15 * 60 * 1_000;
const CEREMONY_TTL: Duration = Duration::from_secs(300);
const MAX_PASSKEYS_PER_USER: usize = 8;

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
    pub last_step_up_at_ms: Option<i64>,
    pub passkey_count: u32,
}

impl PasskeyPolicy {
    #[must_use]
    pub fn reauth_required(&self, now_ms: i64) -> bool {
        self.enabled
            && self.passkey_count > 0
            && self
                .last_step_up_at_ms
                .is_none_or(|stamp| now_ms.saturating_sub(stamp) >= PASSKEY_REAUTH_AFTER_MS)
    }
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

#[derive(Debug, Default)]
pub struct PasskeyCeremonies {
    registrations: parking_lot::Mutex<HashMap<String, StoredRegistration>>,
    assertions: parking_lot::Mutex<HashMap<String, StoredAssertion>>,
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
            serde_json::to_value(ccr).context("encoding registration options")?,
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
            serde_json::to_value(rcr).context("encoding assertion options")?,
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

    fn gc(&self) {
        let now = std::time::Instant::now();
        self.registrations.lock().retain(|_, row| row.expires > now);
        self.assertions.lock().retain(|_, row| row.expires > now);
    }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewing_lock_requires_a_passkey_and_elapsed_window() {
        let policy = PasskeyPolicy {
            enabled: true,
            last_step_up_at_ms: Some(1),
            passkey_count: 0,
        };
        assert!(!policy.reauth_required(PASSKEY_REAUTH_AFTER_MS + 2));
        let locked = PasskeyPolicy {
            enabled: true,
            last_step_up_at_ms: Some(1),
            passkey_count: 1,
        };
        assert!(locked.reauth_required(PASSKEY_REAUTH_AFTER_MS + 2));
        assert!(!locked.reauth_required(PASSKEY_REAUTH_AFTER_MS));
        let disabled = PasskeyPolicy {
            enabled: false,
            last_step_up_at_ms: Some(1),
            passkey_count: 1,
        };
        assert!(!disabled.reauth_required(PASSKEY_REAUTH_AFTER_MS + 2));
    }
}
