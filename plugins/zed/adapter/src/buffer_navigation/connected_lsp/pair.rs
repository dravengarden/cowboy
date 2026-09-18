//! Drive final static adapter/server bytes, not an in-process `respond()` substitute.
use super::*;
use crate::buffer_leases::LeaseRef;
use serde_json::json;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};

struct Client(PathBuf);

fn target_text() -> String {
    let padding = 65_535 - A.len();
    // Keep the real server's 20,000-character LSP line limit intact.
    format!(
        "{A}{}{}🙂tail\n",
        "// x\n".repeat(padding / 5),
        " ".repeat(padding % 5)
    )
}

impl Client {
    async fn request(&self, request: Request) -> Value {
        let value = self.exchange(request).await;
        assert_ne!(value["type"], "error", "private adapter refused: {value}");
        value
    }

    async fn exchange(&self, request: Request) -> Value {
        tokio::time::timeout(Duration::from_secs(15), async {
            let mut stream = tokio::net::UnixStream::connect(&self.0).await.unwrap();
            let mut bytes = serde_json::to_vec(&request).unwrap();
            bytes.push(b'\n');
            stream.write_all(&bytes).await.unwrap();
            let mut bytes = Vec::new();
            tokio::io::BufReader::new(stream)
                .take(4 * 1024 * 1024 + 1)
                .read_until(b'\n', &mut bytes)
                .await
                .unwrap();
            assert!(bytes.len() <= 4 * 1024 * 1024 && bytes.last() == Some(&b'\n'));
            serde_json::from_slice(&bytes).unwrap()
        })
        .await
        .expect("immutable adapter request timed out")
    }
}

pub(crate) async fn immutable_pair(parent: &Path, server: &Path) {
    let Some(adapter) = std::env::var_os("COWBOY_TEST_NATIVE_NAVIGATION_ADAPTER") else {
        println!("immutable adapter navigation not checked; use zed-native-navigation-conformance");
        return;
    };
    let root = parent.join("released");
    std::fs::create_dir(&root).unwrap();
    for name in [
        "home",
        "config",
        "data",
        "cache",
        "state",
        "runtime",
        "tmp",
        "lsp-worktree",
    ] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    configure(&root);
    let socket = root.join("adapter.sock");
    let mut child = tokio::process::Command::new(adapter)
        .arg("serve")
        .arg("--socket")
        .arg(&socket)
        .arg("--zed-server")
        .arg(server)
        .arg("--state-dir")
        .arg(root.join("state"))
        .env_clear()
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("TMPDIR", root.join("tmp"))
        .env("PATH", "/nonexistent-cowboy-test-tools")
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while !socket.exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "immutable adapter failed to start"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let client = Client(socket);
    // Health is handled only after the adapter has started its exact server.
    client.request(Request::Health).await;
    assert_eq!(
        client.request(Request::BufferNavigationSupport {}).await,
        json!({"type":"bufferNavigationSupport","api_version":1,"protocol":1})
    );
    exercise(&client, &root).await;
    failed_open_fences_replacement(&client, &root).await;
    child.kill().await.unwrap();
    // The owning PID namespace, not this kill or local Released, owns final
    // descendant teardown. No native close ACK or recovery is claimed here.
    println!(
        "immutable adapter/server socket: five nonempty LSP kinds, lost handoff reply, independent original-target read/release and no replay passed"
    );
}

async fn prepared_buffer(client: &Client, workspace: &Path, path: &str) -> LeaseRef {
    let response = client
        .request(Request::PrepareBuffer {
            worktree: workspace.into(),
            path: path.into(),
        })
        .await;
    assert_eq!(response["state"], "prepared");
    serde_json::from_value(response["lease"].clone()).unwrap()
}

