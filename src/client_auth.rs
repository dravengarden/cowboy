//! Browser-authorized, sender-constrained sessions for non-browser clients.
//!
//! The browser still performs the configured Cowboy login. A client generates
//! an Ed25519 key and PKCE verifier locally, then asks that browser to approve
//! the public key. Access tokens are short lived and every request carries a
//! proof from the approved key. Refresh tokens are persisted only as hashes
//! and rotated by the storage layer after every use.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use axum::http::{HeaderMap, Method};
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::admin::hex_sha256;

pub const ACCESS_TOKEN_PREFIX: &str = "cow_access_";
pub const REFRESH_TOKEN_PREFIX: &str = "cow_refresh_";
pub const ACCESS_TOKEN_TTL_MS: i64 = 10 * 60 * 1_000;
pub const REFRESH_TOKEN_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
pub const AUTHORIZATION_TTL_MS: i64 = 5 * 60 * 1_000;
pub const PROOF_CLOCK_SKEW_MS: i64 = 90 * 1_000;
pub const DEVICE_LAST_USED_TOUCH_MS: i64 = 10 * 60 * 1_000;

const PROOF_NAMESPACE: &str = "cowboy-device-proof-v1";
const AUTHORIZATION_TTL: Duration = Duration::from_secs(5 * 60);
const REPLAY_NONCE_TTL: Duration = Duration::from_secs(3 * 60);
const MAX_AUTHORIZATIONS: usize = 256;
const MAX_AUTHORIZATIONS_PER_IP: usize = 8;
const MAX_REPLAY_NONCES: usize = 8_192;
const DEVICE_NAME_MAX_CHARS: usize = 64;

