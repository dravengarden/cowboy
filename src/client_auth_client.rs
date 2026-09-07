//! Durable browser-authorized credentials for non-browser Cowboy clients.
//!
//! A client keeps a local Ed25519 key and rotating refresh token in a private
//! file. The server receives only the public key and hashes of refresh tokens.
//! Every access request is sender-constrained by a fresh signed proof.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use futures::{SinkExt as _, StreamExt as _};
use reqwest::{Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

const CREDENTIAL_VERSION: u32 = 1;
const ACCESS_EXPIRY_MARGIN_MS: i64 = 30_000;
const LOCK_RETRY: Duration = Duration::from_millis(100);
const LOCK_TIMEOUT: Duration = Duration::from_secs(6 * 60);
const MAX_CREDENTIAL_BYTES: u64 = 64 * 1_024;

#[derive(Clone)]
pub(crate) enum ClientAuthentication {
    Legacy(Arc<str>),
    Device(Arc<DeviceCredentialManager>),
}

#[derive(Debug, Clone)]
pub(crate) struct ClientRequestAuthorization {
    pub bearer: Option<String>,
    pub proof_headers: Vec<(String, String)>,
}

impl ClientRequestAuthorization {
    pub fn apply_reqwest(
        &self,
        mut request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder> {
        if let Some(bearer) = &self.bearer {
            request = request.bearer_auth(bearer);
        }
        for (name, value) in &self.proof_headers {
            request = request.header(name, value);
        }
        Ok(request)
    }
}

impl ClientAuthentication {
    pub(crate) fn new(
        base_url: Url,
        legacy_token: Option<&str>,
        state_dir: Option<PathBuf>,
        device_name: Option<String>,
    ) -> Result<Self> {
        if let Some(token) = legacy_token {
            let token = token.trim();
            ensure!(!token.is_empty(), "legacy user token cannot be empty");
            ensure!(
                token.starts_with(crate::product_auth::API_TOKEN_SECRET_PREFIX),
                "legacy user token must start with {}",
                crate::product_auth::API_TOKEN_SECRET_PREFIX
            );
            return Ok(Self::Legacy(Arc::from(token)));
        }
        Ok(Self::Device(Arc::new(DeviceCredentialManager::new(
            base_url,
            state_dir,
            device_name,
        )?)))
    }

    pub(crate) async fn authorize(
        &self,
        method: &Method,
        path_and_query: &str,
        rejected_access_token: Option<&str>,
    ) -> Result<ClientRequestAuthorization> {
        match self {
            Self::Legacy(token) => Ok(ClientRequestAuthorization {
                bearer: Some(token.to_string()),
                proof_headers: Vec::new(),
            }),
            Self::Device(manager) => {
                manager
                    .authorize(method, path_and_query, rejected_access_token)
                    .await
            }
        }
    }

    pub(crate) const fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy(_))
    }

    pub(crate) async fn ensure_login(&self) -> Result<()> {
        match self {
            Self::Legacy(_) => Ok(()),
            Self::Device(manager) => manager.ensure_login().await,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDeviceCredential {
    version: u32,
    origin: String,
    name: String,
    device_id: String,
    private_key: String,
    access_token: String,
    access_expires_at_ms: i64,
    refresh_token: String,
    refresh_expires_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct DeviceCredentialManager {
    base_url: Url,
    device_name: String,
    credential_path: PathBuf,
    lock_path: PathBuf,
    http: reqwest::Client,
    cached: RwLock<Option<StoredDeviceCredential>>,
    local_mode: AtomicBool,
}

impl DeviceCredentialManager {
    fn new(base_url: Url, state_dir: Option<PathBuf>, device_name: Option<String>) -> Result<Self> {
        validate_base_url(&base_url)?;
        let state_dir = state_dir.map_or_else(default_auth_state_dir, Ok)?;
        prepare_private_directory(&state_dir)?;
        let key = crate::admin::hex_sha256(base_url.as_str().as_bytes());
        let credential_path = state_dir.join(format!("{}.json", &key[..24]));
        let lock_path = state_dir.join(format!("{}.lock", &key[..24]));
        let device_name = device_name.unwrap_or_else(default_device_name);
        let device_name = normalize_local_name(&device_name)?;
        Ok(Self {
            base_url,
            device_name,
            credential_path,
            lock_path,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .context("building Cowboy device-auth HTTP client")?,
            cached: RwLock::new(None),
            local_mode: AtomicBool::new(false),
        })
    }

    async fn ensure_login(&self) -> Result<()> {
        let url = self.base_url.join("api/auth/me")?;
        let mut rejected_access = None;
        for attempt in 0..2 {
            let authorization = self
                .authorize(&Method::GET, url.path(), rejected_access.as_deref())
                .await?;
            if authorization.bearer.is_none() {
                // The live policy check explicitly selected local auth-off mode.
                return Ok(());
            }
            let response = authorization
                .apply_reqwest(self.http.get(url.clone()))?
                .header(reqwest::header::CACHE_CONTROL, "no-cache")
                .send()
                .await
                .context("validating Cowboy device login")?;
            if response.status() == StatusCode::UNAUTHORIZED && attempt == 0 {
                // Access grants are ephemeral across Controller restarts. Reuse
                // the owned rotating-refresh path, including its process lock.
                rejected_access = authorization.bearer;
                continue;
            }
            ensure!(
                response.status().is_success(),
                "Cowboy device login validation failed with {}",
                response.status()
            );
            return Ok(());
        }
        unreachable!("the second login validation always returns")
    }

    async fn authorize(
        &self,
        method: &Method,
        path_and_query: &str,
        rejected_access_token: Option<&str>,
    ) -> Result<ClientRequestAuthorization> {
        ensure!(
            path_and_query.starts_with('/') && !path_and_query.contains(['\r', '\n']),
            "request path is invalid"
        );
        let CredentialOutcome::Credential(credential) =
            self.ensure_credential(rejected_access_token).await?
        else {
            return Ok(ClientRequestAuthorization {
                bearer: None,
                proof_headers: Vec::new(),
            });
        };
        let signing_key = crate::client_auth::signing_key_from_base64(&credential.private_key)?;
        let proof_headers = crate::client_auth::signed_proof_headers(
            &signing_key,
            &credential.device_id,
            &credential.access_token,
            method.as_str(),
            path_and_query,
            now_ms(),
        )?;
        Ok(ClientRequestAuthorization {
            bearer: Some(credential.access_token),
            proof_headers,
        })
    }

    async fn ensure_credential(
        &self,
        rejected_access_token: Option<&str>,
    ) -> Result<CredentialOutcome> {
        if self.local_mode.load(Ordering::Acquire) {
            if rejected_access_token.is_none() {
                return Ok(CredentialOutcome::Local);
            }
            self.local_mode.store(false, Ordering::Release);
        }
        if let Some(cached) = self.cached.read().await.clone()
            && credential_access_is_usable(&cached, rejected_access_token)
        {
            return Ok(CredentialOutcome::Credential(cached));
        }

        let _lock = acquire_credential_lock(&self.lock_path).await?;
        let stored = load_credential(&self.credential_path)?;
        if let Some(stored) = stored.as_ref()
            && stored.origin == self.base_url.as_str()
            && credential_access_is_usable(stored, rejected_access_token)
        {
            *self.cached.write().await = Some(stored.clone());
            return Ok(CredentialOutcome::Credential(stored.clone()));
        }

        if self.product_auth_disabled().await? {
            self.local_mode.store(true, Ordering::Release);
            *self.cached.write().await = None;
            return Ok(CredentialOutcome::Local);
        }

        let credential = if let Some(stored) = stored
            && stored.version == CREDENTIAL_VERSION
            && stored.origin == self.base_url.as_str()
            && stored.refresh_expires_at_ms > now_ms()
        {
            match self.refresh(&stored).await? {
                RefreshOutcome::Credential(credential) => credential,
                RefreshOutcome::Reauthorize => self.authorize_in_browser().await?,
            }
        } else {
            self.authorize_in_browser().await?
        };
        save_credential(&self.credential_path, &credential)?;
        *self.cached.write().await = Some(credential.clone());
        Ok(CredentialOutcome::Credential(credential))
    }

    async fn product_auth_disabled(&self) -> Result<bool> {
        let response = self
            .http
            .get(self.base_url.join("api/auth/status")?)
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .send()
            .await
            .context("checking Cowboy login policy")?;
        ensure!(
            response.status().is_success(),
            "Cowboy login policy check failed with {}",
            response.status()
        );
        let status = response
            .json::<serde_json::Value>()
            .await
            .context("decoding Cowboy login policy")?;
        Ok(status.pointer("/me/auth_enabled") == Some(&serde_json::Value::Bool(false)))
    }

    async fn refresh(&self, stored: &StoredDeviceCredential) -> Result<RefreshOutcome> {
        let url = self.base_url.join("api/auth/device/refresh")?;
        let key = crate::client_auth::signing_key_from_base64(&stored.private_key)?;
        let proof = crate::client_auth::signed_proof_headers(
            &key,
            &stored.device_id,
            &stored.refresh_token,
            Method::POST.as_str(),
            url.path(),
            now_ms(),
        )?;
        let mut request = self.http.post(url).bearer_auth(&stored.refresh_token);
        for (name, value) in proof {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .context("refreshing Cowboy device login")?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::GONE
        ) {
            return Ok(RefreshOutcome::Reauthorize);
        }
        ensure!(
            response.status().is_success(),
            "Cowboy device refresh failed with {}",
            response.status()
        );
        let tokens = response
            .json::<crate::client_auth::DeviceTokenResponse>()
            .await
            .context("decoding Cowboy device refresh")?;
        ensure!(
            tokens.device_id == stored.device_id,
            "Cowboy refreshed the wrong device"
        );
        Ok(RefreshOutcome::Credential(StoredDeviceCredential {
            version: CREDENTIAL_VERSION,
            origin: stored.origin.clone(),
            name: stored.name.clone(),
            device_id: tokens.device_id,
            private_key: stored.private_key.clone(),
            access_token: tokens.access_token,
            access_expires_at_ms: tokens.access_expires_at_ms,
            refresh_token: tokens.refresh_token,
            refresh_expires_at_ms: tokens.refresh_expires_at_ms,
        }))
    }

    async fn authorize_in_browser(&self) -> Result<StoredDeviceCredential> {
        let signing_key = crate::client_auth::new_signing_key()?;
        let code_verifier = crate::client_auth::new_code_verifier()?;
        let response = self
            .http
            .post(self.base_url.join("api/auth/device/authorizations")?)
            .json(&crate::client_auth::StartAuthorizationRequest {
                name: self.device_name.clone(),
                public_key: crate::client_auth::public_key_to_base64(&signing_key),
                code_challenge: crate::client_auth::code_challenge(&code_verifier)?,
            })
            .send()
            .await
            .context("starting Cowboy device authorization")?;
        ensure!(
            response.status().is_success(),
            "Cowboy device authorization failed with {}",
            response.status()
        );
        let started = response
            .json::<crate::client_auth::StartAuthorizationResponse>()
            .await
            .context("decoding Cowboy device authorization")?;
        let verification_url = resolve_verification_url(&self.base_url, &started.verification_url)?;
        eprintln!(
            "Cowboy needs your approval. Opening the sign-in page for {}.",
            self.base_url.origin().ascii_serialization()
        );
        eprintln!(
            "Verify this client fingerprint before approving: {}",
            crate::client_auth::device_fingerprint(signing_key.verifying_key().as_bytes())
        );
        if let Err(error) = open_browser(&verification_url) {
            eprintln!("Could not open a browser automatically: {error}");
            eprintln!("Open this URL to continue: {verification_url}");
        }
        self.wait_for_approval(&started.request_id, &code_verifier, started.expires_at_ms)
            .await?;
        let exchange = crate::client_auth::signed_exchange_request(
            &signing_key,
            started.request_id,
            code_verifier,
            now_ms(),
        )?;
        let response = self
            .http
            .post(self.base_url.join("api/auth/device/exchange")?)
            .json(&exchange)
            .send()
            .await
            .context("completing Cowboy device authorization")?;
        ensure!(
            response.status().is_success(),
            "Cowboy device authorization exchange failed with {}",
            response.status()
        );
        let tokens = response
            .json::<crate::client_auth::DeviceTokenResponse>()
            .await
            .context("decoding Cowboy device credentials")?;
        Ok(StoredDeviceCredential {
            version: CREDENTIAL_VERSION,
            origin: self.base_url.to_string(),
            name: self.device_name.clone(),
            device_id: tokens.device_id,
            private_key: crate::client_auth::signing_key_to_base64(&signing_key),
            access_token: tokens.access_token,
            access_expires_at_ms: tokens.access_expires_at_ms,
            refresh_token: tokens.refresh_token,
            refresh_expires_at_ms: tokens.refresh_expires_at_ms,
        })
    }

    async fn wait_for_approval(
        &self,
        request_id: &str,
        code_verifier: &str,
        expires_at_ms: i64,
    ) -> Result<()> {
        let mut url = self
            .base_url
            .join("api/auth/device/authorizations/events")?;
        let scheme = if self.base_url.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        url.set_scheme(scheme)
            .map_err(|()| anyhow!("could not build Cowboy authorization WebSocket URL"))?;
        let (mut socket, _) = connect_async(url.as_str())
            .await
            .context("connecting Cowboy authorization notifications")?;
        socket
            .send(Message::Text(
                serde_json::to_string(&crate::client_auth::AuthorizationEventsHandshake {
                    request_id: request_id.to_owned(),
                    code_verifier: code_verifier.to_owned(),
                })?
                .into(),
            ))
            .await
            .context("sending Cowboy authorization handshake")?;
        let wait = async {
            while let Some(message) = socket.next().await {
                match message.context("reading Cowboy authorization notification")? {
                    Message::Text(text) => {
                        let status = serde_json::from_str::<
                            crate::client_auth::AuthorizationEventStatus,
                        >(text.as_str())?;
                        match status.status.as_str() {
                            "approved" => return Ok(()),
                            "denied" => bail!("Cowboy device authorization was denied"),
                            "unavailable" => bail!("Cowboy device authorization expired"),
                            _ => {}
                        }
                    }
                    Message::Ping(payload) => socket.send(Message::Pong(payload)).await?,
                    Message::Close(_) => break,
                    Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
                }
            }
            bail!("Cowboy device authorization closed before approval")
        };
        let remaining_ms = expires_at_ms
            .saturating_sub(now_ms())
            .clamp(1_000, 6 * 60 * 1_000);
        tokio::time::timeout(
            Duration::from_millis(u64::try_from(remaining_ms).unwrap_or(1_000)),
            wait,
        )
        .await
        .context("Cowboy device authorization timed out")??;
        Ok(())
    }
}

enum RefreshOutcome {
    Credential(StoredDeviceCredential),
    Reauthorize,
}

enum CredentialOutcome {
    Local,
    Credential(StoredDeviceCredential),
}

fn credential_access_is_usable(
    credential: &StoredDeviceCredential,
    rejected_access_token: Option<&str>,
) -> bool {
    credential.version == CREDENTIAL_VERSION
        && credential.access_expires_at_ms > now_ms().saturating_add(ACCESS_EXPIRY_MARGIN_MS)
        && rejected_access_token != Some(credential.access_token.as_str())
}

pub(crate) fn normalized_base_url(input: &str) -> Result<Url> {
    let mut url = Url::parse(input).with_context(|| format!("invalid Cowboy URL {input:?}"))?;
    validate_base_url(&url)?;
    url.set_query(None);
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

fn validate_base_url(url: &Url) -> Result<()> {
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "Cowboy URL must use HTTP or HTTPS"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Cowboy URL cannot contain credentials"
    );
    let loopback = url_is_loopback(url);
    ensure!(
        url.scheme() == "https" || loopback,
        "remote Cowboy login requires HTTPS"
    );
    Ok(())
}

fn url_is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

fn resolve_verification_url(base_url: &Url, advertised: &str) -> Result<Url> {
    let verification_url = base_url
        .join(advertised)
        .context("resolving Cowboy authorization URL")?;
    ensure!(
        verification_url.username().is_empty() && verification_url.password().is_none(),
        "Cowboy authorization URL cannot contain credentials"
    );
    let same_origin = verification_url.origin() == base_url.origin();
    ensure!(
        same_origin || (url_is_loopback(base_url) && verification_url.scheme() == "https"),
        "Cowboy returned an untrusted authorization origin"
    );
    ensure!(
        verification_url.path().ends_with("/auth/device")
            && verification_url.query().is_none()
            && verification_url.fragment().is_some(),
        "Cowboy returned an invalid authorization URL"
    );
    Ok(verification_url)
}

fn default_auth_state_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("cowboy/client-auth"));
    }
    let home = std::env::var_os("HOME").context("HOME is required for Cowboy client login")?;
    Ok(PathBuf::from(home).join(".config/cowboy/client-auth"))
}

fn default_device_name() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "this computer".to_owned());
    format!("Cowboy CLI on {host}")
}

