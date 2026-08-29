//! Cardea OIDC Authorization Code + PKCE consumer profile.

use std::collections::HashMap;
use std::io::Read as _;
use std::net::IpAddr;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::admin::hex_sha256;
use crate::product_auth::new_session_token;

pub const TRANSACTION_COOKIE: &str = "cowboy_oidc";
pub const NATIVE_CALLBACK_SCHEME: &str = "xyz.stormbird.cowboy.manager";
const CONFIG_SCHEMA: &str = "dravengarden.cowboy.cardea-oidc/v1";
const TRANSACTION_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_TRANSACTIONS: usize = 256;
const MAX_TRANSACTIONS_PER_IP: usize = 8;
const NATIVE_HANDOFF_TTL: Duration = Duration::from_secs(60);
const MAX_NATIVE_HANDOFFS: usize = 128;
const MAX_SECRET_BYTES: u64 = 8_192;
const MAX_TOKEN_RESPONSE_BYTES: usize = 32 * 1_024;
const MAX_JWT_BYTES: usize = 8_192;
const CLIENT_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderDocument {
    schema: String,
    display_name: String,
    issuer: String,
    client_id: String,
    client_key_id: String,
    client_private_key_file: PathBuf,
    id_token_key_id: String,
    id_token_public_key_jwk: PublicJwk,
    subject: String,
    account: String,
    #[serde(default)]
    admin_account: Option<String>,
    redirect_uri: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateJwk {
    crv: String,
    d: String,
    kty: String,
    x: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicJwk {
    crv: String,
    kty: String,
    x: String,
}

#[derive(Clone)]
pub struct OidcProvider {
    display_name: String,
    issuer: String,
    authorization_endpoint: Url,
    token_endpoint: String,
    client_id: String,
    client_key_id: String,
    client_signing_seed: [u8; 32],
    id_token_key_id: String,
    id_token_verifying_key: [u8; 32],
    subject: String,
    account: String,
    admin_account: Option<String>,
    redirect_uri: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicProvider {
    pub id: &'static str,
    pub display_name: String,
    pub start_url: &'static str,
}

#[derive(Debug)]
struct StoredTransaction {
    state_hash: String,
    nonce: String,
    pkce_verifier: String,
    source_ip: IpAddr,
    expires: Instant,
    created: Instant,
    target: AuthorizationTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationTarget {
    Browser,
    MacOs { code_challenge: String },
}

#[derive(Debug)]
pub struct OidcTransaction {
    nonce: String,
    pkce_verifier: String,
    target: AuthorizationTarget,
}

#[derive(Debug)]
pub struct StartedAuthorization {
    pub location: String,
    pub cookie_token: String,
}

#[derive(Debug, Default)]
pub struct OidcTransactions {
    entries: parking_lot::Mutex<HashMap<String, StoredTransaction>>,
}

#[derive(Debug)]
struct StoredNativeHandoff {
    user_id: String,
    code_challenge: String,
    expires: Instant,
    created: Instant,
}

#[derive(Debug, Default)]
pub struct NativeHandoffs {
    entries: parking_lot::Mutex<HashMap<String, StoredNativeHandoff>>,
}

#[derive(Debug)]
pub struct StartedNativeHandoff {
    pub location: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    scope: String,
    id_token: String,
}

#[derive(Debug)]
pub struct VerifiedIdentity {
    pub subject: String,
    pub authenticated_at: u64,
}

#[derive(Serialize)]
struct ClientAssertionHeader<'a> {
    alg: &'static str,
    kid: &'a str,
    typ: &'static str,
}

#[derive(Serialize)]
struct ClientAssertionClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    jti: &'a str,
    iat: u64,
    exp: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdTokenHeader {
    alg: String,
    kid: String,
    typ: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdTokenClaims {
    iss: String,
    sub: String,
    aud: String,
    nonce: String,
    iat: u64,
    exp: u64,
    auth_time: u64,
}

impl OidcProvider {
    pub fn load(path: &Path) -> Result<Self> {
        let document: ProviderDocument = serde_json::from_slice(&read_protected_file(path)?)
            .context("decoding OIDC provider config")?;
        anyhow::ensure!(
            document.schema == CONFIG_SCHEMA,
            "unsupported OIDC provider config"
        );
        anyhow::ensure!(
            valid_display_name(&document.display_name),
            "OIDC display_name must be 1-64 printable characters"
        );
        anyhow::ensure!(
            valid_identifier(&document.client_id)
                && valid_identifier(&document.client_key_id)
                && valid_identifier(&document.id_token_key_id)
                && valid_identifier(&document.subject),
            "OIDC identifiers are invalid"
        );
        let account = crate::product_auth::normalize_username(&document.account)
            .context("normalizing OIDC account mapping")?;
        let admin_account = document
            .admin_account
            .as_deref()
            .map(crate::product_auth::normalize_username)
            .transpose()
            .context("normalizing OIDC admin account mapping")?;
        let issuer = exact_https_origin(&document.issuer)?;
        let redirect = exact_https_url(&document.redirect_uri)?;
        anyhow::ensure!(
            redirect.path() == "/api/auth/oidc/callback" && redirect.query().is_none(),
            "OIDC redirect_uri must use the fixed Cowboy callback"
        );
        anyhow::ensure!(
            document.client_private_key_file.is_absolute(),
            "OIDC client_private_key_file must be absolute"
        );
        let private: PrivateJwk =
            serde_json::from_slice(&read_protected_file(&document.client_private_key_file)?)
                .context("decoding OIDC client private JWK")?;
        anyhow::ensure!(
            private.kty == "OKP" && private.crv == "Ed25519",
            "OIDC client key must be Ed25519"
        );
        let client_signing_seed = decode_32(&private.d, "OIDC client private key")?;
        let client_public = decode_32(&private.x, "OIDC client public key")?;
        anyhow::ensure!(
            SigningKey::from_bytes(&client_signing_seed)
                .verifying_key()
                .as_bytes()
                == &client_public,
            "OIDC client private JWK is inconsistent"
        );
        anyhow::ensure!(
            document.id_token_public_key_jwk.kty == "OKP"
                && document.id_token_public_key_jwk.crv == "Ed25519",
            "OIDC ID-token key must be Ed25519"
        );
        let id_token_verifying_key = decode_32(
            &document.id_token_public_key_jwk.x,
            "OIDC ID-token public key",
        )?;
        let issuer = issuer.as_str().trim_end_matches('/').to_owned();
        let authorization_endpoint = Url::parse(&format!("{issuer}/oauth2/authorize"))?;
        let token_endpoint = format!("{issuer}/oauth2/token");
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .context("building OIDC HTTP client")?;
        Ok(Self {
            display_name: document.display_name,
            issuer,
            authorization_endpoint,
            token_endpoint,
            client_id: document.client_id,
            client_key_id: document.client_key_id,
            client_signing_seed,
            id_token_key_id: document.id_token_key_id,
            id_token_verifying_key,
            subject: document.subject,
            account,
            admin_account,
            redirect_uri: document.redirect_uri,
            http,
        })
    }

    #[must_use]
    pub fn public(&self) -> PublicProvider {
        PublicProvider {
            id: "cardea",
            display_name: self.display_name.clone(),
            start_url: "/api/auth/oidc/start",
        }
    }

    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }

    #[must_use]
    pub fn admin_account(&self) -> Option<&str> {
        self.admin_account.as_deref()
    }

    pub async fn exchange(
        &self,
        transaction: &OidcTransaction,
        code: &str,
    ) -> Result<VerifiedIdentity> {
        anyhow::ensure!(valid_authorization_code(code), "invalid authorization code");
        let now = now_seconds()?;
        let assertion_id = new_session_token()?;
        let assertion = client_assertion(
            &self.client_signing_seed,
            &self.client_key_id,
            &self.client_id,
            &self.token_endpoint,
            &assertion_id,
            now,
        )
        .context("building OIDC client assertion")?;
        let response = self
            .http
            .post(&self.token_endpoint)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", self.client_id.as_str()),
                ("client_assertion_type", CLIENT_ASSERTION_TYPE),
                ("client_assertion", assertion.as_str()),
                ("code", code),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("code_verifier", transaction.pkce_verifier.as_str()),
            ])
            .send()
            .await
            .context("exchanging OIDC authorization code")?;
        anyhow::ensure!(
            response.status().is_success(),
            "OIDC token exchange rejected"
        );
        anyhow::ensure!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.split(';').next() == Some("application/json")),
            "OIDC token endpoint returned an invalid content type"
        );
        anyhow::ensure!(
            response
                .content_length()
                .is_none_or(|length| length <= MAX_TOKEN_RESPONSE_BYTES as u64),
            "OIDC token response is too large"
        );
        let bytes = response
            .bytes()
            .await
            .context("reading OIDC token response")?;
        anyhow::ensure!(
            bytes.len() <= MAX_TOKEN_RESPONSE_BYTES,
            "OIDC token response is too large"
        );
        let token: TokenResponse =
            serde_json::from_slice(&bytes).context("decoding OIDC token response")?;
        anyhow::ensure!(
            token.token_type == "Bearer"
                && token.expires_in == 300
                && token.scope == "openid"
                && !token.access_token.is_empty(),
            "OIDC token response policy mismatch"
        );
        let identity = verify_id_token(
            &token.id_token,
            &self.id_token_verifying_key,
            &self.id_token_key_id,
            &self.issuer,
            &self.client_id,
            &transaction.nonce,
            now,
        )
        .context("OIDC ID token verification failed")?;
        anyhow::ensure!(
            identity.subject == self.subject,
            "OIDC subject is not allowed"
        );
        Ok(identity)
    }
}

