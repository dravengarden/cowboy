//! Real signed-runtime gate, disposable identities/configuration only. The
//! test-only LSP is an explicit executable, not an installed runtime dependency.
use super::*;
use crate::machine_protocol::code_buffer_navigation::{
    Action, BufferRef, Content, Kind, NavigationRef, Phase, Point, Request,
};
use serde_json::{Value, json};

const SOURCE: &str = "fn source() { target(); }\n";
const A: &str = "// a🙂z\npub fn target() {}\n";
const B: &str = "// b🙂q\npub struct Target;\n";

pub(super) struct Prepared {
    root: PathBuf,
    scope: PluginExecutionScope,
    navigation: NavigationRef,
    destination: BufferRef,
    content: Content,
}

fn content(text: &str) -> Content {
    Content {
        sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
        utf8_bytes: u32::try_from(text.len()).unwrap(),
    }
}

fn invocation(scope: &PluginExecutionScope, action: Action) -> CodeNavigationInvocation {
    scope
        .code_navigation(Request {
            service_id: format!("svc-{}", "a".repeat(32)),
            machine_id: "fixture".into(),
            action,
        })
        .unwrap()
}

pub(super) fn configure(store: &MachinePluginStore, digest: &str, root: &Path) -> PathBuf {
    let root = root.join("navigation");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("lsp-worktree")).unwrap();
    let executable = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("plugins/zed/adapter/target/debug/examples/navigation_lsp")
        .canonicalize()
        .expect("build the explicit test-only LSP fixture");
    assert!(executable.is_file());
    let config = store
        .plugin_root("zed")
        .join("runtime")
        .join(digest_generation_name(digest).unwrap())
        .join("home/.config/zed");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("settings.json"), serde_json::to_vec(&json!({
        "languages":{"Rust":{"language_servers":["rust-analyzer"]}},
        "lsp":{"rust-analyzer":{"binary":{"path":executable,"arguments":[root],"ignore_system_version":true}}}
    })).unwrap()).unwrap();
    root
}

