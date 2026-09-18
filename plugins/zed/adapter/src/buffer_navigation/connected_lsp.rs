//! An actual immutable Zed server talks stdio LSP to an explicit test fixture.
//! Synthetic language answers, but real native target acquisition and events.
use super::*;
use crate::{Request, Worktrees, respond};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

mod pair;
pub(crate) use pair::immutable_pair;

const SOURCE: &str = "fn source() { target(); }\n";
const A: &str = "// a🙂z\npub fn target() {}\n";
const B: &str = "// b🙂q\npub struct Target;\n";

pub(crate) fn configure(root: &Path) {
    let executable = std::env::current_exe().unwrap();
    let fixture = executable
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/navigation_lsp")
        .canonicalize()
        .expect("build the test-only LSP example first");
    assert!(fixture.is_file());
    let directory = root.join("config/zed");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("settings.json"),
        serde_json::to_vec(&serde_json::json!({
            "languages": {"Rust": {"language_servers": ["rust-analyzer"]}},
            "lsp": {"rust-analyzer": {"binary": {
                "path": fixture, "arguments": [root], "ignore_system_version": true
            }}}
        }))
        .unwrap(),
    )
    .unwrap();
}

fn events(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("lsp-events.jsonl"))
        .unwrap_or_default()
        .lines()
        // The last line may still be being written by the fixture process.
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