impl OidcTransactions {
    pub fn begin(
        &self,
        provider: &OidcProvider,
        source_ip: IpAddr,
        target: AuthorizationTarget,
    ) -> Result<StartedAuthorization> {
        if let AuthorizationTarget::MacOs { code_challenge } = &target {
            anyhow::ensure!(
                valid_pkce_challenge(code_challenge),
                "invalid native PKCE challenge"
            );
        }
        let cookie_token = new_session_token()?;
        let state = new_session_token()?;
        let nonce = new_session_token()?;
        let pkce_verifier = new_session_token()?;
        let challenge = pkce_challenge(&pkce_verifier).context("building OIDC PKCE challenge")?;
        let now = Instant::now();
        let mut entries = self.entries.lock();
        entries.retain(|_, row| row.expires > now);
        anyhow::ensure!(
            entries
                .values()
                .filter(|row| row.source_ip == source_ip)
                .count()
                < MAX_TRANSACTIONS_PER_IP,
            "too many OIDC login attempts"
        );
        if entries.len() >= MAX_TRANSACTIONS
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, row)| row.created)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(
            hex_sha256(cookie_token.as_bytes()),
            StoredTransaction {
                state_hash: hex_sha256(state.as_bytes()),
                nonce: nonce.clone(),
                pkce_verifier,
                source_ip,
                expires: now + TRANSACTION_TTL,
                created: now,
                target,
            },
        );
        drop(entries);
        let mut authorization = provider.authorization_endpoint.clone();
        authorization.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", provider.client_id.as_str()),
            ("redirect_uri", provider.redirect_uri.as_str()),
            ("scope", "openid"),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ]);
        Ok(StartedAuthorization {
            location: authorization.into(),
            cookie_token,
        })
    }

    pub fn consume(&self, cookie_token: &str, state: &str) -> Result<OidcTransaction> {
        anyhow::ensure!(
            valid_opaque_value(cookie_token) && valid_opaque_value(state),
            "invalid OIDC transaction"
        );
        let stored = self
            .entries
            .lock()
            .remove(&hex_sha256(cookie_token.as_bytes()))
            .context("OIDC transaction expired")?;
        anyhow::ensure!(stored.expires > Instant::now(), "OIDC transaction expired");
        anyhow::ensure!(
            constant_time_equal(
                stored.state_hash.as_bytes(),
                hex_sha256(state.as_bytes()).as_bytes(),
            ),
            "OIDC state mismatch"
        );
        Ok(OidcTransaction {
            nonce: stored.nonce,
            pkce_verifier: stored.pkce_verifier,
            target: stored.target,
        })
    }
}

