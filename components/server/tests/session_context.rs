#![cfg(feature = "full")]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, SigningKey, signature::Signer as _};
use serde_json::{Value, json};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Controller(Child);
impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    root: tempfile::TempDir,
    store: sqlx::SqlitePool,
    url: String,
    http: reqwest::Client,
    key: SigningKey,
    epoch: String,
    cookie: Option<String>,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", root.path().join("service.sqlite3").display());
        let options = url
            .parse::<sqlx::sqlite::SqliteConnectOptions>()
            .unwrap()
            .create_if_missing(true);
        let store = sqlx::SqlitePool::connect_with(options).await.unwrap();
        let mut migrations = sqlx::migrate!("../../migrations/sqlite");
        migrations.set_ignore_missing(true);
        migrations.run(&store).await.unwrap();
        sqlx::query("INSERT INTO machines(id,display_name,connection_mode,platform,architecture) VALUES ('offline-fixture','Fixture','outbound_wss','linux','x86_64')")
            .execute(&store).await.unwrap();
        for id in ["source", "destination"] {
            sqlx::query("INSERT INTO sessions(id,provider,machine_id,cwd,title,status,origin,agent_session_id) VALUES (?1,'codex','offline-fixture',?2,?1,'exited','web',?3)")
                .bind(id).bind(root.path().to_str().unwrap()).bind(format!("original-native-{id}"))
                .execute(&store).await.unwrap();
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut fixture = Self {
            root,
            store,
            url,
            http: reqwest::Client::new(),
            key: SigningKey::random(&mut rand::rngs::OsRng),
            epoch: String::new(),
            cookie: None,
        };
        let (controller, base) = fixture.start().await;
        let token = std::fs::read_to_string(fixture.root.path().join("admin-setup.token")).unwrap();
        let setup = fixture
            .request(reqwest::Method::POST, &format!("{base}/api/auth/setup"))
            .json(&json!({"token":token.trim()}))
            .send()
            .await
            .unwrap();
        assert_eq!(setup.status(), 200);
        let registered = fixture
            .request(reqwest::Method::POST, &format!("{base}/api/auth/register"))
            .header("cookie", cookie(&setup, "cowboy_admin_setup="))
            .json(&json!({"account":"context-fixture","password":"Correct-horse-bat1"}))
            .send()
            .await
            .unwrap();
        assert_eq!(registered.status(), 200);
        fixture.cookie = Some(cookie(&registered, "cowboy_user="));
        let owner = fixture.get(&format!("{base}/api/auth/me")).await;
        // Seed only this private fixture DB. Production ownership is assigned
        // by session creation, never by this test helper.
        sqlx::query("UPDATE sessions SET owner_user_id=?1")
            .bind(owner["user_id"].as_str().unwrap())
            .execute(&fixture.store)
            .await
            .unwrap();
        drop(controller);
        fixture
    }

    async fn start(&mut self) -> (Controller, String) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let log = std::fs::File::create(self.root.path().join("controller.log")).unwrap();
        let mut controller = Controller(
            Command::new(env!("CARGO_BIN_EXE_cowboy-controller"))
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .envs(
                    std::env::vars_os()
                        .filter(|(name, _)| name == "SSL_CERT_FILE" || name == "SSL_CERT_DIR"),
                )
                .env("COWBOY_PUBLIC_ORIGIN", "https://cowboy.example")
                .args([
                    "serve",
                    "--bind",
                    &address.to_string(),
                    "--product-auth-enabled=true",
                ])
                .args(["--database-url", &self.url])
                .arg("--data-dir")
                .arg(self.root.path())
                .arg("--workspace-root")
                .arg(self.root.path())
                .arg("--telemetry-dir")
                .arg(self.root.path().join("telemetry"))
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        let base = format!("http://{address}");
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if self
                    .http
                    .get(format!("{base}/healthz"))
                    .send()
                    .await
                    .is_ok()
                {
                    break;
                }
                if let Some(status) = controller.0.try_wait().unwrap() {
                    panic!(
                        "controller {status}: {}",
                        std::fs::read_to_string(self.root.path().join("controller.log")).unwrap()
                    );
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        let challenge: Value = self
            .http
            .get(format!("{base}/api/auth/browser/challenge"))
            .header("x-forwarded-proto", "https")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        self.epoch = challenge["epoch"].as_str().unwrap().to_owned();
        (controller, base)
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        use rand::RngCore as _;
        let mut nonce = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let nonce = URL_SAFE_NO_PAD.encode(nonce);
        let public =
            URL_SAFE_NO_PAD.encode(self.key.verifying_key().to_encoded_point(false).as_bytes());
        let time = chrono::Utc::now().timestamp_millis();
        let target = reqwest::Url::parse(url).unwrap().path().to_owned();
        let message = format!(
            "cowboy-browser-proof-v1\n{}\nhttps://cowboy.example\n{public}\n{method}\n{target}\n{time}\n{nonce}",
            self.epoch
        );
        let signature: Signature = self.key.sign(message.as_bytes());
        let proof = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "key":public,"epoch":self.epoch,"origin":"https://cowboy.example","time":time,
                "nonce":nonce,"signature":URL_SAFE_NO_PAD.encode(signature.to_bytes())
            }))
            .unwrap(),
        );
        let request = self
            .http
            .request(method, url)
            .header("x-forwarded-proto", "https")
            .header("origin", "https://cowboy.example")
            .header("x-cowboy-browser-proof", proof);
        if let Some(cookie) = &self.cookie {
            request.header("cookie", cookie)
        } else {
            request
        }
    }

    async fn get(&self, url: &str) -> Value {
        let response = self
            .request(reqwest::Method::GET, url)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }
}

