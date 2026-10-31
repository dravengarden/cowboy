use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use rand::rngs::OsRng;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::{Deserialize, Serialize};
use web_push_native::{Auth, WebPushBuilder, p256::PublicKey};

use crate::task_notifications::Category;

const VAPID_KEY_FILE: &str = "web-push-vapid.key";
const VAPID_SUBJECT: &str = "mailto:dravengarden@gmail.com";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationCategories {
    pub completed: bool,
    pub input: bool,
    pub permission: bool,
    pub error: bool,
}

/// Preferences an enrollment stores. `schema` 2 is the first with an explicit
/// completion opt-in (docs/task-notifications.md#preferences).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebPushPreferences {
    #[serde(default)]
    pub schema: u32,
    pub show_session_names: bool,
    pub categories: NotificationCategories,
    #[serde(default)]
    pub muted_session_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebPushSubscription {
    pub endpoint: String,
    pub keys: SubscriptionKeys,
    pub preferences: WebPushPreferences,
}

/// What one push asks of the push service (RFC 8030).
#[derive(Debug, Clone)]
pub struct PushRequest<'a> {
    pub payload: &'a [u8],
    pub urgent: bool,
    pub ttl: Duration,
    /// Replaces an undelivered message with the same topic.
    pub topic: &'a str,
}

/// The push service's answer. Success means it accepted the message, never
/// that a device presented it.
#[derive(Debug, Clone, Copy)]
pub struct PushResponse {
    pub status: reqwest::StatusCode,
    pub retry_after: Option<Duration>,
}

pub struct WebPushService {
    signing_key: SigningKey,
    public_key: String,
    client: reqwest::Client,
}

impl WebPushService {
    pub fn open(data_dir: &Path) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(data_dir).context("creating Cowboy data directory")?;
        let key_path = data_dir.join(VAPID_KEY_FILE);
        let signing_key = load_or_create_key(&key_path)?;
        let public_key = URL_SAFE_NO_PAD.encode(
            signing_key
                .verifying_key()
                .to_encoded_point(false)
                .as_bytes(),
        );
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .context("building Web Push HTTP client")?;
        Ok(Arc::new(Self {
            signing_key,
            public_key,
            client,
        }))
    }

    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    pub async fn send(
        &self,
        endpoint: &str,
        keys: &SubscriptionKeys,
        push: &PushRequest<'_>,
    ) -> anyhow::Result<PushResponse> {
        let p256dh = PublicKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(&keys.p256dh)?)
            .context("decoding Web Push client public key")?;
        let auth_bytes = URL_SAFE_NO_PAD.decode(&keys.auth)?;
        if auth_bytes.len() != 16 {
            return Err(anyhow!("invalid Web Push auth secret length"));
        }
        let auth = Auth::clone_from_slice(&auth_bytes);
        let mut request = WebPushBuilder::new(endpoint.parse()?, p256dh, auth)
            .with_valid_duration(push.ttl)
            .build(push.payload)?;
        let headers = request.headers_mut();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&vapid_authorization(endpoint, &self.signing_key)?)?,
        );
        headers.insert(
            "urgency",
            HeaderValue::from_static(if push.urgent { "high" } else { "normal" }),
        );
        headers.insert("topic", HeaderValue::from_str(push.topic)?);
        let (parts, body) = request.into_parts();
        let response = self
            .client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body)
            .send()
            .await?;
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs);
        Ok(PushResponse {
            status: response.status(),
            retry_after,
        })
    }
}

/// The storage key of an endpoint; endpoints are capabilities and stay out of
/// delivery rows and logs.
#[must_use]
pub fn endpoint_sha256(endpoint: &str) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(endpoint.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// RFC 8030 topics are at most 32 URL-safe characters.
#[must_use]
pub fn push_topic(identity: &str) -> String {
    use sha2::Digest as _;
    URL_SAFE_NO_PAD.encode(&sha2::Sha256::digest(identity.as_bytes())[..24])
}

pub fn safe_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

impl WebPushPreferences {
    /// Map an unversioned value: only permission requests were ever delivered
    /// as input, and `completed` was a default, not an opt-in.
    #[must_use]
    pub fn migrated(mut self) -> Self {
        if self.schema < 2 {
            self.categories.input = self.categories.permission;
            self.categories.completed = false;
            self.schema = 2;
        }
        self
    }

    #[must_use]
    pub fn wants(&self, category: Category, session_id: &str) -> bool {
        let enabled = match category {
            Category::Input => self.categories.input,
            Category::Failure => self.categories.error,
            Category::Completion => self.categories.completed,
        };
        enabled
            && !self
                .muted_session_ids
                .iter()
                .any(|muted| muted == session_id)
    }
}

pub fn validate_subscription(subscription: &WebPushSubscription) -> anyhow::Result<()> {
    let endpoint = reqwest::Url::parse(&subscription.endpoint)?;
    let host = endpoint.host_str().unwrap_or_default();
    if endpoint.scheme() != "https"
        || !matches!(
            host,
            "web.push.apple.com" | "fcm.googleapis.com" | "updates.push.services.mozilla.com"
        )
    {
        return Err(anyhow!("unsupported Web Push endpoint"));
    }
    if subscription.endpoint.len() > 2_048
        || URL_SAFE_NO_PAD.decode(&subscription.keys.p256dh)?.len() != 65
        || URL_SAFE_NO_PAD.decode(&subscription.keys.auth)?.len() != 16
        || subscription.preferences.muted_session_ids.len() > 500
        || subscription
            .preferences
            .muted_session_ids
            .iter()
            .any(|session_id| !safe_session_id(session_id))
    {
        return Err(anyhow!("invalid Web Push subscription"));
    }
    Ok(())
}

fn load_or_create_key(path: &Path) -> anyhow::Result<SigningKey> {
    if let Ok(encoded) = std::fs::read_to_string(path) {
        let bytes = URL_SAFE_NO_PAD.decode(encoded.trim())?;
        return SigningKey::from_slice(&bytes).context("reading persisted VAPID key");
    }
    let signing_key = SigningKey::random(&mut OsRng);
    atomic_write_private(
        path,
        URL_SAFE_NO_PAD.encode(signing_key.to_bytes()).as_bytes(),
    )?;
    Ok(signing_key)
}

fn vapid_authorization(endpoint: &str, signing_key: &SigningKey) -> anyhow::Result<String> {
    let endpoint = reqwest::Url::parse(endpoint)?;
    let audience = endpoint.origin().ascii_serialization();
    let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&serde_json::json!({
        "aud": audience,
        "exp": chrono::Utc::now().timestamp() + 12 * 60 * 60,
        "sub": VAPID_SUBJECT,
    }))?);
    let unsigned = format!("{header}.{claims}");
    let signature: Signature = signing_key.sign(unsigned.as_bytes());
    let token = format!(
        "{unsigned}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    );
    Ok(format!(
        "vapid t={token}, k={}",
        URL_SAFE_NO_PAD.encode(
            signing_key
                .verifying_key()
                .to_encoded_point(false)
                .as_bytes()
        )
    ))
}

