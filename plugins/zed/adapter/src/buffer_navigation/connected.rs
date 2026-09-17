//! Real private-server protocol regression in the existing isolated harness.
//! Plaintext has no LSP destinations; nonempty target/late-result behavior is
//! tested separately with deterministic native-protocol fixtures, not claimed
//! here as a production language-server or cross-file consumer acceptance.
use super::*;
use crate::{Request, Worktrees, respond};
use std::path::Path;
use std::sync::Arc;

pub(crate) async fn exercise(zed: &Zed, workspace: &Path) {
    let worktrees: Worktrees = Arc::default();
    let buffers: Buffers = Arc::default();
    let path = workspace.join("owned-navigation.txt");
    tokio::fs::write(&path, "native original🙂\n")
        .await
        .unwrap();
    let request = |request| respond(request, &worktrees, &buffers, Some(zed));
    request(Request::OpenWorktree {
        path: workspace.to_path_buf(),
        trusted: true,
    })
    .await
    .unwrap();
    let Response::BufferLease { lease, .. } = request(Request::PrepareBuffer {
        worktree: workspace.to_path_buf(),
        path: "owned-navigation.txt".into(),
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
    let native = buffers
        .active
        .read()
        .await
        .values()
        .next()
        .unwrap()
        .remote_id;
    let content = zed.diagnostics.lock().unwrap().content(native).unwrap();
    let mut retained = Vec::new();
    for kind in [
        NavigationKind::Definition,
        NavigationKind::Declaration,
        NavigationKind::TypeDefinition,
        NavigationKind::Implementation,
        NavigationKind::References,
    ] {
        retained.push(acquire(zed, &worktrees, &buffers, &lease, &content, kind).await);
    }
    assert!(
        request(Request::PrepareBufferSync {
            lease: lease.clone(),
            purpose: crate::sync_owners::Purpose::RefreshFromDisk,
            content,
        })
        .await
        .is_err(),
        "navigation peer was excluded from sync ownership"
    );
    tokio::fs::remove_file(&path).await.unwrap();
    request(Request::ReleaseBufferLease { lease })
        .await
        .unwrap();
    assert!(zed.diagnostics.lock().unwrap().revision(native).is_ok());
    for navigation in retained {
        for _ in 0..2 {
            let Response::OwnedBufferNavigation {
                state: State::Released,
                ..
            } = request(Request::BufferNavigation {
                navigation: navigation.clone(),
                action: Action::Release,
            })
            .await
            .unwrap()
            else {
                panic!("local navigation owner not released")
            };
        }
        assert!(
            Registry::default()
                .act(navigation, Action::Query, &buffers, Some(zed))
                .await
                .is_err()
        );
    }
    assert!(buffers.active.read().await.is_empty());
    assert!(zed.diagnostics.lock().unwrap().revision(native).is_err());
    request(Request::CloseWorktree {
        path: workspace.to_path_buf(),
    })
    .await
    .unwrap();
    println!(
        "native plaintext navigation: five one-use queries, retained source, shared-sync refusal and path-free local release passed (no nonempty-LSP acceptance)"
    );
}

async fn acquire(
    zed: &Zed,
    worktrees: &Worktrees,
    buffers: &Buffers,
    lease: &buffer_leases::LeaseRef,
    content: &Content,
    kind: NavigationKind,
) -> NavigationRef {
    let request = |request| respond(request, worktrees, buffers, Some(zed));
    let Response::OwnedBufferNavigation {
        navigation,
        state: State::Prepared,
        ..
    } = request(Request::PrepareBufferNavigation {
        lease: lease.clone(),
        content: content.clone(),
        position: Point { row: 0, column: 1 },
        kind,
    })
    .await
    .unwrap()
    else {
        panic!("navigation not prepared")
    };
    let before = zed.next_lsp_request_id.load(crate::Ordering::Relaxed);
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
        panic!("native query not retained")
    };
    assert!(
        locations.is_empty(),
        "plaintext unexpectedly acquired a language server"
    );
    assert_eq!(
        zed.next_lsp_request_id.load(crate::Ordering::Relaxed),
        before + 1
    );
    for action in [Action::Query, Action::Execute] {
        request(Request::BufferNavigation {
            navigation: navigation.clone(),
            action,
        })
        .await
        .unwrap();
    }
    assert_eq!(
        zed.next_lsp_request_id.load(crate::Ordering::Relaxed),
        before + 1
    );
    navigation
}