async fn opened(root: &Path, name: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if events(root).iter().any(|event| {
                event["method"] == "textDocument/didOpen"
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
    .expect("explicit fixture LSP never opened the native buffer");
}

pub(crate) async fn exercise(zed: &Zed, root: &Path) {
    // A distinct native root: the earlier sync fixture deliberately retains a
    // worktree. Duplicate AddWorktree roots could route LSP targets to that peer.
    let workspace = root.join("lsp-worktree");
    std::fs::create_dir(&workspace).unwrap();
    for (name, text) in [
        ("navigation.rs", SOURCE),
        ("destination-a.rs", A),
        ("destination-b.rs", B),
    ] {
        std::fs::write(workspace.join(name), text).unwrap();
    }
    let worktrees: Worktrees = Arc::default();
    let buffers: Buffers = Arc::default();
    let request = |request| respond(request, &worktrees, &buffers, Some(zed));
    request(Request::OpenWorktree {
        path: workspace.clone(),
        trusted: true,
    })
    .await
    .unwrap();
    let Response::BufferLease { lease, .. } = request(Request::PrepareBuffer {
        worktree: workspace.clone(),
        path: "navigation.rs".into(),
    })
    .await
    .unwrap() else {
        panic!("not prepared")
    };
    request(Request::OpenBufferLease {
        lease: lease.clone(),
    })
    .await
    .unwrap();
    opened(root, "navigation.rs").await;
    let source = buffers
        .active
        .read()
        .await
        .values()
        .next()
        .unwrap()
        .remote_id;
    let content = zed.diagnostics.lock().unwrap().content(source).unwrap();
    let retained = acquire_all(zed, root, &lease, &content, &worktrees, &buffers).await;
    exercise_refusals(zed, root, &workspace).await;
    exercise_handoff(zed, &workspace, lease, retained, &worktrees, &buffers).await;
    assert!(buffers.active.read().await.is_empty());
    request(Request::CloseWorktree { path: workspace })
        .await
        .unwrap();
    println!(
        "real native nonempty LSP: five kinds, two cross-file targets, UTF-16, duplicate no-replay, path-free read/release and independent handoff passed (synthetic LSP, no production consumer)"
    );
}

async fn exercise_refusals(zed: &Zed, root: &Path, workspace: &Path) {
    use crate::navigation_native::{self, NativeRefusal};
    use crate::sync_native::wire::cowboy_navigation_response::Refusal;
    navigation_native::support(Some(zed)).await.unwrap();
    let worktree_id = *zed
        .worktree_paths
        .read()
        .await
        .iter()
        .find(|(_, path)| path.as_path() == workspace)
        .unwrap()
        .0;
    // Direct native protocol probes; adapter Unknown/no-replay ownership is
    // tested separately. Never reinterpret these refusals as restoration.
    let before = zed.buffer_files.read().await.len();
    for (index, (name, reason)) in [
        ("reject-locations.rs", Refusal::Budget),
        ("reject-targets.rs", Refusal::Budget),
        ("reject-external.rs", Refusal::Target),
        ("reject-range.rs", Refusal::Target),
        ("reject-lsp.rs", Refusal::LanguageServer),
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::write(workspace.join(name), SOURCE).unwrap();
        let (id, _) = zed.open_buffer(worktree_id, Path::new(name)).await.unwrap();
        opened(root, name).await;
        let position = zed.diagnostics.lock().unwrap().position(id, 0, 14).unwrap();
        let error = navigation_native::query(
            zed,
            crate::navigation_request(
                id,
                &position.version,
                position.anchor,
                NavigationKind::Definition,
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<NativeRefusal>()
                .expect("native refusal required, not timeout or generic transport error")
                .0,
            reason
        );
        assert_eq!(
            zed.buffer_files.read().await.len(),
            before + index + 1,
            "refused query unexpectedly shared targets"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join(name)).unwrap(),
            SOURCE
        );
        zed.close_buffer(id).unwrap();
    }
    println!(
        "actual native typed navigation refusals: location/target budgets, external target, invalid UTF-16 and LSP content-modified; no partial result passed"
    );
}

async fn acquire_all(
    zed: &Zed,
    root: &Path,
    lease: &buffer_leases::LeaseRef,
    content: &Content,
    worktrees: &Worktrees,
    buffers: &Buffers,
) -> Vec<(NavigationRef, Vec<Location>)> {
    let request = |request| respond(request, worktrees, buffers, Some(zed));
    let mut retained = Vec::new();
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
        let Response::OwnedBufferNavigation {
            navigation,
            state: State::Prepared,
            ..
        } = request(Request::PrepareBufferNavigation {
            lease: lease.clone(),
            content: content.clone(),
            position: Point { row: 0, column: 14 },
            kind,
        })
        .await
        .unwrap()
        else {
            panic!("not prepared")
        };
        let Response::OwnedBufferNavigation {
            state: State::Retained { locations },
            ..
        } = request(Request::BufferNavigation {
            navigation: navigation.clone(),
            action: Action::Execute,
        })
        .await
        .unwrap()
        else {
            panic!("not retained")
        };
        assert!(
            !locations.is_empty(),
            "real LSP returned no targets: {method}"
        );
        // Zed may deduplicate repeated LSP locations; neither target may be lost.
        for (name, text) in [("destination-a.rs", A), ("destination-b.rs", B)] {
            let location = locations
                .iter()
                .find(|location| location.path == Path::new(name))
                .unwrap();
            assert_eq!(
                location.content,
                *crate::coordinates::Mirror::new(1, text).unwrap().content()
            );
            assert_eq!(
                (
                    location.start.row,
                    location.start.column,
                    location.end.row,
                    location.end.column
                ),
                (0, 4, 0, 6)
            );
        }
        for action in [Action::Query, Action::Execute] {
            request(Request::BufferNavigation {
                navigation: navigation.clone(),
                action,
            })
            .await
            .unwrap();
        }
        assert_eq!(
            events(root)
                .iter()
                .filter(|event| event["method"] == method)
                .count(),
            1
        );
        retained.push((navigation, locations));
    }
    retained
}

async fn exercise_handoff(
    zed: &Zed,
    workspace: &Path,
    lease: buffer_leases::LeaseRef,
    retained: Vec<(NavigationRef, Vec<Location>)>,
    worktrees: &Worktrees,
    buffers: &Buffers,
) {
    let request = |request| respond(request, worktrees, buffers, Some(zed));
    assert_eq!(buffers.active.read().await.len(), 3);
    let destination = retained[0]
        .1
        .iter()
        .position(|location| location.path == Path::new("destination-a.rs"))
        .unwrap();
    let target_content = retained[0].1[destination].content.clone();
    let target_id = buffers.active.read().await
        [&(workspace.to_path_buf(), "destination-a.rs".into())]
        .remote_id;
    let before_handoff = zed.next_message_id.load(crate::Ordering::Relaxed);
    let Response::BufferLease {
        lease: handoff,
        state: buffer_leases::LeaseState::Prepared,
        ..
    } = request(Request::PrepareNavigationBuffer {
        navigation: retained[0].0.clone(),
        destination: u32::try_from(destination).unwrap(),
        content: target_content.clone(),
    })
    .await
    .unwrap()
    else {
        panic!("no handoff")
    };
    request(Request::OpenBufferLease {
        lease: handoff.clone(),
    })
    .await
    .unwrap();
    assert_eq!(
        zed.next_message_id.load(crate::Ordering::Relaxed),
        before_handoff,
        "handoff performed native I/O"
    );
    request(Request::ReleaseBufferLease { lease })
        .await
        .unwrap();
    // No disk lookup may participate in these original-target reads/releases.
    for name in ["navigation.rs", "destination-a.rs", "destination-b.rs"] {
        std::fs::remove_file(workspace.join(name)).unwrap();
    }
    read_and_release(zed, worktrees, buffers, retained).await;
    assert_eq!(buffers.active.read().await.len(), 1);
    assert_eq!(
        buffers
            .active
            .read()
            .await
            .values()
            .next()
            .unwrap()
            .remote_id,
        target_id
    );
    read_handoff(zed, worktrees, buffers, handoff, target_content).await;
}

async fn read_and_release(
    zed: &Zed,
    worktrees: &Worktrees,
    buffers: &Buffers,
    retained: Vec<(NavigationRef, Vec<Location>)>,
) {
    let request = |request| respond(request, worktrees, buffers, Some(zed));
    for (navigation, locations) in retained {
        for (destination, location) in locations.into_iter().enumerate() {
            let Response::BufferNavigationRead {
                result: crate::content_reads::Output::Hover { contents },
                ..
            } = request(Request::ReadBufferNavigation {
                navigation: navigation.clone(),
                destination: u32::try_from(destination).unwrap(),
                content: location.content,
                query: Query::Hover {
                    position: Point { row: 0, column: 4 },
                },
            })
            .await
            .unwrap()
            else {
                panic!("not hover")
            };
            assert_eq!(contents.len(), 1);
            assert_eq!(contents[0].text, "owned native destination fixture");
        }
        request(Request::BufferNavigation {
            navigation,
            action: Action::Release,
        })
        .await
        .unwrap();
    }
}

async fn read_handoff(
    zed: &Zed,
    worktrees: &Worktrees,
    buffers: &Buffers,
    handoff: buffer_leases::LeaseRef,
    target_content: Content,
) {
    let request = |request| respond(request, worktrees, buffers, Some(zed));
    let Response::BufferLeaseRead {
        result:
            buffer_leases::ReadOutput::Content {
                result: crate::content_reads::Output::Hover { contents },
                ..
            },
        ..
    } = request(Request::ReadBufferLease {
        lease: handoff.clone(),
        request: buffer_leases::ReadRequest::Content {
            content: target_content,
            query: Query::Hover {
                position: Point { row: 0, column: 4 },
            },
        },
    })
    .await
    .unwrap()
    else {
        panic!("not original target hover")
    };
    assert_eq!(contents.len(), 1);
    assert_eq!(contents[0].text, "owned native destination fixture");
    request(Request::ReleaseBufferLease { lease: handoff })
        .await
        .unwrap();
}
