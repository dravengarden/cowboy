//! Mandatory external HTTPS and device-bound browser credentials.

use super::*;
use crate::browser_device::{BrowserDevices, CHALLENGE_PATH, HEADER, WS_PREFIX};

const COOKIES: [&str; 2] = [
    crate::product_auth::USER_SESSION_COOKIE,
    crate::admin::ADMIN_SESSION_COOKIE,
];

pub(super) struct SecureTransport {
    pub store: Store,
    pub origins: Vec<String>,
    devices: BrowserDevices,
}

impl SecureTransport {
    pub fn new(store: Store, origins: Vec<String>) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !origins.is_empty() && origins.iter().all(|origin| origin.starts_with("https://")),
            "Option 1 requires COWBOY_PUBLIC_ORIGIN with HTTPS origins and a local TLS proxy"
        );
        Ok(Self {
            store,
            origins,
            devices: BrowserDevices::new(),
        })
    }
}

fn proof_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(HEADER)
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            headers
                .get(header::SEC_WEBSOCKET_PROTOCOL)?
                .to_str()
                .ok()?
                .split(',')
                .find_map(|protocol| protocol.trim().strip_prefix(WS_PREFIX))
        })
}

fn reject_proof() -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "code": "device_proof_required",
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert("x-cowboy-device-proof", "required".parse().unwrap());
    response
}

fn remove_cookie(headers: &mut HeaderMap, name: &str) {
    let cookies = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|part| part.split_once('=').is_some_and(|(key, _)| key != name))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>()
        .join("; ");
    headers.remove(header::COOKIE);
    if !cookies.is_empty()
        && let Ok(value) = cookies.parse()
    {
        headers.insert(header::COOKIE, value);
    }
}

/// A primary credential may establish a new binding. An old cookie alone
/// never registers a device, including during an upgrade.
fn primary_login(path: &str) -> bool {
    matches!(
        path,
        "/api/auth/login"
            | "/api/auth/register"
            | "/api/admin/auth/login"
            | "/api/admin/auth/bootstrap"
    ) || path.starts_with("/api/auth/")
        && (path.ends_with("/native/poll") || path.ends_with("/native/exchange"))
}

fn oidc_callback(path: &str) -> bool {
    path == "/api/auth/oidc/callback"
        || (path.starts_with("/api/auth/providers/") && path.ends_with("/callback"))
}

fn trusted_https(peer: SocketAddr, headers: &HeaderMap) -> bool {
    peer.ip().is_loopback() && crate::product_auth::request_is_https(headers)
}

// IdP callbacks are top-level navigations. Run the same-origin signer before
// consuming the authorization code; no account cookie is issued to this page.
async fn callback_bridge(request: axum::extract::Request) -> Response {
    use base64::Engine as _;
    let method = request.method().as_str().to_owned();
    let target = request.uri().to_string();
    let body = match axum::body::to_bytes(request.into_body(), 16_384).await {
        Ok(body) => body,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        serde_json::to_vec(&serde_json::json!({
            "method": method, "target": target, "body": String::from_utf8_lossy(&body),
        }))
        .unwrap(),
    );
    let mut response = axum::response::Html(format!(
        r#"<!doctype html><meta charset="utf-8"><meta name="referrer" content="no-referrer">
<title>Completing sign in</title><p id="status">Completing sign in…</p>
<script src="/device-proof.js"></script><script>
(async () => {{
try {{
CowboyDeviceProof.install();
const input = JSON.parse(atob("{encoded}"));
const response = await fetch(input.target, {{ method: input.method, credentials: "same-origin",
  headers: {{ "Content-Type": "application/x-www-form-urlencoded" }},
  body: input.method === "POST" ? input.body : undefined }});
const result = await response.json();
if (!response.ok || typeof result.redirect !== "string") throw new Error();
location.replace(result.redirect);
}} catch {{ document.getElementById("status").textContent = "Sign in failed. Return to Cowboy and try again."; }}
}})();</script>"#
    )).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; script-src 'self' 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'".parse().unwrap());
    response
}