fn atomic_write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::signature::Verifier as _;

    use super::*;

    #[test]
    fn notification_subscription_rejects_ssrf_and_unbounded_mutes() {
        let valid = WebPushSubscription {
            endpoint: "https://web.push.apple.com/Q/example".to_owned(),
            keys: SubscriptionKeys {
                p256dh: URL_SAFE_NO_PAD.encode([4_u8; 65]),
                auth: URL_SAFE_NO_PAD.encode([5_u8; 16]),
            },
            preferences: WebPushPreferences {
                schema: 2,
                show_session_names: false,
                categories: NotificationCategories {
                    completed: true,
                    input: true,
                    permission: true,
                    error: true,
                },
                muted_session_ids: vec!["sess-1".to_owned()],
            },
        };
        assert!(validate_subscription(&valid).is_ok());
        let mut internal = valid.clone();
        internal.endpoint = "https://127.0.0.1/private".to_owned();
        assert!(validate_subscription(&internal).is_err());
    }

    #[test]
    fn unversioned_preferences_never_opt_into_completions() {
        let legacy = |input, permission, completed| WebPushPreferences {
            schema: 0,
            show_session_names: false,
            categories: NotificationCategories {
                completed,
                input,
                permission,
                error: true,
            },
            muted_session_ids: vec!["muted".to_owned()],
        };
        let migrated = legacy(true, false, true).migrated();
        assert_eq!(migrated.schema, 2);
        assert!(!migrated.categories.input, "permission was the only input");
        assert!(!migrated.categories.completed);
        assert!(migrated.wants(Category::Failure, "s"));
        assert!(!migrated.wants(Category::Failure, "muted"));
        let mut chosen = legacy(true, true, false).migrated();
        chosen.categories.completed = true;
        assert!(
            chosen.clone().migrated().categories.completed,
            "schema 2 is kept"
        );
    }

    /// One real request to a local push service: the RFC 8030 headers arrive and
    /// `Retry-After` comes back.
    #[tokio::test]
    async fn a_push_carries_its_delivery_headers() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let root = tempfile::tempdir().unwrap();
        let service = WebPushService::open(root.path()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 8_192];
            let read = socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 429 Too Many\r\nretry-after: 7\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8_lossy(&request[..read]).to_lowercase()
        });
        let client = SigningKey::random(&mut OsRng);
        let keys = SubscriptionKeys {
            p256dh: URL_SAFE_NO_PAD.encode(
                client.verifying_key().to_encoded_point(false).as_bytes(),
            ),
            auth: URL_SAFE_NO_PAD.encode([5_u8; 16]),
        };
        let response = service
            .send(
                &format!("http://{address}/push"),
                &keys,
                &PushRequest {
                    payload: b"{}",
                    urgent: true,
                    ttl: Duration::from_secs(120),
                    topic: "topic-1",
                },
            )
            .await
            .unwrap();
        assert_eq!(response.status.as_u16(), 429);
        assert_eq!(response.retry_after, Some(Duration::from_secs(7)));
        let request = server.await.unwrap();
        assert!(request.contains("urgency: high"), "{request}");
        assert!(request.contains("topic: topic-1"), "{request}");
        assert!(request.contains("ttl: 120"), "{request}");
    }

    #[test]
    fn topics_fit_rfc8030() {
        let topic = push_topic("cowboy-session-with-a-long-name-failure");
        assert!(topic.len() <= 32);
        assert!(
            topic
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
    }

    #[test]
    fn vapid_authorization_has_correct_audience_and_signature() {
        let signing_key = SigningKey::random(&mut OsRng);
        let authorization =
            vapid_authorization("https://web.push.apple.com/Q/private-token", &signing_key)
                .unwrap();
        let value = authorization.strip_prefix("vapid t=").unwrap();
        let (token, encoded_key) = value.split_once(", k=").unwrap();
        let parts = token.split('.').collect::<Vec<_>>();
        assert_eq!(parts.len(), 3);
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://web.push.apple.com");
        assert_eq!(claims["sub"], VAPID_SUBJECT);
        assert!(claims["exp"].as_i64().unwrap() > chrono::Utc::now().timestamp());
        assert_eq!(URL_SAFE_NO_PAD.decode(encoded_key).unwrap().len(), 65);
        let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
        signing_key
            .verifying_key()
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
    }
}
