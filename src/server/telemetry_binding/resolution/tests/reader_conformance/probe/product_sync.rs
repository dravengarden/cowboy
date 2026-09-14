//! Exact immutable Controller handshake acceptance. All users, passwords and
//! cookies are disposable fixture data; no production auth or Machine runs.
use super::*;
use reqwest::{Client, StatusCode, header};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Legacy,
    Bridge,
    Bound,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    readers: manifest::ControllerMatrix,
    /// In active, next-transaction recovery, cold order. A legacy reader must
    /// fail the new client's negotiation, not be labeled dataset-compatible.
    modes: [Mode; 3],
}

#[derive(Serialize)]
struct Check {
    role: Role,
    mode: Mode,
    cold_read: u8,
    accepted: bool,
    failure: Option<Failure>,
    stage: &'static str,
}

#[derive(Serialize)]
struct Receipt {
    schema: &'static str,
    source_revision: String,
    artifacts: Vec<Artifact>,
    checks: Vec<Check>,
    accepted: bool,
    not_checked: [&'static str; 4],
}

async fn seed_users(root: &Path) -> Result<String> {
    let store =
        crate::store::Store::connect(&database(root), root.join("controller/artifacts")).await?;
    let password = crate::client_auth::new_code_verifier()?;
    let hash = crate::product_auth::hash_password(&password)?;
    let now = chrono::Utc::now().timestamp_millis();
    for (id, username) in [("c", "dataset-operator"), ("d", "dataset-viewer")] {
        store
            .insert_user(&crate::store::ProductUser {
                id: id.repeat(32),
                username: username.into(),
                password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.into(),
                password_hash: hash.clone(),
                created_at_ms: now,
                updated_at_ms: now,
                disabled_at_ms: None,
            })
            .await?;
    }
    store
        .put_setting(
            crate::admin::PERMISSIONS_SETTING,
            &json!({
                "default_role":"viewer", "grants":[{"account":"dataset-operator","role":"operator"}]
            }),
        )
        .await?;
    private_write(
        &root.join("core-security.json"),
        &serde_json::to_vec(&json!({
            "schema":"dravengarden.cowboy.core-security/v1",
            "passkeys":{"namespace_id":"passkey","source":"fresh"}
        }))?,
    )?;
    Ok(password)
}

async fn body(mut response: reqwest::Response) -> Result<Value, Failure> {
    if !response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes().starts_with(b"application/json"))
    {
        return Err(Failure::WrongObservation);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Failure::WrongObservation)?
    {
        if bytes.len() + chunk.len() > 16 * 1024 {
            return Err(Failure::WrongObservation);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| Failure::FrameDecode)
}

async fn login(
    client: &Client,
    base: &str,
    account: &str,
    password: &str,
) -> Result<(String, Value), Failure> {
    let response = client
        .post(format!("{base}/api/auth/login"))
        .header(header::ORIGIN, base)
        .json(&json!({"account":account,"password":password}))
        .send()
        .await
        .map_err(|_| Failure::WrongObservation)?;
    if response.status() != StatusCode::OK {
        eprintln!(
            "product-sync fixture login status: {}",
            response.status().as_u16()
        );
        return Err(Failure::WrongObservation);
    }
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok()?.split(';').next())
        .find(|h| h.starts_with("cowboy_user="))
        .ok_or(Failure::WrongObservation)?
        .to_owned();
    Ok((cookie, body(response).await?))
}

async fn get(
    client: &Client,
    base: &str,
    path: &str,
    cookie: Option<&str>,
) -> Result<reqwest::Response, Failure> {
    let mut request = client.get(format!("{base}{path}"));
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    request.send().await.map_err(|_| Failure::WrongObservation)
}

