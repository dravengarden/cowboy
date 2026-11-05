
use super::*;
use crate::admin::{ADMIN_IDENTITIES_SETTING, AdminCredentials, AdminIdentities};
use crate::product_auth::USER_SESSION_COOKIE;
use crate::store::Store;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::SigningKey;
use std::os::unix::fs::PermissionsExt as _;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

#[test]
fn deployment_workspace_fingerprint_is_order_independent_and_fail_closed() {
    let first = serde_json::json!({"workspaces": [
        {"id": "z", "path": "/private/one"}, {"id": "a", "path": "/private/two"}
    ]});
    let second = serde_json::json!({"workspaces": [{"id": "a"}, {"id": "z"}]});
    let expected = crate::admin::hex_sha256(br#"["a","z"]"#);
    assert_eq!(machine_workspace_ids_sha256(&first), Some(expected.clone()));
    assert_eq!(machine_workspace_ids_sha256(&second), Some(expected));
    for malformed in [
        serde_json::json!({}),
        serde_json::json!({"workspaces": null}),
        serde_json::json!({"workspaces": [{"id": "a"}, {}]}),
        serde_json::json!({"workspaces": [{"id": 1}]}),
    ] {
        assert_eq!(machine_workspace_ids_sha256(&malformed), None);
    }
    assert_eq!(
        machine_workspace_ids_sha256(&serde_json::json!({"workspaces": []})),
        Some(crate::admin::hex_sha256(b"[]"))
    );
}

fn auth_state(hub: Hub, store: Option<Store>) -> ProductAuthState {
    let data_dir = std::env::temp_dir().join(format!(
        "cowboy-admin-setup-state-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&data_dir).unwrap();
    let setup = Arc::new(crate::admin::AdminSetupState::new(data_dir));
    let mut identities = AdminIdentities::from_setting(
        hub.settings_snapshot()
            .get(crate::admin::ADMIN_IDENTITIES_SETTING),
    );
    if let Ok(Some(_)) =
        crate::admin::ensure_admin_setup_token(&setup.data_dir, &mut identities, true)
    {
        hub.set_setting(
            ADMIN_IDENTITIES_SETTING.to_owned(),
            serde_json::to_value(&identities).unwrap(),
        );
    }
    ProductAuthState {
        hub,
        store,
        rate_limits: Arc::new(crate::product_auth::AuthRateLimiter::default()),
        public_origins: Arc::new(Vec::new()),
        runtime_health: None,
        persistence_health: None,
        runtime_router: None,
        plugin_catalog: None,
        plugin_storage: None,
        provider_catalog: None,
        passkeys: Arc::new(crate::passkey::PasskeyCeremonies::default()),
        setup,
        setup_lock: Arc::new(tokio::sync::Mutex::new(())),
        product_auth_enabled: true,
        product_authentication: Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
            None,
        )),
        oidc_transactions: Arc::new(crate::oidc::OidcTransactions::default()),
        oidc_native_handoffs: Arc::new(crate::oidc::NativeHandoffs::default()),
        device_authorizations: Arc::new(crate::client_auth::DeviceAuthorizations::default()),
        device_access: Arc::new(crate::client_auth::DeviceAccessSessions::default()),
        capacity_runtime: Arc::new(ActiveCapacityRuntime::default()),
    }
}

#[test]
fn auth_off_does_not_read_oidc_secret_files() {
    let missing = std::env::temp_dir().join(format!(
        "cowboy-missing-oidc-config-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    assert!(
        crate::plugin_activation::load_legacy_oidc_provider(false, Some(&missing))
            .unwrap()
            .is_none()
    );
    assert!(crate::plugin_activation::load_legacy_oidc_provider(true, Some(&missing)).is_err());
}

async fn test_store() -> (Store, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "cowboy-product-auth-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let url = format!("sqlite://{}", root.join("cowboy.sqlite3").display());
    let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    (store, root)
}

fn test_oidc_provider(
    root: &std::path::Path,
    product_account: &str,
    admin_account: Option<&str>,
) -> Arc<crate::oidc::OidcProvider> {
    let client_key = SigningKey::from_bytes(&[7_u8; 32]);
    let id_token_key = SigningKey::from_bytes(&[9_u8; 32]);
    let private_key_path = root.join("oidc-client-private.jwk");
    std::fs::write(
        &private_key_path,
        serde_json::to_vec(&serde_json::json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "d": URL_SAFE_NO_PAD.encode(client_key.to_bytes()),
            "x": URL_SAFE_NO_PAD.encode(client_key.verifying_key().as_bytes()),
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&private_key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let config_path = root.join("oidc.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec(&serde_json::json!({
            "schema": "dravengarden.cowboy.cardea-oidc/v1",
            "display_name": "Cardea",
            "issuer": "https://cardea.example",
            "client_id": "cowboy-test",
            "client_key_id": "primary",
            "client_private_key_file": private_key_path,
            "id_token_key_id": "issuer-primary",
            "id_token_public_key_jwk": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": URL_SAFE_NO_PAD.encode(id_token_key.verifying_key().as_bytes()),
            },
            "subject": "draven",
            "account": product_account,
            "admin_account": admin_account,
            "redirect_uri": "https://cowboy.example/api/auth/oidc/callback",
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    Arc::new(crate::oidc::OidcProvider::load(&config_path).unwrap())
}

fn install_rustls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn test_verified_identity() -> crate::oidc::VerifiedIdentity {
    crate::oidc::VerifiedIdentity {
        issuer: "https://cardea.example".to_owned(),
        subject: "draven".to_owned(),
        session_id: Some("cardea-session".to_owned()),
        issued_at: 1_000,
        authenticated_at: Some(999),
        authentication_context: None,
        authentication_methods: vec!["passkey".to_owned()],
    }
}

async fn spawn_auth(state: ProductAuthState) -> (String, tokio::task::JoinHandle<()>) {
    install_rustls();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            product_auth_router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (format!("http://{addr}"), handle)
}

fn origin_for(base: &str) -> String {
    base.trim_end_matches('/').to_owned()
}

#[tokio::test]
async fn mandatory_transport_binds_real_password_sessions_and_honors_logout() {
    install_rustls();
    let (store, root) = test_store().await;
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: "a".repeat(32),
        username: "owner".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let mut state = auth_state(Hub::new(), Some(store.clone()));
    state.public_origins = Arc::new(vec!["https://cowboy.example".to_owned()]);
    let transport = Arc::new(
        secure_transport::SecureTransport::new(
            store.clone(),
            state.public_origins.as_ref().clone(),
        )
        .unwrap(),
    );
    let app = product_auth_router(state).layer(middleware::from_fn_with_state(
        transport,
        secure_transport::enforce,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let http = reqwest::Client::new();
    let challenge: serde_json::Value = http
        .get(format!("{base}/api/auth/browser/challenge"))
        .header("x-forwarded-proto", "https")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng);
    let request = |method: Method, path: &str, cookie: Option<&str>| {
        let proof = crate::browser_device::tests::sign(
            &key,
            challenge["epoch"].as_str().unwrap(),
            method.as_str(),
            path,
            auth_now_ms(),
        );
        let mut request = http
            .request(method, format!("{base}{path}"))
            .header("x-forwarded-proto", "https")
            .header(header::ORIGIN, "https://cowboy.example")
            .header(crate::browser_device::HEADER, proof);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        request
    };
    let login = request(Method::POST, "/api/auth/login", None)
        .json(&serde_json::json!({"account": "owner", "password": "Correct-horse-bat1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = cookie_header(&set_cookie(&login, USER_SESSION_COOKIE).unwrap());
    assert_eq!(
        request(Method::GET, "/api/auth/me", Some(&cookie))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        http.get(format!("{base}/api/auth/me"))
            .header("x-forwarded-proto", "https")
            .header(header::COOKIE, &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(Method::POST, "/api/auth/logout", Some(&cookie))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        request(Method::GET, "/api/auth/me", Some(&cookie))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // A refresh bearer is owned by the existing Ed25519 refresh handler,
    // which must still see it and reject an invalid credential itself.
    let refresh = http
        .post(format!("{base}/api/auth/device/refresh"))
        .header("x-forwarded-proto", "https")
        .bearer_auth("cow_refresh_fixture")
        .send()
        .await
        .unwrap();
    assert_eq!(refresh.status(), StatusCode::UNAUTHORIZED);
    assert!(!refresh.headers().contains_key("x-cowboy-device-proof"));
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

async fn post_json(
    url: &str,
    origin: &str,
    cookie: Option<&str>,
    body: serde_json::Value,
) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .post(url)
        .header(header::ORIGIN, origin)
        .json(&body);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    request.send().await.unwrap()
}

async fn put_json(
    url: &str,
    origin: &str,
    cookie: Option<&str>,
    body: serde_json::Value,
) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .put(url)
        .header(header::ORIGIN, origin)
        .json(&body);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    request.send().await.unwrap()
}

fn set_cookie(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{name}=")))
        .map(ToOwned::to_owned)
}

fn cookie_header(set_cookie: &str) -> String {
    set_cookie
        .split(';')
        .next()
        .unwrap_or(set_cookie)
        .trim()
        .to_owned()
}

fn seed_admin(hub: &Hub) -> String {
    let mut identities = AdminIdentities::default();
    let token = identities
        .bootstrap(
            &AdminCredentials {
                account: "owner".to_owned(),
                password: "Correct-horse-bat1".to_owned(),
            },
            1_900_000_000_000,
        )
        .unwrap();
    hub.set_setting(
        ADMIN_IDENTITIES_SETTING.to_owned(),
        serde_json::to_value(&identities).unwrap(),
    );
    format!("cowboy_admin={token}")
}

async fn prove_setup(base: &str, origin: &str, token: &str) -> String {
    let prepared = post_json(
        &format!("{base}/api/auth/setup"),
        origin,
        None,
        serde_json::json!({ "token": token }),
    )
    .await;
    assert_eq!(prepared.status(), StatusCode::OK);
    cookie_header(&set_cookie(&prepared, crate::admin::ADMIN_SETUP_COOKIE).expect("setup cookie"))
}

fn setup_token_from(state: &ProductAuthState) -> String {
    std::fs::read_to_string(state.setup.token_path())
        .unwrap()
        .trim()
        .to_owned()
}

async fn spawn_enforcement(state: ProductAuthState) -> (String, tokio::task::JoinHandle<()>) {
    install_rustls();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let app = Router::new()
            .route("/ws", any(test_ws_upgrade))
            .route("/metrics", get(test_metrics))
            .with_state(state);
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (format!("http://{addr}"), handle)
}

fn me_proof_headers(
    key: &SigningKey,
    device: &str,
    token: &str,
    method: &str,
    path: &str,
    now: i64,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    for (name, value) in
        crate::client_auth::signed_proof_headers(key, device, token, method, path, now).unwrap()
    {
        headers.insert(
            header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    headers
}

async fn me_test_user(store: &Store, disabled: bool) -> crate::store::ProductUser {
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: "c".repeat(32),
        username: "device-user".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        // These tests authorize an ephemeral proof key, never a password.
        password_hash: "unused-device-fixture".to_owned(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: disabled.then_some(now),
    };
    store.insert_user(&user).await.unwrap();
    user
}

#[tokio::test]
async fn auth_me_accepts_device_proofs_once_and_reuses_middleware_identity() {
    let (store, root) = test_store().await;
    let user = me_test_user(&store, false).await;
    let state = auth_state(Hub::new(), Some(store.clone()));
    let key = crate::client_auth::new_signing_key().unwrap();
    let public = crate::client_auth::public_key_to_base64(&key);
    let device = "dddddddddddddddddddddddddddddddd";
    persist_fixture_device(&store, &user.id, device, &public).await;
    let (token, _) = state
        .device_access
        .issue(device, &user.id, &public, auth_now_ms())
        .unwrap();
    let (base, server) = spawn_auth(state.clone()).await;
    let client = reqwest::Client::new();
    let path = "/api/auth/me?source=cli";
    for (method, signed_path, now) in [
        ("GET", "/api/plugins", auth_now_ms()),
        ("POST", path, auth_now_ms()),
        ("GET", "/api/auth/me?source=different", auth_now_ms()),
        ("GET", path, auth_now_ms() - 3_600_000),
    ] {
        let response = client
            .get(format!("{base}{path}"))
            .headers(me_proof_headers(
                &key,
                device,
                &token,
                method,
                signed_path,
                now,
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        client
            .get(format!("{base}{path}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let headers = me_proof_headers(&key, device, &token, "GET", path, auth_now_ms());
    let response = client
        .get(format!("{base}{path}"))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let me = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(me["account"], "device-user");
    assert!(me.get("access_token").is_none());
    assert_eq!(
        client
            .get(format!("{base}{path}"))
            .headers(headers)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let headers = me_proof_headers(&key, device, &token, "GET", path, auth_now_ms());
    let uri: Uri = path.parse().unwrap();
    let verified = resolve_product_api_request_principal(
        ProductRequestAuth::from(&state),
        &Method::GET,
        &uri,
        &headers,
    )
    .await
    .unwrap()
    .unwrap();
    // This is the production middleware's extension. The handler must not
    // authenticate its already-consumed nonce a second time.
    let response = api_auth_me(
        State(state.clone()),
        Some(Extension(verified)),
        Method::GET,
        uri,
        headers,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    state.device_access.revoke_device(device);
    assert_eq!(
        client
            .get(format!("{base}{path}"))
            .headers(me_proof_headers(
                &key,
                device,
                &token,
                "GET",
                path,
                auth_now_ms()
            ))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    server.abort();
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(&state.setup.data_dir).unwrap();
}

#[tokio::test]
async fn auth_me_keeps_automation_policy_scopes_and_expiry() {
    let (store, root) = test_store().await;
    let user = me_test_user(&store, false).await;
    let key = crate::client_auth::new_signing_key().unwrap();
    let public = crate::client_auth::public_key_to_base64(&key);
    for (enabled, scope, age, expected) in [
        (true, "api:read", 0, StatusCode::OK),
        (false, "api:read", 0, StatusCode::UNAUTHORIZED),
        (true, "sessions:write", 0, StatusCode::UNAUTHORIZED),
        (true, "api:read", 3_600_000, StatusCode::UNAUTHORIZED),
    ] {
        let mut state = auth_state(Hub::new(), Some(store.clone()));
        let mut authentication = crate::auth_plugins::ProductAuthentication::test_default(None);
        authentication.automation.enabled = enabled;
        state.product_authentication = Arc::new(authentication);
        let (token, _) = state
            .device_access
            .issue_automation(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &user.id,
                &public,
                vec![scope.to_owned()],
                auth_now_ms() - age,
                60_000,
            )
            .unwrap();
        let (base, server) = spawn_auth(state.clone()).await;
        let response = reqwest::Client::new()
            .get(format!("{base}/api/auth/me"))
            .headers(me_proof_headers(
                &key,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &token,
                "GET",
                "/api/auth/me",
                auth_now_ms(),
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        server.abort();
        std::fs::remove_dir_all(&state.setup.data_dir).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn auth_me_rejects_disabled_device_users() {
    let (store, root) = test_store().await;
    let user = me_test_user(&store, true).await;
    let state = auth_state(Hub::new(), Some(store));
    let key = crate::client_auth::new_signing_key().unwrap();
    let (token, _) = state
        .device_access
        .issue(
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            &user.id,
            &crate::client_auth::public_key_to_base64(&key),
            auth_now_ms(),
        )
        .unwrap();
    let (base, server) = spawn_auth(state.clone()).await;
    let response = reqwest::Client::new()
        .get(format!("{base}/api/auth/me"))
        .headers(me_proof_headers(
            &key,
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            &token,
            "GET",
            "/api/auth/me",
            auth_now_ms(),
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    server.abort();
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(&state.setup.data_dir).unwrap();
}

#[tokio::test]
async fn cardea_device_exchange_reuses_capacity_and_cannot_revive_local_revocation() {
    install_rustls();
    let key = crate::client_auth::new_signing_key().unwrap();
    let public = crate::client_auth::public_key_to_base64(&key);
    let identity_public = public.clone();
    let mock = Router::new()
            .route("/oauth2/token", post(|| async { Json(serde_json::json!({"token_type":"Bearer","access_token":"consumer-fixture"})) }))
            .route("/v1/device-grant-exchanges", post(move |headers: HeaderMap, Json(value): Json<serde_json::Value>| {
                let public = identity_public.clone();
                async move {
                    assert_eq!(headers[header::AUTHORIZATION], "Bearer consumer-fixture");
                    assert_eq!(value["access_token"], "identity-fixture");
                    let now = auth_now_ms() as u64 / 1000;
                    Json(serde_json::json!({
                        "schema":"dravengarden.cardea.device-identity/v1","subject_id":"draven",
                        "client_id":"cowboy-production","grant_id":"01234567-89ab-4cde-8fab-0123456789ab",
                        "public_key":public,"grant_public_key":public,"generation":0,
                        "authenticated_at":now,"expires_at":now+300,"grant_expires_at":now+365*86400,
                    }))
                }
            }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let mock_server = tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    let (store, root) = test_store().await;
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: "a".repeat(32),
        username: "draven".into(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.into(),
        password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let mut state = auth_state(Hub::new(), Some(store.clone()));
    crate::oidc::OidcProvider::cardea_device_fixture(&issuer)
        .exchange_cardea_device("identity-fixture", "signed-fixture", &public)
        .await
        .unwrap();
    state.product_authentication = Arc::new(
        crate::auth_plugins::ProductAuthentication::test_default(Some(Arc::new(
            crate::oidc::OidcProvider::cardea_device_fixture(&issuer),
        ))),
    );
    let device_access = state.device_access.clone();
    let (base, server) = spawn_auth(state).await;
    let endpoint = format!("{base}/api/auth/device/cardea");
    let configuration = reqwest::get(&endpoint)
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(configuration["issuer"], issuer);
    let request = serde_json::json!({"name":"Hawk CLI","public_key":public,"access_token":"identity-fixture","assertion":"signed-fixture"});
    let http = reqwest::Client::new();
    let response = http.post(&endpoint).json(&request).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let first = response.json::<serde_json::Value>().await.unwrap();
    assert!(first["access_expires_at_ms"].as_i64().unwrap() <= auth_now_ms() + 300_000);
    assert!(first.get("refresh_token").is_none());
    let response = http.post(&endpoint).json(&request).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let second = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(first["device_id"], second["device_id"]);
    assert_eq!(
        store
            .list_user_devices_for_user(&user.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let wrong =
        crate::client_auth::public_key_to_base64(&crate::client_auth::new_signing_key().unwrap());
    let mut mismatched = request.clone();
    mismatched["public_key"] = serde_json::json!(wrong);
    assert_eq!(
        http.post(&endpoint)
            .json(&mismatched)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let id = first["device_id"].as_str().unwrap();
    store.revoke_user_device(id, auth_now_ms()).await.unwrap();
    // Model a mint racing after durable revocation: resource admission must
    // reject it even when the process-local access map contains the token.
    let (late_token, _) = device_access
        .issue(id, &user.id, &public, auth_now_ms())
        .unwrap();
    let mut protected = http
        .get(format!("{base}/api/auth/me"))
        .bearer_auth(&late_token);
    for (name, value) in crate::client_auth::signed_proof_headers(
        &key,
        id,
        &late_token,
        "GET",
        "/api/auth/me",
        auth_now_ms(),
    )
    .unwrap()
    {
        protected = protected.header(name, value);
    }
    assert_eq!(
        protected.send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        http.post(&endpoint)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert!(
        store
            .list_user_devices_for_user(&user.id)
            .await
            .unwrap()
            .is_empty()
    );
    server.abort();
    mock_server.abort();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn browser_approved_device_rotates_sender_constrained_credentials() {
    let (store, root) = test_store().await;
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: "a".repeat(32),
        username: "draven".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let cookie_token = crate::product_auth::new_session_token().unwrap();
    store
        .insert_user_session(&crate::store::ProductUserSession {
            token_hash: crate::admin::hex_sha256(cookie_token.as_bytes()),
            session_id: "device-session-000000000000000001".to_owned(),
            user_id: user.id.clone(),
            client_kind: "browser".to_owned(),
            principal_class: "human".to_owned(),
            created_at_ms: now,
            expires_at_ms: now
                + crate::auth_plugins::SessionServerPolicy::default().primary_max_age_ms,
            last_seen_at_ms: now,
            user_agent: Some("device-authorization-test".to_owned()),
            passkey_verified_at_ms: None,
            primary_authenticated_at_ms: now,
            primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
            revoked_at_ms: None,
            revoke_reason: None,
            auth_provider_id: None,
            auth_issuer: None,
            auth_subject: None,
            auth_sid: None,
            id_token_ciphertext: None,
        })
        .await
        .unwrap();
    let cookie = format!("{USER_SESSION_COOKIE}={cookie_token}");
    let state = auth_state(Hub::new(), Some(store.clone()));
    let device_access = state.device_access.clone();
    let (base, server) = spawn_auth(state).await;
    let origin = origin_for(&base);

    let signing_key = crate::client_auth::new_signing_key().unwrap();
    let code_verifier = crate::client_auth::new_code_verifier().unwrap();
    let started = reqwest::Client::new()
        .post(format!("{base}/api/auth/device/authorizations"))
        .json(&crate::client_auth::StartAuthorizationRequest {
            name: "Zed on Hawk".to_owned(),
            public_key: crate::client_auth::public_key_to_base64(&signing_key),
            code_challenge: crate::client_auth::code_challenge(&code_verifier).unwrap(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::CREATED);
    assert_eq!(
        started.headers()[header::CACHE_CONTROL],
        header::HeaderValue::from_static("no-store")
    );
    let started = started
        .json::<crate::client_auth::StartAuthorizationResponse>()
        .await
        .unwrap();
    let verification = url::Url::parse(&format!("{base}{}", started.verification_url)).unwrap();
    assert_eq!(verification.path(), "/auth/device");
    let capability = url::form_urlencoded::parse(
        verification
            .fragment()
            .expect("authorization fragment")
            .as_bytes(),
    )
    .into_owned()
    .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(capability.get("request_id"), Some(&started.request_id));
    let approval_token = capability
        .get("approval_token")
        .expect("approval token")
        .clone();
    let browser_request = serde_json::json!({
        "request_id": started.request_id,
        "approval_token": approval_token,
    });

    let inspected = reqwest::Client::new()
        .post(format!("{base}/api/auth/device/authorizations/inspect"))
        .json(&browser_request)
        .send()
        .await
        .unwrap();
    assert_eq!(inspected.status(), StatusCode::OK);
    let inspected: serde_json::Value = inspected.json().await.unwrap();
    assert_eq!(inspected["name"], "Zed on Hawk");
    assert_eq!(inspected["status"], "pending");
    assert!(
        inspected["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("SHA256:")
    );

    let socket_url = format!(
        "{}/api/auth/device/authorizations/events",
        base.replacen("http://", "ws://", 1)
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(socket_url).await.unwrap();
    socket
        .send(TungsteniteMessage::Text(
            serde_json::to_string(&crate::client_auth::AuthorizationEventsHandshake {
                request_id: started.request_id.clone(),
                code_verifier: code_verifier.clone(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

    let missing_login = post_json(
        &format!("{base}/api/auth/device/authorizations/approve"),
        &origin,
        None,
        browser_request.clone(),
    )
    .await;
    assert_eq!(missing_login.status(), StatusCode::UNAUTHORIZED);
    let approved = post_json(
        &format!("{base}/api/auth/device/authorizations/approve"),
        &origin,
        Some(&cookie),
        browser_request,
    )
    .await;
    assert_eq!(approved.status(), StatusCode::OK);
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .expect("device approval event timed out")
        .expect("device approval socket closed")
        .expect("device approval event failed");
    let TungsteniteMessage::Text(event) = event else {
        panic!("expected a device authorization text event");
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&event).unwrap()["status"],
        "approved"
    );

    let exchange = crate::client_auth::signed_exchange_request(
        &signing_key,
        started.request_id.clone(),
        code_verifier,
        auth_now_ms(),
    )
    .unwrap();
    let exchanged = reqwest::Client::new()
        .post(format!("{base}/api/auth/device/exchange"))
        .json(&exchange)
        .send()
        .await
        .unwrap();
    assert_eq!(exchanged.status(), StatusCode::OK);
    let tokens = exchanged
        .json::<crate::client_auth::DeviceTokenResponse>()
        .await
        .unwrap();
    assert!(
        tokens
            .access_token
            .starts_with(crate::client_auth::ACCESS_TOKEN_PREFIX)
    );
    assert!(
        tokens
            .refresh_token
            .starts_with(crate::client_auth::REFRESH_TOKEN_PREFIX)
    );

    let mut proof_headers = HeaderMap::new();
    proof_headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {}", tokens.access_token).parse().unwrap(),
    );
    for (name, value) in crate::client_auth::signed_proof_headers(
        &signing_key,
        &tokens.device_id,
        &tokens.access_token,
        "GET",
        "/api/sessions",
        auth_now_ms(),
    )
    .unwrap()
    {
        proof_headers.insert(
            header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    let identity = device_access
        .authenticate(&proof_headers, &Method::GET, "/api/sessions", auth_now_ms())
        .unwrap()
        .expect("device access identity");
    assert_eq!(identity.user_id, user.id);
    assert!(
        device_access
            .authenticate(&proof_headers, &Method::GET, "/api/sessions", auth_now_ms(),)
            .is_err()
    );

    let listed = reqwest::Client::new()
        .get(format!("{base}/api/auth/devices"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(
        listed.json::<serde_json::Value>().await.unwrap()["devices"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let refresh_path = "/api/auth/device/refresh";
    let mut refresh_request = reqwest::Client::new()
        .post(format!("{base}{refresh_path}"))
        .bearer_auth(&tokens.refresh_token);
    for (name, value) in crate::client_auth::signed_proof_headers(
        &signing_key,
        &tokens.device_id,
        &tokens.refresh_token,
        "POST",
        refresh_path,
        auth_now_ms(),
    )
    .unwrap()
    {
        refresh_request = refresh_request.header(name, value);
    }
    let refreshed = refresh_request.send().await.unwrap();
    assert_eq!(refreshed.status(), StatusCode::OK);
    let refreshed = refreshed
        .json::<crate::client_auth::DeviceTokenResponse>()
        .await
        .unwrap();
    assert_ne!(refreshed.refresh_token, tokens.refresh_token);

    let unsigned_replay = reqwest::Client::new()
        .post(format!("{base}{refresh_path}"))
        .bearer_auth(&tokens.refresh_token)
        .send()
        .await
        .unwrap();
    assert_eq!(unsigned_replay.status(), StatusCode::UNAUTHORIZED);
    assert!(device_access.token_still_valid(&refreshed.access_token, &identity, auth_now_ms()));

    let mut replay = reqwest::Client::new()
        .post(format!("{base}{refresh_path}"))
        .bearer_auth(&tokens.refresh_token);
    for (name, value) in crate::client_auth::signed_proof_headers(
        &signing_key,
        &tokens.device_id,
        &tokens.refresh_token,
        "POST",
        refresh_path,
        auth_now_ms(),
    )
    .unwrap()
    {
        replay = replay.header(name, value);
    }
    assert_eq!(
        replay.send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(!device_access.token_still_valid(&refreshed.access_token, &identity, auth_now_ms()));
    assert!(
        store
            .list_user_devices_for_user(&user.id)
            .await
            .unwrap()
            .is_empty()
    );

    let replayed_exchange = reqwest::Client::new()
        .post(format!("{base}/api/auth/device/exchange"))
        .json(&exchange)
        .send()
        .await
        .unwrap();
    assert_eq!(replayed_exchange.status(), StatusCode::GONE);

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

async fn test_ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let principal = resolve_product_principal(
        state.product_auth_enabled,
        state.store.as_ref(),
        &state.hub,
        &headers,
    )
    .await;
    match authorize_ws_upgrade(&headers, peer, &state.public_origins, principal) {
        Ok(_) => {
            drop(ws);
            StatusCode::SWITCHING_PROTOCOLS.into_response()
        }
        Err(status) => status.into_response(),
    }
}

async fn test_metrics(ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if crate::product_auth::metrics_scrape_allowed(peer, &headers) {
        StatusCode::OK.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

#[test]
fn web_push_enrollment_uses_product_authentication() {
    for (method, path) in [
        (Method::GET, "/api/web-push/config"),
        (Method::HEAD, "/api/web-push/config"),
        (Method::PUT, "/api/web-push/subscription"),
        (Method::DELETE, "/api/web-push/subscription"),
    ] {
        assert_eq!(classify_route(&method, path), RouteAuth::Product);
        assert!(enforce_product_session_freshness_before_dispatch(path));
    }
    for (method, path) in [
        (Method::POST, "/api/web-push/config"),
        (Method::GET, "/api/web-push/subscription"),
        (Method::PUT, "/api/web-push/unknown"),
    ] {
        assert_eq!(classify_route(&method, path), RouteAuth::AdminOperator);
    }
}

#[test]
fn classify_route_table_matches_capability_matrix() {
    assert_eq!(
        classify_route(&Method::GET, "/api/agent-tools"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::PUT, "/api/agent-tools/codex"),
        RouteAuth::ProductOperator
    );
    for path in [
        "/api/project-policies",
        "/api/project-placements",
        "/api/machines/hawk/projects",
    ] {
        assert_eq!(classify_route(&Method::GET, path), RouteAuth::Product);
    }
    for (method, path) in [
        (Method::POST, "/api/machines/hawk/projects"),
        (Method::PUT, "/api/machines/ovh/project-policy"),
        (Method::PUT, "/api/machines/ovh/maintenance"),
    ] {
        assert_eq!(classify_route(&method, path), RouteAuth::ProductOperator);
    }
    assert_eq!(classify_route(&Method::GET, "/healthz"), RouteAuth::Public);
    assert_eq!(classify_route(&Method::GET, "/ws"), RouteAuth::Product);
    assert_eq!(
        classify_route(&Method::GET, "/metrics"),
        RouteAuth::MetricsScrape
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/sessions"),
        RouteAuth::ProductOperator
    );
    for path in [
        "/api/plugins/codex/auth",
        "/api/plugins/codex/auth/start",
        "/api/plugins/codex/auth/request-1",
        "/api/providers/codex/auth",
        "/api/providers/codex/auth/start",
        "/api/providers/codex/auth/request-1",
    ] {
        assert_eq!(
            classify_route(&Method::POST, path),
            RouteAuth::ProductOperator,
            "Provider authentication route {path} must accept a product operator",
        );
    }
    assert_eq!(
        classify_route(&Method::POST, "/api/plugins/catalog/refresh"),
        RouteAuth::AdminOperator,
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/plugins/password/ui/index.js"),
        RouteAuth::AdminOperator,
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/plugins/password/call"),
        RouteAuth::ProductOperator,
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/plugins/password/ui/index.js"),
        RouteAuth::AdminOperator,
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/sessions/abc/info"),
        RouteAuth::ProductSessionSee
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/sessions/abc/prompt"),
        RouteAuth::ProductSessionMutate
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/artifacts/hash"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/machines/enrollment"),
        RouteAuth::ProductOrAdminOperator
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/machine/service"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::DELETE, "/api/machines/enrollment"),
        RouteAuth::ProductOrAdminOperator
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/machines/m-123/revoke"),
        RouteAuth::ProductOrAdminOperator
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/machines/m-123/refresh"),
        RouteAuth::ProductOrAdminOperator
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/machines/m-123/deployment-health"),
        RouteAuth::Public
    );
    // Durable-state counts are operational diagnostics: never public, and
    // reachable by the product UI and by the Operator CLI.
    assert_eq!(
        classify_route(&Method::GET, "/api/machines/m-123/durable-state"),
        RouteAuth::ProductOrAdminOperator
    );
    for path in [
        "/api/metrics",
        "/api/logs",
        "/api/logs/log-123",
        "/api/observability/incidents",
        "/api/machines/m-123/plugins/victoria/operations/op-123/machine-receipt",
        "/api/machines/m-123/plugins/victoria/installation-operations",
        "/api/machines/m-123/plugins/victoria/operations/op-123/recovery-assessment",
    ] {
        assert_eq!(
            classify_route(&Method::GET, path),
            RouteAuth::ProductOrAdminOperator,
            "observability route {path} must accept product and legacy admin operators",
        );
    }
    for (method, path) in [
        (Method::POST, "/api/usage/codex/reset"),
        (Method::PUT, "/api/usage/codex/reset/schedule"),
        (Method::DELETE, "/api/usage/codex/reset/schedule"),
        (Method::PUT, "/api/usage/anthropic/executor"),
    ] {
        assert_eq!(
            classify_route(&method, path),
            RouteAuth::ProductOrAdminOperator,
            "usage reset route {path} is driven from the product UI and must \
                 accept a product operator",
        );
    }
    assert_eq!(
        classify_route(&Method::GET, "/api/usage/codex/activity"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/auth/tokens"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/auth/tokens"),
        RouteAuth::ProductOperator
    );
    assert_eq!(
        classify_route(&Method::DELETE, "/api/auth/tokens/abc"),
        RouteAuth::Product
    );
    for path in [
        "/api/auth/device/authorizations",
        "/api/auth/device/authorizations/inspect",
        "/api/auth/device/exchange",
        "/api/auth/device/refresh",
    ] {
        assert_eq!(classify_route(&Method::POST, path), RouteAuth::Public);
    }
    for path in [
        "/api/telemetry/v1/logs",
        "/api/telemetry/v1/metrics",
        "/api/telemetry/v1/traces",
    ] {
        assert_eq!(classify_route(&Method::POST, path), RouteAuth::Product);
    }
    assert_eq!(
        classify_route(&Method::GET, "/api/auth/device/authorizations/events"),
        RouteAuth::Public
    );
    for path in [
        "/api/auth/device/authorizations/approve",
        "/api/auth/device/authorizations/deny",
    ] {
        assert_eq!(classify_route(&Method::POST, path), RouteAuth::Product);
    }
    assert_eq!(
        classify_route(&Method::GET, "/api/auth/devices"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::DELETE, "/api/auth/devices/device-1"),
        RouteAuth::Product
    );
    for path in [
        "/api/auth/passkeys/external/options",
        "/api/auth/passkeys/external/complete",
        "/api/auth/passkeys/external/fail",
    ] {
        assert_eq!(classify_route(&Method::POST, path), RouteAuth::Public);
    }
    for path in [
        "/api/auth/passkeys/external/start",
        "/api/auth/passkeys/external/finalize",
    ] {
        assert_eq!(classify_route(&Method::POST, path), RouteAuth::Product);
    }
    assert_eq!(
        classify_route(&Method::GET, "/api/auth/passkeys/external/events"),
        RouteAuth::Product
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/auth/setup"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/auth/oidc/start"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/auth/oidc/callback"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/auth/oidc/native/exchange"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/auth/oidc/native/poll"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/auth/oidc/native/events"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/auth/oidc/native/cancel"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/auth/oidc/native/complete"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/auth/providers/cardea/native/poll"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/auth/providers/cardea/native/events"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/auth/providers/cardea/native/cancel"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/admin/auth"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/admin/auth/setup"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/admin/auth/bootstrap"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/admin/auth/login"),
        RouteAuth::Public
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/admin/overview"),
        RouteAuth::AdminViewer
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/admin/accounts"),
        RouteAuth::AdminViewer
    );
    assert_eq!(
        classify_route(&Method::POST, "/api/admin/accounts"),
        RouteAuth::AdminOwner
    );
    assert_eq!(
        classify_route(&Method::PUT, "/api/admin/registration"),
        RouteAuth::AdminOperator
    );
    assert_eq!(
        classify_route(&Method::PUT, "/api/admin/permissions"),
        RouteAuth::AdminOwner
    );
    assert!(!matches!(
        classify_route(&Method::GET, "/api/unknown"),
        RouteAuth::Public | RouteAuth::Product
    ));
}

#[test]
fn websocket_auth_deadline_is_reported_by_the_protocol_handler() {
    assert!(enforce_product_session_freshness_before_dispatch(
        "/api/sessions/session-1/bootstrap"
    ));
    assert!(enforce_product_session_freshness_before_dispatch(
        "/api/auth/me"
    ));
    assert!(!enforce_product_session_freshness_before_dispatch("/ws"));
}

#[test]
fn device_authorization_prefers_the_configured_public_browser_origin() {
    assert_eq!(
        device_verification_url(
            &["https://cowboy.example".to_owned()],
            "request",
            "approval",
        ),
        "https://cowboy.example/auth/device#request_id=request&approval_token=approval",
    );
    assert_eq!(
        device_verification_url(&[], "request", "approval"),
        "/auth/device#request_id=request&approval_token=approval",
    );
}

#[tokio::test]
async fn legacy_browser_session_binds_once_to_its_next_primary_method() {
    let (store, root) = test_store().await;
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: "a".repeat(32),
        username: "owner".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let legacy_token = "b".repeat(64);
    let legacy_hash = crate::admin::hex_sha256(legacy_token.as_bytes());
    store
        .insert_user_session(&crate::store::ProductUserSession {
            token_hash: legacy_hash.clone(),
            session_id: "legacy-session-000000000000000001".to_owned(),
            user_id: user.id.clone(),
            client_kind: "browser".to_owned(),
            principal_class: "human".to_owned(),
            created_at_ms: now,
            expires_at_ms: now
                + crate::auth_plugins::SessionServerPolicy::default().primary_max_age_ms,
            last_seen_at_ms: now,
            user_agent: Some("legacy-method-test".to_owned()),
            passkey_verified_at_ms: None,
            primary_authenticated_at_ms: now,
            primary_auth_method: None,
            revoked_at_ms: None,
            revoke_reason: None,
            auth_provider_id: None,
            auth_issuer: None,
            auth_subject: None,
            auth_sid: None,
            id_token_ciphertext: None,
        })
        .await
        .unwrap();
    let state = auth_state(Hub::new(), Some(store.clone()));
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        format!("{USER_SESSION_COOKIE}={legacy_token}")
            .parse()
            .unwrap(),
    );

    let bound = issue_product_session(
        &state,
        &store,
        &user,
        &headers,
        crate::auth_plugins::PASSWORD_LOGIN_METHOD,
        None,
    )
    .await;
    assert_eq!(bound.status(), StatusCode::OK);
    assert!(
        store
            .user_session_by_token_hash(&legacy_hash)
            .await
            .unwrap()
            .is_none()
    );
    let replacement_cookie = bound
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{USER_SESSION_COOKIE}=")))
        .expect("replacement product cookie")
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let replacement_token = replacement_cookie
        .strip_prefix(&format!("{USER_SESSION_COOKIE}="))
        .unwrap();
    assert_eq!(
        store
            .user_session_by_token_hash(&crate::admin::hex_sha256(replacement_token.as_bytes()))
            .await
            .unwrap()
            .expect("bound replacement session")
            .primary_auth_method
            .as_deref(),
        Some("password")
    );

    headers.insert(header::COOKIE, replacement_cookie.parse().unwrap());
    let switched = issue_product_session(&state, &store, &user, &headers, "cardea", None).await;
    assert_eq!(switched.status(), StatusCode::CONFLICT);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn native_oidc_handoff_requires_origin_pkce_and_sets_both_account_cookies() {
    install_rustls();
    let (store, root) = test_store().await;
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: "a".repeat(32),
        username: "owner".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let hub = Hub::new();
    let _ = seed_admin(&hub);
    let mut state = auth_state(hub, Some(store));
    state.product_authentication =
        Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
            Some(test_oidc_provider(&root, "owner", Some("owner"))),
        ));
    let verifier = "v".repeat(64);
    let challenge = crate::oidc::pkce_challenge(&verifier).unwrap();
    let handoff = state
        .oidc_native_handoffs
        .issue("cardea", &user.id, &challenge, &test_verified_identity())
        .unwrap();
    let callback = url::Url::parse(&handoff.location).unwrap();
    let code = callback
        .query_pairs()
        .find(|(name, _)| name == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap();
    let (base, handle) = spawn_auth(state).await;
    let body = serde_json::json!({
        "code": code,
        "code_verifier": verifier,
    });

    let missing_origin = reqwest::Client::new()
        .post(format!("{base}/api/auth/oidc/native/exchange"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_origin.status(), StatusCode::FORBIDDEN);

    let exchanged = post_json(
        &format!("{base}/api/auth/oidc/native/exchange"),
        &origin_for(&base),
        None,
        body.clone(),
    )
    .await;
    assert_eq!(exchanged.status(), StatusCode::OK);
    let user_cookie =
        set_cookie(&exchanged, crate::product_auth::USER_SESSION_COOKIE).expect("product cookie");
    let admin_cookie =
        set_cookie(&exchanged, crate::admin::ADMIN_SESSION_COOKIE).expect("admin cookie");
    assert!(user_cookie.contains("Max-Age=2592000"));
    assert!(admin_cookie.contains("Max-Age=43200"));

    let replayed = post_json(
        &format!("{base}/api/auth/oidc/native/exchange"),
        &origin_for(&base),
        None,
        body,
    )
    .await;
    assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);

    let cookies = format!(
        "{}; {}",
        cookie_header(&user_cookie),
        cookie_header(&admin_cookie)
    );
    let product_status: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/api/auth/status"))
        .header(header::COOKIE, &cookies)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let admin_status: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/api/admin/auth"))
        .header(header::COOKIE, &cookies)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(product_status["me"]["account"], "owner");
    assert_eq!(product_status["me"]["primary_auth_method"], "cardea");
    assert_eq!(
        product_status["login_method_order"],
        serde_json::json!(["cardea", "password"])
    );
    assert_eq!(admin_status["authenticated"], true);

    let cardea_cookie = cookie_header(&user_cookie);
    let switched_without_sign_out = post_json(
        &format!("{base}/api/auth/login"),
        &origin_for(&base),
        Some(&cardea_cookie),
        serde_json::json!({
            "account": "owner",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(switched_without_sign_out.status(), StatusCode::CONFLICT);
    assert!(
        switched_without_sign_out
            .text()
            .await
            .unwrap()
            .contains("Sign out before switching")
    );

    let signed_out = post_json(
        &format!("{base}/api/auth/logout"),
        &origin_for(&base),
        Some(&cardea_cookie),
        serde_json::json!({}),
    )
    .await;
    assert!(signed_out.status().is_success());
    let password_login = post_json(
        &format!("{base}/api/auth/login"),
        &origin_for(&base),
        None,
        serde_json::json!({
            "account": "owner",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(password_login.status(), StatusCode::OK);
    let password_body: serde_json::Value = password_login.json().await.unwrap();
    assert_eq!(password_body["primary_auth_method"], "password");

    handle.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn browser_shell_oidc_poll_is_origin_bound_pending_and_single_use() {
    install_rustls();
    let (store, root) = test_store().await;
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: "a".repeat(32),
        username: "owner".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let hub = Hub::new();
    let _ = seed_admin(&hub);
    let mut state = auth_state(hub, Some(store));
    state.product_authentication =
        Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
            Some(test_oidc_provider(&root, "owner", Some("owner"))),
        ));
    let verifier = "v".repeat(64);
    let handoff_token = "h".repeat(64);
    let code_challenge = crate::oidc::pkce_challenge(&verifier).unwrap();
    let handoff_challenge = crate::oidc::pkce_challenge(&handoff_token).unwrap();
    let handoffs = state.oidc_native_handoffs.clone();
    let (base, handle) = spawn_auth(state).await;
    let mut start_url = url::Url::parse(&format!("{base}/api/auth/oidc/start")).unwrap();
    start_url.query_pairs_mut().extend_pairs([
        ("client", "browser-shell"),
        ("code_challenge", code_challenge.as_str()),
        ("handoff_challenge", handoff_challenge.as_str()),
    ]);
    let start = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(start_url)
        .send()
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::SEE_OTHER);
    assert!(
        start
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|location| location.starts_with("https://cardea.example/"))
    );
    assert!(
        set_cookie(&start, crate::oidc::TRANSACTION_COOKIE)
            .is_some_and(|cookie| cookie.contains("HttpOnly"))
    );
    let body = serde_json::json!({
        "handoff_token": handoff_token,
        "code_verifier": verifier,
    });

    let missing_origin = reqwest::Client::new()
        .post(format!("{base}/api/auth/oidc/native/poll"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_origin.status(), StatusCode::FORBIDDEN);

    let pending = post_json(
        &format!("{base}/api/auth/oidc/native/poll"),
        &origin_for(&base),
        None,
        body.clone(),
    )
    .await;
    assert_eq!(pending.status(), StatusCode::ACCEPTED);
    assert_eq!(
        pending.json::<serde_json::Value>().await.unwrap()["status"],
        "pending"
    );

    handoffs
        .complete_browser(
            "cardea",
            &handoff_challenge,
            &user.id,
            &test_verified_identity(),
        )
        .unwrap();
    let exchanged = post_json(
        &format!("{base}/api/auth/providers/cardea/native/poll"),
        &origin_for(&base),
        None,
        body.clone(),
    )
    .await;
    assert_eq!(exchanged.status(), StatusCode::OK);
    let user_cookie =
        set_cookie(&exchanged, crate::product_auth::USER_SESSION_COOKIE).expect("product cookie");
    let admin_cookie =
        set_cookie(&exchanged, crate::admin::ADMIN_SESSION_COOKIE).expect("admin cookie");
    assert!(user_cookie.contains("Max-Age=2592000"));
    assert!(admin_cookie.contains("Max-Age=43200"));

    let replayed = post_json(
        &format!("{base}/api/auth/oidc/native/poll"),
        &origin_for(&base),
        None,
        body,
    )
    .await;
    assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);

    let cancelled_verifier = "c".repeat(64);
    let cancelled_handoff_token = "x".repeat(64);
    handoffs
        .begin_browser(
            "cardea",
            &crate::oidc::pkce_challenge(&cancelled_verifier).unwrap(),
            &crate::oidc::pkce_challenge(&cancelled_handoff_token).unwrap(),
        )
        .unwrap();
    let cancelled_body = serde_json::json!({
        "handoff_token": cancelled_handoff_token,
        "code_verifier": cancelled_verifier,
    });
    let cancelled = post_json(
        &format!("{base}/api/auth/providers/cardea/native/cancel"),
        &origin_for(&base),
        None,
        cancelled_body.clone(),
    )
    .await;
    assert_eq!(cancelled.status(), StatusCode::NO_CONTENT);
    let cancelled_poll = post_json(
        &format!("{base}/api/auth/oidc/native/poll"),
        &origin_for(&base),
        None,
        cancelled_body,
    )
    .await;
    assert_eq!(cancelled_poll.status(), StatusCode::UNAUTHORIZED);

    let denied_verifier = "d".repeat(64);
    let denied_handoff_token = "n".repeat(64);
    let denied_code_challenge = crate::oidc::pkce_challenge(&denied_verifier).unwrap();
    let denied_handoff_challenge = crate::oidc::pkce_challenge(&denied_handoff_token).unwrap();
    let mut denied_start_url = url::Url::parse(&format!("{base}/api/auth/oidc/start")).unwrap();
    denied_start_url.query_pairs_mut().extend_pairs([
        ("client", "browser-shell"),
        ("code_challenge", denied_code_challenge.as_str()),
        ("handoff_challenge", denied_handoff_challenge.as_str()),
    ]);
    let denied_start = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(denied_start_url)
        .send()
        .await
        .unwrap();
    assert_eq!(denied_start.status(), StatusCode::SEE_OTHER);
    let denied_transaction_cookie = cookie_header(
        &set_cookie(&denied_start, crate::oidc::TRANSACTION_COOKIE)
            .expect("OIDC transaction cookie"),
    );
    let denied_state = url::Url::parse(
        denied_start
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("authorization location"),
    )
    .unwrap()
    .query_pairs()
    .find(|(name, _)| name == "state")
    .map(|(_, value)| value.into_owned())
    .expect("OIDC state");
    let denied_callback = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(format!(
            "{base}/api/auth/oidc/callback?state={denied_state}&error=access_denied"
        ))
        .header(header::COOKIE, denied_transaction_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(denied_callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        denied_callback
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/api/auth/oidc/native/complete")
    );
    let denied_poll = post_json(
        &format!("{base}/api/auth/oidc/native/poll"),
        &origin_for(&base),
        None,
        serde_json::json!({
            "handoff_token": denied_handoff_token,
            "code_verifier": denied_verifier,
        }),
    )
    .await;
    assert_eq!(denied_poll.status(), StatusCode::GONE);

    let completion = reqwest::Client::new()
        .get(format!("{base}/api/auth/oidc/native/complete"))
        .send()
        .await
        .unwrap();
    assert_eq!(completion.status(), StatusCode::OK);
    assert_eq!(
        completion
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert!(completion.headers().contains_key("content-security-policy"));
    let completion_csp = completion
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let completion_body = completion.text().await.unwrap();
    // The window closes itself because a Provider serving
    // Cross-Origin-Opener-Policy severs the opener's handle to it.
    assert!(completion_body.contains("window.close()"));
    assert!(completion_body.contains(r#"<a href="/">"#));
    let script_digest = base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(NATIVE_COMPLETE_SCRIPT.as_bytes()));
    assert!(
        completion_csp.contains(&format!("script-src 'sha256-{script_digest}'")),
        "inline script must stay covered by its own CSP hash: {completion_csp}"
    );

    handle.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn browser_shell_oidc_events_are_origin_bound_and_do_not_consume_the_handoff() {
    install_rustls();
    let (store, root) = test_store().await;
    let mut state = auth_state(Hub::new(), Some(store));
    state.product_authentication =
        Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
            Some(test_oidc_provider(&root, "owner", Some("owner"))),
        ));
    let verifier = "v".repeat(64);
    let handoff_token = "h".repeat(64);
    let handoff_challenge = crate::oidc::pkce_challenge(&handoff_token).unwrap();
    state
        .oidc_native_handoffs
        .begin_browser(
            "cardea",
            &crate::oidc::pkce_challenge(&verifier).unwrap(),
            &handoff_challenge,
        )
        .unwrap();
    let handoffs = state.oidc_native_handoffs.clone();
    let (base, handle) = spawn_auth(state).await;
    let socket_url = format!(
        "{}/api/auth/oidc/native/events",
        base.replacen("http://", "ws://", 1)
    );

    let rejected = tokio_tungstenite::connect_async(&socket_url)
        .await
        .expect_err("a native handoff socket without Origin must fail");
    assert!(matches!(
        rejected,
        tokio_tungstenite::tungstenite::Error::Http(response)
            if response.status() == StatusCode::FORBIDDEN
    ));

    let mut request = socket_url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert(header::ORIGIN, origin_for(&base).parse().unwrap());
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    socket
        .send(TungsteniteMessage::Text(
            serde_json::json!({
                "handoff_token": handoff_token,
                "code_verifier": verifier,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    handoffs
        .complete_browser(
            "cardea",
            &handoff_challenge,
            &"a".repeat(32),
            &test_verified_identity(),
        )
        .unwrap();
    let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .expect("native handoff event timed out")
        .expect("native handoff socket closed")
        .expect("native handoff event failed");
    let TungsteniteMessage::Text(message) = message else {
        panic!("expected a native handoff text event");
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&message).unwrap(),
        serde_json::json!({ "status": "ready" })
    );
    assert!(matches!(
        handoffs
            .poll_browser("cardea", &"h".repeat(64), &"v".repeat(64))
            .unwrap(),
        crate::oidc::BrowserHandoffPoll::Ready { .. }
    ));

    let _ = socket.close(None).await;
    handle.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn product_machine_list_includes_enrolled_local_and_excludes_unenrolled_records() {
    let machine = |connection_mode: &str, fingerprint: Option<&str>, revoked: bool| {
        crate::store::MachineRecord {
            id: "machine".to_owned(),
            display_name: "Machine".to_owned(),
            connection_mode: connection_mode.to_owned(),
            platform: "linux".to_owned(),
            architecture: "x86_64".to_owned(),
            status: "offline".to_owned(),
            inventory: serde_json::json!({}),
            last_seen_at_ms: None,
            revoked,
            fingerprint: fingerprint.map(str::to_owned),
        }
    };
    assert!(product_machine_is_visible(
        &machine("outbound_wss", Some("SHA256:key"), false),
        true,
    ));
    assert!(product_machine_is_visible(
        &machine("local", Some("SHA256:key"), false),
        true,
    ));
    assert!(!product_machine_is_visible(
        &machine("local", None, false),
        true,
    ));
    assert!(!product_machine_is_visible(
        &machine("outbound_wss", None, false,),
        true
    ));
    assert!(!product_machine_is_visible(
        &machine("outbound_wss", Some("SHA256:key"), true),
        true,
    ));
    assert!(product_machine_is_visible(
        &machine("local", None, false),
        false,
    ));
    assert!(!product_machine_is_visible(
        &machine("local", None, true),
        false,
    ));
}

#[test]
fn authorize_ws_upgrade_requires_a_principal() {
    let headers = HeaderMap::new();
    let err = authorize_ws_upgrade(
        &headers,
        SocketAddr::from(([127, 0, 0, 1], 3333)),
        &[],
        None,
    )
    .unwrap_err();
    assert_eq!(err, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn disabled_product_auth_allows_anonymous_ws_as_local_owner() {
    let headers = HeaderMap::new();
    let principal = resolve_product_principal(false, None, &Hub::new(), &headers)
        .await
        .expect("disabled product auth should create a local principal");
    assert!(product_session_owner(false, &principal).is_none());
    let principal = authorize_ws_upgrade(
        &headers,
        SocketAddr::from(([127, 0, 0, 1], 3333)),
        &[],
        Some(principal),
    )
    .expect("local principal should be accepted");
    assert_eq!(principal.user_id, "local");
    assert_eq!(principal.role, crate::admin::AdminRole::Owner);
}

#[test]
fn enabled_product_auth_stamps_the_durable_user_as_session_owner() {
    let principal = ProductPrincipal {
        user_id: "0123456789abcdef0123456789abcdef".to_owned(),
        username: "draven".to_owned(),
        role: crate::admin::AdminRole::Owner,
    };
    let owner = product_session_owner(true, &principal).expect("session owner");
    assert_eq!(owner.user_id, principal.user_id);
    assert_eq!(owner.username, Some(principal.username.as_str()));
}

#[tokio::test]
async fn unauthenticated_ws_is_rejected_before_upgrade() {
    let (store, root) = test_store().await;
    let (base, server) = spawn_enforcement(auth_state(Hub::new(), Some(store))).await;
    let response = reqwest::Client::new()
        .get(format!("{base}/ws"))
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn metrics_with_forwarded_for_is_rejected() {
    let (store, root) = test_store().await;
    let (base, server) = spawn_enforcement(auth_state(Hub::new(), Some(store))).await;
    let rejected = reqwest::Client::new()
        .get(format!("{base}/metrics"))
        .header("x-forwarded-for", "1.2.3.4")
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::NOT_FOUND);
    let allowed = reqwest::Client::new()
        .get(format!("{base}/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn core_security_without_auth_plugins_supports_setup_login_and_session_reopen() {
    use std::os::unix::fs::PermissionsExt as _;
    let (store, root) = test_store().await;
    let core_path = root.join("core-security.json");
    std::fs::write(
        &core_path,
        serde_json::to_vec(&serde_json::json!({
            "schema": "dravengarden.cowboy.core-security/v1",
            "passkeys": { "namespace_id": "passkey", "source": "fresh" },
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&core_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut args = crate::cli::ServeArgs::test_plugin_check(&root);
    args.database_url = Some(format!(
        "sqlite://{}",
        root.join("cowboy.sqlite3").display()
    ));
    args.core_security_config = Some(core_path);
    args.product_auth_enabled = true;
    let (catalog, authentication, config) =
        crate::plugin_activation::prepare_controller_hosts(&args).unwrap();
    catalog.initialize().unwrap();
    let dir = crate::plugin_dir::PluginDir::open(&root).unwrap();
    let owner = crate::core_security::ControllerLock::acquire(&dir).unwrap();
    store
        .initialize_core_security(config.as_ref(), &dir)
        .await
        .unwrap();
    let storage = store.plugin_storage(dir.clone());
    let runtime = catalog.activate_runtime(&storage).await.unwrap();
    assert!(runtime.default_hosts().is_empty());
    assert!(runtime.namespace_for_capability("webauthn").is_none());
    let mut state = auth_state(Hub::new(), Some(store));
    state.plugin_catalog = Some(Arc::new(catalog));
    state.product_authentication = Arc::new(authentication);
    // A declared HTTPS relying-party origin, with all traffic staying on
    // the hermetic loopback listener. No live Service/device credential.
    let origin = "https://cowboy.example";
    state.public_origins = Arc::new(vec![origin.to_owned()]);
    let token = setup_token_from(&state);
    let (base, server) = spawn_auth(state.clone()).await;
    let client = reqwest::Client::new();
    let status: serde_json::Value = client
        .get(format!("{base}/api/auth/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["setup_required"], true);
    assert_eq!(status["password_enabled"], true);
    assert_eq!(status["host_plugins"], serde_json::json!([]));
    let setup_cookie = prove_setup(&base, origin, &token).await;
    let registered = post_json(
        &format!("{base}/api/auth/register"),
        origin,
        Some(&setup_cookie),
        serde_json::json!({"account":"owner", "password":"Correct-horse-bat1"}),
    )
    .await;
    assert_eq!(registered.status(), StatusCode::OK);
    let logged_in = post_json(
        &format!("{base}/api/auth/login"),
        origin,
        None,
        serde_json::json!({"account":"owner", "password":"Correct-horse-bat1"}),
    )
    .await;
    assert_eq!(logged_in.status(), StatusCode::OK);
    let cookie = cookie_header(&set_cookie(&logged_in, USER_SESSION_COOKIE).unwrap());
    let options = post_json(
        &format!("{base}/api/auth/passkeys/register/options"),
        origin,
        Some(&cookie),
        serde_json::json!({"nickname":"fixture key"}),
    )
    .await;
    assert_eq!(
        options.status(),
        StatusCode::OK,
        "{}",
        options.text().await.unwrap()
    );
    let listed = client
        .get(format!("{base}/api/auth/passkeys"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let signing_key = crate::client_auth::new_signing_key().unwrap();
    let started = client
        .post(format!("{base}/api/auth/device/authorizations"))
        .json(&crate::client_auth::StartAuthorizationRequest {
            name: "core-only fixture".to_owned(),
            public_key: crate::client_auth::public_key_to_base64(&signing_key),
            code_challenge: crate::client_auth::code_challenge(
                &crate::client_auth::new_code_verifier().unwrap(),
            )
            .unwrap(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::CREATED);
    server.abort();
    let _ = server.await;
    drop(owner);
    let _owner = crate::core_security::ControllerLock::acquire(&dir).unwrap();
    let reopened = Store::connect(args.database_url().unwrap(), root.join("artifacts"))
        .await
        .unwrap();
    reopened.migrate().await.unwrap();
    reopened
        .initialize_core_security(config.as_ref(), &dir)
        .await
        .unwrap();
    state.store = Some(reopened);
    let (base, server) = spawn_auth(state.clone()).await;
    let me = client
        .get(format!("{base}/api/auth/me"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let me: serde_json::Value = me.json().await.unwrap();
    assert_eq!(me["account"], "owner");
    server.abort();
    let _ = server.await;
    std::fs::remove_dir_all(&state.setup.data_dir).unwrap();
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn default_policy_rejects_register() {
    let (store, root) = test_store().await;
    let (base, server) = spawn_auth(auth_state(Hub::new(), Some(store))).await;
    let origin = origin_for(&base);
    let response = post_json(
        &format!("{base}/api/auth/register"),
        &origin,
        None,
        serde_json::json!({
            "account": "draven",
            "password": "long-enough-password",
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.text().await.unwrap(), "setup token required");
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn in_memory_store_returns_503() {
    let (base, server) = spawn_auth(auth_state(Hub::new(), None)).await;
    let origin = origin_for(&base);
    let response = post_json(
        &format!("{base}/api/auth/register"),
        &origin,
        None,
        serde_json::json!({
            "account": "draven",
            "password": "long-enough-password",
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let login = post_json(
        &format!("{base}/api/auth/login"),
        &origin,
        None,
        serde_json::json!({
            "account": "draven",
            "password": "long-enough-password",
        }),
    )
    .await;
    assert_eq!(login.status(), StatusCode::SERVICE_UNAVAILABLE);
    server.abort();
}

#[tokio::test]
async fn open_mode_ignores_token_and_login_sets_cookie() {
    let (store, root) = test_store().await;
    let hub = Hub::new();
    let state = auth_state(hub, Some(store));
    let token = setup_token_from(&state);
    let (base, server) = spawn_auth(state).await;
    let origin = origin_for(&base);
    let setup_cookie = prove_setup(&base, &origin, &token).await;
    let registered = post_json(
        &format!("{base}/api/auth/register"),
        &origin,
        Some(&setup_cookie),
        serde_json::json!({
            "account": "Draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(registered.status(), StatusCode::OK);
    let cookie = set_cookie(&registered, USER_SESSION_COOKIE).expect("register cookie");
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    let body: serde_json::Value = registered.json().await.unwrap();
    assert_eq!(body["account"], "draven");
    assert_eq!(body["role"], "owner");

    let _ = post_json(
        &format!("{base}/api/auth/logout"),
        &origin,
        Some(&cookie_header(&cookie)),
        serde_json::json!({}),
    )
    .await;

    let logged_in = post_json(
        &format!("{base}/api/auth/login"),
        &origin,
        None,
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(logged_in.status(), StatusCode::OK);
    let login_cookie = set_cookie(&logged_in, USER_SESSION_COOKIE).expect("login cookie");
    assert!(login_cookie.starts_with(&format!("{USER_SESSION_COOKIE}=")));
    assert!(login_cookie.contains("Max-Age=2592000"));

    let me = reqwest::Client::new()
        .get(format!("{base}/api/auth/me"))
        .header(header::COOKIE, cookie_header(&login_cookie))
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn operator_can_create_list_and_revoke_own_api_tokens() {
    let (store, root) = test_store().await;
    let state = auth_state(Hub::new(), Some(store));
    let token = setup_token_from(&state);
    let (base, server) = spawn_auth(state).await;
    let origin = origin_for(&base);
    let setup_cookie = prove_setup(&base, &origin, &token).await;
    let created = post_json(
        &format!("{base}/api/auth/register"),
        &origin,
        Some(&setup_cookie),
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);

    let logged_in = post_json(
        &format!("{base}/api/auth/login"),
        &origin,
        None,
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    let cookie = cookie_header(&set_cookie(&logged_in, USER_SESSION_COOKIE).unwrap());

    let minted = post_json(
        &format!("{base}/api/auth/tokens"),
        &origin,
        Some(&cookie),
        serde_json::json!({ "name": "zed" }),
    )
    .await;
    assert_eq!(minted.status(), StatusCode::CREATED);
    let minted_body: serde_json::Value = minted.json().await.unwrap();
    let secret = minted_body["token"].as_str().unwrap().to_owned();
    let token_id = minted_body["id"].as_str().unwrap().to_owned();
    assert!(secret.starts_with("cow_"));
    assert_eq!(minted_body["token_prefix"].as_str().unwrap(), &secret[..8]);
    assert!(minted_body.get("token_hash").is_none());

    let listed = reqwest::Client::new()
        .get(format!("{base}/api/auth/tokens"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed_body: serde_json::Value = listed.json().await.unwrap();
    assert_eq!(listed_body["tokens"][0]["id"], token_id);
    assert!(listed_body["tokens"][0].get("token").is_none());
    assert!(listed_body["tokens"][0].get("token_hash").is_none());

    let me = reqwest::Client::new()
        .get(format!("{base}/api/auth/me"))
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let me_body: serde_json::Value = me.json().await.unwrap();
    assert_eq!(me_body["account"], "draven");
    assert_eq!(me_body["role"], "owner");

    let bearer_create = reqwest::Client::new()
        .post(format!("{base}/api/auth/tokens"))
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .header(header::ORIGIN, "https://evil.example")
        .json(&serde_json::json!({ "name": "curl" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bearer_create.status(), StatusCode::CREATED);

    let revoked = reqwest::Client::new()
        .delete(format!("{base}/api/auth/tokens/{token_id}"))
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, &origin)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::OK);
    let rejected = reqwest::Client::new()
        .get(format!("{base}/api/auth/me"))
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn admin_created_user_is_operator() {
    let (store, root) = test_store().await;
    let hub = Hub::new();
    let admin = seed_admin(&hub);
    let (base, server) = spawn_auth(auth_state(hub.clone(), Some(store))).await;
    let origin = origin_for(&base);
    let created = post_json(
        &format!("{base}/api/admin/users"),
        &origin,
        Some(&admin),
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::FORBIDDEN);
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn concurrent_single_use_token_keeps_one_user() {
    let (store, root) = test_store().await;
    let state = auth_state(Hub::new(), Some(store.clone()));
    let token = setup_token_from(&state);
    let (base, server) = spawn_auth(state).await;
    let origin = origin_for(&base);
    let setup_cookie = prove_setup(&base, &origin, &token).await;
    let register_url = format!("{base}/api/auth/register");
    let first = post_json(
        &register_url,
        &origin,
        Some(&setup_cookie),
        serde_json::json!({
            "account": "alice",
            "password": "Correct-horse-bat1",
        }),
    );
    let second = post_json(
        &register_url,
        &origin,
        Some(&setup_cookie),
        serde_json::json!({
            "account": "bob",
            "password": "Correct-horse-bat1",
        }),
    );
    let (first, second) = tokio::join!(first, second);
    let statuses = [first.status(), second.status()];
    assert!(statuses.contains(&StatusCode::OK));
    assert!(statuses.contains(&StatusCode::FORBIDDEN) || statuses.contains(&StatusCode::CONFLICT));
    let users = store.list_users().await.unwrap();
    assert_eq!(users.len(), 1);
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn admin_bootstrap_creates_owner_then_product_user_can_login() {
    let (store, root) = test_store().await;
    let state = auth_state(Hub::new(), Some(store));
    let setup_token = std::fs::read_to_string(state.setup.token_path())
        .unwrap()
        .trim()
        .to_owned();
    assert!(setup_token.starts_with("cow_setup_"));
    let (base, server) = spawn_auth(state).await;
    let origin = origin_for(&base);

    let status = reqwest::Client::new()
        .get(format!("{base}/api/auth/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let body: serde_json::Value = status.json().await.unwrap();
    assert_eq!(body["setup_required"], true);
    assert_eq!(body["setup_pending"], false);
    assert_eq!(body["login_method_order"], serde_json::json!(["password"]));
    assert_eq!(body["session"]["activity_sliding_enabled"], true);
    assert_eq!(body["session"]["idle_timeout_ms"], 86_400_000_i64);
    assert_eq!(body["session"]["primary_max_age_ms"], 2_592_000_000_i64);
    assert_eq!(body["session"]["primary_warning_ms"], 86_400_000_i64);
    assert_eq!(body["registration"]["accepts_registration"], false);
    assert!(body.get("setup_token").is_none());
    assert!(!body.to_string().contains(&setup_token));

    let skipped = post_json(
        &format!("{base}/api/auth/register"),
        &origin,
        None,
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(skipped.status(), StatusCode::FORBIDDEN);

    let rejected = post_json(
        &format!("{base}/api/auth/setup"),
        &origin,
        None,
        serde_json::json!({ "token": "cow_setup_nope" }),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let prepared = post_json(
        &format!("{base}/api/auth/setup"),
        &origin,
        None,
        serde_json::json!({ "token": setup_token }),
    )
    .await;
    assert_eq!(prepared.status(), StatusCode::OK);
    let setup_set_cookie =
        set_cookie(&prepared, crate::admin::ADMIN_SETUP_COOKIE).expect("setup cookie");
    assert!(setup_set_cookie.contains("SameSite=Strict"));
    assert!(setup_set_cookie.contains("HttpOnly"));
    let setup_cookie = cookie_header(&setup_set_cookie);
    let prepared_body: serde_json::Value = prepared.json().await.unwrap();
    assert_eq!(prepared_body["setup_pending"], true);
    assert!(!prepared_body.to_string().contains(&setup_token));

    let created = post_json(
        &format!("{base}/api/auth/register"),
        &origin,
        Some(&setup_cookie),
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);

    let logged_in = post_json(
        &format!("{base}/api/auth/login"),
        &origin,
        None,
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(logged_in.status(), StatusCode::OK);
    let user_set_cookie = set_cookie(&logged_in, USER_SESSION_COOKIE).unwrap();
    assert!(user_set_cookie.contains("Max-Age=2592000"));

    let first_cookie = cookie_header(&user_set_cookie);
    let reauthenticated = post_json(
        &format!("{base}/api/auth/login"),
        &origin,
        Some(&first_cookie),
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(reauthenticated.status(), StatusCode::OK);
    let replacement_set_cookie =
        set_cookie(&reauthenticated, USER_SESSION_COOKIE).expect("replacement user cookie");
    let replacement_cookie = cookie_header(&replacement_set_cookie);

    let stale = reqwest::Client::new()
        .get(format!("{base}/api/auth/me"))
        .header(header::COOKIE, first_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);

    let me = reqwest::Client::new()
        .get(format!("{base}/api/auth/me"))
        .header(header::COOKIE, replacement_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let me_body: serde_json::Value = me.json().await.unwrap();
    assert_eq!(me_body["account"], "draven");
    assert_eq!(me_body["role"], "owner");
    assert_eq!(me_body["primary_auth_method"], "password");

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn disabled_product_auth_reports_a_local_owner_without_setup() {
    let (store, root) = test_store().await;
    let mut state = auth_state(Hub::new(), Some(store));
    state.product_auth_enabled = false;
    let (base, server) = spawn_auth(state).await;

    let status = reqwest::Client::new()
        .get(format!("{base}/api/auth/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let body: serde_json::Value = status.json().await.unwrap();
    assert_eq!(body["setup_required"], false);
    assert_eq!(body["setup_pending"], false);
    assert_eq!(body["login_method_order"], serde_json::json!([]));
    assert_eq!(body["me"]["account"], "local");
    assert_eq!(body["me"]["role"], "owner");
    assert_eq!(body["me"]["auth_enabled"], false);

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn admin_login_rejects_forwarded_cleartext() {
    let (store, root) = test_store().await;
    let hub = Hub::new();
    let _admin = seed_admin(&hub);
    let (base, server) = spawn_auth(auth_state(hub, Some(store))).await;
    let origin = origin_for(&base);
    let rejected = reqwest::Client::new()
        .post(format!("{base}/api/admin/auth/login"))
        .header(header::ORIGIN, &origin)
        .header("x-forwarded-for", "203.0.113.9")
        .header("x-forwarded-proto", "http")
        .json(&serde_json::json!({
            "account": "owner",
            "password": "Correct-horse-bat1",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_eq!(rejected.text().await.unwrap(), "admin login requires HTTPS");
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn admin_passkey_lock_is_off_until_a_credential_exists() {
    let (store, root) = test_store().await;
    let hub = Hub::new();
    let admin = seed_admin(&hub);
    let (base, server) = spawn_auth(auth_state(hub, Some(store))).await;
    let origin = origin_for(&base);
    let listed = reqwest::Client::new()
        .get(format!("{base}/api/admin/passkeys"))
        .header(header::COOKIE, &admin)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed_body: serde_json::Value = listed.json().await.unwrap();
    assert_eq!(listed_body["passkeys"].as_array().unwrap().len(), 0);

    let disabled = put_json(
        &format!("{base}/api/admin/passkeys/reauth"),
        &origin,
        Some(&admin),
        serde_json::json!({ "enabled": false }),
    )
    .await;
    assert_eq!(disabled.status(), StatusCode::OK);
    let body: serde_json::Value = disabled.json().await.unwrap();
    assert_eq!(body["passkey_reauth_enabled"], false);
    assert_eq!(body["passkey_reauth_required"], false);

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn admin_overview_requires_cookie() {
    let (store, root) = test_store().await;
    let hub = Hub::new();
    let admin = seed_admin(&hub);
    let (base, server) = spawn_auth(auth_state(hub, Some(store))).await;

    let denied = reqwest::Client::new()
        .get(format!("{base}/api/admin/overview"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let overview = reqwest::Client::new()
        .get(format!("{base}/api/admin/overview"))
        .header(header::COOKIE, &admin)
        .send()
        .await
        .unwrap();
    assert_eq!(overview.status(), StatusCode::OK);
    let body: serde_json::Value = overview.json().await.unwrap();
    assert_eq!(body["healthy"], true);
    assert_eq!(body["backend"], "store");
    assert_eq!(body["registration"]["accepts_registration"], false);

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn extra_users_and_admin_first_run_fail_closed() {
    let (store, root) = test_store().await;
    let hub = Hub::new();
    let admin = seed_admin(&hub);
    let (base, server) = spawn_auth(auth_state(hub.clone(), Some(store))).await;
    let origin = origin_for(&base);

    let saved = put_json(
        &format!("{base}/api/admin/registration"),
        &origin,
        Some(&admin),
        serde_json::json!({
            "enabled": true,
            "mode": "token",
        }),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::FORBIDDEN);
    assert_eq!(saved.text().await.unwrap(), "this instance is single-user");

    let extra_admin = post_json(
        &format!("{base}/api/admin/accounts"),
        &origin,
        Some(&admin),
        serde_json::json!({
            "account": "ops",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(extra_admin.status(), StatusCode::FORBIDDEN);

    let admin_setup = post_json(
        &format!("{base}/api/admin/auth/setup"),
        &origin,
        None,
        serde_json::json!({ "token": "cow_setup_nope" }),
    )
    .await;
    assert_eq!(admin_setup.status(), StatusCode::FORBIDDEN);
    assert_eq!(admin_setup.text().await.unwrap(), "complete setup on /");

    let admin_bootstrap = post_json(
        &format!("{base}/api/admin/auth/bootstrap"),
        &origin,
        None,
        serde_json::json!({
            "account": "ops",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(admin_bootstrap.status(), StatusCode::FORBIDDEN);

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn passkey_viewing_lock_is_off_until_a_credential_exists() {
    let (store, root) = test_store().await;
    let state = auth_state(Hub::new(), Some(store));
    let token = setup_token_from(&state);
    let (base, server) = spawn_auth(state).await;
    let origin = origin_for(&base);
    let setup_cookie = prove_setup(&base, &origin, &token).await;
    let registered = post_json(
        &format!("{base}/api/auth/register"),
        &origin,
        Some(&setup_cookie),
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(registered.status(), StatusCode::OK);
    let cookie = cookie_header(&set_cookie(&registered, USER_SESSION_COOKIE).unwrap());
    let me: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/api/auth/me"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["passkey_count"], 0);
    assert_eq!(me["passkey_reauth_required"], false);
    assert_eq!(me["passkey_reauth_enabled"], false);
    assert_eq!(
        me["passkey_reauth_after_ms"],
        crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS
    );

    let listed = reqwest::Client::new()
        .get(format!("{base}/api/auth/passkeys"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed_body: serde_json::Value = listed.json().await.unwrap();
    assert_eq!(listed_body["passkeys"].as_array().unwrap().len(), 0);

    let rejected_enable = put_json(
        &format!("{base}/api/auth/passkeys/reauth"),
        &origin,
        Some(&cookie),
        serde_json::json!({ "enabled": true }),
    )
    .await;
    assert_eq!(rejected_enable.status(), StatusCode::BAD_REQUEST);

    let disabled = put_json(
        &format!("{base}/api/auth/passkeys/reauth"),
        &origin,
        Some(&cookie),
        serde_json::json!({ "enabled": false }),
    )
    .await;
    assert_eq!(disabled.status(), StatusCode::OK);
    let disabled_body: serde_json::Value = disabled.json().await.unwrap();
    assert_eq!(disabled_body["passkey_reauth_enabled"], false);

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn external_passkey_handoff_is_public_only_in_safari_and_bound_to_one_session() {
    use sha2::Digest as _;

    let (store, root) = test_store().await;
    let mut state = auth_state(Hub::new(), Some(store));
    state.public_origins = Arc::new(vec!["https://cowboy.example".to_owned()]);
    let token = setup_token_from(&state);
    let (base, server) = spawn_auth(state).await;
    let origin = "https://cowboy.example".to_owned();
    let setup_cookie = prove_setup(&base, &origin, &token).await;
    let registered = post_json(
        &format!("{base}/api/auth/register"),
        &origin,
        Some(&setup_cookie),
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(registered.status(), StatusCode::OK);
    let first_cookie = cookie_header(&set_cookie(&registered, USER_SESSION_COOKIE).unwrap());
    let second_login = post_json(
        &format!("{base}/api/auth/login"),
        &origin,
        None,
        serde_json::json!({
            "account": "draven",
            "password": "Correct-horse-bat1",
        }),
    )
    .await;
    assert_eq!(second_login.status(), StatusCode::OK);
    let second_cookie = cookie_header(&set_cookie(&second_login, USER_SESSION_COOKIE).unwrap());
    let verifier = "a".repeat(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let started = post_json(
        &format!("{base}/api/auth/passkeys/external/start"),
        &origin,
        Some(&first_cookie),
        serde_json::json!({
            "action": "register",
            "nickname": "iPhone",
            "code_challenge": challenge,
        }),
    )
    .await;
    let started_status = started.status();
    let started_text = started.text().await.unwrap();
    assert_eq!(started_status, StatusCode::OK, "{started_text}");
    let started_body: serde_json::Value = serde_json::from_str(&started_text).unwrap();
    let transaction_id = started_body["transaction_id"].as_str().unwrap();
    assert_eq!(transaction_id.len(), 64);
    assert_eq!(
        started_body["expires_in_seconds"],
        crate::passkey::EXTERNAL_CEREMONY_TTL_SECS
    );

    let evil_options = post_json(
        &format!("{base}/api/auth/passkeys/external/options"),
        "https://evil.example",
        None,
        serde_json::json!({ "transaction_id": transaction_id }),
    )
    .await;
    assert_eq!(evil_options.status(), StatusCode::FORBIDDEN);
    let options = post_json(
        &format!("{base}/api/auth/passkeys/external/options"),
        &origin,
        None,
        serde_json::json!({ "transaction_id": transaction_id }),
    )
    .await;
    assert_eq!(options.status(), StatusCode::OK);
    let options_body: serde_json::Value = options.json().await.unwrap();
    assert_eq!(options_body["status"], "ready");
    assert_eq!(options_body["action"], "register");
    let public_key = options_body["publicKey"].as_object().unwrap();
    assert!(
        public_key
            .get("challenge")
            .is_some_and(|value| value.is_string())
    );
    assert!(public_key.get("rp").is_some_and(|value| value.is_object()));
    assert!(
        public_key
            .get("user")
            .is_some_and(|value| value.is_object())
    );
    assert!(public_key.get("publicKey").is_none());

    let wrong_session = post_json(
        &format!("{base}/api/auth/passkeys/external/finalize"),
        &origin,
        Some(&second_cookie),
        serde_json::json!({
            "transaction_id": transaction_id,
            "code_verifier": verifier,
        }),
    )
    .await;
    assert_eq!(wrong_session.status(), StatusCode::UNAUTHORIZED);
    let pending = post_json(
        &format!("{base}/api/auth/passkeys/external/finalize"),
        &origin,
        Some(&first_cookie),
        serde_json::json!({
            "transaction_id": transaction_id,
            "code_verifier": verifier,
        }),
    )
    .await;
    assert_eq!(pending.status(), StatusCode::ACCEPTED);

    let socket_url = format!(
        "{}/api/auth/passkeys/external/events",
        base.replacen("http://", "ws://", 1)
    );
    let rejected = tokio_tungstenite::connect_async(&socket_url)
        .await
        .expect_err("an external Passkey socket without Origin must fail");
    assert!(matches!(
        rejected,
        tokio_tungstenite::tungstenite::Error::Http(response)
            if response.status() == StatusCode::FORBIDDEN
    ));
    let mut socket_request = socket_url.into_client_request().unwrap();
    socket_request
        .headers_mut()
        .insert(header::ORIGIN, origin.parse().unwrap());
    socket_request
        .headers_mut()
        .insert(header::COOKIE, first_cookie.parse().unwrap());
    let (mut socket, response) = tokio_tungstenite::connect_async(socket_request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    socket
        .send(TungsteniteMessage::Text(
            serde_json::json!({
                "transaction_id": transaction_id,
                "code_verifier": verifier,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let cancelled = post_json(
        &format!("{base}/api/auth/passkeys/external/fail"),
        &origin,
        None,
        serde_json::json!({ "transaction_id": transaction_id }),
    )
    .await;
    assert_eq!(cancelled.status(), StatusCode::OK);
    let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .expect("external Passkey event timed out")
        .expect("external Passkey socket closed")
        .expect("external Passkey event failed");
    let TungsteniteMessage::Text(message) = message else {
        panic!("expected an external Passkey text event");
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&message).unwrap(),
        serde_json::json!({ "status": "failed" })
    );
    let finalized = post_json(
        &format!("{base}/api/auth/passkeys/external/finalize"),
        &origin,
        Some(&first_cookie),
        serde_json::json!({
            "transaction_id": transaction_id,
            "code_verifier": verifier,
        }),
    )
    .await;
    assert_eq!(finalized.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        finalized.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!({
            "code": "passkey_transaction_cancelled",
            "message": "Passkey verification was cancelled.",
        })
    );
    let _ = socket.close(None).await;

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn overdue_extended_session_is_rejected_server_side_but_base_login_can_recover() {
    let (store, root) = test_store().await;
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: "a".repeat(32),
        username: "draven".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    store
        .insert_user_passkey(&crate::passkey::UserPasskey {
            id: "b".repeat(32),
            user_id: user.id.clone(),
            credential_id: "credential".to_owned(),
            nickname: "Test Passkey".to_owned(),
            passkey_json: "{}".to_owned(),
            created_at_ms: now,
            last_used_at_ms: Some(now),
        })
        .await
        .unwrap();
    store
        .set_user_passkey_reauth(
            &user.id,
            true,
            crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS,
        )
        .await
        .unwrap();

    let extended_token = "c".repeat(64);
    let extended = crate::store::ProductUserSession {
        token_hash: crate::admin::hex_sha256(extended_token.as_bytes()),
        session_id: "extended-session-0000000000000001".to_owned(),
        user_id: user.id.clone(),
        client_kind: "browser".to_owned(),
        principal_class: "human".to_owned(),
        created_at_ms: now - 8 * 24 * 60 * 60 * 1_000,
        expires_at_ms: now + 22 * 24 * 60 * 60 * 1_000,
        last_seen_at_ms: now,
        user_agent: None,
        passkey_verified_at_ms: Some(now - 8 * 24 * 60 * 60 * 1_000),
        primary_authenticated_at_ms: now - 8 * 24 * 60 * 60 * 1_000,
        primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
        revoked_at_ms: None,
        revoke_reason: None,
        auth_provider_id: None,
        auth_issuer: None,
        auth_subject: None,
        auth_sid: None,
        id_token_ciphertext: None,
    };
    store.insert_user_session(&extended).await.unwrap();
    let base_token = "d".repeat(64);
    let base_session = crate::store::ProductUserSession {
        token_hash: crate::admin::hex_sha256(base_token.as_bytes()),
        session_id: "base-session-00000000000000000001".to_owned(),
        created_at_ms: now,
        expires_at_ms: now + crate::auth_plugins::SessionServerPolicy::default().primary_max_age_ms,
        passkey_verified_at_ms: None,
        ..extended.clone()
    };
    store.insert_user_session(&base_session).await.unwrap();

    let (base, server) = spawn_auth(auth_state(Hub::new(), Some(store))).await;
    let extended_cookie = format!("{USER_SESSION_COOKIE}={extended_token}");
    let status = reqwest::Client::new()
        .get(format!("{base}/api/auth/me"))
        .header(header::COOKIE, &extended_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let status: serde_json::Value = status.json().await.unwrap();
    assert_eq!(status["passkey_reauth_required"], true);

    let blocked = reqwest::Client::new()
        .get(format!("{base}/api/auth/tokens"))
        .header(header::COOKIE, &extended_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::PRECONDITION_REQUIRED);

    let base_cookie = format!("{USER_SESSION_COOKIE}={base_token}");
    let recoverable = reqwest::Client::new()
        .get(format!("{base}/api/auth/tokens"))
        .header(header::COOKIE, base_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(recoverable.status(), StatusCode::OK);

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn all_session_logout_requires_recent_proof_before_revoking_anything() {
    let (store, root) = test_store().await;
    let now = auth_now_ms();
    let user = crate::store::ProductUser {
        id: "a".repeat(32),
        username: "draven".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: crate::product_auth::hash_password("Correct-horse-bat1").unwrap(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let stale_token = "stale-logout-token-0000000000000000000000000000000000000001";
    let recent_token = "recent-logout-token-000000000000000000000000000000000000001";
    let other_token = "other-logout-token-0000000000000000000000000000000000000001";
    let session = |token: &str, id: &str, created_at_ms: i64| crate::store::ProductUserSession {
        token_hash: crate::admin::hex_sha256(token.as_bytes()),
        session_id: id.to_owned(),
        user_id: user.id.clone(),
        client_kind: "browser".to_owned(),
        principal_class: "human".to_owned(),
        created_at_ms,
        expires_at_ms: now + 24 * 60 * 60 * 1_000,
        last_seen_at_ms: now,
        user_agent: None,
        passkey_verified_at_ms: None,
        primary_authenticated_at_ms: created_at_ms,
        primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
        revoked_at_ms: None,
        revoke_reason: None,
        auth_provider_id: None,
        auth_issuer: None,
        auth_subject: None,
        auth_sid: None,
        id_token_ciphertext: None,
    };
    store
        .insert_user_session(&session(
            stale_token,
            "stale-logout-session-000000000001",
            now - PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS - 1,
        ))
        .await
        .unwrap();
    store
        .insert_user_session(&session(
            recent_token,
            "recent-logout-session-00000000001",
            now,
        ))
        .await
        .unwrap();
    store
        .insert_user_session(&session(
            other_token,
            "other-logout-session-000000000001",
            now - 1,
        ))
        .await
        .unwrap();

    let (base, server) = spawn_auth(auth_state(Hub::new(), Some(store.clone()))).await;
    let origin = origin_for(&base);
    let rejected = post_json(
        &format!("{base}/api/auth/logout"),
        &origin,
        Some(&format!("{USER_SESSION_COOKIE}={stale_token}")),
        serde_json::json!({ "scope": "all" }),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(
        store
            .list_user_sessions_for_user(&user.id, now)
            .await
            .unwrap()
            .len(),
        3,
        "a rejected all-session logout must not partially revoke sessions"
    );

    let accepted = post_json(
        &format!("{base}/api/auth/logout"),
        &origin,
        Some(&format!("{USER_SESSION_COOKIE}={recent_token}")),
        serde_json::json!({ "scope": "all" }),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let body: serde_json::Value = accepted.json().await.unwrap();
    assert_eq!(body["revoked_sessions"], 3);
    assert!(
        store
            .list_user_sessions_for_user(&user.id, now)
            .await
            .unwrap()
            .is_empty()
    );

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_deadlines_keep_passkey_and_primary_proofs_independent() {
    let policy = crate::passkey::PasskeyPolicy {
        enabled: true,
        reauth_after_ms: 7 * 24 * 60 * 60 * 1_000,
        last_step_up_at_ms: Some(99_000),
        passkey_count: 1,
    };
    let server = crate::auth_plugins::SessionServerPolicy::default();
    let mut session = crate::store::ProductUserSession {
        token_hash: "aa".repeat(32),
        session_id: "deadline-session-0000000000000001".to_owned(),
        user_id: "user-1".to_owned(),
        client_kind: "browser".to_owned(),
        principal_class: "human".to_owned(),
        created_at_ms: 100_000,
        expires_at_ms: 100_000 + server.primary_max_age_ms + 10_000,
        last_seen_at_ms: 100_000,
        user_agent: None,
        passkey_verified_at_ms: None,
        primary_authenticated_at_ms: 100_000,
        primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
        revoked_at_ms: None,
        revoke_reason: None,
        auth_provider_id: None,
        auth_issuer: None,
        auth_subject: None,
        auth_sid: None,
        id_token_ciphertext: None,
    };

    let deadlines = product_session_deadlines(server, &policy, true, &session, 100_000);
    assert_eq!(deadlines.passkey_due_at_ms, None);
    session.passkey_verified_at_ms = Some(100_000);
    let deadlines = product_session_deadlines(server, &policy, true, &session, 100_000);
    assert_eq!(
        deadlines.passkey_due_at_ms,
        Some(100_000 + server.passkey_max_age_ms)
    );
    assert_eq!(
        deadlines.primary_due_at_ms,
        100_000 + server.primary_max_age_ms
    );
    let original = deadlines;
    session.last_seen_at_ms += 10_000;
    let active = product_session_deadlines(server, &policy, true, &session, 110_000);
    assert_eq!(
        active.idle_due_at_ms,
        original.idle_due_at_ms.map(|due| due + 10_000)
    );
    assert_eq!(active.passkey_due_at_ms, original.passkey_due_at_ms);
    assert_eq!(active.primary_due_at_ms, original.primary_due_at_ms);
    assert_eq!(
        product_session_deadlines(server, &policy, true, &session, active.primary_due_at_ms,)
            .required_kind,
        Some(ProductSessionReauthKind::Primary),
        "activity and Passkey proof cannot move the primary-login cap"
    );

    let disabled = crate::passkey::PasskeyPolicy {
        enabled: false,
        ..policy
    };
    assert_eq!(
        product_session_deadlines(server, &disabled, true, &session, 100_000).passkey_due_at_ms,
        None
    );
    assert_eq!(
        product_session_deadlines(server, &policy, false, &session, 100_000).passkey_due_at_ms,
        None,
        "a persisted user toggle cannot bypass the server feature flag"
    );
}

#[test]
fn passkey_management_requires_a_recent_session_local_step_up() {
    let now = 1_000_000;
    let mut session = crate::store::ProductUserSession {
        token_hash: "aa".repeat(32),
        session_id: "step-up-session-0000000000000001".to_owned(),
        user_id: "user-1".to_owned(),
        client_kind: "browser".to_owned(),
        principal_class: "human".to_owned(),
        created_at_ms: now - PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS + 1,
        expires_at_ms: now + 1_000,
        last_seen_at_ms: now,
        user_agent: None,
        passkey_verified_at_ms: None,
        primary_authenticated_at_ms: now - PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS + 1,
        primary_auth_method: Some(crate::auth_plugins::PASSWORD_LOGIN_METHOD.to_owned()),
        revoked_at_ms: None,
        revoke_reason: None,
        auth_provider_id: None,
        auth_issuer: None,
        auth_subject: None,
        auth_sid: None,
        id_token_ciphertext: None,
    };
    assert!(product_session_has_recent_step_up(&session, now));
    session.created_at_ms = now - PASSKEY_MANAGEMENT_STEP_UP_MAX_AGE_MS;
    assert!(!product_session_has_recent_step_up(&session, now));
    session.passkey_verified_at_ms = Some(now - 1);
    assert!(product_session_has_recent_step_up(&session, now));
    session.passkey_verified_at_ms = Some(now + 60_001);
    assert!(!product_session_has_recent_step_up(&session, now));
}
