use super::super::{Buffers, Request, WorktreeState, Worktrees, respond};
use super::ensure_language;
use std::sync::Arc;

async fn scratch() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cowboy-zed-trust-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::canonicalize(dir).await.unwrap()
}

async fn worktree(path: &std::path::Path, trusted: bool, open: bool, all: &Worktrees) {
    let request = if open {
        Request::OpenWorktree {
            path: path.to_path_buf(),
            trusted,
        }
    } else {
        Request::EnsureWorktree {
            path: path.to_path_buf(),
            trusted,
        }
    };
    respond(request, all, &Buffers::default(), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn revoking_trust_restricts_now_and_drops_the_native_worktree_once_unowned() {
    let root = scratch().await;
    let all: Worktrees = Arc::default();

    worktree(&root, true, true, &all).await;
    let trusted = all.read().await[&root].clone();
    assert!(matches!(trusted.state, WorktreeState::Warming));
    assert!(trusted.trusted_once);
    ensure_language(&root, &all).await.unwrap();

    // A Code pane still holds the worktree: restrict, keep the native entry.
    worktree(&root, false, false, &all).await;
    let held = all.read().await[&root].clone();
    assert!(matches!(held.state, WorktreeState::Restricted));
    assert!(Arc::ptr_eq(&held.incarnation, &trusted.incarnation));
    assert!(ensure_language(&root, &all).await.is_err());

    respond(
        Request::CloseWorktree { path: root.clone() },
        &all,
        &Buffers::default(),
        None,
    )
    .await
    .unwrap();
    // The last close removed it; a later restricted ensure starts clean.
    worktree(&root, false, false, &all).await;
    let fresh = all.read().await[&root].clone();
    assert!(!Arc::ptr_eq(&fresh.incarnation, &trusted.incarnation));
    assert!(!fresh.trusted_once);
    assert!(matches!(fresh.state, WorktreeState::Restricted));

    // Trust can be granted again.
    worktree(&root, true, false, &all).await;
    assert!(matches!(
        all.read().await[&root].state,
        WorktreeState::Warming
    ));
    ensure_language(&root, &all).await.unwrap();
}

#[tokio::test]
async fn an_unleased_trusted_worktree_is_replaced_when_trust_is_revoked() {
    let root = scratch().await;
    let all: Worktrees = Arc::default();
    // Manifest discovery: trusted without a lease.
    worktree(&root, true, false, &all).await;
    let before = all.read().await[&root].clone();
    worktree(&root, false, false, &all).await;
    let after = all.read().await[&root].clone();
    assert!(!Arc::ptr_eq(&before.incarnation, &after.incarnation));
    assert!(matches!(after.state, WorktreeState::Restricted));
    assert!(!after.trusted_once);
}