pub const DEVICE_ID_HEADER: &str = "x-cowboy-device-id";
pub const PROOF_TIME_HEADER: &str = "x-cowboy-proof-time";
pub const PROOF_NONCE_HEADER: &str = "x-cowboy-proof-nonce";
pub const PROOF_SIGNATURE_HEADER: &str = "x-cowboy-proof";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartAuthorizationRequest {
    pub name: String,
    pub public_key: String,
    pub code_challenge: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StartAuthorizationResponse {
    pub request_id: String,
    pub verification_url: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserAuthorizationRequest {
    pub request_id: String,
    pub approval_token: String,
}

#[derive(Debug, Serialize)]
pub struct BrowserAuthorizationInfo {
    pub request_id: String,
    pub name: String,
    pub fingerprint: String,
    pub expires_at_ms: i64,
    pub status: &'static str,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExchangeRequest {
    pub request_id: String,
    pub code_verifier: String,
    pub timestamp_ms: i64,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceTokenResponse {
    pub device_id: String,
    pub access_token: String,
    pub access_expires_at_ms: i64,
    pub refresh_token: String,
    pub refresh_expires_at_ms: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AuthorizationEventStatus {
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationEventsHandshake {
    pub request_id: String,
    pub code_verifier: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationEvent {
    Pending,
    Approved,
    Denied,
}

#[derive(Debug)]
enum AuthorizationState {
    Pending,
    Approved { user_id: String },
    Exchanging { user_id: String },
    Denied,
}

#[derive(Debug)]
struct StoredAuthorization {
    name: String,
    public_key: [u8; 32],
    code_challenge: String,
    approval_token_hash: String,
    source_ip: IpAddr,
    state: AuthorizationState,
    events: tokio::sync::watch::Sender<AuthorizationEvent>,
    expires: Instant,
    expires_at_ms: i64,
}

#[derive(Debug)]
pub struct ApprovedAuthorization {
    pub name: String,
    pub public_key: String,
    pub user_id: String,
}

#[derive(Debug, Default)]
pub struct DeviceAuthorizations {
    entries: parking_lot::Mutex<HashMap<String, StoredAuthorization>>,
}

impl DeviceAuthorizations {
    pub fn start(
        &self,
        request: StartAuthorizationRequest,
        source_ip: IpAddr,
        now_ms: i64,
    ) -> Result<(String, String, i64)> {
        let name = normalize_device_name(&request.name)?;
        let public_key = decode_public_key(&request.public_key)?;
        validate_code_challenge(&request.code_challenge)?;
        let request_id = random_urlsafe::<24>()?;
        let approval_token = random_urlsafe::<32>()?;
        let expires_at_ms = now_ms.saturating_add(AUTHORIZATION_TTL_MS);
        let (events, _) = tokio::sync::watch::channel(AuthorizationEvent::Pending);
        let mut entries = self.entries.lock();
        prune_authorizations(&mut entries);
        ensure!(
            entries.len() < MAX_AUTHORIZATIONS,
            "too many device authorizations are active"
        );
        ensure!(
            entries
                .values()
                .filter(|entry| entry.source_ip == source_ip)
                .count()
                < MAX_AUTHORIZATIONS_PER_IP,
            "too many device authorizations are active for this address"
        );
        entries.insert(
            request_id.clone(),
            StoredAuthorization {
                name,
                public_key,
                code_challenge: request.code_challenge,
                approval_token_hash: hex_sha256(approval_token.as_bytes()),
                source_ip,
                state: AuthorizationState::Pending,
                events,
                expires: Instant::now() + AUTHORIZATION_TTL,
                expires_at_ms,
            },
        );
        Ok((request_id, approval_token, expires_at_ms))
    }

    pub fn browser_info(
        &self,
        request_id: &str,
        approval_token: &str,
    ) -> Result<BrowserAuthorizationInfo> {
        let mut entries = self.entries.lock();
        prune_authorizations(&mut entries);
        let entry = entries
            .get(request_id)
            .context("device authorization is unavailable")?;
        verify_approval_token(entry, approval_token)?;
        let status = match entry.state {
            AuthorizationState::Pending => "pending",
            AuthorizationState::Approved { .. } | AuthorizationState::Exchanging { .. } => {
                "approved"
            }
            AuthorizationState::Denied => "denied",
        };
        Ok(BrowserAuthorizationInfo {
            request_id: request_id.to_owned(),
            name: entry.name.clone(),
            fingerprint: device_fingerprint(&entry.public_key),
            expires_at_ms: entry.expires_at_ms,
            status,
        })
    }

    pub fn approve(&self, request: &BrowserAuthorizationRequest, user_id: &str) -> Result<()> {
        let mut entries = self.entries.lock();
        prune_authorizations(&mut entries);
        let entry = entries
            .get_mut(&request.request_id)
            .context("device authorization is unavailable")?;
        verify_approval_token(entry, &request.approval_token)?;
        ensure!(
            matches!(entry.state, AuthorizationState::Pending),
            "device authorization is no longer pending"
        );
        entry.state = AuthorizationState::Approved {
            user_id: user_id.to_owned(),
        };
        entry.events.send_replace(AuthorizationEvent::Approved);
        Ok(())
    }

    pub fn deny(&self, request: &BrowserAuthorizationRequest) -> Result<()> {
        let mut entries = self.entries.lock();
        prune_authorizations(&mut entries);
        let entry = entries
            .get_mut(&request.request_id)
            .context("device authorization is unavailable")?;
        verify_approval_token(entry, &request.approval_token)?;
        if matches!(entry.state, AuthorizationState::Pending) {
            entry.state = AuthorizationState::Denied;
            entry.events.send_replace(AuthorizationEvent::Denied);
        }
        Ok(())
    }

    pub fn subscribe(
        &self,
        request_id: &str,
        code_verifier: &str,
    ) -> Result<tokio::sync::watch::Receiver<AuthorizationEvent>> {
        let mut entries = self.entries.lock();
        prune_authorizations(&mut entries);
        let entry = entries
            .get(request_id)
            .context("device authorization is unavailable")?;
        ensure!(
            verify_code_challenge(code_verifier, &entry.code_challenge),
            "device authorization proof is invalid"
        );
        Ok(entry.events.subscribe())
    }

    pub fn begin_exchange(
        &self,
        request: &ExchangeRequest,
        now_ms: i64,
    ) -> Result<ApprovedAuthorization> {
        ensure_proof_fresh(request.timestamp_ms, now_ms)?;
        validate_nonce(&request.nonce)?;
        let mut entries = self.entries.lock();
        prune_authorizations(&mut entries);
        let entry = entries
            .get_mut(&request.request_id)
            .context("device authorization is unavailable")?;
        ensure!(
            verify_code_challenge(&request.code_verifier, &entry.code_challenge),
            "device authorization proof is invalid"
        );
        let AuthorizationState::Approved { user_id } = &entry.state else {
            bail!("device authorization is not approved");
        };
        let user_id = user_id.clone();
        let proof = exchange_proof(
            &request.request_id,
            &request.code_verifier,
            request.timestamp_ms,
            &request.nonce,
        );
        verify_signature(&entry.public_key, &proof, &request.signature)?;
        let approved = ApprovedAuthorization {
            name: entry.name.clone(),
            public_key: encode_public_key(&entry.public_key),
            user_id: user_id.clone(),
        };
        entry.state = AuthorizationState::Exchanging { user_id };
        Ok(approved)
    }

    pub fn finish_exchange(&self, request_id: &str, committed: bool) {
        let mut entries = self.entries.lock();
        if committed {
            entries.remove(request_id);
            return;
        }
        if let Some(entry) = entries.get_mut(request_id)
            && let AuthorizationState::Exchanging { user_id } = &entry.state
        {
            entry.state = AuthorizationState::Approved {
                user_id: user_id.clone(),
            };
        }
    }
}

#[derive(Debug, Clone)]
struct AccessSession {
    device_id: String,
    user_id: String,
    public_key: [u8; 32],
    expires_at_ms: i64,
}

#[derive(Debug)]
struct ReplayNonce {
    expires: Instant,
    created: Instant,
}

#[derive(Debug, Default)]
struct AccessState {
    tokens: HashMap<String, AccessSession>,
    replay_nonces: HashMap<String, ReplayNonce>,
}

#[derive(Debug, Default)]
pub struct DeviceAccessSessions {
    state: parking_lot::Mutex<AccessState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAccessIdentity {
    pub device_id: String,
    pub user_id: String,
}

pub struct TokenProofContext<'a> {
    pub method: &'a Method,
    pub path_and_query: &'a str,
    pub token: &'a str,
    pub device_id: &'a str,
    pub public_key: &'a str,
    pub now_ms: i64,
}

impl DeviceAccessSessions {
    pub fn issue(
        &self,
        device_id: &str,
        user_id: &str,
        public_key: &str,
        now_ms: i64,
    ) -> Result<(String, i64)> {
        let public_key = decode_public_key(public_key)?;
        let token = format!("{ACCESS_TOKEN_PREFIX}{}", random_urlsafe::<32>()?);
        let expires_at_ms = now_ms.saturating_add(ACCESS_TOKEN_TTL_MS);
        let mut state = self.state.lock();
        prune_access_state(&mut state, now_ms);
        // A refresh replaces, rather than accumulates, short-lived access for
        // this device. This bounds memory under an authorized refresh loop and
        // makes a rotated credential immediately authoritative.
        state
            .tokens
            .retain(|_, session| session.device_id != device_id);
        state.tokens.insert(
            hex_sha256(token.as_bytes()),
            AccessSession {
                device_id: device_id.to_owned(),
                user_id: user_id.to_owned(),
                public_key,
                expires_at_ms,
            },
        );
        Ok((token, expires_at_ms))
    }

    pub fn authenticate(
        &self,
        headers: &HeaderMap,
        method: &Method,
        path_and_query: &str,
        now_ms: i64,
    ) -> Result<Option<DeviceAccessIdentity>> {
        let Some(token) = bearer(headers) else {
            return Ok(None);
        };
        if !token.starts_with(ACCESS_TOKEN_PREFIX) {
            return Ok(None);
        }
        let mut state = self.state.lock();
        prune_access_state(&mut state, now_ms);
        let session = state
            .tokens
            .get(&hex_sha256(token.as_bytes()))
            .cloned()
            .context("device access token is invalid or expired")?;
        let proof = proof_headers(headers)?;
        ensure!(
            proof.device_id == session.device_id,
            "device proof has the wrong identity"
        );
        ensure_proof_fresh(proof.timestamp_ms, now_ms)?;
        let replay_key = format!("{}:{}", session.device_id, proof.nonce);
        ensure!(
            !state.replay_nonces.contains_key(&replay_key),
            "device proof was already used"
        );
        let canonical = request_proof(
            method.as_str(),
            path_and_query,
            &token,
            proof.timestamp_ms,
            &proof.nonce,
        );
        verify_signature(&session.public_key, &canonical, &proof.signature)?;
        state.replay_nonces.insert(
            replay_key,
            ReplayNonce {
                expires: Instant::now() + REPLAY_NONCE_TTL,
                created: Instant::now(),
            },
        );
        Ok(Some(DeviceAccessIdentity {
            device_id: session.device_id,
            user_id: session.user_id,
        }))
    }

    pub fn verify_token_proof(
        &self,
        headers: &HeaderMap,
        context: TokenProofContext<'_>,
    ) -> Result<()> {
        let public_key = decode_public_key(context.public_key)?;
        let proof = proof_headers(headers)?;
        ensure!(
            proof.device_id == context.device_id,
            "device proof has the wrong identity"
        );
        ensure_proof_fresh(proof.timestamp_ms, context.now_ms)?;
        let mut state = self.state.lock();
        prune_access_state(&mut state, context.now_ms);
        let replay_key = format!("{}:{}", context.device_id, proof.nonce);
        ensure!(
            !state.replay_nonces.contains_key(&replay_key),
            "device proof was already used"
        );
        let canonical = request_proof(
            context.method.as_str(),
            context.path_and_query,
            context.token,
            proof.timestamp_ms,
            &proof.nonce,
        );
        verify_signature(&public_key, &canonical, &proof.signature)?;
        state.replay_nonces.insert(
            replay_key,
            ReplayNonce {
                expires: Instant::now() + REPLAY_NONCE_TTL,
                created: Instant::now(),
            },
        );
        Ok(())
    }

    pub fn token_still_valid(
        &self,
        token: &str,
        expected: &DeviceAccessIdentity,
        now_ms: i64,
    ) -> bool {
        let mut state = self.state.lock();
        prune_access_state(&mut state, now_ms);
        state
            .tokens
            .get(&hex_sha256(token.as_bytes()))
            .is_some_and(|session| {
                session.device_id == expected.device_id && session.user_id == expected.user_id
            })
    }

    pub fn revoke_device(&self, device_id: &str) {
        self.state
            .lock()
            .tokens
            .retain(|_, session| session.device_id != device_id);
    }

    pub fn revoke_user(&self, user_id: &str) {
        self.state
            .lock()
            .tokens
            .retain(|_, session| session.user_id != user_id);
    }
}

#[derive(Debug)]
struct ParsedProofHeaders {
    device_id: String,
    timestamp_ms: i64,
    nonce: String,
    signature: String,
}

pub fn request_proof(
    method: &str,
    path_and_query: &str,
    token: &str,
    timestamp_ms: i64,
    nonce: &str,
) -> Vec<u8> {
    format!(
        "{PROOF_NAMESPACE}\n{}\n{path_and_query}\n{timestamp_ms}\n{nonce}\n{}",
        method.to_ascii_uppercase(),
        hex_sha256(token.as_bytes())
    )
    .into_bytes()
}

pub fn exchange_proof(
    request_id: &str,
    code_verifier: &str,
    timestamp_ms: i64,
    nonce: &str,
) -> Vec<u8> {
    format!(
        "{PROOF_NAMESPACE}\nEXCHANGE\n{request_id}\n{timestamp_ms}\n{nonce}\n{}",
        hex_sha256(code_verifier.as_bytes())
    )
    .into_bytes()
}

pub fn signed_proof_headers(
    signing_key: &SigningKey,
    device_id: &str,
    token: &str,
    method: &str,
    path_and_query: &str,
    now_ms: i64,
) -> Result<Vec<(String, String)>> {
    ensure!(valid_device_id(device_id), "device id is invalid");
    let nonce = random_urlsafe::<18>()?;
    let proof = request_proof(method, path_and_query, token, now_ms, &nonce);
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(signing_key.sign(&proof).to_bytes());
    Ok(vec![
        (DEVICE_ID_HEADER.to_owned(), device_id.to_owned()),
        (PROOF_TIME_HEADER.to_owned(), now_ms.to_string()),
        (PROOF_NONCE_HEADER.to_owned(), nonce),
        (PROOF_SIGNATURE_HEADER.to_owned(), signature),
    ])
}

pub fn signed_exchange_request(
    signing_key: &SigningKey,
    request_id: String,
    code_verifier: String,
    now_ms: i64,
) -> Result<ExchangeRequest> {
    let nonce = random_urlsafe::<18>()?;
    let proof = exchange_proof(&request_id, &code_verifier, now_ms, &nonce);
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(signing_key.sign(&proof).to_bytes());
    Ok(ExchangeRequest {
        request_id,
        code_verifier,
        timestamp_ms: now_ms,
        nonce,
        signature,
    })
}

pub fn new_signing_key() -> Result<SigningKey> {
    Ok(SigningKey::from_bytes(&random_bytes::<32>()?))
}

pub fn signing_key_from_base64(value: &str) -> Result<SigningKey> {
    Ok(SigningKey::from_bytes(&decode_fixed::<32>(
        value,
        "device private key",
    )?))
}

pub fn signing_key_to_base64(key: &SigningKey) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.to_bytes())
}

pub fn public_key_to_base64(key: &SigningKey) -> String {
    encode_public_key(key.verifying_key().as_bytes())
}

pub fn code_challenge(verifier: &str) -> Result<String> {
    validate_code_verifier(verifier)?;
    Ok(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes())),
    )
}

pub fn new_code_verifier() -> Result<String> {
    random_urlsafe::<48>()
}

pub fn new_refresh_token() -> Result<String> {
    Ok(format!("{REFRESH_TOKEN_PREFIX}{}", random_urlsafe::<32>()?))
}

pub fn valid_device_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn normalize_device_name(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(!value.is_empty(), "device name cannot be empty");
    ensure!(
        value.chars().count() <= DEVICE_NAME_MAX_CHARS && !value.chars().any(char::is_control),
        "device name is invalid"
    );
    Ok(value.to_owned())
}

fn validate_code_challenge(value: &str) -> Result<()> {
    let decoded = decode_fixed::<32>(value, "PKCE code challenge")?;
    ensure!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(decoded) == value,
        "PKCE code challenge is not canonical"
    );
    Ok(())
}

fn validate_code_verifier(value: &str) -> Result<()> {
    ensure!(
        (43..=128).contains(&value.len())
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
            }),
        "PKCE code verifier is invalid"
    );
    Ok(())
}

