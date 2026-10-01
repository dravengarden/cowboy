#![cfg(feature = "full")]

use futures::{SinkExt as _, StreamExt as _};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

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
            .args([
                "serve",
                "--bind",
                &address.to_string(),
                "--product-auth-enabled=false",
            ])
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
    let url = format!("ws://{address}/ws?bootstrap=lazy&client_kind=cli");
    let (mut first, _) = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(socket) = tokio_tungstenite::connect_async(&url).await {
                break socket;
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
    .unwrap_or_else(|error| {
        panic!(
            "test controller did not start: {error}: {}",
            std::fs::read_to_string(root.path().join("controller.log")).unwrap()
        )
    });
    let (mut second, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

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