fn cookie(response: &reqwest::Response, prefix: &str) -> String {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(prefix))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn session_context_cas_is_durable_and_first_send_replays_only_its_receipt() {
    let mut fixture = Fixture::new().await;
    let (controller, base) = fixture.start().await;
    let context_url = format!("{base}/api/sessions/destination/context");
    let edit = json!({"revision":0,"sources":["source"]});
    let (first, second) = tokio::join!(
        fixture
            .request(reqwest::Method::PUT, &context_url)
            .json(&edit)
            .send(),
        fixture
            .request(reqwest::Method::PUT, &context_url)
            .json(&edit)
            .send(),
    );
    let mut statuses = [
        first.unwrap().status().as_u16(),
        second.unwrap().status().as_u16(),
    ];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 409]);
    let context = fixture.get(&context_url).await;
    assert_eq!(context["revision"], 1);
    // Links wait for a prompt: nothing is pending, and they survive a restart.
    assert_eq!(context["applied_revision"], 1);
    assert_eq!(context["pending_links"], json!(["source"]));
    drop(controller);

    let (controller, base) = fixture.start().await;
    let context_url = format!("{base}/api/sessions/destination/context");
    let restored = fixture.get(&context_url).await;
    assert_eq!(restored["revision"], 1);
    assert_eq!(restored["pending_links"], json!(["source"]));
    let send_url = format!("{context_url}/send");
    // Exercise the HTTP body budget used by inline attachments. Axum's default
    // 2 MiB limit would reject this before the context handler ran.
    let message = json!({"revision":1,"operation":"one-send","text":"continue","content":[{"type":"text","text":"x".repeat(2 * 1024 * 1024)}]});
    let send = fixture
        .request(reqwest::Method::POST, &send_url)
        .json(&message)
        .send()
        .await
        .unwrap();
    assert_eq!(send.status(), 202);
    let accepted: Value = send.json().await.unwrap();
    assert_eq!(accepted["state"], "accepted");
    assert_eq!(accepted["session_id"], "destination");
    let retry = fixture
        .request(reqwest::Method::POST, &send_url)
        .json(&message)
        .send()
        .await
        .unwrap();
    assert_eq!(retry.status(), 202);
    let replay: Value = retry.json().await.unwrap();
    assert_eq!(replay["operation"], accepted["operation"]);
    assert_eq!(replay["session_id"], accepted["session_id"]);
    let changed = fixture
        .request(reqwest::Method::POST, &send_url)
        .json(&json!({"revision":1,"operation":"one-send","text":"different"}))
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), 409);
    assert_eq!(fixture.get(&context_url).await["applied_revision"], 2);
    let stale_edit = fixture
        .request(reqwest::Method::PUT, &context_url)
        .json(&json!({"revision":1,"sources":["source"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale_edit.status(), 409);
    assert!(stale_edit.text().await.unwrap().contains("Context changed"));
    let saved: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='session_context:destination'")
            .fetch_one(&fixture.store)
            .await
            .unwrap();
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved["injection"]["cmid"], "context-one-send");
    assert_eq!(saved["delivery"]["state"], "accepted");
    assert_eq!(saved["injection"]["sources"][0]["session_id"], "source");
    let native: String =
        sqlx::query_scalar("SELECT agent_session_id FROM sessions WHERE id='source'")
            .fetch_one(&fixture.store)
            .await
            .unwrap();
    assert_eq!(native, "original-native-source");
    drop(controller);

    // A second restart must keep the receipt; retrying cannot reserve another
    // destination or submit another prompt under the same operation.
    let (_controller, base) = fixture.start().await;
    let replay = fixture
        .request(
            reqwest::Method::POST,
            &format!("{base}/api/sessions/destination/context/send"),
        )
        .json(&message)
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 202);
    assert_eq!(replay.json::<Value>().await.unwrap()["state"], "accepted");
    let clear = fixture
        .request(
            reqwest::Method::PUT,
            &format!("{base}/api/sessions/destination/context"),
        )
        .json(&json!({"revision":2,"sources":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(clear.status(), 200);
    assert_eq!(
        clear.json::<Value>().await.unwrap()["delivery"]["operation"],
        "one-send"
    );
    let retained = fixture
        .request(
            reqwest::Method::POST,
            &format!("{base}/api/sessions/destination/context/send"),
        )
        .json(&message)
        .send()
        .await
        .unwrap();
    assert_eq!(retained.status(), 202);
}

#[tokio::test]
async fn refused_fresh_agent_records_failure_and_releases_its_destination_fence() {
    let mut fixture = Fixture::new().await;
    let (_controller, base) = fixture.start().await;
    let context_url = format!("{base}/api/sessions/source/context");
    let saved = fixture.request(reqwest::Method::PUT, &context_url).json(&json!({
        "revision":0,"sources":["source"],"next_agent":{
            "provider":"codex","provider_account_id":"default","machine_id":"offline-fixture","cwd":"missing-project","inherit_settings":false
        }
    })).send().await.unwrap();
    assert_eq!(saved.status(), 200, "{}", saved.text().await.unwrap());
    let sent = fixture
        .request(reqwest::Method::POST, &format!("{context_url}/send"))
        .json(&json!({"revision":1,"operation":"refused-fresh","text":"continue"}))
        .send()
        .await
        .unwrap();
    assert_eq!(sent.status(), 202);
    let receipt: Value = sent.json().await.unwrap();
    assert_eq!(receipt["state"], "failed");
    assert!(!receipt["error"].as_str().unwrap().is_empty());
    let source = fixture.get(&context_url).await;
    assert_eq!(source["revision"], 2);
    assert_eq!(source["delivery"]["state"], "failed");
    let saved: String = sqlx::query_scalar("SELECT value FROM settings WHERE key=?1")
        .bind(format!(
            "session_context:{}",
            receipt["session_id"].as_str().unwrap()
        ))
        .fetch_one(&fixture.store)
        .await
        .unwrap();
    let target: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(target["revision"], target["applied_revision"]);
    assert_eq!(target["delivery"]["state"], "failed");
    let clear = fixture
        .request(reqwest::Method::PUT, &context_url)
        .json(&json!({"revision":2,"sources":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(clear.status(), 200);
}

#[tokio::test]
async fn inheritance_preflight_refusal_creates_no_destination_or_context_reservation() {
    let mut fixture = Fixture::new().await;
    let (_controller, base) = fixture.start().await;
    let context_url = format!("{base}/api/sessions/source/context");
    let saved = fixture.request(reqwest::Method::PUT, &context_url).json(&json!({
        "revision":0,"sources":["source"],"next_agent":{
            "provider":"codex","provider_account_id":"default","machine_id":"offline-fixture","cwd":"missing-project"
        }
    })).send().await.unwrap();
    assert_eq!(saved.status(), 200);
    let receipt = fixture
        .request(reqwest::Method::POST, &format!("{context_url}/send"))
        .json(&json!({"revision":1,"operation":"preflight-refused","text":"continue"}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(receipt["state"], "failed");
    let id = receipt["session_id"].as_str().unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE id=?1")
            .bind(id)
            .fetch_one(&fixture.store)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM settings WHERE key=?1")
            .bind(format!("session_context:{id}"))
            .fetch_one(&fixture.store)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        fixture.get(&context_url).await["delivery"]["state"],
        "failed"
    );
}
