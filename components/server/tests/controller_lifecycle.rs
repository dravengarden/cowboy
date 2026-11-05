#![cfg(feature = "full")]
//! The Controller's process lifecycle: it serves `/healthz` once startup has
//! restored its state, a SIGTERM drains the write-behind store before the
//! process exits, and a restart on the same data directory reopens what the
//! previous process accepted. This pins the ordering `serve` must keep while
//! its startup and shutdown phases are decomposed.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, SigningKey, signature::Signer as _};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

const ORIGIN: &str = "https://cowboy.example";
const ACCOUNT: &str = "fixture-owner";
const PASSWORD: &str = "Correct-horse-bat1";

struct TestController(Child);

impl Drop for TestController {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl TestController {
    /// SIGTERM, then wait for the process to exit on its own.
    async fn terminate(mut self) -> ExitStatus {
        let signalled = Command::new("kill")
            .args(["-TERM", &self.0.id().to_string()])
            .status()
            .unwrap();
        assert!(signalled.success(), "kill -TERM failed");
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Some(status) = self.0.try_wait().unwrap() {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("Controller did not exit after SIGTERM")
    }
}

async fn start(root: &Path, address: std::net::SocketAddr) -> TestController {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("controller.log"))
        .unwrap();
    let mut controller = TestController(
        Command::new(env!("CARGO_BIN_EXE_cowboy-controller"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .envs(
                std::env::vars_os()
                    .filter(|(name, _)| name == "SSL_CERT_FILE" || name == "SSL_CERT_DIR"),
            )
            .env("COWBOY_PUBLIC_ORIGIN", ORIGIN)
            .args([
                "serve",
                "--bind",
                &address.to_string(),
                "--product-auth-enabled=true",
            ])
            .arg("--database-url")
            .arg(format!(
                "sqlite://{}",
                root.join("controller.sqlite3").display()
            ))
            .arg("--data-dir")
            .arg(root)
            .arg("--workspace-root")
            .arg(root)
            .arg("--telemetry-dir")
            .arg(root.join("telemetry"))
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let http = reqwest::Client::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(response) = http.get(format!("http://{address}/healthz")).send().await {
                assert_eq!(response.status(), 200, "healthz before ready");
                break;
            }
            if let Some(status) = controller.0.try_wait().unwrap() {
                panic!(
                    "test Controller exited {status}: {}",
                    std::fs::read_to_string(root.join("controller.log")).unwrap()
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    controller
}

/// A signed browser session against one Controller process.
struct Browser {
    http: reqwest::Client,
    base: String,
    key: SigningKey,
    epoch: String,
    cookie: Option<String>,
}

impl Browser {
    async fn new(address: std::net::SocketAddr) -> Self {
        let http = reqwest::Client::new();
        let base = format!("http://{address}");
        let challenge: serde_json::Value = http
            .get(format!("{base}/api/auth/browser/challenge"))
            .header("x-forwarded-proto", "https")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        Self {
            http,
            base,
            key: SigningKey::random(&mut rand::rngs::OsRng),
            epoch: challenge["epoch"].as_str().unwrap().to_owned(),
            cookie: None,
        }
    }

    fn proof(&self, method: &str, target: &str) -> String {
        use rand::RngCore as _;
        let mut nonce = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let public =
            URL_SAFE_NO_PAD.encode(self.key.verifying_key().to_encoded_point(false).as_bytes());
        let time = chrono::Utc::now().timestamp_millis();
        let nonce = URL_SAFE_NO_PAD.encode(nonce);
        let epoch = &self.epoch;
        let message = format!(
            "cowboy-browser-proof-v1\n{epoch}\n{ORIGIN}\n{public}\n{method}\n{target}\n{time}\n{nonce}"
        );
        let signature: Signature = self.key.sign(message.as_bytes());
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({
                "key": public, "epoch": epoch, "origin": ORIGIN, "time": time,
                "nonce": nonce, "signature": URL_SAFE_NO_PAD.encode(signature.to_bytes()),
            }))
            .unwrap(),
        )
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .request(method.clone(), format!("{}{path}", self.base))
            .header("x-forwarded-proto", "https")
            .header("origin", ORIGIN)
            .header("x-cowboy-browser-proof", self.proof(method.as_str(), path));
        if let Some(cookie) = &self.cookie {
            request = request.header("cookie", cookie);
        }
        request
    }

    fn keep_cookie(&mut self, response: &reqwest::Response, prefix: &str) {
        self.cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with(prefix))
            .map(|value| value.split(';').next().unwrap().to_owned());
        assert!(self.cookie.is_some(), "no {prefix} cookie");
    }
}

#[tokio::test]
async fn sigterm_drains_accepted_state_and_restart_reopens_it() {
    let root = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let _ = rustls::crypto::ring::default_provider().install_default();

    let controller = start(root.path(), address).await;
    let mut browser = Browser::new(address).await;
    let token = std::fs::read_to_string(root.path().join("admin-setup.token")).unwrap();
    let setup = browser
        .request(reqwest::Method::POST, "/api/auth/setup")
        .json(&serde_json::json!({"token": token.trim()}))
        .send()
        .await
        .unwrap();
    assert_eq!(setup.status(), 200);
    browser.keep_cookie(&setup, "cowboy_admin_setup=");
    let registered = browser
        .request(reqwest::Method::POST, "/api/auth/register")
        .json(&serde_json::json!({"account": ACCOUNT, "password": PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(registered.status(), 200);
    browser.keep_cookie(&registered, "cowboy_user=");
    // Folder state reaches the database through the write-behind store, so
    // terminating immediately after the acknowledgement exercises the drain.
    let created = browser
        .request(reqwest::Method::POST, "/api/folders")
        .json(&serde_json::json!({"name": "lifecycle"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201, "{}", created.text().await.unwrap());
    let status = controller.terminate().await;
    assert!(
        status.success(),
        "SIGTERM exit {status}: {}",
        std::fs::read_to_string(root.path().join("controller.log")).unwrap()
    );

    let controller = start(root.path(), address).await;
    let mut browser = Browser::new(address).await;
    let login = browser
        .request(reqwest::Method::POST, "/api/auth/login")
        .json(&serde_json::json!({"account": ACCOUNT, "password": PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200, "{}", login.text().await.unwrap());
    browser.keep_cookie(&login, "cowboy_user=");
    let folders: serde_json::Value = browser
        .request(reqwest::Method::GET, "/api/folders")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        folders.to_string().contains("\"lifecycle\""),
        "folder accepted before SIGTERM was lost: {folders}"
    );
    assert!(controller.terminate().await.success());
}