impl OidcTransaction {
    #[must_use]
    pub fn target(&self) -> &AuthorizationTarget {
        &self.target
    }
}

impl NativeHandoffs {
    pub fn issue(&self, user_id: &str, code_challenge: &str) -> Result<StartedNativeHandoff> {
        anyhow::ensure!(valid_identifier(user_id), "invalid native handoff user");
        anyhow::ensure!(
            valid_pkce_challenge(code_challenge),
            "invalid native PKCE challenge"
        );
        let code = new_session_token()?;
        let now = Instant::now();
        let mut entries = self.entries.lock();
        entries.retain(|_, row| row.expires > now);
        if entries.len() >= MAX_NATIVE_HANDOFFS
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, row)| row.created)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(
            hex_sha256(code.as_bytes()),
            StoredNativeHandoff {
                user_id: user_id.to_owned(),
                code_challenge: code_challenge.to_owned(),
                expires: now + NATIVE_HANDOFF_TTL,
                created: now,
            },
        );
        drop(entries);
        let mut callback = Url::parse(&format!("{NATIVE_CALLBACK_SCHEME}://auth/callback"))?;
        callback.query_pairs_mut().append_pair("code", &code);
        Ok(StartedNativeHandoff {
            location: callback.into(),
        })
    }

    pub fn consume(&self, code: &str, verifier: &str) -> Result<String> {
        anyhow::ensure!(valid_opaque_value(code), "invalid native handoff");
        let challenge = pkce_challenge(verifier).context("invalid native PKCE verifier")?;
        let stored = self
            .entries
            .lock()
            .remove(&hex_sha256(code.as_bytes()))
            .context("native handoff expired")?;
        anyhow::ensure!(stored.expires > Instant::now(), "native handoff expired");
        anyhow::ensure!(
            constant_time_equal(challenge.as_bytes(), stored.code_challenge.as_bytes()),
            "native PKCE mismatch"
        );
        Ok(stored.user_id)
    }
}

