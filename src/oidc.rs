//! Cardea OIDC Authorization Code + PKCE consumer profile.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read as _;
use std::net::IpAddr;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use base64::Engine as _;
use cowboy_plugin_sdk::{
    AuthenticationProtocol, AuthenticationProviderContract, OidcClientAuthenticationMethod,
    OidcIdTokenAlgorithm,
};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use p256::ecdsa::{Signature as P256Signature, SigningKey as P256SigningKey};
use p256::pkcs8::DecodePrivateKey as _;
use reqwest::header::CONTENT_TYPE;
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
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
const MAX_PAR_RESPONSE_BYTES: usize = 8 * 1_024;
const MAX_JWT_BYTES: usize = 8_192;
const MAX_AUTHENTICATION_METHODS: usize = 16;
const MAX_AUTHENTICATION_METHOD_BYTES: usize = 64;
const MAX_AUTHENTICATION_CONTEXT_BYTES: usize = 256;
const CLIENT_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OidcProviderRuntimeDocument {
    pub client_id: String,
    pub redirect_uri: String,
    pub subject: String,
    pub account: String,
    #[serde(default)]
    pub admin_account: Option<String>,
    pub client_authentication: OidcRuntimeClientAuthentication,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum OidcRuntimeClientAuthentication {
    ClientSecretPost {
        client_secret_file: PathBuf,
    },
    PrivateKeyJwtEd25519 {
        key_id: String,
        private_key_file: PathBuf,
    },
    AppleClientSecretEs256 {
        team_id: String,
        key_id: String,
        private_key_file: PathBuf,
    },
}

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PushedAuthorizationResponse {
    request_uri: String,
    expires_in: u64,
}

#[derive(Clone)]
pub struct OidcProvider {
    id: String,
    display_name: String,
    button_label: String,
    issuer: String,
    authorization_endpoint: Url,
    pushed_authorization_request_endpoint: Option<String>,
    token_endpoint: String,
    jwks_uri: Option<String>,
    scopes: String,
    authorization_parameters: BTreeMap<String, String>,
    client_id: String,
    client_authentication: RuntimeClientAuthentication,
    id_token_verifier: IdTokenVerifier,
    subject: String,
    account: String,
    admin_account: Option<String>,
    redirect_uri: String,
    http: reqwest::Client,
}

#[derive(Clone)]
enum RuntimeClientAuthentication {
    ClientSecretPost(String),
    PrivateKeyJwtEd25519 {
        key_id: String,
        signing_seed: [u8; 32],
    },
    AppleClientSecretEs256 {
        team_id: String,
        key_id: String,
        signing_key: P256SigningKey,
    },
}

#[derive(Clone)]
enum IdTokenVerifier {
    PinnedEd25519 {
        key_id: String,
        verifying_key: [u8; 32],
    },
    Jwks {
        allowed_algorithms: Vec<OidcIdTokenAlgorithm>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicProvider {
    pub id: String,
    pub display_name: String,
    pub button_label: String,
    pub start_url: String,
}

#[derive(Debug)]
struct StoredTransaction {
    provider_id: String,
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
    MacOs {
        code_challenge: String,
    },
    BrowserShell {
        code_challenge: String,
        handoff_challenge: String,
    },
}

#[derive(Debug)]
pub struct OidcTransaction {
    provider_id: String,
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
    provider_id: String,
    user_id: String,
    code_challenge: String,
    expires: Instant,
    created: Instant,
}

#[derive(Debug, Clone)]
enum BrowserHandoffState {
    Pending,
    Ready { user_id: String },
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserHandoffEvent {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug)]
struct StoredBrowserHandoff {
    provider_id: String,
    code_challenge: String,
    state: BrowserHandoffState,
    events: tokio::sync::watch::Sender<BrowserHandoffEvent>,
    expires: Instant,
    created: Instant,
}

#[derive(Debug, Default)]
pub struct NativeHandoffs {
    entries: parking_lot::Mutex<HashMap<String, StoredNativeHandoff>>,
    browser_entries: parking_lot::Mutex<HashMap<String, StoredBrowserHandoff>>,
}

#[derive(Debug)]
pub struct StartedNativeHandoff {
    pub location: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BrowserHandoffPoll {
    Pending,
    Ready { user_id: String },
    Failed,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    token_type: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    id_token: String,
}

#[derive(Debug)]
pub struct VerifiedIdentity {
    pub issuer: String,
    pub subject: String,
    pub issued_at: u64,
    pub authenticated_at: Option<u64>,
    pub authentication_context: Option<String>,
    pub authentication_methods: Vec<String>,
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

#[derive(Serialize)]
struct AppleClientSecretClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'static str,
    iat: u64,
    exp: u64,
}

#[derive(Deserialize)]
struct IdTokenHeader {
    alg: String,
    kid: String,
    #[serde(default)]
    typ: Option<String>,
}

#[derive(Deserialize)]
struct IdTokenClaims {
    iss: String,
    sub: String,
    aud: Audience,
    #[serde(default)]
    azp: Option<String>,
    nonce: String,
    iat: u64,
    exp: u64,
    #[serde(default)]
    auth_time: Option<u64>,
    #[serde(default)]
    acr: Option<String>,
    #[serde(default)]
    amr: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(value) => value == expected,
            Self::Many(values) => values.iter().any(|value| value == expected),
        }
    }

    fn requires_authorized_party(&self) -> bool {
        matches!(self, Self::Many(values) if values.len() > 1)
    }
}

#[derive(Debug, Deserialize)]
struct JsonWebKeySet {
    keys: Vec<JsonWebKey>,
}

#[derive(Debug, Deserialize)]
struct JsonWebKey {
    kty: String,
    kid: String,
    #[serde(default)]
    alg: Option<String>,
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default, rename = "use")]
    usage: Option<String>,
    #[serde(default)]
    key_ops: Option<Vec<String>>,
}

