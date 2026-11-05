
use super::*;
use tokio::net::UnixListener;

mod buffer;
mod operation;
mod scope;

fn test_socket(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "cowboy-zed-{label}-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[tokio::test]
async fn ensure_worktree_uses_the_stable_adapter_contract() {
    let socket = test_socket("worktree-client");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut request = String::new();
        BufReader::new(read).read_line(&mut request).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["type"], "ensureWorktree");
        assert_eq!(request["path"], "/tmp/worktree");
        assert_eq!(request["trusted"], true);
        write
            .write_all(b"{\"type\":\"worktree\",\"api_version\":1,\"state\":\"ready\"}\n")
            .await
            .unwrap();
    });

    assert!(ensure_zed_worktree(&socket, "/tmp/worktree").await.unwrap());
    server.await.unwrap();
    tokio::fs::remove_file(socket).await.unwrap();
}

#[tokio::test]
async fn buffer_lease_uses_the_stable_adapter_contract() {
    let socket = test_socket("buffer-client");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut request = String::new();
        BufReader::new(read).read_line(&mut request).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["type"], "openBuffer");
        assert_eq!(request["worktree"], "/tmp/worktree");
        assert_eq!(request["path"], "src/main.rs");
        assert_eq!(request["leaseId"], "browser-tab-1");
        write
            .write_all(
                b"{\"type\":\"buffer\",\"api_version\":1,\"path\":\"src/main.rs\",\"leases\":1}\n",
            )
            .await
            .unwrap();
    });

    let response = zed_adapter_request(
        &socket,
        serde_json::json!({
            "type": "openBuffer",
            "worktree": "/tmp/worktree",
            "path": "src/main.rs",
            "leaseId": "browser-tab-1",
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        response,
        ZedAdapterResponse::Buffer {
            path,
            leases: 1,
            ..
        } if path == "src/main.rs"
    ));
    server.await.unwrap();
    tokio::fs::remove_file(socket).await.unwrap();
}