async fn socket(
    base: &str,
    cookie: &str,
    dataset: Option<&str>,
    protocol: bool,
    expected: u16,
    kind: &str,
) -> Result<(), Failure> {
    let mut url =
        url::Url::parse(&base.replacen("http://", "ws://", 1)).map_err(|_| Failure::Setup)?;
    url.set_path("/ws");
    url.query_pairs_mut()
        .append_pair("bootstrap", "lazy")
        .append_pair("client_kind", kind);
    if let Some(dataset) = dataset {
        url.query_pairs_mut().append_pair("dataset", dataset);
    }
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| Failure::Setup)?;
    request
        .headers_mut()
        .insert(header::COOKIE, cookie.parse().map_err(|_| Failure::Setup)?);
    request
        .headers_mut()
        .insert(header::ORIGIN, base.parse().map_err(|_| Failure::Setup)?);
    if protocol {
        request.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "cowboy-sync-v1".parse().unwrap(),
        );
    }
    let outcome = tokio::time::timeout(DEADLINE, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| Failure::Timeout)?;
    match outcome {
        Ok((mut socket, response)) => {
            let matched = expected == 101
                && (!protocol
                    || response
                        .headers()
                        .get(header::SEC_WEBSOCKET_PROTOCOL)
                        .is_some_and(|v| v == "cowboy-sync-v1"));
            let _ = socket.close(None).await;
            check(matched)
        }
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            check(response.status().as_u16() == expected)
        }
        // The Web constructor also rejects an old peer that ignored the offered
        // subprotocol. This is a reader refusal, never successful new sync.
        Err(tokio_tungstenite::tungstenite::Error::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::SecWebSocketSubProtocolError(_),
        )) if expected == 0 => Ok(()),
        Err(_) => Err(Failure::SocketUpgrade),
    }
}

fn check(value: bool) -> Result<(), Failure> {
    if value {
        Ok(())
    } else {
        Err(Failure::WrongObservation)
    }
}

async fn exercise(
    address: std::net::SocketAddr,
    password: &str,
    mode: Mode,
    previous: &mut Option<String>,
    stage: &mut &'static str,
) -> Result<(), Failure> {
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(DEADLINE)
        .build()
        .map_err(|_| Failure::Setup)?;
    let base = format!("http://{address}");
    *stage = "operator_login";
    let (operator, operator_me) = login(&client, &base, "dataset-operator", password).await?;
    *stage = "viewer_login";
    let (viewer, viewer_me) = login(&client, &base, "dataset-viewer", password).await?;
    *stage = "roles";
    check(operator_me["role"] == "operator" && viewer_me["role"] == "viewer")?;
    *stage = "descriptor_status";
    let response = get(&client, &base, "/api/sync/dataset", Some(&operator)).await?;
    if mode == Mode::Legacy {
        // These retained Controllers classify unknown API routes as admin-only;
        // a Product cookie must get 401, not bypass the fallback to reach 404.
        check(response.status() == StatusCode::UNAUTHORIZED)?;
        *stage = "legacy_new_socket";
        socket(
            &base,
            &operator,
            Some(&format!("dataset-{}", "a".repeat(64))),
            true,
            0,
            "browser",
        )
        .await?;
        *stage = "legacy_old_socket";
        return socket(&base, &operator, None, false, 101, "browser").await;
    }
    check(
        response.status() == StatusCode::OK
            && response
                .headers()
                .get(header::CACHE_CONTROL)
                .is_some_and(|v| v == "no-store"),
    )?;
    let descriptor = body(response).await?;
    *stage = "descriptor_identity";
    let dataset = descriptor["dataset_id"]
        .as_str()
        .ok_or(Failure::WrongObservation)?;
    check(
        descriptor
            == json!({
                "schema":"dravengarden.cowboy.product-sync-dataset/v1", "dataset_id":dataset,
                "user_id":"c".repeat(32), "database_version":2, "outbox_contract":"atomic-delta-v1"
            }),
    )?;
    check(operator_me["user_id"] == "c".repeat(32) && viewer_me["user_id"] == "d".repeat(32))?;
    if let Some(old) = previous {
        check(old == dataset)?;
    }
    *previous = Some(dataset.into());
    *stage = "anonymous_refusal";
    check(
        get(&client, &base, "/api/sync/dataset", None)
            .await?
            .status()
            == StatusCode::UNAUTHORIZED,
    )?;
    let own_viewer = body(get(&client, &base, "/api/sync/dataset", Some(&viewer)).await?).await?;
    let own_viewer = own_viewer["dataset_id"]
        .as_str()
        .ok_or(Failure::WrongObservation)?;
    check(own_viewer != dataset)?;
    for kind in ["browser", "native_shell"] {
        *stage = "owned_socket";
        socket(&base, &operator, Some(dataset), true, 101, kind).await?;
        socket(&base, &viewer, Some(own_viewer), true, 101, kind).await?;
        *stage = "foreign_socket_refusal";
        socket(&base, &viewer, Some(dataset), true, 409, kind).await?;
        socket(&base, &operator, Some(own_viewer), true, 409, kind).await?;
        socket(&base, &operator, Some("dataset-untrusted"), true, 409, kind).await?;
        *stage = "protocol_refusal";
        socket(&base, &operator, Some(dataset), false, 426, kind).await?;
        *stage = "missing_dataset";
        socket(
            &base,
            &operator,
            None,
            false,
            if mode == Mode::Bound { 426 } else { 101 },
            kind,
        )
        .await?;
    }
    // Cookie clients cannot bypass browser compatibility by claiming a CLI.
    *stage = "cookie_cli_refusal";
    socket(&base, &operator, None, false, 400, "cli").await?;
    // Dataset selection is not a role or effect grant.
    *stage = "viewer_effect_refusal";
    check(
        get(&client, &base, "/api/sync/dataset", Some(&viewer))
            .await?
            .status()
            == StatusCode::OK,
    )?;
    let denied = client
        .post(format!(
            "{base}/api/machines/{MACHINE}/plugins/victoria/install"
        ))
        .header(header::ORIGIN, &base)
        .header(header::COOKIE, &viewer)
        .send()
        .await
        .map_err(|_| Failure::WrongObservation)?;
    check(denied.status() == StatusCode::FORBIDDEN)
}