impl JsonWebKeySet {
    fn signing_key(&self, key_id: &str, algorithm: &str) -> Option<&JsonWebKey> {
        if self.keys.is_empty() || self.keys.len() > 64 {
            return None;
        }
        let mut matches = self.keys.iter().filter(|key| {
            key.kid == key_id
                && key.alg.as_deref().is_none_or(|value| value == algorithm)
                && key.usage.as_deref().is_none_or(|value| value == "sig")
                && key
                    .key_ops
                    .as_ref()
                    .is_none_or(|operations| operations.iter().any(|value| value == "verify"))
        });
        let selected = matches.next()?;
        matches.next().is_none().then_some(selected)
    }
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
            valid_label(&document.display_name, 64),
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
            id: "cardea".to_owned(),
            display_name: document.display_name,
            button_label: "Continue with Cardea".to_owned(),
            issuer,
            authorization_endpoint,
            pushed_authorization_request_endpoint: None,
            token_endpoint,
            jwks_uri: None,
            scopes: "openid".to_owned(),
            authorization_parameters: BTreeMap::new(),
            client_id: document.client_id,
            client_authentication: RuntimeClientAuthentication::PrivateKeyJwtEd25519 {
                key_id: document.client_key_id,
                signing_seed: client_signing_seed,
            },
            id_token_verifier: IdTokenVerifier::PinnedEd25519 {
                key_id: document.id_token_key_id,
                verifying_key: id_token_verifying_key,
            },
            subject: document.subject,
            account,
            admin_account,
            redirect_uri: document.redirect_uri,
            http,
        })
    }

    pub(crate) fn load_plugin(
        contract: &AuthenticationProviderContract,
        runtime: OidcProviderRuntimeDocument,
    ) -> Result<Self> {
        let AuthenticationProtocol::OpenIdConnect(protocol) = &contract.protocol;
        anyhow::ensure!(
            valid_identifier(&contract.id),
            "authentication Plugin id is invalid"
        );
        anyhow::ensure!(
            valid_label(&contract.display_name, 64) && valid_label(&contract.button_label, 80),
            "authentication Plugin labels are invalid"
        );
        anyhow::ensure!(
            valid_oidc_token(&runtime.client_id, 255),
            "OIDC client id is invalid"
        );
        anyhow::ensure!(valid_subject(&runtime.subject), "OIDC subject is invalid");
        let issuer = exact_https_url(&protocol.issuer)?;
        anyhow::ensure!(
            issuer.query().is_none(),
            "OIDC issuer must not contain a query"
        );
        let authorization_endpoint = exact_https_url(&protocol.authorization_endpoint)?;
        let pushed_authorization_request_endpoint = protocol
            .pushed_authorization_request_endpoint
            .as_deref()
            .map(exact_https_url)
            .transpose()?
            .map(Into::into);
        let token_endpoint = exact_https_url(&protocol.token_endpoint)?.into();
        let jwks_uri = exact_https_url(&protocol.jwks_uri)?.into();
        let redirect = exact_https_url(&runtime.redirect_uri)?;
        let scoped_callback = format!("/api/auth/providers/{}/callback", contract.id);
        let legacy_cardea_callback =
            contract.id == "cardea" && redirect.path() == "/api/auth/oidc/callback";
        anyhow::ensure!(
            (redirect.path() == scoped_callback || legacy_cardea_callback)
                && redirect.query().is_none(),
            "OIDC redirect_uri must use the configured provider callback"
        );
        let account = crate::product_auth::normalize_username(&runtime.account)
            .context("normalizing OIDC account mapping")?;
        let admin_account = runtime
            .admin_account
            .as_deref()
            .map(crate::product_auth::normalize_username)
            .transpose()
            .context("normalizing OIDC admin account mapping")?;
        let client_authentication = match runtime.client_authentication {
            OidcRuntimeClientAuthentication::ClientSecretPost { client_secret_file } => {
                anyhow::ensure!(
                    protocol
                        .client_authentication_methods
                        .contains(&OidcClientAuthenticationMethod::ClientSecretPost),
                    "Authentication Provider does not allow client_secret_post"
                );
                let secret = read_secret_text(&client_secret_file, "OIDC client secret")?;
                anyhow::ensure!(
                    secret.len() <= 4_096 && !secret.chars().any(char::is_control),
                    "OIDC client secret is invalid"
                );
                RuntimeClientAuthentication::ClientSecretPost(secret)
            }
            OidcRuntimeClientAuthentication::PrivateKeyJwtEd25519 {
                key_id,
                private_key_file,
            } => {
                anyhow::ensure!(
                    protocol
                        .client_authentication_methods
                        .contains(&OidcClientAuthenticationMethod::PrivateKeyJwtEd25519),
                    "Authentication Provider does not allow Ed25519 private_key_jwt"
                );
                anyhow::ensure!(
                    valid_oidc_token(&key_id, 255),
                    "OIDC client key id is invalid"
                );
                anyhow::ensure!(
                    private_key_file.is_absolute(),
                    "OIDC client private key file must be absolute"
                );
                let private: PrivateJwk =
                    serde_json::from_slice(&read_protected_file(&private_key_file)?)
                        .context("decoding OIDC client private JWK")?;
                anyhow::ensure!(
                    private.kty == "OKP" && private.crv == "Ed25519",
                    "OIDC client key must be Ed25519"
                );
                let signing_seed = decode_32(&private.d, "OIDC client private key")?;
                let public = decode_32(&private.x, "OIDC client public key")?;
                anyhow::ensure!(
                    SigningKey::from_bytes(&signing_seed)
                        .verifying_key()
                        .as_bytes()
                        == &public,
                    "OIDC client private JWK is inconsistent"
                );
                RuntimeClientAuthentication::PrivateKeyJwtEd25519 {
                    key_id,
                    signing_seed,
                }
            }
            OidcRuntimeClientAuthentication::AppleClientSecretEs256 {
                team_id,
                key_id,
                private_key_file,
            } => {
                anyhow::ensure!(
                    protocol
                        .client_authentication_methods
                        .contains(&OidcClientAuthenticationMethod::AppleClientSecretEs256),
                    "Authentication Provider does not allow Apple client-secret JWT"
                );
                anyhow::ensure!(
                    valid_identifier(&team_id) && valid_identifier(&key_id),
                    "Apple OIDC identifiers are invalid"
                );
                let pem = read_secret_text(&private_key_file, "Apple private key")?;
                let signing_key = P256SigningKey::from_pkcs8_pem(&pem)
                    .context("decoding Apple ES256 private key")?;
                RuntimeClientAuthentication::AppleClientSecretEs256 {
                    team_id,
                    key_id,
                    signing_key,
                }
            }
        };
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .context("building OIDC HTTP client")?;
        Ok(Self {
            id: contract.id.clone(),
            display_name: contract.display_name.clone(),
            button_label: contract.button_label.clone(),
            issuer: protocol.issuer.clone(),
            authorization_endpoint,
            pushed_authorization_request_endpoint,
            token_endpoint,
            jwks_uri: Some(jwks_uri),
            scopes: protocol.scopes.join(" "),
            authorization_parameters: protocol.authorization_parameters.clone(),
            client_id: runtime.client_id,
            client_authentication,
            id_token_verifier: IdTokenVerifier::Jwks {
                allowed_algorithms: protocol.id_token_signing_algorithms.clone(),
            },
            subject: runtime.subject,
            account,
            admin_account,
            redirect_uri: runtime.redirect_uri,
            http,
        })
    }

    #[must_use]
    pub fn public(&self) -> PublicProvider {
        let start_url = if self.id == "cardea"
            && Url::parse(&self.redirect_uri)
                .is_ok_and(|redirect| redirect.path() == "/api/auth/oidc/callback")
        {
            "/api/auth/oidc/start".to_owned()
        } else {
            format!("/api/auth/providers/{}/start", self.id)
        };
        PublicProvider {
            id: self.id.clone(),
            display_name: self.display_name.clone(),
            button_label: self.button_label.clone(),
            start_url,
        }
    }

    #[must_use]
    pub fn requires_cross_site_post_cookie(&self) -> bool {
        self.authorization_parameters
            .get("response_mode")
            .is_some_and(|value| value == "form_post")
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }

    #[must_use]
    pub fn admin_account(&self) -> Option<&str> {
        self.admin_account.as_deref()
    }

    fn append_client_authentication(
        &self,
        form: &mut Vec<(String, String)>,
        audience: &str,
        now: u64,
    ) -> Result<()> {
        match &self.client_authentication {
            RuntimeClientAuthentication::ClientSecretPost(secret) => {
                form.push(("client_secret".to_owned(), secret.clone()));
            }
            RuntimeClientAuthentication::PrivateKeyJwtEd25519 {
                key_id,
                signing_seed,
            } => {
                let assertion_id = new_session_token()?;
                let assertion = client_assertion(
                    signing_seed,
                    key_id,
                    &self.client_id,
                    audience,
                    &assertion_id,
                    now,
                )
                .context("building OIDC client assertion")?;
                form.push((
                    "client_assertion_type".to_owned(),
                    CLIENT_ASSERTION_TYPE.to_owned(),
                ));
                form.push(("client_assertion".to_owned(), assertion));
            }
            RuntimeClientAuthentication::AppleClientSecretEs256 {
                team_id,
                key_id,
                signing_key,
            } => {
                form.push((
                    "client_secret".to_owned(),
                    apple_client_secret(signing_key, team_id, key_id, &self.client_id, now)
                        .context("building Apple client secret")?,
                ));
            }
        }
        Ok(())
    }

    fn authorization_parameters(
        &self,
        state: &str,
        nonce: &str,
        challenge: &str,
    ) -> Vec<(String, String)> {
        let mut parameters = vec![
            ("response_type".to_owned(), "code".to_owned()),
            ("client_id".to_owned(), self.client_id.clone()),
            ("redirect_uri".to_owned(), self.redirect_uri.clone()),
            ("scope".to_owned(), self.scopes.clone()),
            ("state".to_owned(), state.to_owned()),
            ("nonce".to_owned(), nonce.to_owned()),
            ("code_challenge".to_owned(), challenge.to_owned()),
            ("code_challenge_method".to_owned(), "S256".to_owned()),
        ];
        parameters.extend(
            self.authorization_parameters
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        parameters
    }

    fn pushed_authorization_location(&self, pushed: PushedAuthorizationResponse) -> Result<String> {
        let request_uri = Url::parse(&pushed.request_uri)
            .context("OIDC pushed authorization request URI is invalid")?;
        anyhow::ensure!(
            pushed.request_uri.len() <= 2_048
                && matches!(request_uri.scheme(), "https" | "urn")
                && request_uri.fragment().is_none()
                && (1..=TRANSACTION_TTL.as_secs()).contains(&pushed.expires_in),
            "OIDC pushed authorization response policy mismatch"
        );
        let mut authorization = self.authorization_endpoint.clone();
        authorization.query_pairs_mut().extend_pairs([
            ("client_id", self.client_id.as_str()),
            ("request_uri", pushed.request_uri.as_str()),
        ]);
        Ok(authorization.into())
    }

    async fn authorization_location(
        &self,
        state: &str,
        nonce: &str,
        challenge: &str,
    ) -> Result<String> {
        let mut parameters = self.authorization_parameters(state, nonce, challenge);

        let Some(endpoint) = self.pushed_authorization_request_endpoint.as_deref() else {
            let mut authorization = self.authorization_endpoint.clone();
            authorization.query_pairs_mut().extend_pairs(parameters);
            return Ok(authorization.into());
        };

        self.append_client_authentication(&mut parameters, endpoint, now_seconds()?)?;
        let response = self
            .http
            .post(endpoint)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&parameters)
            .send()
            .await
            .context("pushing OIDC authorization request")?;
        anyhow::ensure!(
            response.status().is_success(),
            "OIDC pushed authorization request rejected"
        );
        anyhow::ensure!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.split(';').next() == Some("application/json")),
            "OIDC pushed authorization endpoint returned an invalid content type"
        );
        anyhow::ensure!(
            response
                .content_length()
                .is_none_or(|length| length <= MAX_PAR_RESPONSE_BYTES as u64),
            "OIDC pushed authorization response is too large"
        );
        let bytes = response
            .bytes()
            .await
            .context("reading OIDC pushed authorization response")?;
        anyhow::ensure!(
            bytes.len() <= MAX_PAR_RESPONSE_BYTES,
            "OIDC pushed authorization response is too large"
        );
        let pushed: PushedAuthorizationResponse = serde_json::from_slice(&bytes)
            .context("decoding OIDC pushed authorization response")?;
        self.pushed_authorization_location(pushed)
    }

    pub async fn exchange(
        &self,
        transaction: &OidcTransaction,
        code: &str,
    ) -> Result<VerifiedIdentity> {
        anyhow::ensure!(valid_authorization_code(code), "invalid authorization code");
        let now = now_seconds()?;
        anyhow::ensure!(transaction.provider_id == self.id, "OIDC provider mismatch");
        let mut form = vec![
            ("grant_type".to_owned(), "authorization_code".to_owned()),
            ("client_id".to_owned(), self.client_id.clone()),
            ("code".to_owned(), code.to_owned()),
            ("redirect_uri".to_owned(), self.redirect_uri.clone()),
            (
                "code_verifier".to_owned(),
                transaction.pkce_verifier.clone(),
            ),
        ];
        self.append_client_authentication(&mut form, &self.token_endpoint, now)?;
        let response = self
            .http
            .post(&self.token_endpoint)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&form)
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
            token.token_type.eq_ignore_ascii_case("Bearer")
                && !token.access_token.is_empty()
                && !token.id_token.is_empty(),
            "OIDC token response policy mismatch"
        );
        if let Some(expires_in) = token.expires_in {
            anyhow::ensure!(
                (1..=3_600).contains(&expires_in),
                "OIDC token lifetime is invalid"
            );
        }
        if let Some(scope) = token.scope.as_deref() {
            anyhow::ensure!(
                scope.split_whitespace().any(|value| value == "openid"),
                "OIDC scope mismatch"
            );
        }
        let _ = token.access_token;
        let _ = token.refresh_token;
        let identity = self
            .verify_id_token(&token.id_token, &transaction.nonce, now)
            .await
            .context("OIDC ID token verification failed")?;
        anyhow::ensure!(
            identity.subject == self.subject,
            "OIDC subject is not allowed"
        );
        Ok(identity)
    }

    async fn verify_id_token(
        &self,
        token: &str,
        nonce: &str,
        now: u64,
    ) -> Option<VerifiedIdentity> {
        let decoded = decode_id_token(token, &self.issuer, &self.client_id, nonce, now)?;
        match &self.id_token_verifier {
            IdTokenVerifier::PinnedEd25519 {
                key_id,
                verifying_key,
            } => {
                if decoded.header.alg != "EdDSA" || decoded.header.kid != *key_id {
                    return None;
                }
                let signature_bytes: [u8; 64] = decoded.signature.try_into().ok()?;
                VerifyingKey::from_bytes(verifying_key)
                    .ok()?
                    .verify(
                        decoded.signing_input.as_bytes(),
                        &Signature::from_bytes(&signature_bytes),
                    )
                    .ok()?;
            }
            IdTokenVerifier::Jwks { allowed_algorithms } => {
                let algorithm = match decoded.header.alg.as_str() {
                    "EdDSA" => OidcIdTokenAlgorithm::EdDSA,
                    "RS256" => OidcIdTokenAlgorithm::RS256,
                    _ => return None,
                };
                if !allowed_algorithms.contains(&algorithm) {
                    return None;
                }
                let response = self.http.get(self.jwks_uri.as_deref()?).send().await.ok()?;
                if !response.status().is_success()
                    || response
                        .content_length()
                        .is_some_and(|length| length > 64 * 1_024)
                {
                    return None;
                }
                let bytes = response.bytes().await.ok()?;
                if bytes.len() > 64 * 1_024 {
                    return None;
                }
                let set: JsonWebKeySet = serde_json::from_slice(&bytes).ok()?;
                let key = set.signing_key(&decoded.header.kid, &decoded.header.alg)?;
                match algorithm {
                    OidcIdTokenAlgorithm::EdDSA => {
                        if key.kty != "OKP" || key.crv.as_deref() != Some("Ed25519") {
                            return None;
                        }
                        let verifying_key = decode_32(key.x.as_deref()?, "OIDC JWKS key").ok()?;
                        let signature_bytes: [u8; 64] = decoded.signature.try_into().ok()?;
                        VerifyingKey::from_bytes(&verifying_key)
                            .ok()?
                            .verify(
                                decoded.signing_input.as_bytes(),
                                &Signature::from_bytes(&signature_bytes),
                            )
                            .ok()?;
                    }
                    OidcIdTokenAlgorithm::RS256 => {
                        if key.kty != "RSA" {
                            return None;
                        }
                        let modulus = base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .decode(key.n.as_deref()?)
                            .ok()?;
                        let exponent = base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .decode(key.e.as_deref()?)
                            .ok()?;
                        RsaPublicKeyComponents {
                            n: &modulus,
                            e: &exponent,
                        }
                        .verify(
                            &RSA_PKCS1_2048_8192_SHA256,
                            decoded.signing_input.as_bytes(),
                            &decoded.signature,
                        )
                        .ok()?;
                    }
                }
            }
        }
        Some(verified_identity(decoded.claims))
    }
}