fn events(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("lsp-events.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

async fn wait_event(root: &Path, method: &str, name: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if events(root).iter().any(|event| {
                event["method"] == method
                    && event["document"]
                        .as_str()
                        .is_some_and(|uri| uri.ends_with(name))
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("explicit fixture LSP did not observe the owned document");
}

pub(super) async fn prepare(store: &MachinePluginStore, root: PathBuf) -> Prepared {
    let worktree = root.join("lsp-worktree");
    for (name, text) in [
        ("navigation.rs", SOURCE),
        ("destination-a.rs", A),
        ("destination-b.rs", B),
    ] {
        fs::write(worktree.join(name), text).unwrap();
    }
    // Ensure does not leave a legacy worktree lease requiring a path close.
    store
        .code_request(
            "zed",
            &json!({"type":"ensureWorktree","path":worktree,"trusted":true}),
            None,
        )
        .await
        .unwrap();
    let prepared = store
        .code_request(
            "zed",
            &json!({"type":"prepareBuffer","worktree":worktree,"path":"navigation.rs"}),
            None,
        )
        .await
        .unwrap();
    let source: BufferRef = serde_json::from_value(prepared["lease"].clone()).unwrap();
    store
        .code_request(
            "zed",
            &json!({"type":"openBufferLease","lease":source}),
            None,
        )
        .await
        .unwrap();
    wait_event(&root, "textDocument/didOpen", "navigation.rs").await;
    let scope = PluginExecutionScope::new(Some(&format!("svc-{}", "a".repeat(32))), "fixture");
    let last = acquire(store, &scope, &source).await;
    let index = u32::try_from(
        last.locations
            .iter()
            .position(|location| location.path == "destination-a.rs")
            .unwrap(),
    )
    .unwrap();
    let action = Action::PrepareDestination {
        navigation: last.navigation.clone(),
        destination: index,
        content: content(A),
    };
    let handoff = store
        .navigate_code_buffer(invocation(&scope, action.clone()))
        .await
        .unwrap();
    assert_eq!(
        store
            .navigate_code_buffer(invocation(&scope, action))
            .await
            .unwrap(),
        handoff
    );
    let destination = handoff.destinations[0].lease.clone();
    store
        .code_request(
            "zed",
            &json!({"type":"releaseBufferLease","lease":source}),
            None,
        )
        .await
        .unwrap();
    for method in [
        "definition",
        "declaration",
        "typeDefinition",
        "implementation",
        "references",
    ] {
        assert_eq!(
            events(&root)
                .iter()
                .filter(|event| event["method"] == format!("textDocument/{method}"))
                .count(),
            1
        );
    }
    Prepared {
        root,
        scope,
        navigation: last.navigation,
        destination,
        content: content(A),
    }
}

async fn acquire(
    store: &MachinePluginStore,
    scope: &PluginExecutionScope,
    source: &BufferRef,
) -> crate::machine_protocol::code_buffer_navigation::Snapshot {
    let mut groups = Vec::new();
    for query in [
        Kind::Definition,
        Kind::Declaration,
        Kind::TypeDefinition,
        Kind::Implementation,
        Kind::References,
    ] {
        let prepared = store
            .navigate_code_buffer(invocation(
                scope,
                Action::Prepare {
                    lease: source.clone(),
                    content: content(SOURCE),
                    position: Point { row: 0, column: 3 },
                    query,
                },
            ))
            .await
            .unwrap();
        assert_eq!(prepared.phase, Phase::Prepared);
        let retained = store
            .navigate_code_buffer(invocation(
                scope,
                Action::Execute {
                    navigation: prepared.navigation.clone(),
                },
            ))
            .await
            .unwrap();
        assert_eq!(retained.phase, Phase::Retained);
        assert_eq!(retained.locations.len(), 3);
        for location in &retained.locations {
            assert_eq!(location.start, Point { row: 0, column: 4 });
            assert_eq!(location.end, Point { row: 0, column: 6 });
            assert_eq!(
                location.content,
                content(match location.path.as_str() {
                    "destination-a.rs" => A,
                    "destination-b.rs" => B,
                    _ => panic!("foreign destination"),
                })
            );
        }
        for action in [
            Action::Execute {
                navigation: retained.navigation.clone(),
            },
            Action::Query {
                navigation: retained.navigation.clone(),
            },
        ] {
            assert_eq!(
                store
                    .navigate_code_buffer(invocation(scope, action))
                    .await
                    .unwrap(),
                retained
            );
        }
        groups.push(retained);
    }
    let last = groups.pop().unwrap();
    for group in groups {
        assert_eq!(
            store
                .navigate_code_buffer(invocation(
                    scope,
                    Action::Release {
                        navigation: group.navigation
                    }
                ))
                .await
                .unwrap()
                .phase,
            Phase::Released
        );
    }
    last
}

pub(super) async fn finish(store: &MachinePluginStore, prepared: Prepared) {
    let Prepared {
        root,
        scope,
        navigation,
        destination,
        content,
    } = prepared;
    // Uninstalled: this must open only the already retained exact target.
    let opened = store
        .code_request(
            "zed",
            &json!({"type":"openBufferLease","lease":destination}),
            None,
        )
        .await
        .unwrap();
    assert_eq!(opened["state"], "open");
    for name in ["navigation.rs", "destination-a.rs", "destination-b.rs"] {
        fs::remove_file(root.join("lsp-worktree").join(name)).unwrap();
    }
    fs::rename(root.join("lsp-worktree"), root.join("removed-worktree")).unwrap();
    assert_eq!(
        store
            .navigate_code_buffer(invocation(&scope, Action::Release { navigation }))
            .await
            .unwrap()
            .phase,
        Phase::Released
    );
    let read = store.code_request("zed", &json!({"type":"readBufferLease","lease":destination,
        "request":{"kind":"content","content":content,"query":{"kind":"hover","position":{"row":0,"column":4}}}}), None).await.unwrap();
    assert_eq!(read["result"]["result"]["kind"], "hover");
    assert_eq!(
        read["result"]["result"]["contents"][0]["text"],
        "owned native destination fixture"
    );
    store
        .code_request(
            "zed",
            &json!({"type":"releaseBufferLease","lease":destination}),
            None,
        )
        .await
        .unwrap();
    for name in ["navigation.rs", "destination-a.rs", "destination-b.rs"] {
        wait_event(&root, "textDocument/didClose", name).await;
        for method in ["textDocument/didOpen", "textDocument/didClose"] {
            assert_eq!(
                events(&root)
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
    println!(
        "Machine navigation: five nonempty kinds, original destination handoff across uninstall and path removal, exact-content read and independent release passed (synthetic LSP/authority)"
    );
}