async fn failed_open_fences_replacement(client: &Client, root: &Path) {
    let workspace = root.join("open-failure-worktree");
    std::fs::create_dir(&workspace).unwrap();
    let text = "known🙂\n";
    std::fs::write(workspace.join("known.txt"), text).unwrap();
    std::fs::write(
        workspace.join("oversized.txt"),
        vec![b'a'; 4 * 1024 * 1024 + 1],
    )
    .unwrap();
    client
        .request(Request::OpenWorktree {
            path: workspace.clone(),
            trusted: true,
        })
        .await;
    let known = prepared_buffer(client, &workspace, "known.txt").await;
    client
        .request(Request::OpenBufferLease {
            lease: known.clone(),
        })
        .await;
    let uncertain = prepared_buffer(client, &workspace, "oversized.txt").await;
    let refusal = client
        .exchange(Request::OpenBufferLease {
            lease: uncertain.clone(),
        })
        .await;
    assert_eq!(refusal["type"], "error");
    assert!(
        refusal["message"]
            .as_str()
            .unwrap()
            .contains("bounded regular file")
    );
    // Fixing the fixture source cannot authorize replay or replacement adoption.
    std::fs::write(workspace.join("oversized.txt"), "now small\n").unwrap();
    for request in [
        Request::QueryBufferLease {
            lease: uncertain.clone(),
        },
        Request::OpenBufferLease {
            lease: uncertain.clone(),
        },
        Request::ReleaseBufferLease {
            lease: uncertain.clone(),
        },
    ] {
        assert_eq!(client.request(request).await["state"], "unknown");
    }
    for path in ["oversized.txt", "known.txt"] {
        let replacement = prepared_buffer(client, &workspace, path).await;
        let refused = client
            .exchange(Request::OpenBufferLease {
                lease: replacement.clone(),
            })
            .await;
        assert_eq!(refused["type"], "error");
        assert!(
            refused["message"]
                .as_str()
                .unwrap()
                .contains("unresolved native open")
        );
        assert_eq!(
            client
                .request(Request::QueryBufferLease {
                    lease: replacement.clone()
                })
                .await["state"],
            "prepared"
        );
        client
            .request(Request::ReleaseBufferLease { lease: replacement })
            .await;
    }
    read_and_release_known_buffer(client, known, text).await;
    assert_eq!(
        client
            .request(Request::QueryBufferLease { lease: uncertain })
            .await["state"],
        "unknown"
    );
    client.request(Request::BufferNavigationSupport {}).await;
    println!(
        "immutable socket native-open failure: real input refusal, original Unknown without replay, changed-source/known-key replacement refusal, retained independent text read/release passed (not recovery)"
    );
}

async fn read_and_release_known_buffer(client: &Client, known: LeaseRef, text: &str) {
    let content = crate::coordinates::Mirror::new(1, text)
        .unwrap()
        .content()
        .clone();
    let read = client
        .request(Request::ReadBufferLease {
            lease: known.clone(),
            request: buffer_leases::ReadRequest::Text {
                content,
                page: crate::text_reads::Page::Start {},
            },
        })
        .await;
    assert_eq!(read["result"]["result"]["text"], text);
    assert_eq!(
        client
            .request(Request::ReleaseBufferLease { lease: known })
            .await["state"],
        "released"
    );
}

async fn exercise(client: &Client, root: &Path) {
    let workspace = root.join("lsp-worktree");
    let target = target_text();
    for (name, text) in [
        ("navigation.rs", SOURCE),
        ("destination-a.rs", target.as_str()),
        ("destination-b.rs", B),
    ] {
        std::fs::write(workspace.join(name), text).unwrap();
    }
    client
        .request(Request::OpenWorktree {
            path: workspace.clone(),
            trusted: true,
        })
        .await;
    let prepared = client
        .request(Request::PrepareBuffer {
            worktree: workspace.clone(),
            path: "navigation.rs".into(),
        })
        .await;
    let lease: LeaseRef = serde_json::from_value(prepared["lease"].clone()).unwrap();
    client
        .request(Request::OpenBufferLease {
            lease: lease.clone(),
        })
        .await;
    opened(root, "navigation.rs").await;
    let (navigation, destination, content) = acquire(client, root, &lease).await;
    let handoff = lost_handoff_reply(client, &navigation, destination, &content).await;
    client.request(Request::ReleaseBufferLease { lease }).await;
    for name in ["navigation.rs", "destination-a.rs", "destination-b.rs"] {
        std::fs::remove_file(workspace.join(name)).unwrap();
    }
    client
        .request(Request::BufferNavigation {
            navigation,
            action: Action::Release,
        })
        .await;
    read_target_text(client, &handoff, &content, &target).await;
    let read = client
        .request(Request::ReadBufferLease {
            lease: handoff.clone(),
            request: buffer_leases::ReadRequest::Content {
                content,
                query: Query::Hover {
                    position: Point { row: 0, column: 4 },
                },
            },
        })
        .await;
    assert_eq!(
        read["result"]["result"]["contents"],
        json!([{
            "text":"owned native destination fixture", "language":null, "markdown":false
        }])
    );
    for _ in 0..2 {
        assert_eq!(
            client
                .request(Request::ReleaseBufferLease {
                    lease: handoff.clone()
                })
                .await["state"],
            "released"
        );
    }
    client
        .request(Request::CloseWorktree { path: workspace })
        .await;
    check_closed(root).await;
}

async fn read_target_text(client: &Client, handoff: &LeaseRef, content: &Content, target: &str) {
    let first = client
        .request(Request::ReadBufferLease {
            lease: handoff.clone(),
            request: buffer_leases::ReadRequest::Text {
                content: content.clone(),
                page: crate::text_reads::Page::Start {},
            },
        })
        .await;
    let page = &first["result"]["result"];
    assert_eq!(page["kind"], "page");
    assert_eq!(page["text"], target[..65_535]);
    assert_eq!(page["offset"], 0);
    assert_eq!(page["nextOffset"], 65_535);
    let second = client
        .request(Request::ReadBufferLease {
            lease: handoff.clone(),
            request: buffer_leases::ReadRequest::Text {
                content: content.clone(),
                page: crate::text_reads::Page::Continue {
                    offset: 65_535,
                    snapshot: page["snapshot"].as_str().unwrap().into(),
                },
            },
        })
        .await;
    let end = &second["result"]["result"];
    assert_eq!(end["kind"], "page");
    assert_eq!(end["snapshot"], page["snapshot"]);
    assert_eq!(end["text"], "🙂tail\n");
    assert_eq!(end["offset"], 65_535);
    assert!(end["nextOffset"].is_null());
}