fn verify_code_challenge(verifier: &str, expected: &str) -> bool {
    code_challenge(verifier).is_ok_and(|candidate| constant_time_eq(&candidate, expected))
}

fn verify_approval_token(entry: &StoredAuthorization, approval_token: &str) -> Result<()> {
    ensure!(
        constant_time_eq(
            &entry.approval_token_hash,
            &hex_sha256(approval_token.as_bytes())
        ),
        "device authorization is unavailable"
    );
    Ok(())
}

fn proof_headers(headers: &HeaderMap) -> Result<ParsedProofHeaders> {
    let text = |name: &'static str| -> Result<&str> {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .with_context(|| format!("missing {name}"))
    };
    let device_id = text(DEVICE_ID_HEADER)?.to_owned();
    ensure!(valid_device_id(&device_id), "device id is invalid");
    let timestamp_ms = text(PROOF_TIME_HEADER)?
        .parse::<i64>()
        .context("device proof time is invalid")?;
    let nonce = text(PROOF_NONCE_HEADER)?.to_owned();
    validate_nonce(&nonce)?;
    Ok(ParsedProofHeaders {
        device_id,
        timestamp_ms,
        nonce,
        signature: text(PROOF_SIGNATURE_HEADER)?.to_owned(),
    })
}

fn validate_nonce(value: &str) -> Result<()> {
    ensure!(
        (16..=64).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "device proof nonce is invalid"
    );
    Ok(())
}