fn normalize_local_name(name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(!name.is_empty(), "device name cannot be empty");
    ensure!(
        name.chars().count() <= 64 && !name.chars().any(char::is_control),
        "device name is invalid"
    );
    Ok(name.to_owned())
}

fn prepare_private_directory(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("creating Cowboy credential directory {}", path.display()))?;
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspecting Cowboy credential directory {}", path.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Cowboy credential directory must be a real directory"
    );
    #[cfg(unix)]
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("protecting Cowboy credential directory {}", path.display()))?;
    Ok(())
}

fn private_open_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
}

fn open_lock_file(path: &Path) -> Result<File> {
    let file = private_open_options()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .with_context(|| format!("opening Cowboy credential lock {}", path.display()))?;
    assert_private_file(&file, path)?;
    Ok(file)
}

async fn acquire_credential_lock(path: &Path) -> Result<CredentialLock> {
    let file = open_lock_file(path)?;
    let deadline = tokio::time::Instant::now() + LOCK_TIMEOUT;
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(CredentialLock(file)),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "timed out waiting for another Cowboy login"
                );
                tokio::time::sleep(LOCK_RETRY).await;
            }
            Err(error) => return Err(error).context("locking Cowboy credentials"),
        }
    }
}

struct CredentialLock(File);

impl Drop for CredentialLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn load_credential(path: &Path) -> Result<Option<StoredDeviceCredential>> {
    let mut file = match private_open_options().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("opening Cowboy credentials {}", path.display()));
        }
    };
    assert_private_file(&file, path)?;
    ensure!(
        file.metadata()?.len() <= MAX_CREDENTIAL_BYTES,
        "Cowboy credential file is too large"
    );
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    serde_json::from_str(&contents)
        .with_context(|| format!("decoding Cowboy credentials {}", path.display()))
        .map(Some)
}

