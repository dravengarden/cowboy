//! Real bridge process, with a fixture gateway instead of Controller admission.
use super::*;
use crate::managed_calls::protocol::{Action, LocalRequest};
use crate::mcp_binding::{ProcessProfile, RelayAction};
use std::os::unix::fs::PermissionsExt as _;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};

struct Gateway {
    root: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
    seen: Arc<parking_lot::Mutex<Vec<&'static str>>>,
}
impl Gateway {
    fn new(link: Arc<Link>, profile: ProcessProfile) -> Self {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("gateway.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let context = root.path().join("context.json");
        std::fs::write(
            &context,
            json!({"schema":1,"socket":socket,"capability":"c".repeat(64)}).to_string(),
        )
        .unwrap();
        std::fs::set_permissions(context, std::fs::Permissions::from_mode(0o600)).unwrap();
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let observed = Arc::clone(&seen);
        let task = tokio::spawn(async move {
            let handle = Arc::new(parking_lot::Mutex::new(None::<String>));
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.unwrap();
                        let link = Arc::clone(&link);
                        let profile = profile.clone();
                        let handle = Arc::clone(&handle);
                        let observed = Arc::clone(&observed);
                        clients.spawn(async move {
                            let mut stream = BufReader::new(stream);
                            let mut bytes = Vec::new();
                            (&mut stream).take(262145).read_until(b'\n', &mut bytes).await.unwrap();
                            let frame = LocalRequest::parse(&bytes).unwrap();
                            assert_eq!(frame.capability, "c".repeat(64));
                            let Action::Mcp { request } = frame.action else { panic!("MCP only") };
                            assert_eq!(request.binding, profile.binding.name);
                            assert_eq!(request.epoch, profile.epoch);
                            let value = dispatch(&link, &profile, &handle, &observed, request.action).await;
                            let reply = json!({"schema":1,"state":"ok","result":value});
                            // Cancellation can close this socket before its response arrives.
                            let _ = stream.get_mut().write_all(format!("{reply}\n").as_bytes()).await;
                        });
                    }
                    Some(result) = clients.join_next() => { result.unwrap(); }
                }
            }
        });
        Self { root, task, seen }
    }
}
impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn dispatch(
    link: &Link,
    profile: &ProcessProfile,
    handle: &parking_lot::Mutex<Option<String>>,
    seen: &parking_lot::Mutex<Vec<&'static str>>,
    action: RelayAction,
) -> Value {
    let relay = format!("relay-{}", "b".repeat(32));
    match action {
        RelayAction::Open => {
            seen.lock().push("open");
            assert!(handle.lock().is_none());
            let opened = link
                .request(TransportRequest::Open {
                    profile: Box::new(profile.clone()),
                })
                .await;
            *handle.lock() = Some(opened["handle"].as_str().unwrap().to_owned());
            json!({"handle":relay})
        }
        RelayAction::Request {
            handle: requested,
            message,
        } => {
            assert_eq!(requested, relay);
            seen.lock().push("request");
            let remote = handle.lock().clone().unwrap();
            link.message(&remote, message).await
        }
        RelayAction::Close { handle: requested } => {
            assert_eq!(requested, relay);
            seen.lock().push("close");
            let remote = handle.lock().take().unwrap();
            link.request(TransportRequest::Close { handle: remote })
                .await
        }
    }
}

async fn send(stdin: &mut tokio::process::ChildStdin, message: Value) {
    stdin
        .write_all(format!("{message}\n").as_bytes())
        .await
        .unwrap();
}
async fn read(stdout: &mut BufReader<tokio::process::ChildStdout>) -> Value {
    let mut line = String::new();
    timeout(Duration::from_secs(5), stdout.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
#[ignore = "requires COWBOY_MCP_CONFORMANCE_CASE and COWBOY_MCP_CONFORMANCE_CLI"]
async fn signed_codeact_native_bridge_cancellation() {
    run(false).await;
}

#[tokio::test]
#[ignore = "requires COWBOY_MCP_CONFORMANCE_CASE and COWBOY_MCP_CONFORMANCE_CLI"]
async fn signed_codeact_native_bridge_eof() {
    run(true).await;
}

async fn run(eof: bool) {
    let cli = std::env::var("COWBOY_MCP_CONFORMANCE_CLI").expect("built native CLI required");
    assert!(std::path::Path::new(&cli).is_absolute());
    let f = mcp_conformance::install().await;
    let link = Arc::new(Link::new(f.store, &f.profile.service_id));
    let prepared = link
        .request(TransportRequest::PrepareNative {
            profile: Box::new(f.profile.clone()),
        })
        .await;
    let receipt = prepared["handle"].as_str().unwrap();
    assert_eq!(
        link.request(TransportRequest::ConfirmNative {
            handle: receipt.into()
        })
        .await,
        json!({"confirmed":true})
    );
    let gateway = Gateway::new(Arc::clone(&link), f.profile);
    let mut child = tokio::process::Command::new(cli)
        .args(["mcp", "--binding", prepared["path"].as_str().unwrap()])
        .env_clear()
        .env(
            "COWBOY_CALL_CONTEXT",
            gateway.root.path().join("context.json"),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
    )
    .await;
    assert!(read(&mut stdout).await["result"].is_object());
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    let listed = read(&mut stdout).await;
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(listed["result"]["tools"][0]["name"], f.case["tool"]);
    assert!(
        gateway.seen.lock().is_empty(),
        "discovery must stay offline"
    );
    let call = |id, arguments| json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":f.case["tool"],"arguments":arguments}});
    send(&mut stdin, call(3, f.case["arguments"].clone())).await;
    let result = read(&mut stdout).await;
    let content: Value =
        serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(content["state"], "ok");
    assert_eq!(content["value"], f.case["expected"]);
    send(
        &mut stdin,
        call(4, f.case.get("cancel_arguments").unwrap().clone()),
    )
    .await;
    let pids = timeout(Duration::from_secs(3), async {
        loop {
            let pids: Vec<_> = artifact_processes(f.root.path())
                .into_iter()
                .filter(|pid| Some(*pid) != child.id())
                .collect();
            if pids.len() >= 4 {
                break pids;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    if eof {
        drop(stdin);
    } else {
        send(
            &mut stdin,
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":4}}),
        )
        .await;
    }
    assert!(
        timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    timeout(Duration::from_secs(2), async {
        while pids.iter().any(|pid| process_live(*pid)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("sandbox descendants survived native bridge shutdown");
    assert_eq!(
        gateway
            .seen
            .lock()
            .iter()
            .filter(|action| **action == "open")
            .count(),
        1
    );
    assert_eq!(
        gateway
            .seen
            .lock()
            .iter()
            .filter(|action| **action == "close")
            .count(),
        1
    );
    assert!(!gateway.task.is_finished(), "fixture gateway failed");
    assert_eq!(
        link.request(TransportRequest::ReleaseNative {
            handle: receipt.into()
        })
        .await,
        json!({"released":true})
    );
}