pub fn ensure_proof_fresh(timestamp_ms: i64, now_ms: i64) -> Result<()> {
    ensure!(
        timestamp_ms.abs_diff(now_ms) <= PROOF_CLOCK_SKEW_MS.unsigned_abs(),
        "device proof is outside the allowed clock window"
    );
    Ok(())
}

pub fn verify_signature(public_key: &[u8; 32], proof: &[u8], encoded: &str) -> Result<()> {
    let signature = decode_fixed::<64>(encoded, "device signature")?;
    VerifyingKey::from_bytes(public_key)
        .context("device public key is invalid")?
        .verify(proof, &Signature::from_bytes(&signature))
        .context("device signature is invalid")
}

pub fn decode_public_key(value: &str) -> Result<[u8; 32]> {
    decode_fixed::<32>(value, "device public key")
}

fn encode_public_key(value: &[u8; 32]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value)
}

pub fn device_fingerprint(public_key: &[u8; 32]) -> String {
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(public_key))
    )
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .trim();
    Some(
        value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))?
            .trim()
            .to_owned(),
    )
}

fn prune_authorizations(entries: &mut HashMap<String, StoredAuthorization>) {
    let now = Instant::now();
    entries.retain(|_, entry| entry.expires > now);
}

fn prune_access_state(state: &mut AccessState, now_ms: i64) {
    state.tokens.retain(|_, token| token.expires_at_ms > now_ms);
    let now = Instant::now();
    state.replay_nonces.retain(|_, nonce| nonce.expires > now);
    while state.replay_nonces.len() >= MAX_REPLAY_NONCES {
        let Some(oldest) = state
            .replay_nonces
            .iter()
            .min_by_key(|(_, nonce)| nonce.created)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        state.replay_nonces.remove(&oldest);
    }
}