fn save_credential(path: &Path, credential: &StoredDeviceCredential) -> Result<()> {
    let parent = path
        .parent()
        .context("Cowboy credential path has no parent")?;
    prepare_private_directory(parent)?;
    let random_suffix = crate::client_auth::new_code_verifier()?;
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        &random_suffix[..16]
    ));
    let mut file = private_open_options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("creating Cowboy credential file {}", temporary.display()))?;
    let bytes = serde_json::to_vec_pretty(credential)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("installing Cowboy credentials {}", path.display()))?;
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn assert_private_file(file: &File, path: &Path) -> Result<()> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "Cowboy credential path {} is not a file",
        path.display()
    );
    #[cfg(unix)]
    ensure!(
        metadata.permissions().mode() & 0o077 == 0,
        "Cowboy credential file {} must not be accessible by group or others",
        path.display()
    );
    Ok(())
}

fn open_browser(url: &Url) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url.as_str());
    command.spawn().context("starting the system browser")?;
    Ok(())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    struct LoginFixture {
        manager: Arc<DeviceCredentialManager>,
        requests: Arc<std::sync::Mutex<Vec<String>>>,
        server: tokio::task::JoinHandle<()>,
        state_dir: PathBuf,
    }

    impl Drop for LoginFixture {
        fn drop(&mut self) {
            self.server.abort();
            let _ = std::fs::remove_dir_all(&self.state_dir);
        }
    }

    fn assert_fixture_proof(headers: &axum::http::HeaderMap, method: &str, path: &str) {
        let token = headers["authorization"]
            .to_str()
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap();
        let timestamp = headers[crate::client_auth::PROOF_TIME_HEADER]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        let nonce = headers[crate::client_auth::PROOF_NONCE_HEADER]
            .to_str()
            .unwrap();
        let proof = crate::client_auth::request_proof(method, path, token, timestamp, nonce);
        let public = ed25519_dalek::SigningKey::from_bytes(&[7; 32])
            .verifying_key()
            .to_bytes();
        crate::client_auth::verify_signature(
            &public,
            &proof,
            headers[crate::client_auth::PROOF_SIGNATURE_HEADER]
                .to_str()
                .unwrap(),
        )
        .unwrap();
    }

    async fn login_fixture(stale: bool, validation: StatusCode) -> LoginFixture {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/base/", listener.local_addr().unwrap());
        let state_dir = std::env::temp_dir().join(format!(
            "cowboy-login-validation-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        let manager = Arc::new(
            DeviceCredentialManager::new(
                normalized_base_url(&origin).unwrap(),
                Some(state_dir.clone()),
                Some("Fixture client".to_owned()),
            )
            .unwrap(),
        );
        save_credential(
            &manager.credential_path,
            &StoredDeviceCredential {
                version: CREDENTIAL_VERSION,
                origin,
                name: "Fixture client".to_owned(),
                device_id: "0".repeat(32),
                private_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7; 32]),
                access_token: if stale { "stale" } else { "current" }.to_owned(),
                access_expires_at_ms: now_ms() + 120_000,
                refresh_token: "refresh-fixture".to_owned(),
                refresh_expires_at_ms: now_ms() + 240_000,
            },
        )
        .unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let me_requests = requests.clone();
        let refresh_requests = requests.clone();
        let app = axum::Router::new()
            .route(
                "/base/api/auth/me",
                axum::routing::get(move |headers: axum::http::HeaderMap| async move {
                    assert_fixture_proof(&headers, "GET", "/base/api/auth/me");
                    me_requests.lock().unwrap().push("me".to_owned());
                    if headers["authorization"] == "Bearer stale" {
                        StatusCode::UNAUTHORIZED
                    } else {
                        validation
                    }
                }),
            )
            .route(
                "/base/api/auth/status",
                axum::routing::get(|| async {
                    axum::Json(serde_json::json!({"me": {"auth_enabled": true}}))
                }),
            )
            .route(
                "/base/api/auth/device/refresh",
                axum::routing::post(move |headers: axum::http::HeaderMap| async move {
                    assert_fixture_proof(&headers, "POST", "/base/api/auth/device/refresh");
                    assert_eq!(headers["authorization"], "Bearer refresh-fixture");
                    refresh_requests.lock().unwrap().push("refresh".to_owned());
                    axum::Json(crate::client_auth::DeviceTokenResponse {
                        device_id: "0".repeat(32),
                        access_token: "current".to_owned(),
                        access_expires_at_ms: now_ms() + 120_000,
                        refresh_token: "rotated-fixture".to_owned(),
                        refresh_expires_at_ms: now_ms() + 240_000,
                    })
                }),
            );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        LoginFixture {
            manager,
            requests,
            server,
            state_dir,
        }
    }

    #[tokio::test]
    async fn login_checks_cached_access_and_recovers_controller_restart() {
        let fixture = login_fixture(true, StatusCode::OK).await;
        let authentication = ClientAuthentication::Device(fixture.manager.clone());
        authentication.ensure_login().await.unwrap();
        assert_eq!(*fixture.requests.lock().unwrap(), ["me", "refresh", "me"]);
        let stored = load_credential(&fixture.manager.credential_path)
            .unwrap()
            .unwrap();
        assert_eq!(stored.access_token, "current");
        assert_eq!(stored.refresh_token, "rotated-fixture");
        authentication.ensure_login().await.unwrap();
        assert_eq!(
            *fixture.requests.lock().unwrap(),
            ["me", "refresh", "me", "me"]
        );
    }

    #[tokio::test]
    async fn login_never_claims_success_after_validation_failure_or_retries_forever() {
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let fixture = login_fixture(true, status).await;
            let authentication = ClientAuthentication::Device(fixture.manager.clone());
            assert!(authentication.ensure_login().await.is_err());
            assert_eq!(*fixture.requests.lock().unwrap(), ["me", "refresh", "me"]);
        }
        for status in [StatusCode::FORBIDDEN, StatusCode::SERVICE_UNAVAILABLE] {
            let fixture = login_fixture(false, status).await;
            let authentication = ClientAuthentication::Device(fixture.manager.clone());
            assert!(authentication.ensure_login().await.is_err());
            assert_eq!(*fixture.requests.lock().unwrap(), ["me"]);
        }
    }

    #[test]
    fn remote_login_requires_https() {
        assert!(normalized_base_url("http://cowboy.example").is_err());
        assert!(normalized_base_url("http://127.0.0.1:3333").is_ok());
        assert!(normalized_base_url("https://cowboy.example/base").is_ok());
    }

    #[test]
    fn loopback_api_accepts_only_a_public_https_authorization_page() {
        let loopback = normalized_base_url("http://127.0.0.1:3333").unwrap();
        let public = resolve_verification_url(
            &loopback,
            "https://cowboy.example/auth/device#request_id=test&approval_token=secret",
        )
        .unwrap();
        assert_eq!(
            public.origin().ascii_serialization(),
            "https://cowboy.example"
        );
        assert!(
            resolve_verification_url(
                &loopback,
                "http://cowboy.example/auth/device#request_id=test&approval_token=secret",
            )
            .is_err()
        );

        let remote = normalized_base_url("https://cowboy.example").unwrap();
        assert!(
            resolve_verification_url(
                &remote,
                "https://other.example/auth/device#request_id=test&approval_token=secret",
            )
            .is_err()
        );
    }

    #[test]
    fn rejected_access_token_forces_refresh() {
        let credential = StoredDeviceCredential {
            version: CREDENTIAL_VERSION,
            origin: "https://cowboy.example/".to_owned(),
            name: "test".to_owned(),
            device_id: "0".repeat(32),
            private_key: "private".to_owned(),
            access_token: "cow_access_example".to_owned(),
            access_expires_at_ms: now_ms() + 60_000,
            refresh_token: "cow_refresh_example".to_owned(),
            refresh_expires_at_ms: now_ms() + 120_000,
        };
        assert!(credential_access_is_usable(&credential, None));
        assert!(!credential_access_is_usable(
            &credential,
            Some("cow_access_example")
        ));
    }

    #[tokio::test]
    async fn explicit_auth_off_uses_local_access_without_creating_a_credential() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route(
                    "/api/auth/status",
                    axum::routing::get(|| async {
                        axum::Json(serde_json::json!({
                            "me": {
                                "account": "local",
                                "role": "owner",
                                "auth_enabled": false,
                            }
                        }))
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let state_dir = std::env::temp_dir().join(format!(
            "cowboy-client-auth-off-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let authentication = ClientAuthentication::new(
            normalized_base_url(&format!("http://{address}")).unwrap(),
            None,
            Some(state_dir.clone()),
            Some("Test client".to_owned()),
        )
        .unwrap();
        let authorization = authentication
            .authorize(&Method::GET, "/ws", None)
            .await
            .unwrap();
        assert!(authorization.bearer.is_none());
        assert!(authorization.proof_headers.is_empty());
        authentication.ensure_login().await.unwrap();
        assert_eq!(
            std::fs::read_dir(&state_dir).unwrap().count(),
            1,
            "only the process lock should exist in auth-off mode"
        );

        server.abort();
        let _ = std::fs::remove_dir_all(state_dir);
    }
}
