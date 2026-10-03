#![cfg(feature = "full")]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::{SinkExt as _, StreamExt as _};
use p256::ecdsa::{Signature, SigningKey, signature::Signer as _};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

struct TestController(Child);

impl Drop for TestController {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn foreground_probe_replies_only_to_the_requesting_websocket() {
    let root = tempfile::tempdir().unwrap();
    let log = std::fs::File::create(root.path().join("controller.log")).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut controller = TestController(
        Command::new(env!("CARGO_BIN_EXE_cowboy"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("COWBOY_PUBLIC_ORIGIN", "https://cowboy.example")
            .args([
                "serve",
                "--bind",
                &address.to_string(),
                "--product-auth-enabled=true",
            ])
            .arg("--database-url")
            .arg(format!(
                "sqlite://{}",
                root.path().join("controller.sqlite3").display()
            ))
            .arg("--data-dir")
            .arg(root.path())
            .arg("--workspace-root")
            .arg(root.path())
            .arg("--telemetry-dir")
            .arg(root.path().join("telemetry"))
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = reqwest::Client::new();
    let base = format!("http://{address}");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if http.get(format!("{base}/healthz")).send().await.is_ok() {
                break;
            }
            if let Some(status) = controller.0.try_wait().unwrap() {
                panic!(
                    "test controller exited {status}: {}",
                    std::fs::read_to_string(root.path().join("controller.log")).unwrap()
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let challenge: serde_json::Value = http
        .get(format!("{base}/api/auth/browser/challenge"))
        .header("x-forwarded-proto", "https")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = SigningKey::random(&mut rand::rngs::OsRng);
    let proof = |method: &str, target: &str| {
        use rand::RngCore as _;
        let mut nonce = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let public = URL_SAFE_NO_PAD.encode(key.verifying_key().to_encoded_point(false).as_bytes());
        let epoch = challenge["epoch"].as_str().unwrap();
        let time = chrono::Utc::now().timestamp_millis();
        let nonce = URL_SAFE_NO_PAD.encode(nonce);
        let message = format!(
            "cowboy-browser-proof-v1\n{epoch}\nhttps://cowboy.example\n{public}\n{method}\n{target}\n{time}\n{nonce}"
        );
        let signature: Signature = key.sign(message.as_bytes());
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({
                "key": public, "epoch": epoch, "origin": "https://cowboy.example", "time": time,
                "nonce": nonce, "signature": URL_SAFE_NO_PAD.encode(signature.to_bytes()),
            }))
            .unwrap(),
        )
    };
    let post = |path: &str| {
        http.post(format!("{base}{path}"))
            .header("x-forwarded-proto", "https")
            .header("origin", "https://cowboy.example")
            .header("x-cowboy-browser-proof", proof("POST", path))
    };
    let cookie = |response: &reqwest::Response, prefix: &str| {
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
    };
    let token = std::fs::read_to_string(root.path().join("admin-setup.token")).unwrap();
    let setup = post("/api/auth/setup")
        .json(&serde_json::json!({"token": token.trim()}))
        .send()
        .await
        .unwrap();
    assert_eq!(setup.status(), 200);
    let registered = post("/api/auth/register")
        .header("cookie", cookie(&setup, "cowboy_admin_setup="))
        .json(&serde_json::json!({"account":"fixture-owner","password":"Correct-horse-bat1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(registered.status(), 200);
    let user_cookie = cookie(&registered, "cowboy_user=");
    let dataset: serde_json::Value = http
        .get(format!("{base}/api/sync/dataset"))
        .header("cookie", &user_cookie)
        .header("x-forwarded-proto", "https")
        .header("x-cowboy-browser-proof", proof("GET", "/api/sync/dataset"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let socket_request = |client: &str| {
        let target = format!(
            "/ws?bootstrap=lazy&client_kind=browser&client_id={client}&dataset={}",
            dataset["dataset_id"].as_str().unwrap()
        );
        let mut request = format!("ws://{address}{target}")
            .into_client_request()
            .unwrap();
        let headers = request.headers_mut();
        headers.insert("cookie", user_cookie.parse().unwrap());
        headers.insert("origin", "https://cowboy.example".parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        headers.insert(
            "x-cowboy-browser-proof",
            proof("GET", &target).parse().unwrap(),
        );
        headers.insert("sec-websocket-protocol", "cowboy-sync-v1".parse().unwrap());
        request
    };
    let (mut first, _) = tokio_tungstenite::connect_async(socket_request("fixture-client-first"))
        .await
        .unwrap();
    let (mut second, _) = tokio_tungstenite::connect_async(socket_request("fixture-client-second"))
        .await
        .unwrap();

    for socket in [&mut first, &mut second] {
        tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "bootstrap_complete" {
                    return;
                }
            }
            panic!("socket closed before bootstrap");
        })
        .await
        .unwrap();
    }

    for nonce in [1, 2] {
        first
            .send(Message::Text(
                serde_json::json!({ "type": "connection_probe", "nonce": nonce })
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let Message::Text(text) = tokio::time::timeout(Duration::from_secs(3), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        else {
            panic!("expected addressed probe reply");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            serde_json::json!({ "type": "connection_probe", "nonce": nonce })
        );
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), second.next())
            .await
            .is_err()
    );
    first.close(None).await.unwrap();
    second.close(None).await.unwrap();
}