impl OidcTransactions {
    pub async fn begin(
        &self,
        provider: &OidcProvider,
        source_ip: IpAddr,
        target: AuthorizationTarget,
    ) -> Result<StartedAuthorization> {
        match &target {
            AuthorizationTarget::MacOs { code_challenge } => {
                anyhow::ensure!(
                    valid_pkce_challenge(code_challenge),
                    "invalid native PKCE challenge"
                );
            }
            AuthorizationTarget::BrowserShell {
                code_challenge,
                handoff_challenge,
            } => {
                anyhow::ensure!(
                    valid_pkce_challenge(code_challenge),
                    "invalid browser-shell PKCE challenge"
                );
                anyhow::ensure!(
                    valid_pkce_challenge(handoff_challenge),
                    "invalid browser-shell handoff challenge"
                );
            }
            AuthorizationTarget::Browser => {}
        }
        let cookie_token = new_session_token()?;
        let state = new_session_token()?;
        let nonce = new_session_token()?;
        let pkce_verifier = new_session_token()?;
        let challenge = pkce_challenge(&pkce_verifier).context("building OIDC PKCE challenge")?;
        let now = Instant::now();
        {
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
                    provider_id: provider.id.clone(),
                    state_hash: hex_sha256(state.as_bytes()),
                    nonce: nonce.clone(),
                    pkce_verifier,
                    source_ip,
                    expires: now + TRANSACTION_TTL,
                    created: now,
                    target,
                },
            );
        }
        let location = match provider
            .authorization_location(&state, &nonce, &challenge)
            .await
        {
            Ok(location) => location,
            Err(error) => {
                self.cancel(&cookie_token);
                return Err(error);
            }
        };
        Ok(StartedAuthorization {
            location,
            cookie_token,
        })
    }

    pub fn consume(
        &self,
        provider_id: &str,
        cookie_token: &str,
        state: &str,
    ) -> Result<OidcTransaction> {
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
            constant_time_equal(stored.provider_id.as_bytes(), provider_id.as_bytes()),
            "OIDC provider mismatch"
        );
        anyhow::ensure!(
            constant_time_equal(
                stored.state_hash.as_bytes(),
                hex_sha256(state.as_bytes()).as_bytes(),
            ),
            "OIDC state mismatch"
        );
        Ok(OidcTransaction {
            provider_id: stored.provider_id,
            nonce: stored.nonce,
            pkce_verifier: stored.pkce_verifier,
            target: stored.target,
        })
    }

    pub fn cancel(&self, cookie_token: &str) {
        if valid_opaque_value(cookie_token) {
            self.entries
                .lock()
                .remove(&hex_sha256(cookie_token.as_bytes()));
        }
    }
}