#[must_use]
pub fn native_error_location() -> &'static str {
    "xyz.stormbird.cowboy.manager://auth/callback?error=authorization_failed"
}

#[must_use]
pub fn transaction_cookie(token: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{TRANSACTION_COOKIE}={token}; Path=/api/auth/oidc/callback; HttpOnly; SameSite=Lax; Max-Age={}",
        TRANSACTION_TTL.as_secs()
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[must_use]
pub fn clear_transaction_cookie(secure: bool) -> String {
    let mut cookie = format!(
        "{TRANSACTION_COOKIE}=; Path=/api/auth/oidc/callback; HttpOnly; SameSite=Lax; Max-Age=0"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

fn read_protected_file(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("opening protected file {}", path.display()))?;
    let metadata = file.metadata().context("inspecting protected file")?;
    anyhow::ensure!(metadata.is_file(), "protected file must be regular");
    anyhow::ensure!(
        metadata.mode() & 0o077 == 0,
        "protected file permissions are too broad"
    );
    anyhow::ensure!(
        metadata.len() <= MAX_SECRET_BYTES,
        "protected file is too large"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_SECRET_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_SECRET_BYTES,
        "protected file is too large"
    );
    Ok(bytes)
}

fn decode_32(value: &str, label: &str) -> Result<[u8; 32]> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .with_context(|| format!("decoding {label}"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("{label} must be 32 bytes"))
}

/// Exact S256 verifier contract shared with Cardea's canonical consumer SDK.
#[must_use]
pub(crate) fn pkce_challenge(verifier: &str) -> Option<String> {
    valid_pkce_verifier(verifier).then(|| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    })
}

fn client_assertion(
    signing_seed: &[u8; 32],
    key_id: &str,
    client_id: &str,
    token_endpoint: &str,
    assertion_id: &str,
    now: u64,
) -> Option<String> {
    if !valid_identifier(key_id)
        || !valid_identifier(client_id)
        || !valid_identifier(assertion_id)
        || exact_https_url(token_endpoint).is_err()
    {
        return None;
    }
    let header = encode_json(&ClientAssertionHeader {
        alg: "EdDSA",
        kid: key_id,
        typ: "JWT",
    })?;
    let claims = encode_json(&ClientAssertionClaims {
        iss: client_id,
        sub: client_id,
        aud: token_endpoint,
        jti: assertion_id,
        iat: now,
        exp: now.checked_add(300)?,
    })?;
    let signing_input = format!("{header}.{claims}");
    let signature = SigningKey::from_bytes(signing_seed).sign(signing_input.as_bytes());
    Some(format!(
        "{signing_input}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

#[allow(clippy::too_many_arguments)]
fn verify_id_token(
    token: &str,
    verifying_key: &[u8; 32],
    key_id: &str,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now: u64,
) -> Option<VerifiedIdentity> {
    if token.len() > MAX_JWT_BYTES
        || !valid_identifier(key_id)
        || !valid_identifier(client_id)
        || !valid_opaque_value(nonce)
        || exact_https_origin(issuer).is_err()
    {
        return None;
    }
    let mut segments = token.split('.');
    let (header, claims, signature, None) = (
        segments.next()?,
        segments.next()?,
        segments.next()?,
        segments.next(),
    ) else {
        return None;
    };
    let header_value: IdTokenHeader = decode_json(header)?;
    if header_value.alg != "EdDSA" || header_value.typ != "JWT" || header_value.kid != key_id {
        return None;
    }
    let claims_value: IdTokenClaims = decode_json(claims)?;
    if claims_value.iss != issuer
        || claims_value.aud != client_id
        || claims_value.nonce != nonce
        || !valid_identifier(&claims_value.sub)
        || claims_value.iat > now
        || claims_value.auth_time > claims_value.iat
        || claims_value.exp <= now
        || claims_value.exp > claims_value.iat.checked_add(300)?
    {
        return None;
    }
    let signature_bytes: [u8; 64] = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(signature)
        .ok()?
        .try_into()
        .ok()?;
    let signature = Signature::from_bytes(&signature_bytes);
    VerifyingKey::from_bytes(verifying_key)
        .ok()?
        .verify(format!("{header}.{claims}").as_bytes(), &signature)
        .ok()?;
    Some(VerifiedIdentity {
        subject: claims_value.sub,
        authenticated_at: claims_value.auth_time,
    })
}

fn encode_json<T: Serialize>(value: &T) -> Option<String> {
    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).ok()?))
}

fn decode_json<T: for<'de> Deserialize<'de>>(value: &str) -> Option<T> {
    serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(value)
            .ok()?,
    )
    .ok()
}