async fn lost_handoff_reply(
    client: &Client,
    navigation: &NavigationRef,
    destination: u32,
    content: &Content,
) -> LeaseRef {
    let prepared = client
        .request(Request::PrepareNavigationBuffer {
            navigation: navigation.clone(),
            destination,
            content: content.clone(),
        })
        .await;
    assert_eq!(prepared["state"], "prepared");
    let handoff: LeaseRef = serde_json::from_value(prepared["lease"].clone()).unwrap();
    let mut disconnected = tokio::net::UnixStream::connect(&client.0).await.unwrap();
    let mut bytes = serde_json::to_vec(&Request::OpenBufferLease {
        lease: handoff.clone(),
    })
    .unwrap();
    bytes.push(b'\n');
    disconnected.write_all(&bytes).await.unwrap();
    drop(disconnected); // discard the real response, never synthesize one
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let saved = client
                .request(Request::QueryBufferLease {
                    lease: handoff.clone(),
                })
                .await;
            if saved["state"] == "open" {
                break;
            }
            assert_eq!(saved["state"], "prepared");
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    handoff
}

async fn check_closed(root: &Path) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let audit = events(root);
            let closed = audit
                .iter()
                .filter(|event| event["method"] == "textDocument/didClose")
                .count();
            if closed == 3 {
                break;
            }
            assert!(closed < 3, "duplicate native close");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture LSP did not observe three final closes");
    for name in ["navigation.rs", "destination-a.rs", "destination-b.rs"] {
        for method in ["textDocument/didOpen", "textDocument/didClose"] {
            assert_eq!(
                events(root)
                    .iter()
                    .filter(|event| event["method"] == method
                        && event["document"]
                            .as_str()
                            .is_some_and(|uri| uri.ends_with(name)))
                    .count(),
                1
            );
        }
    }
}

async fn acquire(client: &Client, root: &Path, lease: &LeaseRef) -> (NavigationRef, u32, Content) {
    let source = crate::coordinates::Mirror::new(1, SOURCE)
        .unwrap()
        .content()
        .clone();
    let mut last = None;
    for (kind, method) in [
        (NavigationKind::Definition, "textDocument/definition"),
        (NavigationKind::Declaration, "textDocument/declaration"),
        (
            NavigationKind::TypeDefinition,
            "textDocument/typeDefinition",
        ),
        (
            NavigationKind::Implementation,
            "textDocument/implementation",
        ),
        (NavigationKind::References, "textDocument/references"),
    ] {
        let prepared = client
            .request(Request::PrepareBufferNavigation {
                lease: lease.clone(),
                content: source.clone(),
                position: Point { row: 0, column: 14 },
                kind,
            })
            .await;
        assert_eq!(prepared["state"]["kind"], "prepared");
        let navigation: NavigationRef =
            serde_json::from_value(prepared["navigation"].clone()).unwrap();
        let acquired = client
            .request(Request::BufferNavigation {
                navigation: navigation.clone(),
                action: Action::Execute,
            })
            .await;
        assert_eq!(acquired["state"]["kind"], "retained");
        let locations = acquired["state"]["locations"].as_array().unwrap();
        let target = target_text();
        for (name, text) in [
            ("destination-a.rs", target.as_str()),
            ("destination-b.rs", B),
        ] {
            let location = locations
                .iter()
                .find(|location| location["path"] == name)
                .unwrap();
            assert_eq!(location["start"], json!({"row":0,"column":4}));
            assert_eq!(location["end"], json!({"row":0,"column":6}));
            assert_eq!(
                location["content"],
                serde_json::to_value(crate::coordinates::Mirror::new(1, text).unwrap().content())
                    .unwrap()
            );
        }
        for action in [Action::Query, Action::Execute] {
            assert_eq!(
                client
                    .request(Request::BufferNavigation {
                        navigation: navigation.clone(),
                        action
                    })
                    .await,
                acquired
            );
        }
        assert_eq!(
            events(root)
                .iter()
                .filter(|event| event["method"] == method)
                .count(),
            1
        );
        let destination = locations
            .iter()
            .position(|location| location["path"] == "destination-a.rs")
            .unwrap();
        let content = serde_json::from_value(locations[destination]["content"].clone()).unwrap();
        if let Some((previous, _, _)) = last.take() {
            client
                .request(Request::BufferNavigation {
                    navigation: previous,
                    action: Action::Release,
                })
                .await;
        }
        last = Some((navigation, u32::try_from(destination).unwrap(), content));
    }
    last.unwrap()
}