pub(super) async fn enforce(
    State(state): State<Arc<SecureTransport>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    let local = peer.ip().is_loopback();
    let device_bearer = crate::product_auth::bearer_token(request.headers()).is_some_and(|token| {
        token.starts_with(crate::client_auth::ACCESS_TOKEN_PREFIX)
            || token.starts_with(crate::client_auth::AUTOMATION_ACCESS_TOKEN_PREFIX)
    });
    let internal = local
        && !crate::product_auth::has_forwarded_client_headers(request.headers())
        && (matches!(path.as_str(), "/healthz" | "/version" | "/metrics")
            || (request.method() == Method::GET
                && (path == "/"
                    || (path.starts_with("/api/machines/")
                        && path.ends_with("/deployment-health"))))
            || path.starts_with("/api/machine/")
            || device_bearer
            || path == "/api/auth/status"
            || path.starts_with("/api/auth/device/"));
    // Forwarded headers are accepted only from the same-host TLS terminator.
    // A remote caller cannot bypass TLS with X-Forwarded-Proto: https.
    if !internal && !trusted_https(peer, request.headers()) {
        return (StatusCode::UPGRADE_REQUIRED, "HTTPS is required").into_response();
    }
    if path == CHALLENGE_PATH && request.method() == Method::GET {
        let mut response = Json(serde_json::json!({
            "epoch": state.devices.epoch,
            "server_time_ms": auth_now_ms(),
        }))
        .into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
        return response;
    }
    if path == "/api/auth/tokens" && request.method() == Method::POST {
        return (StatusCode::GONE, "Use registered device credentials").into_response();
    }
    let now = auth_now_ms();
    if oidc_callback(&path) && proof_header(request.headers()).is_none() {
        return callback_bridge(request).await;
    }
    let proof = match proof_header(request.headers()) {
        Some(encoded) => match state.devices.verify(
            encoded,
            request.method().as_str(),
            request
                .uri()
                .path_and_query()
                .map_or(path.as_str(), axum::http::uri::PathAndQuery::as_str),
            &state.origins,
            now,
        ) {
            Ok(key) => Some(key),
            Err(_) => return reject_proof(),
        },
        None => None,
    };
    let key = proof.as_ref().map(|proof| proof.key.clone());
    // Reject before dispatch so refreshing an expired epoch can safely retry.
    if primary_login(&path) && key.is_none() {
        return reject_proof();
    }
    // Old bearer-only credentials are never a second authentication option.
    if let Some(token) = crate::product_auth::bearer_token(request.headers())
        && !token.starts_with(crate::client_auth::ACCESS_TOKEN_PREFIX)
        && !token.starts_with(crate::client_auth::AUTOMATION_ACCESS_TOKEN_PREFIX)
        && !(path == "/api/auth/device/refresh"
            && token.starts_with(crate::client_auth::REFRESH_TOKEN_PREFIX))
    {
        return reject_proof();
    }
    let mut registered = false;
    for cookie in COOKIES {
        let Some(token) = crate::product_auth::cookie_value(request.headers(), cookie) else {
            continue;
        };
        let bound = match state
            .store
            .browser_device_key(cookie, &crate::admin::hex_sha256(token.as_bytes()), now)
            .await
        {
            Ok(bound) => bound,
            Err(error) => {
                tracing::error!(%error, "device binding lookup failed");
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
        };
        registered |= !internal && key.is_some() && key == bound;
        if (internal || key.is_none() || key != bound) && !(primary_login(&path) && key.is_some()) {
            // Public status becomes signed-out; protected handlers reject.
            // Preserve the independent OIDC/Passkey transaction cookies.
            remove_cookie(request.headers_mut(), cookie);
        }
    }
    if let Some(proof) = proof.as_ref()
        && state.devices.consume(proof, registered, now).is_err()
    {
        return reject_proof();
    }
    let mut response = next.run(request).await;
    let issued = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .filter_map(|value| {
            let (first, _) = value.split_once(';')?;
            let (cookie, token) = first.split_once('=')?;
            if !COOKIES.contains(&cookie) || token.is_empty() {
                return None;
            }
            let ttl = value
                .split(';')
                .map(str::trim)
                .find_map(|part| part.strip_prefix("Max-Age=")?.parse::<i64>().ok())?;
            (ttl > 0).then(|| {
                (
                    cookie.to_owned(),
                    crate::admin::hex_sha256(token.as_bytes()),
                    ttl,
                )
            })
        })
        .collect::<Vec<_>>();
    for (cookie, token_hash, ttl) in issued {
        let Some(key) = key.as_deref() else {
            // A new credential issuer must explicitly join the proof protocol.
            // This response is deliberately not retryable after dispatch.
            return StatusCode::UNAUTHORIZED.into_response();
        };
        if let Err(error) = state
            .store
            .bind_browser_device(
                &cookie,
                &token_hash,
                key,
                now,
                now.saturating_add(ttl.saturating_mul(1000)),
            )
            .await
        {
            tracing::error!(%error, "device binding commit failed");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    if oidc_callback(&path)
        && response.status().is_redirection()
        && let Some(destination) = response.headers_mut().remove(header::LOCATION)
        && let Ok(destination) = destination.to_str()
    {
        let cookies = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        response = Json(serde_json::json!({ "redirect": destination })).into_response();
        for cookie in cookies {
            response.headers_mut().append(header::SET_COOKIE, cookie);
        }
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    if !internal {
        response.headers_mut().insert(
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=31536000".parse().unwrap(),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_device::tests::sign;
    use p256::ecdsa::SigningKey;

    #[test]
    fn forwarded_tls_requires_a_same_host_proxy() {
        let mut headers = HeaderMap::new();
        let local = "127.0.0.1:3333".parse().unwrap();
        let remote = "192.0.2.10:3333".parse().unwrap();
        assert!(!trusted_https(local, &headers));
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        assert!(trusted_https(local, &headers));
        assert!(!trusted_https(remote, &headers));
        headers.insert("x-forwarded-proto", "http".parse().unwrap());
        assert!(!trusted_https(local, &headers));
    }

    /// The companion runner owns a network namespace, TLS proxy, browser and
    /// empty data directory. This endpoint can never be started by `serve`.
    #[tokio::test]
    #[ignore = "run the isolated browser device conformance recipe"]
    async fn browser_conformance_fixture() {
        let root =
            PathBuf::from(std::env::var("COWBOY_DEVICE_FIXTURE").expect("fixture directory"));
        let origin = std::fs::read_to_string(root.join("origin")).unwrap();
        let store = Store::connect(
            &format!("sqlite://{}", root.join("fixture.sqlite3").display()),
            root.join("artifacts"),
        )
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let transport = Arc::new(SecureTransport::new(store, vec![origin]).unwrap());
        let app = Router::new()
            .route(
                "/api/auth/oidc/callback",
                any(|| async {
                    (
                        StatusCode::SEE_OTHER,
                        [
                            (header::LOCATION, "/fixture?reloaded".to_owned()),
                            (
                                header::SET_COOKIE,
                                format!(
                                    "cowboy_user={}; Path=/; Secure; HttpOnly; Max-Age=3600",
                                    crate::product_auth::new_session_token().unwrap()
                                ),
                            ),
                        ],
                    )
                }),
            )
            .route(
                "/api/auth/login",
                post(|| async {
                    (
                        [(
                            header::SET_COOKIE,
                            format!(
                                "cowboy_user={}; Path=/; Secure; HttpOnly; Max-Age=3600",
                                crate::product_auth::new_session_token().unwrap()
                            ),
                        )],
                        "signed in",
                    )
                }),
            )
            .route(
                "/api/private",
                get(|headers: HeaderMap| async move {
                    if crate::product_auth::user_cookie_token(&headers).is_some() {
                        StatusCode::OK
                    } else {
                        StatusCode::UNAUTHORIZED
                    }
                }),
            )
            .route(
                "/ws",
                get(|ws: WebSocketUpgrade, headers: HeaderMap| async move {
                    if crate::product_auth::user_cookie_token(&headers).is_none() {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    ws.protocols(["cowboy-fixture"])
                        .on_upgrade(|mut socket| async move {
                            let _ = socket.send(Message::Text("authenticated".into())).await;
                            let _ = socket.recv().await;
                        })
                }),
            )
            .layer(middleware::from_fn_with_state(transport, enforce));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        std::fs::write(
            root.join("backend"),
            listener.local_addr().unwrap().to_string(),
        )
        .unwrap();
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn transport_rejects_plaintext_stolen_cookies_unsigned_tokens_and_replays() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = std::env::temp_dir().join(format!(
            "cowboy-device-proof-{}-{}",
            std::process::id(),
            auth_now_ms()
        ));
        let store = Store::connect("sqlite::memory:", root.join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let transport = Arc::new(
            SecureTransport::new(store.clone(), vec!["https://cowboy.example".to_owned()]).unwrap(),
        );
        let app = Router::new()
            .route(
                "/api/auth/login",
                post(|| async {
                    (
                        [(
                            header::SET_COOKIE,
                            "cowboy_user=fixture; Path=/; Secure; HttpOnly; Max-Age=3600",
                        )],
                        "signed in",
                    )
                }),
            )
            .route(
                "/api/private",
                get(|headers: HeaderMap| async move {
                    if crate::product_auth::user_cookie_token(&headers).as_deref()
                        == Some("fixture")
                    {
                        StatusCode::OK
                    } else {
                        StatusCode::UNAUTHORIZED
                    }
                }),
            )
            .layer(middleware::from_fn_with_state(transport.clone(), enforce));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let http = reqwest::Client::new();
        let device = SigningKey::random(&mut rand::rngs::OsRng);
        let proof = |key: &SigningKey, method: &str, path: &str| {
            sign(key, &transport.devices.epoch, method, path, auth_now_ms())
        };
        assert_eq!(
            http.get(format!("{base}/api/private"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UPGRADE_REQUIRED
        );
        let login = http
            .post(format!("{base}/api/auth/login"))
            .header("x-forwarded-proto", "https")
            .header(HEADER, proof(&device, "POST", "/api/auth/login"))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        assert!(
            store
                .browser_device_key(
                    "cowboy_user",
                    &crate::admin::hex_sha256(b"fixture"),
                    auth_now_ms()
                )
                .await
                .unwrap()
                .is_some()
        );
        let request = || {
            http.get(format!("{base}/api/private"))
                .header("x-forwarded-proto", "https")
                .header(header::COOKIE, "cowboy_user=fixture")
        };
        assert_eq!(
            request().send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let attacker = SigningKey::random(&mut rand::rngs::OsRng);
        assert_eq!(
            request()
                .header(HEADER, proof(&attacker, "GET", "/api/private"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let valid = proof(&device, "GET", "/api/private");
        assert_eq!(
            request()
                .header(HEADER, &valid)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            request()
                .header(HEADER, &valid)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request()
                .bearer_auth("cow_legacy")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        task.abort();
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn websocket_proof_is_carried_in_protocol_not_url() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "cowboy-sync-v1, cowboy-device.fixture".parse().unwrap(),
        );
        assert_eq!(proof_header(&headers), Some("fixture"));
    }
}