fn exact_https_origin(value: &str) -> Result<Url> {
    let url = exact_https_url(value)?;
    anyhow::ensure!(
        url.path() == "/" && url.query().is_none() && !value.ends_with('/'),
        "OIDC issuer must be an exact HTTPS origin without a trailing slash"
    );
    Ok(url)
}

fn exact_https_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("parsing OIDC HTTPS URL")?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "OIDC URLs must use HTTPS without credentials or fragments"
    );
    Ok(url)
}

fn valid_identifier(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_display_name(value: &str) -> bool {
    (1..=64).contains(&value.chars().count())
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_opaque_value(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_authorization_code(value: &str) -> bool {
    (16..=512).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
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

fn now_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates Unix epoch")?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use super::{
        NATIVE_CALLBACK_SCHEME, NativeHandoffs, OidcTransactions, TRANSACTION_COOKIE,
        clear_transaction_cookie, read_protected_file,
    };
    use base64::Engine as _;
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    #[test]
    fn transaction_cookie_is_callback_scoped() {
        let cleared = clear_transaction_cookie(true);
        assert!(cleared.starts_with(&format!("{TRANSACTION_COOKIE}=;")));
        assert!(cleared.contains("Path=/api/auth/oidc/callback"));
        assert!(cleared.contains("SameSite=Lax"));
        assert!(cleared.contains("Secure"));
    }

    #[test]
    fn missing_transaction_fails_closed() {
        assert!(
            OidcTransactions::default()
                .consume(&"a".repeat(64), &"b".repeat(64))
                .is_err()
        );
    }

    #[test]
    fn native_handoff_is_fixed_single_use_pkce() {
        let handoffs = NativeHandoffs::default();
        let verifier = "a".repeat(64);
        let challenge = super::pkce_challenge(&verifier).unwrap();
        let started = handoffs.issue(&"b".repeat(32), &challenge).unwrap();
        let callback = url::Url::parse(&started.location).unwrap();
        assert_eq!(callback.scheme(), NATIVE_CALLBACK_SCHEME);
        assert_eq!(callback.host_str(), Some("auth"));
        assert_eq!(callback.path(), "/callback");
        let code = callback
            .query_pairs()
            .find(|(name, _)| name == "code")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        assert_eq!(handoffs.consume(&code, &verifier).unwrap(), "b".repeat(32));
        assert!(handoffs.consume(&code, &verifier).is_err());
    }

    #[test]
    fn native_handoff_wrong_verifier_fails_closed_and_consumes_code() {
        let handoffs = NativeHandoffs::default();
        let verifier = "a".repeat(64);
        let challenge = super::pkce_challenge(&verifier).unwrap();
        let started = handoffs.issue(&"b".repeat(32), &challenge).unwrap();
        let callback = url::Url::parse(&started.location).unwrap();
        let code = callback
            .query_pairs()
            .find(|(name, _)| name == "code")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        assert!(handoffs.consume(&code, &"c".repeat(64)).is_err());
        assert!(handoffs.consume(&code, &verifier).is_err());
    }

    #[test]
    fn oidc_trust_files_must_be_private_regular_files_without_symlinks() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-oidc-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("provider.json");
        std::fs::write(&file, b"{}").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_protected_file(&file).is_err());

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_protected_file(&file).unwrap(), b"{}");
        let link = root.join("provider-link.json");
        symlink(&file, &link).unwrap();
        assert!(read_protected_file(&link).is_err());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn id_token_verification_pins_signature_and_exact_claims() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let nonce = "a".repeat(64);
        let header = super::encode_json(&serde_json::json!({
            "alg": "EdDSA",
            "kid": "cardea-2026",
            "typ": "JWT",
        }))
        .unwrap();
        let claims = super::encode_json(&serde_json::json!({
            "iss": "https://cardea.example",
            "sub": "draven",
            "aud": "cowboy-production",
            "nonce": nonce,
            "iat": 1_000,
            "exp": 1_300,
            "auth_time": 900,
        }))
        .unwrap();
        let input = format!("{header}.{claims}");
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(signing.sign(input.as_bytes()).to_bytes());
        let token = format!("{input}.{signature}");

        let identity = super::verify_id_token(
            &token,
            signing.verifying_key().as_bytes(),
            "cardea-2026",
            "https://cardea.example",
            "cowboy-production",
            &"a".repeat(64),
            1_100,
        )
        .unwrap();
        assert_eq!(identity.subject, "draven");
        assert_eq!(identity.authenticated_at, 900);
        assert!(
            super::verify_id_token(
                &token,
                signing.verifying_key().as_bytes(),
                "cardea-2026",
                "https://cardea.example",
                "other-client",
                &"a".repeat(64),
                1_100,
            )
            .is_none()
        );

        let mut tampered = token.into_bytes();
        let last = tampered.last_mut().unwrap();
        *last = if *last == b'A' { b'B' } else { b'A' };
        assert!(
            super::verify_id_token(
                std::str::from_utf8(&tampered).unwrap(),
                signing.verifying_key().as_bytes(),
                "cardea-2026",
                "https://cardea.example",
                "cowboy-production",
                &"a".repeat(64),
                1_100,
            )
            .is_none()
        );
    }
}