#[tokio::test]
#[ignore = "just product-sync-controller-conformance <matrix> <new receipt>"]
async fn immutable_product_sync() -> Result<()> {
    let input: Input = serde_json::from_slice(&std::fs::read(std::env::var(
        "COWBOY_TEST_PRODUCT_SYNC_MATRIX",
    )?)?)?;
    let path = PathBuf::from(std::env::var("COWBOY_TEST_PRODUCT_SYNC_RECEIPT")?);
    let mut receipt = Receipt {
        schema: "dravengarden.cowboy.product-sync-controller-conformance/v1",
        source_revision: manifest::clean_revision()?,
        artifacts: input.readers.resolve()?,
        checks: Vec::new(),
        accepted: true,
        not_checked: [
            "production_accounts_and_host_activation",
            "physical_native_origin_and_gesture",
            "browser_storage_and_power_loss",
            "general_graph_state_leases_or_refactor_completion",
        ],
    };
    let helper = manifest::ssh_keygen()?;
    for (artifact, mode) in receipt.artifacts.iter().zip(input.modes) {
        let root = tempfile::tempdir()?;
        let fixture = Fixture::build(Case::Absent).await?;
        seed(root.path(), &fixture, &helper.path).await?;
        let password = seed_users(root.path()).await?;
        let mut previous = None;
        for cold_read in 1..=2 {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            drop(listener);
            let mut command = configured_command(artifact, root.path(), address);
            command
                .arg("--product-auth-enabled")
                .arg("true")
                .arg("--core-security-config")
                .arg(root.path().join("core-security.json"));
            let mut running = Running::spawn(&mut command)
                .map_err(|_| anyhow::anyhow!("fixture spawn failed"))?;
            let mut stage = "startup";
            let result = async {
                tokio::time::timeout(DEADLINE, controller(&mut running, address, &fixture))
                    .await
                    .map_err(|_| Failure::Timeout)??;
                exercise(address, &password, mode, &mut previous, &mut stage).await
            }
            .await;
            let cleanup = running.terminate().await;
            let result = result.and(cleanup);
            if result.is_ok() {
                stage = "complete";
            }
            receipt.accepted &= result.is_ok();
            eprintln!(
                "product-sync {:?}/{cold_read}/{stage}: {result:?}",
                artifact.role
            );
            receipt.checks.push(Check {
                role: artifact.role,
                mode,
                cold_read,
                accepted: result.is_ok(),
                failure: result.err(),
                stage,
            });
        }
    }
    write_receipt(&path, &receipt)?;
    ensure!(
        receipt.accepted && receipt.checks.len() == 6,
        "product sync Controller acceptance failed"
    );
    Ok(())
}