impl OidcTransaction {
    #[must_use]
    pub fn target(&self) -> &AuthorizationTarget {
        &self.target
    }
}

impl NativeHandoffs {
    pub fn begin_browser(
        &self,
        provider_id: &str,
        code_challenge: &str,
        handoff_challenge: &str,
    ) -> Result<()> {
        anyhow::ensure!(
            valid_identifier(provider_id),
            "invalid browser handoff provider"
        );
        anyhow::ensure!(
            valid_pkce_challenge(code_challenge),
            "invalid browser-shell PKCE challenge"
        );
        anyhow::ensure!(
            valid_pkce_challenge(handoff_challenge),
            "invalid browser-shell handoff challenge"
        );
        let key = hex_sha256(handoff_challenge.as_bytes());
        let now = Instant::now();
        let (events, _) = tokio::sync::watch::channel(BrowserHandoffEvent::Pending);
        let mut entries = self.browser_entries.lock();
        entries.retain(|_, row| row.expires > now);
        anyhow::ensure!(
            !entries.contains_key(&key),
            "browser handoff already exists"
        );
        if entries.len() >= MAX_NATIVE_HANDOFFS
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, row)| row.created)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(
            key,
            StoredBrowserHandoff {
                provider_id: provider_id.to_owned(),
                code_challenge: code_challenge.to_owned(),
                state: BrowserHandoffState::Pending,
                events,
                expires: now + TRANSACTION_TTL,
                created: now,
            },
        );
        Ok(())
    }

    pub fn complete_browser(
        &self,
        provider_id: &str,
        handoff_challenge: &str,
        user_id: &str,
    ) -> Result<()> {
        anyhow::ensure!(
            valid_identifier(provider_id),
            "invalid browser handoff provider"
        );
        anyhow::ensure!(valid_identifier(user_id), "invalid browser handoff user");
        anyhow::ensure!(
            valid_pkce_challenge(handoff_challenge),
            "invalid browser-shell handoff challenge"
        );
        let now = Instant::now();
        let mut entries = self.browser_entries.lock();
        entries.retain(|_, row| row.expires > now);
        let stored = entries
            .get_mut(&hex_sha256(handoff_challenge.as_bytes()))
            .context("browser handoff expired")?;
        anyhow::ensure!(
            constant_time_equal(stored.provider_id.as_bytes(), provider_id.as_bytes()),
            "browser handoff provider mismatch"
        );
        stored.state = BrowserHandoffState::Ready {
            user_id: user_id.to_owned(),
        };
        stored.events.send_replace(BrowserHandoffEvent::Ready);
        Ok(())
    }

    pub fn fail_browser(&self, provider_id: &str, handoff_challenge: &str) -> Result<()> {
        anyhow::ensure!(
            valid_identifier(provider_id),
            "invalid browser handoff provider"
        );
        anyhow::ensure!(
            valid_pkce_challenge(handoff_challenge),
            "invalid browser-shell handoff challenge"
        );
        let now = Instant::now();
        let mut entries = self.browser_entries.lock();
        entries.retain(|_, row| row.expires > now);
        let stored = entries
            .get_mut(&hex_sha256(handoff_challenge.as_bytes()))
            .context("browser handoff expired")?;
        anyhow::ensure!(
            constant_time_equal(stored.provider_id.as_bytes(), provider_id.as_bytes()),
            "browser handoff provider mismatch"
        );
        stored.state = BrowserHandoffState::Failed;
        stored.events.send_replace(BrowserHandoffEvent::Failed);
        Ok(())
    }

    pub fn subscribe_browser(
        &self,
        provider_id: &str,
        handoff_token: &str,
        code_verifier: &str,
    ) -> Result<tokio::sync::watch::Receiver<BrowserHandoffEvent>> {
        anyhow::ensure!(
            valid_identifier(provider_id),
            "invalid browser handoff provider"
        );
        let handoff_challenge =
            pkce_challenge(handoff_token).context("invalid browser handoff token")?;
        let code_challenge =
            pkce_challenge(code_verifier).context("invalid browser-shell PKCE verifier")?;
        let key = hex_sha256(handoff_challenge.as_bytes());
        let now = Instant::now();
        let mut entries = self.browser_entries.lock();
        entries.retain(|_, row| row.expires > now);
        let stored = entries.get(&key).context("browser handoff expired")?;
        anyhow::ensure!(
            constant_time_equal(stored.provider_id.as_bytes(), provider_id.as_bytes()),
            "browser handoff provider mismatch"
        );
        anyhow::ensure!(
            constant_time_equal(code_challenge.as_bytes(), stored.code_challenge.as_bytes()),
            "browser-shell PKCE mismatch"
        );
        Ok(stored.events.subscribe())
    }

    pub fn poll_browser(
        &self,
        provider_id: &str,
        handoff_token: &str,
        code_verifier: &str,
    ) -> Result<BrowserHandoffPoll> {
        anyhow::ensure!(
            valid_identifier(provider_id),
            "invalid browser handoff provider"
        );
        let handoff_challenge =
            pkce_challenge(handoff_token).context("invalid browser handoff token")?;
        let code_challenge =
            pkce_challenge(code_verifier).context("invalid browser-shell PKCE verifier")?;
        let key = hex_sha256(handoff_challenge.as_bytes());
        let now = Instant::now();
        let mut entries = self.browser_entries.lock();
        entries.retain(|_, row| row.expires > now);
        let stored = entries.remove(&key).context("browser handoff expired")?;
        anyhow::ensure!(
            constant_time_equal(stored.provider_id.as_bytes(), provider_id.as_bytes()),
            "browser handoff provider mismatch"
        );
        anyhow::ensure!(
            constant_time_equal(code_challenge.as_bytes(), stored.code_challenge.as_bytes()),
            "browser-shell PKCE mismatch"
        );
        match stored.state {
            BrowserHandoffState::Pending => {
                entries.insert(key, stored);
                Ok(BrowserHandoffPoll::Pending)
            }
            BrowserHandoffState::Ready { user_id } => Ok(BrowserHandoffPoll::Ready { user_id }),
            BrowserHandoffState::Failed => Ok(BrowserHandoffPoll::Failed),
        }
    }

    pub fn cancel_browser(
        &self,
        provider_id: &str,
        handoff_token: &str,
        code_verifier: &str,
    ) -> Result<()> {
        anyhow::ensure!(
            valid_identifier(provider_id),
            "invalid browser handoff provider"
        );
        let handoff_challenge =
            pkce_challenge(handoff_token).context("invalid browser handoff token")?;
        let code_challenge =
            pkce_challenge(code_verifier).context("invalid browser-shell PKCE verifier")?;
        let key = hex_sha256(handoff_challenge.as_bytes());
        let now = Instant::now();
        let mut entries = self.browser_entries.lock();
        entries.retain(|_, row| row.expires > now);
        let stored = entries.remove(&key).context("browser handoff expired")?;
        anyhow::ensure!(
            constant_time_equal(stored.provider_id.as_bytes(), provider_id.as_bytes()),
            "browser handoff provider mismatch"
        );
        anyhow::ensure!(
            constant_time_equal(code_challenge.as_bytes(), stored.code_challenge.as_bytes()),
            "browser-shell PKCE mismatch"
        );
        stored.events.send_replace(BrowserHandoffEvent::Failed);
        Ok(())
    }

    pub fn issue(
        &self,
        provider_id: &str,
        user_id: &str,
        code_challenge: &str,
    ) -> Result<StartedNativeHandoff> {
        anyhow::ensure!(
            valid_identifier(provider_id),
            "invalid native handoff provider"
        );
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
                provider_id: provider_id.to_owned(),
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

    pub fn consume(&self, provider_id: &str, code: &str, verifier: &str) -> Result<String> {
        anyhow::ensure!(
            valid_identifier(provider_id),
            "invalid native handoff provider"
        );
        anyhow::ensure!(valid_opaque_value(code), "invalid native handoff");
        let challenge = pkce_challenge(verifier).context("invalid native PKCE verifier")?;
        let stored = self
            .entries
            .lock()
            .remove(&hex_sha256(code.as_bytes()))
            .context("native handoff expired")?;
        anyhow::ensure!(stored.expires > Instant::now(), "native handoff expired");
        anyhow::ensure!(
            constant_time_equal(stored.provider_id.as_bytes(), provider_id.as_bytes()),
            "native handoff provider mismatch"
        );
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
pub fn transaction_cookie(token: &str, secure: bool, cross_site_post: bool) -> String {
    let same_site = if cross_site_post { "None" } else { "Lax" };
    let mut cookie = format!(
        "{TRANSACTION_COOKIE}={token}; Path=/api/auth; HttpOnly; SameSite={same_site}; Max-Age={}",
        TRANSACTION_TTL.as_secs()
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[must_use]
pub fn clear_transaction_cookie(secure: bool) -> String {
    let mut cookie =
        format!("{TRANSACTION_COOKIE}=; Path=/api/auth; HttpOnly; SameSite=Lax; Max-Age=0");
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

fn read_secret_text(path: &Path, label: &str) -> Result<String> {
    anyhow::ensure!(path.is_absolute(), "{label} file must be absolute");
    let bytes = read_protected_file(path)?;
    let value = std::str::from_utf8(&bytes)
        .with_context(|| format!("decoding {label}"))?
        .trim()
        .to_owned();
    anyhow::ensure!(!value.is_empty(), "{label} is empty");
    Ok(value)
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
    if !valid_oidc_token(key_id, 255)
        || !valid_oidc_token(client_id, 255)
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

fn apple_client_secret(
    signing_key: &P256SigningKey,
    team_id: &str,
    key_id: &str,
    client_id: &str,
    now: u64,
) -> Option<String> {
    if !valid_identifier(team_id) || !valid_identifier(key_id) || !valid_oidc_token(client_id, 255)
    {
        return None;
    }
    let header = encode_json(&ClientAssertionHeader {
        alg: "ES256",
        kid: key_id,
        typ: "JWT",
    })?;
    let claims = encode_json(&AppleClientSecretClaims {
        iss: team_id,
        sub: client_id,
        aud: "https://appleid.apple.com",
        iat: now,
        exp: now.checked_add(300)?,
    })?;
    let signing_input = format!("{header}.{claims}");
    let signature: P256Signature = signing_key.sign(signing_input.as_bytes());
    Some(format!(
        "{signing_input}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

struct DecodedIdToken {
    header: IdTokenHeader,
    claims: IdTokenClaims,
    signature: Vec<u8>,
    signing_input: String,
}

fn decode_id_token(
    token: &str,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now: u64,
) -> Option<DecodedIdToken> {
    if token.len() > MAX_JWT_BYTES
        || !valid_oidc_token(client_id, 255)
        || !valid_opaque_value(nonce)
        || exact_https_url(issuer).is_err()
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
    if header_value.kid.is_empty()
        || header_value
            .typ
            .as_deref()
            .is_some_and(|value| value != "JWT")
    {
        return None;
    }
    let claims_value: IdTokenClaims = decode_json(claims)?;
    if claims_value.iss != issuer
        || !claims_value.aud.contains(client_id)
        || (claims_value.aud.requires_authorized_party()
            && claims_value.azp.as_deref() != Some(client_id))
        || claims_value
            .azp
            .as_deref()
            .is_some_and(|authorized_party| authorized_party != client_id)
        || claims_value.nonce != nonce
        || !valid_subject(&claims_value.sub)
        || claims_value.iat > now.saturating_add(60)
        || claims_value
            .auth_time
            .is_some_and(|authenticated_at| authenticated_at > claims_value.iat)
        || claims_value
            .acr
            .as_deref()
            .is_some_and(|value| !valid_label(value, MAX_AUTHENTICATION_CONTEXT_BYTES))
        || claims_value.amr.len() > MAX_AUTHENTICATION_METHODS
        || claims_value
            .amr
            .iter()
            .any(|value| !valid_label(value, MAX_AUTHENTICATION_METHOD_BYTES))
        || claims_value.amr.iter().collect::<BTreeSet<_>>().len() != claims_value.amr.len()
        || claims_value.exp <= now
        || claims_value.exp > claims_value.iat.checked_add(3_600)?
    {
        return None;
    }
    Some(DecodedIdToken {
        header: header_value,
        claims: claims_value,
        signature: base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(signature)
            .ok()?,
        signing_input: format!("{header}.{claims}"),
    })
}

#[cfg(test)]
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
    let decoded = decode_id_token(token, issuer, client_id, nonce, now)?;
    if decoded.header.alg != "EdDSA" || decoded.header.kid != key_id {
        return None;
    }
    let signature_bytes: [u8; 64] = decoded.signature.try_into().ok()?;
    let signature = Signature::from_bytes(&signature_bytes);
    VerifyingKey::from_bytes(verifying_key)
        .ok()?
        .verify(decoded.signing_input.as_bytes(), &signature)
        .ok()?;
    Some(verified_identity(decoded.claims))
}

fn verified_identity(claims: IdTokenClaims) -> VerifiedIdentity {
    VerifiedIdentity {
        issuer: claims.iss,
        subject: claims.sub,
        issued_at: claims.iat,
        authenticated_at: claims.auth_time,
        authentication_context: claims.acr,
        authentication_methods: claims.amr,
    }
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

fn valid_subject(value: &str) -> bool {
    (1..=255).contains(&value.len())
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_oidc_token(value: &str, maximum: usize) -> bool {
    (1..=maximum).contains(&value.len()) && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

fn valid_label(value: &str, maximum: usize) -> bool {
    (1..=maximum).contains(&value.len())
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_opaque_value(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_authorization_code(value: &str) -> bool {
    valid_oidc_token(value, 1_024)
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
        IdTokenVerifier, NATIVE_CALLBACK_SCHEME, NativeHandoffs, OidcProvider, OidcTransactions,
        PushedAuthorizationResponse, RuntimeClientAuthentication, TRANSACTION_COOKIE,
        clear_transaction_cookie, read_protected_file,
    };
    use base64::Engine as _;
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::collections::{BTreeMap, HashMap};
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    #[test]
    fn transaction_cookie_is_callback_scoped() {
        let cleared = clear_transaction_cookie(true);
        assert!(cleared.starts_with(&format!("{TRANSACTION_COOKIE}=;")));
        assert!(cleared.contains("Path=/api/auth"));
        assert!(cleared.contains("SameSite=Lax"));
        assert!(cleared.contains("Secure"));
    }

    #[test]
    fn form_post_transaction_cookie_is_cross_site_and_secure() {
        let cookie = super::transaction_cookie(&"a".repeat(64), true, true);
        assert!(cookie.contains("SameSite=None"));
        assert!(cookie.contains("Secure"));
        let ordinary = super::transaction_cookie(&"a".repeat(64), true, false);
        assert!(ordinary.contains("SameSite=Lax"));
    }

    #[test]
    fn pushed_authorization_is_signed_and_browser_url_contains_only_request_uri() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let par_endpoint = "https://cardea.example/oauth2/par".to_owned();
        let signing = SigningKey::from_bytes(&[7; 32]);
        let provider = OidcProvider {
            id: "cardea".to_owned(),
            display_name: "Cardea".to_owned(),
            button_label: "Continue with Cardea".to_owned(),
            issuer: "https://cardea.example".to_owned(),
            authorization_endpoint: url::Url::parse("https://cardea.example/oauth2/authorize")
                .unwrap(),
            pushed_authorization_request_endpoint: Some(par_endpoint.clone()),
            token_endpoint: "https://cardea.example/oauth2/token".to_owned(),
            jwks_uri: None,
            scopes: "openid".to_owned(),
            authorization_parameters: BTreeMap::from([(
                "approval_mode".to_owned(),
                "manual".to_owned(),
            )]),
            client_id: "cowboy-production".to_owned(),
            client_authentication: RuntimeClientAuthentication::PrivateKeyJwtEd25519 {
                key_id: "cowboy-2026".to_owned(),
                signing_seed: signing.to_bytes(),
            },
            id_token_verifier: IdTokenVerifier::PinnedEd25519 {
                key_id: "cardea-2026".to_owned(),
                verifying_key: signing.verifying_key().to_bytes(),
            },
            subject: "draven".to_owned(),
            account: "draven".to_owned(),
            admin_account: Some("draven".to_owned()),
            redirect_uri: "https://cowboy.example/api/auth/oidc/callback".to_owned(),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        };

        let mut form =
            provider.authorization_parameters(&"s".repeat(64), &"n".repeat(64), &"c".repeat(43));
        provider
            .append_client_authentication(&mut form, &par_endpoint, 1_000)
            .unwrap();
        let form: HashMap<_, _> = form.into_iter().collect();
        let expected_request_uri = format!("urn:ietf:params:oauth:request_uri:{}", "r".repeat(43));
        let location = provider
            .pushed_authorization_location(PushedAuthorizationResponse {
                request_uri: expected_request_uri.clone(),
                expires_in: 300,
            })
            .unwrap();
        let location = url::Url::parse(&location).unwrap();
        let query: HashMap<_, _> = location.query_pairs().into_owned().collect();
        let expected_code_challenge = "c".repeat(43);
        assert_eq!(query.len(), 2);
        assert_eq!(
            query.get("client_id").map(String::as_str),
            Some("cowboy-production")
        );
        assert_eq!(
            query.get("request_uri").map(String::as_str),
            Some(expected_request_uri.as_str())
        );

        assert_eq!(
            form.get("approval_mode").map(String::as_str),
            Some("manual")
        );
        assert_eq!(
            form.get("code_challenge").map(String::as_str),
            Some(expected_code_challenge.as_str())
        );
        assert!(!form.contains_key("client_secret"));
        let assertion = form.get("client_assertion").unwrap();
        let claims = assertion.split('.').nth(1).unwrap();
        let claims: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(claims)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["aud"], par_endpoint);
        assert_eq!(claims["iss"], "cowboy-production");
        assert_eq!(claims["sub"], "cowboy-production");
    }

    #[test]
    fn generic_oidc_values_accept_opaque_provider_syntax_but_not_controls() {
        assert!(super::valid_authorization_code(
            "4/0AVMBsJg.example~opaque-code"
        ));
        assert!(super::valid_oidc_token(
            "client:https://tenant.example/application",
            255
        ));
        assert!(!super::valid_authorization_code("code\nheader"));
        assert!(!super::valid_oidc_token("client id", 255));
    }

    #[test]
    fn missing_transaction_fails_closed() {
        assert!(
            OidcTransactions::default()
                .consume("cardea", &"a".repeat(64), &"b".repeat(64))
                .is_err()
        );
    }

    #[test]
    fn native_handoff_is_fixed_single_use_pkce() {
        let handoffs = NativeHandoffs::default();
        let verifier = "a".repeat(64);
        let challenge = super::pkce_challenge(&verifier).unwrap();
        let started = handoffs
            .issue("cardea", &"b".repeat(32), &challenge)
            .unwrap();
        let callback = url::Url::parse(&started.location).unwrap();
        assert_eq!(callback.scheme(), NATIVE_CALLBACK_SCHEME);
        assert_eq!(callback.host_str(), Some("auth"));
        assert_eq!(callback.path(), "/callback");
        let code = callback
            .query_pairs()
            .find(|(name, _)| name == "code")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        assert_eq!(
            handoffs.consume("cardea", &code, &verifier).unwrap(),
            "b".repeat(32)
        );
        assert!(handoffs.consume("cardea", &code, &verifier).is_err());
    }

    #[test]
    fn native_handoff_wrong_verifier_fails_closed_and_consumes_code() {
        let handoffs = NativeHandoffs::default();
        let verifier = "a".repeat(64);
        let challenge = super::pkce_challenge(&verifier).unwrap();
        let started = handoffs
            .issue("cardea", &"b".repeat(32), &challenge)
            .unwrap();
        let callback = url::Url::parse(&started.location).unwrap();
        let code = callback
            .query_pairs()
            .find(|(name, _)| name == "code")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        assert!(handoffs.consume("cardea", &code, &"c".repeat(64)).is_err());
        assert!(handoffs.consume("cardea", &code, &verifier).is_err());
    }

    #[test]
    fn browser_handoff_is_pending_then_ready_and_single_use() {
        let handoffs = NativeHandoffs::default();
        let verifier = "v".repeat(64);
        let handoff_token = "h".repeat(64);
        let code_challenge = super::pkce_challenge(&verifier).unwrap();
        let handoff_challenge = super::pkce_challenge(&handoff_token).unwrap();
        handoffs
            .begin_browser("cardea", &code_challenge, &handoff_challenge)
            .unwrap();
        assert_eq!(
            handoffs
                .poll_browser("cardea", &handoff_token, &verifier)
                .unwrap(),
            super::BrowserHandoffPoll::Pending
        );
        handoffs
            .complete_browser("cardea", &handoff_challenge, &"b".repeat(32))
            .unwrap();
        assert_eq!(
            handoffs
                .poll_browser("cardea", &handoff_token, &verifier)
                .unwrap(),
            super::BrowserHandoffPoll::Ready {
                user_id: "b".repeat(32)
            }
        );
        assert!(
            handoffs
                .poll_browser("cardea", &handoff_token, &verifier)
                .is_err()
        );
    }

    #[tokio::test]
    async fn browser_handoff_pushes_only_bound_state_changes() {
        let handoffs = NativeHandoffs::default();
        let verifier = "v".repeat(64);
        let handoff_token = "h".repeat(64);
        let code_challenge = super::pkce_challenge(&verifier).unwrap();
        let handoff_challenge = super::pkce_challenge(&handoff_token).unwrap();
        handoffs
            .begin_browser("cardea", &code_challenge, &handoff_challenge)
            .unwrap();
        assert!(
            handoffs
                .subscribe_browser("cardea", &handoff_token, &"w".repeat(64))
                .is_err()
        );
        let mut events = handoffs
            .subscribe_browser("cardea", &handoff_token, &verifier)
            .unwrap();
        assert_eq!(
            *events.borrow_and_update(),
            super::BrowserHandoffEvent::Pending
        );
        handoffs
            .complete_browser("cardea", &handoff_challenge, &"b".repeat(32))
            .unwrap();
        events.changed().await.unwrap();
        assert_eq!(
            *events.borrow_and_update(),
            super::BrowserHandoffEvent::Ready
        );
        assert_eq!(
            handoffs
                .poll_browser("cardea", &handoff_token, &verifier)
                .unwrap(),
            super::BrowserHandoffPoll::Ready {
                user_id: "b".repeat(32)
            }
        );
    }

    #[test]
    fn browser_handoff_wrong_verifier_fails_closed() {
        let handoffs = NativeHandoffs::default();
        let verifier = "v".repeat(64);
        let handoff_token = "h".repeat(64);
        let code_challenge = super::pkce_challenge(&verifier).unwrap();
        let handoff_challenge = super::pkce_challenge(&handoff_token).unwrap();
        handoffs
            .begin_browser("cardea", &code_challenge, &handoff_challenge)
            .unwrap();
        handoffs
            .complete_browser("cardea", &handoff_challenge, &"b".repeat(32))
            .unwrap();
        assert!(
            handoffs
                .poll_browser("cardea", &handoff_token, &"w".repeat(64))
                .is_err()
        );
        assert!(
            handoffs
                .poll_browser("cardea", &handoff_token, &verifier)
                .is_err()
        );
    }

    #[test]
    fn browser_handoff_reports_authorization_failure_once() {
        let handoffs = NativeHandoffs::default();
        let verifier = "v".repeat(64);
        let handoff_token = "h".repeat(64);
        let code_challenge = super::pkce_challenge(&verifier).unwrap();
        let handoff_challenge = super::pkce_challenge(&handoff_token).unwrap();
        handoffs
            .begin_browser("cardea", &code_challenge, &handoff_challenge)
            .unwrap();
        handoffs.fail_browser("cardea", &handoff_challenge).unwrap();
        assert_eq!(
            handoffs
                .poll_browser("cardea", &handoff_token, &verifier)
                .unwrap(),
            super::BrowserHandoffPoll::Failed
        );
        assert!(
            handoffs
                .poll_browser("cardea", &handoff_token, &verifier)
                .is_err()
        );
    }

    #[test]
    fn browser_handoff_cancel_requires_both_proofs_and_is_single_use() {
        let handoffs = NativeHandoffs::default();
        let verifier = "v".repeat(64);
        let handoff_token = "h".repeat(64);
        let code_challenge = super::pkce_challenge(&verifier).unwrap();
        let handoff_challenge = super::pkce_challenge(&handoff_token).unwrap();
        handoffs
            .begin_browser("cardea", &code_challenge, &handoff_challenge)
            .unwrap();
        handoffs
            .cancel_browser("cardea", &handoff_token, &verifier)
            .unwrap();
        assert!(
            handoffs
                .poll_browser("cardea", &handoff_token, &verifier)
                .is_err()
        );

        handoffs
            .begin_browser("cardea", &code_challenge, &handoff_challenge)
            .unwrap();
        assert!(
            handoffs
                .cancel_browser("cardea", &handoff_token, &"w".repeat(64))
                .is_err()
        );
        assert!(
            handoffs
                .cancel_browser("cardea", &handoff_token, &verifier)
                .is_err()
        );
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
            "acr": "urn:cardea:assurance:manual-approval",
            "amr": ["pwd", "hwk"],
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
        assert_eq!(identity.issuer, "https://cardea.example");
        assert_eq!(identity.subject, "draven");
        assert_eq!(identity.issued_at, 1_000);
        assert_eq!(identity.authenticated_at, Some(900));
        assert_eq!(
            identity.authentication_context.as_deref(),
            Some("urn:cardea:assurance:manual-approval")
        );
        assert_eq!(
            identity.authentication_methods,
            vec!["pwd".to_owned(), "hwk".to_owned()]
        );
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

    #[test]
    fn oidc_id_token_accepts_standard_extra_claims_and_requires_azp_for_multiple_audiences() {
        let signing = SigningKey::from_bytes(&[9; 32]);
        let header = super::encode_json(&serde_json::json!({
            "alg": "EdDSA",
            "kid": "cardea-2026",
        }))
        .unwrap();
        let claims = super::encode_json(&serde_json::json!({
            "iss": "https://cardea.example",
            "sub": "opaque-subject:with/provider-format",
            "aud": ["cowboy-production", "another-audience"],
            "azp": "cowboy-production",
            "nonce": "a".repeat(64),
            "iat": 1_000,
            "exp": 1_300,
            "email": "presentation-only@example.test",
            "email_verified": true,
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
        assert_eq!(identity.authenticated_at, None);
        assert_eq!(identity.authentication_context, None);
        assert!(identity.authentication_methods.is_empty());

        let claims_without_azp = super::encode_json(&serde_json::json!({
            "iss": "https://cardea.example",
            "sub": "opaque-subject",
            "aud": ["cowboy-production", "another-audience"],
            "nonce": "a".repeat(64),
            "iat": 1_000,
            "exp": 1_300,
        }))
        .unwrap();
        let input = format!("{header}.{claims_without_azp}");
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(signing.sign(input.as_bytes()).to_bytes());
        assert!(
            super::verify_id_token(
                &format!("{input}.{signature}"),
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

    #[test]
    fn oidc_id_token_rejects_ambiguous_or_unbounded_assurance_claims() {
        let signing = SigningKey::from_bytes(&[10; 32]);
        let header = super::encode_json(&serde_json::json!({
            "alg": "EdDSA",
            "kid": "cardea-2026",
        }))
        .unwrap();
        let sign = |claims| {
            let claims = super::encode_json(&claims).unwrap();
            let input = format!("{header}.{claims}");
            let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(signing.sign(input.as_bytes()).to_bytes());
            format!("{input}.{signature}")
        };
        let verify = |token: &str| {
            super::verify_id_token(
                token,
                signing.verifying_key().as_bytes(),
                "cardea-2026",
                "https://cardea.example",
                "cowboy-production",
                &"a".repeat(64),
                1_100,
            )
        };

        let duplicate_methods = sign(serde_json::json!({
            "iss": "https://cardea.example",
            "sub": "draven",
            "aud": "cowboy-production",
            "nonce": "a".repeat(64),
            "iat": 1_000,
            "exp": 1_300,
            "amr": ["pwd", "pwd"],
        }));
        assert!(verify(&duplicate_methods).is_none());

        let oversized_context = sign(serde_json::json!({
            "iss": "https://cardea.example",
            "sub": "draven",
            "aud": "cowboy-production",
            "nonce": "a".repeat(64),
            "iat": 1_000,
            "exp": 1_300,
            "acr": "x".repeat(super::MAX_AUTHENTICATION_CONTEXT_BYTES + 1),
        }));
        assert!(verify(&oversized_context).is_none());

        let too_many_methods = sign(serde_json::json!({
            "iss": "https://cardea.example",
            "sub": "draven",
            "aud": "cowboy-production",
            "nonce": "a".repeat(64),
            "iat": 1_000,
            "exp": 1_300,
            "amr": (0..=super::MAX_AUTHENTICATION_METHODS)
                .map(|index| format!("method-{index}"))
                .collect::<Vec<_>>(),
        }));
        assert!(verify(&too_many_methods).is_none());
    }

    #[test]
    fn jwks_selection_rejects_duplicate_or_non_signing_keys() {
        let valid: super::JsonWebKeySet = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "kid": "key-1",
                "alg": "RS256",
                "use": "sig",
                "key_ops": ["verify"],
                "n": "AQAB",
                "e": "AQAB"
            }]
        }))
        .unwrap();
        assert!(valid.signing_key("key-1", "RS256").is_some());

        let duplicate: super::JsonWebKeySet = serde_json::from_value(serde_json::json!({
            "keys": [
                {"kty": "RSA", "kid": "key-1", "alg": "RS256"},
                {"kty": "RSA", "kid": "key-1", "alg": "RS256"}
            ]
        }))
        .unwrap();
        assert!(duplicate.signing_key("key-1", "RS256").is_none());

        let encryption: super::JsonWebKeySet = serde_json::from_value(serde_json::json!({
            "keys": [{"kty": "RSA", "kid": "key-1", "use": "enc"}]
        }))
        .unwrap();
        assert!(encryption.signing_key("key-1", "RS256").is_none());
    }
}