fn random_urlsafe<const N: usize>() -> Result<String> {
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes::<N>()?))
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    Ok(bytes)
}

fn decode_fixed<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .with_context(|| format!("decoding {label}"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("{label} must contain exactly {N} bytes"))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, header};

    fn signed_headers(
        key: &SigningKey,
        device_id: &str,
        token: &str,
        method: &str,
        path: &str,
        now_ms: i64,
    ) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        for (name, value) in
            signed_proof_headers(key, device_id, token, method, path, now_ms).unwrap()
        {
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(&value).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn pkce_exchange_requires_approved_local_key() {
        let key = new_signing_key().unwrap();
        let verifier = new_code_verifier().unwrap();
        let manager = DeviceAuthorizations::default();
        let now = 1_900_000_000_000;
        let (request_id, approval_token, _) = manager
            .start(
                StartAuthorizationRequest {
                    name: "Zed on Hawk".to_owned(),
                    public_key: public_key_to_base64(&key),
                    code_challenge: code_challenge(&verifier).unwrap(),
                },
                "127.0.0.1".parse().unwrap(),
                now,
            )
            .unwrap();
        manager
            .approve(
                &BrowserAuthorizationRequest {
                    request_id: request_id.clone(),
                    approval_token,
                },
                "0123456789abcdef0123456789abcdef",
            )
            .unwrap();
        let request = signed_exchange_request(&key, request_id.clone(), verifier, now).unwrap();
        let approved = manager.begin_exchange(&request, now).unwrap();
        manager.finish_exchange(&request_id, true);
        assert_eq!(approved.name, "Zed on Hawk");
        assert_eq!(approved.public_key, public_key_to_base64(&key));
    }

    #[test]
    fn access_proof_is_route_bound_and_single_use() {
        let key = new_signing_key().unwrap();
        let manager = DeviceAccessSessions::default();
        let now = 1_900_000_000_000;
        let device_id = "0123456789abcdef0123456789abcdef";
        let user_id = "abcdef0123456789abcdef0123456789";
        let (token, _) = manager
            .issue(device_id, user_id, &public_key_to_base64(&key), now)
            .unwrap();
        let headers = signed_headers(&key, device_id, &token, "GET", "/api/sessions", now);
        assert!(
            manager
                .authenticate(&headers, &Method::GET, "/api/sessions", now)
                .unwrap()
                .is_some()
        );
        assert!(
            manager
                .authenticate(&headers, &Method::GET, "/api/sessions", now)
                .is_err()
        );
        let wrong_path = signed_headers(&key, device_id, &token, "GET", "/api/sessions", now);
        assert!(
            manager
                .authenticate(&wrong_path, &Method::GET, "/api/machines", now)
                .is_err()
        );

        let (replacement, _) = manager
            .issue(device_id, user_id, &public_key_to_base64(&key), now + 1)
            .unwrap();
        let replaced = signed_headers(&key, device_id, &token, "GET", "/api/sessions", now + 1);
        assert!(
            manager
                .authenticate(&replaced, &Method::GET, "/api/sessions", now + 1)
                .is_err()
        );
        let replacement_headers = signed_headers(
            &key,
            device_id,
            &replacement,
            "GET",
            "/api/sessions",
            now + 1,
        );
        assert!(
            manager
                .authenticate(&replacement_headers, &Method::GET, "/api/sessions", now + 1,)
                .unwrap()
                .is_some()
        );
    }
}
