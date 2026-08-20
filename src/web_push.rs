use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use parking_lot::Mutex;
use rand::rngs::OsRng;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::{Deserialize, Serialize};
use web_push_native::{Auth, WebPushBuilder, p256::PublicKey};

const VAPID_KEY_FILE: &str = "web-push-vapid.key";
const SUBSCRIPTIONS_FILE: &str = "web-push-subscriptions.json";
const VAPID_SUBJECT: &str = "mailto:dravengarden@gmail.com";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationCategories {
    pub completed: bool,
    pub input: bool,
    pub permission: bool,
    pub error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebPushPreferences {
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

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct StoredSubscriptions {
    subscriptions: Vec<WebPushSubscription>,
}

#[derive(Debug, Clone, Copy)]
pub enum NotificationCategory {
    Permission,
    Error,
}

impl NotificationCategory {
    fn enabled(self, categories: &NotificationCategories) -> bool {
        match self {
            Self::Permission => categories.permission,
            Self::Error => categories.error,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Permission => "permission",
            Self::Error => "error",
        }
    }

    fn copy(self) -> (&'static str, &'static str) {
        match self {
            Self::Permission => ("Cowboy needs permission", "Review the pending request."),
            Self::Error => ("Cowboy session error", "Open the session for details."),
        }
    }
}

pub struct WebPushService {
    signing_key: SigningKey,
    public_key: String,
    subscriptions_path: PathBuf,
    subscriptions: Mutex<Vec<WebPushSubscription>>,
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
        let subscriptions_path = data_dir.join(SUBSCRIPTIONS_FILE);
        let subscriptions = load_subscriptions(&subscriptions_path)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .context("building Web Push HTTP client")?;
        Ok(Arc::new(Self {
            signing_key,
            public_key,
            subscriptions_path,
            subscriptions: Mutex::new(subscriptions),
            client,
        }))
    }

    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    pub fn upsert(&self, subscription: WebPushSubscription) -> anyhow::Result<()> {
        validate_subscription(&subscription)?;
        let mut subscriptions = self.subscriptions.lock();
        if let Some(existing) = subscriptions
            .iter_mut()
            .find(|existing| existing.endpoint == subscription.endpoint)
        {
            *existing = subscription;
        } else {
            if subscriptions.len() >= 32 {
                return Err(anyhow!("too many notification subscriptions"));
            }
            subscriptions.push(subscription);
        }
        persist_subscriptions(&self.subscriptions_path, &subscriptions)
    }

    pub fn remove(&self, endpoint: &str) -> anyhow::Result<()> {
        let mut subscriptions = self.subscriptions.lock();
        subscriptions.retain(|subscription| subscription.endpoint != endpoint);
        persist_subscriptions(&self.subscriptions_path, &subscriptions)
    }

    pub async fn notify(
        &self,
        category: NotificationCategory,
        session_id: &str,
        session_title: &str,
    ) {
        if !safe_session_id(session_id) {
            return;
        }
        let subscriptions = self.subscriptions.lock().clone();
        let mut expired = Vec::new();
        for subscription in subscriptions {
            if !category.enabled(&subscription.preferences.categories)
                || subscription
                    .preferences
                    .muted_session_ids
                    .iter()
                    .any(|muted| muted == session_id)
            {
                continue;
            }
            let (title, generic_body) = category.copy();
            let body = if subscription.preferences.show_session_names {
                format!(
                    "{} — {generic_body}",
                    session_title.chars().take(120).collect::<String>()
                )
            } else {
                generic_body.to_owned()
            };
            let payload = serde_json::json!({
                "version": 1,
                "category": category.name(),
                "sessionId": session_id,
                "title": title,
                "body": body,
            });
            match self
                .send(&subscription, payload.to_string().as_bytes())
                .await
            {
                Ok(status)
                    if status == reqwest::StatusCode::GONE
                        || status == reqwest::StatusCode::NOT_FOUND =>
                {
                    expired.push(subscription.endpoint);
                }
                Ok(status) if status.is_success() => {}
                Ok(status) => tracing::warn!(%status, "Web Push service rejected notification"),
                Err(error) => tracing::warn!(%error, "Web Push delivery failed"),
            }
        }
        if !expired.is_empty() {
            let expired: HashSet<_> = expired.into_iter().collect();
            let mut subscriptions = self.subscriptions.lock();
            subscriptions.retain(|subscription| !expired.contains(&subscription.endpoint));
            if let Err(error) = persist_subscriptions(&self.subscriptions_path, &subscriptions) {
                tracing::warn!(%error, "persisting expired Web Push subscription cleanup");
            }
        }
    }

    async fn send(
        &self,
        subscription: &WebPushSubscription,
        payload: &[u8],
    ) -> anyhow::Result<reqwest::StatusCode> {
        let p256dh =
            PublicKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(&subscription.keys.p256dh)?)
                .context("decoding Web Push client public key")?;
        let auth_bytes = URL_SAFE_NO_PAD.decode(&subscription.keys.auth)?;
        if auth_bytes.len() != 16 {
            return Err(anyhow!("invalid Web Push auth secret length"));
        }
        let auth = Auth::clone_from_slice(&auth_bytes);
        let mut request =
            WebPushBuilder::new(subscription.endpoint.parse()?, p256dh, auth).build(payload)?;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&vapid_authorization(
                &subscription.endpoint,
                &self.signing_key,
            )?)?,
        );
        let (parts, body) = request.into_parts();
        let response = self
            .client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body)
            .send()
            .await?;
        Ok(response.status())
    }
}

pub fn safe_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn validate_subscription(subscription: &WebPushSubscription) -> anyhow::Result<()> {
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

fn load_subscriptions(path: &Path) -> anyhow::Result<Vec<WebPushSubscription>> {
    let Ok(bytes) = std::fs::read(path) else {
        return Ok(Vec::new());
    };
    let mut stored: StoredSubscriptions =
        serde_json::from_slice(&bytes).context("reading Web Push subscriptions")?;
    stored.subscriptions.retain(|subscription| {
        let valid = validate_subscription(subscription).is_ok();
        if !valid {
            tracing::warn!("ignoring invalid persisted Web Push subscription");
        }
        valid
    });
    Ok(stored.subscriptions)
}

fn persist_subscriptions(path: &Path, subscriptions: &[WebPushSubscription]) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(&StoredSubscriptions {
        subscriptions: subscriptions.to_vec(),
    })?;
    atomic_write_private(path, &bytes)
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
